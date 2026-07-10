//! Read-only tool surface for the agentic AI chat.
//!
//! SAFETY RULE: every tool here is strictly read-only. `fix_issue`,
//! `restart_as_admin`, tag updates, and any other mutation are deliberately
//! NOT exposed to the model — the system prompt directs users to the Issues
//! tab for vetted one-click fixes. Do not add write tools without revisiting
//! that decision.

use crate::ai_providers::{ToolCall, ToolSpec};
use crate::diagnostics;
use crate::issue_catalog::{DetectCtx, IssueSeverity, detect_all};
use crate::native_monitor::SystemMonitor;
use crate::results_storage::ScanStorage;
use crate::state::DiagnosticSession;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

/// Executes tool calls. A trait so the chat loop is testable with a mock.
pub trait ToolExecutor: Send + Sync {
    fn execute<'a>(&'a self, call: &'a ToolCall) -> ToolFuture<'a>;
}

/// The curated read-only tool set offered to tool-capable providers.
pub fn tool_registry() -> Vec<ToolSpec> {
    let task_ids: Vec<String> = diagnostics::get_all_tasks()
        .into_iter()
        .map(|t| t.id)
        .collect();
    let task_list = diagnostics::get_all_tasks()
        .iter()
        .map(|t| format!("{}: {}", t.id, t.description))
        .collect::<Vec<_>>()
        .join("; ");

    vec![
        ToolSpec {
            name: "run_diagnostic".into(),
            description: format!(
                "Run one Windows diagnostic task and return its output. Available tasks — {}",
                task_list
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The ID of the diagnostic task to run",
                        "enum": task_ids,
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why this diagnostic is needed for the user's question",
                    }
                },
                "required": ["task_id", "reason"],
            }),
        },
        ToolSpec {
            name: "search_windows_knowledge".into(),
            description: "Live RAG search through WindowsForum MCP, including proxied Microsoft \
                          KB/support material. Use for current Windows release, build, KB, \
                          support, known-issue, driver, and troubleshooting facts instead of \
                          guessing from memory."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Focused Windows question or diagnostic fact to ground",
                    }
                },
                "required": ["query"],
            }),
        },
        ToolSpec {
            name: "get_scan_summary".into(),
            description: "Summarize the current session's scan: which diagnostics ran, which \
                          passed or failed, and how old the scan is. Check this before \
                          re-running diagnostics."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "get_detected_issues".into(),
            description: "List the issues the app's rule-based detector found in the current \
                          scan, with severity and recommendations."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "compare_with_previous_scan".into(),
            description: "Compare the two most recent stored scans: new failures, recoveries \
                          and changed outputs."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "get_live_stats".into(),
            description: "Current CPU, memory, disk and network stats from the live monitor \
                          (only when monitoring is running)."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "list_remediations".into(),
            description: "List the app's vetted one-click fixes (id, tier, what each runs). \
                          You can reference them by label; only the USER can run them, from \
                          the Issues tab."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "list_scan_history".into(),
            description: "List up to 10 stored scans (id, time, pass/fail counts, labels).".into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// Render the current session as a compact scan summary. Also used as inline
/// context for providers without tool support.
pub fn scan_summary_text(session: Option<&DiagnosticSession>) -> String {
    let Some(session) = session else {
        return "No scan has been run this session. Suggest the user run a scan from the \
                Diagnostics tab for current data."
            .to_string();
    };
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut lines: Vec<String> = Vec::with_capacity(session.results.len());
    let mut ids: Vec<&String> = session.results.keys().collect();
    ids.sort();
    for id in ids {
        let result = &session.results[id];
        if result.success {
            ok += 1;
            lines.push(format!("{}: OK", id));
        } else {
            failed += 1;
            lines.push(format!(
                "{}: FAIL{}",
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
        "Scan from {} minute(s) ago: {} passed, {} failed, {} total.\n{}",
        age_minutes,
        ok,
        failed,
        session.results.len(),
        lines.join("\n")
    )
}

/// Real executor over the app state. All locks are taken briefly per call.
pub struct AppToolExecutor {
    pub current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    pub scan_storage: Arc<Mutex<Option<ScanStorage>>>,
    pub system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
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
                "Task {} FAILED: {}",
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

    async fn search_windows_knowledge(&self, arguments: &Value) -> Result<String, String> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| "search_windows_knowledge requires a query".to_string())?;
        crate::ai_grounding::search_grounding(query, self.max_result_chars).await
    }

    async fn get_detected_issues(&self) -> Result<String, String> {
        let session = self.current_session.lock().await;
        let Some(session) = session.as_ref() else {
            return Ok("No scan data — run a scan first, then issues can be detected.".to_string());
        };
        let detect_ctx = DetectCtx {
            results: &session.results,
            now: crate::timestamp::Timestamp::now(),
            temp_file_count: None, // env probing stays out of the chat path
        };
        let issues = detect_all(&detect_ctx);
        let mut detected: Vec<String> = Vec::new();
        for issue in issues.iter().filter(|i| i.detected) {
            let severity = match issue.severity {
                IssueSeverity::Critical => "CRITICAL",
                IssueSeverity::Warning => "WARNING",
                IssueSeverity::Info => "INFO",
                IssueSeverity::Ok => "OK",
            };
            detected.push(format!(
                "{} | {} — {} | Recommendation: {}",
                severity, issue.title, issue.description, issue.recommendation
            ));
        }
        let text = if detected.is_empty() {
            "No issues detected in the current scan.".to_string()
        } else {
            format!(
                "{} issue(s) detected:\n{}",
                detected.len(),
                detected.join("\n")
            )
        };
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
            "Compared {} (current) with {} (previous): {} change(s).\nNew failures: {}\nRecovered: {}",
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
                    "{} | {} | {} ok / {} failed{}",
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
}

impl ToolExecutor for AppToolExecutor {
    fn execute<'a>(&'a self, call: &'a ToolCall) -> ToolFuture<'a> {
        Box::pin(async move {
            match call.name.as_str() {
                "run_diagnostic" => self.run_diagnostic(&call.arguments).await,
                "search_windows_knowledge" => self.search_windows_knowledge(&call.arguments).await,
                "get_scan_summary" => self.get_scan_summary().await,
                "get_detected_issues" => self.get_detected_issues().await,
                "compare_with_previous_scan" => self.compare_with_previous_scan().await,
                "get_live_stats" => self.get_live_stats().await,
                "list_scan_history" => self.list_scan_history().await,
                "list_remediations" => self.list_remediations().await,
                other => Err(format!("Unknown tool '{}'", other)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::TaskResult;
    use std::collections::HashMap;

    #[test]
    fn registry_has_exactly_the_read_only_tools() {
        let names: Vec<String> = tool_registry().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "run_diagnostic",
                "search_windows_knowledge",
                "get_scan_summary",
                "get_detected_issues",
                "compare_with_previous_scan",
                "get_live_stats",
                "list_remediations",
                "list_scan_history",
            ]
        );
        // The fix/write surface must never be exposed to the model
        assert!(!names.iter().any(|n| n.contains("fix")));
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
            selected_tasks: vec![],
            results,
        };
        let text = scan_summary_text(Some(&session));
        assert!(text.contains("1 passed, 1 failed, 2 total"));
        assert!(text.contains("chkdsk: FAIL (access denied)"));
        assert!(text.contains("os_info: OK"));
        // Without a session the text guides instead of erroring
        assert!(scan_summary_text(None).contains("No scan has been run"));
    }
}
