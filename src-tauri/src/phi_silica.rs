//! Phi Silica integration for on-device AI inference on Copilot+ PCs.
//!
//! This module provides detection and wrapper for the Microsoft.Windows.AI.Text
//! WinRT APIs to enable local AI analysis using Phi Silica.
//!
//! Note: This requires:
//! - A Copilot+ PC with NPU hardware (40+ TOPS)
//! - Windows 11 24H2/25H2 or later
//! - AI Dev Gallery installed (provides Windows App SDK AI runtime)
//! - App must be packaged as MSIX with systemAIModels capability
//! - Limited Access Feature (LAF) token from Microsoft

use serde::{Deserialize, Serialize};

/// LAF constants for Phi Silica access
#[cfg(windows)]
const LAF_FEATURE_ID: &str = "com.microsoft.windows.ai.languagemodel";
#[cfg(windows)]
const LAF_TOKEN: &str = "edibyiYSeHx+qsGpzHNoCQ==";
#[cfg(windows)]
const LAF_PUBLISHER_ID: &str = "t6j5qexy2jpp2"; // From package family name: 32827MikeFara.WindowsForumDiagnostics_t6j5qexy2jpp2

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
    /// Ready state from the API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_state: Option<String>,
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

/// Check if AI Dev Gallery is installed
#[cfg(windows)]
fn check_ai_dev_gallery_installed() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    std::process::Command::new("powershell")
        .args(["-Command", "(Get-AppxPackage -Name 'Microsoft.AIDevGallery').Name"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// Initialize WinRT runtime (required before using WinRT APIs)
#[cfg(windows)]
fn ensure_winrt_initialized() {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    // RoInitialize is safe to call multiple times - it will return S_FALSE if already initialized
    // Use multi-threaded apartment for Windows App SDK AI APIs
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

/// Track if bootstrapper has been initialized
#[cfg(windows)]
static BOOTSTRAPPER_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Track if LAF has been unlocked
#[cfg(windows)]
static LAF_UNLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Unlock the Limited Access Feature for Phi Silica
/// Returns (success, status_message)
#[cfg(windows)]
fn try_unlock_laf() -> (bool, String) {
    use std::sync::atomic::Ordering;
    use windows::ApplicationModel::{
        LimitedAccessFeatureStatus, LimitedAccessFeatures,
    };
    use windows_core::HSTRING;

    // Only try once
    if LAF_UNLOCKED.load(Ordering::SeqCst) {
        return (true, "LAF already unlocked".to_string());
    }

    let feature_id = HSTRING::from(LAF_FEATURE_ID);
    let token = HSTRING::from(LAF_TOKEN);
    let attestation = HSTRING::from(format!(
        "{} has registered their use of {} with Microsoft and agrees to the terms of use.",
        LAF_PUBLISHER_ID, LAF_FEATURE_ID
    ));

    match LimitedAccessFeatures::TryUnlockFeature(&feature_id, &token, &attestation) {
        Ok(result) => {
            let status = result.Status().unwrap_or(LimitedAccessFeatureStatus::Unknown);
            let status_name = match status {
                LimitedAccessFeatureStatus::Available => "Available",
                LimitedAccessFeatureStatus::AvailableWithoutToken => "AvailableWithoutToken",
                LimitedAccessFeatureStatus::Unknown => "Unknown",
                LimitedAccessFeatureStatus::Unavailable => "Unavailable",
                _ => "Unknown",
            };

            if status == LimitedAccessFeatureStatus::Available
                || status == LimitedAccessFeatureStatus::AvailableWithoutToken
            {
                LAF_UNLOCKED.store(true, Ordering::SeqCst);
                (true, format!("LAF unlocked successfully (status: {})", status_name))
            } else {
                (false, format!("LAF unlock returned status: {}", status_name))
            }
        }
        Err(e) => {
            let code = e.code().0 as u32;
            (false, format!("LAF unlock failed: 0x{:08X}: {}", code, e.message()))
        }
    }
}

/// Initialize Windows App SDK bootstrapper for AI APIs access
/// This is required for unpackaged apps to access Windows App SDK features
#[cfg(windows)]
fn init_windows_app_sdk() -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    // Only initialize once
    if BOOTSTRAPPER_INITIALIZED.load(Ordering::SeqCst) {
        return Ok(());
    }

    // MddBootstrapInitialize2 function signature
    type MddBootstrapInitialize2Fn = unsafe extern "system" fn(
        major_minor_version: u32,
        version_tag: PCWSTR,
        min_version: u64,
        options: u32,
    ) -> windows::core::HRESULT;

    unsafe {
        // Try to load the bootstrapper DLL
        let dll_name = windows::core::w!("Microsoft.WindowsAppRuntime.Bootstrap.dll");
        let module: HMODULE = match LoadLibraryW(dll_name) {
            Ok(m) => m,
            Err(_) => {
                // DLL not found - might be running as packaged app without the DLL
                return Ok(());
            }
        };

        // Get MddBootstrapInitialize2 function
        let init_fn = GetProcAddress(module, windows::core::s!("MddBootstrapInitialize2"));
        if init_fn.is_none() {
            return Ok(()); // Function not found, skip
        }

        let init: MddBootstrapInitialize2Fn = std::mem::transmute(init_fn.unwrap());

        // Try multiple Windows App SDK versions (1.8, 1.7, 1.6)
        let versions: [(u32, &str); 3] = [
            (0x00010008, "1.8"),
            (0x00010007, "1.7"),
            (0x00010006, "1.6"),
        ];

        for (major_minor, version_name) in versions {
            let hr = init(major_minor, PCWSTR::null(), 0, 0);
            if hr.is_ok() {
                BOOTSTRAPPER_INITIALIZED.store(true, Ordering::SeqCst);
                return Ok(());
            }
            // 0x80070032 = ERROR_NOT_SUPPORTED (packaged app, bootstrapper not needed)
            // 0x80040154 = CLASS_E_CLASSNOTREGISTERED
            let code = hr.0 as u32;
            if code == 0x80070032 {
                // Packaged app - bootstrapper not supported but not needed
                return Ok(());
            }
            // Try next version
        }
    }

    Ok(())
}

/// Get the app's installation directory (where the exe and bundled DLLs are)
#[cfg(windows)]
fn get_app_directory() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// Cached state for DLL loading
#[cfg(windows)]
static AI_TEXT_DLL_LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Store the loaded DLL module handle
#[cfg(windows)]
static mut AI_TEXT_DLL_MODULE: Option<windows::Win32::Foundation::HMODULE> = None;

/// Try to load the AI DLL from our app's bundled directory
#[cfg(windows)]
fn try_direct_dll_activation() -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use windows::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_core::PCWSTR;

    // Only load once
    if AI_TEXT_DLL_LOADED.load(Ordering::SeqCst) {
        return Ok(());
    }

    log_phi_silica("Attempting to load bundled AI DLLs...");

    // Get app directory where bundled DLLs are
    let app_dir = get_app_directory()
        .ok_or_else(|| "Failed to get app directory".to_string())?;

    log_phi_silica(&format!("App directory: {:?}", app_dir));

    // Load the DLLs in dependency order
    let dlls = [
        "Microsoft.WindowsAppRuntime.dll",
        "Microsoft.Windows.AI.Text.dll",
    ];

    for dll_name in &dlls {
        let dll_path = app_dir.join(dll_name);
        if !dll_path.exists() {
            log_phi_silica(&format!("DLL not found: {:?}", dll_path));
            continue;
        }

        log_phi_silica(&format!("Loading: {:?}", dll_path));

        let dll_path_wide: Vec<u16> = dll_path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            match LoadLibraryW(PCWSTR::from_raw(dll_path_wide.as_ptr())) {
                Ok(module) => {
                    log_phi_silica(&format!("Loaded: {}", dll_name));
                    if dll_name == &"Microsoft.Windows.AI.Text.dll" {
                        AI_TEXT_DLL_MODULE = Some(module);
                    }
                },
                Err(e) => log_phi_silica(&format!("Failed to load {}: {}", dll_name, e.message())),
            }
        }
    }

    AI_TEXT_DLL_LOADED.store(true, Ordering::SeqCst);
    log_phi_silica("Bundled DLLs loaded");

    Ok(())
}

