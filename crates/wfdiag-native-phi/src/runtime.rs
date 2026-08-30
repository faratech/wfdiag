//! Phi Silica integration for on-device AI inference on Copilot+ PCs.
//!
//! This module provides detection and wrapper for the Microsoft.Windows.AI.Text
//! WinRT APIs to enable local AI analysis using Phi Silica.
//!
//! Note: This requires:
//! - A Copilot+ PC with NPU hardware (40+ TOPS)
//! - Windows 11 24H2/25H2 or later
//! - Registered package identity (the Microsoft Store build) with the
//!   systemAIModels capability — proven non-negotiable: even direct DLL
//!   activation returns 0x80070005 in an unpackaged process
//! - Limited Access Feature (LAF) token from Microsoft (bound to the Store
//!   package family name)
//!
//! Loose/portable builds cannot use Phi Silica; the AI service routes them
//! to Foundry Local or OpenAI instead.

use serde::{Deserialize, Serialize};

use crate::PhiError;

/// LAF constants for Phi Silica access
#[cfg(windows)]
const LAF_FEATURE_ID: &str = "com.microsoft.windows.ai.languagemodel";
/// Primary built-in token supplied by Microsoft. A Microsoft-issued token is
/// tied to a specific package family; an explicit runtime override can still
/// be supplied via the `phiSilicaLafToken` setting or the
/// `WFDIAG_LAF_TOKEN` env var.
#[cfg(windows)]
const LAF_TOKEN: &str = "ZSF3bP1v81nh6EwD4DF4QQ==";
/// Previous Microsoft-issued token. This is attempted only if the selected
/// primary token fails, to preserve access on systems where it remains valid.
#[cfg(windows)]
const LEGACY_LAF_TOKEN: &str = "edibyiYSeHx+qsGpzHNoCQ==";
/// Fallback publisher id, used only when the running package's family name is
/// unavailable. Normally derived at runtime from the package identity.
#[cfg(windows)]
const LAF_PUBLISHER_ID: &str = "t6j5qexy2jpp2";

/// The LAF token to use, plus which source it came from — env var first
/// (handy for testing an approved token without editing settings), then the
/// `phiSilicaLafToken` setting, then the primary built-in token. The source is
/// for diagnostics only (`try_unlock_laf` logs it, never the token itself) so
/// a stale settings override can be distinguished from the built-in token
/// failing. The legacy built-in token is handled separately and only retried
/// after this selected primary token fails.
#[cfg(windows)]
fn configured_laf_token() -> (String, &'static str) {
    if let Ok(token) = std::env::var("WFDIAG_LAF_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() && token != LEGACY_LAF_TOKEN {
            return (token, "env");
        }
        if token == LEGACY_LAF_TOKEN {
            log_phi_silica("WFDIAG_LAF_TOKEN contains the legacy token; reserving it for fallback");
        }
    }
    let setting_token = wfdiag_native_settings::shipping_settings_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| {
            serde_json::from_str::<wfdiag_native_settings::AppSettings>(&content).ok()
        })
        .and_then(|settings| settings.phi_silica_laf_token)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .filter(|t| t != LEGACY_LAF_TOKEN);
    if let Some(token) = setting_token {
        return (token, "setting");
    }
    (LAF_TOKEN.to_string(), "built-in-primary")
}

/// Publisher id hash from the running package's family name
/// (`<name>_<hash>`). A Microsoft-issued LAF token validates against this, so
/// it must match the actual identity rather than a hardcoded guess.
#[cfg(windows)]
fn current_publisher_id() -> String {
    use windows::ApplicationModel::Package;
    Package::Current()
        .ok()
        .and_then(|pkg| pkg.Id().ok())
        .and_then(|id| id.FamilyName().ok())
        .and_then(|family| family.to_string().rsplit('_').next().map(str::to_string))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| LAF_PUBLISHER_ID.to_string())
}

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

/// Runtime-measured prompt fit. Windows reports string lengths in UTF-16 code
/// units, so callers must not compare this value with UTF-8 bytes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhiPromptFit {
    pub input_utf16_units: u64,
    pub usable_utf16_units: u64,
    pub fits: bool,
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

/// Get the Windows update build revision (UBR) — the number after the dot in
/// a full build string (e.g. `26200.7309`). Microsoft's Phi Silica
/// requirements are sometimes stated down to this revision, but
/// `CurrentBuildNumber` alone can't distinguish an old vs. new servicing
/// update within the same major build; this is debug-log-only, not surfaced
/// in `PhiSilicaStatus` or any user-facing text.
#[cfg(windows)]
fn get_windows_ubr() -> Option<u32> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?;
    key.get_value("UBR").ok()
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

