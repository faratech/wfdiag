#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod architecture;
mod diagnostics;
mod native_monitor;
mod native_diagnostics;
mod openai_integration;
mod results_storage;
mod timestamp;
mod windows_native;
#[cfg(windows)]
mod wmi_native;
mod security;
mod issue_detector;
mod issue_fixer;
mod encrypted_storage;
#[cfg(windows)]
mod dpapi;

// Phi Silica wrapper (uses windows-bindgen generated bindings)
mod phi_silica;
#[cfg(windows)]
mod windows_ai_bindings;

// Unified AI service layer
mod ai_service;
mod ai_cache;
mod ai_prompts;

use crate::diagnostics::{DiagnosticTask, TaskResult};
use crate::issue_detector::Issue;
use native_monitor::{NetworkConnection, SystemMonitor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::path::PathBuf;
use tauri::Emitter;
use tauri::State;
use tokio::sync::Mutex;
use results_storage::{ScanStorage, ScanRecord, ScanSummary, ComparisonResult};

/// Application settings that persist across sessions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub auto_save: bool,
    #[serde(default)]
    pub scan_on_startup: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    #[serde(default = "default_export_format")]
    pub export_format: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub show_notifications: bool,
    #[serde(default)]
    pub custom_export_path: Option<String>,
    #[serde(default = "default_true")]
    pub retain_history: bool,
    #[serde(default = "default_history_limit")]
    pub history_limit: u32,
    // AI settings
    #[serde(default = "default_true")]
    pub ai_enabled: bool,
    #[serde(default = "default_ai_provider")]
    pub preferred_ai_provider: String,
    // API key - stored in keyring, included in frontend response
    #[serde(default)]
    pub open_ai_api_key: Option<String>,
}

fn default_max_concurrent() -> u32 { 5 }
fn default_export_format() -> String { "text".to_string() }
fn default_theme() -> String { "dark".to_string() }
fn default_true() -> bool { true }
fn default_history_limit() -> u32 { 30 }
fn default_ai_provider() -> String { "auto".to_string() }

/// Get the settings file path
fn get_settings_path() -> Result<PathBuf, String> {
    let app_data = dirs::config_dir()
        .ok_or_else(|| "Could not find config directory".to_string())?;
    let settings_dir = app_data.join("com.windowsforum.diagnostics");

    // Create directory if it doesn't exist
    if !settings_dir.exists() {
        std::fs::create_dir_all(&settings_dir)
            .map_err(|e| format!("Failed to create settings directory: {}", e))?;
    }

    Ok(settings_dir.join("settings.json"))
}

#[tauri::command]
async fn save_settings(settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path()?;

    // If API key is provided, store it securely (separate from settings file)
    if let Some(ref api_key) = settings.open_ai_api_key {
        #[cfg(windows)]
        {
            if !api_key.is_empty() {
                match dpapi::store_api_key(api_key) {
                    Ok(_) => println!("API key stored with DPAPI"),
                    Err(e) => println!("Warning: Failed to store API key with DPAPI: {}", e),
                }
            } else {
                // Empty string means clear the key
                let _ = dpapi::clear_api_key();
                println!("API key cleared from DPAPI storage");
            }
        }
        #[cfg(not(windows))]
        {
            if !api_key.is_empty() {
                if let Ok(entry) = Entry::new("wfdiag-tauri", "openai_api_key") {
                    match entry.set_password(api_key) {
                        Ok(_) => println!("API key stored in keyring"),
                        Err(e) => println!("Warning: Failed to store API key in keyring: {}", e),
                    }
                }
            } else {
                if let Ok(entry) = Entry::new("wfdiag-tauri", "openai_api_key") {
                    let _ = entry.delete_credential();
                    println!("API key cleared from keyring");
                }
            }
        }
    }

    // Create a copy without the API key for file storage (security: don't write key to file)
    let mut settings_for_file = settings.clone();
    settings_for_file.open_ai_api_key = None;

    let json = serde_json::to_string_pretty(&settings_for_file)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write settings file: {}", e))?;
    println!("Settings saved to {:?}", path);
    Ok(())
}

#[tauri::command]
async fn load_settings() -> Result<AppSettings, String> {
    let path = get_settings_path()?;

    let mut settings = if !path.exists() {
        println!("No settings file found, returning defaults");
        AppSettings::default()
    } else {
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read settings file: {}", e))?;
        serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse settings: {}", e))?
    };

    // Load API key from keyring and include in response
    if let Some(api_key) = load_api_key_internal().await {
        settings.open_ai_api_key = Some(api_key);
    }

    println!("Settings loaded from {:?}", path);
    Ok(settings)
}

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
    #[cfg(windows)]
    {
        dpapi::store_api_key(&key)
    }
    #[cfg(not(windows))]
    {
        // Fallback to keyring on non-Windows
        let entry = Entry::new("wfdiag-tauri", "openai_api_key")
            .map_err(|e| format!("Failed to access keyring entry: {e}"))?;
        entry
            .set_password(&key)
            .map_err(|e| format!("Failed to store API key: {e}"))?;
        Ok(())
    }
}

