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

use crate::error::DiagError;

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

/// Response from Phi Silica analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiSilicaAnalysisResponse {
    pub analysis: String,
    pub diagnostics_run: Vec<String>,
    pub provider: String,
}

/// Get Windows build number
#[cfg(windows)]
fn get_windows_build() -> Option<u32> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?;
    let build_str: String = key.get_value("CurrentBuildNumber").ok()?;
    build_str.parse().ok()
}

/// Check if AI Dev Gallery is installed
#[allow(dead_code)]
#[cfg(windows)]
fn check_ai_dev_gallery_installed() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    std::process::Command::new("powershell")
        .args([
            "-Command",
            "(Get-AppxPackage -Name 'Microsoft.AIDevGallery').Name",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// Initialize WinRT runtime (required before using WinRT APIs)
#[cfg(windows)]
fn ensure_winrt_initialized() {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    // RoInitialize is safe to call multiple times - it will return S_FALSE if already initialized
    // Use multi-threaded apartment for Windows App SDK AI APIs
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

/// Track if bootstrapper has been initialized
#[cfg(windows)]
static BOOTSTRAPPER_INITIALIZED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Track if LAF has been unlocked
#[cfg(windows)]
static LAF_UNLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Unlock the Limited Access Feature for Phi Silica
/// Returns (success, status_message)
#[cfg(windows)]
fn try_unlock_laf() -> (bool, String) {
    use std::sync::atomic::Ordering;
    use windows::ApplicationModel::{LimitedAccessFeatureStatus, LimitedAccessFeatures};
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
            let status = result
                .Status()
                .unwrap_or(LimitedAccessFeatureStatus::Unknown);
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
                (
                    true,
                    format!("LAF unlocked successfully (status: {})", status_name),
                )
            } else {
                (
                    false,
                    format!("LAF unlock returned status: {}", status_name),
                )
            }
        }
        Err(e) => {
            let code = e.code().0 as u32;
            (
                false,
                format!("LAF unlock failed: 0x{:08X}: {}", code, e.message()),
            )
        }
    }
}

