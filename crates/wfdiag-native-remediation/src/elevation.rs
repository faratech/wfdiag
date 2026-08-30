//! Administrator relaunch shared by both shells.
//!
//! Moved verbatim from the shipping backend's `relaunch_self_elevated`:
//! ShellExecuteEx with the `runas` verb, COM apartment handling, and the
//! deliberate distinction between a dismissed UAC prompt (Ok(false)) and a
//! real failure.

/// Relaunch the running executable with administrator rights.
///
/// Returns `Ok(true)` when the elevated copy launched, `Ok(false)` when the
/// user dismissed the UAC prompt, and `Err` on a real failure.
#[cfg(windows)]
pub fn relaunch_self_elevated() -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_CANCELLED};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HRESULT, PCWSTR};

    let exe =
        std::env::current_exe().map_err(|e| format!("Could not resolve the running program path: {e}"))?;
    // Run the elevated copy from its own directory rather than inheriting this
    // process's working directory, so it can't pick up a hijacked DLL/exe from
    // an attacker-controlled CWD (same reasoning as security::trusted_system_program).
    let dir = exe.parent().map(std::path::Path::to_path_buf);

    let to_wide =
        |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(std::iter::once(0)).collect() };
    let exe_w = to_wide(exe.as_os_str());
    let dir_w = dir.as_deref().map(|d| to_wide(d.as_os_str()));
    let verb_w: Vec<u16> = "runas\0".encode_utf16().collect();

    // ShellExecuteEx may route through COM (shell extensions); this blocking
    // thread has no apartment of its own. RPC_E_CHANGED_MODE means COM was
    // already initialized in another mode — leave it as-is and don't uninit.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let should_uninit = hr.is_ok();

    // Zeroed base is the canonical way to build SHELLEXECUTEINFOW (all fields
    // are null/0-valid, including the hIcon/hMonitor union).
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    // NOASYNC: complete the launch before returning — we exit immediately after,
    // and an async in-process handler would be torn down mid-flight.
    info.fMask = SEE_MASK_NOASYNC;
    info.lpVerb = PCWSTR(verb_w.as_ptr());
    info.lpFile = PCWSTR(exe_w.as_ptr());
    info.lpDirectory = dir_w
        .as_ref()
        .map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
    info.nShow = SW_SHOWNORMAL.0;

    let result = unsafe { ShellExecuteExW(&mut info) };

    if should_uninit {
        unsafe { CoUninitialize() };
    }

    match result {
        Ok(()) => Ok(true),
        Err(e) => {
            let code = e.code();
            if code == HRESULT::from_win32(ERROR_CANCELLED.0) {
                // User clicked "No" on the UAC prompt — deliberate, not an error.
                Ok(false)
            } else if code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
                Err("Windows denied elevation. Try launching the app as administrator manually.".to_string())
            } else {
                Err(format!("Could not relaunch with administrator rights: {e}"))
            }
        }
    }
}

/// Non-Windows builds have no elevation path; the caller surfaces this.
#[cfg(not(windows))]
pub fn relaunch_self_elevated() -> Result<bool, String> {
    Err("Administrator restart is only available on Windows".to_string())
}
