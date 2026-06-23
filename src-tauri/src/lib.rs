#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod architecture;
mod commands;
pub mod diagnostics;
// dpapi self-gates internally (non-Windows stubs); the module stays
// cross-platform because ProviderKeyId names the keyring entries too
mod dpapi;
mod encrypted_storage;
pub mod error;
mod issue_catalog;
mod issue_detector;
mod native_diagnostics;
pub mod native_monitor;
mod remediation;
pub mod results_storage;
mod security;
#[cfg(windows)]
mod sparse_identity;
pub mod state;
pub mod timestamp;
mod tray;
mod update_check;
mod windows_native;
#[cfg(windows)]
mod wmi_native;

// Phi Silica wrapper (uses windows-bindgen generated bindings)
pub mod phi_silica;
#[cfg(windows)]
mod windows_ai_bindings;

// Unified AI service layer
mod ai_cache;
mod ai_chat;
mod ai_fix_plan;
mod ai_grounding;
mod ai_prompts;
pub mod ai_providers;
mod ai_report;
mod ai_service;
mod ai_tools;
mod mcp_client;

#[cfg(windows)]
mod adapter_monitor;

use crate::diagnostics::{DiagnosticTask, TaskResult};
use crate::error::DiagError;
use crate::issue_catalog::Issue;
use native_monitor::{NetworkConnection, SystemMonitor};
use results_storage::{ComparisonResult, ScanRecord, ScanStorage, ScanSummary, TaskTrend};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::Emitter;
use tauri::State;

// Re-export settings types from commands module
pub use commands::AppSettings;

// Use state types from state module
use state::{AppState, DiagnosticSession, SystemInfo};

// Re-export load_api_key_internal for ai_service module
pub(crate) use commands::settings::load_api_key_internal;

/// Get detailed Windows version information from registry
#[cfg(windows)]
fn get_windows_version_info() -> String {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    if let Ok(cv_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion") {
        let product_name = cv_key
            .get_value::<String, _>("ProductName")
            .unwrap_or_else(|_| "Windows".to_string());
        let display_version = cv_key.get_value::<String, _>("DisplayVersion").ok();
        let edition_id = cv_key.get_value::<String, _>("EditionID").ok();
        let current_build = cv_key.get_value::<String, _>("CurrentBuild").ok();

        // Determine Windows version (10 or 11) based on build number
        let is_win11 = if let Some(build) = current_build.as_ref() {
            match build.parse::<u32>() {
                Ok(num) => num >= 22000,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to parse Windows build number '{}': {}",
                        build, e
                    );
                    // Fall back to checking product name
                    product_name.contains("Windows 11")
                }
            }
        } else {
            product_name.contains("Windows 11")
        };

        // Build the version string
        let mut version_parts = Vec::new();

        // Windows 10 or 11
        if is_win11 {
            version_parts.push("Windows 11".to_string());
        } else if product_name.contains("Windows 10") {
            version_parts.push("Windows 10".to_string());
        } else {
            // Fallback to product name if not 10/11
            version_parts.push(product_name.clone());
        }

        // Add edition (Home, Pro, Enterprise, etc.)
        if let Some(edition) = edition_id {
            version_parts.push(edition);
        }

        // Add version number (e.g., 23H2, 22H2)
        if let Some(display_ver) = display_version {
            version_parts.push(format!("({})", display_ver));
        }

        return version_parts.join(" ");
    }

    "Windows".to_string()
}

#[tauri::command]
async fn get_system_info() -> Result<SystemInfo, String> {
    let computer_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string());

    #[cfg(windows)]
    let os_version = get_windows_version_info();

    #[cfg(not(windows))]
    let os_version = std::env::var("OS").unwrap_or_else(|_| "Unknown".to_string());

    #[cfg(windows)]
    let is_admin = {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = HANDLE(std::ptr::null_mut());
            let mut is_elevated = false;

            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
                let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
                let mut ret_len = 0u32;

                if GetTokenInformation(
                    token,
                    TokenElevation,
                    Some(&mut elevation as *mut _ as *mut _),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut ret_len,
                )
                .is_ok()
                {
                    is_elevated = elevation.TokenIsElevated != 0;
                }
            }

            is_elevated
        }
    };

    #[cfg(not(windows))]
    let is_admin = false;

    Ok(SystemInfo {
        computer_name,
        os_version,
        is_admin,
    })
}

