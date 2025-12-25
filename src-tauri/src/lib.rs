#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod architecture;
mod diagnostics;
mod native_monitor;
mod native_diagnostics;
mod openai_integration;
mod results_storage;
mod timestamp;
mod windows_native;
mod security;
mod issue_detector;
mod issue_fixer;
mod encrypted_storage;

// Phi Silica wrapper (uses windows-bindgen generated bindings)
mod phi_silica;
#[cfg(windows)]
mod windows_ai_bindings;

use crate::diagnostics::{DiagnosticTask, TaskResult};
use crate::issue_detector::Issue;
use keyring::{Entry, Error as KeyringError};
use native_monitor::{NetworkConnection, SystemMonitor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tauri::State;
use tokio::sync::Mutex;
use results_storage::{ScanStorage, ScanRecord, ScanSummary, ComparisonResult};

// Helper function to format JSON values into readable text
fn format_json_value(value: &serde_json::Value, indent_level: usize) -> String {
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
                            "{}{} : {}
",
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SystemInfo {
    computer_name: String,
    os_version: String,
    is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiagnosticSession {
    session_id: String,
    start_time: std::time::SystemTime,
    selected_tasks: Vec<String>,
    results: HashMap<String, TaskResult>,
}

struct AppState {
    current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
    scan_storage: Arc<Mutex<ScanStorage>>,
}

#[tauri::command]
async fn store_api_key(key: String) -> Result<(), String> {
    let entry = Entry::new("wfdiag-tauri", "openai_api_key")
        .map_err(|e| format!("Failed to access keyring entry: {e}"))?;
    entry
        .set_password(&key)
        .map_err(|e| format!("Failed to store API key: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn load_api_key() -> Result<String, String> {
    let entry = Entry::new("wfdiag-tauri", "openai_api_key")
        .map_err(|e| format!("Failed to access keyring entry: {e}"))?;
    match entry.get_password() {
        Ok(pwd) => Ok(pwd),
        Err(KeyringError::NoEntry) => Ok(String::new()),
        Err(e) => Err(format!("Failed to load API key: {e}")),
    }
}

#[tauri::command]
async fn clear_api_key() -> Result<(), String> {
    let entry = Entry::new("wfdiag-tauri", "openai_api_key")
        .map_err(|e| format!("Failed to access keyring entry: {e}"))?;
    match entry.delete_credential() {
        Ok(_) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(format!("Failed to clear API key: {e}")),
    }
}


/// Get detailed Windows version information from registry
#[cfg(windows)]
fn get_windows_version_info() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    if let Ok(cv_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion") {
        let product_name = cv_key.get_value::<String, _>("ProductName").unwrap_or_else(|_| "Windows".to_string());
        let display_version = cv_key.get_value::<String, _>("DisplayVersion").ok();
        let edition_id = cv_key.get_value::<String, _>("EditionID").ok();
        let current_build = cv_key.get_value::<String, _>("CurrentBuild").ok();

        // Determine Windows version (10 or 11) based on build number
        let is_win11 = if let Some(build) = current_build.as_ref() {
            build.parse::<u32>().unwrap_or(0) >= 22000
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
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
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
                .is_ok() {
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
        return Err("No valid tasks provided".to_string());
    }
    
    println!("Session {} will run {} tasks", session_id, valid_task_ids.len());
    
    let session = DiagnosticSession {
        session_id: session_id.clone(),
        start_time: std::time::SystemTime::now(),
        selected_tasks: valid_task_ids,
        results: HashMap::new(),
    };

    let mut current = state.current_session.lock().await;
    *current = Some(session);
    
    println!("Diagnostic session {} started successfully", session_id);
    
    Ok(session_id)
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
        .ok_or_else(|| format!("Task not found: {}", task_id))?;

    // Emit progress event
    window
        .emit(
            "task-progress",
            serde_json::json!({
                "task_id": &task_id,
                "status": "running",
                "task_name": &task.name,
            }),
        )
        .map_err(|e| e.to_string())?;

    // Run the diagnostic task
    let result = diagnostics::run_diagnostic_task(&task_id).await;

    // Store result
    let mut current = state.current_session.lock().await;
    if let Some(ref mut session) = *current {
        session.results.insert(task_id.clone(), result.clone());
    }

    // Emit completion event
    window
        .emit(
            "task-progress",
            serde_json::json!({
                "task_id": &task_id,
                "status": "completed",
                "success": result.success,
            }),
        )
        .map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
async fn run_diagnostics_parallel(
    task_ids: Vec<String>,
    max_concurrent: Option<usize>,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<Vec<(String, TaskResult)>, String> {
    use futures::stream::{self, StreamExt};

    let max_concurrent = max_concurrent.unwrap_or(5); // Default to 5 concurrent tasks
    let tasks = diagnostics::get_all_tasks();

    // Create futures for each diagnostic task
    let futures: Vec<_> = task_ids
        .into_iter()
        .map(|task_id| {
            let window_clone = window.clone();
            let state_clone = state.inner();
            let tasks_clone = tasks.clone();

            async move {
                // Find task details
                let task = tasks_clone
                    .iter()
                    .find(|t| t.id == task_id)
                    .ok_or_else(|| format!("Task not found: {}", task_id))?;

                // Emit progress event
                window_clone
                    .emit(
                        "task-progress",
                        serde_json::json!({
                            "task_id": &task_id,
                            "status": "running",
                            "task_name": &task.name,
                        }),
                    )
                    .map_err(|e| e.to_string())?;

                // Run the diagnostic task
                let result = diagnostics::run_diagnostic_task(&task_id).await;

                // Store result
                let mut current = state_clone.current_session.lock().await;
                if let Some(ref mut session) = *current {
                    session.results.insert(task_id.clone(), result.clone());
                }

                // Emit completion event
                window_clone
                    .emit(
                        "task-progress",
                        serde_json::json!({
                            "task_id": &task_id,
                            "status": "completed",
                            "success": result.success,
                        }),
                    )
                    .map_err(|e| e.to_string())?;

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
            Err("Session ID mismatch".to_string())
        }
    } else {
        Err("No active session".to_string())
    }
}

#[tauri::command]
async fn export_results(
    format: String,
    _include_raw: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        // Get all available tasks to map IDs to names
        let all_tasks = diagnostics::get_all_tasks();
        let task_map: std::collections::HashMap<String, &DiagnosticTask> = 
            all_tasks.iter().map(|t| (t.id.clone(), t)).collect();

        match format.as_str() {
            "json" => {
                let json = 
                    serde_json::to_string_pretty(&session.results).map_err(|e| e.to_string())?;
                Ok(json)
            }
            "text" => {
                let mut text = String::new();

                // Group results by category
                let mut results_by_category: std::collections::HashMap<
                    String,
                    Vec<(&String, &diagnostics::TaskResult)>,
                > = std::collections::HashMap::new();

                for (task_id, result) in &session.results {
                    if let Some(task) = task_map.get(task_id) {
                        results_by_category
                            .entry(task.category.clone())
                            .or_default()
                            .push((task_id, result));
                    }
                }

                // Sort categories for consistent output
                let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
                categories.sort();

                // Write results organized by category
                for category in categories {
                    text.push_str(&format!("\n=== {} ===\n\n", category));

                    if let Some(results) = results_by_category.get(&category) {
                        for (task_id, result) in results {
                            if let Some(task) = task_map.get(*task_id) {
                                text.push_str(&format!("{}:\n", task.name));

                                if result.success {
                                    // Parse and format the output
                                    if let Ok(parsed) = 
                                        serde_json::from_str::<serde_json::Value>(&result.output)
                                    {
                                        text.push_str(&format_json_value(&parsed, 1));
                                    } else {
                                        // Raw text output
                                        text.push_str(&result.output);
                                    }
                                } else if let Some(error) = &result.error {
                                    text.push_str(&format!("  Error: {}\n", error));
                                }

                                text.push('\n');
                            }
                        }
                    }
                }

                Ok(text)
            }
            _ => Err("Unsupported format".to_string()),
        }
    } else {
        Err("No active session".to_string())
    }
}

#[tauri::command]
async fn save_results_to_file(path: String, content: String) -> Result<(), String> {
    use std::fs;

    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
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

#[tauri::command]
async fn fix_issue(issue_id: String) -> Result<issue_fixer::FixResult, String> {
    let fixer = issue_fixer::IssueFixer::new();
    fixer.fix_issue(&issue_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn restart_as_admin() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::env;
        use std::ptr;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let exe_path = env::current_exe().map_err(|e| e.to_string())?;

        // Convert path to wide string for Windows API
        let exe_path_str = exe_path.to_string_lossy().to_string();
        let exe_wide: Vec<u16> = exe_path_str.encode_utf16().chain(std::iter::once(0)).collect();
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
                return Err(format!("Failed to restart with elevation. Error code: {}", result.0 as i32));
            }
        }

        // Give the elevated process time to start before exiting
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Exit current process
        std::process::exit(0);
    }

    #[cfg(not(windows))]
    {
        Err("Administrator restart is only supported on Windows".to_string())
    }
}

#[tauri::command]
async fn start_monitoring(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut monitor_opt = state.system_monitor.lock().await;

    // Create a new monitor if none exists
    if monitor_opt.is_none() {
        *monitor_opt = Some(SystemMonitor::new(app_handle));
    }

    // Start monitoring
    if let Some(monitor) = monitor_opt.as_ref() {
        monitor.start_monitoring().await;
        Ok(())
    } else {
        Err("Failed to create system monitor".to_string())
    }
}

#[tauri::command]
async fn stop_monitoring(state: State<'_, AppState>) -> Result<(), String> {
    let monitor_opt = state.system_monitor.lock().await;

    if let Some(monitor) = monitor_opt.as_ref() {
        monitor.stop_monitoring().await;
        Ok(())
    } else {
        Err("No active monitoring session".to_string())
    }
}

#[tauri::command]
async fn get_current_stats(state: State<'_, AppState>) -> Result<native_monitor::SystemStats, String> {
    let monitor_opt = state.system_monitor.lock().await;

    if let Some(monitor) = monitor_opt.as_ref() {
        Ok(monitor.get_current_stats().await)
    } else {
        Err("No active monitoring session".to_string())
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
        storage.save_scan(&scan_record)?;
        
        println!("Scan auto-saved successfully: {}", scan_record.id);
        
        Ok(scan_record.id)
    } else {
        Err("No active session to save".to_string())
    }
}

#[tauri::command]
async fn list_scan_history(state: State<'_, AppState>) -> Result<Vec<ScanSummary>, String> {
    println!("Listing scan history");
    let storage = state.scan_storage.lock().await;
    match storage.list_scans() {
        Ok(scans) => {
            println!("Found {} scans in history", scans.len());
            for scan in &scans {
                println!("  - Scan ID: {}, Timestamp: {}, Tasks: {}", 
                    scan.id, scan.timestamp, scan.task_count);
            }
            Ok(scans)
        }
        Err(e) => {
            println!("Error listing scans: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
async fn load_scan(state: State<'_, AppState>, scan_id: String) -> Result<ScanRecord, String> {
    println!("Loading scan: {}", scan_id);
    let storage = state.scan_storage.lock().await;
    storage.load_scan(&scan_id)
}

#[tauri::command]
async fn compare_scans(
    state: State<'_, AppState>,
    current_id: String,
    previous_id: String,
) -> Result<ComparisonResult, String> {
    println!("Comparing scans: '{}' vs '{}'", current_id, previous_id);
    let storage = state.scan_storage.lock().await;
    match storage.compare_scans(&current_id, &previous_id) {
        Ok(comparison) => {
            println!("Comparison successful:");
            println!("  - Current scan: {} ({})", comparison.current_scan.id, comparison.current_scan.timestamp);
            println!("  - Previous scan: {} ({})", comparison.previous_scan.id, comparison.previous_scan.timestamp);
            println!("  - Total changes: {}", comparison.total_changes);
            println!("  - New failures: {}", comparison.new_failures.len());
            println!("  - New successes: {}", comparison.new_successes.len());
            Ok(comparison)
        }
        Err(e) => {
            println!("Error comparing scans: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
async fn clear_scan_history(
    state: State<'_, AppState>,
) -> Result<String, String> {
    println!("Clearing scan history via Tauri command...");

    let storage = state.scan_storage.lock().await;
    match storage.clear_history() {
        Ok(_) => {
            println!("Scan history cleared successfully");
            Ok("Scan history cleared successfully".to_string())
        }
        Err(e) => {
            println!("Failed to clear scan history: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
async fn detect_issues(
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        let issue_detector = issue_detector::IssueDetector::new();
        let issues = issue_detector.detect_issues(&session.results);
        Ok(issues)
    } else {
        Err("No active session".to_string())
    }
}

#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        // For non-Windows systems, we'd need the opener crate
        return Err("URL opening is only supported on Windows currently".to_string());
    }

    Ok(())
}

#[tauri::command]
async fn copy_minidumps_to_desktop() -> Result<serde_json::Value, String> {
    let diagnostics = native_diagnostics::NativeDiagnostics::new().unwrap();
    diagnostics.copy_minidumps_to_desktop()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_architecture_info() -> Result<serde_json::Value, String> {
    architecture::get_architecture_json()
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let scan_storage = ScanStorage::new()
        .unwrap_or_else(|e| {
            eprintln!("Failed to initialize scan storage: {}", e);
            std::process::exit(1);
        });
    
    let app_state = AppState {
        current_session: Arc::new(Mutex::new(None)),
        system_monitor: Arc::new(Mutex::new(None)),
        scan_storage: Arc::new(Mutex::new(scan_storage)),
    };

    tauri::Builder::default()
        .setup(|app| {
            // Handle deep links for OAuth callbacks
            #[cfg(desktop)]
            {
                let _handle = app.handle().clone();
                std::thread::spawn(move || {
                    // Listen for deep link URLs
                    tauri::async_runtime::block_on(async move {
                        // This will be handled by the plugin system
                    });
                });
            }
            Ok(())
        })
        .manage(app_state)
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            store_api_key,
            load_api_key,
            clear_api_key,
            get_system_info,
            get_architecture_info,
            get_available_tasks,
            start_diagnostics,
            run_diagnostic_task,
            run_diagnostics_parallel,
            get_session_results,
            export_results,
            save_results_to_file,
            get_uptime,
            fix_issue,
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
            openai_integration::analyze_with_openai,
            openai_integration::analyze_system_with_ai,
            openai_integration::get_ai_provider_status,
            phi_silica::check_phi_silica_available,
            phi_silica::ensure_phi_silica,
            phi_silica::analyze_with_phi_silica,
            phi_silica::check_phi_silica_updates,
            detect_issues,
            copy_minidumps_to_desktop,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}