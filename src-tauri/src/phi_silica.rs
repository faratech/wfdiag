//! Phi Silica integration for on-device AI inference on Copilot+ PCs.
//!
//! This module provides a wrapper around the Microsoft.Windows.AI.Generative
//! WinRT APIs to enable local AI analysis using Phi Silica.
//!
//! Note: This requires:
//! - A Copilot+ PC with NPU hardware
//! - Windows 11 24H2 or later
//! - Windows App Runtime 1.7 or later installed

use serde::{Deserialize, Serialize};

/// Response from checking Phi Silica availability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiSilicaStatus {
    pub available: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Windows build number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_build: Option<u32>,
}

/// Get Windows build number
#[cfg(windows)]
fn get_windows_build() -> Option<u32> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").ok()?;
    let build_str: String = key.get_value("CurrentBuildNumber").ok()?;
    build_str.parse().ok()
}

/// Check if Phi Silica is available on this device
#[cfg(windows)]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    use crate::phi_silica_bindings::Microsoft::Windows::AI::Generative::LanguageModel;

    let build = get_windows_build();

    match LanguageModel::IsAvailable() {
        Ok(available) => {
            if available {
                PhiSilicaStatus {
                    available: true,
                    message: "Phi Silica is available on this device".to_string(),
                    error_code: None,
                    windows_build: build,
                }
            } else {
                // API exists but returned false - Phi Silica component not installed/available
                PhiSilicaStatus {
                    available: false,
                    message: format!(
                        "Phi Silica not available. Requires Copilot+ PC with Windows 11 24H2/25H2. Check Windows Update for Phi Silica component updates. Build: {}",
                        build.map_or("unknown".to_string(), |b| b.to_string())
                    ),
                    error_code: None,
                    windows_build: build,
                }
            }
        },
        Err(e) => {
            let error_code = format!("{:?}", e);

            // Parse the error to give helpful message
            let message = if error_code.contains("0x80040154") || error_code.contains("CLASS_NOT_REGISTERED") {
                format!(
                    "Phi Silica API not registered. This requires a Copilot+ PC with NPU (40+ TOPS). Build: {}",
                    build.map_or("unknown".to_string(), |b| b.to_string())
                )
            } else if error_code.contains("0x80070002") || error_code.contains("NOT_FOUND") {
                format!(
                    "Phi Silica component not found. Check Windows Update for Phi Silica updates (KB5072641/KB5072642/KB5072643). Build: {}",
                    build.map_or("unknown".to_string(), |b| b.to_string())
                )
            } else {
                format!(
                    "Phi Silica check failed: {}. Build: {}",
                    e,
                    build.map_or("unknown".to_string(), |b| b.to_string())
                )
            };

            PhiSilicaStatus {
                available: false,
                message,
                error_code: Some(error_code),
                windows_build: build,
            }
        },
    }
}

#[cfg(not(windows))]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    PhiSilicaStatus {
        available: false,
        message: "Phi Silica is only available on Windows".to_string(),
        error_code: None,
        windows_build: None,
    }
}

/// Generate a response using Phi Silica
#[cfg(windows)]
pub async fn generate_response(prompt: &str) -> Result<String, String> {
    use crate::phi_silica_bindings::Microsoft::Windows::AI::Generative::LanguageModel;
    use windows_future::AsyncOperationCompletedHandler;

    // First check if available
    let available = LanguageModel::IsAvailable()
        .map_err(|e| format!("Failed to check availability: {}", e))?;

    if !available {
        return Err("Phi Silica is not available on this device. This feature requires a Copilot+ PC.".to_string());
    }

    // Create the language model - this returns an IAsyncOperation
    let create_op = LanguageModel::CreateAsync()
        .map_err(|e| format!("Failed to start creating model: {}", e))?;

    // Wait for creation to complete using blocking get
    let model = create_op.GetResults()
        .map_err(|e| format!("Failed to create language model: {}", e))?;

    // Generate response
    let prompt_hstring = windows_core::HSTRING::from(prompt);
    let response_op = model.GenerateResponseAsync(&prompt_hstring)
        .map_err(|e| format!("Failed to start generating response: {}", e))?;

    // Wait for response
    let response = response_op.GetResults()
        .map_err(|e| format!("Failed to generate response: {}", e))?;

    // Get the response text
    let text = response.Response()
        .map_err(|e| format!("Failed to get response text: {}", e))?;

    Ok(text.to_string())
}

#[cfg(not(windows))]
pub async fn generate_response(_prompt: &str) -> Result<String, String> {
    Err("Phi Silica is only available on Windows".to_string())
}

/// Tauri command to check Phi Silica availability
#[tauri::command]
pub async fn check_phi_silica_available() -> Result<PhiSilicaStatus, String> {
    Ok(is_phi_silica_available())
}

/// Tauri command to open Windows Update to check for Phi Silica updates
#[tauri::command]
#[cfg(windows)]
pub async fn check_phi_silica_updates() -> Result<String, String> {
    use std::process::Command;

    // Open Windows Update settings
    Command::new("cmd")
        .args(["/c", "start", "ms-settings:windowsupdate"])
        .spawn()
        .map_err(|e| format!("Failed to open Windows Update: {}", e))?;

    Ok("Opening Windows Update. Check for updates to install Phi Silica component (KB5072641/KB5072642/KB5072643).".to_string())
}

#[tauri::command]
#[cfg(not(windows))]
pub async fn check_phi_silica_updates() -> Result<String, String> {
    Err("Windows Update is only available on Windows".to_string())
}

/// Tauri command to analyze system with Phi Silica
#[tauri::command]
pub async fn analyze_with_phi_silica(prompt: String) -> Result<String, String> {
    // Check availability first
    let status = is_phi_silica_available();
    if !status.available {
        return Err(status.message);
    }

    // Build a system context with diagnostic information
    let mut context = String::new();
    context.push_str("You are a Windows system diagnostic assistant running locally on a Copilot+ PC.\n");
    context.push_str("Analyze the following system information and provide specific, actionable recommendations.\n\n");

    // Run some basic diagnostics to include in context
    let diagnostics = vec![
        "comp_system",
        "os_info",
        "processor",
        "physical_memory",
    ];

    for task_id in diagnostics {
        if let Ok(result) = crate::diagnostics::run_diagnostic_task_sync(task_id) {
            context.push_str(&format!("=== {} ===\n{}\n\n", task_id, result.output));
        }
    }

    // Append user prompt
    context.push_str(&format!("User question: {}\n\nProvide a helpful response:", prompt));

    // Generate response
    generate_response(&context).await
}
