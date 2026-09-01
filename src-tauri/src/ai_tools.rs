//! Bounded tool surface for agentic AI chat.
//!
//! Diagnostics and grounding are read-only. The sole stateful operation may
//! stage one immutable catalog-backed proposal for user review; it cannot
//! approve, authorize, or execute anything. Arbitrary commands, paths, URLs,
//! process/service control, and file writes are never accepted from a model.

use crate::ai_providers::{ToolCall, ToolSpec};
use crate::diagnostics;
use crate::issue_catalog::{DetectCtx, Issue, IssueSeverity, IssueStatus, detect_all};
use crate::native_monitor::SystemMonitor;
use crate::results_storage::ScanStorage;
use crate::state::{DiagnosticSession, ScanKind};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use wfdiag_native_ai_chat::MAX_GROUNDING_QUERY_CHARS;
use wfdiag_native_ai_chat::{
    BoundedToolBackend, BoundedToolCatalog, BoundedToolOperation, DiagnosticToolDescriptor,
    RemediationToolDescriptor,
};
pub use wfdiag_native_ai_chat::{ToolExecutor, ToolFuture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanCoverage {
    None,
    InProgress,
    Quick,
    Full,
    Targeted,
}

impl ScanCoverage {
    fn label(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::InProgress => "IN_PROGRESS",
            Self::Quick => "QUICK",
            Self::Full => "FULL",
            Self::Targeted => "TARGETED",
        }
    }
}

pub(crate) fn scan_coverage(session: Option<&DiagnosticSession>) -> ScanCoverage {
    let Some(session) = session else {
        return ScanCoverage::None;
    };
    let selected: std::collections::HashSet<&str> =
        session.selected_tasks.iter().map(String::as_str).collect();
    let completed = selected
        .iter()
        .filter(|task_id| session.results.contains_key(**task_id))
        .count();
    if !selected.is_empty() && completed < selected.len() {
        return ScanCoverage::InProgress;
    }
    match session.scan_kind {
        ScanKind::Quick => ScanCoverage::Quick,
        ScanKind::Full => ScanCoverage::Full,
        ScanKind::Targeted => ScanCoverage::Targeted,
    }
}

/// Compact machine-readable coverage for deterministic evidence packets.
/// It stays first so tiny-context providers cannot lose whether the evidence
/// is from a Quick or Full Scan.
pub(crate) fn scan_coverage_header(session: Option<&DiagnosticSession>) -> String {
    let Some(session) = session else {
        return "SCAN_SCOPE kind=none state=empty selected=0 completed=0".to_string();
    };
    let selected: std::collections::HashSet<&str> =
        session.selected_tasks.iter().map(String::as_str).collect();
    let completed = selected
        .iter()
        .filter(|task_id| session.results.contains_key(**task_id))
        .count();
    let coverage = scan_coverage(Some(session));
    let (kind, state) = match coverage {
        ScanCoverage::None => ("none", "empty"),
        ScanCoverage::InProgress => (
            match session.scan_kind {
                ScanKind::Quick => "quick",
                ScanKind::Full => "full",
                ScanKind::Targeted => "targeted",
            },
            "running",
        ),
        ScanCoverage::Quick => ("quick", "complete"),
        ScanCoverage::Full => ("full", "complete"),
        ScanCoverage::Targeted => ("targeted", "complete"),
    };
    format!(
        "SCAN_SCOPE kind={} state={} selected={} completed={}",
        kind,
        state,
        selected.len(),
        completed
    )
}

pub(crate) fn scan_coverage_text(session: Option<&DiagnosticSession>) -> String {
    let Some(session) = session else {
        return format!(
            "{}. The application must collect a Quick Scan before this scan-dependent question is answered; do not tell the user to navigate elsewhere or to run a scan manually.",
            scan_coverage_header(None)
        );
    };
    let coverage = scan_coverage(Some(session));
    let guidance = match coverage {
        ScanCoverage::None => unreachable!("a session is present"),
        ScanCoverage::InProgress => {
            "Wait for the active scan to finish before judging whether evidence is sufficient."
        }
        ScanCoverage::Quick => {
            "This is a completed Quick Scan. If it cannot answer the question reliably, request the user's confirmation for a Full Scan with request_full_scan; do not tell them to start one manually."
        }
        ScanCoverage::Full => {
            "This is a completed Full Scan. Use its evidence and state any specific collection gaps."
        }
        ScanCoverage::Targeted => {
            "This is targeted/partial coverage. If broader system coverage is required, request the user's confirmation for a Full Scan with request_full_scan."
        }
    };
    format!(
        "{}. SCAN COVERAGE: {}. {}",
        scan_coverage_header(Some(session)),
        coverage.label(),
        guidance
    )
}