#[tauri::command]
async fn get_available_tasks() -> Result<Vec<DiagnosticTask>, String> {
    Ok(diagnostics::get_all_tasks())
}

#[tauri::command]
async fn start_diagnostics(
    task_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    use uuid::Uuid;

    // Create consistent scan ID that will be used throughout
    let session_id = format!("scan_{}", Uuid::new_v4().simple());

    println!("Starting new diagnostic session: {}", session_id);

    // Validate task IDs
    let available_tasks = diagnostics::get_all_tasks();
    let valid_task_ids: Vec<String> = task_ids
        .into_iter()
        .filter(|id| available_tasks.iter().any(|task| task.id == *id))
        .collect();

    if valid_task_ids.is_empty() {
        return Err(DiagError::NoValidTasks.into());
    }

    println!(
        "Session {} will run {} tasks",
        session_id,
        valid_task_ids.len()
    );

    let session = DiagnosticSession {
        session_id: session_id.clone(),
        start_time: std::time::SystemTime::now(),
        selected_tasks: valid_task_ids,
        results: HashMap::new(),
    };

    let mut current = state.current_session.lock().await;
    *current = Some(session);
    drop(current);

    println!("Diagnostic session {} started successfully", session_id);

    Ok(session_id)
}

/// Cancel a running diagnostic session. Cancellation is task-granular:
/// tasks already in flight finish normally, queued tasks are skipped, and the
/// pending `run_diagnostics_parallel` call resolves quickly with the results
/// collected so far.
#[tauri::command]
async fn cancel_diagnostics(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    println!("Cancelling diagnostic session: {}", session_id);
    state.cancelled_sessions.lock().await.insert(session_id);
    Ok(())
}

#[tauri::command]
async fn run_diagnostic_task(
    task_id: String,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<TaskResult, String> {
    // Find the task details
    let tasks = diagnostics::get_all_tasks();
    let task = tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| DiagError::TaskNotFound {
            task_id: task_id.clone(),
        })?;

    let session_id = {
        let current = state.current_session.lock().await;
        current.as_ref().map(|s| s.session_id.clone())
    };

    // Emit progress event
    window
        .emit(
            "task-progress",
            serde_json::json!({
                "session_id": &session_id,
                "task_id": &task_id,
                "status": "running",
                "task_name": &task.name,
            }),
        )
        .map_err(|e| DiagError::internal(format!("Failed to emit event: {}", e)))?;

    // Run the diagnostic task
    let result = diagnostics::run_diagnostic_task(&task_id).await;

    // Store result
    let mut current = state.current_session.lock().await;
    if let Some(ref mut session) = *current
        && session_id.as_ref() == Some(&session.session_id)
    {
        session.results.insert(task_id.clone(), result.clone());
    }

    // Emit completion event
    window
        .emit(
            "task-progress",
            serde_json::json!({
                "session_id": &session_id,
                "task_id": &task_id,
                "status": "completed",
                "success": result.success,
            }),
        )
        .map_err(|e| DiagError::internal(format!("Failed to emit event: {}", e)))?;

    Ok(result)
}