/// Create LanguageModel using DllGetActivationFactory from bundled DLL
/// This bypasses RoGetActivationFactory entirely, like CsWinRT does
#[cfg(windows)]
fn create_language_model_direct() -> Result<crate::windows_ai_bindings::LanguageModel, String> {
    use windows::Win32::System::LibraryLoader::GetProcAddress;
    use windows_core::{Interface, HSTRING};
    use crate::windows_ai_bindings::{ILanguageModelStatics, LanguageModel};

    log_phi_silica("Creating LanguageModel via DllGetActivationFactory...");

    // Ensure DLLs are loaded
    try_direct_dll_activation()?;

    let module = unsafe { AI_TEXT_DLL_MODULE }
        .ok_or_else(|| "AI Text DLL not loaded".to_string())?;

    // DllGetActivationFactory signature:
    // HRESULT DllGetActivationFactory(HSTRING classId, IActivationFactory** factory)
    type DllGetActivationFactoryFn = unsafe extern "system" fn(
        class_id: *mut std::ffi::c_void,      // HSTRING (passed by value, it's a pointer)
        factory: *mut *mut std::ffi::c_void,  // IActivationFactory**
    ) -> windows_core::HRESULT;

    let proc = unsafe { GetProcAddress(module, windows::core::s!("DllGetActivationFactory")) };
    let get_factory: DllGetActivationFactoryFn = match proc {
        Some(p) => unsafe { std::mem::transmute(p) },
        None => return Err("DllGetActivationFactory not found in DLL".to_string()),
    };

    log_phi_silica("Got DllGetActivationFactory");

    // Create HSTRING for the class name
    let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");

    // Get the raw HSTRING handle - HSTRING is repr(transparent) over a pointer
    let hstring_raw: *mut std::ffi::c_void = unsafe { std::mem::transmute_copy(&class_name) };

    let mut factory_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

    log_phi_silica(&format!("Calling DllGetActivationFactory with class: {}", class_name));

    let hr = unsafe { get_factory(hstring_raw, &mut factory_ptr) };

    if hr.is_err() {
        log_phi_silica(&format!("DllGetActivationFactory failed: 0x{:08X}", hr.0 as u32));
        return Err(format!("DllGetActivationFactory failed: 0x{:08X}", hr.0 as u32));
    }

    if factory_ptr.is_null() {
        return Err("DllGetActivationFactory returned null factory".to_string());
    }

    log_phi_silica("Got activation factory, querying for ILanguageModelStatics...");

    // Cast to IActivationFactory and then query for ILanguageModelStatics
    let factory: windows_core::IInspectable = unsafe {
        windows_core::IInspectable::from_raw(factory_ptr)
    };

    let statics: ILanguageModelStatics = factory.cast()
        .map_err(|e| format!("Failed to get ILanguageModelStatics: {}", e.message()))?;

    log_phi_silica("Got ILanguageModelStatics, calling CreateAsync...");

    // Call CreateAsync
    let async_op = unsafe {
        let mut result = std::mem::zeroed();
        let vtable = statics.as_raw() as *const *const crate::windows_ai_bindings::ILanguageModelStatics_Vtbl;
        let hr = ((**vtable).CreateAsync)(
            statics.as_raw(),
            &mut result,
        );
        if hr.is_err() {
            log_phi_silica(&format!("CreateAsync call failed: 0x{:08X}", hr.0 as u32));
            return Err(format!("CreateAsync failed: 0x{:08X}", hr.0 as u32));
        }
        windows_future::IAsyncOperation::<LanguageModel>::from_raw(result)
    };

    log_phi_silica("CreateAsync started, waiting...");

    // Wait for async operation
    let model = wait_for_async_blocking(async_op)?;

    log_phi_silica("LanguageModel created successfully via direct activation!");

    Ok(model)
}