#[tauri::command]
async fn load_api_key() -> Result<String, String> {
    load_api_key_internal().await.ok_or_else(|| "No API key set".to_string())
}

/// Internal function for loading API key (used by ai_service)
pub(crate) async fn load_api_key_internal() -> Option<String> {
    #[cfg(windows)]
    {
        dpapi::load_api_key().ok().flatten()
    }
    #[cfg(not(windows))]
    {
        let entry = Entry::new("wfdiag-tauri", "openai_api_key").ok()?;
        match entry.get_password() {
            Ok(pwd) if !pwd.is_empty() => Some(pwd),
            _ => None,
        }
    }
}

#[tauri::command]
async fn clear_api_key() -> Result<(), String> {
    #[cfg(windows)]
    {
        dpapi::clear_api_key()
    }
    #[cfg(not(windows))]
    {
        let entry = Entry::new("wfdiag-tauri", "openai_api_key")
            .map_err(|e| format!("Failed to access keyring entry: {e}"))?;
        match entry.delete_credential() {
            Ok(_) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(format!("Failed to clear API key: {e}")),
        }
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
    let tasks = std::sync::Arc::new(diagnostics::get_all_tasks());

    // Create futures for each diagnostic task
    let futures: Vec<_> = task_ids
        .into_iter()
        .map(|task_id| {
            let window_clone = window.clone();
            let state_clone = state.inner();
            let tasks_ref = tasks.clone(); // Arc clone is cheap

            async move {
                // Find task details
                let task = tasks_ref
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
            "html" => {
                let mut html = String::new();
                html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
                html.push_str("<meta charset=\"UTF-8\">\n");
                html.push_str("<title>WindowsForum Diagnostic Report</title>\n");
                html.push_str("<style>\n");
                html.push_str("body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 1200px; margin: 0 auto; padding: 20px; background: #1a1a2e; color: #eee; }\n");
                html.push_str("h1 { color: #60a5fa; border-bottom: 2px solid #3b82f6; padding-bottom: 10px; }\n");
                html.push_str("h2 { color: #93c5fd; margin-top: 30px; }\n");
                html.push_str(".task { background: #16213e; border-radius: 8px; padding: 15px; margin: 10px 0; border-left: 4px solid #3b82f6; }\n");
                html.push_str(".task.error { border-left-color: #ef4444; }\n");
                html.push_str(".task-name { font-weight: bold; color: #60a5fa; margin-bottom: 8px; }\n");
                html.push_str(".output { white-space: pre-wrap; font-family: 'Consolas', 'Monaco', monospace; font-size: 13px; background: #0f0f1a; padding: 10px; border-radius: 4px; overflow-x: auto; }\n");
                html.push_str(".error-msg { color: #f87171; }\n");
                html.push_str(".meta { color: #9ca3af; font-size: 12px; margin-top: 20px; }\n");
                html.push_str("</style>\n</head>\n<body>\n");
                html.push_str("<h1>WindowsForum Diagnostic Report</h1>\n");
                // Format date using JavaScript in the HTML
                html.push_str("<p class=\"meta\">Generated: <span id=\"gendate\"></span></p>\n");
                html.push_str("<script>document.getElementById('gendate').textContent = new Date().toLocaleString();</script>\n");

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

                let mut categories: Vec<_> = results_by_category.keys().cloned().collect();
                categories.sort();

                for category in categories {
                    html.push_str(&format!("<h2>{}</h2>\n", html_escape(&category)));

                    if let Some(results) = results_by_category.get(&category) {
                        for (task_id, result) in results {
                            if let Some(task) = task_map.get(*task_id) {
                                let class = if result.success { "task" } else { "task error" };
                                html.push_str(&format!("<div class=\"{}\">\n", class));
                                html.push_str(&format!("<div class=\"task-name\">{}</div>\n", html_escape(&task.name)));

                                if result.success {
                                    html.push_str("<div class=\"output\">");
                                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result.output) {
                                        html.push_str(&html_escape(&format_json_value(&parsed, 0)));
                                    } else {
                                        html.push_str(&html_escape(&result.output));
                                    }
                                    html.push_str("</div>\n");
                                } else if let Some(error) = &result.error {
                                    html.push_str(&format!("<div class=\"error-msg\">Error: {}</div>\n", html_escape(error)));
                                }

                                html.push_str("</div>\n");
                            }
                        }
                    }
                }

                html.push_str("<p class=\"meta\">Generated using WindowsForum Diagnostics Tool</p>\n");
                html.push_str("</body>\n</html>");
                Ok(html)
            }
            _ => Err("Unsupported format".to_string()),
        }
    } else {
        Err("No active session".to_string())
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
        // Use PowerShell's Start-Process which handles URLs with special characters better
        // This avoids issues with & and other chars in mailto: URLs
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("Start-Process '{}'", url.replace("'", "''"))])
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
        .manage(app_state)
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            save_settings,
            load_settings,
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
            // Unified AI service commands
            ai_service::ai_get_status,
            ai_service::ai_analyze_diagnostic,
            ai_service::ai_analyze_section,
            ai_service::ai_explain_health,
            ai_service::ai_set_preference,
            ai_service::ai_clear_cache,
            detect_issues,
            copy_minidumps_to_desktop,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}