//! Canonical, bounded diagnostic-tool contract shared by every UI shell.
//!
//! Provider schemas are guidance only. [`BoundedToolCatalog::parse`] is the
//! trust boundary: it rejects unknown operations, unknown catalog ids, extra
//! properties, malformed values, and unbounded free text before a host
//! backend sees a typed operation. No operation accepts a command, path, URL,
//! process id, service name, or arbitrary executable input.

use crate::{ToolCall, ToolExecutor, ToolFuture, ToolSpec};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const MAX_REASON_CHARS: usize = 300;
pub const MAX_GROUNDING_QUERY_CHARS: usize = 420;
const MAX_ISSUE_ID_CHARS: usize = 120;

/// Closed task metadata used by the `run_diagnostic` schema and validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticToolDescriptor {
    pub id: String,
    pub description: String,
}

/// Closed remediation metadata used by proposal staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemediationToolDescriptor {
    pub id: String,
}

/// Immutable catalog captured by a chat worker.
#[derive(Debug, Clone)]
pub struct BoundedToolCatalog {
    tasks: Arc<[DiagnosticToolDescriptor]>,
    remediations: Arc<[RemediationToolDescriptor]>,
}

impl BoundedToolCatalog {
    #[must_use]
    pub fn new(
        tasks: Vec<DiagnosticToolDescriptor>,
        remediations: Vec<RemediationToolDescriptor>,
    ) -> Self {
        Self {
            tasks: tasks.into(),
            remediations: remediations.into(),
        }
    }

