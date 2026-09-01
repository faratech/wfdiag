//! Phi Silica integration for on-device AI inference on Copilot+ PCs.
//!
//! This module provides detection and wrapper for the Microsoft.Windows.AI.Text
//! `WinRT` APIs to enable local AI analysis using Phi Silica.
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
//! to Foundry Local or `OpenAI` instead.

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

/// Keeps the process multi-threaded apartment alive for the whole run (#191).
///
/// Every successful `RoInitialize` now has a matching `RoUninitialize` (see
/// [`WinRtApartment`]), which means the apartment would otherwise be torn down
/// the moment the last blocking-pool thread finished — taking the cached
/// `LanguageModel` and the loaded AI DLLs with it. One dedicated, named thread
/// initializes the MTA once and parks forever, so the apartment outlives every
/// worker without serializing Phi work onto a single thread (status probes
/// deliberately must not queue behind an in-flight generation).
#[cfg(windows)]
fn ensure_mta_anchor() {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    static ANCHOR: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ANCHOR.get_or_init(|| {
        let started = std::thread::Builder::new()
            .name("wfdiag-phi-mta".to_string())
            .stack_size(64 * 1024)
            .spawn(|| {
                match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                    Ok(()) => log_phi_silica("Phi MTA anchor thread holds the apartment"),
                    Err(error) => log_phi_silica(&format!(
                        "Phi MTA anchor could not initialize WinRT: 0x{:08X}: {}",
                        error.code().0.cast_unsigned(),
                        error.message()
                    )),
                }
                // The anchor exists purely to hold the apartment open; it never
                // runs Phi work and never exits (park can wake spuriously).
                loop {
                    std::thread::park();
                }
            });
        if let Err(error) = started {
            log_phi_silica(&format!(
                "Could not start the Phi MTA anchor thread: {error}"
            ));
        }
    });
}

/// RAII membership in the `WinRT` multi-threaded apartment for one thread (#191).
///
/// `RoInitialize` used to be fire-and-forget: its `HRESULT` was discarded, so a
/// thread that was already an STA (`RPC_E_CHANGED_MODE`) silently carried on as
/// if it had joined the MTA, and no call was ever balanced with
/// `RoUninitialize` — every retired `spawn_blocking` thread leaked an apartment
/// reference. COM requires one `RoUninitialize` per *successful* call,
/// including the `S_FALSE` "already initialized on this thread" case, which is
/// exactly what `owns_initialization` tracks.
#[cfg(windows)]
struct WinRtApartment {
    owns_initialization: bool,
}

#[cfg(windows)]
impl Drop for WinRtApartment {
    fn drop(&mut self) {
        use windows::Win32::System::WinRT::RoUninitialize;
        if self.owns_initialization {
            unsafe { RoUninitialize() };
        }
    }
}

/// Join the multi-threaded apartment for the lifetime of the returned guard.
///
/// Failure is tolerated rather than fatal, exactly as before: a thread that is
/// already an STA can still make these `WinRT` calls, and any other failure
/// surfaces through the specific API that needs the apartment. What changed is
/// that the outcome is inspected and logged instead of discarded, and only a
/// successful initialization is ever undone.
#[cfg(windows)]
fn enter_winrt_apartment() -> WinRtApartment {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    ensure_mta_anchor();
    // S_OK (initialized by us) and S_FALSE (this thread was already in the
    // apartment) both come back as `Ok` from the binding and both take an
    // apartment reference, so both are released on drop.
    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => WinRtApartment {
            owns_initialization: true,
        },
        Err(error) if error.code() == windows_core::HRESULT(RPC_E_CHANGED_MODE) => {
            log_phi_silica(
                "Thread is already a single-threaded apartment (RPC_E_CHANGED_MODE); \
                 continuing without changing it",
            );
            WinRtApartment {
                owns_initialization: false,
            }
        }
        Err(error) => {
            log_phi_silica(&format!(
                "RoInitialize failed: 0x{:08X}: {}",
                error.code().0.cast_unsigned(),
                error.message()
            ));
            WinRtApartment {
                owns_initialization: false,
            }
        }
    }
}

/// `RPC_E_CHANGED_MODE`: the calling thread already belongs to a single-threaded
/// apartment, so the requested concurrency model was refused.
#[cfg(any(windows, test))]
const RPC_E_CHANGED_MODE: i32 = 0x8001_0106_u32.cast_signed();

