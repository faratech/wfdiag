#![allow(clippy::format_push_string, clippy::implicit_hasher)]

use crate::ExportError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wfdiag_native_issues::TaskResult;

/// Diagnostic metadata required by text and HTML rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportTask {
    pub id: String,
    pub name: String,
    pub category: String,
}

impl ExportTask {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category: category.into(),
        }
    }
}

/// Core report format accepted by the shipping `export_results` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    Json,
    Text,
    Html,
}

impl TryFrom<&str> for ReportFormat {
    type Error = ExportError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            "html" => Ok(Self::Html),
            other => Err(ExportError::UnsupportedFormat(other.to_string())),
        }
    }
}

/// Caller-rendered local metadata used by saved text and sharing payloads.
///
/// Locale-sensitive date formatting remains with the UI shell. Supplying the
/// rendered strings also makes payload generation deterministic in tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub generated: String,
    pub local_date: String,
    pub computer_name: String,
    pub os_version: String,
    pub is_admin: bool,
}

/// Email preparation output. Clipboard and external-link delivery are owned
/// by the shell, not this crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailPayload {
    pub subject: String,
    pub clipboard_body: String,
    pub mailto_body: String,
}

/// Typed request variants supported by the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportRequestKind {
    /// Report payload used by support-package generation and other callers
    /// that need the renderer's undecorated JSON, text, or HTML output.
    Report {
        format: ReportFormat,
        include_raw: bool,
    },
    /// User-selected single-file export. Store 2.5.8 decorates only the text
    /// format with local machine metadata; JSON and HTML remain byte-for-byte
    /// identical to [`Self::Report`].
    SavedReport {
        format: ReportFormat,
        include_raw: bool,
        metadata: ExportMetadata,
    },
    WindowsForumPost {
        metadata: ExportMetadata,
    },
    ForumClipboard {
        metadata: ExportMetadata,
    },
    Email {
        metadata: ExportMetadata,
    },
}

/// Typed payload returned by pure rendering or the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportPayload {
    Report(String),
    WindowsForumPost(String),
    ForumClipboard(String),
    Email(EmailPayload),
}

