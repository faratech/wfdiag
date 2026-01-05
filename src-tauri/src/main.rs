#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Use the library crate for all diagnostics functionality
use wfdiag_tauri::diagnostics::{self, DiagnosticTask, TaskResult};
use wfdiag_tauri::results_storage::{ScanStorage, ScanRecord, ScanSummary as StoredScanSummary, ComparisonResult};
use wfdiag_tauri::phi_silica::PhiSilicaStatus;

mod ui;

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Application state
pub struct WfDiagApp {
    /// Tokio runtime for async operations
    runtime: Arc<Runtime>,

    /// Available diagnostic tasks
    available_tasks: Vec<DiagnosticTask>,

    /// Results from diagnostic runs
    results: Arc<Mutex<HashMap<String, TaskResult>>>,

    /// Current scan state
    scan_state: ScanState,

    /// Channel for receiving scan updates
    scan_rx: Option<mpsc::Receiver<ScanUpdate>>,

    /// Current progress (0-100)
    progress: f32,

    /// Current task being executed
    current_task: String,

    /// Selected tab
    selected_tab: Tab,

    /// Expanded sections in results view
    expanded_sections: HashMap<String, bool>,

    /// Expanded result rows
    expanded_rows: HashMap<String, bool>,

    /// System info
    system_info: Option<SystemInfo>,

    /// Scan history
    scan_history: Vec<ScanSummary>,

    /// Settings
    settings: Settings,

    /// Show settings window
    show_settings: bool,

    /// Show about window
    show_about: bool,

    /// Status message
    status_message: String,

    /// Scan start time
    scan_start_time: Option<std::time::Instant>,

    /// Last scan duration
    last_scan_duration: Option<std::time::Duration>,

    /// System monitoring state
    monitoring_state: ui::monitoring::MonitoringState,

    /// Channel for receiving monitoring stats
    monitoring_rx: Option<mpsc::Receiver<wfdiag_tauri::native_monitor::SystemStats>>,

    /// Flag to control monitoring
    monitoring_active: Arc<std::sync::atomic::AtomicBool>,

    /// Process list state (for full htop-like process viewer)
    process_list_state: ui::process_list::ProcessListState,

    /// All processes (updated alongside monitoring)
    all_processes: Vec<wfdiag_tauri::native_monitor::ProcessInfo>,

    /// Show full process list vs top processes
    show_full_process_list: bool,

    /// Scan storage for persisting history
    scan_storage: Option<ScanStorage>,

    /// Stored scan summaries (from disk)
    stored_history: Vec<StoredScanSummary>,

    /// Currently selected scan for viewing
    selected_scan: Option<ScanRecord>,

    /// Comparison result when comparing two scans
    comparison_result: Option<ComparisonResult>,

    /// First scan ID selected for comparison
    compare_scan_id: Option<String>,

    /// History view mode
    history_view: HistoryView,

    /// Expanded tasks in scan detail view
    expanded_history_tasks: HashMap<String, bool>,

    // === AI State ===
    /// AI provider status (Phi Silica availability)
    ai_phi_silica_status: Option<PhiSilicaStatus>,

    /// Whether AI status check is in progress
    ai_status_checking: bool,

    /// Channel for receiving AI status updates
    ai_status_rx: Option<mpsc::Receiver<PhiSilicaStatus>>,

    /// Cached AI interpretations (task_id -> interpretation)
    ai_interpretations: HashMap<String, String>,

    /// AI analysis loading state (task_id -> loading)
    ai_loading: HashMap<String, bool>,

    /// AI analysis errors (task_id -> error message)
    ai_errors: HashMap<String, String>,

    /// Channel for receiving AI analysis results
    ai_analysis_rx: Option<mpsc::Receiver<AiAnalysisResult>>,

    /// Pending AI analysis requests
    ai_pending_requests: Vec<String>,

    // === AI Chat State ===
    /// Chat message history
    ai_chat_messages: Vec<AiChatMessage>,