    /// The one canonical tool schema exposed to tool-capable providers.
    #[must_use]
    #[allow(clippy::too_many_lines)] // One audited, closed ten-tool schema is clearer kept together.
    pub fn specs(&self) -> Vec<ToolSpec> {
        let task_ids: Vec<&str> = self.tasks.iter().map(|task| task.id.as_str()).collect();
        let task_list = self
            .tasks
            .iter()
            .map(|task| format!("{}: {}", task.id, task.description))
            .collect::<Vec<_>>()
            .join("; ");
        let remediation_ids: Vec<&str> = self
            .remediations
            .iter()
            .map(|remediation| remediation.id.as_str())
            .collect();

        vec![
            ToolSpec {
                name: "run_diagnostic".into(),
                description: format!(
                    "Run one Windows diagnostic task and return its output. Available tasks — {task_list}"
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
                    "additionalProperties": false,
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
                    "additionalProperties": false,
                }),
            },
            ToolSpec {
                name: "get_scan_summary".into(),
                description: "Summarize the current session's scan: which diagnostics ran, which \
                              returned data or had collection errors, and how old the scan is. Check \
                              this before re-running diagnostics."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "request_full_scan".into(),
                description: "Request explicit user confirmation to run a Full Scan when the current \
                              completed Quick or targeted scan is not broad enough to answer \
                              reliably. This only creates a UI request; it never starts a scan. Do \
                              not use it when scan coverage is NONE, IN_PROGRESS, or FULL."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "reason": {
                            "type": "string",
                            "description": "Concise explanation of what broader evidence is needed",
                        }
                    },
                    "required": ["reason"],
                    "additionalProperties": false,
                }),
            },
            ToolSpec {
                name: "get_detected_issues".into(),
                description: "List the issues the app's rule-based detector found in the current \
                              scan, with severity and recommendations."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "compare_with_previous_scan".into(),
                description: "Compare the two most recent stored scans: new collection errors, \
                              newly collected tasks, and changed outputs."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "get_live_stats".into(),
                description: "Current CPU, memory, disk and network stats from the live monitor \
                              (only when monitoring is running)."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "list_remediations".into(),
                description: "List the app's vetted one-click fixes (id, tier, what each runs). \
                              You can reference them by label; only the user can approve and run \
                              them through the app's review UI."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "list_scan_history".into(),
                description: "List up to 10 stored scans (id, time, collected/collection-error \
                              counts, labels)."
                    .into(),
                parameters: empty_object_schema(),
            },
            ToolSpec {
                name: "stage_remediation".into(),
                description: "Stage exactly one vetted remediation-catalog action for the user to \
                              review. This creates an expiring preview only: it never approves or \
                              runs the action. Use only after a detected issue or explicit \
                              maintenance request identifies the exact catalog ID."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "remediation_id": {
                            "type": "string",
                            "description": "Exact ID from list_remediations",
                            "enum": remediation_ids,
                        },
                        "issue_id": {
                            "type": "string",
                            "description": "Detected issue ID when the remediation is issue-bound",
                        }
                    },
                    "required": ["remediation_id"],
                    "additionalProperties": false,
                }),
            },
        ]
    }

    /// Validate an untrusted provider call and project it into a closed,
    /// strongly typed operation.
    pub fn parse(&self, call: &ToolCall) -> Result<BoundedToolOperation, String> {
        let args = require_object(call)?;
        match call.name.as_str() {
            "run_diagnostic" => {
                reject_extra_keys(call, args, &["task_id", "reason"])?;
                let task_id = required_string(call, args, "task_id")?;
                if !self.tasks.iter().any(|task| task.id == task_id) {
                    return Err(format!("Unknown diagnostic task '{task_id}'"));
                }
                let reason = bounded_required_string(call, args, "reason", MAX_REASON_CHARS)?;
                Ok(BoundedToolOperation::RunDiagnostic {
                    task_id: task_id.to_string(),
                    reason,
                })
            }
            "search_windows_knowledge" => {
                reject_extra_keys(call, args, &["query"])?;
                Ok(BoundedToolOperation::SearchWindowsKnowledge {
                    query: bounded_required_string(call, args, "query", MAX_GROUNDING_QUERY_CHARS)?,
                })
            }
            "request_full_scan" => {
                reject_extra_keys(call, args, &["reason"])?;
                Ok(BoundedToolOperation::RequestFullScan {
                    reason: bounded_required_string(call, args, "reason", MAX_REASON_CHARS)?,
                })
            }
            "stage_remediation" => {
                reject_extra_keys(call, args, &["remediation_id", "issue_id"])?;
                let remediation_id = required_string(call, args, "remediation_id")?;
                if !self
                    .remediations
                    .iter()
                    .any(|remediation| remediation.id == remediation_id)
                {
                    return Err(format!("Unknown remediation '{remediation_id}'"));
                }
                let issue_id = args
                    .get("issue_id")
                    .map(|_| bounded_required_string(call, args, "issue_id", MAX_ISSUE_ID_CHARS))
                    .transpose()?;
                Ok(BoundedToolOperation::StageRemediation {
                    remediation_id: remediation_id.to_string(),
                    issue_id,
                })
            }
            "get_scan_summary" => empty_operation(call, args, BoundedToolOperation::GetScanSummary),
            "get_detected_issues" => {
                empty_operation(call, args, BoundedToolOperation::GetDetectedIssues)
            }
            "compare_with_previous_scan" => {
                empty_operation(call, args, BoundedToolOperation::CompareWithPreviousScan)
            }
            "get_live_stats" => empty_operation(call, args, BoundedToolOperation::GetLiveStats),
            "list_remediations" => {
                empty_operation(call, args, BoundedToolOperation::ListRemediations)
            }
            "list_scan_history" => {
                empty_operation(call, args, BoundedToolOperation::ListScanHistory)
            }
            other => Err(format!("Unknown tool '{other}'")),
        }
    }
}

/// Typed operation admitted by [`BoundedToolCatalog::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedToolOperation {
    RunDiagnostic {
        task_id: String,
        reason: String,
    },
    SearchWindowsKnowledge {
        query: String,
    },
    GetScanSummary,
    RequestFullScan {
        reason: String,
    },
    GetDetectedIssues,
    CompareWithPreviousScan,
    GetLiveStats,
    ListRemediations,
    ListScanHistory,
    StageRemediation {
        remediation_id: String,
        issue_id: Option<String>,
    },
}

/// Host-specific implementations receive only validated, closed operations.
pub trait BoundedToolBackend: Send + Sync {
    #[allow(clippy::elidable_lifetime_names)] // Makes the returned future's self-borrow explicit.
    fn execute<'a>(
        &'a self,
        operation: BoundedToolOperation,
        cancel: CancellationToken,
    ) -> ToolFuture<'a>;
}

