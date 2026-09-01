//! The facade's read-only chat tool backend.
//!
//! The tool registry is **strictly read-only**. `run_diagnostic` executes one
//! catalog task and returns its text; nothing else touches the system, and the
//! two tools that look like actions —`request_full_scan` and
//! `stage_remediation` — deliberately return a typed envelope for the host to
//! act on rather than starting a scan or running a remediation themselves.
//!
//! The evidence a turn reads is captured once, on the service's thread, into
//! [`ChatToolSnapshot`]. The worker may read that value but holds no reference
//! back into service state, which is what lets a turn keep answering about the
//! scan it was asked about even while a newer scan replaces it.

use std::sync::Arc;
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::tools::{
    IssueText, IssueTextSeverity, IssueTextStatus, ScanText, ScanTextKind, StageableIssue,
    StageableRemediation, TaskResultText, detected_issues_text, request_full_scan_envelope,
    scan_coverage, scan_summary_text, stage_remediation_envelope, tool_catalog,
};
use wfdiag_native_ai_chat::{
    BoundedToolBackend, BoundedToolCatalog, BoundedToolOperation, ChatTurnTools, ScanCoverage,
    ToolFuture, search_windows_knowledge, truncate_output,
};
use wfdiag_native_diagnostics::{DiagnosticExecutor, ScanKind};
use wfdiag_native_history::{NativeHistoryRuntime, ScanRecord, ScanSummary, TaskChange};
use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, RemediationSummary};
use wfdiag_ui_core::{DiagnosticTaskResult, SystemStats};

/// The scan half of one turn's evidence.
#[derive(Clone, Debug)]
pub struct ChatScanSnapshot {
    /// The session the evidence came from.
    pub session_id: String,
    /// When the scan started, for the age the model is told about.
    pub started_at: SystemTime,
    /// Which scan produced it.
    pub scan_kind: ScanKind,
    /// The tasks the scan selected.
    pub selected_tasks: Vec<String>,
    /// The results collected so far.
    pub results: Vec<DiagnosticTaskResult>,
    /// Whether the scan is still running.
    pub running: bool,
}

/// Everything one chat turn may read, captured once.
#[derive(Clone, Debug, Default)]
pub struct ChatToolSnapshot {
    /// Host identity, rendered for the model.
    pub system_overview: Option<String>,
    /// The committed or in-flight scan.
    pub scan: Option<ChatScanSnapshot>,
    /// The current scan projected into history form, used only as the left
    /// side of a comparison. The history worker picks the right side.
    pub current_history_scan: Option<ScanRecord>,
    /// The current issue projection.
    pub issues: Vec<Issue>,
    /// Stored scans, as a fallback when the history worker is unavailable.
    pub history: Vec<ScanSummary>,
    /// The newest telemetry sample.
    pub live_stats: Option<SystemStats>,
    /// The remediation catalog, for staging.
    pub remediations: Vec<RemediationSummary>,
    /// Whether the user enabled live network grounding.
    pub network_grounding_enabled: bool,
}

/// The read-only platform ports the tools reach through.
#[derive(Clone)]
pub struct ChatToolPorts {
    diagnostics: Arc<dyn DiagnosticExecutor>,
    history: Option<Arc<NativeHistoryRuntime>>,
}

impl std::fmt::Debug for ChatToolPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChatToolPorts")
            .field("history", &self.history.is_some())
            .finish_non_exhaustive()
    }
}

impl ChatToolPorts {
    /// Bind the tool backend to a diagnostic executor and, when one started, a
    /// history runtime.
    #[must_use]
    pub fn new(
        diagnostics: Arc<dyn DiagnosticExecutor>,
        history: Option<Arc<NativeHistoryRuntime>>,
    ) -> Self {
        Self {
            diagnostics,
            history,
        }
    }
}

/// Binds the facade's evidence snapshot to the chat crate's tool surface.
pub struct AppChatTools {
    ports: ChatToolPorts,
}