/// LanguageModel creation is comparatively expensive and the runtime is most
/// reliable with one generation at a time. The generated WinRT type is
/// Send+Sync; the mutex serializes inference and lets us invalidate a model
/// after a genuine runtime failure without recreating it for every message.
#[cfg(windows)]
static LANGUAGE_MODEL_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<crate::windows_ai_bindings::LanguageModel>>,
> = std::sync::OnceLock::new();

#[cfg(windows)]
fn cached_model_guard()
-> std::sync::MutexGuard<'static, Option<crate::windows_ai_bindings::LanguageModel>> {
    LANGUAGE_MODEL_CACHE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| {
            log_phi_silica("LanguageModel cache mutex was poisoned; recovering it");
            poisoned.into_inner()
        })
}

/// Non-blocking variant for STATUS PROBES: the generation path holds the
/// cache guard for the whole response, so a probing `.lock()` used to queue
/// behind in-flight inference for minutes. Returns None only when another
/// thread actively holds the mutex (poison is still recovered).
#[cfg(windows)]
fn try_cached_model_guard()
-> Option<std::sync::MutexGuard<'static, Option<crate::windows_ai_bindings::LanguageModel>>> {
    use std::sync::TryLockError;
    let mutex = LANGUAGE_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => {
            log_phi_silica("LanguageModel cache mutex was poisoned; recovering it");
            Some(poisoned.into_inner())
        }
        Err(TryLockError::WouldBlock) => {
            log_phi_silica("LanguageModel cache is busy (generation or model load in flight)");
            None
        }
    }
}

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

    let (primary_token, primary_source) = configured_laf_token();
    let publisher_id = current_publisher_id();
    log_phi_silica(&format!(
        "LAF unlock: token source={primary_source}, publisher id={publisher_id}"
    ));

    let feature_id = HSTRING::from(LAF_FEATURE_ID);
    let attestation = HSTRING::from(format!(
        "{} has registered their use of {} with Microsoft and agrees to the terms of use.",
        publisher_id, LAF_FEATURE_ID
    ));

    let try_token = |token_value: &str| {
        let token = HSTRING::from(token_value);
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
                    (
                        true,
                        format!("LAF unlocked successfully (status: {status_name})"),
                    )
                } else {
                    (false, format!("LAF unlock returned status: {status_name}"))
                }
            }
            Err(e) => {
                let code = e.code().0 as u32;
                (
                    false,
                    format!("LAF unlock failed: 0x{code:08X}: {}", e.message()),
                )
            }
        }
    };

    let (primary_success, primary_message) = try_token(&primary_token);
    if primary_success {
        LAF_UNLOCKED.store(true, Ordering::SeqCst);
        return (true, primary_message);
    }

    if primary_token == LEGACY_LAF_TOKEN {
        return (false, primary_message);
    }

    log_phi_silica(&format!(
        "LAF unlock using token source={primary_source} failed ({primary_message}); trying legacy built-in fallback"
    ));
    let (fallback_success, fallback_message) = try_token(LEGACY_LAF_TOKEN);
    if fallback_success {
        LAF_UNLOCKED.store(true, Ordering::SeqCst);
        return (
            true,
            format!("{fallback_message}; token source=legacy-built-in-fallback"),
        );
    }

    (
        false,
        format!(
            "primary token source={primary_source}: {primary_message}; legacy built-in fallback: {fallback_message}"
        ),
    )
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

#[cfg(all(windows, target_arch = "x86_64"))]
const DLL_ARCH: &str = "x64";
#[cfg(all(windows, target_arch = "aarch64"))]
const DLL_ARCH: &str = "arm64";
#[cfg(all(windows, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
const DLL_ARCH: &str = "unknown";

/// Install directories of the Windows App SDK framework packages this app
/// depends on. With package identity (full MSIX or the sparse package), the
/// framework's AI DLLs are already on disk here — so we never need to ship
/// our own copies. Empty when the process has no identity.
#[cfg(windows)]
fn framework_package_dirs() -> Vec<std::path::PathBuf> {
    use windows::ApplicationModel::Package;

    let mut dirs = Vec::new();
    let Ok(current) = Package::Current() else {
        return dirs;
    };
    let Ok(deps) = current.Dependencies() else {
        return dirs;
    };
    for dep in deps {
        let is_runtime = dep
            .Id()
            .and_then(|id| id.Name())
            .map(|name| name.to_string().starts_with("Microsoft.WindowsAppRuntime"))
            .unwrap_or(false);
        if is_runtime && let Ok(path) = dep.InstalledPath() {
            dirs.push(std::path::PathBuf::from(path.to_string()));
        }
    }
    dirs
}

