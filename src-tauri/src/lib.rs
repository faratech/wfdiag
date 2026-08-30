#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod action_broker;
pub mod ai_evidence;
mod architecture;
mod commands;
pub mod diagnostic_events;
pub mod diagnostics;
// dpapi self-gates internally (non-Windows stubs); the module stays
// cross-platform because ProviderKeyId names the keyring entries too
mod dpapi;
mod encrypted_storage;
pub mod error;
mod issue_catalog;
mod native_diagnostics;
pub use wfdiag_native_monitor as native_monitor;
pub use wfdiag_native_remediation::remediation;
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

// Tauri command wrapper over the shared package-identity/LAF-aware runtime.
pub mod phi_silica;

// Unified AI service layer
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
pub mod ui_event_adapter;

use crate::diagnostic_events::{
    DiagnosticEvent, DiagnosticEventSink, DiagnosticProgress, DiagnosticResultEvent,
    DiagnosticTaskStatus, TauriDiagnosticEventSink,
};
use crate::diagnostics::{DiagnosticTask, TaskResult};
use crate::error::DiagError;
use crate::issue_catalog::Issue;
#[cfg(windows)]
use crate::ui_event_adapter::TauriMonitorEmitter;
use native_monitor::{NetworkConnection, ProcessPage, ProcessQuery, SystemMonitor};
use results_storage::{
    ComparisonResult, ComparisonSummary, ScanRecord, ScanStorage, ScanSummary, TaskDiffDetail,
    TaskTrend,
};
use std::collections::HashMap;
use std::sync::{Arc, atomic::Ordering};
use tauri::State;

// Re-export settings types from commands module
pub use commands::AppSettings;

// Use state types from state module
use state::{AppState, DiagnosticSession, SystemInfo};

// Re-export load_api_key_internal for ai_service module
pub(crate) use commands::settings::load_api_key_internal;

#[tauri::command]
async fn get_system_info() -> Result<SystemInfo, String> {
    tokio::task::spawn_blocking(wfdiag_native_system::get_system_info)
        .await
        .map_err(|error| {
            String::from(DiagError::internal(format!(
                "System information task failed: {error}"
            )))
        })?
        .map_err(|error| DiagError::internal(error.to_string()).into())
}

#[tauri::command]
async fn get_available_tasks() -> Result<Vec<DiagnosticTask>, String> {
    Ok(diagnostics::get_all_tasks())
}

#[tauri::command]
async fn start_diagnostics(
    task_ids: Vec<String>,
    scan_kind: Option<crate::state::ScanKind>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    start_diagnostics_session(task_ids, scan_kind, state.inner()).await
}

/// Start a diagnostic session without depending on a UI framework.
///
/// Tauri commands and native shells share this state transition so session
/// validation and replacement semantics cannot drift during the migration.
pub async fn start_diagnostics_session(
    task_ids: Vec<String>,
    scan_kind: Option<crate::state::ScanKind>,
    state: &AppState,
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
        scan_kind: scan_kind.unwrap_or_default(),
        selected_tasks: valid_task_ids,
        results: HashMap::new(),
    };

    let mut current = state.current_session.lock().await;
    let mut previous = state.previous_session.lock().await;
    *previous = current.replace(session);
    drop(previous);
    drop(current);

    println!("Diagnostic session {} started successfully", session_id);

    Ok(session_id)
}

/// Cancel a running diagnostic session. Cancellation is task-granular:
/// tasks already in flight finish normally, queued tasks are skipped, and the
/// pending `run_diagnostics_parallel` call resolves quickly with the results
/// collected so far.
fn should_restore_previous_session(
    session: Option<&DiagnosticSession>,
    session_id: &str,
    runner_active: bool,
) -> bool {
    session.is_some_and(|session| {
        let selected: std::collections::HashSet<&str> =
            session.selected_tasks.iter().map(String::as_str).collect();
        let incomplete = selected
            .iter()
            .any(|task_id| !session.results.contains_key(*task_id));
        session.session_id == session_id && (runner_active || incomplete)
    })
}