impl AppChatTools {
    /// Bind the tool surface to `ports`.
    #[must_use]
    pub const fn new(ports: ChatToolPorts) -> Self {
        Self { ports }
    }
}

impl ChatTurnTools for AppChatTools {
    type Evidence = ChatToolSnapshot;

    fn catalog(&self, evidence: &Self::Evidence) -> BoundedToolCatalog {
        let tasks = self.ports.diagnostics.available_tasks();
        tool_catalog(
            tasks
                .iter()
                .map(|task| (task.id.as_str(), task.description.as_str())),
            evidence
                .remediations
                .iter()
                .map(|remediation| remediation.id.as_str()),
        )
    }

    fn backend(
        &self,
        evidence: &Self::Evidence,
        max_result_chars: usize,
    ) -> Box<dyn BoundedToolBackend> {
        Box::new(AppChatToolBackend {
            snapshot: evidence.clone(),
            ports: self.ports.clone(),
            max_result_chars,
        })
    }

    fn prompt_evidence(&self, evidence: &Self::Evidence) -> String {
        format!(
            "{}\n\n{}",
            snapshot_scan_summary_text(evidence),
            snapshot_detected_issues_text(evidence)
        )
    }

    fn network_grounding_enabled(&self, evidence: &Self::Evidence) -> bool {
        evidence.network_grounding_enabled
    }

    fn scan_coverage(&self, evidence: &Self::Evidence) -> ScanCoverage {
        let scan = evidence.scan.as_ref().map(scan_text);
        scan_coverage(scan.as_ref())
    }

    fn scan_session_id(&self, evidence: &Self::Evidence) -> String {
        evidence
            .scan
            .as_ref()
            .map_or_else(String::new, |scan| scan.session_id.clone())
    }
}

/// The read-only backend behind one turn's validated tool operations.
pub struct AppChatToolBackend {
    snapshot: ChatToolSnapshot,
    ports: ChatToolPorts,
    max_result_chars: usize,
}

impl AppChatToolBackend {
    /// Build a backend directly, for tests that exercise the tool surface
    /// without a chat runtime.
    #[must_use]
    pub const fn new(
        snapshot: ChatToolSnapshot,
        ports: ChatToolPorts,
        max_result_chars: usize,
    ) -> Self {
        Self {
            snapshot,
            ports,
            max_result_chars,
        }
    }
}

