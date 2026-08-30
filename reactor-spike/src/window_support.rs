//! Tray icon + close-to-tray + show/hide lifecycle via documented Win32
//! interop (the owner-approved path for surfaces Reactor's own API does not
//! expose yet).
//!
//! All raw interop lives in this one file so a future Reactor-native
//! equivalent is a single-file swap. The main window's procedure is
//! subclassed with `SetWindowSubclass` (comctl32), which chains to the
//! original Reactor procedure for every message it does not intercept:
//!
//! - `WM_CLOSE` is swallowed only while close-to-tray is enabled and the
//!   window is visible (hide + add the tray icon instead).
//! - `WM_APP_TRAY` is the tray icon's callback: left click restores, right
//!   click opens the Store 2.5.8 tray menu (Show / Quick Scan / Exit).
//! - The menu choice (and left-click restore) land in a shared command slot
//!   the component polls, because Reactor owns the message loop and the
//!   component state must only be touched on the UI thread.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    DefSubclassProc, ExtractIconW, SetWindowSubclass, Shell_NotifyIconW, NOTIFYICONDATAW,
    NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, IsIconic, IsWindowVisible,
    LoadIconW, SetForegroundWindow, ShowWindow, IDI_APPLICATION, MF_SEPARATOR, MF_STRING,
    SW_HIDE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, WM_CLOSE,
    WM_COMMAND,
};
use windows::Win32::UI::WindowsAndMessaging::TrackPopupMenu;
use windows::core::PCWSTR;

/// Sentinel: no tray command pending.
pub const TRAY_COMMAND_NONE: u8 = 0;
/// Tray menu: show/restore the window.
pub const TRAY_COMMAND_SHOW: u8 = 1;
/// Tray menu: start the Quick Scan.
pub const TRAY_COMMAND_QUICK_SCAN: u8 = 2;
/// Tray menu: exit the application (bypasses close-to-tray).
pub const TRAY_COMMAND_EXIT: u8 = 3;

/// Menu ids for `TrackPopupMenu` with `TPM_RETURNCMD`.
const MENU_SHOW: u32 = 1;
const MENU_QUICK_SCAN: u32 = 2;
const MENU_EXIT: u32 = 3;

/// Win32 app-defined message used for the tray icon callback.
const WM_APP_TRAY: u32 = 0x8000; // WM_APP
const SUBCLASS_ID: usize = 0x5754_4449; // "WTDI"

static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);
static TRAY_COMMAND: AtomicU8 = AtomicU8::new(TRAY_COMMAND_NONE);
static TRAY_ADDED: AtomicBool = AtomicBool::new(false);
/// Set when the user picks Exit so the next WM_CLOSE passes through.
static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);

/// Install the close-to-tray behavior and the tray icon on the given window.
///
/// # Errors
/// When the subclass or tray icon cannot be installed.
pub fn install(window: HWND, tooltip: &str) -> Result<(), String> {
    // SAFETY: the window belongs to this thread; comctl32 subclassing chains
    // to the previous procedure for every message not handled below.
    let ok = unsafe { SetWindowSubclass(window, Some(tray_subclass_proc), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        return Err("Windows refused the tray subclass installation".to_string());
    }
    add_tray_icon(window, tooltip)
}

/// Update the close-to-tray policy (wired to the settings toggle).
pub fn set_close_to_tray(enabled: bool) {
    CLOSE_TO_TRAY.store(enabled, Ordering::SeqCst);
}

/// Remove the tray icon (called before a real exit).
pub fn remove_tray_icon(window: HWND) {
    if !TRAY_ADDED.swap(false, Ordering::SeqCst) {
        return;
    }
    let data = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX),
        hWnd: window,
        uID: 1,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

/// Take and clear a pending tray command, if any.
#[must_use]
pub fn take_tray_command() -> u8 {
    TRAY_COMMAND.swap(TRAY_COMMAND_NONE, Ordering::SeqCst)
}

/// Restore + foreground the window (tray Show, activation hand-off).
pub fn restore(window: HWND) {
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        } else {
            let _ = ShowWindow(window, SW_SHOW);
        }
        let _ = SetForegroundWindow(window);
    }
}