/// Check Phi Silica availability using GetReadyState (like AI Dev Gallery does)
#[cfg(windows)]
fn check_phi_silica_safe() -> (bool, String, Option<String>) {
    use crate::windows_ai_bindings::{LanguageModel, AIFeatureReadyState};

    log_phi_silica("=== check_phi_silica_safe called ===");

    // Ensure WinRT is initialized before calling any WinRT APIs
    ensure_winrt_initialized();
    log_phi_silica("WinRT initialized");

    // Try to initialize Windows App SDK bootstrapper (may fail for packaged apps, that's OK)
    let _ = init_windows_app_sdk();
    log_phi_silica("Windows App SDK init attempted");

    // Try to pre-load the AI DLL directly to ensure it's available for activation
    match try_direct_dll_activation() {
        Ok(()) => log_phi_silica("Direct DLL activation succeeded"),
        Err(e) => log_phi_silica(&format!("Direct DLL activation failed (continuing anyway): {}", e)),
    }

    let build = get_windows_build().unwrap_or(0);
    log_phi_silica(&format!("Windows build: {}", build));

    // Try to unlock Limited Access Feature BEFORE accessing Phi Silica APIs
    let (laf_success, laf_message) = try_unlock_laf();
    log_phi_silica(&format!("LAF unlock: success={}, msg={}", laf_success, laf_message));

    // Store LAF status for error reporting
    let laf_status_str = if laf_success {
        format!("LAF: OK ({})", laf_message)
    } else {
        format!("LAF: FAILED ({})", laf_message)
    };

    // Phi Silica requires Windows 11 24H2 (build 26100+) with a Copilot+ PC
    if build < 26100 {
        log_phi_silica("Build too old, returning");
        return (false, format!(
            "Phi Silica requires Windows 11 24H2 or later (build 26100+). Current build: {}",
            build
        ), None);
    }

    // Use GetReadyState() like AI Dev Gallery does - this is the correct way to check
    log_phi_silica("Calling LanguageModel::GetReadyState()...");
    match LanguageModel::GetReadyState() {
        Ok(state) => {
            log_phi_silica(&format!("GetReadyState succeeded: state={:?}", state.0));
            if state == AIFeatureReadyState::Ready {
                (true, format!("Phi Silica is ready. Build: {}", build), Some("Ready".to_string()))
            } else if state == AIFeatureReadyState::NotReady {
                // Model needs to be downloaded/initialized
                (true, format!("Phi Silica available but not ready. Build: {}", build), Some("NotReady".to_string()))
            } else if state == AIFeatureReadyState::DisabledByUser {
                (false, format!("Phi Silica disabled by user. Build: {}", build), Some("DisabledByUser".to_string()))
            } else if state == AIFeatureReadyState::NotSupportedOnCurrentSystem {
                (false, format!("Phi Silica not supported on this system (requires Copilot+ PC with NPU). Build: {}", build), Some("NotSupportedOnCurrentSystem".to_string()))
            } else {
                (false, format!("Phi Silica unknown state: {:?}. Build: {}", state.0, build), Some(format!("Unknown({})", state.0)))
            }
        }
        Err(e) => {
            let code = e.code().0 as u32;
            log_phi_silica(&format!("GetReadyState FAILED: 0x{:08X} {}", code, e.message()));
            if code == 0x80040154 {
                // CLASS_E_CLASSNOTREGISTERED - API not available
                // This happens when the Windows AI runtime is not present
                (false, format!(
                    "Phi Silica API not registered (0x{:08X}). Build: {}. \
                     Requires Copilot+ PC with Windows AI features enabled.",
                    code, build
                ), Some(format!("0x{:08X}", code)))
            } else if code == 0x80070005 {
                // E_ACCESSDENIED - LAF unlock may have failed
                (false, format!(
                    "Phi Silica access denied (0x80070005). {}. Build: {}.",
                    laf_status_str, build
                ), Some(format!("LAF_REQUIRED ({})", laf_status_str)))
            } else {
                (false, format!(
                    "Failed to check Phi Silica: 0x{:08X}: {}. {}. Build: {}",
                    code, e.message(), laf_status_str, build
                ), Some(format!("0x{:08X}", code)))
            }
        }
    }
}

