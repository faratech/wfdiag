//! Thin Tauri adapter over the UI-framework-neutral update service.

use wfdiag_native_update::{UpdateInfo, UpdateService, WindowsPackageSignatureProvider};

/// True only for a genuine Microsoft Store install. Package API failures for
/// an identified process fail closed so Store users are never shown GitHub
/// update prompts.
#[must_use]
#[allow(dead_code)] // Retained for backend callers and package-channel diagnostics.
pub fn is_store_install() -> bool {
    wfdiag_native_update::is_store_install(&WindowsPackageSignatureProvider::new())
}

/// Returns a newer GitHub release for direct-distribution builds and `None`
/// for Store/debug builds or every failure mode.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.clone();
    // `UpdateService::check` owns the same ten-second blocking HTTP contract as
    // the native worker, so keep it off Tauri's async command executor too.
    Ok(tokio::task::spawn_blocking(move || {
        UpdateService::shipping(current, cfg!(debug_assertions)).check()
    })
    .await
    .ok()
    .flatten())
}