#[tauri::command]
async fn run_diagnostics_parallel(
    task_ids: Vec<String>,
    max_concurrent: Option<usize>,
    session_id: String,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<Vec<(String, TaskResult)>, String> {
    use futures::stream::{self, StreamExt};

    let max_concurrent = max_concurrent.unwrap_or(5).clamp(1, 16); // Default to 5, bounded
    let tasks = std::sync::Arc::new(diagnostics::get_all_tasks());
    {
        let mut active_runners = state.active_scan_runners.lock().await;
        if !active_runners.insert(session_id.clone()) {
            return Err(format!(
                "Diagnostics are already running for session {}",
                session_id
            ));
        }
    }

    // Create futures for each diagnostic task
    let futures: Vec<_> = task_ids
        .into_iter()
        .map(|task_id| {
            let window_clone = window.clone();
            let state_clone = state.inner();
            let tasks_ref = tasks.clone(); // Arc clone is cheap
            let session_id = session_id.clone();

            async move {
                // Skip queued tasks of a cancelled session before doing any work.
                // No events are emitted: the frontend has already torn down its
                // progress UI when it requested cancellation.
                if state_clone
                    .cancelled_sessions
                    .lock()
                    .await
                    .contains(&session_id)
                {
                    return Err::<(String, TaskResult), String>(format!(
                        "session {} cancelled",
                        session_id
                    ));
                }

                // Find task details
                let task = match tasks_ref.iter().find(|t| t.id == task_id) {
                    Some(task) => task,
                    None => {
                        // Emit a terminal event so the frontend's completed/total progress
                        // counter doesn't stall below 100% for an unknown task id (which
                        // would otherwise return before emitting any "completed" event).
                        let _ = window_clone.emit(
                            "task-progress",
                            serde_json::json!({
                                "session_id": &session_id,
                                "task_id": &task_id,
                                "status": "completed",
                                "success": false,
                            }),
                        );
                        return Err::<(String, TaskResult), String>(
                            DiagError::TaskNotFound {
                                task_id: task_id.clone(),
                            }
                            .into(),
                        );
                    }
                };

                // Emit progress event
                window_clone
                    .emit(
                        "task-progress",
                        serde_json::json!({
                            "session_id": &session_id,
                            "task_id": &task_id,
                            "status": "running",
                            "task_name": &task.name,
                        }),
                    )
                    .map_err(|e| DiagError::internal(format!("Failed to emit event: {}", e)))?;

                // Run the diagnostic task
                let result = diagnostics::run_diagnostic_task(&task_id).await;

                // Store result ONLY if the active session is still the one this run
                // belongs to. Without this, a scan started while a prior parallel run is
                // still draining would have the prior run's in-flight tasks write into the
                // NEW session, mixing two scans' results.
                {
                    let mut current = state_clone.current_session.lock().await;
                    if let Some(ref mut session) = *current
                        && session.session_id == session_id
                    {
                        session.results.insert(task_id.clone(), result.clone());
                    }
                }

                // Emit completion event
                window_clone
                    .emit(
                        "task-progress",
                        serde_json::json!({
                            "session_id": &session_id,
                            "task_id": &task_id,
                            "status": "completed",
                            "success": result.success,
                        }),
                    )
                    .map_err(|e| DiagError::internal(format!("Failed to emit event: {}", e)))?;

                Ok::<(String, TaskResult), String>((task_id, result))
            }
        })
        .collect();

    // Run tasks concurrently with limited parallelism
    let results: Vec<Result<(String, TaskResult), String>> = stream::iter(futures)
        .buffer_unordered(max_concurrent)
        .collect()
        .await;

    // Collect successful results and propagate errors
    let mut successful_results = Vec::new();
    for result in results {
        match result {
            Ok(task_result) => successful_results.push(task_result),
            Err(e) => eprintln!("Task failed: {}", e),
        }
    }

    state.cancelled_sessions.lock().await.remove(&session_id);
    state.active_scan_runners.lock().await.remove(&session_id);

    Ok(successful_results)
}

#[tauri::command]
async fn get_session_results(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<HashMap<String, TaskResult>, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        if session.session_id == session_id {
            Ok(session.results.clone())
        } else {
            Err(DiagError::SessionMismatch {
                expected: session.session_id.clone(),
                actual: session_id,
            }
            .into())
        }
    } else {
        Err(DiagError::NoActiveSession.into())
    }
}

#[tauri::command]
async fn get_uptime() -> Result<serde_json::Value, String> {
    let uptime_seconds = native_monitor::get_uptime_seconds();
    let days = uptime_seconds / 86400;
    let hours = (uptime_seconds % 86400) / 3600;
    let minutes = (uptime_seconds % 3600) / 60;
    let seconds = uptime_seconds % 60;

    let formatted = if days > 0 {
        format!("{} days, {}:{:02}:{:02}", days, hours, minutes, seconds)
    } else if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{}:{:02}", minutes, seconds)
    };

    Ok(serde_json::json!({
        "seconds": uptime_seconds,
        "formatted": formatted,
        "days": days,
        "hours": hours,
        "minutes": minutes,
        "seconds_remainder": seconds
    }))
}

/// Run a remediation from the vetted catalog. The Repair-tier confirmation
/// gate is enforced in remediation::execute, not here and not in the UI.
#[tauri::command]
async fn run_remediation(
    remediation_id: String,
    confirmed: Option<bool>,
) -> Result<remediation::FixOutcome, String> {
    remediation::execute(
        &remediation_id,
        confirmed.unwrap_or(false),
        &remediation::RealRunner,
    )
    .await
}