#[tauri::command]
async fn cancel_diagnostics(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    cancel_diagnostics_session(session_id, state.inner()).await
}

/// Cancel a diagnostic session through the shared backend state.
pub async fn cancel_diagnostics_session(
    session_id: String,
    state: &AppState,
) -> Result<(), String> {
    println!("Cancelling diagnostic session: {}", session_id);
    state
        .cancelled_sessions
        .lock()
        .await
        .insert(session_id.clone());
    // Restore the last completed scan immediately. In-flight work from the
    // cancelled session is already guarded by session id and therefore cannot
    // contaminate the restored evidence.
    let runner_active = state.active_scan_runners.lock().await.contains(&session_id);
    let mut current = state.current_session.lock().await;
    let should_restore =
        should_restore_previous_session(current.as_ref(), &session_id, runner_active);
    if should_restore {
        let mut previous = state.previous_session.lock().await;
        *current = previous.take();
    }
    Ok(())
}

#[tauri::command]
async fn run_diagnostic_task(
    task_id: String,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<TaskResult, String> {
    run_diagnostic_task_with_sink(
        task_id,
        state.inner(),
        Arc::new(TauriDiagnosticEventSink::new(window)),
    )
    .await
}

/// Run one diagnostic and publish its progress and result through a
/// UI-framework-neutral sink.
pub async fn run_diagnostic_task_with_sink(
    task_id: String,
    state: &AppState,
    sink: Arc<dyn DiagnosticEventSink>,
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

    sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
        session_id: session_id.clone(),
        task_id: task_id.clone(),
        status: DiagnosticTaskStatus::Running,
        task_name: Some(task.name.clone()),
        success: None,
    }))
    .await
    .map_err(DiagError::internal)?;

    // Run the diagnostic task
    let result = diagnostics::run_diagnostic_task(&task_id).await;

    // Store result
    let mut current = state.current_session.lock().await;
    if let Some(ref mut session) = *current
        && session_id.as_ref() == Some(&session.session_id)
    {
        session.results.insert(task_id.clone(), result.clone());
    }
    drop(current);

    // Native shells need the complete evidence; the Tauri compatibility sink
    // intentionally ignores this event because the invoke already returns it.
    if sink.accepts_results() {
        sink.emit(DiagnosticEvent::Result(DiagnosticResultEvent {
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            result: result.clone(),
        }))
        .await
        .map_err(DiagError::internal)?;
    }

    sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
        session_id,
        task_id,
        status: DiagnosticTaskStatus::Completed,
        task_name: None,
        success: Some(result.success),
    }))
    .await
    .map_err(DiagError::internal)?;

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
    run_diagnostics_parallel_with_sink(
        task_ids,
        max_concurrent,
        session_id,
        state.inner(),
        Arc::new(TauriDiagnosticEventSink::new(window)),
    )
    .await
}