/// Candidate directories for the Windows App SDK AI DLLs, in priority order.
///
/// The installed framework package comes FIRST: with package identity (the
/// Store build), the package graph resolves the exact framework version the
/// manifest declares, which is the supported configuration. Bundled copies
/// next to the exe are the fallback for MSIX layouts that ship their own
/// DLLs (the historically proven configuration) or machines where the
/// framework package is missing.
#[cfg(windows)]
fn dll_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = framework_package_dirs();
    if let Some(app_dir) = get_app_directory() {
        dirs.push(app_dir.clone());
        dirs.push(app_dir.join("ai-sdk-dlls").join(DLL_ARCH));
        dirs.push(app_dir.join("ai-sdk").join(DLL_ARCH));
    }
    dirs
}

/// Cached state for DLL loading
#[cfg(windows)]
static AI_TEXT_DLL_LOADED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// HMODULE wraps a raw pointer, so it's `!Send`/`!Sync` by default even
/// though the loaded-module handle itself is just an opaque, immutable
/// value once stored — safe to read from any thread after `OnceLock::set`.
#[cfg(windows)]
struct AiTextDllHandle(windows::Win32::Foundation::HMODULE);
#[cfg(windows)]
unsafe impl Send for AiTextDllHandle {}
#[cfg(windows)]
unsafe impl Sync for AiTextDllHandle {}

/// Store the loaded DLL module handle
#[cfg(windows)]
static AI_TEXT_DLL_MODULE: std::sync::OnceLock<AiTextDllHandle> = std::sync::OnceLock::new();

