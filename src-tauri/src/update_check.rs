//! Thin Tauri adapter over the UI-framework-neutral update service.

use wfdiag_native_update::{
    NativeUpdateRuntime, UpdateInfo, UpdateService, WindowsPackageSignatureProvider,
};

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
///
/// The check itself blocks for up to the request timeout and makes Windows
/// package calls, so it runs on `NativeUpdateRuntime`'s dedicated worker
/// thread rather than on Tauri's async command executor; this command only
/// awaits the typed reply. The runtime is dropped at the end of the command,
/// which stops and reaps that worker.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.clone();
    let service = UpdateService::shipping(current, cfg!(debug_assertions));
    let Ok(runtime) = NativeUpdateRuntime::start(service) else {
        return Ok(None);
    };
    let Ok(reply) = runtime.request_check() else {
        return Ok(None);
    };
    // Every outcome except an available release is deliberately silent here:
    // the command contract has always been "a release, or nothing".
    Ok(reply
        .await
        .ok()
        .and_then(wfdiag_native_update::UpdateOutcome::into_available))
}