/// Hide the window to the tray (tray menu "Hide to tray").
pub fn hide(window: HWND) {
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

/// Whether the main window is currently visible.
#[must_use]
pub fn is_visible(window: HWND) -> bool {
    unsafe { IsWindowVisible(window) }.as_bool()
}

fn add_tray_icon(window: HWND, tooltip: &str) -> Result<(), String> {
    let mut data = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX),
        hWnd: window,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: load_app_icon(),
        szTip: [0; 128],
        ..Default::default()
    };
    let tip: Vec<u16> = tooltip
        .encode_utf16()
        .take(data.szTip.len() - 1)
        .chain(std::iter::once(0))
        .collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    // SAFETY: data is fully initialized for NIM_ADD.
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err("Windows refused the tray icon".to_string())
    }
}

/// The executable's first icon resource, or the default application icon.
fn load_app_icon() -> windows::Win32::UI::WindowsAndMessaging::HICON {
    unsafe {
        if let Ok(module) = GetModuleHandleW(None) {
            let instance = HINSTANCE(module.0);
            let icon = ExtractIconW(Some(instance), PCWSTR::null(), 0);
            if !icon.is_invalid() && icon.0 as usize != 1 {
                return icon;
            }
        }
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }
}

unsafe extern "system" fn tray_subclass_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _context: usize,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            let visible = unsafe { IsWindowVisible(window) }.as_bool();
            if !FORCE_CLOSE.swap(false, Ordering::SeqCst)
                && CLOSE_TO_TRAY.load(Ordering::SeqCst)
                && visible
            {
                // Hide to the tray instead of closing; the tray menu's Exit
                // sets FORCE_CLOSE so the next close is real.
                unsafe {
                    let _ = ShowWindow(window, SW_HIDE);
                }
                return LRESULT(0);
            }
        }
        WM_APP_TRAY => {
            // lParam carries the mouse event for tray callback icons.
            let mouse_event = lparam.0 as u32;
            const WM_LBUTTONUP: u32 = 0x0202;
            const WM_RBUTTONUP: u32 = 0x0205;
            if mouse_event == WM_LBUTTONUP {
                TRAY_COMMAND.store(TRAY_COMMAND_SHOW, Ordering::SeqCst);
            } else if mouse_event == WM_RBUTTONUP {
                show_tray_menu(window);
            }
            return LRESULT(0);
        }
        WM_COMMAND => {
            let menu_id = (wparam.0 & 0xFFFF) as u32;
            let command = match menu_id {
                MENU_SHOW => Some(TRAY_COMMAND_SHOW),
                MENU_QUICK_SCAN => Some(TRAY_COMMAND_QUICK_SCAN),
                MENU_EXIT => Some(TRAY_COMMAND_EXIT),
                _ => None,
            };
            if let Some(command) = command {
                TRAY_COMMAND.store(command, Ordering::SeqCst);
                return LRESULT(0);
            }
        }
        _ => {}
    }
    // SAFETY: the subclass contract hands the original procedure back for
    // every message we do not fully handle.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

fn show_tray_menu(window: HWND) {
    unsafe {
        let menu = CreatePopupMenu().unwrap_or_default();
        if menu.is_invalid() {
            return;
        }
        let visible = IsWindowVisible(window).as_bool();
        let show_label: Vec<u16> = if visible {
            "Hide to tray".encode_utf16().chain(std::iter::once(0)).collect()
        } else {
            "Show".encode_utf16().chain(std::iter::once(0)).collect()
        };
        let quick_label: Vec<u16> =
            "Quick Scan".encode_utf16().chain(std::iter::once(0)).collect();
        let exit_label: Vec<u16> = "Exit".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the menu is owned by this scope; text pointers outlive
        // each AppendMenuW call.
        let _ = AppendMenuW(menu, MF_STRING, MENU_SHOW as usize, PCWSTR(show_label.as_ptr()));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_QUICK_SCAN as usize,
            PCWSTR(quick_label.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT as usize, PCWSTR(exit_label.as_ptr()));

        // The tray click arrives on the window's thread; foreground first so
        // the menu dismisses on outside clicks.
        let _ = SetForegroundWindow(window);
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // With TPM_RETURNCMD the chosen menu id comes back in the BOOL.
        let chosen = TrackPopupMenu(
            menu,
            TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            None,
            window,
            None,
        );
        let _ = DestroyMenu(menu);
        if chosen.as_bool() {
            TRAY_COMMAND.store(chosen.0 as u8, Ordering::SeqCst);
        }
    }
}

/// Arm a real close for the next WM_CLOSE, bypassing close-to-tray once.
pub fn request_forced_close() {
    FORCE_CLOSE.store(true, Ordering::SeqCst);
}
