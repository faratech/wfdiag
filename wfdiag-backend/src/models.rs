use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiagnosticTask {
    pub id: String,
    pub name: String,
    pub description: String,
    pub admin_required: bool,
    pub category: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticRequest {
    pub selected_tasks: Vec<String>,
    pub output_format: Option<OutputFormat>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Zip,
    Both,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Both
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiagnosticSession {
    pub id: Uuid,
    pub status: SessionStatus,
    pub progress: f32,
    pub current_task: Option<String>,
    pub completed_tasks: usize,
    pub total_tasks: usize,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output_path: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProgressUpdate {
    pub session_id: Uuid,
    pub progress: f32,
    pub status: SessionStatus,
    pub current_task: Option<String>,
    pub message: String,
    pub completed_tasks: usize,
    pub total_tasks: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_name: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub session_id: Uuid,
    pub system_info: SystemInfo,
    pub tasks_results: Vec<TaskResult>,
    pub summary: ReportSummary,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os_version: String,
    pub computer_name: String,
    pub username: String,
    pub is_admin: bool,
    pub cpu_info: String,
    pub total_memory_gb: f64,
    pub available_memory_gb: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReportSummary {
    pub total_tasks: usize,
    pub successful_tasks: usize,
    pub failed_tasks: usize,
    pub total_duration_seconds: f64,
    pub warnings: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}