/// Track if bootstrapper has been initialized
#[cfg(windows)]
static BOOTSTRAPPER_INITIALIZED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Track if LAF has been unlocked
#[cfg(windows)]
static LAF_UNLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `LanguageModel` creation is comparatively expensive and the runtime is most
/// reliable with one generation at a time. The generated `WinRT` type is
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
/// Returns `(success, status_message)`
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
        "{publisher_id} has registered their use of {LAF_FEATURE_ID} with Microsoft and agrees to the terms of use."
    ));

    let try_token = |token_value: &str| {
        let token = HSTRING::from(token_value);
        match LimitedAccessFeatures::TryUnlockFeature(&feature_id, &token, &attestation) {
            Ok(result) => {
                let status = result
                    .Status()
                    .unwrap_or(LimitedAccessFeatureStatus::Unknown);
                // The catch-all covers `LimitedAccessFeatureStatus::Unknown`
                // itself as well as any value a future Windows adds; both
                // report "Unknown", exactly as before.
                let status_name = match status {
                    LimitedAccessFeatureStatus::Available => "Available",
                    LimitedAccessFeatureStatus::AvailableWithoutToken => "AvailableWithoutToken",
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
                let code = e.code().0.cast_unsigned();
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

/// `MddBootstrapInitialize2` function signature.
#[cfg(windows)]
type MddBootstrapInitialize2Fn = unsafe extern "system" fn(
    major_minor_version: u32,
    version_tag: windows::core::PCWSTR,
    min_version: u64,
    options: u32,
) -> windows::core::HRESULT;

/// Initialize Windows App SDK bootstrapper for AI APIs access
/// This is required for unpackaged apps to access Windows App SDK features
#[cfg(windows)]
// Every path is deliberately tolerant today, but bootstrapper initialization is
// a fallible contract and the `?` call sites must stay ready for a real failure.
#[allow(clippy::unnecessary_wraps)]
fn init_windows_app_sdk() -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::PCWSTR;

    // Only initialize once
    if BOOTSTRAPPER_INITIALIZED.load(Ordering::SeqCst) {
        return Ok(());
    }

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
            (0x0001_0008, "1.8"),
            (0x0001_0007, "1.7"),
            (0x0001_0006, "1.6"),
        ];

        for (major_minor, _version_name) in versions {
            let hr = init(major_minor, PCWSTR::null(), 0, 0);
            if hr.is_ok() {
                BOOTSTRAPPER_INITIALIZED.store(true, Ordering::SeqCst);
                return Ok(());
            }
            // 0x80070032 = ERROR_NOT_SUPPORTED (packaged app, bootstrapper not needed)
            // 0x80040154 = CLASS_E_CLASSNOTREGISTERED
            let code = hr.0.cast_unsigned();
            if code == 0x8007_0032 {
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
        .map(std::path::Path::to_path_buf)
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
            .is_ok_and(|name| name.to_string().starts_with("Microsoft.WindowsAppRuntime"));
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
// `{path:?}` is the existing on-disk debug-log format for these lines; keep it.
#[allow(clippy::unnecessary_debug_formatting)]
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
        log_phi_silica(&format!("Loaded {dll_name} via package graph (bare name)"));
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
                log_phi_silica(&format!("Loaded {dll_name} from {dll_path:?}"));
                return Some(module);
            }
            Err(e) => log_phi_silica(&format!(
                "Failed to load {dll_name} from {dll_path:?}: {}",
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
    log_phi_silica(&format!("DLL search dirs: {search_dirs:?}"));

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
                 next to the exe, or ai-sdk-dlls\\{DLL_ARCH}"
            ),
        )
        .into()),
    }
}

/// Create `LanguageModel` via standard `WinRT` activation (`RoGetActivationFactory`).
/// This is the supported path and works whenever the process has package
/// identity — full MSIX install OR a developer-registered sparse package — with
/// the Windows App SDK framework resolvable.
#[cfg(windows)]
fn create_language_model_winrt(
    is_cancelled: &dyn Fn() -> bool,
) -> Result<crate::windows_ai_bindings::LanguageModel, String> {
    use crate::windows_ai_bindings::LanguageModel;

    log_phi_silica("Creating LanguageModel via standard WinRT activation...");
    let op = LanguageModel::CreateAsync().map_err(|e| {
        format!(
            "CreateAsync (WinRT path) failed: 0x{:08X} {}",
            e.code().0.cast_unsigned(),
            e.message()
        )
    })?;
    wait_for_async_blocking(op, is_cancelled)
}

/// Create a `LanguageModel`, preferring the Microsoft-documented standard
/// `WinRT` activation path (`LanguageModel::CreateAsync()` — every official
/// sample uses only this) and falling back to a direct
/// `DllGetActivationFactory` call if that fails.
///
/// Standard activation is the default as of 2026-08-23: a live test on a
/// real Copilot+ device (pure PowerShell, zero `WFDiag` code, zero LAF
/// unlock attempted) showed `LanguageModel::GetReadyState()` succeeding
/// cleanly via the standard path, which updates the older finding that
/// justified direct-DLL-first (`RoGetActivationFactory` returning
/// `E_ACCESSDENIED` for third-party apps even with identity — see CLAUDE.md's
/// "Audit vs. official docs" note for the full history). Both paths still
/// require registered package identity at the API level — an unpackaged
/// process gets 0x80070005 from either, which is why loose builds don't
/// route here at all.
///
/// `WFDIAG_ACTIVATION_ORDER=direct` forces the old direct-DLL-first
/// behavior, for comparison/debugging if standard activation ever
/// regresses on some device.
#[cfg(windows)]
fn create_language_model(
    is_cancelled: &dyn Fn() -> bool,
) -> Result<crate::windows_ai_bindings::LanguageModel, String> {
    let force_direct_first =
        std::env::var("WFDIAG_ACTIVATION_ORDER").is_ok_and(|v| v.eq_ignore_ascii_case("direct"));

    if force_direct_first {
        log_phi_silica(
            "WFDIAG_ACTIVATION_ORDER=direct set — trying direct DLL activation before standard WinRT",
        );
        return match create_language_model_direct(is_cancelled) {
            Ok(model) => {
                log_phi_silica("LanguageModel created via direct DLL activation");
                Ok(model)
            }
            Err(direct_err) => {
                log_phi_silica(&format!(
                    "Direct DLL activation failed ({direct_err}); falling back to standard WinRT activation"
                ));
                create_language_model_winrt(is_cancelled).map_err(|winrt_err| {
                    format!(
                        "Phi Silica model creation failed. Direct DLL path: {direct_err} | WinRT path: {winrt_err}"
                    )
                })
            }
        };
    }

    match create_language_model_winrt(is_cancelled) {
        Ok(model) => {
            log_phi_silica("LanguageModel created via standard WinRT activation");
            Ok(model)
        }
        Err(winrt_err) => {
            log_phi_silica(&format!(
                "Standard WinRT activation failed ({winrt_err}); falling back to direct DLL activation"
            ));
            create_language_model_direct(is_cancelled).map_err(|direct_err| {
                format!(
                    "Phi Silica model creation failed. WinRT path: {winrt_err} | Direct DLL path: {direct_err}"
                )
            })
        }
    }
}

#[cfg(windows)]
fn ensure_cached_model_locked(
    cached: &mut Option<crate::windows_ai_bindings::LanguageModel>,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    if cached.is_none() {
        *cached = Some(create_language_model(is_cancelled)?);
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
            error.code().0.cast_unsigned(),
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
fn ensure_feature_ready(is_cancelled: &dyn Fn() -> bool) -> Result<(), String> {
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
                    error.code().0.cast_unsigned(),
                    error.message()
                )
            })?;
            // #205: a 15-minute preparation is exactly the wait a user is most
            // likely to abandon; honour their cancellation instead of pinning
            // the model mutex for the whole budget.
            let result = wait_for_async_with_progress_blocking_timeout(
                operation,
                std::time::Duration::from_mins(15),
                "Phi Silica preparation",
                is_cancelled,
            )?;
            let status = result.Status().map_err(|error| {
                format!(
                    "Could not read Phi Silica preparation status: 0x{:08X}: {}",
                    error.code().0.cast_unsigned(),
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
            let code = error.code().0.cast_unsigned();
            if code == 0x8004_0154 || code == 0x8007_0005 {
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

/// Prepare the runtime and hand back the apartment membership it established.
///
/// The guard MUST be held by the caller for as long as it keeps calling `WinRT`
/// on this thread (#191) — dropping it early leaves the thread outside the
/// apartment it just joined.
#[cfg(windows)]
fn prepare_phi_runtime(is_cancelled: &dyn Fn() -> bool) -> Result<WinRtApartment, String> {
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
    let apartment = enter_winrt_apartment();
    init_windows_app_sdk()?;
    let (laf_ok, laf_message) = try_unlock_laf();
    if !laf_ok {
        return Err(format!("Phi Silica LAF unlock failed: {laf_message}"));
    }
    ensure_feature_ready(is_cancelled)?;
    Ok(apartment)
}

#[cfg(windows)]
fn format_hresult(value: Option<windows_core::HRESULT>) -> String {
    value.map_or_else(
        || "unavailable".to_string(),
        |value| format!("0x{:08X}", value.0.cast_unsigned()),
    )
}

/// `DllGetActivationFactory` signature:
/// `HRESULT DllGetActivationFactory(HSTRING classId, IActivationFactory** factory)`
#[cfg(windows)]
type DllGetActivationFactoryFn = unsafe extern "system" fn(
    class_id: *mut std::ffi::c_void, // HSTRING (passed by value, it's a pointer)
    factory: *mut *mut std::ffi::c_void, // IActivationFactory**
) -> windows_core::HRESULT;

/// Create `LanguageModel` using `DllGetActivationFactory` from bundled DLL
/// This bypasses `RoGetActivationFactory` entirely, like `CsWinRT` does
#[cfg(windows)]
fn create_language_model_direct(
    is_cancelled: &dyn Fn() -> bool,
) -> Result<crate::windows_ai_bindings::LanguageModel, String> {
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
        "Calling DllGetActivationFactory with class: {class_name}"
    ));

    let hr = unsafe { get_factory(hstring_raw, &raw mut factory_ptr) };

    if hr.is_err() {
        log_phi_silica(&format!(
            "DllGetActivationFactory failed: 0x{:08X}",
            hr.0.cast_unsigned()
        ));
        return Err(PhiError::ai_unavailable(
            "phi_silica",
            format!(
                "DllGetActivationFactory failed: 0x{:08X}",
                hr.0.cast_unsigned()
            ),
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
        let hr = ((**vtable).CreateAsync)(statics.as_raw(), &raw mut result);
        if hr.is_err() {
            log_phi_silica(&format!(
                "CreateAsync call failed: 0x{:08X}",
                hr.0.cast_unsigned()
            ));
            return Err(PhiError::ai_unavailable(
                "phi_silica",
                format!("CreateAsync failed: 0x{:08X}", hr.0.cast_unsigned()),
            )
            .into());
        }
        // S_OK with a null out-parameter would become a null vtable
        // dereference on the first cast (#190).
        if result.is_null() {
            log_phi_silica("CreateAsync returned S_OK with a null async operation");
            return Err(PhiError::ai_unavailable(
                "phi_silica",
                "CreateAsync returned a null async operation".to_string(),
            )
            .into());
        }
        windows_future::IAsyncOperation::<LanguageModel>::from_raw(result)
    };

    log_phi_silica("CreateAsync started, waiting...");

    // Wait for async operation
    let model = wait_for_async_blocking(async_op, is_cancelled)?;

    log_phi_silica("LanguageModel created successfully via direct activation!");

    Ok(model)
}

/// Check Phi Silica availability using `GetReadyState` (like AI Dev Gallery does)
#[cfg(windows)]
// Returns (available, message, ready_state, error_code). ready_state carries the
// AIFeatureReadyState on the success path; error_code carries the HRESULT/LAF string on
// the failure path. They are kept SEPARATE so the frontend's PhiSilicaStatus.error_code
// is populated correctly instead of error info being mislabeled into ready_state.
fn check_phi_silica_safe() -> (bool, String, Option<String>, Option<String>) {
    check_phi_silica_safe_for_identity(crate::has_package_identity())
}

#[cfg(windows)]
// One linear LAF/activation state machine; splitting it would scatter the
// ordering that CLAUDE.md pins down.
#[allow(clippy::too_many_lines)]
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

    // Join the apartment for the rest of this probe; the guard leaves it again
    // when the function returns (#191).
    let _apartment = enter_winrt_apartment();
    log_phi_silica("WinRT initialized");

    // Try to initialize Windows App SDK bootstrapper (may fail for packaged apps, that's OK)
    let _ = init_windows_app_sdk();
    log_phi_silica("Windows App SDK init attempted");

    // Try to pre-load the AI DLL directly to ensure it's available for activation
    match try_direct_dll_activation() {
        Ok(()) => log_phi_silica("Direct DLL activation succeeded"),
        Err(e) => log_phi_silica(&format!(
            "Direct DLL activation failed (continuing anyway): {e}"
        )),
    }

    let build = get_windows_build().unwrap_or(0);
    let ubr = get_windows_ubr();
    let ubr_text = ubr.map_or_else(|| "?".to_string(), |ubr| ubr.to_string());
    log_phi_silica(&format!("Windows build: {build}.{ubr_text}"));

    // Try to unlock Limited Access Feature BEFORE accessing Phi Silica APIs
    let (laf_success, laf_message) = try_unlock_laf();
    log_phi_silica(&format!(
        "LAF unlock: success={laf_success}, msg={laf_message}"
    ));

    // Store LAF status for error reporting
    let laf_status_str = if laf_success {
        format!("LAF: OK ({laf_message})")
    } else {
        format!("LAF: FAILED ({laf_message})")
    };

    // Phi Silica requires Windows 11 24H2 (build 26100+) with a Copilot+ PC
    if build < 26100 {
        log_phi_silica("Build too old, returning");
        return (
            false,
            format!(
                "Phi Silica requires Windows 11 24H2 or later (build 26100+). Current build: {build}"
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
                    format!("Phi Silica is ready. Build: {build}"),
                    Some("Ready".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::NotReady {
                // Model needs to be downloaded/initialized
                (
                    true,
                    format!("Phi Silica available but not ready. Build: {build}"),
                    Some("NotReady".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::DisabledByUser {
                (
                    false,
                    format!("Phi Silica disabled by user. Build: {build}"),
                    Some("DisabledByUser".to_string()),
                    None,
                )
            } else if state == AIFeatureReadyState::NotSupportedOnCurrentSystem {
                (
                    false,
                    format!(
                        "Phi Silica not supported on this system (requires Copilot+ PC with NPU). Build: {build}"
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
            let code = e.code().0.cast_unsigned();
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
            if code == 0x8004_0154 || code == 0x8007_0005 {
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
                             Build: {build}"
                        ),
                        Some("Busy".to_string()),
                        None,
                    );
                };
                match ensure_cached_model_locked(&mut cached, &|| false) {
                    Ok(()) => {
                        log_phi_silica("Direct DLL activation succeeded — Phi Silica IS available");
                        return (
                            true,
                            format!(
                                "Phi Silica is ready (via direct DLL activation). Build: {build}"
                            ),
                            Some("Ready".to_string()),
                            None,
                        );
                    }
                    Err(direct_err) => {
                        log_phi_silica(&format!("Direct DLL activation also failed: {direct_err}"));
                    }
                }
            }

            if code == 0x8004_0154 {
                // CLASS_E_CLASSNOTREGISTERED - API not available
                // This happens when the Windows AI runtime is not present
                (
                    false,
                    format!(
                        "Phi Silica API not registered (0x{code:08X}). Build: {build}. \
                     Requires Copilot+ PC with Windows AI features enabled."
                    ),
                    None,
                    Some(format!("0x{code:08X}")),
                )
            } else if code == 0x8007_0005 {
                // E_ACCESSDENIED - LAF unlock may have failed
                (
                    false,
                    format!(
                        "Phi Silica access denied (0x80070005). {laf_status_str}. Build: {build}."
                    ),
                    None,
                    Some(format!("LAF_REQUIRED ({laf_status_str})")),
                )
            } else {
                (
                    false,
                    format!(
                        "Failed to check Phi Silica: 0x{code:08X}: {}. {laf_status_str}. Build: {build}",
                        e.message()
                    ),
                    None,
                    Some(format!("0x{code:08X}")),
                )
            }
        }
    }
}

/// Check if Phi Silica is available on this device
#[cfg(windows)]
#[must_use]
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
#[must_use]
pub fn is_phi_silica_available() -> PhiSilicaStatus {
    PhiSilicaStatus {
        available: false,
        message: "Phi Silica is only available on Windows".to_string(),
        error_code: None,
        windows_build: None,
        ready_state: None,
    }
}

/// How long to sleep between polls of a `WinRT` async operation (#205).
///
/// The bindings expose only `IAsyncInfo::Status`, so these waits poll rather
/// than wait on a completion handler; a flat 10 ms tick spent ~6,000 wakeups a
/// minute on operations that routinely run for minutes. Stay at 10 ms while a
/// fast completion is still plausible, then back off — 50 ms is still far
/// finer-grained than any user-visible deadline, including cancellation.
#[cfg(any(windows, test))]
fn async_poll_interval(elapsed: std::time::Duration) -> std::time::Duration {
    if elapsed < std::time::Duration::from_millis(500) {
        std::time::Duration::from_millis(10)
    } else {
        std::time::Duration::from_millis(50)
    }
}

/// Blocking wait for an async operation - runs in `spawn_blocking` to be Send-safe
///
/// `is_cancelled` is honoured for the same reason the generation wait honours
/// it (#205): `LanguageModel` creation can occupy the process-wide model mutex
/// for up to two minutes, so an abandoned turn must be able to cancel the `WinRT`
/// operation and release the lock instead of idling out the full budget.
#[cfg(windows)]
// Takes the WinRT operation by value on purpose: the wait owns it, releases it
// when it returns, and no caller can poll a completed operation again.
#[allow(clippy::needless_pass_by_value)]
fn wait_for_async_blocking<T>(
    op: windows_future::IAsyncOperation<T>,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<T, String>
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
                    format!("Async operation failed: 0x{:08X}", hr.0.cast_unsigned()),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(
                    PhiError::ai_unavailable("phi_silica", "Async operation was canceled").into(),
                );
            }
            AsyncStatus::Started => {
                let elapsed = started.elapsed();
                if is_cancelled() {
                    let _ = info.Cancel();
                    return Err(PhiError::ai_unavailable(
                        "phi_silica",
                        "Phi Silica model creation was cancelled",
                    )
                    .into());
                }
                if elapsed >= Duration::from_mins(2) {
                    let _ = info.Cancel();
                    return Err(PhiError::ai_unavailable(
                        "phi_silica",
                        "LanguageModel creation timed out after 2 minutes",
                    )
                    .into());
                }
                sleep(async_poll_interval(elapsed));
            }
            _ => {
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {status:?}"),
                )
                .into());
            }
        }
    }
}

/// Blocking wait for a normal inference operation. Model preparation uses a
/// longer explicit timeout because it may download assets.
///
/// The 150s budget MUST stay strictly below the chat engine's
/// `TURN_TIMEOUT_SECS` (180s): the engine's outer timeout only drops the
/// stream future, while this `spawn_blocking` call keeps running and holds the
/// process-wide model mutex. Bounding inference below the outer deadline means
/// the lock is always released before a superseding turn can starve behind it.
/// `is_cancelled` additionally lets an abandoned turn release the mutex well
/// before that 150s ceiling: the engine's `select!` already returns
/// "Cancelled" to the UI the instant its token fires and drops its handle to
/// this future, but the `spawn_blocking` closure keeps running underneath
/// regardless — polling `is_cancelled()` here (a plain closure, not
/// `tokio_util::CancellationToken`, so this crate doesn't need that
/// dependency just to check a bool) is what actually stops it and calls
/// `IAsyncInfo::Cancel()` on the `WinRT` operation instead of idling out.
#[cfg(windows)]
fn wait_for_async_with_progress_blocking<T, P>(
    op: windows_future::IAsyncOperationWithProgress<T, P>,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    wait_for_async_with_progress_blocking_timeout(
        op,
        std::time::Duration::from_secs(150),
        "Phi Silica generation",
        is_cancelled,
    )
}

#[cfg(windows)]
// Same ownership contract as `wait_for_async_blocking`: the wait consumes the
// WinRT operation and releases it on return.
#[allow(clippy::needless_pass_by_value)]
fn wait_for_async_with_progress_blocking_timeout<T, P>(
    op: windows_future::IAsyncOperationWithProgress<T, P>,
    timeout: std::time::Duration,
    operation_name: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<T, String>
where
    T: windows_core::RuntimeType,
    P: windows_core::RuntimeType,
{
    use std::thread::sleep;
    use std::time::Instant;
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
                    format!("Async operation failed: 0x{:08X}", hr.0.cast_unsigned()),
                )
                .into());
            }
            AsyncStatus::Canceled => {
                return Err(
                    PhiError::ai_unavailable("phi_silica", "Async operation was canceled").into(),
                );
            }
            AsyncStatus::Started => {
                let elapsed = started.elapsed();
                if is_cancelled() {
                    let _ = info.Cancel();
                    return Err(PhiError::ai_unavailable(
                        "phi_silica",
                        format!("{operation_name} was cancelled"),
                    )
                    .into());
                }
                if elapsed >= timeout {
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
                sleep(async_poll_interval(elapsed));
            }
            _ => {
                return Err(PhiError::ai_unavailable(
                    "phi_silica",
                    format!("Unknown async status: {status:?}"),
                )
                .into());
            }
        }
    }
}

/// The opt-in debug log's location under a local-app-data root (#217).
///
/// It used to be a hardcoded `C:\temp\phi-silica-rust.log`: a world-writable
/// directory outside the app's own storage, which any user on the machine can
/// pre-create or plant a link in. This follows the same
/// `%LOCALAPPDATA%\WFDiag` convention as the credential store in
/// `wfdiag-native-settings`.
#[cfg(any(windows, test))]
fn phi_log_path_in(local_data_dir: &std::path::Path) -> std::path::PathBuf {
    local_data_dir
        .join("WFDiag")
        .join("logs")
        .join("phi-silica.log")
}

/// True when this metadata describes a file with more than one hard link.
/// Only the Unix test build can ask: `number_of_links` is still unstable in
/// `std::os::windows::fs::MetadataExt`, so the Windows build relies on the
/// symlink/reparse refusal plus per-user directory ownership.
#[cfg(all(unix, test))]
fn is_multiply_linked(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

/// Open the debug log for appending without ever following a link (#217).
///
/// An existing entry is used only when it is a plain file: `symlink_metadata`
/// does not traverse, so a symlink or reparse point planted at the path is
/// refused rather than redirecting the appended text somewhere else. When
/// nothing is there, `create_new` makes creation itself the atomic check, so a
/// link that appears between the probe and the open cannot win the race.
#[cfg(any(windows, test))]
fn open_log_file(path: &std::path::Path) -> Option<std::fs::File> {
    use std::fs::OpenOptions;

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return None;
            }
            #[cfg(all(unix, test))]
            if is_multiply_linked(&metadata) {
                return None;
            }
            OpenOptions::new().append(true).open(path).ok()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)
            .ok(),
        Err(_) => None,
    }
}

