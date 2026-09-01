//! Reactor's platform wiring for the shared native AI chat runtime.
//!
//! [`wfdiag_native_ai_chat::NativeChatRuntime`] owns the worker thread, the
//! conversation, cancellation, and every event this shell drains. What stays
//! here is the part that genuinely needs Windows and the shell's own domain
//! types: on-device Phi resolution, the read-only diagnostics/history ports
//! behind the bounded tools, and the projection of Reactor's evidence snapshot
//! onto the crate's neutral tool views.

#![deny(unsafe_code)]

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::SystemTime;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::tools::{
    IssueText, IssueTextSeverity, IssueTextStatus, ScanText, ScanTextKind, StageableIssue,
    StageableRemediation, TaskResultText, detected_issues_text, request_full_scan_envelope,
    scan_coverage, scan_summary_text, stage_remediation_envelope, tool_catalog,
};
use wfdiag_native_ai_chat::{
    BoundedToolBackend, BoundedToolCatalog, BoundedToolOperation, ChatProvider, ChatResolveFuture,
    ChatTurnTools, ChatWorkerEvent, CompatChatProvider, ProviderResolver, ResolvedChatProvider,
    ScanCoverage, ToolFuture, search_windows_knowledge, truncate_output,
};
use wfdiag_native_ai_provider::{
    AIProvider, CompatConfigPorts, FoundryEndpointSource, OllamaSource,
    ProcessSubscriptionCliStatusSource, SettingsProviderKeySource, SubscriptionCliStatusSource,
    SubscriptionConfigPorts, provider_config_fingerprint, resolve_compat_config,
    resolve_subscription_config,
};
use wfdiag_native_diagnostics::{DiagnosticExecutor, NativeDiagnosticExecutor, ScanKind};
use wfdiag_native_history::{NativeHistoryRuntime, ScanRecord, ScanSummary};
use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, RemediationSummary};
use wfdiag_native_phi::PhiChatProvider;
use wfdiag_native_settings::SettingsService;
use wfdiag_ui_core::{DiagnosticTaskResult, SystemStats};

use crate::ui_wake_support;

/// The chat runtime as this shell configures it.
pub type NativeChatRuntime = wfdiag_native_ai_chat::NativeChatRuntime<ReactorChatTools>;

/// Immutable evidence captured by the component for one user turn. The tool
/// worker may read this value but has no reference back to mutable UI state.
#[derive(Clone, Debug, Default)]
pub struct ChatToolSnapshot {
    pub system_overview: Option<String>,
    pub scan: Option<ChatScanSnapshot>,
    /// Live current-scan projection used only as the left side of a history
    /// comparison. The history worker selects the newest stored different id.
    pub current_history_scan: Option<ScanRecord>,
    pub issues: Vec<Issue>,
    pub history: Vec<ScanSummary>,
    pub live_stats: Option<SystemStats>,
    pub remediations: Vec<RemediationSummary>,
    pub network_grounding_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ChatScanSnapshot {
    pub session_id: String,
    pub started_at: SystemTime,
    pub scan_kind: ScanKind,
    pub selected_tasks: Vec<String>,
    pub results: Vec<DiagnosticTaskResult>,
    pub running: bool,
}

/// Read-only platform ports owned by the Reactor adapter.
#[derive(Clone)]
pub struct ChatToolPorts {
    diagnostics: Arc<dyn DiagnosticExecutor>,
    history: Option<Arc<NativeHistoryRuntime>>,
}

impl ChatToolPorts {
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

    #[must_use]
    pub fn shipping(history: Option<Arc<NativeHistoryRuntime>>) -> Self {
        Self::new(Arc::new(NativeDiagnosticExecutor), history)
    }
}

/// Credential and endpoint ports for one resolved turn. Rebuilt per turn so
/// settings edits and saved keys apply to the very next message. Shared with
/// the report worker, which resolves the same provider set.
pub(crate) struct ShellChatSource {
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    subscription: Arc<dyn SubscriptionCliStatusSource>,
}

impl ShellChatSource {
    pub(crate) fn new(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
    ) -> Self {
        Self {
            settings,
            foundry,
            ollama,
            subscription: Arc::new(ProcessSubscriptionCliStatusSource::new()),
        }
    }

    pub(crate) fn ports(&self) -> CompatConfigPorts {
        CompatConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            keys: Arc::new(SettingsProviderKeySource(self.settings.clone())),
            foundry: Arc::clone(&self.foundry),
            ollama: Arc::clone(&self.ollama),
        }
    }

    pub(crate) fn subscription_ports(&self) -> SubscriptionConfigPorts {
        SubscriptionConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            status: Arc::clone(&self.subscription),
        }
    }
}

