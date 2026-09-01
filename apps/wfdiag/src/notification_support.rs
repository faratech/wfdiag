//! Best-effort scan-completion toast.
//!
//! The AUMID matches the Store package identity so the packaged shell's
//! toasts are attributed correctly. An unpackaged candidate has no
//! Start-menu shortcut backing that AUMID, and Windows drops the toast
//! silently in that case — the same graceful degradation the shipping
//! plugin path shows when permission is missing. Failures are reported to
//! the caller and never block the scan-completion flow.

/// Show the scan-completion toast with the Store 2.5.8 wording.
///
/// # Errors
/// When WinRT activation or the notifier call fails (reported, never fatal).
pub fn show_scan_complete_toast(collected: usize, errors: usize) -> Result<(), String> {
    use windows::UI::Notifications::{
        ToastNotification, ToastNotificationManager, ToastTemplateType,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HSTRING;

    const AUMID: &str = "32827MikeFara.WindowsForumDiagnostics_t6j5qexy2jpp2!App";

    // WinRT activation needs an apartment, and this runs on a detached worker
    // that never pumps messages — an STA without a pump is the wrong
    // apartment and can stall activation. The MTA has no such requirement and
    // fits these fire-and-forget calls; an in-place RPC_E_CHANGED_MODE leaves
    // the existing apartment alone (`should_uninit` stays false).
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let should_uninit = hr.is_ok();

    let result = (|| {
        let xml = ToastNotificationManager::GetTemplateContent(ToastTemplateType::ToastText02)
            .map_err(|e| format!("Could not build the notification: {e}"))?;
        let text_nodes = xml
            .GetElementsByTagName(&HSTRING::from("text"))
            .map_err(|e| format!("Could not build the notification: {e}"))?;
        text_nodes
            .Item(0)
            .map_err(|e| format!("Could not build the notification: {e}"))?
            .SetInnerText(&HSTRING::from("Diagnostics complete"))
            .map_err(|e| format!("Could not build the notification: {e}"))?;
        let body: HSTRING = if errors > 0 {
            HSTRING::from(format!(
                "{collected} diagnostics collected, {errors} errors"
            ))
        } else {
            HSTRING::from(format!(
                "{collected} diagnostics collected with no collection errors"
            ))
        };
        text_nodes
            .Item(1)
            .map_err(|e| format!("Could not build the notification: {e}"))?
            .SetInnerText(&body)
            .map_err(|e| format!("Could not build the notification: {e}"))?;

        let toast = ToastNotification::CreateToastNotification(&xml)
            .map_err(|e| format!("Could not create the notification: {e}"))?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
            .map_err(|e| format!("Could not create the notifier: {e}"))?;
        notifier
            .Show(&toast)
            .map_err(|e| format!("Could not show the notification: {e}"))
    })();

    if should_uninit {
        unsafe { CoUninitialize() };
    }
    result
}