/// Load a DLL by bare name first (resolves from the framework package via the
/// package graph when the process has identity), then from each candidate
/// directory. Returns the module handle on the first success.
#[cfg(windows)]
fn load_ai_dll(
    dll_name: &str,
    search_dirs: &[std::path::PathBuf],
) -> Option<windows::Win32::Foundation::HMODULE> {
    use windows::Win32::System::LibraryLoader::{
        LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW, LoadLibraryW,
    };
    use windows_core::{HSTRING, PCWSTR};

    // Bare name FIRST — with package identity this resolves from the package
    // graph (framework package or the MSIX's own root), the supported path.
    let wide = HSTRING::from(dll_name);
    if let Ok(module) = unsafe { LoadLibraryW(PCWSTR::from_raw(wide.as_ptr())) } {
        log_phi_silica(&format!(
            "Loaded {} via package graph (bare name)",
            dll_name
        ));
        return Some(module);
    }

    // Explicit candidate paths as fallback (framework dirs, then bundled
    // copies next to the exe). LOAD_WITH_ALTERED_SEARCH_PATH resolves the
    // DLL's own imports from its directory.
    for dir in search_dirs {
        let dll_path = dir.join(dll_name);
        if !dll_path.exists() {
            continue;
        }
        let path_wide: Vec<u16> = dll_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        match unsafe {
            LoadLibraryExW(
                PCWSTR::from_raw(path_wide.as_ptr()),
                None,
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        } {
            Ok(module) => {
                log_phi_silica(&format!("Loaded {} from {:?}", dll_name, dll_path));
                return Some(module);
            }
            Err(e) => log_phi_silica(&format!(
                "Failed to load {} from {:?}: {}",
                dll_name,
                dll_path,
                e.message()
            )),
        }
    }
    None
}

/// Ensure the AI Text DLL is loaded — from the package graph / framework
/// package or, failing that, a bundled copy next to the exe.
#[cfg(windows)]
fn try_direct_dll_activation() -> Result<(), String> {
    use std::sync::atomic::Ordering;

    // Only load once (the flag is set only on success, so a DLL that becomes
    // available later is still picked up on a subsequent attempt)
    if AI_TEXT_DLL_LOADED.load(Ordering::SeqCst) {
        return Ok(());
    }

    let search_dirs = dll_search_dirs();
    log_phi_silica(&format!("DLL search dirs: {:?}", search_dirs));

    // Load WindowsAppRuntime first so the Text DLL's static import resolves
    let _ = load_ai_dll("Microsoft.WindowsAppRuntime.dll", &search_dirs);

    match load_ai_dll("Microsoft.Windows.AI.Text.dll", &search_dirs) {
        Some(module) => {
            // If another thread already raced us here, keep its handle — both
            // are valid loaded-module handles for the same DLL.
            let _ = AI_TEXT_DLL_MODULE.set(AiTextDllHandle(module));
            AI_TEXT_DLL_LOADED.store(true, Ordering::SeqCst);
            log_phi_silica("AI Text DLL loaded");
            Ok(())
        }
        None => Err(PhiError::ai_unavailable(
            "phi_silica",
            format!(
                "Microsoft.Windows.AI.Text.dll could not be loaded from the framework package, \
                 next to the exe, or ai-sdk-dlls\\{}",
                DLL_ARCH
            ),
        )
        .into()),
    }
}

/// Create LanguageModel via standard WinRT activation (RoGetActivationFactory).
/// This is the supported path and works whenever the process has package
/// identity — full MSIX install OR a developer-registered sparse package — with
/// the Windows App SDK framework resolvable.
#[cfg(windows)]
fn create_language_model_winrt() -> Result<crate::windows_ai_bindings::LanguageModel, String> {
    use crate::windows_ai_bindings::LanguageModel;

    log_phi_silica("Creating LanguageModel via standard WinRT activation...");
    let op = LanguageModel::CreateAsync().map_err(|e| {
        format!(
            "CreateAsync (WinRT path) failed: 0x{:08X} {}",
            e.code().0 as u32,
            e.message()
        )
    })?;
    wait_for_async_blocking(op)
}

/// Create a LanguageModel, preferring the Microsoft-documented standard
/// WinRT activation path (`LanguageModel::CreateAsync()` — every official
/// sample uses only this) and falling back to a direct
/// `DllGetActivationFactory` call if that fails.
///
/// Standard activation is the default as of 2026-08-23: a live test on a
/// real Copilot+ device (pure PowerShell, zero WFDiag code, zero LAF
/// unlock attempted) showed `LanguageModel::GetReadyState()` succeeding
/// cleanly via the standard path, which updates the older finding that
/// justified direct-DLL-first (`RoGetActivationFactory` returning
/// E_ACCESSDENIED for third-party apps even with identity — see CLAUDE.md's
/// "Audit vs. official docs" note for the full history). Both paths still
/// require registered package identity at the API level — an unpackaged
/// process gets 0x80070005 from either, which is why loose builds don't
/// route here at all.
///
/// `WFDIAG_ACTIVATION_ORDER=direct` forces the old direct-DLL-first
/// behavior, for comparison/debugging if standard activation ever
/// regresses on some device.
#[cfg(windows)]
fn create_language_model() -> Result<crate::windows_ai_bindings::LanguageModel, String> {
    let force_direct_first = std::env::var("WFDIAG_ACTIVATION_ORDER")
        .map(|v| v.eq_ignore_ascii_case("direct"))
        .unwrap_or(false);

    if force_direct_first {
        log_phi_silica(
            "WFDIAG_ACTIVATION_ORDER=direct set — trying direct DLL activation before standard WinRT",
        );
        return match create_language_model_direct() {
            Ok(model) => {
                log_phi_silica("LanguageModel created via direct DLL activation");
                Ok(model)
            }
            Err(direct_err) => {
                log_phi_silica(&format!(
                    "Direct DLL activation failed ({}); falling back to standard WinRT activation",
                    direct_err
                ));
                create_language_model_winrt().map_err(|winrt_err| {
                    format!(
                        "Phi Silica model creation failed. Direct DLL path: {} | WinRT path: {}",
                        direct_err, winrt_err
                    )
                })
            }
        };
    }

    match create_language_model_winrt() {
        Ok(model) => {
            log_phi_silica("LanguageModel created via standard WinRT activation");
            Ok(model)
        }
        Err(winrt_err) => {
            log_phi_silica(&format!(
                "Standard WinRT activation failed ({}); falling back to direct DLL activation",
                winrt_err
            ));
            create_language_model_direct().map_err(|direct_err| {
                format!(
                    "Phi Silica model creation failed. WinRT path: {} | Direct DLL path: {}",
                    winrt_err, direct_err
                )
            })
        }
    }
}

#[cfg(windows)]
fn ensure_cached_model_locked(
    cached: &mut Option<crate::windows_ai_bindings::LanguageModel>,
) -> Result<(), String> {
    if cached.is_none() {
        *cached = Some(create_language_model()?);
    }
    Ok(())
}

#[cfg(windows)]
fn invalidate_cached_model_locked(cached: &mut Option<crate::windows_ai_bindings::LanguageModel>) {
    if let Some(model) = cached.take() {
        let _ = model.Close();
        log_phi_silica("Invalidated cached LanguageModel after a runtime failure");
    }
}

#[cfg(windows)]
fn prompt_fit_for_model(
    model: &crate::windows_ai_bindings::LanguageModel,
    prompt: &str,
) -> Result<PhiPromptFit, String> {
    use windows_core::HSTRING;

    let input_utf16_units = prompt.encode_utf16().count() as u64;
    let prompt = HSTRING::from(prompt);
    let usable_utf16_units = model.GetUsablePromptLength(&prompt).map_err(|error| {
        format!(
            "Phi Silica could not measure prompt fit: 0x{:08X}: {}",
            error.code().0 as u32,
            error.message()
        )
    })?;
    Ok(PhiPromptFit {
        input_utf16_units,
        usable_utf16_units,
        fits: usable_utf16_units >= input_utf16_units,
    })
}

#[cfg(windows)]
fn ensure_feature_ready() -> Result<(), String> {
    use crate::windows_ai_bindings::{
        AIFeatureReadyResultState, AIFeatureReadyState, LanguageModel,
    };

    match LanguageModel::GetReadyState() {
        Ok(state) if state == AIFeatureReadyState::Ready => Ok(()),
        Ok(state) if state == AIFeatureReadyState::NotReady => {
            log_phi_silica("Phi Silica is not ready; starting EnsureReadyAsync");
            let operation = LanguageModel::EnsureReadyAsync().map_err(|error| {
                format!(
                    "Could not start Phi Silica preparation: 0x{:08X}: {}",
                    error.code().0 as u32,
                    error.message()
                )
            })?;
            let result = wait_for_async_with_progress_blocking_timeout(
                operation,
                std::time::Duration::from_secs(15 * 60),
                "Phi Silica preparation",
            )?;
            let status = result.Status().map_err(|error| {
                format!(
                    "Could not read Phi Silica preparation status: 0x{:08X}: {}",
                    error.code().0 as u32,
                    error.message()
                )
            })?;
            if status == AIFeatureReadyResultState::Success {
                return Ok(());
            }
            let error = result.Error().ok();
            let extended = result.ExtendedError().ok();
            let display = result
                .ErrorDisplayText()
                .ok()
                .map(|message| message.to_string())
                .filter(|message| !message.trim().is_empty());
            Err(format!(
                "Phi Silica preparation failed (status {}). error={} extended={}{}",
                status.0,
                format_hresult(error),
                format_hresult(extended),
                display
                    .map(|message| format!(" message={message}"))
                    .unwrap_or_default(),
            ))
        }
        Ok(state) if state == AIFeatureReadyState::DisabledByUser => {
            Err("Phi Silica is disabled by the Windows user setting".to_string())
        }
        Ok(state) if state == AIFeatureReadyState::NotSupportedOnCurrentSystem => Err(
            "Phi Silica is not supported on this system; a Copilot+ PC with a supported NPU is required"
                .to_string(),
        ),
        Ok(state) => Err(format!("Phi Silica returned unknown ready state {}", state.0)),
        Err(error) => {
            let code = error.code().0 as u32;
            if code == 0x80040154 || code == 0x80070005 {
                // Some third-party packaged configurations cannot resolve the
                // static factory through RoGetActivationFactory even though
                // the direct DLL activation path works. Model creation below
                // remains the authoritative readiness proof in that case.
                log_phi_silica(&format!(
                    "GetReadyState unavailable through WinRT (0x{code:08X}); deferring to direct model creation"
                ));
                Ok(())
            } else {
                Err(format!(
                    "Failed to read Phi Silica ready state: 0x{:08X}: {}",
                    code,
                    error.message()
                ))
            }
        }
    }
}

#[cfg(windows)]
fn prepare_phi_runtime() -> Result<(), String> {
    if !crate::has_package_identity() {
        return Err(PhiError::ai_unavailable(
            "phi_silica",
            "Phi Silica requires the Microsoft Store version of this app",
        )
        .into());
    }
    let build = get_windows_build().unwrap_or_default();
    if build < 26100 {
        return Err(format!(
            "Phi Silica requires Windows 11 build 26100 or later; current build is {build}"
        ));
    }
    ensure_winrt_initialized();
    init_windows_app_sdk()?;
    let (laf_ok, laf_message) = try_unlock_laf();
    if !laf_ok {
        return Err(format!("Phi Silica LAF unlock failed: {laf_message}"));
    }
    ensure_feature_ready()
}

#[cfg(windows)]
fn format_hresult(value: Option<windows_core::HRESULT>) -> String {
    value
        .map(|value| format!("0x{:08X}", value.0 as u32))
        .unwrap_or_else(|| "unavailable".to_string())
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

    let module = AI_TEXT_DLL_MODULE
        .get()
        .map(|handle| handle.0)
        .ok_or_else(|| PhiError::ai_unavailable("phi_silica", "AI Text DLL not loaded"))?;

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
            return Err(PhiError::ai_unavailable(
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
        return Err(PhiError::ai_unavailable(
            "phi_silica",
            format!("DllGetActivationFactory failed: 0x{:08X}", hr.0 as u32),
        )
        .into());
    }

    if factory_ptr.is_null() {
        return Err(PhiError::ai_unavailable(
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
            return Err(PhiError::ai_unavailable(
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
    check_phi_silica_safe_for_identity(crate::has_package_identity())
}

#[cfg(windows)]
fn check_phi_silica_safe_for_identity(
    has_package_identity: bool,
) -> (bool, String, Option<String>, Option<String>) {
    use crate::windows_ai_bindings::{AIFeatureReadyState, LanguageModel};

    log_phi_silica("=== check_phi_silica_safe called ===");

    // Without registered package identity the Windows AI APIs deny access
    // (0x80070005) on every activation path, so don't probe further — report
    // the real reason and what to do about it.
    if !has_package_identity {
        log_phi_silica("No package identity — Phi Silica unavailable");
        return (
            false,
            "Phi Silica requires the Microsoft Store version of this app (Windows AI APIs \
             need registered package identity). For local AI in this build, install \
             Foundry Local (winget install Microsoft.FoundryLocal)."
                .to_string(),
            None,
            Some("NO_PACKAGE_IDENTITY".to_string()),
        );
    }

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
    let ubr = get_windows_ubr();
    log_phi_silica(&format!(
        "Windows build: {}.{}",
        build,
        ubr.map(|u| u.to_string())
            .unwrap_or_else(|| "?".to_string())
    ));

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

    if !laf_success {
        return (
            false,
            format!("Phi Silica access is not unlocked. {laf_status_str}. Build: {build}."),
            None,
            Some(format!("LAF_REQUIRED ({laf_status_str})")),
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
                // Never block here: the generation path holds this guard for
                // the whole response, so a status probe used to wait minutes.
                // Contention itself is evidence a model exists and is busy —
                // report that instead of queuing behind it.
                let Some(mut cached) = try_cached_model_guard() else {
                    return (
                        true,
                        format!(
                            "Phi Silica is ready but currently generating; try again shortly. \
                             Build: {}",
                            build
                        ),
                        Some("Busy".to_string()),
                        None,
                    );
                };
                match ensure_cached_model_locked(&mut cached) {
                    Ok(()) => {
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
    use std::time::{Duration, Instant};
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    // Poll the async operation
    let info: IAsyncInfo = op
        .cast()
        .map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    let started = Instant::now();
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
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Async operation failed: 0x{:08X}", hr.0 as u32),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(
                    PhiError::ai_unavailable("phi_silica", "Async operation was canceled").into(),
                );
            }
            AsyncStatus::Started => {
                if started.elapsed() >= Duration::from_secs(2 * 60) {
                    let _ = info.Cancel();
                    return Err(PhiError::ai_unavailable(
                        "phi_silica",
                        "LanguageModel creation timed out after 2 minutes",
                    )
                    .into());
                }
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {:?}", status),
                )
                .into());
            }
        }
    }
}

/// Blocking wait for a normal inference operation. Model preparation uses a
/// longer explicit timeout because it may download assets.
#[cfg(windows)]
fn wait_for_async_with_progress_blocking<T, P>(
    op: windows_future::IAsyncOperationWithProgress<T, P>,
) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    wait_for_async_with_progress_blocking_timeout(
        op,
        std::time::Duration::from_secs(3 * 60),
        "Phi Silica generation",
    )
}

#[cfg(windows)]
fn wait_for_async_with_progress_blocking_timeout<T, P>(
    op: windows_future::IAsyncOperationWithProgress<T, P>,
    timeout: std::time::Duration,
    operation_name: &str,
) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    // Poll the async operation
    let info: IAsyncInfo = op
        .cast()
        .map_err(|e| format!("Failed to cast to IAsyncInfo: {}", e.message()))?;

    let started = Instant::now();
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
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Async operation failed: 0x{:08X}", hr.0 as u32),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(
                    PhiError::ai_unavailable("phi_silica", "Async operation was canceled").into(),
                );
            }
            AsyncStatus::Started => {
                if started.elapsed() >= timeout {
                    let _ = info.Cancel();
                    return Err(PhiError::ai_unavailable(
                        "phi_silica",
                        format!(
                            "{operation_name} timed out after {} seconds",
                            timeout.as_secs()
                        ),
                    )
                    .into());
                }
                sleep(Duration::from_millis(10));
            }
            _ => {
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {:?}", status),
                )
                .into());
            }
        }
    }
}