fn bounded_tool_catalog() -> BoundedToolCatalog {
    BoundedToolCatalog::new(
        diagnostics::get_all_tasks()
            .into_iter()
            .map(|task| DiagnosticToolDescriptor {
                id: task.id,
                description: task.description,
            })
            .collect(),
        crate::remediation::remediations()
            .iter()
            .map(|remediation| RemediationToolDescriptor {
                id: remediation.id.to_string(),
            })
            .collect(),
    )
}

/// Provider schemas guide the model, but this is the actual trust boundary.
/// Every call is checked again immediately before dispatch, including extra
/// properties and bounded free text. A malformed call never reaches a tool.
#[cfg(test)]
pub(crate) fn validate_tool_call(call: &ToolCall) -> Result<(), String> {
    bounded_tool_catalog().parse(call).map(|_| ())
    /* legacy inlined validator retained in git history
    match call.name.as_str() {
        "run_diagnostic" => {
            reject_extra_keys(call, args, &["task_id", "reason"])?;
            let task_id = args
                .get("task_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "run_diagnostic requires a string task_id".to_string())?;
            if !diagnostics::get_all_tasks()
                .iter()
                .any(|task| task.id == task_id)
            {
                return Err(format!("Unknown diagnostic task '{}'", task_id));
            }
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .ok_or_else(|| "run_diagnostic requires a string reason".to_string())?;
            let reason_len = reason.chars().count();
            if reason.trim().is_empty() || reason_len > MAX_REASON_CHARS {
                return Err(format!(
                    "run_diagnostic reason must be 1-{} characters",
                    MAX_REASON_CHARS
                ));
            }
        }
        "search_windows_knowledge" => {
            reject_extra_keys(call, args, &["query"])?;
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| "search_windows_knowledge requires a string query".to_string())?;
            let query_len = query.chars().count();
            if query.trim().is_empty() || query_len > MAX_GROUNDING_QUERY_CHARS {
                return Err(format!(
                    "search_windows_knowledge query must be 1-{} characters",
                    MAX_GROUNDING_QUERY_CHARS
                ));
            }
        }
        "stage_remediation" => {
            reject_extra_keys(call, args, &["remediation_id", "issue_id"])?;
            let remediation_id = args
                .get("remediation_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "stage_remediation requires a string remediation_id".to_string())?;
            if crate::remediation::find(remediation_id).is_none() {
                return Err(format!("Unknown remediation '{}'", remediation_id));
            }
            if let Some(issue_id) = args.get("issue_id") {
                let issue_id = issue_id
                    .as_str()
                    .ok_or_else(|| "stage_remediation issue_id must be a string".to_string())?;
                if issue_id.trim().is_empty() || issue_id.chars().count() > 120 {
                    return Err("stage_remediation issue_id must be 1-120 characters".to_string());
                }
            }
        }
        "request_full_scan" => {
            reject_extra_keys(call, args, &["reason"])?;
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .ok_or_else(|| "request_full_scan requires a string reason".to_string())?;
            let reason_len = reason.chars().count();
            if reason.trim().is_empty() || reason_len > MAX_REASON_CHARS {
                return Err(format!(
                    "request_full_scan reason must be 1-{} characters",
                    MAX_REASON_CHARS
                ));
            }
        }
        "get_scan_summary"
        | "get_detected_issues"
        | "compare_with_previous_scan"
        | "get_live_stats"
        | "list_remediations"
        | "list_scan_history" => reject_extra_keys(call, args, &[])?,
        other => return Err(format!("Unknown tool '{}'", other)),
    }
    Ok(()) */
}

