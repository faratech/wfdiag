#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod diagnostics;
mod native_diagnostics;
mod windows_native;
mod monitoring;
mod openai_integration;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::Emitter;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;
use diagnostics::{DiagnosticTask, TaskResult};
use monitoring::{SystemMonitor, NetworkConnection};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SystemInfo {
    computer_name: String,
    os_version: String,
    is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiagnosticSession {
    session_id: String,
    selected_tasks: Vec<String>,
    results: HashMap<String, TaskResult>,
}

struct AppState {
    current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
}

#[tauri::command]
async fn get_system_info() -> Result<SystemInfo, String> {
    let computer_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string());
    let os_version = std::env::var("OS").unwrap_or_else(|_| "Windows".to_string());
    
    #[cfg(windows)]
    let is_admin = {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
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
                ).is_ok() {
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
    use chrono::Utc;
    
    let session_id = format!("session_{}", Utc::now().timestamp());
    let session = DiagnosticSession {
        session_id: session_id.clone(),
        selected_tasks: task_ids,
        results: HashMap::new(),
    };
    
    let mut current = state.current_session.lock().await;
    *current = Some(session);
    
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
    let task = tasks.iter().find(|t| t.id == task_id)
        .ok_or_else(|| format!("Task not found: {}", task_id))?;
    
    // Emit progress event
    window.emit("task-progress", serde_json::json!({
        "task_id": &task_id,
        "status": "running",
        "task_name": &task.name,
    })).map_err(|e| e.to_string())?;
    
    // Run the diagnostic task
    let result = diagnostics::run_diagnostic_task(&task_id).await;
    
    // Store result
    let mut current = state.current_session.lock().await;
    if let Some(ref mut session) = *current {
        session.results.insert(task_id.clone(), result.clone());
    }
    
    // Emit completion event
    window.emit("task-progress", serde_json::json!({
        "task_id": &task_id,
        "status": "completed",
        "success": result.success,
    })).map_err(|e| e.to_string())?;
    
    Ok(result)
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
    include_raw: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let current = state.current_session.lock().await;
    if let Some(ref session) = *current {
        match format.as_str() {
            "json" => {
                let json = serde_json::to_string_pretty(&session.results)
                    .map_err(|e| e.to_string())?;
                Ok(json)
            }
            "text" => {
                let mut text = String::from("=== WindowsForum Diagnostic Report ===\n\n");
                
                for (task_id, result) in &session.results {
                    text.push_str(&format!("Task: {}\n", task_id));
                    text.push_str(&format!("Success: {}\n", result.success));
                    
                    if include_raw {
                        text.push_str(&format!("Output:\n{}\n", result.output));
                    }
                    
                    if let Some(error) = &result.error {
                        text.push_str(&format!("Error: {}\n", error));
                    }
                    
                    text.push_str("\n---\n\n");
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
async fn save_results_to_file(
    path: String,
    content: String,
) -> Result<(), String> {
    use std::fs;
    
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_uptime() -> Result<serde_json::Value, String> {
    use sysinfo::System;
    
    let uptime_seconds = System::uptime();
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
async fn restart_as_admin() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::env;
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        
        let exe_path = env::current_exe().map_err(|e| e.to_string())?;
        
        // Use PowerShell to restart with elevation
        Command::new("powershell")
            .arg("-Command")
            .arg(&format!("Start-Process '{}' -Verb RunAs", exe_path.display()))
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| e.to_string())?;
        
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
async fn get_current_stats(state: State<'_, AppState>) -> Result<monitoring::SystemStats, String> {
    let monitor_opt = state.system_monitor.lock().await;
    
    if let Some(monitor) = monitor_opt.as_ref() {
        Ok(monitor.get_current_stats().await)
    } else {
        Err("No active monitoring session".to_string())
    }
}

#[tauri::command]
async fn get_network_connections() -> Result<Vec<NetworkConnection>, String> {
    Ok(monitoring::get_network_connections().await)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState {
        current_session: Arc::new(Mutex::new(None)),
        system_monitor: Arc::new(Mutex::new(None)),
    };
    
    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            get_system_info,
            get_available_tasks,
            start_diagnostics,
            run_diagnostic_task,
            get_session_results,
            export_results,
            save_results_to_file,
            get_uptime,
            restart_as_admin,
            start_monitoring,
            stop_monitoring,
            get_current_stats,
            get_network_connections,
            openai_integration::analyze_with_openai,
            openai_integration::analyze_system_with_ai,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}