/// File logging for debugging MSIX apps, opt-in via `WFDIAG_AI_LOG=1` so
/// production runs don't write to C:\temp on every AI call.
#[cfg(windows)]
fn log_phi_silica(msg: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::OnceLock;

    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| {
        std::env::var("WFDIAG_AI_LOG")
            .map(|v| !v.trim().is_empty() && v != "0")
            .unwrap_or(false)
    });
    if !enabled {
        return;
    }

    let log_path = std::path::Path::new("C:\\temp\\phi-silica-rust.log");
    let _ = std::fs::create_dir_all("C:\\temp");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(
            file,
            "[{}] {}",
            crate::format_log_time(std::time::SystemTime::now()),
            msg
        );
    }
}

/// Ensure the Phi Silica model is ready
#[cfg(windows)]
pub async fn ensure_phi_silica_ready() -> Result<(), String> {
    use tokio::task::spawn_blocking;

    spawn_blocking(|| {
        log_phi_silica("=== ensure_phi_silica_ready called ===");
        let mut cached = cached_model_guard();
        prepare_phi_runtime()?;
        ensure_cached_model_locked(&mut cached)?;
        log_phi_silica("Phi Silica runtime and cached model are ready");
        Ok(())
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

#[cfg(not(windows))]
pub async fn ensure_phi_silica_ready() -> Result<(), String> {
    Err(PhiError::PlatformNotSupported {
        operation: "Phi Silica".to_string(),
    }
    .into())
}

/// Ask the installed Phi runtime how much of this exact prompt fits. This is
/// useful to callers assembling an evidence packet: if `fits` is false they
/// can rebuild it with fewer whole records instead of chopping off the latest
/// user question.
#[cfg(windows)]
pub async fn measure_prompt_fit(prompt: &str) -> Result<PhiPromptFit, String> {
    let prompt = prompt.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cached = cached_model_guard();
        prepare_phi_runtime()?;
        ensure_cached_model_locked(&mut cached)?;
        let result =
            prompt_fit_for_model(cached.as_ref().expect("cached model initialized"), &prompt);
        if result.is_err() {
            invalidate_cached_model_locked(&mut cached);
        }
        result
    })
    .await
    .map_err(|error| format!("Phi Silica prompt-fit task failed: {error}"))?
}

#[cfg(not(windows))]
pub async fn measure_prompt_fit(_prompt: &str) -> Result<PhiPromptFit, String> {
    Err(PhiError::PlatformNotSupported {
        operation: "Phi Silica".to_string(),
    }
    .into())
}

#[cfg(windows)]
struct GenerationFailure {
    message: String,
    invalidate_model: bool,
}

#[cfg(windows)]
impl GenerationFailure {
    fn request(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            invalidate_model: false,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            invalidate_model: true,
        }
    }
}

