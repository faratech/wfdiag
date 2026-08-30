//! Export-related Tauri commands.
//!
//! This module handles exporting diagnostic results in various formats
//! (JSON, text, HTML) and saving them to disk.

use crate::diagnostics;
use crate::error::DiagError;
use crate::state::AppState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::State;

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

/// Every allowed save directory — not just Temp — is restricted to the
/// app's own report extensions. Without this, Documents/Desktop/Downloads/
/// AppData would accept any filename and extension a caller supplied.
fn has_report_extension(path: &Path) -> bool {
    path.file_name()
        .and_then(|f| f.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|lower| {
            lower.ends_with(".txt") || lower.ends_with(".html") || lower.ends_with(".json")
        })
}

fn safe_report_filename(filename: &str) -> Result<&str, String> {
    let path = Path::new(filename);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DiagError::path_validation(filename.to_string(), "filename is required").to_string()
        })?;

    if name != filename || name.contains(['/', '\\']) {
        return Err(DiagError::path_validation(
            filename.to_string(),
            "filename must not contain a path",
        )
        .into());
    }

    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".txt") || lower.ends_with(".html") || lower.ends_with(".json")) {
        return Err(DiagError::path_validation(
            filename.to_string(),
            "filename must end in .txt, .html, or .json",
        )
        .into());
    }

    Ok(name)
}

fn default_export_dir() -> Result<PathBuf, String> {
    dirs::download_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::desktop_dir)
        .or_else(|| dirs::config_dir().map(|dir| dir.join("com.windowsforum.diagnostics")))
        .ok_or_else(|| DiagError::internal("Could not find an export directory").into())
}

/// Validate that the save path is within allowed scopes (security). Returns
/// the canonicalized path that was actually checked, so callers can write to
/// exactly what was validated instead of re-resolving the original string.
fn validate_save_path(path: &str) -> Result<PathBuf, String> {
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

    if !has_report_extension(&path_to_check) {
        return Err(DiagError::path_validation(
            path_to_check.display().to_string(),
            "filename must end in .txt, .html, or .json",
        )
        .into());
    }

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
            return Ok(path_to_check);
        }
    }

    Err(DiagError::path_validation(
        path.display().to_string(),
        "Path not in allowed scope. Allowed: Documents, Desktop, Downloads, AppData, or Temp (app report filenames only)",
    )
    .into())
}

#[tauri::command]
pub async fn validate_export_path(path: String) -> Result<(), String> {
    validate_save_path(&path).map(|_| ())
}

#[tauri::command]
pub async fn suggest_export_path(filename: String) -> Result<String, String> {
    let filename = safe_report_filename(&filename)?;
    Ok(default_export_dir()?
        .join(filename)
        .to_string_lossy()
        .into_owned())
}

fn export_results_content(
    format: String,
    include_raw: bool,
    results: &HashMap<String, diagnostics::TaskResult>,
) -> Result<String, String> {
    let report_format =
        wfdiag_native_export::ReportFormat::try_from(format.as_str()).map_err(|_| {
            DiagError::UnsupportedFormat {
                format: format.clone(),
            }
        })?;
    let tasks = diagnostics::get_all_tasks()
        .into_iter()
        .map(|task| wfdiag_native_export::ExportTask::new(task.id, task.name, task.category))
        .collect::<Vec<_>>();
    wfdiag_native_export::render_report(report_format, include_raw, results, &tasks).map_err(
        |error| match error {
            wfdiag_native_export::ExportError::Serialization(reason) => {
                DiagError::serialization(reason).into()
            }
            other => DiagError::internal(other.to_string()).into(),
        },
    )
}

#[tauri::command]
pub async fn export_results(
    format: String,
    include_raw: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let current = state.current_session.lock().await;
    let results = current
        .as_ref()
        .map(|session| session.results.clone())
        .ok_or_else(|| String::from(DiagError::NoActiveSession))?;
    drop(current);

    tokio::task::spawn_blocking(move || export_results_content(format, include_raw, &results))
        .await
        .map_err(|error| {
            String::from(DiagError::internal(format!(
                "Export generation task failed: {error}"
            )))
        })?
}

#[tauri::command]
pub async fn save_results_to_file(path: String, content: String) -> Result<(), String> {
    use std::fs;

    // Validate path is within allowed scopes (security fix), then write to
    // the exact canonicalized path that was validated rather than
    // re-resolving the original string — shrinks the window in which a
    // swapped path component between validation and write could matter.
    let validated_path = validate_save_path(&path)?;

    fs::write(&validated_path, &content)
        .map_err(|e| DiagError::file(path.clone(), e.to_string()))?;
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

    #[test]
    fn has_report_extension_applies_outside_temp_too() {
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Documents\wf-diagnostics.txt"
        )));
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Desktop\report.html"
        )));
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Downloads\report.JSON"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Desktop\run.bat"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Documents\startup.vbs"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Documents\noext"
        )));
    }

    #[test]
    fn safe_report_filename_rejects_paths_and_non_report_extensions() {
        assert_eq!(
            safe_report_filename("wf-diagnostics-2026-06-22.txt").unwrap(),
            "wf-diagnostics-2026-06-22.txt"
        );
        assert!(safe_report_filename("../report.txt").is_err());
        assert!(safe_report_filename("nested/report.txt").is_err());
        assert!(safe_report_filename("report.exe").is_err());
    }
}
