use crate::models::*;
use crate::diagnostics;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;
use chrono::Utc;
use std::path::PathBuf;
use log::{info, error};

pub type SessionStore = Arc<RwLock<HashMap<Uuid, Arc<Mutex<DiagnosticSession>>>>>;
pub type ProgressSender = tokio::sync::mpsc::Sender<ProgressUpdate>;
pub type ProgressReceiver = tokio::sync::mpsc::Receiver<ProgressUpdate>;

pub struct DiagnosticService {
    sessions: SessionStore,
    progress_sender: ProgressSender,
}

impl DiagnosticService {
    pub fn new(progress_sender: ProgressSender) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            progress_sender,
        }
    }

    pub async fn get_available_tasks(&self) -> Vec<DiagnosticTask> {
        let is_admin = crate::admin::is_running_as_admin();
        diagnostics::get_filtered_tasks(is_admin)
            .into_iter()
            .map(|task| DiagnosticTask {
                id: task.name.to_lowercase().replace(" ", "_"),
                name: task.name.to_string(),
                description: self.get_task_description(task.name),
                admin_required: task.admin_required,
                category: self.get_task_category(task.name),
            })
            .collect()
    }

    pub async fn start_diagnostics(
        &self,
        request: DiagnosticRequest,
    ) -> Result<DiagnosticSession> {
        let session_id = Uuid::new_v4();
        let session = DiagnosticSession {
            id: session_id,
            status: SessionStatus::Pending,
            progress: 0.0,
            current_task: None,
            completed_tasks: 0,
            total_tasks: request.selected_tasks.len(),
            started_at: Utc::now(),
            completed_at: None,
            output_path: None,
            errors: Vec::new(),
        };

        let session_arc = Arc::new(Mutex::new(session.clone()));
        self.sessions.write().await.insert(session_id, session_arc.clone());

        // Start diagnostic task
        let _sessions = self.sessions.clone();
        let progress_sender = self.progress_sender.clone();
        let selected_tasks = request.selected_tasks.clone();
        let output_format = request.output_format.unwrap_or_default();

        tokio::spawn(async move {
            let result = run_diagnostics_with_progress(
                session_id,
                session_arc.clone(),
                selected_tasks,
                output_format,
                progress_sender.clone(),
            ).await;

            // Update final status
            let mut session = session_arc.lock().await;
            match result {
                Ok(output_path) => {
                    session.status = SessionStatus::Completed;
                    session.output_path = Some(output_path);
                    session.completed_at = Some(Utc::now());
                    info!("Diagnostics completed for session {}", session_id);
                }
                Err(e) => {
                    session.status = SessionStatus::Failed;
                    session.errors.push(e.to_string());
                    session.completed_at = Some(Utc::now());
                    error!("Diagnostics failed for session {}: {}", session_id, e);
                }
            }

            // Send final progress update
            let _ = progress_sender.send(ProgressUpdate {
                session_id,
                progress: session.progress,
                status: session.status.clone(),
                current_task: session.current_task.clone(),
                message: match &session.status {
                    SessionStatus::Completed => "Diagnostics completed successfully".to_string(),
                    SessionStatus::Failed => format!("Diagnostics failed: {}", session.errors.join(", ")),
                    _ => String::new(),
                },
                completed_tasks: session.completed_tasks,
                total_tasks: session.total_tasks,
                timestamp: Utc::now(),
            }).await;
        });

        Ok(session)
    }

    pub async fn get_session(&self, session_id: Uuid) -> Option<DiagnosticSession> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(&session_id) {
            Some(session_arc.lock().await.clone())
        } else {
            None
        }
    }

    pub async fn cancel_session(&self, session_id: Uuid) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(&session_id) {
            let mut session = session_arc.lock().await;
            if session.status == SessionStatus::Running {
                session.status = SessionStatus::Cancelled;
                session.completed_at = Some(Utc::now());
                Ok(())
            } else {
                Err(anyhow::anyhow!("Session is not running"))
            }
        } else {
            Err(anyhow::anyhow!("Session not found"))
        }
    }

    fn get_task_description(&self, task_name: &str) -> String {
        match task_name {
            "Computer System" => "Hardware and system information",
            "Operating System" => "Windows version and configuration",
            "BIOS" => "Firmware and boot settings",
            "BaseBoard" => "Motherboard specifications",
            "Processor" => "CPU details and capabilities",
            "Physical Memory" => "RAM configuration and usage",
            "Network Adapter" => "Network interfaces and settings",
            "Disk Drive" => "Storage devices and partitions",
            "DXDiag" => "DirectX and graphics diagnostics",
            "System Services" => "Windows services status",
            "Processes" => "Running applications and tasks",
            "Event Logs" => "System and application logs",
            _ => "System diagnostic information",
        }.to_string()
    }

    fn get_task_category(&self, task_name: &str) -> String {
        match task_name {
            "Computer System" | "Operating System" | "BIOS" | "BaseBoard" => "System",
            "Processor" | "Physical Memory" => "Hardware",
            "Network Adapter" | "IPConfig" => "Network",
            "Disk Drive" | "Disk Partition" | "Chkdsk" => "Storage",
            "System Services" | "Processes" | "Scheduled Tasks" => "Services",
            "Event Logs" | "Windows Update Log" => "Logs",
            "DXDiag" | "Drivers" | "Driver Verifier" => "Drivers",
            _ => "Other",
        }.to_string()
    }
}

