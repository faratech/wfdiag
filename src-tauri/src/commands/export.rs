//! Export-related Tauri commands.
//!
//! This module handles exporting diagnostic results in various formats
//! (JSON, text, HTML) and saving them to disk.

use crate::diagnostics::{self, DiagnosticTask};
use crate::error::DiagError;
use crate::state::AppState;
use std::collections::HashMap;
use std::path::Path;
use tauri::State;

/// Format JSON values into readable text with indentation
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
                        result.push_str(&format!("{}{}:\n", indent, formatted_key));
                        result.push_str(&format_json_value(val, indent_level + 1));
                    }
                    serde_json::Value::Null => {
                        // Skip null values
                    }
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
        serde_json::Value::Array(arr) => {
            let mut result = String::new();
            for (i, val) in arr.iter().enumerate() {
                if i > 0 {
                    result.push_str(&format!("{}---\n", indent));
                }
                result.push_str(&format_json_value(val, indent_level));
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

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Get list of allowed directories for file saves (matching Tauri capabilities)
fn get_allowed_save_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    // $APPDATA/wfdiag-tauri/ (config dir)
    if let Some(appdata) = dirs::config_dir() {
        paths.push(appdata.join("wfdiag-tauri"));
        paths.push(appdata.join("com.windowsforum.diagnostics"));
    }

    // $HOME/Documents/
    if let Some(docs) = dirs::document_dir() {
        paths.push(docs);
    }

    // $HOME/Desktop/
    if let Some(desktop) = dirs::desktop_dir() {
        paths.push(desktop);
    }

    // $HOME/Downloads/
    if let Some(downloads) = dirs::download_dir() {
        paths.push(downloads);
    }

    // $TEMP directory (with pattern restriction enforced in validate_save_path)
    if let Ok(temp) = std::env::var("TEMP") {
        paths.push(std::path::PathBuf::from(temp));
    } else if let Ok(tmp) = std::env::var("TMP") {
        paths.push(std::path::PathBuf::from(tmp));
    }

    paths
}

fn temp_filename_allowed(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let has_text_or_html = lower.ends_with(".txt") || lower.ends_with(".html");
    let has_report_extension = has_text_or_html || lower.ends_with(".json");

    (lower.starts_with("wfdiag_") && has_text_or_html)
        || (lower.starts_with("wf-diagnostics-") && has_report_extension)
        || (lower.starts_with("support-package-") && has_report_extension)
}

/// Validate that the save path is within allowed scopes (security)
fn validate_save_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);

    // Get the path to validate (canonicalize parent for new files)
    let path_to_check = if path.exists() {
        path.canonicalize()
            .map_err(|e| DiagError::path_validation(path.display().to_string(), e.to_string()))?
    } else {
        // For new files, check the parent directory exists and is allowed
        let parent = path.parent().ok_or_else(|| {
            DiagError::path_validation(path.display().to_string(), "no parent directory")
        })?;
        if !parent.exists() {
            return Err(DiagError::ParentNotExists {
                path: parent.display().to_string(),
            }
            .into());
        }
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| DiagError::path_validation(parent.display().to_string(), e.to_string()))?;
        canonical_parent.join(path.file_name().unwrap_or_default())
    };

    let allowed_paths = get_allowed_save_paths();

    // Check if path falls within any allowed scope
    for allowed in &allowed_paths {
        // Canonicalize allowed path for comparison (skip if it doesn't exist)
        let canonical_allowed = match allowed.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };

        if path_to_check.starts_with(&canonical_allowed) {
            // Additional check for TEMP directory: filename must match wfdiag_* pattern
            if let Ok(temp) = std::env::var("TEMP").or_else(|_| std::env::var("TMP")) {
                let temp_path = std::path::PathBuf::from(&temp);
                if let Ok(canonical_temp) = temp_path.canonicalize() {
                    if path_to_check.starts_with(&canonical_temp)
                        && !path_to_check.starts_with(canonical_allowed.clone())
                    {
                        // This is in TEMP but matched a different allowed path, continue checking
                        continue;
                    }
                    if path_to_check.starts_with(&canonical_temp)
                        && let Some(filename) = path_to_check.file_name().and_then(|f| f.to_str())
                        && !temp_filename_allowed(filename)
                    {
                        // Restrict Temp saves to filenames generated by this app.
                        return Err(DiagError::path_validation(
                            path_to_check.display().to_string(),
                            "Temp files must use a WindowsForum Diagnostics report filename",
                        )
                        .into());
                    }
                }
            }
            return Ok(());
        }
    }

    Err(DiagError::path_validation(
        path.display().to_string(),
        "Path not in allowed scope. Allowed: Documents, Desktop, Downloads, AppData, or Temp (app report filenames only)",
    )
    .into())
}

fn redacted_json_results(
    results: &HashMap<String, diagnostics::TaskResult>,
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

fn grouped_results<'a>(
    results: &'a HashMap<String, diagnostics::TaskResult>,
    task_map: &'a HashMap<String, &'a DiagnosticTask>,
) -> HashMap<String, Vec<(&'a String, &'a diagnostics::TaskResult)>> {
    let mut results_by_category: HashMap<String, Vec<(&String, &diagnostics::TaskResult)>> =
        HashMap::new();

    for (task_id, result) in results {
        if let Some(task) = task_map.get(task_id) {
            results_by_category
                .entry(task.category.clone())
                .or_default()
                .push((task_id, result));
        }
    }

    results_by_category
}