/// File logging for debugging MSIX apps, opt-in via `WFDIAG_AI_LOG=1` so
/// production runs don't write to disk on every AI call.
#[cfg(windows)]
fn log_phi_silica(msg: &str) {
    use std::io::Write;
    use std::sync::OnceLock;

    static LOG_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let path = LOG_PATH.get_or_init(|| {
        let enabled = std::env::var("WFDIAG_AI_LOG")
            .is_ok_and(|value| !value.trim().is_empty() && value != "0");
        if !enabled {
            return None;
        }
        let path = phi_log_path_in(&dirs::data_local_dir()?);
        // Ordinary directory permissions: this lives under the user's own
        // local app data, not a shared root (#217).
        std::fs::create_dir_all(path.parent()?).ok()?;
        Some(path)
    });
    let Some(path) = path.as_deref() else {
        return;
    };

    if let Some(mut file) = open_log_file(path) {
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
        // The guard keeps this thread in the WinRT apartment until the closure
        // returns (#191).
        let _apartment = prepare_phi_runtime(&|| false)?;
        ensure_cached_model_locked(&mut cached, &|| false)?;
        log_phi_silica("Phi Silica runtime and cached model are ready");
        Ok(())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
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
///
/// # Panics
///
/// Panics if the cached model is absent immediately after
/// `ensure_cached_model_locked` reported success while this thread still holds
/// the cache guard — an invariant violation rather than a runtime condition.
#[cfg(windows)]
pub async fn measure_prompt_fit(prompt: &str) -> Result<PhiPromptFit, String> {
    let prompt = prompt.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cached = cached_model_guard();
        let _apartment = prepare_phi_runtime(&|| false)?;
        ensure_cached_model_locked(&mut cached, &|| false)?;
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
    is_cancelled: &dyn Fn() -> bool,
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
            .and_then(|()| options.SetTopP(0.9))
            .and_then(|()| options.SetTopK(20));
        match configured {
            Ok(()) => Some(options),
            Err(error) => {
                log_phi_silica(&format!(
                    "Phi Silica options are unavailable; using runtime defaults: 0x{:08X}: {}",
                    error.code().0.cast_unsigned(),
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
            error.code().0.cast_unsigned(),
            error.message()
        ))
    })?;
    let response = wait_for_async_with_progress_blocking(operation, is_cancelled)
        .map_err(GenerationFailure::runtime)?;
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
            error.code().0.cast_unsigned(),
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
                    error.code().0.cast_unsigned(),
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

/// Generate a response using Phi Silica. `is_cancelled` lets an abandoned
/// turn release the process-wide model mutex early instead of idling out the
/// full 150s generation budget — pass `|| false` when no external
/// cancellation applies (e.g. the report/analysis one-shot paths). Deliberately
/// a plain closure rather than `tokio_util::sync::CancellationToken`: the
/// check only needs to be a cheap, thread-safe bool read from the blocking
/// pool, and callers that already hold a `CancellationToken` can pass
/// `move || token.is_cancelled()` without this crate taking on that
/// dependency itself.
#[cfg(windows)]
pub async fn generate_response(
    prompt: &str,
    is_cancelled: impl Fn() -> bool + Send + 'static,
) -> Result<String, String> {
    let prompt_owned = prompt.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cached = cached_model_guard();
        // #205: model preparation and creation can take minutes on a cold
        // runtime, so the turn's cancellation reaches them too — not just the
        // generation call below.
        let _apartment = prepare_phi_runtime(&is_cancelled)?;
        ensure_cached_model_locked(&mut cached, &is_cancelled)?;
        let Some(model) = cached.as_ref() else {
            return Err("Phi Silica model was unavailable after preparation".to_string());
        };
        let result = generate_with_model(model, &prompt_owned, &is_cancelled);
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
    .map_err(|e| format!("Task join error: {e}"))?
}

#[cfg(not(windows))]
pub async fn generate_response(
    _prompt: &str,
    _is_cancelled: impl Fn() -> bool + Send + 'static,
) -> Result<String, String> {
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

/// Platform-neutral tests for the pure helpers behind the `WinRT` waits, the
/// apartment bookkeeping, and the debug log location. These run on the Linux
/// engine build as well as on Windows.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn test_directory(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "wfdiag_phi_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("create test directory");
        directory
    }

    #[test]
    fn poll_interval_backs_off_after_the_first_moments() {
        // #205: responsive while a fast completion is plausible, then cheap.
        assert_eq!(
            async_poll_interval(Duration::ZERO),
            Duration::from_millis(10)
        );
        assert_eq!(
            async_poll_interval(Duration::from_millis(499)),
            Duration::from_millis(10)
        );
        assert_eq!(
            async_poll_interval(Duration::from_millis(500)),
            Duration::from_millis(50)
        );
        assert_eq!(
            async_poll_interval(Duration::from_secs(90)),
            Duration::from_millis(50)
        );
        // A cancellation check must never wait longer than a UI frame budget.
        assert!(async_poll_interval(Duration::from_secs(600)) <= Duration::from_millis(50));
    }

    #[test]
    fn changed_mode_is_the_documented_sta_hresult() {
        // #191: the value the apartment guard tolerates instead of treating a
        // pre-existing STA thread as a hard failure.
        assert_eq!(RPC_E_CHANGED_MODE.cast_unsigned(), 0x8001_0106);
    }

    #[test]
    fn debug_log_lives_under_the_app_local_data_directory() {
        // #217: no more hardcoded C:\temp.
        let path = phi_log_path_in(Path::new("C:/Users/x/AppData/Local"));
        assert_eq!(
            path,
            Path::new("C:/Users/x/AppData/Local")
                .join("WFDiag")
                .join("logs")
                .join("phi-silica.log")
        );
        assert!(!path.starts_with("C:/temp"));
    }

    #[test]
    fn debug_log_appends_to_a_plain_file_and_creates_it_when_absent() {
        let directory = test_directory("log_plain");
        let path = directory.join("phi-silica.log");

        let mut file = open_log_file(&path).expect("create the log");
        std::io::Write::write_all(&mut file, b"first\n").expect("write");
        drop(file);
        let mut file = open_log_file(&path).expect("reopen the log");
        std::io::Write::write_all(&mut file, b"second\n").expect("append");
        drop(file);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[cfg(unix)]
    #[test]
    fn debug_log_refuses_a_planted_symlink_or_hard_link() {
        // #217: appending through a link planted at the log path would write
        // into a file of someone else's choosing.
        let directory = test_directory("log_links");
        let victim = directory.join("victim.txt");
        std::fs::write(&victim, b"untouched").expect("write victim");

        let symlinked = directory.join("symlinked.log");
        std::os::unix::fs::symlink(&victim, &symlinked).expect("symlink");
        assert!(open_log_file(&symlinked).is_none());

        let hard_linked = directory.join("hardlinked.log");
        std::fs::hard_link(&victim, &hard_linked).expect("hard link");
        assert!(open_log_file(&hard_linked).is_none());

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
        std::fs::remove_dir_all(&directory).ok();
    }
}