/// The curated bounded tool set offered to tool-capable providers.
pub fn tool_registry() -> Vec<ToolSpec> {
    bounded_tool_catalog().specs()
}

/// Render the current session as a compact scan summary. Also used as inline
/// context for providers without tool support.
pub fn scan_summary_text(session: Option<&DiagnosticSession>) -> String {
    let Some(session) = session else {
        return scan_coverage_text(None);
    };
    let mut collected = 0usize;
    let mut collection_errors = 0usize;
    let mut lines: Vec<String> = Vec::with_capacity(session.results.len());
    let mut ids: Vec<&String> = session.results.keys().collect();
    ids.sort();
    for id in ids {
        let result = &session.results[id];
        if result.success {
            collected += 1;
            lines.push(format!("{}: COLLECTED", id));
        } else {
            collection_errors += 1;
            lines.push(format!(
                "{}: COLLECTION ERROR{}",
                id,
                result
                    .error
                    .as_deref()
                    .map(|e| format!(" ({})", e))
                    .unwrap_or_default()
            ));
        }
    }
    let age_minutes = session
        .start_time
        .elapsed()
        .map(|d| d.as_secs() / 60)
        .unwrap_or(0);
    format!(
        "{}\nScan from {} minute(s) ago: {} collected, {} collection failures, {} total.\n\
         COLLECTED means the diagnostic returned data; it does not mean the component is healthy.\n{}",
        scan_coverage_text(Some(session)),
        age_minutes,
        collected,
        collection_errors,
        session.results.len(),
        lines.join("\n")
    )
}

fn detected_issue_text(issue: &Issue) -> String {
    let severity = match issue.severity {
        IssueSeverity::Critical => "CRITICAL",
        IssueSeverity::Warning => "WARNING",
        IssueSeverity::Info => "INFO",
        IssueSeverity::Ok => "OK",
    };
    let remediation_id = issue
        .remediation
        .as_ref()
        .map(|remediation| remediation.id.as_str())
        .unwrap_or("none");
    format!(
        "Issue ID: {} | Remediation ID: {} | Severity: {} | {} — {} | Recommendation: {}",
        issue.id, remediation_id, severity, issue.title, issue.description, issue.recommendation
    )
}

/// Real executor over the app state. All locks are taken briefly per call.
pub struct AppToolExecutor {
    pub current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    pub scan_storage: Arc<Mutex<Option<ScanStorage>>>,
    pub system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
    pub action_broker: Arc<Mutex<crate::action_broker::ActionBrokerState>>,
    /// Per-tool-result character cap (from the provider's context plan)
    pub max_result_chars: usize,
}