fn format_text_results(
    results: &HashMap<String, diagnostics::TaskResult>,
    task_map: &HashMap<String, &DiagnosticTask>,
    include_raw: bool,
) -> String {
    let mut text = String::new();
    let results_by_category = grouped_results(results, task_map);

    let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
    categories.sort();

    for category in categories {
        text.push_str(&format!("\n=== {} ===\n\n", category));

        if let Some(results) = results_by_category.get(&category) {
            for (task_id, result) in results {
                if let Some(task) = task_map.get(*task_id) {
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
                        text.push_str(&format!("  Error: {}\n", error));
                    }

                    text.push('\n');
                }
            }
        }
    }

    text
}

fn format_html_results(
    results: &HashMap<String, diagnostics::TaskResult>,
    task_map: &HashMap<String, &DiagnosticTask>,
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

    let results_by_category = grouped_results(results, task_map);
    let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
    categories.sort();

    for category in categories {
        html.push_str(&format!("<h2>{}</h2>\n", html_escape(&category)));

        if let Some(results) = results_by_category.get(&category) {
            for (task_id, result) in results {
                if let Some(task) = task_map.get(*task_id) {
                    let class = if result.success { "task" } else { "task error" };
                    html.push_str(&format!("<div class=\"{}\">\n", class));
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

fn export_results_content(
    format: String,
    include_raw: bool,
    results: &HashMap<String, diagnostics::TaskResult>,
) -> Result<String, String> {
    let all_tasks = diagnostics::get_all_tasks();
    let task_map: HashMap<String, &DiagnosticTask> =
        all_tasks.iter().map(|t| (t.id.clone(), t)).collect();

    match format.as_str() {
        "json" if include_raw => serde_json::to_string_pretty(results)
            .map_err(|e| DiagError::serialization(e.to_string()).into()),
        "json" => serde_json::to_string_pretty(&redacted_json_results(results))
            .map_err(|e| DiagError::serialization(e.to_string()).into()),
        "text" => Ok(format_text_results(results, &task_map, include_raw)),
        "html" => Ok(format_html_results(results, &task_map, include_raw)),
        _ => Err(DiagError::UnsupportedFormat { format }.into()),
    }
}

#[tauri::command]
pub async fn export_results(
    format: String,
    include_raw: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        export_results_content(format, include_raw, &session.results)
    } else {
        Err(DiagError::NoActiveSession.into())
    }
}

#[tauri::command]
pub async fn save_results_to_file(path: String, content: String) -> Result<(), String> {
    use std::fs;

    // Validate path is within allowed scopes (security fix)
    validate_save_path(&path)?;

    fs::write(&path, &content).map_err(|e| DiagError::file(path.clone(), e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_results() -> HashMap<String, diagnostics::TaskResult> {
        HashMap::from([
            (
                "os_info".to_string(),
                diagnostics::TaskResult {
                    success: true,
                    output: "{\"secret_value\":\"raw diagnostic payload\"}".to_string(),
                    error: None,
                    duration_ms: 12,
                },
            ),
            (
                "dism_health".to_string(),
                diagnostics::TaskResult {
                    success: false,
                    output: String::new(),
                    error: Some("DISM failed".to_string()),
                    duration_ms: 34,
                },
            ),
        ])
    }

    #[test]
    fn include_raw_false_omits_success_payload_but_keeps_status_and_errors() {
        let text = export_results_content("text".to_string(), false, &sample_results()).unwrap();
        assert!(text.contains("Operating System:"));
        assert!(text.contains("Status: Passed"));
        assert!(text.contains("Duration: 12 ms"));
        assert!(!text.contains("raw diagnostic payload"));
        assert!(text.contains("DISM Health Check:"));
        assert!(text.contains("Status: Failed"));
        assert!(text.contains("Error: DISM failed"));
    }

    #[test]
    fn include_raw_true_keeps_success_payload() {
        let text = export_results_content("text".to_string(), true, &sample_results()).unwrap();
        assert!(text.contains("Secret Value : raw diagnostic payload"));
    }

    #[test]
    fn temp_filename_rules_allow_app_reports_only() {
        for filename in [
            "wfdiag_report.txt",
            "wfdiag_report.html",
            "wf-diagnostics-2026-06-22.txt",
            "wf-diagnostics-2026-06-22.html",
            "wf-diagnostics-2026-06-22.json",
            "support-package-2026-06-22T12-00-00.json",
            "support-package-2026-06-22T12-00-00.txt",
            "support-package-2026-06-22T12-00-00.html",
        ] {
            assert!(temp_filename_allowed(filename), "{filename}");
        }

        for filename in [
            "wfdiag_report.json",
            "support-package-2026-06-22.exe",
            "other-report.txt",
            "wf-diagnostics-2026-06-22.exe",
        ] {
            assert!(!temp_filename_allowed(filename), "{filename}");
        }
    }
}