    /// Current chat input
    ai_chat_input: String,

    /// Whether chat is waiting for response
    ai_chat_loading: bool,

    /// Chat error message
    ai_chat_error: Option<String>,

    /// Channel for receiving chat responses
    ai_chat_rx: Option<mpsc::Receiver<Result<String, String>>>,
}

/// AI chat message
#[derive(Clone)]
pub struct AiChatMessage {
    pub role: AiChatRole,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum AiChatRole {
    User,
    Assistant,
    System,
}

/// Result of an AI analysis request
#[derive(Clone)]
pub struct AiAnalysisResult {
    pub task_id: String,
    pub interpretation: Result<String, String>,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum HistoryView {
    #[default]
    List,
    Detail,
    Compare,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Diagnostics,
    Monitoring,
    Processes,
    Issues,
    History,
    AI,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ScanState {
    Idle,
    Running,
    Complete,
}

#[derive(Clone)]
pub enum ScanUpdate {
    Started,
    Progress { task_name: String, progress: f32 },
    TaskComplete { task_id: String, result: TaskResult },
    Complete,
    Error(String),
}

#[derive(Clone, Default)]
pub struct SystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub computer_name: String,
    pub is_admin: bool,
}

/// Get basic system information without async dependencies
fn get_basic_system_info() -> Option<SystemInfo> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;

        let product_name: String = key.get_value("ProductName").unwrap_or_else(|_| "Windows".to_string());
        let build: String = key.get_value("CurrentBuild").unwrap_or_else(|_| "Unknown".to_string());

        let computer_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string());

        // Check if running as admin
        let is_admin = is_elevated();

        Some(SystemInfo {
            os_name: product_name,
            os_version: format!("Build {}", build),
            computer_name,
            is_admin,
        })
    }

    #[cfg(not(windows))]
    {
        Some(SystemInfo {
            os_name: "Unknown OS".to_string(),
            os_version: "Unknown".to_string(),
            computer_name: "Unknown".to_string(),
            is_admin: false,
        })
    }
}

#[cfg(windows)]
fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token_handle = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle).is_ok() {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut return_length = 0u32;
            if GetTokenInformation(
                token_handle,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut return_length,
            ).is_ok() {
                return elevation.TokenIsElevated != 0;
            }
        }
        false
    }
}

#[cfg(not(windows))]
fn is_elevated() -> bool {
    false
}

#[derive(Clone)]
pub struct ScanSummary {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub task_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub ai_enabled: bool,
    pub openai_api_key: Option<String>,
    pub ai_provider: AiProvider,
    pub theme: Theme,
    pub quick_scan_tasks: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum AiProvider {
    #[default]
    Auto,      // Use Phi Silica if available, otherwise OpenAI
    PhiSilica, // Local on-device AI (Copilot+ PC)
    OpenAI,    // Cloud API
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ai_enabled: true,
            openai_api_key: None,
            ai_provider: AiProvider::Auto,
            theme: Theme::System,
            quick_scan_tasks: vec![
                "logical_disk".to_string(),
                "physical_memory".to_string(),
                "system_driver".to_string(),
                "services".to_string(),
                "processor".to_string(),
                "network_adapter".to_string(),
                "npu_info".to_string(),
            ],
        }
    }
}