impl AppToolExecutor {
    async fn run_diagnostic(&self, arguments: &Value) -> Result<String, String> {
        let task_id = arguments
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "run_diagnostic requires a task_id".to_string())?;
        // The enum in the schema constrains the model, but validate anyway
        if !diagnostics::get_all_tasks().iter().any(|t| t.id == task_id) {
            return Err(format!("Unknown diagnostic task '{}'", task_id));
        }
        // Deliberately NOT written into the scan session: chat-triggered runs
        // must not pollute scan provenance.
        let result = diagnostics::run_diagnostic_task(task_id).await;
        if result.success {
            Ok(crate::ai_prompts::json_to_readable_text(
                &result.output,
                self.max_result_chars,
            ))
        } else {
            Ok(format!(
                "Task {} COLLECTION ERROR: {}",
                task_id,
                result.error.unwrap_or_else(|| "unknown error".into())
            ))
        }
    }

    async fn get_scan_summary(&self) -> Result<String, String> {
        let session = self.current_session.lock().await;
        Ok(crate::ai_prompts::truncate_output(
            &scan_summary_text(session.as_ref()),
            self.max_result_chars,
        ))
    }

    async fn search_windows_knowledge(
        &self,
        arguments: &Value,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| "search_windows_knowledge requires a query".to_string())?;
        crate::ai_grounding::search_grounding_cancellable(query, self.max_result_chars, cancel)
            .await
    }

    async fn get_detected_issues(&self) -> Result<String, String> {
        let session = self.current_session.lock().await;
        let Some(session) = session.as_ref() else {
            return Ok(
                "No scan data is available yet. The application should collect its automatic Quick Scan before this question reaches the assistant; do not tell the user to run one manually."
                    .to_string(),
            );
        };
        let detect_ctx = DetectCtx {
            results: &session.results,
            now: crate::timestamp::Timestamp::now(),
            temp_file_count: None, // env probing stays out of the chat path
        };
        let issues = detect_all(&detect_ctx);
        let mut detected: Vec<String> = Vec::new();
        let mut unknown: Vec<String> = Vec::new();
        for issue in &issues {
            match issue.status {
                IssueStatus::Detected => detected.push(detected_issue_text(issue)),
                IssueStatus::Unknown | IssueStatus::Skipped => unknown.push(format!(
                    "Issue ID: {} | Status: UNKNOWN | {} — {}",
                    issue.id, issue.title, issue.description
                )),
                IssueStatus::Ok => {}
            }
        }
        let mut sections = vec![if detected.is_empty() {
            "Detected issues: none.".to_string()
        } else {
            format!(
                "{} issue(s) detected:\n{}",
                detected.len(),
                detected.join("\n")
            )
        }];
        if !unknown.is_empty() {
            sections.push(format!(
                "{} check(s) could not be verified:\n{}",
                unknown.len(),
                unknown.join("\n")
            ));
        }
        let text = sections.join("\n\n");
        Ok(crate::ai_prompts::truncate_output(
            &text,
            self.max_result_chars,
        ))
    }

    async fn compare_with_previous_scan(&self) -> Result<String, String> {
        let storage = self.scan_storage.lock().await;
        let Some(storage) = storage.as_ref() else {
            return Ok("Scan history storage is unavailable on this system.".to_string());
        };
        let scans = storage.list_scans()?;
        if scans.len() < 2 {
            return Ok(format!(
                "Only {} stored scan(s) — at least two are needed to compare. Suggest saving \
                 scans over time from the Diagnostics tab.",
                scans.len()
            ));
        }
        let comparison = storage.compare_scans(&scans[0].id, &scans[1].id)?;
        let list = |changes: &[crate::results_storage::TaskChange]| {
            changes
                .iter()
                .map(|c| c.task_id.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let text = format!(
            "Compared {} (current) with {} (previous): {} change(s).\nNew collection errors: {}\nNewly collected: {}",
            comparison.current_scan.timestamp,
            comparison.previous_scan.timestamp,
            comparison.total_changes,
            if comparison.new_failures.is_empty() {
                "none".into()
            } else {
                list(&comparison.new_failures)
            },
            if comparison.new_successes.is_empty() {
                "none".into()
            } else {
                list(&comparison.new_successes)
            },
        );
        Ok(crate::ai_prompts::truncate_output(
            &text,
            self.max_result_chars,
        ))
    }

    async fn get_live_stats(&self) -> Result<String, String> {
        let monitor = self.system_monitor.lock().await;
        let Some(monitor) = monitor.as_ref() else {
            return Ok(
                "Monitoring is not running — live values unavailable. Suggest the user open \
                 the Monitor tab to start it."
                    .to_string(),
            );
        };
        let stats = monitor.get_current_stats().await;
        let value = serde_json::to_string(&stats).unwrap_or_default();
        Ok(crate::ai_prompts::json_to_readable_text(
            &value,
            self.max_result_chars,
        ))
    }

    async fn list_scan_history(&self) -> Result<String, String> {
        let storage = self.scan_storage.lock().await;
        let Some(storage) = storage.as_ref() else {
            return Ok("Scan history storage is unavailable on this system.".to_string());
        };
        let scans = storage.list_scans()?;
        if scans.is_empty() {
            return Ok("No stored scans yet.".to_string());
        }
        let lines: Vec<String> = scans
            .iter()
            .take(10)
            .map(|s| {
                format!(
                    "{} | {} | {} collected / {} collection errors{}",
                    s.id,
                    s.timestamp,
                    s.success_count,
                    s.failure_count,
                    if s.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" | tags: {}", s.tags.join(", "))
                    }
                )
            })
            .collect();
        Ok(crate::ai_prompts::truncate_output(
            &lines.join("\n"),
            self.max_result_chars,
        ))
    }
}