/// Provider resolution for one turn. Phi Silica is resolved in-process and is
/// the only provider that needs the turn's cancellation token, so it can stop
/// generating on the NPU; every other provider resolves through the shared
/// compat/subscription configuration.
struct ShellChatResolver {
    source: ShellChatSource,
}

impl ProviderResolver for ShellChatResolver {
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_> {
        Box::pin(async move {
            if provider == AIProvider::PhiSilica {
                let cancelled = cancel.clone();
                return Ok(ResolvedChatProvider {
                    chat: Arc::new(PhiChatProvider::new(move || cancelled.is_cancelled())),
                    config_fingerprint: "provider=phi_silica;runtime=windows_ai".to_string(),
                    requested_model: None,
                });
            }
            if provider == AIProvider::None {
                return Err("No AI provider is available".to_string());
            }
            let resolution = async {
                match provider {
                    AIProvider::CodexCli | AIProvider::ClaudeCode => {
                        let ports = self.source.subscription_ports();
                        resolve_subscription_config(provider, &ports).await
                    }
                    _ => {
                        let ports = self.source.ports();
                        resolve_compat_config(provider, &ports).await
                    }
                }
            };
            let cfg = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err("AI request cancelled".to_string()),
                cfg = resolution => cfg?,
            };
            let requested_model = cfg.model.clone();
            let config_fingerprint = provider_config_fingerprint(provider, &cfg);
            let chat: Arc<dyn ChatProvider> = Arc::new(CompatChatProvider { provider, cfg });
            Ok(ResolvedChatProvider {
                chat,
                config_fingerprint,
                requested_model,
            })
        })
    }
}

/// Binds Reactor's evidence snapshot to the crate's neutral tool surface.
pub struct ReactorChatTools {
    ports: ChatToolPorts,
}