/// The full remediation catalog (the Maintenance section renders the
/// `maintenance: true` subset).
#[tauri::command]
fn get_remediations() -> Result<Vec<remediation::RemediationSummary>, String> {
    Ok(remediation::remediations()
        .iter()
        .map(|spec| spec.summary())
        .collect())
}

#[tauri::command]
async fn restart_as_admin() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::env;
        use std::ptr;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::PCWSTR;

        if update_check::is_store_install() {
            return Err("Restart as administrator is not supported for the Microsoft Store version. The app will stay open; install the MSI/NSIS build for elevated relaunch, or run admin-only tools manually.".to_string());
        }

        let exe_path = env::current_exe()
            .map_err(|e| DiagError::internal(format!("Failed to get exe path: {}", e)))?;

        // Convert path to wide string for Windows API
        let exe_path_str = exe_path.to_string_lossy().to_string();
        let exe_wide: Vec<u16> = exe_path_str
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let runas_wide: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            // Use ShellExecuteW with "runas" verb to trigger UAC elevation
            let result = ShellExecuteW(
                None,
                PCWSTR(runas_wide.as_ptr()),
                PCWSTR(exe_wide.as_ptr()),
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SW_SHOWNORMAL,
            );

            // ShellExecuteW returns a value > 32 on success
            if result.0 as i32 <= 32 {
                return Err(DiagError::internal(format!(
                    "Failed to restart with elevation. Error code: {}",
                    result.0 as i32
                ))
                .into());
            }
        }

        // Give the elevated process time to start before exiting
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Exit current process
        std::process::exit(0);
    }

    #[cfg(not(windows))]
    {
        Err(DiagError::PlatformNotSupported {
            operation: "Administrator restart".to_string(),
        }
        .into())
    }
}

#[tauri::command]
async fn start_monitoring(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    include_process_adapter_stats: Option<bool>,
) -> Result<u64, String> {
    let lease_id = state.monitoring_lease.fetch_add(1, Ordering::SeqCst) + 1;
    let mut monitor_opt = state.system_monitor.lock().await;

    // Create a new monitor if none exists
    if monitor_opt.is_none() {
        *monitor_opt = Some(SystemMonitor::new(app_handle));
    }

    // Start monitoring
    if let Some(monitor) = monitor_opt.as_ref() {
        monitor
            .start_monitoring(include_process_adapter_stats.unwrap_or(false))
            .await;
        Ok(lease_id)
    } else {
        Err(DiagError::monitor_failed("Failed to create system monitor").into())
    }
}

#[tauri::command]
async fn stop_monitoring(lease_id: Option<u64>, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(lease_id) = lease_id
        && state.monitoring_lease.load(Ordering::SeqCst) != lease_id
    {
        return Ok(());
    }

    let monitor_opt = state.system_monitor.lock().await;

    if let Some(monitor) = monitor_opt.as_ref() {
        monitor.stop_monitoring().await;
        Ok(())
    } else {
        Err(DiagError::NoActiveMonitoring.into())
    }
}

#[tauri::command]
async fn get_current_stats(
    state: State<'_, AppState>,
) -> Result<native_monitor::SystemStats, String> {
    let monitor_opt = state.system_monitor.lock().await;

    if let Some(monitor) = monitor_opt.as_ref() {
        Ok(monitor.get_current_stats().await)
    } else {
        Err(DiagError::NoActiveMonitoring.into())
    }
}

#[tauri::command]
async fn get_network_connections() -> Result<Vec<NetworkConnection>, String> {
    Ok(native_monitor::get_network_connections().await)
}

