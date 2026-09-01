//! Windows launcher for the About dialog's closed external-link surface.
//!
//! Every rule about *what* may be opened — the allowlist, the release-URL
//! check, the daily throttle, and the notice timings — lives in
//! [`wfdiag_native_update::policy`]. This module owns only the
//! `ShellExecuteW` call itself.

use wfdiag_native_update::UpdateInfo;
use wfdiag_native_update::policy::{AboutExternalAction, resolve_external_url};

/// Open one typed About action through the Windows shell.
///
/// The passive update path never calls this function: launching a browser is
/// possible only after an explicit button activation.
pub fn launch_external_action(
    action: AboutExternalAction,
    update: Option<&UpdateInfo>,
) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let url = resolve_external_url(action, update)?;
    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err(format!(
            "Windows could not open the link (ShellExecute code {})",
            result.0 as isize
        ))
    } else {
        Ok(())
    }
}