#[cfg(windows)]
fn generate_with_model(
    model: &crate::windows_ai_bindings::LanguageModel,
    prompt: &str,
) -> Result<String, GenerationFailure> {
    use crate::windows_ai_bindings::LanguageModelOptions;
    use windows_core::HSTRING;

    // Failure to query the model is a runtime/object health failure, not an
    // oversized user request. Invalidate the cached COM object so the next
    // turn can recreate it. Only a successful measurement with `fits=false`
    // is classified as a request error below.
    let fit = prompt_fit_for_model(model, prompt).map_err(GenerationFailure::runtime)?;
    if !fit.fits {
        return Err(GenerationFailure::request(format!(
            "Phi Silica prompt does not fit the runtime context: {} of {} UTF-16 units are usable. Rebuild the evidence packet with fewer complete records; do not truncate the current question.",
            fit.usable_utf16_units, fit.input_utf16_units
        )));
    }

    let prompt = HSTRING::from(prompt);
    // Options are activatable through the normal WinRT factory. Some systems
    // that require direct DLL activation for LanguageModel may still deny that
    // secondary factory; preserve generation with runtime defaults in that
    // case instead of turning an optional tuning feature into an outage.
    let options = LanguageModelOptions::new().ok().and_then(|options| {
        let configured = options
            .SetTemperature(0.2)
            .and_then(|_| options.SetTopP(0.9))
            .and_then(|_| options.SetTopK(20));
        match configured {
            Ok(()) => Some(options),
            Err(error) => {
                log_phi_silica(&format!(
                    "Phi Silica options are unavailable; using runtime defaults: 0x{:08X}: {}",
                    error.code().0 as u32,
                    error.message()
                ));
                None
            }
        }
    });
    if options.is_none() {
        log_phi_silica("Phi Silica LanguageModelOptions activation unavailable; using defaults");
    }
    let operation = match options {
        Some(options) => model.GenerateResponseAsync2(&prompt, &options),
        None => model.GenerateResponseAsync(&prompt),
    }
    .map_err(|error| {
        GenerationFailure::runtime(format!(
            "Failed to start Phi Silica generation: 0x{:08X}: {}",
            error.code().0 as u32,
            error.message()
        ))
    })?;
    let response =
        wait_for_async_with_progress_blocking(operation).map_err(GenerationFailure::runtime)?;
    complete_generation_response(&response)
}