/// Initialize Windows App SDK bootstrapper for AI APIs access
/// This is required for unpackaged apps to access Windows App SDK features
#[cfg(windows)]
fn init_windows_app_sdk() -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::PCWSTR;

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

        for (major_minor, _version_name) in versions {
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
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

/// Cached state for DLL loading
#[cfg(windows)]
static AI_TEXT_DLL_LOADED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        .ok_or_else(|| DiagError::ai_unavailable("phi_silica", "Failed to get app directory"))?;

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

        let dll_path_wide: Vec<u16> = dll_path
            .to_string_lossy()
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
                }
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
    use crate::windows_ai_bindings::{ILanguageModelStatics, LanguageModel};
    use windows::Win32::System::LibraryLoader::GetProcAddress;
    use windows_core::{HSTRING, Interface};

    log_phi_silica("Creating LanguageModel via DllGetActivationFactory...");

    // Ensure DLLs are loaded
    try_direct_dll_activation()?;

    let module = unsafe { AI_TEXT_DLL_MODULE }
        .ok_or_else(|| DiagError::ai_unavailable("phi_silica", "AI Text DLL not loaded"))?;

    // DllGetActivationFactory signature:
    // HRESULT DllGetActivationFactory(HSTRING classId, IActivationFactory** factory)
    type DllGetActivationFactoryFn = unsafe extern "system" fn(
        class_id: *mut std::ffi::c_void, // HSTRING (passed by value, it's a pointer)
        factory: *mut *mut std::ffi::c_void, // IActivationFactory**
    ) -> windows_core::HRESULT;

    let proc = unsafe { GetProcAddress(module, windows::core::s!("DllGetActivationFactory")) };
    let get_factory: DllGetActivationFactoryFn = match proc {
        Some(p) => unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, DllGetActivationFactoryFn>(
                p,
            )
        },
        None => {
            return Err(DiagError::ai_unavailable(
                "phi_silica",
                "DllGetActivationFactory not found in DLL",
            )
            .into());
        }
    };

    log_phi_silica("Got DllGetActivationFactory");

    // Create HSTRING for the class name
    let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");

    // Get the raw HSTRING handle - HSTRING is repr(transparent) over a pointer
    let hstring_raw: *mut std::ffi::c_void = unsafe { std::mem::transmute_copy(&class_name) };

    let mut factory_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

    log_phi_silica(&format!(
        "Calling DllGetActivationFactory with class: {}",
        class_name
    ));

    let hr = unsafe { get_factory(hstring_raw, &mut factory_ptr) };

    if hr.is_err() {
        log_phi_silica(&format!(
            "DllGetActivationFactory failed: 0x{:08X}",
            hr.0 as u32
        ));
        return Err(DiagError::ai_unavailable(
            "phi_silica",
            format!("DllGetActivationFactory failed: 0x{:08X}", hr.0 as u32),
        )
        .into());
    }

    if factory_ptr.is_null() {
        return Err(DiagError::ai_unavailable(
            "phi_silica",
            "DllGetActivationFactory returned null factory",
        )
        .into());
    }

    log_phi_silica("Got activation factory, querying for ILanguageModelStatics...");

    // Cast to IActivationFactory and then query for ILanguageModelStatics
    let factory: windows_core::IInspectable =
        unsafe { windows_core::IInspectable::from_raw(factory_ptr) };

    let statics: ILanguageModelStatics = factory
        .cast()
        .map_err(|e| format!("Failed to get ILanguageModelStatics: {}", e.message()))?;

    log_phi_silica("Got ILanguageModelStatics, calling CreateAsync...");

    // Call CreateAsync
    let async_op = unsafe {
        let mut result = std::mem::zeroed();
        let vtable = statics.as_raw()
            as *const *const crate::windows_ai_bindings::ILanguageModelStatics_Vtbl;
        let hr = ((**vtable).CreateAsync)(statics.as_raw(), &mut result);
        if hr.is_err() {
            log_phi_silica(&format!("CreateAsync call failed: 0x{:08X}", hr.0 as u32));
            return Err(DiagError::ai_unavailable(
                "phi_silica",
                format!("CreateAsync failed: 0x{:08X}", hr.0 as u32),
            )
            .into());
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
// Returns (available, message, ready_state, error_code). ready_state carries the
// AIFeatureReadyState on the success path; error_code carries the HRESULT/LAF string on
// the failure path. They are kept SEPARATE so the frontend's PhiSilicaStatus.error_code
// is populated correctly instead of error info being mislabeled into ready_state.
fn check_phi_silica_safe() -> (bool, String, Option<String>, Option<String>) {
    use crate::windows_ai_bindings::{AIFeatureReadyState, LanguageModel};

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
        Err(e) => log_phi_silica(&format!(
            "Direct DLL activation failed (continuing anyway): {}",
            e
        )),
    }

    let build = get_windows_build().unwrap_or(0);
    log_phi_silica(&format!("Windows build: {}", build));

    // Try to unlock Limited Access Feature BEFORE accessing Phi Silica APIs
    let (laf_success, laf_message) = try_unlock_laf();
    log_phi_silica(&format!(
        "LAF unlock: success={}, msg={}",
        laf_success, laf_message
    ));

    // Store LAF status for error reporting
    let laf_status_str = if laf_success {
        format!("LAF: OK ({})", laf_message)
    } else {
        format!("LAF: FAILED ({})", laf_message)
    };

    // Phi Silica requires Windows 11 24H2 (build 26100+) with a Copilot+ PC
    if build < 26100 {
        log_phi_silica("Build too old, returning");
        return (
            false,
            format!(
                "Phi Silica requires Windows 11 24H2 or later (build 26100+). Current build: {}",
                build
            ),
            None,
            None,
        );
    }

    // Use GetReadyState() like AI Dev Gallery does - this is the correct way to check
    log_phi_silica("Calling LanguageModel::GetReadyState()...");
    match LanguageModel::GetReadyState() {
        Ok(state) => {
            log_phi_silica(&format!("GetReadyState succeeded: state={:?}", state.0));
            // GetReadyState succeeded → these are READY STATES, not errors (error_code None).
            if state == AIFeatureReadyState::Ready {
                (
                    true,
                    format!("Phi Silica is ready. Build: {}", build),
                    Some("Ready".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::NotReady {
                // Model needs to be downloaded/initialized
                (
                    true,
                    format!("Phi Silica available but not ready. Build: {}", build),
                    Some("NotReady".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::DisabledByUser {
                (
                    false,
                    format!("Phi Silica disabled by user. Build: {}", build),
                    Some("DisabledByUser".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::NotSupportedOnCurrentSystem {
                (
                    false,
                    format!(
                        "Phi Silica not supported on this system (requires Copilot+ PC with NPU). Build: {}",
                        build
                    ),
                    Some("NotSupportedOnCurrentSystem".to_string()),
                    None,
                )
            } else {
                (
                    false,
                    format!("Phi Silica unknown state: {:?}. Build: {}", state.0, build),
                    Some(format!("Unknown({})", state.0)),
                    None,
                )
            }
        }
        Err(e) => {
            // GetReadyState failed → carry the HRESULT/LAF in error_code, ready_state None.
            let code = e.code().0 as u32;
            log_phi_silica(&format!(
                "GetReadyState FAILED: 0x{:08X} {}",
                code,
                e.message()
            ));

            // GetReadyState() resolves its factory through RoGetActivationFactory, which is
            // blocked for third-party apps and returns exactly these HRESULTs — yet real
            // inference (generate_response) uses the bundled-DLL DllGetActivationFactory
            // path instead. Before declaring Phi Silica unavailable, fall back to that same
            // direct path so the availability gate matches what inference can actually do
            // (otherwise we false-negative on working Copilot+ PCs).
            if code == 0x80040154 || code == 0x80070005 {
                log_phi_silica(
                    "GetReadyState blocked (Ro path); attempting direct DLL activation...",
                );
                match create_language_model_direct() {
                    Ok(_) => {
                        log_phi_silica("Direct DLL activation succeeded — Phi Silica IS available");
                        return (
                            true,
                            format!(
                                "Phi Silica is ready (via direct DLL activation). Build: {}",
                                build
                            ),
                            Some("Ready".to_string()),
                            None,
                        );
                    }
                    Err(direct_err) => {
                        log_phi_silica(&format!(
                            "Direct DLL activation also failed: {}",
                            direct_err
                        ));
                    }
                }
            }

            if code == 0x80040154 {
                // CLASS_E_CLASSNOTREGISTERED - API not available
                // This happens when the Windows AI runtime is not present
                (
                    false,
                    format!(
                        "Phi Silica API not registered (0x{:08X}). Build: {}. \
                     Requires Copilot+ PC with Windows AI features enabled.",
                        code, build
                    ),
                    None,
                    Some(format!("0x{:08X}", code)),
                )
            } else if code == 0x80070005 {
                // E_ACCESSDENIED - LAF unlock may have failed
                (
                    false,
                    format!(
                        "Phi Silica access denied (0x80070005). {}. Build: {}.",
                        laf_status_str, build
                    ),
                    None,
                    Some(format!("LAF_REQUIRED ({})", laf_status_str)),
                )
            } else {
                (
                    false,
                    format!(
                        "Failed to check Phi Silica: 0x{:08X}: {}. {}. Build: {}",
                        code,
                        e.message(),
                        laf_status_str,
                        build
                    ),
                    None,
                    Some(format!("0x{:08X}", code)),
                )
            }
        }
    }
}

/// Check if Phi Silica is available on this device
#[cfg(windows)]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    let build = get_windows_build();
    let (available, message, ready_state, error_code) = check_phi_silica_safe();

    PhiSilicaStatus {
        available,
        message,
        error_code,
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
    use std::thread::sleep;
    use std::time::Duration;
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    // Poll the async operation
    let info: IAsyncInfo = op
        .cast()
        .map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    loop {
        let status = info
            .Status()
            .map_err(|e| format!("Failed to get status: {}", e.message()))?;
        match status {
            AsyncStatus::Completed => {
                return op
                    .GetResults()
                    .map_err(|e| format!("Failed to get results: {}", e.message()));
            }
            AsyncStatus::Error => {
                let hr = info
                    .ErrorCode()
                    .map_err(|e| format!("Failed to get error: {}", e.message()))?;
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    format!("Async operation failed: 0x{:08X}", hr.0 as u32),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    "Async operation was canceled",
                )
                .into());
            }
            AsyncStatus::Started => {
                // Still running, wait a bit
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {:?}", status),
                )
                .into());
            }
        }
    }
}

/// Blocking wait for an async operation with progress - runs in spawn_blocking to be Send-safe
#[cfg(windows)]
pub(crate) fn wait_for_async_with_progress_blocking<T, P>(
    op: windows_future::IAsyncOperationWithProgress<T, P>,
) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    use std::thread::sleep;
    use std::time::Duration;
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    // Poll the async operation
    let info: IAsyncInfo = op
        .cast()
        .map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    loop {
        let status = info
            .Status()
            .map_err(|e| format!("Failed to get status: {}", e.message()))?;
        match status {
            AsyncStatus::Completed => {
                return op
                    .GetResults()
                    .map_err(|e| format!("Failed to get results: {}", e.message()));
            }
            AsyncStatus::Error => {
                let hr = info
                    .ErrorCode()
                    .map_err(|e| format!("Failed to get error: {}", e.message()))?;
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    format!("Async operation failed: 0x{:08X}", hr.0 as u32),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    "Async operation was canceled",
                )
                .into());
            }
            AsyncStatus::Started => {
                // Still running, wait a bit
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(DiagError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {:?}", status),
                )
                .into());
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
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(
            file,
            "[{}] {}",
            crate::timestamp::format_time(std::time::SystemTime::now()),
            msg
        );
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
    Err(DiagError::PlatformNotSupported {
        operation: "Phi Silica".to_string(),
    }
    .into())
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
        let response_op = model
            .GenerateResponseAsync(&prompt_hstring)
            .map_err(|e| format!("Failed to start response generation: {}", e.message()))?;

        let response = wait_for_async_with_progress_blocking(response_op)?;

        // Check status
        let status = response
            .Status()
            .map_err(|e| format!("Failed to get response status: {}", e.message()))?;

        match status {
            s if s == LanguageModelResponseStatus::Complete
                || s == LanguageModelResponseStatus::InProgress =>
            {
                let text = response
                    .Text()
                    .map_err(|e| format!("Failed to get response text: {}", e.message()))?;
                Ok(text.to_string())
            }
            s if s == LanguageModelResponseStatus::BlockedByPolicy => {
                Err(DiagError::AiAnalysisFailed {
                    reason: "Response blocked by policy".to_string(),
                }
                .into())
            }
            s if s == LanguageModelResponseStatus::PromptBlockedByContentModeration => {
                Err(DiagError::AiAnalysisFailed {
                    reason: "Prompt blocked by content moderation".to_string(),
                }
                .into())
            }
            s if s == LanguageModelResponseStatus::ResponseBlockedByContentModeration => {
                Err(DiagError::AiAnalysisFailed {
                    reason: "Response blocked by content moderation".to_string(),
                }
                .into())
            }
            s if s == LanguageModelResponseStatus::PromptLargerThanContext => {
                Err(DiagError::AiAnalysisFailed {
                    reason: "Prompt is too large for the model context".to_string(),
                }
                .into())
            }
            s if s == LanguageModelResponseStatus::Error => Err(DiagError::AiAnalysisFailed {
                reason: "An error occurred during generation".to_string(),
            }
            .into()),
            _ => Err(DiagError::AiAnalysisFailed {
                reason: format!("Unknown response status: {:?}", status),
            }
            .into()),
        }
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

#[cfg(not(windows))]
pub async fn generate_response(_prompt: &str) -> Result<String, String> {
    Err(DiagError::PlatformNotSupported {
        operation: "Phi Silica".to_string(),
    }
    .into())
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
    use std::ptr;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR};

    let uri = HSTRING::from("ms-settings:windowsupdate");
    let open = HSTRING::from("open");

    unsafe {
        let result = ShellExecuteW(
            None,
            PCWSTR(open.as_ptr()),
            PCWSTR(uri.as_ptr()),
            PCWSTR(ptr::null()),
            PCWSTR(ptr::null()),
            SW_SHOWNORMAL,
        );

        // ShellExecuteW returns a value > 32 on success
        if result.0 as i32 <= 32 {
            return Err(format!(
                "Failed to open Windows Update: error code {}",
                result.0 as i32
            ));
        }
    }

    Ok("Opening Windows Update. Check for updates to install Phi Silica component.".to_string())
}

#[tauri::command]
#[cfg(not(windows))]
pub async fn check_phi_silica_updates() -> Result<String, String> {
    Err(DiagError::PlatformNotSupported {
        operation: "Windows Update".to_string(),
    }
    .into())
}

/// Build a compact list of available diagnostics for Phi Silica
fn get_diagnostic_list_for_prompt() -> String {
    let tasks = crate::diagnostics::get_all_tasks();
    let mut list = String::new();

    // Provide enhanced descriptions for common diagnostics so Phi Silica knows what data they contain
    let enhanced_descriptions: std::collections::HashMap<&str, &str> = [
        (
            "physical_memory",
            "RAM capacity, speed, manufacturer, modules installed",
        ),
        ("comp_system", "Computer manufacturer, model, system type"),
        ("os_info", "Windows version, build number, install date"),
        ("processor", "CPU name, cores, speed, architecture"),
        (
            "logical_disk",
            "Drive letters, total/free space, file system",
        ),
        ("disk_drive", "Physical disk model, size, interface type"),
        (
            "network_adapter",
            "Network cards, MAC addresses, connection status",
        ),
        (
            "systeminfo",
            "Full system summary including RAM, CPU, OS, network",
        ),
        ("bios", "BIOS version, manufacturer, release date"),
        ("ipconfig", "IP addresses, DNS servers, gateway"),
        (
            "installed_programs",
            "List of installed software with versions",
        ),
        ("services", "Windows services and their status"),
        ("processes", "Running processes and resource usage"),
        ("drivers_list", "Installed drivers and versions"),
        (
            "windows_update",
            "Windows Update history and pending updates",
        ),
        ("event_logs", "Recent system/application event logs"),
        ("startup_command", "Programs that run at startup"),
        ("firewall_status", "Windows Firewall status and rules"),
    ]
    .into_iter()
    .collect();

    // Group by category with enhanced descriptions
    let mut by_category: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for task in tasks {
        // Use enhanced description if available, otherwise use default
        let desc = enhanced_descriptions
            .get(task.id.as_str())
            .map(|d| d.to_string())
            .unwrap_or_else(|| task.description.clone());
        let entry = format!("{}: {}", task.id, desc);
        by_category
            .entry(task.category.clone())
            .or_default()
            .push(entry);
    }

    for (category, entries) in by_category {
        list.push_str(&format!("[{}]\n", category));
        for entry in entries {
            list.push_str(&format!("  {}\n", entry));
        }
    }
    list
}

/// Truncate text to max characters
fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!("{}... [truncated]", &text[..max_chars])
    }
}

/// Smart extraction of key fields from WMI output for specific diagnostics
/// This ensures important data (like RAM capacity) isn't lost to truncation
fn smart_extract(task_id: &str, text: &str) -> String {
    match task_id {
        "physical_memory" => extract_ram_essentials(text),
        "comp_system" => extract_comp_system_essentials(text),
        "processor" => extract_processor_essentials(text),
        "logical_disk" => extract_disk_essentials(text),
        _ => text.to_string(),
    }
}

/// Extract essential RAM info: Capacity, Speed, Manufacturer per module
fn extract_ram_essentials(text: &str) -> String {
    let mut result = String::new();
    let mut module_count = 0;
    let mut total_capacity: u64 = 0;

    // Key fields to extract
    let key_fields = [
        "Capacity",
        "Speed",
        "Manufacturer",
        "MemoryType",
        "FormFactor",
    ];

    // Split into instances (WMI output has multiple objects)
    let instances: Vec<&str> = text.split("---").collect();

    for instance in instances {
        if instance.trim().is_empty() {
            continue;
        }

        let mut module_info = Vec::new();

        for line in instance.lines() {
            let line = line.trim();
            for &field in &key_fields {
                if line.starts_with(field) {
                    // Extract value
                    if let Some(value) = line.split(':').nth(1).or_else(|| line.split('=').nth(1)) {
                        let value = value.trim();
                        if !value.is_empty() && value != "N/A" {
                            // Convert capacity bytes to GB for readability
                            if field == "Capacity" {
                                if let Ok(bytes) = value.parse::<u64>() {
                                    total_capacity += bytes;
                                    let gb = bytes / (1024 * 1024 * 1024);
                                    module_info.push(format!("Capacity: {} GB", gb));
                                } else {
                                    module_info.push(format!("{}: {}", field, value));
                                }
                            } else {
                                module_info.push(format!("{}: {}", field, value));
                            }
                        }
                    }
                    break;
                }
            }
        }

        if !module_info.is_empty() {
            module_count += 1;
            result.push_str(&format!(
                "Module {}: {}\n",
                module_count,
                module_info.join(", ")
            ));
        }
    }

    // Add summary at the top
    let total_gb = total_capacity / (1024 * 1024 * 1024);
    let summary = format!("TOTAL RAM: {} GB ({} modules)\n", total_gb, module_count);

    if result.is_empty() {
        // Fallback: try to extract just capacity values from raw text
        let mut capacities = Vec::new();
        for line in text.lines() {
            if line.contains("Capacity")
                && let Some(val) = line.split(':').nth(1).or_else(|| line.split('=').nth(1))
            {
                let val = val.trim();
                if let Ok(bytes) = val.parse::<u64>() {
                    capacities.push(bytes / (1024 * 1024 * 1024));
                }
            }
        }
        if !capacities.is_empty() {
            let total: u64 = capacities.iter().sum();
            return format!("TOTAL RAM: {} GB\nModules: {:?} GB each", total, capacities);
        }
        // Last resort: return original truncated
        truncate_output(text, 2000)
    } else {
        format!("{}{}", summary, result)
    }
}

/// Extract essential computer system info
fn extract_comp_system_essentials(text: &str) -> String {
    let key_fields = [
        "Manufacturer",
        "Model",
        "SystemType",
        "TotalPhysicalMemory",
        "Name",
        "Domain",
    ];
    extract_key_fields(text, &key_fields)
}

/// Extract essential processor info
fn extract_processor_essentials(text: &str) -> String {
    let key_fields = [
        "Name",
        "NumberOfCores",
        "NumberOfLogicalProcessors",
        "MaxClockSpeed",
        "Architecture",
        "Manufacturer",
    ];
    extract_key_fields(text, &key_fields)
}

/// Extract essential disk info
fn extract_disk_essentials(text: &str) -> String {
    let key_fields = [
        "DeviceID",
        "Size",
        "FreeSpace",
        "FileSystem",
        "VolumeName",
        "DriveType",
    ];
    extract_key_fields(text, &key_fields)
}

/// Generic helper to extract key fields from WMI output
fn extract_key_fields(text: &str, key_fields: &[&str]) -> String {
    let mut result = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        for &field in key_fields {
            if line.starts_with(field) {
                if let Some(value) = line.split(':').nth(1).or_else(|| line.split('=').nth(1)) {
                    let value = value.trim();
                    if !value.is_empty() && value != "N/A" {
                        result.push(format!("{}: {}", field, value));
                    }
                }
                break;
            }
        }
    }

    if result.is_empty() {
        truncate_output(text, 2000)
    } else {
        result.join("\n")
    }
}

/// Parse diagnostic IDs from Phi Silica's JSON response
fn parse_diagnostic_ids(response: &str) -> Vec<String> {
    let all_tasks = crate::diagnostics::get_all_tasks();
    let valid_ids: Vec<&str> = all_tasks.iter().map(|t| t.id.as_str()).collect();
    let mut ids = Vec::new();

    // Try to find JSON array in response
    if let Some(start) = response.find('[')
        && let Some(end) = response[start..].find(']')
    {
        let array_str = &response[start..start + end + 1];
        // Parse as JSON array
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(array_str) {
            // Validate each ID against our known tasks
            for id in arr {
                let id_lower = id.to_lowercase();
                // Exact match
                if valid_ids.contains(&id.as_str()) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                } else {
                    // Try to find best match
                    if let Some(matched) = find_best_match(&id_lower, &valid_ids)
                        && !ids.contains(&matched)
                    {
                        ids.push(matched);
                    }
                }
            }
            if !ids.is_empty() {
                return ids;
            }
        }
        // Try parsing as Value and extract strings
        if let Ok(serde_json::Value::Array(arr)) =
            serde_json::from_str::<serde_json::Value>(array_str)
        {
            for item in arr {
                if let serde_json::Value::String(s) = item {
                    let s_lower = s.to_lowercase();
                    if valid_ids.contains(&s.as_str()) {
                        if !ids.contains(&s) {
                            ids.push(s);
                        }
                    } else if let Some(matched) = find_best_match(&s_lower, &valid_ids)
                        && !ids.contains(&matched)
                    {
                        ids.push(matched);
                    }
                }
            }
            if !ids.is_empty() {
                return ids;
            }
        }
    }

    // Fallback: look for known task IDs mentioned in the response
    for task_id in &valid_ids {
        if (response.contains(&format!("\"{}\"", task_id))
            || response.contains(&format!("'{}'", task_id))
            || response.to_lowercase().contains(&format!(" {} ", task_id))
            || response.to_lowercase().contains(&format!("[{}]", task_id)))
            && !ids.contains(&task_id.to_string())
        {
            ids.push(task_id.to_string());
        }
    }

    ids
}

/// Find best matching task ID for a given string
fn find_best_match(input: &str, valid_ids: &[&str]) -> Option<String> {
    // Direct substring match
    for &id in valid_ids {
        if input.contains(id) || id.contains(input) {
            return Some(id.to_string());
        }
    }

    // Map common terms to actual task IDs
    let mappings: &[(&str, &str)] = &[
        // Security
        ("security", "firewall_status"),
        ("firewall", "firewall_status"),
        ("logged_in", "dsregcmd"),
        ("logged_in_users", "dsregcmd"),
        ("users", "dsregcmd"),
        ("user", "dsregcmd"),
        ("login", "dsregcmd"),
        // Hardware
        ("hardware", "comp_system"),
        ("computer", "comp_system"),
        ("system_info", "comp_system"),
        ("cpu", "processor"),
        ("memory", "physical_memory"),
        ("ram", "physical_memory"),
        // Storage
        ("disk", "logical_disk"),
        ("disks", "logical_disk"),
        ("storage", "logical_disk"),
        ("partition", "disk_partition"),
        // Network
        ("network", "network_adapter"),
        ("ip", "ipconfig"),
        ("ipconfig", "ipconfig"),
        // Drivers
        ("drivers", "system_driver"),
        ("driver", "system_driver"),
        // Software
        ("programs", "installed_programs"),
        ("software", "installed_programs"),
        ("apps", "store_apps"),
        ("services", "services"),
        // System
        ("updates", "windows_update"),
        ("update", "windows_update"),
        ("startup", "startup_command"),
        ("processes", "processes"),
        ("performance", "performance"),
        ("bsod", "minidump"),
        ("crash", "minidump"),
        // Logs
        ("logs", "event_logs"),
        ("events", "event_logs"),
        ("event", "event_logs"),
    ];

    for (term, task_id) in mappings {
        if input.contains(term) && valid_ids.contains(task_id) {
            return Some(task_id.to_string());
        }
    }

    None
}

/// Maximum diagnostics to run (Phi Silica has limited context)
const MAX_DIAGNOSTICS: usize = 6;
/// Maximum characters per diagnostic output
const MAX_OUTPUT_CHARS: usize = 2000;
/// Maximum total context size
const MAX_TOTAL_CONTEXT: usize = 12000;

/// Suggest diagnostics based on keywords in the question
fn suggest_diagnostics_for_question(question: &str) -> Vec<&'static str> {
    let mut suggestions = Vec::new();

    // Computer/system info questions
    if question.contains("make")
        || question.contains("model")
        || question.contains("computer")
        || question.contains("laptop")
        || question.contains("desktop")
        || question.contains("pc")
        || question.contains("device")
        || question.contains("machine")
    {
        suggestions.extend(["comp_system", "bios", "baseboard", "os_info"]);
    }

    // CPU questions
    if question.contains("cpu")
        || question.contains("processor")
        || question.contains("core")
        || question.contains("speed")
        || question.contains("ghz")
    {
        suggestions.extend(["processor", "comp_system"]);
    }

    // Memory/RAM questions
    if question.contains("memory") || question.contains("ram") || question.contains("gb") {
        suggestions.extend(["physical_memory", "systeminfo", "comp_system"]);
    }

    // Storage questions
    if question.contains("disk")
        || question.contains("storage")
        || question.contains("drive")
        || question.contains("ssd")
        || question.contains("hdd")
        || question.contains("space")
    {
        suggestions.extend(["logical_disk", "disk_drive", "disk_partition"]);
    }

    // Network questions
    if question.contains("network")
        || question.contains("wifi")
        || question.contains("ethernet")
        || question.contains("ip")
        || question.contains("internet")
        || question.contains("adapter")
    {
        suggestions.extend(["network_adapter", "ipconfig"]);
    }

    // Security questions
    if question.contains("security")
        || question.contains("firewall")
        || question.contains("virus")
        || question.contains("protect")
        || question.contains("safe")
    {
        suggestions.extend(["firewall_status", "services", "startup_command"]);
    }

    // Performance questions
    if question.contains("performance")
        || question.contains("slow")
        || question.contains("fast")
        || question.contains("speed")
        || question.contains("optimize")
    {
        suggestions.extend(["performance", "processes", "services"]);
    }

    // OS/Windows questions
    if question.contains("windows")
        || question.contains("version")
        || question.contains("build")
        || question.contains("update")
        || question.contains("os")
    {
        suggestions.extend(["os_info", "systeminfo", "windows_update"]);
    }

    // Driver questions
    if question.contains("driver") || question.contains("hardware") {
        suggestions.extend(["system_driver", "drivers_list", "system_devices"]);
    }

    // Software questions
    if question.contains("software")
        || question.contains("program")
        || question.contains("app")
        || question.contains("install")
    {
        suggestions.extend(["installed_programs", "store_apps"]);
    }

    // Remove duplicates
    suggestions.sort();
    suggestions.dedup();
    suggestions
}