impl AppToolExecutor {
    async fn request_full_scan(&self, arguments: &Value) -> Result<String, String> {
        let reason = arguments
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| "request_full_scan requires a reason".to_string())?
            .trim();
        let session = self.current_session.lock().await;
        match scan_coverage(session.as_ref()) {
            ScanCoverage::Quick | ScanCoverage::Targeted => serde_json::to_string(&json!({
                "kind": "scan_request",
                "scanKind": "full",
                "reason": reason,
                "notice": "Confirmation requested only. The Full Scan has not started."
            }))
            .map_err(|error| format!("Could not serialize the Full Scan request: {error}")),
            ScanCoverage::None => Ok(
                "No scan evidence is available. Do not request a Full Scan yet; the application must complete its automatic Quick Scan first."
                    .to_string(),
            ),
            ScanCoverage::InProgress => Ok(
                "A scan is still in progress. Wait for it to finish before deciding whether a Full Scan is needed."
                    .to_string(),
            ),
            ScanCoverage::Full => Ok(
                "The current session already contains a Full Scan. Use its evidence and explain any specific collection failure instead of requesting another scan."
                    .to_string(),
            ),
        }
    }

    async fn list_remediations(&self) -> Result<String, String> {
        let lines: Vec<String> = crate::remediation::remediations()
            .iter()
            .map(|r| {
                format!(
                    "{} | {} | {:?}{} | {}",
                    r.id,
                    r.label,
                    r.tier,
                    if r.admin_required { " | admin" } else { "" },
                    r.description
                )
            })
            .collect();
        Ok(crate::ai_prompts::truncate_output(
            &lines.join("\n"),
            self.max_result_chars,
        ))
    }

    async fn stage_remediation(&self, arguments: &Value) -> Result<String, String> {
        let remediation_id = arguments
            .get("remediation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "stage_remediation requires a remediation_id".to_string())?;
        let issue_id = arguments
            .get("issue_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let proposal = crate::action_broker::stage_exact_proposal_from_parts(
            &self.action_broker,
            &self.current_session,
            crate::action_broker::ActionPrepareInput {
                actions: vec![crate::action_broker::ActionRequest {
                    remediation_id: remediation_id.to_string(),
                    issue_id,
                }],
                expected_scan_fingerprint: None,
                expected_catalog_fingerprint: None,
            },
        )
        .await?;
        serde_json::to_string(&json!({
            "kind": "staged_action_proposal",
            "proposal": proposal,
            "notice": "Staged only. Awaiting the user's exact approval; nothing was executed."
        }))
        .map_err(|error| format!("Could not serialize staged proposal: {error}"))
    }
}

impl BoundedToolBackend for AppToolExecutor {
    fn execute<'a>(
        &'a self,
        operation: BoundedToolOperation,
        cancel: CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            match operation {
                BoundedToolOperation::RunDiagnostic { task_id, reason: _ } => {
                    self.run_diagnostic(&json!({ "task_id": task_id })).await
                }
                BoundedToolOperation::SearchWindowsKnowledge { query } => {
                    self.search_windows_knowledge(&json!({ "query": query }), &cancel)
                        .await
                }
                BoundedToolOperation::GetScanSummary => self.get_scan_summary().await,
                BoundedToolOperation::RequestFullScan { reason } => {
                    self.request_full_scan(&json!({ "reason": reason })).await
                }
                BoundedToolOperation::GetDetectedIssues => self.get_detected_issues().await,
                BoundedToolOperation::CompareWithPreviousScan => {
                    self.compare_with_previous_scan().await
                }
                BoundedToolOperation::GetLiveStats => self.get_live_stats().await,
                BoundedToolOperation::ListScanHistory => self.list_scan_history().await,
                BoundedToolOperation::ListRemediations => self.list_remediations().await,
                BoundedToolOperation::StageRemediation {
                    remediation_id,
                    issue_id,
                } => {
                    self.stage_remediation(&json!({
                        "remediation_id": remediation_id,
                        "issue_id": issue_id,
                    }))
                    .await
                }
            }
        })
    }
}