/// Format JSON values into the established human-readable text projection.
#[must_use]
pub fn format_json_value(value: &serde_json::Value, indent_level: usize) -> String {
    let indent = "  ".repeat(indent_level);
    match value {
        serde_json::Value::Object(map) => {
            let mut result = String::new();
            for (key, val) in map {
                let formatted_key = key
                    .replace('_', " ")
                    .split_whitespace()
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(first) => first.to_uppercase().chain(chars).collect(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                match val {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        result.push_str(&format!("{indent}{formatted_key}:\n"));
                        result.push_str(&format_json_value(val, indent_level + 1));
                    }
                    serde_json::Value::Null => {}
                    _ => {
                        result.push_str(&format!(
                            "{}{} : {}\n",
                            indent,
                            formatted_key,
                            val.as_str().unwrap_or(&val.to_string())
                        ));
                    }
                }
            }
            result
        }
        serde_json::Value::Array(array) => {
            let mut result = String::new();
            for (index, value) in array.iter().enumerate() {
                if index > 0 {
                    result.push_str(&format!("{indent}---\n"));
                }
                result.push_str(&format_json_value(value, indent_level));
            }
            result
        }
        _ => format!(
            "{}{}\n",
            indent,
            value.as_str().unwrap_or(&value.to_string())
        ),
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn redacted_json_results(
    results: &HashMap<String, TaskResult>,
) -> HashMap<String, serde_json::Value> {
    results
        .iter()
        .map(|(task_id, result)| {
            let value = serde_json::json!({
                "success": result.success,
                "error": result.error,
                "duration_ms": result.duration_ms,
            });
            (task_id.clone(), value)
        })
        .collect()
}

fn task_map(tasks: &[ExportTask]) -> HashMap<String, &ExportTask> {
    tasks.iter().map(|task| (task.id.clone(), task)).collect()
}

fn grouped_results<'a>(
    results: &'a HashMap<String, TaskResult>,
    tasks: &'a HashMap<String, &'a ExportTask>,
) -> HashMap<String, Vec<(&'a String, &'a TaskResult)>> {
    let mut by_category: HashMap<String, Vec<(&String, &TaskResult)>> = HashMap::new();

    for (task_id, result) in results {
        if let Some(task) = tasks.get(task_id) {
            by_category
                .entry(task.category.clone())
                .or_default()
                .push((task_id, result));
        }
    }

    by_category
}

fn render_text(
    results: &HashMap<String, TaskResult>,
    tasks: &HashMap<String, &ExportTask>,
    include_raw: bool,
) -> String {
    let mut text = String::new();
    let results_by_category = grouped_results(results, tasks);

    let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
    categories.sort();

    for category in categories {
        text.push_str(&format!("\n=== {category} ===\n\n"));

        if let Some(results) = results_by_category.get(&category) {
            for (task_id, result) in results {
                if let Some(task) = tasks.get(*task_id) {
                    text.push_str(&format!("{}:\n", task.name));
                    text.push_str(&format!(
                        "  Status: {}\n",
                        if result.success { "Passed" } else { "Failed" }
                    ));
                    text.push_str(&format!("  Duration: {} ms\n", result.duration_ms));

                    if result.success {
                        if include_raw {
                            if let Ok(parsed) =
                                serde_json::from_str::<serde_json::Value>(&result.output)
                            {
                                text.push_str(&format_json_value(&parsed, 1));
                            } else {
                                text.push_str(&result.output);
                                if !result.output.ends_with('\n') {
                                    text.push('\n');
                                }
                            }
                        }
                    } else if let Some(error) = &result.error {
                        text.push_str(&format!("  Error: {error}\n"));
                    }

                    text.push('\n');
                }
            }
        }
    }

    text
}

fn render_html(
    results: &HashMap<String, TaskResult>,
    tasks: &HashMap<String, &ExportTask>,
    include_raw: bool,
) -> String {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n");
    html.push_str("<title>WindowsForum Diagnostic Report</title>\n");
    html.push_str("<style>\n");
    html.push_str("body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 1200px; margin: 0 auto; padding: 20px; background: #1a1a2e; color: #eee; }\n");
    html.push_str(
        "h1 { color: #60a5fa; border-bottom: 2px solid #3b82f6; padding-bottom: 10px; }\n",
    );
    html.push_str("h2 { color: #93c5fd; margin-top: 30px; }\n");
    html.push_str(".task { background: #16213e; border-radius: 8px; padding: 15px; margin: 10px 0; border-left: 4px solid #3b82f6; }\n");
    html.push_str(".task.error { border-left-color: #ef4444; }\n");
    html.push_str(".task-name { font-weight: bold; color: #60a5fa; margin-bottom: 8px; }\n");
    html.push_str(".output { white-space: pre-wrap; font-family: 'Consolas', 'Monaco', monospace; font-size: 13px; background: #0f0f1a; padding: 10px; border-radius: 4px; overflow-x: auto; }\n");
    html.push_str(".error-msg { color: #f87171; }\n");
    html.push_str(".meta { color: #9ca3af; font-size: 12px; margin-top: 20px; }\n");
    html.push_str("</style>\n</head>\n<body>\n");
    html.push_str("<h1>WindowsForum Diagnostic Report</h1>\n");
    html.push_str("<p class=\"meta\">Generated: <span id=\"gendate\"></span></p>\n");
    html.push_str("<script>document.getElementById('gendate').textContent = new Date().toLocaleString();</script>\n");

    let results_by_category = grouped_results(results, tasks);
    let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
    categories.sort();

    for category in categories {
        html.push_str(&format!("<h2>{}</h2>\n", html_escape(&category)));

        if let Some(results) = results_by_category.get(&category) {
            for (task_id, result) in results {
                if let Some(task) = tasks.get(*task_id) {
                    let class = if result.success { "task" } else { "task error" };
                    html.push_str(&format!("<div class=\"{class}\">\n"));
                    html.push_str(&format!(
                        "<div class=\"task-name\">{}</div>\n",
                        html_escape(&task.name)
                    ));
                    html.push_str(&format!(
                        "<div class=\"meta\">Status: {} - Duration: {} ms</div>\n",
                        if result.success { "Passed" } else { "Failed" },
                        result.duration_ms
                    ));

                    if result.success {
                        if include_raw {
                            html.push_str("<div class=\"output\">");
                            if let Ok(parsed) =
                                serde_json::from_str::<serde_json::Value>(&result.output)
                            {
                                html.push_str(&html_escape(&format_json_value(&parsed, 0)));
                            } else {
                                html.push_str(&html_escape(&result.output));
                            }
                            html.push_str("</div>\n");
                        }
                    } else if let Some(error) = &result.error {
                        html.push_str(&format!(
                            "<div class=\"error-msg\">Error: {}</div>\n",
                            html_escape(error)
                        ));
                    }

                    html.push_str("</div>\n");
                }
            }
        }
    }

    html.push_str("<p class=\"meta\">Generated using WindowsForum Diagnostics Tool</p>\n");
    html.push_str("</body>\n</html>");
    html
}

/// Render JSON, text, or HTML with shipping-compatible semantics.
///
/// # Errors
///
/// Returns [`ExportError::Serialization`] if JSON serialization fails.
pub fn render_report(
    format: ReportFormat,
    include_raw: bool,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> Result<String, ExportError> {
    let tasks = task_map(tasks);
    match format {
        ReportFormat::Json if include_raw => serde_json::to_string_pretty(results)
            .map_err(|error| ExportError::Serialization(error.to_string())),
        ReportFormat::Json => serde_json::to_string_pretty(&redacted_json_results(results))
            .map_err(|error| ExportError::Serialization(error.to_string())),
        ReportFormat::Text => Ok(render_text(results, &tasks, include_raw)),
        ReportFormat::Html => Ok(render_html(results, &tasks, include_raw)),
    }
}

/// Render the payload written by Store 2.5.8's single-file Export Report
/// action.
///
/// Text exports receive the exact `Generated`/`Computer`/`OS`/`Admin Mode`
/// envelope assembled by the shipping UI. JSON and HTML exports deliberately
/// remain the undecorated core report, as do callers that continue to use
/// [`render_report`] directly (including support-package generation).
///
/// # Errors
///
/// Returns [`ExportError::Serialization`] if JSON serialization fails.
pub fn render_saved_report(
    format: ReportFormat,
    include_raw: bool,
    metadata: &ExportMetadata,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> Result<String, ExportError> {
    let report = render_report(format, include_raw, results, tasks)?;
    if format != ReportFormat::Text {
        return Ok(report);
    }

    Ok(format!(
        "=== WindowsForum Diagnostic Report ===\nGenerated: {}\nComputer: {}\nOS: {}\nAdmin Mode: {}\n{}",
        metadata.generated,
        metadata.computer_name,
        metadata.os_version,
        if metadata.is_admin { "Yes" } else { "No" },
        report,
    ))
}

/// Render the shipping `WindowsForum` post body. Delivery remains with the UI.
#[must_use]
pub fn render_windows_forum_post(
    metadata: &ExportMetadata,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> String {
    let content = render_text(results, &task_map(tasks), false);
    format!(
        "[B]WindowsForum Diagnostic Report[/B]\n[CODE]\nGenerated: {}\nComputer: {}\nOS: {}\nAdmin Mode: {}\n\n{}\n[/CODE]\n\n[I]Generated using WindowsForum Diagnostics Tool[/I]",
        metadata.generated,
        metadata.computer_name,
        metadata.os_version,
        if metadata.is_admin { "Yes" } else { "No" },
        content
    )
}

/// Render the raw `[CODE]` clipboard payload used by Copy Report.
#[must_use]
pub fn render_forum_clipboard(
    metadata: &ExportMetadata,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> String {
    let content = render_text(results, &task_map(tasks), true);
    format!(
        "[CODE]\n=== WindowsForum Diagnostic Report ===\nGenerated: {}\nComputer: {}\nOS: {}\nAdmin Mode: {}\n{}\n[/CODE]",
        metadata.generated,
        metadata.computer_name,
        metadata.os_version,
        if metadata.is_admin { "Yes" } else { "No" },
        content
    )
}

/// Render the shipping email subject, clipboard body, and short mail body.
#[must_use]
pub fn render_email(
    metadata: &ExportMetadata,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> EmailPayload {
    let content = render_text(results, &task_map(tasks), false);
    EmailPayload {
        subject: format!(
            "Diagnostic Report - {} - {}",
            metadata.computer_name, metadata.local_date
        ),
        clipboard_body: format!(
            "WindowsForum Diagnostic Report\n\nGenerated: {}\nComputer: {}\nOS: {}\n\n{}",
            metadata.generated, metadata.computer_name, metadata.os_version, content
        ),
        mailto_body: "[Report copied to clipboard - paste here with Ctrl+V]".to_string(),
    }
}

pub(crate) fn render_request(
    kind: &ExportRequestKind,
    results: &HashMap<String, TaskResult>,
    tasks: &[ExportTask],
) -> Result<ExportPayload, ExportError> {
    match kind {
        ExportRequestKind::Report {
            format,
            include_raw,
        } => render_report(*format, *include_raw, results, tasks).map(ExportPayload::Report),
        ExportRequestKind::SavedReport {
            format,
            include_raw,
            metadata,
        } => render_saved_report(*format, *include_raw, metadata, results, tasks)
            .map(ExportPayload::Report),
        ExportRequestKind::WindowsForumPost { metadata } => Ok(ExportPayload::WindowsForumPost(
            render_windows_forum_post(metadata, results, tasks),
        )),
        ExportRequestKind::ForumClipboard { metadata } => Ok(ExportPayload::ForumClipboard(
            render_forum_clipboard(metadata, results, tasks),
        )),
        ExportRequestKind::Email { metadata } => {
            Ok(ExportPayload::Email(render_email(metadata, results, tasks)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks() -> Vec<ExportTask> {
        vec![ExportTask::new("os_info", "Operating System", "System")]
    }

    fn results(output: &str) -> HashMap<String, TaskResult> {
        HashMap::from([(
            "os_info".to_string(),
            TaskResult {
                success: true,
                output: output.to_string(),
                error: None,
                duration_ms: 12,
            },
        )])
    }

    fn metadata() -> ExportMetadata {
        ExportMetadata {
            generated: "8/30/2026, 1:02:03 PM".to_string(),
            local_date: "8/30/2026".to_string(),
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11 Pro (25H2)".to_string(),
            is_admin: true,
        }
    }

    #[test]
    fn raw_and_redacted_json_match_shipping_goldens() {
        let results = results(r#"{"secret_value":"raw diagnostic payload"}"#);
        let raw = render_report(ReportFormat::Json, true, &results, &tasks()).unwrap();
        assert_eq!(
            raw,
            concat!(
                "{\n",
                "  \"os_info\": {\n",
                "    \"success\": true,\n",
                "    \"output\": \"{\\\"secret_value\\\":\\\"raw diagnostic payload\\\"}\",\n",
                "    \"error\": null,\n",
                "    \"duration_ms\": 12\n",
                "  }\n",
                "}"
            )
        );

        let redacted = render_report(ReportFormat::Json, false, &results, &tasks()).unwrap();
        assert_eq!(
            redacted,
            concat!(
                "{\n",
                "  \"os_info\": {\n",
                "    \"success\": true,\n",
                "    \"error\": null,\n",
                "    \"duration_ms\": 12\n",
                "  }\n",
                "}"
            )
        );
    }

    #[test]
    fn text_golden_preserves_json_order_humanization_and_null_rules() {
        let results = results(
            r#"{"first_key":"alpha","nullable":null,"items":[{"name":"one"},{"name":"two"}]}"#,
        );
        let text = render_report(ReportFormat::Text, true, &results, &tasks()).unwrap();
        assert_eq!(
            text,
            concat!(
                "\n=== System ===\n\n",
                "Operating System:\n",
                "  Status: Passed\n",
                "  Duration: 12 ms\n",
                "  First Key : alpha\n",
                "  Items:\n",
                "    Name : one\n",
                "    ---\n",
                "    Name : two\n",
                "\n"
            )
        );
    }

    #[test]
    fn saved_text_report_adds_the_exact_store_258_metadata_envelope() {
        let results = results(r#"{"edition":"Pro"}"#);
        let report =
            render_saved_report(ReportFormat::Text, true, &metadata(), &results, &tasks()).unwrap();

        assert_eq!(
            report,
            concat!(
                "=== WindowsForum Diagnostic Report ===\n",
                "Generated: 8/30/2026, 1:02:03 PM\n",
                "Computer: TEST-PC\n",
                "OS: Windows 11 Pro (25H2)\n",
                "Admin Mode: Yes\n",
                "\n=== System ===\n\n",
                "Operating System:\n",
                "  Status: Passed\n",
                "  Duration: 12 ms\n",
                "  Edition : Pro\n",
                "\n",
            )
        );
    }

    #[test]
    fn saved_json_and_html_remain_identical_to_undecorated_reports() {
        let results = results(r#"{"edition":"Pro"}"#);
        for format in [ReportFormat::Json, ReportFormat::Html] {
            let raw = render_report(format, true, &results, &tasks()).unwrap();
            let saved = render_saved_report(format, true, &metadata(), &results, &tasks()).unwrap();
            assert_eq!(saved, raw);
            assert!(!saved.contains("Computer: TEST-PC"));
            assert!(!saved.contains("Admin Mode: Yes"));
        }
    }

    #[test]
    fn raw_report_request_stays_undecorated_for_support_packages() {
        let results = results(r#"{"edition":"Pro"}"#);
        let payload = render_request(
            &ExportRequestKind::Report {
                format: ReportFormat::Text,
                include_raw: true,
            },
            &results,
            &tasks(),
        )
        .unwrap();
        let ExportPayload::Report(report) = payload else {
            panic!("report request returned the wrong payload variant");
        };
        assert!(report.starts_with("\n=== System ==="));
        assert!(!report.contains("Generated:"));
        assert!(!report.contains("Computer:"));
        assert!(!report.contains("Admin Mode:"));
    }

    #[test]
    fn invalid_json_and_failure_semantics_are_unchanged() {
        let raw = render_report(ReportFormat::Text, true, &results("not json"), &tasks()).unwrap();
        assert!(raw.ends_with("not json\n\n"));

        let failure = HashMap::from([(
            "os_info".to_string(),
            TaskResult {
                success: false,
                output: "must never leak".to_string(),
                error: Some("failed <badly>".to_string()),
                duration_ms: 9,
            },
        )]);
        let text = render_report(ReportFormat::Text, true, &failure, &tasks()).unwrap();
        assert!(!text.contains("must never leak"));
        assert!(text.contains("  Error: failed <badly>\n"));
    }

    #[test]
    fn html_escapes_every_dynamic_text_context() {
        let tasks = vec![ExportTask::new(
            "unsafe",
            "<script>\"name\" & task",
            "<System & Core>",
        )];
        let results = HashMap::from([(
            "unsafe".to_string(),
            TaskResult {
                success: true,
                output: "<b>\"raw\" & value</b>".to_string(),
                error: None,
                duration_ms: 1,
            },
        )]);
        let html = render_report(ReportFormat::Html, true, &results, &tasks).unwrap();
        assert!(html.contains("<h2>&lt;System &amp; Core&gt;</h2>"));
        assert!(html.contains("&lt;script&gt;&quot;name&quot; &amp; task"));
        assert!(html.contains("&lt;b&gt;&quot;raw&quot; &amp; value&lt;/b&gt;"));
        assert!(!html.contains("<script>\"name\""));
        assert!(html.contains(
            "<script>document.getElementById('gendate').textContent = new Date().toLocaleString();</script>"
        ));
        assert!(html.ends_with("</body>\n</html>"));
    }

    #[test]
    fn unknown_tasks_remain_in_json_but_not_text_or_html() {
        let unknown = HashMap::from([(
            "future_task".to_string(),
            TaskResult {
                success: true,
                output: "future".to_string(),
                error: None,
                duration_ms: 1,
            },
        )]);
        assert!(
            render_report(ReportFormat::Json, true, &unknown, &tasks())
                .unwrap()
                .contains("future_task")
        );
        assert_eq!(
            render_report(ReportFormat::Text, true, &unknown, &tasks()).unwrap(),
            ""
        );
        let html = render_report(ReportFormat::Html, true, &unknown, &tasks()).unwrap();
        assert!(!html.contains("future_task"));
    }

    #[test]
    fn forum_and_email_payloads_match_shipping_goldens() {
        let results = results("ignored when redacted");
        let forum = render_windows_forum_post(&metadata(), &results, &tasks());
        assert_eq!(
            forum,
            concat!(
                "[B]WindowsForum Diagnostic Report[/B]\n",
                "[CODE]\n",
                "Generated: 8/30/2026, 1:02:03 PM\n",
                "Computer: TEST-PC\n",
                "OS: Windows 11 Pro (25H2)\n",
                "Admin Mode: Yes\n",
                "\n",
                "\n=== System ===\n\n",
                "Operating System:\n",
                "  Status: Passed\n",
                "  Duration: 12 ms\n",
                "\n",
                "\n[/CODE]\n",
                "\n",
                "[I]Generated using WindowsForum Diagnostics Tool[/I]"
            )
        );

        let email = render_email(&metadata(), &results, &tasks());
        assert_eq!(
            email,
            EmailPayload {
                subject: "Diagnostic Report - TEST-PC - 8/30/2026".to_string(),
                clipboard_body: concat!(
                    "WindowsForum Diagnostic Report\n",
                    "\n",
                    "Generated: 8/30/2026, 1:02:03 PM\n",
                    "Computer: TEST-PC\n",
                    "OS: Windows 11 Pro (25H2)\n",
                    "\n",
                    "\n=== System ===\n\n",
                    "Operating System:\n",
                    "  Status: Passed\n",
                    "  Duration: 12 ms\n",
                    "\n"
                )
                .to_string(),
                mailto_body: "[Report copied to clipboard - paste here with Ctrl+V]".to_string(),
            }
        );
    }

    #[test]
    fn copy_report_payload_keeps_raw_output_and_exact_code_envelope() {
        let clipboard = render_forum_clipboard(&metadata(), &results("raw payload"), &tasks());
        assert_eq!(
            clipboard,
            concat!(
                "[CODE]\n",
                "=== WindowsForum Diagnostic Report ===\n",
                "Generated: 8/30/2026, 1:02:03 PM\n",
                "Computer: TEST-PC\n",
                "OS: Windows 11 Pro (25H2)\n",
                "Admin Mode: Yes\n",
                "\n=== System ===\n\n",
                "Operating System:\n",
                "  Status: Passed\n",
                "  Duration: 12 ms\n",
                "raw payload\n",
                "\n",
                "\n[/CODE]"
            )
        );
    }

    #[test]
    fn report_format_parser_is_case_sensitive() {
        assert_eq!(ReportFormat::try_from("json").unwrap(), ReportFormat::Json);
        assert_eq!(
            ReportFormat::try_from("JSON"),
            Err(ExportError::UnsupportedFormat("JSON".to_string()))
        );
    }
}