/// Run a bounded diagnostic batch independently of the owning UI framework.
pub async fn run_diagnostics_parallel_with_sink(
    task_ids: Vec<String>,
    max_concurrent: Option<usize>,
    session_id: String,
    state: &AppState,
    sink: Arc<dyn DiagnosticEventSink>,
) -> Result<Vec<(String, TaskResult)>, String> {
    use futures::stream::{self, StreamExt};

    let max_concurrent = max_concurrent.unwrap_or(5).clamp(1, 16); // Default to 5, bounded
    let tasks = Arc::new(diagnostics::get_all_tasks());
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
            let sink = Arc::clone(&sink);
            let tasks_ref = tasks.clone(); // Arc clone is cheap
            let session_id = session_id.clone();

            async move {
                // Skip queued tasks of a cancelled session before doing any work.
                // No events are emitted: the frontend has already torn down its
                // progress UI when it requested cancellation.
                if state.cancelled_sessions.lock().await.contains(&session_id) {
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
                        let _ = sink
                            .emit(DiagnosticEvent::Progress(DiagnosticProgress {
                                session_id: Some(session_id.clone()),
                                task_id: task_id.clone(),
                                status: DiagnosticTaskStatus::Completed,
                                task_name: None,
                                success: Some(false),
                            }))
                            .await;
                        return Err::<(String, TaskResult), String>(
                            DiagError::TaskNotFound {
                                task_id: task_id.clone(),
                            }
                            .into(),
                        );
                    }
                };

                sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
                    session_id: Some(session_id.clone()),
                    task_id: task_id.clone(),
                    status: DiagnosticTaskStatus::Running,
                    task_name: Some(task.name.clone()),
                    success: None,
                }))
                .await
                .map_err(DiagError::internal)?;

                // Run the diagnostic task
                let result = diagnostics::run_diagnostic_task(&task_id).await;

                // Store result ONLY if the active session is still the one this run
                // belongs to. Without this, a scan started while a prior parallel run is
                // still draining would have the prior run's in-flight tasks write into the
                // NEW session, mixing two scans' results.
                {
                    let mut current = state.current_session.lock().await;
                    if let Some(ref mut session) = *current
                        && session.session_id == session_id
                    {
                        session.results.insert(task_id.clone(), result.clone());
                    }
                }

                if sink.accepts_results() {
                    sink.emit(DiagnosticEvent::Result(DiagnosticResultEvent {
                        session_id: Some(session_id.clone()),
                        task_id: task_id.clone(),
                        result: result.clone(),
                    }))
                    .await
                    .map_err(DiagError::internal)?;
                }

                sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
                    session_id: Some(session_id.clone()),
                    task_id: task_id.clone(),
                    status: DiagnosticTaskStatus::Completed,
                    task_name: None,
                    success: Some(result.success),
                }))
                .await
                .map_err(DiagError::internal)?;

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

    // Commit only a structurally complete replacement. Infrastructure errors
    // can omit result entries entirely; in that case retain the last usable
    // scan instead of silently replacing it with incomplete evidence. If
    // cancellation already restored it, the id no longer matches.
    let mut current = state.current_session.lock().await;
    if current
        .as_ref()
        .is_some_and(|session| session.session_id == session_id)
    {
        let incomplete = should_restore_previous_session(current.as_ref(), &session_id, false);
        let mut previous = state.previous_session.lock().await;
        if incomplete {
            *current = previous.take();
        } else {
            previous.take();
        }
    }

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

/// The full remediation catalog (the Maintenance section renders the
/// `maintenance: true` subset).
#[tauri::command]
fn get_remediations() -> Result<Vec<remediation::RemediationSummary>, String> {
    Ok(remediation::remediations()
        .iter()
        .map(|spec| spec.summary())
        .collect())
}

