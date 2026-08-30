//! Serializable messages crossing the application-core/UI boundary.

use serde::{Deserialize, Serialize};

/// A framework-neutral event consumed by a UI shell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum UiEvent {
    TaskProgress(TaskProgress),
    DiagnosticResult(DiagnosticTaskResult),
    SystemStats(SystemStats),
    Chat(ChatEvent),
    Report(ReportEvent),
    ActionStatus(ActionStatus),
    QuickScan(QuickScanRequest),
}

/// Complete output for one diagnostic task.
///
/// Unlike replaceable running progress, results are delivered through the
/// lossless event lane. The native shell therefore receives the same evidence
/// that the legacy Tauri command returns when a scan finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticTaskResult {
    pub session_id: String,
    pub task_id: String,
    pub success: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl UiEvent {
    /// Whether the event must use the bounded, lossless FIFO.
    ///
    /// System snapshots and nonterminal progress are the only replaceable
    /// values. A terminal task event is always lossless.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        !matches!(
            self,
            Self::SystemStats(_)
                | Self::TaskProgress(TaskProgress {
                    status: TaskProgressStatus::Queued | TaskProgressStatus::Running,
                    ..
                })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskProgressStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskProgressStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Progress for one diagnostic task.
///
/// Field spelling intentionally matches the existing `task-progress` Tauri
/// payload so the temporary compatibility adapter can serialize it unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub session_id: String,
    pub task_id: String,
    pub status: TaskProgressStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

impl TaskProgress {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    #[must_use]
    pub fn same_task(&self, other: &Self) -> bool {
        self.session_id == other.session_id && self.task_id == other.task_id
    }
}

/// A lightweight process projection carried by live monitoring snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessStats {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub memory_mb: f64,
    pub gpu_percent: f32,
    pub npu_percent: f32,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskStats {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
    pub utilization: f32,
    pub file_system: String,
    pub disk_type: String,
}

/// Latest live system-monitoring snapshot.
///
/// This is intentionally a UI projection: the backend-only full process list
/// never enters the event contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemStats {
    pub cpu_utilization: f32,
    pub per_cpu_utilization: Vec<f32>,
    pub cpu_frequency: u64,
    pub memory_total_gb: f64,
    pub memory_used_gb: f64,
    pub memory_available_gb: f64,
    pub memory_utilization: f32,
    pub swap_total_gb: f64,
    pub swap_used_gb: f64,
    pub swap_utilization: f32,
    pub storage_used_percent: f32,
    /// Compatibility alias retained while the Tauri shell coexists.
    pub disk_utilization: f32,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disks: Vec<DiskStats>,
    pub network_upload_kb: f64,
    pub network_download_kb: f64,
    pub gpu_available: bool,
    pub gpu_name: Option<String>,
    pub gpu_utilization: Option<f32>,
    pub gpu_memory_used_mb: f64,
    pub gpu_memory_total_mb: f64,
    pub npu_available: bool,
    pub npu_name: Option<String>,
    pub npu_utilization: Option<f32>,
    pub npu_memory_used_mb: f64,
    pub npu_memory_total_mb: f64,
    pub top_processes: Vec<ProcessStats>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionClass {
    OnDevice,
    LocalServer,
    SubscriptionCloud,
    ApiCloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUse {
    pub provider_id: String,
    pub execution_class: ProviderExecutionClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actual_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ChatEvent {
    Delta(ChatDelta),
    Tool(ChatTool),
    Done(ChatDone),
    Error(ChatError),
    FallbackRequired(ChatFallbackRequired),
    Proposal(ChatProposal),
    ScanRequest(ChatScanRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDelta {
    pub session_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatToolStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTool {
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: ChatToolStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatFinishReason {
    Stop,
    Length,
    Refusal,
    Cancelled,
    ToolBudget,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDone {
    pub session_id: String,
    pub message_id: String,
    pub finish_reason: ChatFinishReason,
    pub provider: String,
    pub provider_use: ProviderUse,
    pub tool_call_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatError {
    pub session_id: String,
    pub message_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatFallbackRequired {
    pub session_id: String,
    pub message_id: String,
    pub from: ProviderUse,
    pub to: ProviderUse,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProposal {
    pub session_id: String,
    pub message_id: String,
    pub proposal: ActionProposal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatScanRequest {
    pub session_id: String,
    pub message_id: String,
    pub source_scan_id: String,
    pub kind: ScanRequestKind,
    pub reason: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanRequestKind {
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ReportEvent {
    Delta(ReportDelta),
    Done(ReportDone),
    Error(ReportError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDelta {
    pub report_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDone {
    pub report_id: String,
    pub finish_reason: ChatFinishReason,
    pub provider: String,
    pub provider_use: ProviderUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportError {
    pub report_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Exact,
    Batch,
}

pub use wfdiag_remediation_catalog::{RemediationSummary, RemediationTier};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPreview {
    pub remediation: RemediationSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionProposal {
    pub proposal_id: String,
    pub approval_scope: ApprovalScope,
    pub actions: Vec<ActionPreview>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionItemStatus {
    Pending,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRunStatus {
    Running,
    CancelRequested,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

impl ActionRunStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixCompletionStatus {
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationStepStatus {
    Succeeded,
    AlreadySatisfied,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationStepResult {
    pub action: String,
    pub status: RemediationStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixResult {
    pub success: bool,
    pub message: String,
    pub actions_taken: Vec<String>,
    pub requires_restart: bool,
    pub completion_status: FixCompletionStatus,
    #[serde(default)]
    pub steps: Vec<RemediationStepResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemRun {
    pub remediation_id: String,
    pub label: String,
    pub cancellable: bool,
    pub status: ActionItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<FixResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatus {
    pub run_id: String,
    pub proposal_id: String,
    pub authorization_id: String,
    pub status: ActionRunStatus,
    pub actions: Vec<ActionItemRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_index: Option<usize>,
    pub approved_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickScanSource {
    Tray,
    CommandPalette,
    GlobalShortcut,
    ExternalActivation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickScanRequest {
    pub request_id: String,
    pub requested_at_ms: u64,
    pub source: QuickScanSource,
}