impl BoundedToolBackend for AppChatToolBackend {
    #[allow(clippy::elidable_lifetime_names)] // Mirrors the trait's own signature.
    fn execute<'a>(
        &'a self,
        operation: BoundedToolOperation,
        cancel: CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            match operation {
                BoundedToolOperation::RunDiagnostic { task_id, .. } => {
                    let execution = self.ports.diagnostics.execute(task_id.clone());
                    let output = tokio::select! {
                        biased;
                        () = cancel.cancelled() => return Err("Diagnostic cancelled".to_string()),
                        output = execution => output,
                    };
                    if output.success {
                        Ok(bounded_readable_text(&output.output, self.max_result_chars))
                    } else {
                        Ok(truncate_output(
                            &format!(
                                "Task {task_id} COLLECTION ERROR: {}",
                                output.error.unwrap_or_else(|| "unknown error".to_string())
                            ),
                            self.max_result_chars,
                        ))
                    }
                }
                BoundedToolOperation::SearchWindowsKnowledge { query } => {
                    if self.snapshot.network_grounding_enabled {
                        search_windows_knowledge(&query, self.max_result_chars, &cancel).await
                    } else {
                        Err("Network grounding is disabled in Settings".to_string())
                    }
                }
                BoundedToolOperation::GetScanSummary => Ok(truncate_output(
                    &snapshot_scan_summary_text(&self.snapshot),
                    self.max_result_chars,
                )),
                BoundedToolOperation::RequestFullScan { reason } => {
                    let scan = self.snapshot.scan.as_ref().map(scan_text);
                    request_full_scan_envelope(scan_coverage(scan.as_ref()), &reason)
                }
                BoundedToolOperation::GetDetectedIssues => Ok(truncate_output(
                    &snapshot_detected_issues_text(&self.snapshot),
                    self.max_result_chars,
                )),
                BoundedToolOperation::CompareWithPreviousScan => {
                    self.compare_with_previous_scan(&cancel).await
                }
                BoundedToolOperation::GetLiveStats => Ok(self
                    .snapshot
                    .live_stats
                    .as_ref()
                    .map_or_else(
                        || "Monitoring is not running — live values unavailable. Suggest the user open the Monitor tab to start it.".to_string(),
                        |stats| {
                            bounded_readable_text(
                                &serde_json::to_string(stats).unwrap_or_default(),
                                self.max_result_chars,
                            )
                        },
                    )),
                BoundedToolOperation::ListRemediations => Ok(truncate_output(
                    &remediation_list_text(&self.snapshot.remediations),
                    self.max_result_chars,
                )),
                BoundedToolOperation::ListScanHistory => self.list_scan_history(&cancel).await,
                BoundedToolOperation::StageRemediation {
                    remediation_id,
                    issue_id,
                } => stage_remediation_envelope(
                    &stageable_remediations(&self.snapshot.remediations),
                    &stageable_detected_issues(&self.snapshot.issues),
                    &remediation_id,
                    issue_id.as_deref(),
                ),
            }
        })
    }
}

impl AppChatToolBackend {
    async fn compare_with_previous_scan(
        &self,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        let Some(current) = self.snapshot.current_history_scan.clone() else {
            return Ok(
                "Comparison is unavailable because the current scan has not been projected into history format."
                    .to_string(),
            );
        };
        let Some(history) = self.ports.history.as_ref() else {
            return Ok("Scan history storage is unavailable on this system.".to_string());
        };
        let comparison = history
            .request_compare_current_to_latest(Arc::new(current))
            .map_err(|error| error.to_string())?;
        let comparison = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("History comparison cancelled".to_string()),
            result = comparison => result.map_err(|_| "Scan history worker stopped".to_string())??,
        };
        let Some(comparison) = comparison else {
            return Ok(
                "No different stored scan is available to compare with the current scan."
                    .to_string(),
            );
        };
        let task_ids = |changes: &[TaskChange]| {
            changes
                .iter()
                .map(|change| change.task_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        Ok(truncate_output(
            &format!(
                "Compared {} (current) with {} (previous): {} change(s).\nNew collection errors: {}\nNewly collected: {}",
                comparison.current_scan.timestamp,
                comparison.previous_scan.timestamp,
                comparison.total_changes,
                if comparison.new_failures.is_empty() {
                    "none".to_string()
                } else {
                    task_ids(&comparison.new_failures)
                },
                if comparison.new_successes.is_empty() {
                    "none".to_string()
                } else {
                    task_ids(&comparison.new_successes)
                },
            ),
            self.max_result_chars,
        ))
    }

    async fn list_scan_history(&self, cancel: &CancellationToken) -> Result<String, String> {
        let scans = if let Some(history) = self.ports.history.as_ref() {
            let reply = history.request_list().map_err(|error| error.to_string())?;
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err("Scan history request cancelled".to_string()),
                result = reply => result.map_err(|_| "Scan history worker stopped".to_string())??,
            }
        } else {
            self.snapshot.history.clone()
        };
        Ok(truncate_output(
            &scan_history_text(&scans),
            self.max_result_chars,
        ))
    }
}