/// Tauri command to analyze system with Phi Silica using tool calling
#[tauri::command]
pub async fn analyze_with_phi_silica(prompt: String) -> Result<PhiSilicaAnalysisResponse, String> {
    // Check availability first
    let status = is_phi_silica_available();
    if !status.available {
        return Err(status.message);
    }

    // Get list of available diagnostics (compact format)
    let diagnostic_list = get_diagnostic_list_for_prompt();

    // STEP 1: Ask Phi Silica which diagnostics to run (short prompt)
    // First, check for common question patterns and suggest appropriate diagnostics
    let prompt_lower = prompt.to_lowercase();
    let suggested_ids = suggest_diagnostics_for_question(&prompt_lower);

    let planning_prompt = if !suggested_ids.is_empty() {
        format!(
            r#"User question: "{}"

Suggested diagnostics based on keywords: {:?}

Available diagnostics:
{}

Select 3-6 diagnostics. Output ONLY a JSON array. Example: ["comp_system", "os_info"]"#,
            prompt, suggested_ids, diagnostic_list
        )
    } else {
        format!(
            r#"User question: "{}"

Available diagnostics:
{}

Select 3-6 diagnostics to answer this question. Output ONLY a JSON array. Example: ["comp_system", "os_info"]"#,
            prompt, diagnostic_list
        )
    };

    log_phi_silica("Step 1: Asking Phi Silica which diagnostics to run...");
    let planning_response = generate_response(&planning_prompt).await?;
    log_phi_silica(&format!("Planning response: {}", planning_response));

    // STEP 2: Parse the response to get diagnostic IDs
    let mut diagnostic_ids = parse_diagnostic_ids(&planning_response);
    log_phi_silica(&format!("Parsed diagnostic IDs: {:?}", diagnostic_ids));

    if diagnostic_ids.is_empty() {
        // Fallback to basic diagnostics if parsing failed
        log_phi_silica("No diagnostics parsed, using defaults");
        return run_with_default_diagnostics(&prompt).await;
    }

    // Limit number of diagnostics
    if diagnostic_ids.len() > MAX_DIAGNOSTICS {
        diagnostic_ids.truncate(MAX_DIAGNOSTICS);
        log_phi_silica(&format!("Truncated to {} diagnostics", MAX_DIAGNOSTICS));
    }

    // STEP 3: Run the selected diagnostics
    log_phi_silica(&format!("Running {} diagnostics...", diagnostic_ids.len()));
    let mut diagnostic_results = String::new();
    let mut diagnostics_run = Vec::new();

    for task_id in &diagnostic_ids {
        // Check if we're approaching context limit
        if diagnostic_results.len() > MAX_TOTAL_CONTEXT {
            log_phi_silica("Context limit reached, stopping diagnostic collection");
            break;
        }

        match crate::diagnostics::run_diagnostic_task_sync(task_id) {
            Ok(result) => {
                // Smart extract key fields, then truncate if still too long
                let extracted = smart_extract(task_id, &result.output);
                let truncated = truncate_output(&extracted, MAX_OUTPUT_CHARS);
                diagnostic_results.push_str(&format!("[{}]\n{}\n\n", task_id, truncated));
                diagnostics_run.push(task_id.clone());
            }
            Err(e) => {
                log_phi_silica(&format!("Failed to run {}: {}", task_id, e));
            }
        }
    }

    log_phi_silica(&format!(
        "Ran {} diagnostics, {} chars",
        diagnostics_run.len(),
        diagnostic_results.len()
    ));

    // STEP 4: Send results back to Phi Silica for analysis (compact prompt)
    let analysis_prompt = format!(
        r#"USER'S QUESTION: {}

SYSTEM DATA:
{}

INSTRUCTIONS: Answer the user's question above using ONLY the system data provided. Do NOT make up information. Do NOT change or rephrase the question. Be direct and specific."#,
        prompt, diagnostic_results
    );

    log_phi_silica(&format!(
        "Analysis prompt size: {} chars",
        analysis_prompt.len()
    ));
    log_phi_silica("Step 4: Getting final analysis from Phi Silica...");
    let final_response = generate_response(&analysis_prompt).await?;
    log_phi_silica("Analysis complete");

    Ok(PhiSilicaAnalysisResponse {
        analysis: final_response,
        diagnostics_run,
        provider: "phi_silica".to_string(),
    })
}