/// Shared validator/dispatcher used as the model-facing [`ToolExecutor`].
pub struct BoundedToolExecutor<B> {
    catalog: BoundedToolCatalog,
    backend: B,
}

impl<B> BoundedToolExecutor<B> {
    #[must_use]
    pub const fn new(catalog: BoundedToolCatalog, backend: B) -> Self {
        Self { catalog, backend }
    }

    #[must_use]
    pub const fn catalog(&self) -> &BoundedToolCatalog {
        &self.catalog
    }
}

impl<B: BoundedToolBackend> ToolExecutor for BoundedToolExecutor<B> {
    fn execute<'a>(&'a self, call: &'a ToolCall, cancel: CancellationToken) -> ToolFuture<'a> {
        let operation = self.catalog.parse(call);
        Box::pin(async move {
            let operation = operation?;
            self.backend.execute(operation, cancel).await
        })
    }
}

fn empty_object_schema() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn require_object(call: &ToolCall) -> Result<&serde_json::Map<String, Value>, String> {
    call.arguments
        .as_object()
        .ok_or_else(|| format!("{} arguments must be a JSON object", call.name))
}

fn reject_extra_keys(
    call: &ToolCall,
    args: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{} does not accept argument '{key}'", call.name));
    }
    Ok(())
}

fn required_string<'a>(
    call: &ToolCall,
    args: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} requires a string {key}", call.name))
}

fn bounded_required_string(
    call: &ToolCall,
    args: &serde_json::Map<String, Value>,
    key: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = required_string(call, args, key)?;
    let length = value.chars().count();
    if value.trim().is_empty() || length > max_chars {
        return Err(format!(
            "{} {key} must be 1-{max_chars} characters",
            call.name
        ));
    }
    Ok(value.trim().to_string())
}

fn empty_operation(
    call: &ToolCall,
    args: &serde_json::Map<String, Value>,
    operation: BoundedToolOperation,
) -> Result<BoundedToolOperation, String> {
    reject_extra_keys(call, args, &[])?;
    Ok(operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> BoundedToolCatalog {
        BoundedToolCatalog::new(
            vec![DiagnosticToolDescriptor {
                id: "os_info".to_string(),
                description: "Operating system".to_string(),
            }],
            vec![RemediationToolDescriptor {
                id: "open_disk_cleanup".to_string(),
            }],
        )
    }

    #[test]
    fn registry_is_the_closed_shipping_set() {
        let names = catalog()
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
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
    }

    #[test]
    fn parser_rejects_unknown_ids_extra_keys_and_unbounded_text() {
        let catalog = catalog();
        let unknown = ToolCall {
            id: "1".to_string(),
            name: "run_diagnostic".to_string(),
            arguments: json!({"task_id":"arbitrary", "reason":"check"}),
        };
        assert!(
            catalog
                .parse(&unknown)
                .unwrap_err()
                .contains("Unknown diagnostic")
        );

        let extra = ToolCall {
            id: "2".to_string(),
            name: "get_live_stats".to_string(),
            arguments: json!({"command":"whoami"}),
        };
        assert!(
            catalog
                .parse(&extra)
                .unwrap_err()
                .contains("does not accept")
        );

        let too_long = ToolCall {
            id: "3".to_string(),
            name: "search_windows_knowledge".to_string(),
            arguments: json!({"query":"x".repeat(MAX_GROUNDING_QUERY_CHARS + 1)}),
        };
        assert!(catalog.parse(&too_long).unwrap_err().contains("1-420"));
    }

    #[test]
    fn parser_returns_only_typed_catalog_operations() {
        let operation = catalog()
            .parse(&ToolCall {
                id: "1".to_string(),
                name: "stage_remediation".to_string(),
                arguments: json!({
                    "remediation_id": "open_disk_cleanup",
                    "issue_id": "low_disk_space"
                }),
            })
            .unwrap();
        assert_eq!(
            operation,
            BoundedToolOperation::StageRemediation {
                remediation_id: "open_disk_cleanup".to_string(),
                issue_id: Some("low_disk_space".to_string()),
            }
        );
    }
}