async fn run_diagnostics_with_progress(
    session_id: Uuid,
    session: Arc<Mutex<DiagnosticSession>>,
    selected_tasks: Vec<String>,
    output_format: OutputFormat,
    progress_sender: ProgressSender,
) -> Result<String> {
    // Update status to running
    {
        let mut session = session.lock().await;
        session.status = SessionStatus::Running;
        session.started_at = Utc::now();
    }

    // Setup paths
    let desktop_path = dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."));
    let output_dir = desktop_path.join(format!("WindowsForum_{}", session_id));
    let zip_path = desktop_path.join(format!("WF-Diag_{}.zip", session_id));

    // Clean up and create directories
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir)?;
    }
    std::fs::create_dir_all(&output_dir)?;
    std::fs::create_dir_all(output_dir.join("Minidump"))?;

    // Create app state for compatibility with existing diagnostics
    let app_state = Arc::new(std::sync::Mutex::new(crate::AppState {
        progress: 0.0,
        status_text: String::new(),
        current_task: String::new(),
        is_running: true,
        tasks_completed: 0,
        total_tasks: selected_tasks.len(),
        is_admin: crate::admin::is_running_as_admin(),
        diagnostics_started: true,
        selected_tasks: selected_tasks.iter().map(|_| true).collect(),
        task_outputs: vec![String::new(); selected_tasks.len()],
        current_output: String::new(),
    }));

    // Filter tasks based on selection
    let all_tasks = diagnostics::DIAGNOSTIC_TASKS;
    let selected_diagnostics: Vec<_> = all_tasks
        .iter()
        .filter(|task| selected_tasks.contains(&task.name.to_lowercase().replace(" ", "_")))
        .copied()
        .collect();

    // Create progress monitoring task
    let session_clone = session.clone();
    let app_state_clone = app_state.clone();
    let progress_sender_clone = progress_sender.clone();
    let monitor_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            
            let (progress_update, is_running) = {
                let (progress, status_text, current_task, tasks_completed, total_tasks, is_running) = {
                    let app = app_state_clone.lock().unwrap();
                    (app.progress, app.status_text.clone(), app.current_task.clone(), 
                     app.tasks_completed, app.total_tasks, app.is_running)
                };
                
                let mut sess = session_clone.lock().await;
                sess.progress = progress;
                sess.current_task = if current_task.is_empty() { None } else { Some(current_task.clone()) };
                sess.completed_tasks = tasks_completed;
                
                let update = ProgressUpdate {
                    session_id,
                    progress,
                    status: sess.status.clone(),
                    current_task: sess.current_task.clone(),
                    message: status_text,
                    completed_tasks: tasks_completed,
                    total_tasks,
                    timestamp: Utc::now(),
                };
                
                (update, is_running)
            };
            
            let _ = progress_sender_clone.send(progress_update).await;
            
            if !is_running {
                break;
            }
        }
    });

    // Run the actual diagnostics
    diagnostics::run_selected_diagnostics(app_state, output_dir.clone(), zip_path.clone()).await?;

    // Stop monitoring
    monitor_handle.abort();

    // Return appropriate output based on format
    match output_format {
        OutputFormat::Json => {
            let json_path = output_dir.with_extension("json");
            // TODO: Convert diagnostics to JSON format
            Ok(json_path.to_string_lossy().to_string())
        }
        OutputFormat::Zip => Ok(zip_path.to_string_lossy().to_string()),
        OutputFormat::Both => Ok(zip_path.to_string_lossy().to_string()),
    }
}