impl Default for WfDiagApp {
    fn default() -> Self {
        let runtime = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
        let available_tasks = diagnostics::get_all_tasks();

        // Initialize expanded sections
        let mut expanded_sections = HashMap::new();
        expanded_sections.insert("Hardware".to_string(), true);
        expanded_sections.insert("System".to_string(), true);
        expanded_sections.insert("Storage".to_string(), true);
        expanded_sections.insert("Network".to_string(), true);

        // Get basic system info
        let system_info = get_basic_system_info();

        Self {
            runtime,
            available_tasks,
            results: Arc::new(Mutex::new(HashMap::new())),
            scan_state: ScanState::Idle,
            scan_rx: None,
            progress: 0.0,
            current_task: String::new(),
            selected_tab: Tab::Diagnostics,
            expanded_sections,
            expanded_rows: HashMap::new(),
            system_info,
            scan_history: Vec::new(),
            settings: Settings::default(),
            show_settings: false,
            show_about: false,
            status_message: "Ready".to_string(),
            scan_start_time: None,
            last_scan_duration: None,
            monitoring_state: ui::monitoring::MonitoringState::default(),
            monitoring_rx: None,
            monitoring_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            process_list_state: ui::process_list::ProcessListState::default(),
            all_processes: Vec::new(),
            show_full_process_list: false,
            scan_storage: None,
            stored_history: Vec::new(),
            selected_scan: None,
            comparison_result: None,
            compare_scan_id: None,
            history_view: HistoryView::default(),
            expanded_history_tasks: HashMap::new(),
            // AI state
            ai_phi_silica_status: None,
            ai_status_checking: false,
            ai_status_rx: None,
            ai_interpretations: HashMap::new(),
            ai_loading: HashMap::new(),
            ai_errors: HashMap::new(),
            ai_analysis_rx: None,
            ai_pending_requests: Vec::new(),
            // AI chat state
            ai_chat_messages: Vec::new(),
            ai_chat_input: String::new(),
            ai_chat_loading: false,
            ai_chat_error: None,
            ai_chat_rx: None,
        }
    }
}