impl ToolExecutor for AppToolExecutor {
    fn execute<'a>(&'a self, call: &'a ToolCall, cancel: CancellationToken) -> ToolFuture<'a> {
        let operation = bounded_tool_catalog().parse(call);
        Box::pin(async move {
            let operation = operation?;
            BoundedToolBackend::execute(self, operation, cancel).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::TaskResult;
    use std::collections::HashMap;

    #[test]
    fn registry_has_only_bounded_checks_and_proposal_staging() {
        let names: Vec<String> = tool_registry().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "run_diagnostic",
                "search_windows_knowledge",
                "get_scan_summary",
                "request_full_scan",
                "get_detected_issues",
                "compare_with_previous_scan",
                "get_live_stats",
                "list_remediations",
                "list_scan_history",
                "stage_remediation",
            ]
        );
        // Approval, execution, arbitrary command, and file/process mutation
        // surfaces must never be exposed to the model.
        for forbidden in [
            "approve",
            "execute",
            "command",
            "powershell",
            "file",
            "process",
        ] {
            assert!(!names.iter().any(|name| name.contains(forbidden)));
        }

        let descriptions = tool_registry()
            .into_iter()
            .map(|tool| tool.description)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        assert!(!descriptions.contains("passed or failed"));
        assert!(!descriptions.contains("pass/fail"));
    }

    #[test]
    fn run_diagnostic_enum_matches_the_task_registry() {
        let registry = tool_registry();
        let spec = registry
            .iter()
            .find(|t| t.name == "run_diagnostic")
            .unwrap();
        let enum_ids: Vec<&str> = spec.parameters["properties"]["task_id"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let task_ids: Vec<String> = diagnostics::get_all_tasks()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(!enum_ids.is_empty());
        assert_eq!(enum_ids.len(), task_ids.len());
        for id in task_ids {
            assert!(enum_ids.contains(&id.as_str()), "missing task id {}", id);
        }
    }

    #[test]
    fn strict_validation_rejects_unknown_fields_and_oversized_text() {
        let extra = ToolCall {
            id: "c1".into(),
            name: "get_scan_summary".into(),
            arguments: json!({"surprise": true}),
        };
        assert!(
            validate_tool_call(&extra)
                .unwrap_err()
                .contains("does not accept")
        );

        let long_query = ToolCall {
            id: "c2".into(),
            name: "search_windows_knowledge".into(),
            arguments: json!({"query": "x".repeat(MAX_GROUNDING_QUERY_CHARS + 1)}),
        };
        assert!(validate_tool_call(&long_query).is_err());

        let invented_remediation = ToolCall {
            id: "c3".into(),
            name: "stage_remediation".into(),
            arguments: json!({"remediation_id": "run_anything"}),
        };
        assert!(validate_tool_call(&invented_remediation).is_err());

        let missing_scan_reason = ToolCall {
            id: "c4".into(),
            name: "request_full_scan".into(),
            arguments: json!({}),
        };
        assert!(validate_tool_call(&missing_scan_reason).is_err());

        let unknown = ToolCall {
            id: "c5".into(),
            name: "run_powershell".into(),
            arguments: json!({}),
        };
        assert!(
            validate_tool_call(&unknown)
                .unwrap_err()
                .contains("Unknown tool")
        );
    }

    #[test]
    fn scan_summary_renders_counts_and_failures() {
        let mut results = HashMap::new();
        results.insert(
            "os_info".to_string(),
            TaskResult {
                success: true,
                output: "{}".into(),
                error: None,
                duration_ms: 5,
            },
        );
        results.insert(
            "chkdsk".to_string(),
            TaskResult {
                success: false,
                output: String::new(),
                error: Some("access denied".into()),
                duration_ms: 9,
            },
        );
        let session = DiagnosticSession {
            session_id: "s1".into(),
            start_time: std::time::SystemTime::now(),
            scan_kind: crate::state::ScanKind::Targeted,
            selected_tasks: vec![],
            results,
        };
        let text = scan_summary_text(Some(&session));
        assert!(text.contains("1 collected, 1 collection failures, 2 total"));
        assert!(text.contains("does not mean the component is healthy"));
        assert!(text.contains("chkdsk: COLLECTION ERROR (access denied)"));
        assert!(text.contains("os_info: COLLECTED"));
        // Without a session the tool reports the host-orchestrated preflight;
        // it never tells the model to send the user away to run a scan.
        let none = scan_summary_text(None);
        assert!(none.contains("SCAN_SCOPE kind=none"));
        assert!(none.contains("application must collect a Quick Scan"));
        assert!(!none.contains("run a scan first"));
    }

    #[test]
    fn scan_coverage_is_explicit_for_quick_full_and_running_sessions() {
        let session = |kind, selected: Vec<&str>, completed: Vec<&str>| DiagnosticSession {
            session_id: "s".into(),
            start_time: std::time::SystemTime::now(),
            scan_kind: kind,
            selected_tasks: selected.into_iter().map(str::to_string).collect(),
            results: completed
                .into_iter()
                .map(|id| {
                    (
                        id.to_string(),
                        TaskResult {
                            success: true,
                            output: "{}".into(),
                            error: None,
                            duration_ms: 1,
                        },
                    )
                })
                .collect(),
        };
        let quick = session(ScanKind::Quick, vec!["os_info"], vec!["os_info"]);
        assert_eq!(scan_coverage(Some(&quick)), ScanCoverage::Quick);
        assert!(scan_coverage_header(Some(&quick)).contains("kind=quick state=complete"));

        let full = session(ScanKind::Full, vec!["os_info"], vec!["os_info"]);
        assert_eq!(scan_coverage(Some(&full)), ScanCoverage::Full);
        assert!(scan_coverage_text(Some(&full)).contains("SCAN COVERAGE: FULL"));

        let running = session(
            ScanKind::Quick,
            vec!["os_info", "logical_disk"],
            vec!["os_info"],
        );
        assert_eq!(scan_coverage(Some(&running)), ScanCoverage::InProgress);
        assert!(scan_coverage_header(Some(&running)).contains("state=running"));
    }

    #[tokio::test]
    async fn full_scan_request_is_only_an_envelope_for_partial_coverage() {
        let make_executor = |scan_kind| AppToolExecutor {
            current_session: Arc::new(Mutex::new(Some(DiagnosticSession {
                session_id: "s".into(),
                start_time: std::time::SystemTime::now(),
                scan_kind,
                selected_tasks: vec!["os_info".into()],
                results: [(
                    "os_info".into(),
                    TaskResult {
                        success: true,
                        output: "{}".into(),
                        error: None,
                        duration_ms: 1,
                    },
                )]
                .into_iter()
                .collect(),
            }))),
            scan_storage: Arc::new(Mutex::new(None)),
            system_monitor: Arc::new(Mutex::new(None)),
            action_broker: Arc::new(Mutex::new(
                crate::action_broker::ActionBrokerState::default(),
            )),
            max_result_chars: 2_000,
        };
        let arguments = json!({"reason": "Event-log evidence is outside Quick Scan coverage"});
        let quick = make_executor(ScanKind::Quick)
            .request_full_scan(&arguments)
            .await
            .unwrap();
        let envelope: Value = serde_json::from_str(&quick).unwrap();
        assert_eq!(envelope["kind"], "scan_request");
        assert_eq!(envelope["scanKind"], "full");
        assert_eq!(
            envelope["reason"],
            "Event-log evidence is outside Quick Scan coverage"
        );

        let full = make_executor(ScanKind::Full)
            .request_full_scan(&arguments)
            .await
            .unwrap();
        assert!(full.contains("already contains a Full Scan"));
        assert!(serde_json::from_str::<Value>(&full).is_err());
    }

    #[test]
    fn detected_issue_text_exposes_exact_issue_and_remediation_ids() {
        let issue = Issue {
            id: "low_disk_space".into(),
            category: "Storage".into(),
            severity: IssueSeverity::Critical,
            status: IssueStatus::Detected,
            title: "Low Disk Space".into(),
            description: "C: has 4% free".into(),
            recommendation: "Free disk space".into(),
            detected: true,
            source_tasks: Some(vec!["logical_disk".into()]),
            remediation: crate::remediation::summary("open_disk_cleanup"),
        };

        let text = detected_issue_text(&issue);
        assert!(text.contains("Issue ID: low_disk_space"));
        assert!(text.contains("Remediation ID: open_disk_cleanup"));
        assert!(text.contains("Severity: CRITICAL"));
    }
}