#[tauri::command]
async fn save_current_scan(
    state: State<'_, AppState>,
    duration_ms: u64,
    tags: Vec<String>,
) -> Result<String, String> {
    let session = state.current_session.lock().await;

    if let Some(session) = session.as_ref() {
        let system_info = get_system_info().await?;

        let success_count = session.results.values().filter(|r| r.success).count();
        let failure_count = session.results.values().filter(|r| !r.success).count();

        // Use the session ID directly - no more ID mismatch!
        let scan_record = ScanRecord {
            id: session.session_id.clone(),
            timestamp: timestamp::Timestamp::now(),
            computer_name: system_info.computer_name,
            os_version: system_info.os_version,
            is_admin: system_info.is_admin,
            results: session.results.clone(),
            task_count: session.results.len(),
            success_count,
            failure_count,
            duration_ms,
            tags,
        };

        println!("Auto-saving scan with session ID: {}", scan_record.id);
        println!("Scan has {} task results", scan_record.results.len());

        let storage = state.scan_storage.lock().await;
        match storage.as_ref() {
            Some(storage) => {
                storage.save_scan(&scan_record)?;
                println!("Scan auto-saved successfully: {}", scan_record.id);
                Ok(scan_record.id)
            }
            None => {
                let error = state.scan_storage_error.lock().await;
                Err(DiagError::storage(
                    "save_scan",
                    error.as_deref().unwrap_or("Storage initialization failed"),
                )
                .into())
            }
        }
    } else {
        Err(DiagError::NoActiveSession.into())
    }
}