impl ChatTurnTools for ReactorChatTools {
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
        Box::new(ReactorToolBackend {
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

/// Start the chat worker with this shell's Phi/diagnostics/history wiring.
///
/// # Errors
/// When the worker thread cannot be spawned.
pub fn start_chat_runtime(
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    tools: ChatToolPorts,
) -> std::io::Result<(NativeChatRuntime, std_mpsc::Receiver<ChatWorkerEvent>)> {
    NativeChatRuntime::start(
        ShellChatResolver {
            source: ShellChatSource::new(settings, foundry, ollama),
        },
        ReactorChatTools { ports: tools },
        Arc::new(ui_wake_support::notify),
    )
}

struct ReactorToolBackend {
    snapshot: ChatToolSnapshot,
    ports: ChatToolPorts,
    max_result_chars: usize,
}

impl BoundedToolBackend for ReactorToolBackend {
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
                    if !self.snapshot.network_grounding_enabled {
                        Err("Network grounding is disabled in Settings".to_string())
                    } else {
                        search_windows_knowledge(&query, self.max_result_chars, &cancel).await
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
                BoundedToolOperation::GetLiveStats => Ok(self.snapshot.live_stats.as_ref().map_or_else(
                    || "Monitoring is not running — live values unavailable. Suggest the user open the Monitor tab to start it.".to_string(),
                    |stats| bounded_readable_text(
                        &serde_json::to_string(stats).unwrap_or_default(),
                        self.max_result_chars,
                    ),
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

impl ReactorToolBackend {
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
            .request_compare_current_to_latest(std::sync::Arc::new(current))
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
        let task_ids = |changes: &[wfdiag_native_history::TaskChange]| {
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

/// Project one Reactor scan snapshot onto the crate's neutral scan view.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wfdiag_native_ai_chat::{BoundedToolExecutor, ToolCall, ToolExecutor};
    use wfdiag_native_diagnostics::{DiagnosticFuture, DiagnosticOutput, DiagnosticTask};
    use wfdiag_native_history::HistoryRuntimeConfig;

    #[derive(Default)]
    struct FixedDiagnostics {
        calls: AtomicUsize,
    }

    impl DiagnosticExecutor for FixedDiagnostics {
        fn available_tasks(&self) -> Vec<DiagnosticTask> {
            vec![DiagnosticTask {
                id: "os_info".to_string(),
                name: "Operating system".to_string(),
                description: "Collect Windows version information".to_string(),
                category: "System".to_string(),
                admin_required: false,
            }]
        }

        fn execute(&self, task_id: String) -> DiagnosticFuture<'_> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                DiagnosticOutput {
                    success: true,
                    output: format!(r#"{{"task":"{task_id}","build":26100}}"#),
                    error: None,
                    duration_ms: 4,
                }
            })
        }
    }

    fn disk_cleanup() -> RemediationSummary {
        wfdiag_native_issues::remediation_summaries()
            .into_iter()
            .find(|summary| summary.id == "open_disk_cleanup")
            .expect("canonical remediation must exist")
    }

    fn low_disk_issue(remediation: RemediationSummary) -> Issue {
        Issue {
            id: "low_disk_space".to_string(),
            category: "Storage".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: "Low disk space".to_string(),
            description: "C: has little free space".to_string(),
            recommendation: "Review Disk Cleanup".to_string(),
            detected: true,
            source_tasks: Some(vec!["disk_space".to_string()]),
            remediation: Some(remediation),
        }
    }

    fn backend_snapshot() -> (Arc<FixedDiagnostics>, ReactorToolBackend) {
        let diagnostics = Arc::new(FixedDiagnostics::default());
        let remediation = disk_cleanup();
        let backend = ReactorToolBackend {
            snapshot: ChatToolSnapshot {
                system_overview: Some("Windows 11 · ARM64".to_string()),
                scan: Some(ChatScanSnapshot {
                    session_id: "scan-1".to_string(),
                    started_at: SystemTime::now(),
                    scan_kind: ScanKind::Quick,
                    selected_tasks: vec!["os_info".to_string()],
                    results: vec![DiagnosticTaskResult::new(
                        "scan-1",
                        "os_info",
                        Arc::new(wfdiag_native_issues::TaskResult {
                            success: true,
                            output: "Windows 11".to_string(),
                            error: None,
                            duration_ms: 3,
                        }),
                    )],
                    running: false,
                }),
                issues: vec![low_disk_issue(remediation.clone())],
                history: vec![ScanSummary {
                    id: "stored-1".to_string(),
                    timestamp: wfdiag_native_history::Timestamp { secs: 1 },
                    computer_name: "TEST-PC".to_string(),
                    task_count: 1,
                    success_count: 1,
                    failure_count: 0,
                    duration_ms: 3,
                    label: None,
                    tags: vec!["Quick Scan".to_string()],
                }],
                remediations: vec![remediation],
                ..ChatToolSnapshot::default()
            },
            ports: ChatToolPorts::new(diagnostics.clone(), None),
            max_result_chars: 2_000,
        };
        (diagnostics, backend)
    }

    fn catalog_for(backend: &ReactorToolBackend) -> BoundedToolCatalog {
        ReactorChatTools {
            ports: backend.ports.clone(),
        }
        .catalog(&backend.snapshot)
    }

    fn execute(
        executor: &impl ToolExecutor,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, String> {
        let call = ToolCall {
            id: "call-1".to_string(),
            name: name.to_string(),
            arguments,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(executor.execute(&call, CancellationToken::new()))
    }

    #[test]
    fn representative_read_only_tools_return_bounded_snapshot_data() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = catalog_for(&backend);
        let executor = BoundedToolExecutor::new(catalog, backend);

        let summary = execute(&executor, "get_scan_summary", json!({})).unwrap();
        assert!(summary.contains("SCAN_SCOPE kind=quick"));
        assert!(summary.contains("Windows 11 · ARM64"));

        let issues = execute(&executor, "get_detected_issues", json!({})).unwrap();
        assert!(issues.contains("low_disk_space"));
        assert!(issues.contains("open_disk_cleanup"));

        let remediations = execute(&executor, "list_remediations", json!({})).unwrap();
        assert!(remediations.contains("open_disk_cleanup"));

        let history = execute(&executor, "list_scan_history", json!({})).unwrap();
        assert!(history.contains("stored-1"));

        let comparison = execute(&executor, "compare_with_previous_scan", json!({})).unwrap();
        assert!(comparison.contains("current scan has not been projected"));

        let diagnostic = execute(
            &executor,
            "run_diagnostic",
            json!({"task_id":"os_info", "reason":"verify build"}),
        )
        .unwrap();
        assert!(diagnostic.contains("\"build\": 26100"));
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn full_scan_and_remediation_calls_only_stage_typed_ui_requests() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = catalog_for(&backend);
        let executor = BoundedToolExecutor::new(catalog, backend);

        let scan = execute(
            &executor,
            "request_full_scan",
            json!({"reason":"need broader driver evidence"}),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&scan).unwrap()["kind"],
            "scan_request"
        );

        let proposal = execute(
            &executor,
            "stage_remediation",
            json!({
                "remediation_id":"open_disk_cleanup",
                "issue_id":"low_disk_space"
            }),
        )
        .unwrap();
        let proposal = serde_json::from_str::<serde_json::Value>(&proposal).unwrap();
        assert_eq!(proposal["kind"], "staged_action_proposal");
        assert_eq!(proposal["proposal"]["remediationId"], "open_disk_cleanup");
        assert!(
            proposal["notice"]
                .as_str()
                .unwrap()
                .contains("nothing was executed")
        );
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn canonical_parser_rejects_arbitrary_commands_and_catalog_ids() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = catalog_for(&backend);
        let executor = BoundedToolExecutor::new(catalog, backend);

        assert!(
            execute(
                &executor,
                "run_diagnostic",
                json!({"task_id":"powershell", "reason":"run arbitrary command"}),
            )
            .unwrap_err()
            .contains("Unknown diagnostic")
        );
        assert!(
            execute(&executor, "get_live_stats", json!({"command":"whoami"}))
                .unwrap_err()
                .contains("does not accept")
        );
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 0);
    }

    fn history_scan(id: &str, secs: i64, success: bool) -> ScanRecord {
        ScanRecord {
            id: id.to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs },
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: false,
            results: HashMap::from([(
                "os_info".to_string(),
                Arc::new(wfdiag_native_history::TaskResult {
                    success,
                    output: if success {
                        "collected".to_string()
                    } else {
                        "collection failed".to_string()
                    },
                    error: (!success).then(|| "access denied".to_string()),
                    duration_ms: 3,
                }),
            )]),
            task_count: 1,
            success_count: usize::from(success),
            failure_count: usize::from(!success),
            duration_ms: 3,
            label: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn history_tools_query_the_worker_and_compare_live_current_to_latest() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wfdiag-reactor-chat-history-{}-{unique}",
            std::process::id()
        ));
        let history = Arc::new(
            NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
                directory.clone(),
                || (true, 30),
                Vec::new,
            ))
            .unwrap(),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                history
                    .request_save(history_scan("stored", 1, false))
                    .unwrap(),
            )
            .unwrap()
            .unwrap();

        let diagnostics = Arc::new(FixedDiagnostics::default());
        let remediation = disk_cleanup();
        let backend = ReactorToolBackend {
            snapshot: ChatToolSnapshot {
                current_history_scan: Some(history_scan("live", 2, true)),
                history: vec![ScanSummary {
                    id: "stale-ui-copy".to_string(),
                    timestamp: wfdiag_native_history::Timestamp { secs: 0 },
                    computer_name: "TEST-PC".to_string(),
                    task_count: 0,
                    success_count: 0,
                    failure_count: 0,
                    duration_ms: 0,
                    label: None,
                    tags: Vec::new(),
                }],
                remediations: vec![remediation],
                ..ChatToolSnapshot::default()
            },
            ports: ChatToolPorts::new(diagnostics, Some(Arc::clone(&history))),
            max_result_chars: 2_000,
        };
        let executor = BoundedToolExecutor::new(catalog_for(&backend), backend);

        let list = execute(&executor, "list_scan_history", json!({})).unwrap();
        assert!(list.contains("stored"));
        assert!(!list.contains("stale-ui-copy"));
        let comparison = execute(&executor, "compare_with_previous_scan", json!({})).unwrap();
        assert!(comparison.contains("1 change(s)"));
        assert!(comparison.contains("Newly collected: os_info"));

        drop(executor);
        drop(history);
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn snapshot_projections_preserve_the_shipping_evidence_text() {
        let (_diagnostics, backend) = backend_snapshot();

        let summary = snapshot_scan_summary_text(&backend.snapshot);
        assert!(summary.starts_with("SYSTEM OVERVIEW\nWindows 11 · ARM64\n"));
        assert!(summary.contains("SCAN_SCOPE kind=quick state=complete selected=1 completed=1"));
        assert!(summary.contains("os_info: COLLECTED — Windows 11"));

        let issues = snapshot_detected_issues_text(&backend.snapshot);
        assert!(issues.starts_with("1 issue(s) detected:"));
        assert!(issues.contains("Remediation ID: open_disk_cleanup | Severity: WARNING"));

        // Detected-issue mapping is what keeps staging honest: only detected
        // issues may authorize a remediation.
        let mut healthy = backend.snapshot.clone();
        healthy.issues[0].status = IssueStatus::Ok;
        assert!(stageable_detected_issues(&healthy.issues).is_empty());
        assert_eq!(
            stage_remediation_envelope(
                &stageable_remediations(&healthy.remediations),
                &stageable_detected_issues(&healthy.issues),
                "open_disk_cleanup",
                Some("low_disk_space"),
            ),
            Err("Detected issue 'low_disk_space' is not present".to_string())
        );
    }
}