fn scan_text(scan: &ChatScanSnapshot) -> ScanText<'_> {
    ScanText {
        kind: match scan.scan_kind {
            ScanKind::Quick => ScanTextKind::Quick,
            ScanKind::Full => ScanTextKind::Full,
            ScanKind::Targeted => ScanTextKind::Targeted,
        },
        running: scan.running,
        selected_tasks: scan.selected_tasks.len(),
        age_minutes: scan
            .started_at
            .elapsed()
            .map_or(0, |duration| duration.as_secs() / 60),
        results: scan
            .results
            .iter()
            .map(|result| TaskResultText {
                task_id: &result.task_id,
                success: result.success,
                output: &result.output,
                error: result.error.as_deref(),
            })
            .collect(),
    }
}

fn snapshot_scan_summary_text(snapshot: &ChatToolSnapshot) -> String {
    let scan = snapshot.scan.as_ref().map(scan_text);
    scan_summary_text(snapshot.system_overview.as_deref(), scan.as_ref())
}

fn snapshot_detected_issues_text(snapshot: &ChatToolSnapshot) -> String {
    detected_issues_text(&issue_texts(&snapshot.issues))
}

fn issue_texts(issues: &[Issue]) -> Vec<IssueText<'_>> {
    issues
        .iter()
        .map(|issue| IssueText {
            id: &issue.id,
            remediation_id: issue
                .remediation
                .as_ref()
                .map(|remediation| remediation.id.as_str()),
            severity: match issue.severity {
                IssueSeverity::Critical => IssueTextSeverity::Critical,
                IssueSeverity::Warning => IssueTextSeverity::Warning,
                IssueSeverity::Info => IssueTextSeverity::Info,
                IssueSeverity::Ok => IssueTextSeverity::Ok,
            },
            status: match issue.status {
                IssueStatus::Detected => IssueTextStatus::Detected,
                IssueStatus::Unknown | IssueStatus::Skipped => IssueTextStatus::Unverified,
                IssueStatus::Ok => IssueTextStatus::Ok,
            },
            title: &issue.title,
            description: &issue.description,
            recommendation: &issue.recommendation,
        })
        .collect()
}

fn stageable_remediations(remediations: &[RemediationSummary]) -> Vec<StageableRemediation<'_>> {
    remediations
        .iter()
        .map(|remediation| StageableRemediation {
            id: &remediation.id,
            label: &remediation.label,
            maintenance: remediation.maintenance,
        })
        .collect()
}

fn stageable_detected_issues(issues: &[Issue]) -> Vec<StageableIssue<'_>> {
    issues
        .iter()
        .filter(|issue| issue.status == IssueStatus::Detected)
        .map(|issue| StageableIssue {
            id: &issue.id,
            remediation_id: issue
                .remediation
                .as_ref()
                .map(|remediation| remediation.id.as_str()),
        })
        .collect()
}

fn bounded_readable_text(value: &str, max_chars: usize) -> String {
    let rendered = serde_json::from_str::<serde_json::Value>(value).map_or_else(
        |_| value.to_string(),
        |parsed| serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| value.to_string()),
    );
    truncate_output(&rendered, max_chars)
}

fn scan_history_text(scans: &[ScanSummary]) -> String {
    if scans.is_empty() {
        return "No stored scans yet.".to_string();
    }
    scans
        .iter()
        .take(10)
        .map(|scan| {
            format!(
                "{} | {} | {} collected / {} collection errors{}",
                scan.id,
                scan.timestamp,
                scan.success_count,
                scan.failure_count,
                if scan.tags.is_empty() {
                    String::new()
                } else {
                    format!(" | tags: {}", scan.tags.join(", "))
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn remediation_list_text(remediations: &[RemediationSummary]) -> String {
    if remediations.is_empty() {
        return "No vetted remediations are available.".to_string();
    }
    remediations
        .iter()
        .map(|remediation| {
            format!(
                "{} | {} | {:?}{} | {}",
                remediation.id,
                remediation.label,
                remediation.tier,
                if remediation.admin_required {
                    " | admin"
                } else {
                    ""
                },
                remediation.description,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