#[tauri::command]
async fn list_scan_history(state: State<'_, AppState>) -> Result<Vec<ScanSummary>, String> {
    println!("Listing scan history");
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => match storage.list_scans() {
            Ok(scans) => {
                println!("Found {} scans in history", scans.len());
                for scan in &scans {
                    println!(
                        "  - Scan ID: {}, Timestamp: {}, Tasks: {}",
                        scan.id, scan.timestamp, scan.task_count
                    );
                }
                Ok(scans)
            }
            Err(e) => {
                println!("Error listing scans: {}", e);
                Err(e)
            }
        },
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "list_scans",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn load_scan(state: State<'_, AppState>, scan_id: String) -> Result<ScanRecord, String> {
    println!("Loading scan: {}", scan_id);
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.load_scan(&scan_id),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "load_scan",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn compare_scans(
    state: State<'_, AppState>,
    current_id: String,
    previous_id: String,
) -> Result<ComparisonResult, String> {
    println!("Comparing scans: '{}' vs '{}'", current_id, previous_id);
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => match storage.compare_scans(&current_id, &previous_id) {
            Ok(comparison) => {
                println!("Comparison successful:");
                println!(
                    "  - Current scan: {} ({})",
                    comparison.current_scan.id, comparison.current_scan.timestamp
                );
                println!(
                    "  - Previous scan: {} ({})",
                    comparison.previous_scan.id, comparison.previous_scan.timestamp
                );
                println!("  - Total changes: {}", comparison.total_changes);
                println!("  - New failures: {}", comparison.new_failures.len());
                println!("  - New successes: {}", comparison.new_successes.len());
                Ok(comparison)
            }
            Err(e) => {
                println!("Error comparing scans: {}", e);
                Err(e)
            }
        },
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "compare_scans",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn clear_scan_history(state: State<'_, AppState>) -> Result<String, String> {
    println!("Clearing scan history via Tauri command...");

    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => match storage.clear_history() {
            Ok(_) => {
                println!("Scan history cleared successfully");
                Ok("Scan history cleared successfully".to_string())
            }
            Err(e) => {
                println!("Failed to clear scan history: {}", e);
                Err(e)
            }
        },
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "clear_history",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn update_scan_tags(
    state: State<'_, AppState>,
    scan_id: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.update_tags(&scan_id, tags),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "update_tags",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn get_task_trends(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<TaskTrend>, String> {
    // Cap the window: every scan in it is loaded and decrypted
    let limit = limit.unwrap_or(10).min(20);
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.task_failure_trends(limit),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "task_trends",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn detect_issues(state: State<'_, AppState>) -> Result<Vec<Issue>, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        // The OS dependencies live here, not in the detectors: inject the
        // clock and the temp-dir entry count so detection stays pure.
        let temp_file_count = std::fs::read_dir(std::env::temp_dir())
            .ok()
            .map(|entries| entries.count());
        let ctx = issue_catalog::DetectCtx {
            results: &session.results,
            now: timestamp::Timestamp::now(),
            temp_file_count,
        };
        Ok(issue_catalog::detect_all(&ctx))
    } else {
        Err(DiagError::NoActiveSession.into())
    }
}

#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    let parsed =
        url::Url::parse(&url).map_err(|_| DiagError::internal("Invalid URL".to_string()))?;
    match parsed.scheme() {
        "http" | "https" | "mailto" => {}
        scheme => {
            return Err(
                DiagError::internal(format!("URL scheme '{}' is not allowed", scheme)).into(),
            );
        }
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::PCWSTR;

        let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let result = ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.as_ptr()),
                PCWSTR(std::ptr::null()),
                PCWSTR(std::ptr::null()),
                SW_SHOWNORMAL,
            );

            if result.0 as i32 <= 32 {
                return Err(DiagError::internal(format!(
                    "Failed to open URL. Error code: {}",
                    result.0 as i32
                ))
                .into());
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // For non-Windows systems, we'd need the opener crate
        return Err(DiagError::PlatformNotSupported {
            operation: "URL opening".to_string(),
        }
        .into());
    }

    Ok(())
}

#[tauri::command]
async fn copy_minidumps_to_desktop() -> Result<serde_json::Value, String> {
    let diagnostics = native_diagnostics::NativeDiagnostics::new()
        .map_err(|e| DiagError::internal(format!("Failed to initialize diagnostics: {}", e)))?;
    diagnostics
        .copy_minidumps_to_desktop()
        .map_err(|e| DiagError::internal(e.to_string()).into())
}

#[tauri::command]
async fn get_architecture_info() -> Result<serde_json::Value, String> {
    architecture::get_architecture_json().map_err(|e| DiagError::internal(e.to_string()).into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Phi Silica is Store-only: it requires registered package identity, and
    // an unpackaged process is denied (0x80070005) even on the direct DLL
    // activation path. Loose builds therefore do not attempt any identity
    // registration — the AI service routes them to Foundry Local or OpenAI.
    #[cfg(windows)]
    if sparse_identity::has_package_identity() {
        println!("Package identity: present");
    } else {
        println!("Package identity: none (Phi Silica unavailable; local AI via Foundry Local)");
    }

    // Initialize scan storage gracefully - don't crash if it fails
    let (scan_storage, scan_storage_error) = match ScanStorage::new() {
        Ok(storage) => (Some(storage), None),
        Err(e) => {
            eprintln!(
                "Warning: Failed to initialize scan storage: {}. Scan history features will be unavailable.",
                e
            );
            (None, Some(e.to_string()))
        }
    };

    let app_state = AppState::new(scan_storage, scan_storage_error);

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            commands::settings::sync_in_memory_state_from_disk();
            tray::setup_tray(app.handle())?;
            // Pre-initialize NPU detection in background to avoid delay on first monitoring start
            #[cfg(windows)]
            std::thread::spawn(|| {
                native_monitor::prewarm_npu_cache();
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && tray::close_to_tray_enabled()
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::save_settings,
            commands::settings::load_settings,
            commands::settings::store_api_key,
            commands::settings::load_api_key,
            commands::settings::clear_api_key,
            commands::settings::store_provider_api_key,
            commands::settings::clear_provider_api_key,
            get_system_info,
            get_architecture_info,
            get_available_tasks,
            start_diagnostics,
            cancel_diagnostics,
            run_diagnostic_task,
            run_diagnostics_parallel,
            get_session_results,
            commands::export::export_results,
            commands::export::save_results_to_file,
            commands::export::suggest_export_path,
            commands::export::validate_export_path,
            get_uptime,
            run_remediation,
            get_remediations,
            restart_as_admin,
            start_monitoring,
            stop_monitoring,
            get_current_stats,
            get_network_connections,
            save_current_scan,
            list_scan_history,
            load_scan,
            compare_scans,
            clear_scan_history,
            update_scan_tags,
            get_task_trends,
            ai_providers::ollama::ai_list_ollama_models,
            phi_silica::check_phi_silica_available,
            phi_silica::ensure_phi_silica,
            phi_silica::analyze_with_phi_silica,
            phi_silica::check_phi_silica_updates,
            // Unified AI service commands
            ai_service::ai_get_status,
            ai_service::ai_analyze_diagnostic,
            ai_service::ai_analyze_section,
            ai_service::ai_explain_health,
            ai_service::ai_set_preference,
            ai_service::ai_clear_cache,
            ai_service::ai_prioritize_issues,
            ai_fix_plan::ai_propose_fix_plan,
            // Agentic AI chat (streaming via ai-chat:// events)
            ai_chat::ai_chat_send,
            ai_chat::ai_chat_cancel,
            ai_chat::ai_chat_new_session,
            ai_chat::ai_chat_get_history,
            ai_report::ai_generate_report,
            detect_issues,
            copy_minidumps_to_desktop,
            open_url,
            update_check::check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