/// Fallback function using default diagnostics
async fn run_with_default_diagnostics(prompt: &str) -> Result<PhiSilicaAnalysisResponse, String> {
    // Use suggested diagnostics if available, otherwise use defaults
    let prompt_lower = prompt.to_lowercase();
    let suggested = suggest_diagnostics_for_question(&prompt_lower);

    let default_diagnostics: Vec<&str> = if !suggested.is_empty() {
        suggested.into_iter().take(MAX_DIAGNOSTICS).collect()
    } else {
        vec![
            "comp_system",
            "os_info",
            "processor",
            "physical_memory",
            "logical_disk",
            "network_adapter",
        ]
    };

    let mut diagnostic_results = String::new();
    let mut diagnostics_run = Vec::new();

    for task_id in &default_diagnostics {
        if let Ok(result) = crate::diagnostics::run_diagnostic_task_sync(task_id) {
            // Smart extract key fields, then truncate if still too long
            let extracted = smart_extract(task_id, &result.output);
            let truncated = truncate_output(&extracted, MAX_OUTPUT_CHARS);
            diagnostic_results.push_str(&format!("[{}]\n{}\n\n", task_id, truncated));
            diagnostics_run.push(task_id.to_string());
        }
    }

    let analysis_prompt = format!(
        r#"USER'S QUESTION: {}

SYSTEM DATA:
{}

INSTRUCTIONS: Answer the user's question above using ONLY the system data provided. Do NOT make up information. Do NOT change or rephrase the question. Be direct and specific."#,
        prompt, diagnostic_results
    );

    let response = generate_response(&analysis_prompt).await?;

    Ok(PhiSilicaAnalysisResponse {
        analysis: response,
        diagnostics_run,
        provider: "phi_silica".to_string(),
    })
}
