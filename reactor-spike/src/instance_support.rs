//! Single-instance handoff.
//!
//! `Local\<identifier>` mutex decides primary/secondary at startup, before
//! any WinUI work; a secondary signals a named activation event and exits.
//! The primary polls the event and restores + foregrounds its main window,
//! matching the single-instance plugin's focus behavior. The Win32 calls are
//! localized here with explicit SAFETY notes.

use std::sync::OnceLock;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcessId, OpenEventW, SetEvent,
    WaitForSingleObject, EVENT_MODIFY_STATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_RESTORE, SW_SHOW,
};
use windows::core::{BOOL, PCWSTR};

/// HANDLE is a raw pointer; the event handle is process-lifetime and only
/// ever used for wait/signal calls, which are thread-safe.
struct EventHandle(HANDLE);
unsafe impl Send for EventHandle {}
unsafe impl Sync for EventHandle {}

static ACTIVATION_EVENT: OnceLock<EventHandle> = OnceLock::new();

/// The outcome of the startup single-instance acquisition.
pub enum SingleInstanceDecision {
    /// This process owns the mutex; `InstanceWatch` observes activate
    /// requests from later launches.
    Primary(InstanceWatch),
    /// Another instance is already running and has been signalled to come to
    /// the foreground. The caller should exit.
    Secondary,
}

/// Acquire the single-instance mutex and prepare the activation event.
#[must_use]
pub fn acquire(identifier: &str) -> SingleInstanceDecision {
    let mutex_name = wide(&format!("Local\\{identifier}-single-instance"));
    let event_name = wide(&format!("Local\\{identifier}-activate"));

    // SAFETY: names are NUL-terminated UTF-16; no attributes needed.
    let mutex = unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr())) };
    let already_exists = unsafe { windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS };
    let _mutex_guard = mutex; // released on drop of the process handle wrapper

    if already_exists {
        // Signal the primary's activation event. Create it if the primary has
        // not made it yet (startup race) — a set event nobody waits on is
        // harmless.
        let opened =
            unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name.as_ptr())) };
        match opened {
            Ok(handle) => unsafe {
                let _ = SetEvent(handle);
            },
            Err(_) => {
                if let Ok(handle) =
                    unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }
                {
                    unsafe {
                        let _ = SetEvent(handle);
                    }
                }
            }
        }
        return SingleInstanceDecision::Secondary;
    }

    if let Ok(event) = unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) } {
        let _ = ACTIVATION_EVENT.set(EventHandle(event));
    }
    SingleInstanceDecision::Primary(InstanceWatch)
}

/// True when a secondary launch requested activation.
#[must_use]
pub fn activation_requested() -> bool {
    let Some(EventHandle(event)) = ACTIVATION_EVENT.get() else {
        return false;
    };
    unsafe { WaitForSingleObject(*event, 0) == WAIT_OBJECT_0 }
}

/// Watch the activation event from a background thread.
#[derive(Default)]
pub struct InstanceWatch;

/// Find this process's main visible top-level window. Skips tool windows so
/// transient popups never win.
#[must_use]
pub fn main_window_hwnd() -> Option<HWND> {
    struct Collector {
        current_process: u32,
        best: Option<HWND>,
    }
    unsafe extern "system" fn on_window(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam points at our Collector for the duration of the
        // enumeration; the OS hands back the exact pointer we passed.
        let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
        let mut process_id = 0_u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id != collector.current_process
            || !unsafe { IsWindowVisible(window) }.as_bool()
        {
            return BOOL(1);
        }
        // Skip tool windows (transient popups); prefer the main app window.
        let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        if (ex_style & 0x0000_0008) != 0 {
            return BOOL(1);
        }
        if collector.best.is_none() {
            collector.best = Some(window);
        }
        BOOL(1)
    }

    let mut collector = Collector {
        current_process: unsafe { GetCurrentProcessId() },
        best: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(on_window),
            LPARAM((&mut collector as *mut Collector) as isize),
        );
    }
    collector.best
}

/// Restore and foreground this process's main visible window.
pub fn activate_main_window() {
    if let Some(window) = main_window_hwnd() {
        unsafe {
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            } else {
                let _ = ShowWindow(window, SW_SHOW);
            }
            let _ = SetForegroundWindow(window);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
