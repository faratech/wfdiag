//! Export-related Tauri commands.
//!
//! This module handles exporting diagnostic results in various formats
//! (JSON, text, HTML) and saving them to disk.
//!
//! The destination policy — allowed roots, report extensions, the Temp
//! filename restriction, and the canonicalize-then-check ordering — is shared
//! with the native shell and lives in `wfdiag_native_export::path_policy`.
//! Only the shell-folder resolution (`dirs`) and the `DiagError` wire mapping
//! stay here.

use crate::diagnostics;
use crate::error::DiagError;
use crate::state::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;
use wfdiag_native_export::path_policy::{
    ExportExtensionRule, ExportPathError, ExportPathErrorKind, ExportPathPolicy, process_temp_dir,
    safe_report_filename, validate_report_path_with_policy,
};

/// Resolve the shared policy from this user's shell folders.
///
/// `dirs::config_dir()` is Roaming AppData on Windows, which is where both the
/// legacy `wfdiag-tauri` and current `com.windowsforum.diagnostics` data
/// directories live; the crate appends both.
fn current_user_export_policy() -> ExportPathPolicy {
    ExportPathPolicy::for_user_folders(
        dirs::document_dir(),
        dirs::desktop_dir(),
        dirs::download_dir(),
        dirs::config_dir(),
        process_temp_dir(),
    )
}

/// Map a shared policy refusal onto this command surface's error wire format.
fn path_error(error: &ExportPathError) -> String {
    match error.kind() {
        ExportPathErrorKind::ParentMissing => DiagError::ParentNotExists {
            path: error
                .path()
                .parent()
                .unwrap_or_else(|| error.path())
                .display()
                .to_string(),
        }
        .into(),
        _ => {
            DiagError::path_validation(error.path().display().to_string(), error.to_string()).into()
        }
    }
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
    validate_report_path_with_policy(
        std::path::Path::new(path),
        ExportExtensionRule::AnyReport,
        &current_user_export_policy(),
    )
    .map_err(|error| path_error(&error))
}

#[tauri::command]
pub async fn validate_export_path(path: String) -> Result<(), String> {
    validate_save_path(&path).map(|_| ())
}

#[tauri::command]
pub async fn suggest_export_path(filename: String) -> Result<String, String> {
    let filename = safe_report_filename(&filename).map_err(|error| path_error(&error))?;
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
}
