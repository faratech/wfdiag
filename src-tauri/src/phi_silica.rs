//! Tauri command adapters for the UI-neutral Phi Silica runtime.

pub use wfdiag_native_phi::{
    PhiPromptFit, PhiSilicaAnalysisResponse, PhiSilicaStatus, ensure_phi_silica_ready,
    generate_response, is_phi_silica_available, measure_prompt_fit,
};

/// Check Phi Silica availability without blocking an async runtime worker.
#[tauri::command]
pub async fn check_phi_silica_available() -> Result<PhiSilicaStatus, String> {
    wfdiag_native_phi::probe_phi_silica_status().await
}

/// Ensure Phi Silica is ready, downloading model assets when Windows requires it.
#[tauri::command]
pub async fn ensure_phi_silica() -> Result<String, String> {
    ensure_phi_silica_ready().await?;
    Ok("Phi Silica is ready".to_string())
}

/// Open Windows Update to check for Windows AI model/runtime updates.
#[tauri::command]
#[cfg(windows)]
pub async fn check_phi_silica_updates() -> Result<String, String> {
    use std::ptr;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR};

    let uri = HSTRING::from("ms-settings:windowsupdate");
    let open = HSTRING::from("open");

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(open.as_ptr()),
            PCWSTR(uri.as_ptr()),
            PCWSTR(ptr::null()),
            PCWSTR(ptr::null()),
            SW_SHOWNORMAL,
        )
    };

    if result.0 as i32 <= 32 {
        return Err(format!(
            "Failed to open Windows Update: error code {}",
            result.0 as i32
        ));
    }

    Ok("Opening Windows Update. Check for updates to install Phi Silica component.".to_string())
}

#[tauri::command]
#[cfg(not(windows))]
pub async fn check_phi_silica_updates() -> Result<String, String> {
    Err(crate::error::DiagError::PlatformNotSupported {
        operation: "Windows Update".to_string(),
    }
    .into())
}

/// Retained wire command for older frontends. The unified AI flows own analysis.
#[tauri::command]
pub async fn analyze_with_phi_silica(_prompt: String) -> Result<PhiSilicaAnalysisResponse, String> {
    Err(
        "The legacy Phi Silica analysis endpoint has been retired. Use the unified AI chat or AI report flow."
            .to_string(),
    )
}