/// Check if Phi Silica is available on this device
#[cfg(windows)]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    let build = get_windows_build();
    let (available, message, ready_state) = check_phi_silica_safe();

    PhiSilicaStatus {
        available,
        message,
        error_code: None,
        windows_build: build,
        ready_state,
    }
}

#[cfg(not(windows))]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    PhiSilicaStatus {
        available: false,
        message: "Phi Silica is only available on Windows".to_string(),
        error_code: None,
        windows_build: None,
        ready_state: None,
    }
}

/// Blocking wait for an async operation - runs in spawn_blocking to be Send-safe
#[cfg(windows)]
fn wait_for_async_blocking<T>(op: windows_future::IAsyncOperation<T>) -> Result<T, String>
where
    T: windows_core::RuntimeType,
{
    use std::time::Duration;
    use std::thread::sleep;
    use windows_core::Interface;
    use windows_future::{IAsyncInfo, AsyncStatus};

    // Poll the async operation
    let info: IAsyncInfo = op.cast().map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    loop {
        let status = info.Status().map_err(|e| format!("Failed to get status: {}", e.message()))?;
        match status {
            AsyncStatus::Completed => {
                return op.GetResults().map_err(|e| format!("Failed to get results: {}", e.message()));
            }
            AsyncStatus::Error => {
                let hr = info.ErrorCode().map_err(|e| format!("Failed to get error: {}", e.message()))?;
                return Err(format!("Async operation failed: 0x{:08X}", hr.0 as u32));
            }
            AsyncStatus::Canceled => {
                return Err("Async operation was canceled".to_string());
            }
            AsyncStatus::Started => {
                // Still running, wait a bit
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(format!("Unknown async status: {:?}", status));
            }
        }
    }
}

