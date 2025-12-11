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

/// Windows App Runtime installer URL (latest stable)
const WINDOWS_APP_RUNTIME_URL: &str = "https://aka.ms/windowsappsdk/1.6/latest/windowsappruntimeinstall-x64.exe";
const WINDOWS_APP_RUNTIME_ARM64_URL: &str = "https://aka.ms/windowsappsdk/1.6/latest/windowsappruntimeinstall-arm64.exe";

/// Response from checking Phi Silica availability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiSilicaStatus {
    pub available: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Whether the Windows App Runtime needs to be installed
    #[serde(default)]
    pub needs_runtime_install: bool,
}

/// Check if Phi Silica is available on this device
#[cfg(windows)]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    use crate::phi_silica_bindings::Microsoft::Windows::AI::Generative::LanguageModel;

    match LanguageModel::IsAvailable() {
        Ok(available) => PhiSilicaStatus {
            available,
            message: if available {
                "Phi Silica is available on this device".to_string()
            } else {
                "Phi Silica is not available. This feature requires a Copilot+ PC with Windows 11 24H2 or later.".to_string()
            },
            error_code: None,
            needs_runtime_install: false,
        },
        Err(e) => {
            // Provide more detailed error information
            let error_code = format!("{:?}", e);
            let needs_runtime = error_code.contains("CLASS_NOT_REGISTERED")
                || error_code.contains("0x80040154")
                || error_code.contains("0x80070002");

            let message = if error_code.contains("CLASS_NOT_REGISTERED") || error_code.contains("0x80040154") {
                "Windows App Runtime is not installed. Click 'Install Runtime' to enable Phi Silica.".to_string()
            } else if error_code.contains("NOT_FOUND") || error_code.contains("0x80070002") {
                "Phi Silica component not found. This feature requires Windows 11 24H2 on a Copilot+ PC.".to_string()
            } else {
                format!("Failed to check Phi Silica availability: {}", e)
            };

            PhiSilicaStatus {
                available: false,
                message,
                error_code: Some(error_code),
                needs_runtime_install: needs_runtime,
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
        needs_runtime_install: false,
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

/// Get the appropriate Windows App Runtime installer URL for this architecture
#[cfg(windows)]
fn get_runtime_installer_url() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        WINDOWS_APP_RUNTIME_ARM64_URL
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        WINDOWS_APP_RUNTIME_URL
    }
}

/// Tauri command to install Windows App Runtime
#[tauri::command]
#[cfg(windows)]
pub async fn install_windows_app_runtime() -> Result<String, String> {
    use std::process::Command;
    use std::fs::File;
    use std::io::Write;

    let url = get_runtime_installer_url();

    // Download to temp directory
    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join("WindowsAppRuntimeInstall.exe");

    // Download using reqwest (no PowerShell window)
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Failed to download installer: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Failed to download installer: HTTP {}", response.status()));
    }

    let bytes = response.bytes()
        .await
        .map_err(|e| format!("Failed to read installer data: {}", e))?;

    // Write to file
    let mut file = File::create(&installer_path)
        .map_err(|e| format!("Failed to create installer file: {}", e))?;
    file.write_all(&bytes)
        .map_err(|e| format!("Failed to write installer file: {}", e))?;
    drop(file);

    // Run the installer and wait for it to complete
    let output = Command::new(&installer_path)
        .args(["--quiet"])
        .output()
        .map_err(|e| format!("Failed to run installer: {}", e))?;

    // Clean up installer file
    let _ = std::fs::remove_file(&installer_path);

    if output.status.success() {
        Ok("Windows App Runtime installed successfully! Please restart the application to enable Phi Silica.".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Check for common error codes
        if let Some(code) = output.status.code() {
            match code {
                0 => Ok("Windows App Runtime installed successfully! Please restart the application.".to_string()),
                1602 => Err("Installation was cancelled by user.".to_string()),
                1618 => Err("Another installation is in progress. Please wait and try again.".to_string()),
                1641 | 3010 => Ok("Windows App Runtime installed! A system restart may be required.".to_string()),
                _ => Err(format!("Installation failed with code {}. {}{}", code, stdout, stderr)),
            }
        } else {
            Err(format!("Installation process terminated unexpectedly. {}{}", stdout, stderr))
        }
    }
}

#[tauri::command]
#[cfg(not(windows))]
pub async fn install_windows_app_runtime() -> Result<String, String> {
    Err("Windows App Runtime can only be installed on Windows".to_string())
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