impl WfDiagApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure fonts with full Unicode support
        let mut fonts = egui::FontDefinitions::default();

        #[cfg(windows)]
        {
            // Load Segoe UI for comprehensive Unicode coverage (includes box drawing, symbols, etc.)
            if let Ok(font_data) = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf") {
                fonts.font_data.insert(
                    "segoe_ui".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
            }

            // Load Segoe UI Symbol for additional symbols
            if let Ok(font_data) = std::fs::read("C:\\Windows\\Fonts\\seguisym.ttf") {
                fonts.font_data.insert(
                    "segoe_symbol".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
            }

            // Load Segoe UI Emoji for emoji characters
            if let Ok(font_data) = std::fs::read("C:\\Windows\\Fonts\\seguiemj.ttf") {
                fonts.font_data.insert(
                    "segoe_emoji".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
            }

            // Configure font families with fallback chain
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                // Insert Segoe UI at the beginning for primary text
                if fonts.font_data.contains_key("segoe_ui") {
                    family.insert(0, "segoe_ui".to_owned());
                }
                // Add symbol and emoji as fallbacks
                if fonts.font_data.contains_key("segoe_symbol") {
                    family.push("segoe_symbol".to_owned());
                }
                if fonts.font_data.contains_key("segoe_emoji") {
                    family.push("segoe_emoji".to_owned());
                }
            }

            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                // Add fallbacks for monospace too
                if fonts.font_data.contains_key("segoe_symbol") {
                    family.push("segoe_symbol".to_owned());
                }
                if fonts.font_data.contains_key("segoe_emoji") {
                    family.push("segoe_emoji".to_owned());
                }
            }
        }

        cc.egui_ctx.set_fonts(fonts);

        let mut app = Self::default();

        // Load saved settings from storage
        if let Some(storage) = cc.storage {
            if let Some(saved_settings) = eframe::get_value::<Settings>(storage, "settings") {
                app.settings = saved_settings;
            }
        }

        // Initialize scan storage and load history
        match ScanStorage::new() {
            Ok(storage) => {
                // Load existing scan history
                if let Ok(history) = storage.list_scans() {
                    app.stored_history = history;
                }
                app.scan_storage = Some(storage);
            }
            Err(e) => {
                eprintln!("Failed to initialize scan storage: {}", e);
            }
        }

        // Start monitoring immediately so Monitoring and Processes tabs are ready
        app.start_monitoring();

        app
    }

    /// Load scan history from storage
    pub fn load_history(&mut self) {
        if let Some(ref storage) = self.scan_storage {
            if let Ok(history) = storage.list_scans() {
                self.stored_history = history;
            }
        }
    }

    /// View a specific scan
    pub fn view_scan(&mut self, scan_id: &str) {
        if let Some(ref storage) = self.scan_storage {
            match storage.load_scan(scan_id) {
                Ok(scan) => {
                    self.selected_scan = Some(scan);
                    self.history_view = HistoryView::Detail;
                }
                Err(e) => {
                    eprintln!("Failed to load scan: {}", e);
                }
            }
        }
    }

    /// Start comparing scans - select first scan
    pub fn select_for_compare(&mut self, scan_id: &str) {
        self.compare_scan_id = Some(scan_id.to_string());
    }

    /// Compare two scans
    pub fn compare_scans(&mut self, scan_id: &str) {
        if let Some(ref first_id) = self.compare_scan_id.clone() {
            if let Some(ref storage) = self.scan_storage {
                match storage.compare_scans(scan_id, first_id) {
                    Ok(result) => {
                        self.comparison_result = Some(result);
                        self.history_view = HistoryView::Compare;
                        self.compare_scan_id = None;
                    }
                    Err(e) => {
                        eprintln!("Failed to compare scans: {}", e);
                    }
                }
            }
        }
    }

    /// Delete a scan from history
    pub fn delete_scan(&mut self, scan_id: &str) {
        if let Some(ref storage) = self.scan_storage {
            // Use clear_history for now since individual delete isn't exposed
            // TODO: Add a delete_scan method to ScanStorage
            eprintln!("Delete scan {} - individual delete not yet implemented", scan_id);
            self.load_history();
        }
    }

    /// Clear all history
    pub fn clear_history(&mut self) {
        if let Some(ref storage) = self.scan_storage {
            if let Err(e) = storage.clear_history() {
                eprintln!("Failed to clear history: {}", e);
            } else {
                self.stored_history.clear();
            }
        }
    }

    /// Save current scan to history
    pub fn save_current_scan(&mut self) {
        let results = self.results.lock().unwrap().clone();
        if results.is_empty() {
            return;
        }

        let system_info = self.system_info.clone().unwrap_or_default();
        let duration_ms = self.last_scan_duration.map(|d| d.as_millis() as u64).unwrap_or(0);

        let success_count = results.values().filter(|r| r.success).count();
        let failure_count = results.values().filter(|r| !r.success).count();

        let scan = ScanRecord {
            id: format!("scan_{}", chrono::Local::now().format("%Y%m%d_%H%M%S")),
            timestamp: wfdiag_tauri::timestamp::Timestamp::now(),
            computer_name: system_info.computer_name,
            os_version: format!("{} {}", system_info.os_name, system_info.os_version),
            is_admin: system_info.is_admin,
            results,
            task_count: success_count + failure_count,
            success_count,
            failure_count,
            duration_ms,
            tags: Vec::new(),
        };

        if let Some(ref storage) = self.scan_storage {
            if let Err(e) = storage.save_scan(&scan) {
                eprintln!("Failed to save scan: {}", e);
            } else {
                self.load_history();
            }
        }
    }

    pub fn start_scan(&mut self, quick: bool) {
        if self.scan_state == ScanState::Running {
            return;
        }

        self.scan_state = ScanState::Running;
        self.progress = 0.0;
        self.results.lock().unwrap().clear();
        self.scan_start_time = Some(std::time::Instant::now());

        let (tx, rx) = mpsc::channel(100);
        self.scan_rx = Some(rx);

        let tasks: Vec<DiagnosticTask> = if quick {
            self.available_tasks.iter()
                .filter(|t| self.settings.quick_scan_tasks.contains(&t.id))
                .cloned()
                .collect()
        } else {
            self.available_tasks.clone()
        };

        let results = self.results.clone();
        let runtime = self.runtime.clone();

        std::thread::spawn(move || {
            runtime.block_on(async move {
                let _ = tx.send(ScanUpdate::Started).await;

                let total = tasks.len();
                let batch_size = 5; // Run 5 tasks in parallel like Tauri version
                let mut completed = 0;

                for batch in tasks.chunks(batch_size) {
                    // Spawn all tasks in the batch concurrently
                    let mut handles = Vec::new();
                    for task in batch {
                        let task_id = task.id.clone();
                        let handle = tokio::spawn(async move {
                            let result = diagnostics::run_diagnostic_task(&task_id).await;
                            (task_id, result)
                        });
                        handles.push((task.name.clone(), handle));
                    }

                    // Wait for all tasks in batch to complete
                    for (task_name, handle) in handles {
                        completed += 1;
                        let progress = ((completed as f32) / (total as f32)) * 100.0;
                        let _ = tx.send(ScanUpdate::Progress {
                            task_name: task_name.clone(),
                            progress,
                        }).await;

                        if let Ok((task_id, result)) = handle.await {
                            results.lock().unwrap().insert(task_id.clone(), result.clone());
                            let _ = tx.send(ScanUpdate::TaskComplete {
                                task_id,
                                result,
                            }).await;
                        }
                    }
                }

                let _ = tx.send(ScanUpdate::Complete).await;
            });
        });
    }

    fn process_scan_updates(&mut self) -> bool {
        let mut had_updates = false;
        let mut should_save = false;

        if let Some(ref mut rx) = self.scan_rx {
            while let Ok(update) = rx.try_recv() {
                had_updates = true;
                match update {
                    ScanUpdate::Started => {
                        self.status_message = "Scanning...".to_string();
                    }
                    ScanUpdate::Progress { task_name, progress } => {
                        self.current_task = task_name;
                        self.progress = progress;
                    }
                    ScanUpdate::TaskComplete { .. } => {}
                    ScanUpdate::Complete => {
                        self.scan_state = ScanState::Complete;
                        self.progress = 100.0;
                        self.current_task.clear();

                        if let Some(start) = self.scan_start_time {
                            self.last_scan_duration = Some(start.elapsed());
                        }

                        let results = self.results.lock().unwrap();
                        let passed = results.values().filter(|r| r.success).count();
                        let failed = results.len() - passed;
                        self.status_message = format!("{} passed, {} failed", passed, failed);

                        // Add to in-memory history
                        self.scan_history.push(ScanSummary {
                            id: uuid::Uuid::new_v4().to_string(),
                            timestamp: chrono::Local::now(),
                            task_count: results.len(),
                            passed,
                            failed,
                            duration_ms: self.last_scan_duration.map(|d| d.as_millis() as u64).unwrap_or(0),
                        });

                        // Flag to save after loop (can't call self methods while rx is borrowed)
                        should_save = true;
                    }
                    ScanUpdate::Error(msg) => {
                        self.status_message = format!("Error: {}", msg);
                        self.scan_state = ScanState::Idle;
                    }
                }
            }
        }

        // Save to persistent storage after releasing rx borrow
        if should_save {
            self.save_current_scan();
        }

        had_updates
    }

    pub fn copy_results_to_clipboard(&self) {
        let results = self.results.lock().unwrap();
        let mut output = String::new();

        output.push_str("=== WFDiag Scan Results ===\n");
        output.push_str(&format!("Generated: {}\n\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));

        // Sort tasks by category for organized output
        let mut sorted_results: Vec<_> = results.iter().collect();
        sorted_results.sort_by(|(a_id, _), (b_id, _)| {
            let a_task = self.available_tasks.iter().find(|t| t.id == **a_id);
            let b_task = self.available_tasks.iter().find(|t| t.id == **b_id);
            let a_cat = a_task.map(|t| t.category.as_str()).unwrap_or("");
            let b_cat = b_task.map(|t| t.category.as_str()).unwrap_or("");
            a_cat.cmp(b_cat)
        });

        for (task_id, result) in sorted_results {
            let task = self.available_tasks.iter().find(|t| t.id == *task_id);
            let name = task.map(|t| t.name.as_str()).unwrap_or(task_id);
            let category = task.map(|t| t.category.as_str()).unwrap_or("Unknown");

            output.push_str(&format!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"));
            output.push_str(&format!("[{}] {} ({})\n",
                if result.success { "✓" } else { "✗" },
                name,
                category
            ));
            output.push_str(&format!("Duration: {}ms\n", result.duration_ms));
            output.push_str(&format!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"));

            if let Some(ref error) = result.error {
                output.push_str(&format!("Error: {}\n", error));
            }

            if !result.output.is_empty() {
                // Try to pretty-print JSON, otherwise show raw
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                        output.push_str(&pretty);
                    } else {
                        output.push_str(&result.output);
                    }
                } else {
                    output.push_str(&result.output);
                }
            }
            output.push_str("\n\n");
        }

        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(output);
        }
    }

    pub fn export_results(&self) -> Option<String> {
        let results = self.results.lock().unwrap();
        if results.is_empty() {
            return None;
        }

        // Build export data
        let mut export = serde_json::Map::new();
        export.insert("generated".to_string(), serde_json::json!(chrono::Local::now().to_rfc3339()));
        export.insert("version".to_string(), serde_json::json!(env!("CARGO_PKG_VERSION")));

        let mut tasks = serde_json::Map::new();
        for (task_id, result) in results.iter() {
            let task = self.available_tasks.iter().find(|t| t.id == *task_id);
            let mut task_data = serde_json::Map::new();

            task_data.insert("name".to_string(), serde_json::json!(task.map(|t| t.name.as_str()).unwrap_or(task_id)));
            task_data.insert("category".to_string(), serde_json::json!(task.map(|t| t.category.as_str()).unwrap_or("Unknown")));
            task_data.insert("success".to_string(), serde_json::json!(result.success));
            task_data.insert("duration_ms".to_string(), serde_json::json!(result.duration_ms));

            if let Some(ref error) = result.error {
                task_data.insert("error".to_string(), serde_json::json!(error));
            }

            // Parse output as JSON if possible
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&result.output) {
                task_data.insert("data".to_string(), json);
            } else {
                task_data.insert("output".to_string(), serde_json::json!(result.output));
            }

            tasks.insert(task_id.clone(), serde_json::Value::Object(task_data));
        }
        export.insert("results".to_string(), serde_json::Value::Object(tasks));

        serde_json::to_string_pretty(&serde_json::Value::Object(export)).ok()
    }

    pub fn clear_results(&mut self) {
        self.results.lock().unwrap().clear();
        self.scan_state = ScanState::Idle;
        self.status_message = "Ready".to_string();
        self.last_scan_duration = None;
    }

    pub fn start_monitoring(&mut self) {
        if self.monitoring_active.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        // Set active flags immediately so UI shows "Live" status
        self.monitoring_active.store(true, std::sync::atomic::Ordering::Relaxed);
        self.monitoring_state.is_active = true;

        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel(32);
            self.monitoring_rx = Some(rx);

            let active_flag = self.monitoring_active.clone();
            let runtime = self.runtime.clone();

            std::thread::spawn(move || {
                runtime.block_on(async move {
                    // Create monitoring state for stats collection
                    let pdh_state = Arc::new(std::sync::Mutex::new(wfdiag_tauri::native_monitor_helpers::PdhStateWrapper::default()));
                    let previous_network = Arc::new(tokio::sync::Mutex::new(wfdiag_tauri::native_monitor_helpers::NetworkStateWrapper::default()));
                    let previous_processes = Arc::new(tokio::sync::Mutex::new(wfdiag_tauri::native_monitor_helpers::ProcessCpuCache::new()));
                    let cached_disks = Arc::new(tokio::sync::Mutex::new(None));
                    let cached_npu_util = Arc::new(tokio::sync::Mutex::new(None));
                    let cached_fast_stats = Arc::new(tokio::sync::Mutex::new(None));
                    let disk_update_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let npu_update_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let fast_update_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let mut tick: u64 = 0;

                    loop {
                        if !active_flag.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }

                        // Collect stats using the standalone helper
                        let stats = wfdiag_tauri::native_monitor_helpers::collect_stats_standalone(
                            &pdh_state,
                            &previous_network,
                            &previous_processes,
                            &cached_disks,
                            &cached_npu_util,
                            &cached_fast_stats,
                            &disk_update_flag,
                            &npu_update_flag,
                            &fast_update_flag,
                            tick,
                        ).await;

                        tick += 1;

                        if tx.send(stats).await.is_err() {
                            break;
                        }

                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                });
            });
        }

        #[cfg(not(windows))]
        {
            self.status_message = "System monitoring is only available on Windows".to_string();
        }
    }

    pub fn stop_monitoring(&mut self) {
        self.monitoring_active.store(false, std::sync::atomic::Ordering::Relaxed);
        self.monitoring_state.is_active = false;
        self.monitoring_rx = None;
        self.monitoring_state.clear();
    }

    fn process_monitoring_updates(&mut self) -> bool {
        let mut had_updates = false;
        if let Some(ref mut rx) = self.monitoring_rx {
            while let Ok(stats) = rx.try_recv() {
                had_updates = true;
                // Store all processes for the full process list view
                self.all_processes = stats.all_processes.clone();
                self.monitoring_state.update_stats(stats);
            }
        }
        had_updates
    }
}