/// Relaunch THIS process elevated by ShellExecute-ing its own absolute image
/// path (`current_exe()`) with the `runas` verb. Blocking: ShellExecuteEx shows
/// the UAC consent prompt and doesn't return until the user answers, and it can
/// delegate to COM shell extensions — so this runs on a dedicated blocking
/// thread with its own STA apartment, never the async worker.
///
/// The same code path serves the loose exe and the packaged Store build: both
/// relaunch whatever absolute path the process is actually running from
/// (`current_exe()` resolves the real image path in either case). The MSIX is a
/// `runFullTrust` package, so elevating its exe is permitted; if the elevated
/// session comes up without package identity, the AI layer just routes away
/// from Phi Silica — the admin-only diagnostics that motivate the restart don't
/// need identity. If Windows refuses the launch outright we surface that instead
/// of silently pretending the Store build can never elevate (the old behavior).
/// Returns `Ok(true)` when the elevated instance was launched, `Ok(false)` when
/// the user dismissed the UAC prompt (not a failure — the caller should leave
/// the app running untouched), and `Err` only on a genuine failure to launch.
#[cfg(windows)]
fn relaunch_self_elevated() -> Result<bool, String> {
    wfdiag_native_remediation::elevation::relaunch_self_elevated()
}
#[tauri::command]
async fn restart_as_admin(app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        // UAC prompt blocks the calling thread; keep it off the async runtime.
        match tokio::task::spawn_blocking(relaunch_self_elevated).await {
            Ok(Ok(true)) => {
                // Elevated instance is up. Fully quit this one (bypassing
                // close-to-tray, like the tray "Exit") so only one copy runs.
                app_handle.exit(0);
                Ok(())
            }
            // User dismissed the UAC prompt — leave this instance running, no error.
            Ok(Ok(false)) => Ok(()),
            Ok(Err(message)) => Err(message),
            Err(join_err) => {
                Err(DiagError::internal(format!("Elevation task failed: {}", join_err)).into())
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = app_handle;
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
        *monitor_opt = Some(SystemMonitor::with_emitter(Arc::new(
            TauriMonitorEmitter::new(app_handle),
        )));
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

    // Re-check after acquiring the lock: a concurrent start_monitoring may
    // have bumped the lease (and started a newer loop) while this call was
    // waiting for the lock. Without this, a stale stop could kill a session
    // it was never meant to touch.
    if let Some(lease_id) = lease_id
        && state.monitoring_lease.load(Ordering::SeqCst) != lease_id
    {
        return Ok(());
    }

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
async fn list_processes(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    query: ProcessQuery,
) -> Result<ProcessPage, String> {
    let mut monitor_opt = state.system_monitor.lock().await;
    if monitor_opt.is_none() {
        *monitor_opt = Some(SystemMonitor::with_emitter(Arc::new(
            TauriMonitorEmitter::new(app_handle),
        )));
    }

    match monitor_opt.as_ref() {
        Some(monitor) => Ok(monitor.list_processes(query).await),
        None => Err(DiagError::monitor_failed("Failed to create system monitor").into()),
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
    // System info first, OUTSIDE the session lock: it awaits real WMI work.
    let system_info = get_system_info().await?;

    // Snapshot the session under a short-lived guard. This used to hold
    // current_session across get_system_info().await AND the blocking disk
    // write below, stalling every other command that needs the session
    // (chat tools, scan start/cancel) for the whole save.
    let scan_record = {
        let session = state.current_session.lock().await;
        let Some(session) = session.as_ref() else {
            return Err(DiagError::NoActiveSession.into());
        };

        let success_count = session.results.values().filter(|r| r.success).count();
        let failure_count = session.results.values().filter(|r| !r.success).count();

        // Use the session ID directly - no more ID mismatch!
        ScanRecord {
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
            label: None,
            tags,
        }
    }; // session guard dropped

    println!("Auto-saving scan with session ID: {}", scan_record.id);
    println!("Scan has {} task results", scan_record.results.len());

    let id = scan_record.id.clone();
    let storage = state.scan_storage.clone();
    let storage_error = state.scan_storage_error.clone();
    // save_scan is synchronous file IO: run it on the blocking pool so no
    // async worker (or anything awaiting these mutexes) waits on the disk.
    tokio::task::spawn_blocking(move || {
        let storage = storage.blocking_lock();
        match storage.as_ref() {
            Some(storage) => {
                storage.save_scan(&scan_record)?;
                println!("Scan auto-saved successfully: {}", id);
                Ok(id)
            }
            None => {
                let error = storage_error.blocking_lock();
                Err(DiagError::storage(
                    "save_scan",
                    error.as_deref().unwrap_or("Storage initialization failed"),
                )
                .into())
            }
        }
    })
    .await
    .map_err(|e| format!("Scan save task failed: {e}"))?
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
async fn compare_scans_summary(
    state: State<'_, AppState>,
    current_id: String,
    previous_id: String,
) -> Result<ComparisonSummary, String> {
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.compare_scans_summary(&current_id, &previous_id),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "compare_scans_summary",
                error.as_deref().unwrap_or("Storage initialization failed"),
            )
            .into())
        }
    }
}

#[tauri::command]
async fn get_scan_task_diff(
    state: State<'_, AppState>,
    current_id: String,
    previous_id: String,
    task_id: String,
) -> Result<TaskDiffDetail, String> {
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.scan_task_diff(&current_id, &previous_id, &task_id),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "get_scan_task_diff",
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
async fn update_scan_label(
    state: State<'_, AppState>,
    scan_id: String,
    label: Option<String>,
) -> Result<(), String> {
    let storage = state.scan_storage.lock().await;
    match storage.as_ref() {
        Some(storage) => storage.update_label(&scan_id, label),
        None => {
            let error = state.scan_storage_error.lock().await;
            Err(DiagError::storage(
                "update_label",
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
    tokio::task::spawn_blocking(wfdiag_native_system::get_architecture_json)
        .await
        .map_err(|error| {
            String::from(DiagError::internal(format!(
                "Architecture information task failed: {error}"
            )))
        })?
        .map_err(|error| DiagError::internal(error.to_string()).into())
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            get_remediations,
            action_broker::action_prepare,
            action_broker::action_get_proposal,
            action_broker::action_list_pending_proposals,
            action_broker::action_discard_proposal,
            action_broker::action_approve,
            action_broker::action_cancel,
            action_broker::action_get_status,
            action_broker::action_list_history,
            restart_as_admin,
            start_monitoring,
            stop_monitoring,
            get_current_stats,
            list_processes,
            get_network_connections,
            save_current_scan,
            list_scan_history,
            load_scan,
            compare_scans,
            compare_scans_summary,
            get_scan_task_diff,
            clear_scan_history,
            update_scan_tags,
            update_scan_label,
            get_task_trends,
            ai_providers::ollama::ai_list_ollama_models,
            ai_providers::model_catalog::ai_list_models,
            // Subscription CLI bridge (Codex): the CLI owns sign-in
            ai_providers::cli_bridge::ai_bridge_status,
            ai_providers::cli_bridge::ai_bridge_install,
            ai_providers::cli_bridge::ai_bridge_sign_in,
            ai_providers::cli_bridge::ai_bridge_sign_in_cancel,
            ai_providers::cli_bridge::ai_bridge_sign_out,
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
            ai_chat::ai_chat_resolve_fallback,
            ai_chat::ai_chat_new_session,
            ai_chat::ai_chat_get_history,
            ai_report::ai_generate_report,
            ai_report::ai_report_cancel,
            detect_issues,
            copy_minidumps_to_desktop,
            open_url,
            update_check::check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod scan_session_tests {
    use super::*;

    fn session(completed: bool) -> DiagnosticSession {
        let mut results = HashMap::new();
        if completed {
            results.insert(
                "os_info".to_string(),
                TaskResult {
                    success: true,
                    output: "{}".to_string(),
                    error: None,
                    duration_ms: 1,
                },
            );
        }
        DiagnosticSession {
            session_id: "scan-new".to_string(),
            start_time: std::time::SystemTime::now(),
            scan_kind: crate::state::ScanKind::Full,
            selected_tasks: vec!["os_info".to_string()],
            results,
        }
    }

    #[test]
    fn cancellation_restores_only_the_active_or_incomplete_replacement() {
        let incomplete = session(false);
        assert!(should_restore_previous_session(
            Some(&incomplete),
            "scan-new",
            false
        ));

        let complete = session(true);
        assert!(should_restore_previous_session(
            Some(&complete),
            "scan-new",
            true
        ));
        assert!(!should_restore_previous_session(
            Some(&complete),
            "scan-new",
            false
        ));
        assert!(!should_restore_previous_session(
            Some(&incomplete),
            "different-session",
            true
        ));
    }
}