/// Blocking wait for an async operation with progress - runs in spawn_blocking to be Send-safe
#[cfg(windows)]
fn wait_for_async_with_progress_blocking<T, P>(op: windows_future::IAsyncOperationWithProgress<T, P>) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    use std::time::Duration;
    use std::thread::sleep;
    use windows_core::Interface;
    use windows_future::{IAsyncInfo, AsyncStatus};

    // Poll the async operation
    let info: IAsyncInfo = op.cast().map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    loop {
        let status = info.Status().map_err(|e| format!("Failed to get status: {}", e.message()))?;
        match status {
            AsyncStatus::Completed => {
                return op.GetResults().map_err(|e| format!("Failed to get results: {}", e.message()));
            }
            AsyncStatus::Error => {
                let hr = info.ErrorCode().map_err(|e| format!("Failed to get error: {}", e.message()))?;
                return Err(format!("Async operation failed: 0x{:08X}", hr.0 as u32));
            }
            AsyncStatus::Canceled => {
                return Err("Async operation was canceled".to_string());
            }
            AsyncStatus::Started => {
                // Still running, wait a bit
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(format!("Unknown async status: {:?}", status));
            }
        }
    }
}

/// Log to file for debugging MSIX apps
#[cfg(windows)]
fn log_phi_silica(msg: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    // Use C:\temp which should be accessible
    let log_path = std::path::Path::new("C:\\temp\\phi-silica-rust.log");
    // Create dir if needed
    let _ = std::fs::create_dir_all("C:\\temp");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(file, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
    }
}

/// Ensure the Phi Silica model is ready
#[cfg(windows)]
pub async fn ensure_phi_silica_ready() -> Result<(), String> {
    use tokio::task::spawn_blocking;

    // Run blocking COM code in spawn_blocking to make it Send-safe
    spawn_blocking(|| {
        log_phi_silica("=== ensure_phi_silica_ready called ===");

        // Ensure WinRT is initialized and LAF is unlocked
        ensure_winrt_initialized();
        let (laf_ok, laf_msg) = try_unlock_laf();
        log_phi_silica(&format!("LAF unlock: ok={}, msg={}", laf_ok, laf_msg));

        // Use direct DLL activation to create the model
        // This bypasses RoGetActivationFactory and uses DllGetActivationFactory directly
        let _model = create_language_model_direct()?;

        log_phi_silica("Model created successfully!");
        Ok(())
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

#[cfg(not(windows))]
pub async fn ensure_phi_silica_ready() -> Result<(), String> {
    Err("Phi Silica is only available on Windows".to_string())
}

/// Generate a response using Phi Silica
#[cfg(windows)]
pub async fn generate_response(prompt: &str) -> Result<String, String> {
    use tokio::task::spawn_blocking;

    let prompt_owned = prompt.to_string();

    // Run blocking COM code in spawn_blocking to make it Send-safe
    spawn_blocking(move || {
        use crate::windows_ai_bindings::LanguageModelResponseStatus;
        use windows_core::HSTRING;

        // Ensure WinRT is initialized and LAF is unlocked
        ensure_winrt_initialized();
        let _ = try_unlock_laf();

        // Create model using direct DLL activation
        let model = create_language_model_direct()?;

        // Generate response
        let prompt_hstring = HSTRING::from(prompt_owned.as_str());
        let response_op = model.GenerateResponseAsync(&prompt_hstring)
            .map_err(|e| format!("Failed to start response generation: {}", e.message()))?;

        let response = wait_for_async_with_progress_blocking(response_op)?;

        // Check status
        let status = response.Status()
            .map_err(|e| format!("Failed to get response status: {}", e.message()))?;

        match status {
            s if s == LanguageModelResponseStatus::Complete || s == LanguageModelResponseStatus::InProgress => {
                let text = response.Text()
                    .map_err(|e| format!("Failed to get response text: {}", e.message()))?;
                Ok(text.to_string())
            }
            s if s == LanguageModelResponseStatus::BlockedByPolicy => {
                Err("Response blocked by policy".to_string())
            }
            s if s == LanguageModelResponseStatus::PromptBlockedByContentModeration => {
                Err("Prompt blocked by content moderation".to_string())
            }
            s if s == LanguageModelResponseStatus::ResponseBlockedByContentModeration => {
                Err("Response blocked by content moderation".to_string())
            }
            s if s == LanguageModelResponseStatus::PromptLargerThanContext => {
                Err("Prompt is too large for the model context".to_string())
            }
            s if s == LanguageModelResponseStatus::Error => {
                Err("An error occurred during generation".to_string())
            }
            _ => {
                Err(format!("Unknown response status: {:?}", status))
            }
        }
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
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

/// Tauri command to ensure Phi Silica is ready (downloads model if needed)
#[tauri::command]
pub async fn ensure_phi_silica() -> Result<String, String> {
    ensure_phi_silica_ready().await?;
    Ok("Phi Silica is ready".to_string())
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

    Ok("Opening Windows Update. Check for updates to install Phi Silica component.".to_string())
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