#[cfg(windows)]
fn complete_generation_response(
    response: &crate::windows_ai_bindings::LanguageModelResponseResult,
) -> Result<String, GenerationFailure> {
    use crate::windows_ai_bindings::LanguageModelResponseStatus;

    let status = response.Status().map_err(|error| {
        GenerationFailure::runtime(format!(
            "Failed to read Phi Silica response status: 0x{:08X}: {}",
            error.code().0 as u32,
            error.message()
        ))
    })?;
    if status == LanguageModelResponseStatus::Complete {
        return response
            .Text()
            .map(|text| text.to_string())
            .map_err(|error| {
                GenerationFailure::runtime(format!(
                    "Failed to read Phi Silica response text: 0x{:08X}: {}",
                    error.code().0 as u32,
                    error.message()
                ))
            });
    }

    let extended = response.ExtendedError().ok();
    let detail = format!(
        "status={} extended_error={}",
        status.0,
        format_hresult(extended)
    );
    if status == LanguageModelResponseStatus::BlockedByPolicy {
        Err(GenerationFailure::request(format!(
            "Phi Silica response was blocked by policy ({detail})"
        )))
    } else if status == LanguageModelResponseStatus::PromptBlockedByContentModeration {
        Err(GenerationFailure::request(format!(
            "Phi Silica prompt was blocked by content moderation ({detail})"
        )))
    } else if status == LanguageModelResponseStatus::ResponseBlockedByContentModeration {
        Err(GenerationFailure::request(format!(
            "Phi Silica response was blocked by content moderation ({detail})"
        )))
    } else if status == LanguageModelResponseStatus::PromptLargerThanContext {
        Err(GenerationFailure::request(format!(
            "Phi Silica rejected the prompt as larger than its context ({detail})"
        )))
    } else if status == LanguageModelResponseStatus::Error {
        Err(GenerationFailure::runtime(format!(
            "Phi Silica generation failed ({detail})"
        )))
    } else if status == LanguageModelResponseStatus::InProgress {
        Err(GenerationFailure::runtime(format!(
            "Phi Silica returned InProgress after its async operation completed ({detail})"
        )))
    } else {
        Err(GenerationFailure::runtime(format!(
            "Phi Silica returned an unknown response status ({detail})"
        )))
    }
}

/// Generate a response using Phi Silica
#[cfg(windows)]
pub async fn generate_response(prompt: &str) -> Result<String, String> {
    let prompt_owned = prompt.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cached = cached_model_guard();
        prepare_phi_runtime()?;
        ensure_cached_model_locked(&mut cached)?;
        let result = generate_with_model(
            cached.as_ref().expect("cached model initialized"),
            &prompt_owned,
        );
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.invalidate_model)
        {
            invalidate_cached_model_locked(&mut cached);
        }
        result.map_err(|error| error.message)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

#[cfg(not(windows))]
pub async fn generate_response(_prompt: &str) -> Result<String, String> {
    Err(PhiError::PlatformNotSupported {
        operation: "Phi Silica".to_string(),
    }
    .into())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn unpackaged_probe_returns_before_windows_ai_activation() {
        let (available, message, ready_state, error_code) =
            check_phi_silica_safe_for_identity(false);

        assert!(!available);
        assert!(message.contains("requires the Microsoft Store version"));
        assert_eq!(ready_state, None);
        assert_eq!(error_code.as_deref(), Some("NO_PACKAGE_IDENTITY"));
    }
}
