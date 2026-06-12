//! Self-registration of the sparse identity package.
//!
//! The Windows AI APIs (Phi Silica) gate on package identity + the
//! `systemAIModels` capability — not on full MSIX packaging. A loose exe can
//! gain identity by registering a tiny signed "package with external
//! location" MSIX whose `<Identity>` matches the `<msix>` element embedded in
//! the exe's application manifest (src-tauri/windows-app.manifest).
//!
//! This module performs that registration programmatically at startup via
//! `PackageManager.AddPackageByUriAsync` (the API behind
//! `Add-AppxPackage -ExternalLocation`), so no PowerShell step is needed.
//! Identity attaches at process creation, so after a successful registration
//! the app relaunches itself once (guarded by an env marker so a mismatch can
//! never cause a relaunch loop).
//!
//! Requirements that remain outside the app's control:
//! - The sparse .msix must sit next to the exe (`wfdiag-sparse-<arch>.msix`,
//!   produced by `build-cross.py build-sparse`).
//! - Its signing certificate must already be trusted (one-time admin step for
//!   the self-signed cert; not needed with a publicly trusted cert).

#![cfg(windows)]

use std::path::{Path, PathBuf};

#[cfg(target_arch = "x86_64")]
const ARCH: &str = "x64";
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "arm64";
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const ARCH: &str = "unknown";

/// Marker env var set on the relaunched child so registration is attempted
/// at most once per launch chain.
const RELAUNCH_MARKER: &str = "WFDIAG_IDENTITY_RELAUNCH";

/// Outcome of the startup identity check, for logging.
#[derive(Debug, PartialEq)]
pub enum IdentityOutcome {
    /// Process already has package identity (MSIX install or registered sparse package)
    AlreadyPackaged,
    /// No sparse package next to the exe — nothing to do (dev runs, plain portable use)
    NotAvailable,
    /// Registration succeeded and a relaunch was spawned; caller must exit
    RelaunchPending,
    /// Registration was attempted but failed (untrusted cert, sideloading off, …)
    Failed(String),
    /// Already relaunched once and still no identity — giving up to avoid a loop
    RelaunchDidNotAttach,
}

/// True when the current process runs with package identity.
pub fn has_package_identity() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    let mut length: u32 = 0;
    // With no buffer: ERROR_INSUFFICIENT_BUFFER means "there IS a package
    // name" (we only asked for its length); APPMODEL_ERROR_NO_PACKAGE (15700)
    // means the process is unpackaged.
    let result = unsafe { GetCurrentPackageFullName(&mut length, None) };
    result == ERROR_INSUFFICIENT_BUFFER
}

/// Locate the sparse identity package shipped next to the exe.
fn find_sparse_msix(exe_dir: &Path) -> Option<PathBuf> {
    let candidate = exe_dir.join(format!("wfdiag-sparse-{}.msix", ARCH));
    if candidate.exists() {
        return Some(candidate);
    }
    // Dev layout: build-cross.py leaves the packages in sparse-build/
    let pattern_dir = exe_dir.join("sparse-build");
    if pattern_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&pattern_dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("windowsforum_diagnostics_sparse")
                && name.ends_with(&format!("_{}.msix", ARCH))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Convert an absolute Windows path to a file:/// URI string.
fn path_to_file_uri(path: &Path) -> String {
    format!("file:///{}", path.display().to_string().replace('\\', "/"))
}

/// Register the sparse package with the exe's directory as external location.
/// Blocking (WinRT async .get()) — called once during startup, before the UI.
fn register_sparse_package(msix: &Path, external_dir: &Path) -> Result<(), String> {
    use windows::Foundation::Uri;
    use windows::Management::Deployment::{AddPackageOptions, PackageManager};
    use windows::core::HSTRING;

    let package_manager =
        PackageManager::new().map_err(|e| format!("PackageManager::new failed: {}", e))?;

    let msix_uri = Uri::CreateUri(&HSTRING::from(path_to_file_uri(msix)))
        .map_err(|e| format!("bad package uri: {}", e))?;
    let external_uri = Uri::CreateUri(&HSTRING::from(path_to_file_uri(external_dir)))
        .map_err(|e| format!("bad external location uri: {}", e))?;

    let options =
        AddPackageOptions::new().map_err(|e| format!("AddPackageOptions failed: {}", e))?;
    options
        .SetExternalLocationUri(&external_uri)
        .map_err(|e| format!("SetExternalLocationUri failed: {}", e))?;
    // Re-registering after a version bump (or a moved install dir) must work
    options
        .SetForceUpdateFromAnyVersion(true)
        .map_err(|e| format!("SetForceUpdateFromAnyVersion failed: {}", e))?;

    let operation = package_manager
        .AddPackageByUriAsync(&msix_uri, &options)
        .map_err(|e| format!("AddPackageByUriAsync failed: {}", e))?;
    let result = crate::phi_silica::wait_for_async_with_progress_blocking(operation)
        .map_err(|e| format!("deployment await failed: {}", e))?;

    let registered = result.IsRegistered().unwrap_or(false);
    if registered {
        Ok(())
    } else {
        let text = result
            .ErrorText()
            .map(|t| t.to_string())
            .unwrap_or_default();
        let code = result
            .ExtendedErrorCode()
            .map(|c| format!("0x{:08X}", c.0))
            .unwrap_or_default();
        Err(format!(
            "deployment rejected ({}): {} — if this mentions a certificate, the signing cert \
             must be installed to Trusted Root once (see Install-SparseIdentity.ps1)",
            code, text
        ))
    }
}

/// Startup hook: ensure the loose exe runs with package identity when a
/// sparse package is available. On successful registration the process
/// relaunches itself and the caller must exit immediately.
pub fn ensure_sparse_identity() -> IdentityOutcome {
    if has_package_identity() {
        return IdentityOutcome::AlreadyPackaged;
    }

    // A relaunched child without identity means the registered package does
    // not match this exe (wrong arch/publisher/path) — do not loop.
    if std::env::var(RELAUNCH_MARKER).is_ok() {
        return IdentityOutcome::RelaunchDidNotAttach;
    }

    let Ok(exe) = std::env::current_exe() else {
        return IdentityOutcome::Failed("current_exe() failed".into());
    };
    let Some(exe_dir) = exe.parent().map(Path::to_path_buf) else {
        return IdentityOutcome::Failed("exe has no parent directory".into());
    };
    let Some(msix) = find_sparse_msix(&exe_dir) else {
        return IdentityOutcome::NotAvailable;
    };

    println!(
        "Sparse identity: registering {} (external location {})",
        msix.display(),
        exe_dir.display()
    );

    if let Err(e) = register_sparse_package(&msix, &exe_dir) {
        return IdentityOutcome::Failed(e);
    }

    // Identity only attaches at process creation — relaunch once
    match std::process::Command::new(&exe)
        .env(RELAUNCH_MARKER, "1")
        .spawn()
    {
        Ok(_) => IdentityOutcome::RelaunchPending,
        Err(e) => IdentityOutcome::Failed(format!(
            "package registered but relaunch failed: {} — restart the app manually",
            e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_conversion() {
        assert_eq!(
            path_to_file_uri(Path::new("C:\\code\\wfdiag-x64.exe")),
            "file:///C:/code/wfdiag-x64.exe"
        );
    }

    #[test]
    fn sparse_msix_lookup_misses_cleanly() {
        let dir = std::env::temp_dir().join("wfdiag-sparse-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(find_sparse_msix(&dir), None);
        let _ = std::fs::remove_dir(&dir);
    }
}