impl eframe::App for WfDiagApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "settings", &self.settings);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        match self.settings.theme {
            Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            Theme::System => {
                // Detect system theme
                let is_dark = ctx.style().visuals.dark_mode;
                // On first run or if we want to follow system, check OS preference
                #[cfg(windows)]
                {
                    use winreg::enums::*;
                    use winreg::RegKey;
                    if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER)
                        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
                    {
                        if let Ok(val) = hkcu.get_value::<u32, _>("AppsUseLightTheme") {
                            let system_is_dark = val == 0;
                            if system_is_dark != is_dark {
                                if system_is_dark {
                                    ctx.set_visuals(egui::Visuals::dark());
                                } else {
                                    ctx.set_visuals(egui::Visuals::light());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process any pending scan updates
        let had_scan_updates = self.process_scan_updates();

        // Process any pending monitoring updates
        let had_monitoring_updates = self.process_monitoring_updates();

        // Process AI status and analysis updates
        ui::ai::process_ai_status_updates(self);
        ui::ai::process_ai_analysis_updates(self);

        // Request repaints based on active states
        let has_ai_pending = self.ai_status_checking || !self.ai_loading.is_empty() || self.ai_chat_loading;
        if self.scan_state == ScanState::Running {
            // Use a slower repaint rate to reduce flashing (100ms = 10 FPS during scan)
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.monitoring_state.is_active {
            // Continuous repaint for monitoring (1 second intervals)
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        } else if has_ai_pending {
            // Repaint for AI updates
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else if had_scan_updates || had_monitoring_updates {
            // Single repaint after updates complete
            ctx.request_repaint();
        }

        // Top toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui::toolbar::show(self, ui);
        });

        // Status bar
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui::statusbar::show(self, ui);
        });

        // Left navigation
        egui::SidePanel::left("nav").exact_width(48.0).show(ctx, |ui| {
            ui::nav::show(self, ui);
        });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.selected_tab {
                Tab::Diagnostics => ui::diagnostics::show(self, ui),
                Tab::Monitoring => ui::monitoring::show(self, ui),
                Tab::Processes => ui::processes::show(self, ui),
                Tab::Issues => ui::issues::show(self, ui),
                Tab::History => ui::history::show(self, ui),
                Tab::AI => ui::ai_analysis::show(self, ui),
            }
        });

        // Settings window
        if self.show_settings {
            ui::settings::show(self, ctx);
        }

        // About window
        if self.show_about {
            ui::about::show(self, ctx);
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "WFDiag - System Diagnostics",
        options,
        Box::new(|cc| Ok(Box::new(WfDiagApp::new(cc)))),
    )
}
