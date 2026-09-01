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
//! - Shipping global key chords are decoded without touching component state
//!   and published alongside the tray command. One coalesced app-defined
//!   message wakes Reactor so its state remains UI-thread-only without polling.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use crate::instance_support;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyState};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ExtractIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW, RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::TrackPopupMenu;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, DestroyMenu, GetClassNameW, GetCursorPos,
    GetForegroundWindow, HHOOK, IDI_APPLICATION, IsIconic, IsWindowVisible, LoadIconW,
    MF_SEPARATOR, MF_STRING, PostMessageW, RegisterWindowMessageW, SIZE_MINIMIZED, SW_HIDE,
    SW_RESTORE, SW_SHOW, SetForegroundWindow, SetWindowsHookExW, ShowWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, UnhookWindowsHookEx, WA_INACTIVE, WH_KEYBOARD, WM_ACTIVATE,
    WM_CLOSE, WM_COMMAND, WM_NCDESTROY, WM_SHOWWINDOW, WM_SIZE, WM_WINDOWPOSCHANGED,
};
use windows::core::{PCWSTR, w};

/// Sentinel: no tray command pending.
pub const TRAY_COMMAND_NONE: u8 = 0;
/// Tray menu: show/restore the window.
pub const TRAY_COMMAND_SHOW: u8 = 1;
/// Tray menu: start the Quick Scan.
pub const TRAY_COMMAND_QUICK_SCAN: u8 = 2;
/// Tray menu: exit the application (bypasses close-to-tray).
pub const TRAY_COMMAND_EXIT: u8 = 3;

/// One shipping application-wide shortcut captured by the window subclass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalShortcutCommand {
    TogglePalette,
    PalettePrevious,
    PaletteNext,
    PaletteExecute,
    PaletteClose,
    Navigate(u8),
    ShowHelp,
    QuickScan,
    FullScan,
}

/// Shortcut evidence captured at key-down time.
///
/// WinUI 3 controls are normally windowless. `editable_focused` is therefore
/// best-effort: it is exact for HWND-backed Edit/RichEdit/ComboBox controls,
/// while the UI-thread policy remains the authoritative overlay/scan gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalShortcutEvent {
    pub command: GlobalShortcutCommand,
    pub editable_focused: bool,
}

/// Menu ids for `TrackPopupMenu` with `TPM_RETURNCMD`.
const MENU_SHOW: u32 = 1;
const MENU_QUICK_SCAN: u32 = 2;
const MENU_EXIT: u32 = 3;

/// Win32 app-defined message used for the tray icon callback.
const WM_APP_TRAY: u32 = 0x8000; // WM_APP
/// Event-driven bridge from native/backend producers back to the Reactor UI
/// thread. `WM_APP_TRAY` already occupies the first application message.
const WM_APP_UI_WAKE: u32 = WM_APP_TRAY + 1;
const SUBCLASS_ID: usize = 0x5754_4449; // "WTDI"

const FLAG_REGISTERED: u8 = 1 << 0;
const FLAG_VISIBLE: u8 = 1 << 1;
const FLAG_MINIMIZED: u8 = 1 << 2;
const FLAG_FOCUSED: u8 = 1 << 3;
const FLAG_TRAY_PRESENT: u8 = 1 << 4;
const FLAG_CLOSE_TO_TRAY: u8 = 1 << 5;
const WINDOW_STATE_FLAGS: u8 = FLAG_REGISTERED | FLAG_VISIBLE | FLAG_MINIMIZED | FLAG_FOCUSED;
const STATE_FLAGS_MASK: u64 = 0xff;
const STATE_REVISION_SHIFT: u32 = 8;

/// One atomically published word: low eight bits are state flags and the
/// remaining bits are a monotonically wrapping revision. A single word keeps
/// readers from observing a new visibility flag with an old revision (or vice
/// versa) while the Win32 subclass and the component watcher run on different
/// threads.
static LIFECYCLE_STATE: AtomicU64 = AtomicU64::new(0);
static TRAY_COMMAND: AtomicU8 = AtomicU8::new(TRAY_COMMAND_NONE);
/// Mirrored from component state so the raw subclass only captures and
/// consumes palette-local navigation keys while the palette actually owns
/// them. Ordinary TextBox arrows/Enter/Escape otherwise remain untouched.
static PALETTE_OPEN: AtomicBool = AtomicBool::new(false);
/// Preserve discrete key-down order between the Win32 callback and Reactor's
/// 50 ms poll. A single atomic slot lost rapid sequences such as Down+Enter.
static SHORTCUT_EVENTS: Mutex<VecDeque<u16>> = Mutex::new(VecDeque::new());
const SHORTCUT_QUEUE_CAPACITY: usize = 64;
/// Set when the user picks Exit so the next WM_CLOSE passes through.
static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);
/// At most one native wake message is queued at a time. This replaces the
/// former 50 ms Reactor polling task without allowing a busy producer to
/// flood the Win32 queue.
static UI_WAKE_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
struct KeyboardHookState {
    top_window: isize,
    hook: isize,
}

thread_local! {
    /// Reactor's `LocalSender` is deliberately !Send. Keeping the callback in
    /// UI-thread local storage lets the raw window procedure notify the
    /// component without moving that sender through a worker thread.
    static UI_WAKE_HANDLER: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
    /// `WH_KEYBOARD` is scoped to the Reactor UI thread. It observes the
    /// thread's WinUI/XAML input route without installing a desktop-global
    /// hook and without depending on implementation-detail child HWNDs.
    static KEYBOARD_HOOK_STATE: RefCell<KeyboardHookState> = RefCell::new(KeyboardHookState::default());
}

const SHORTCUT_EDITABLE_FLAG: u16 = 1 << 8;
const SHORTCUT_COMMAND_MASK: u16 = 0xff;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ShortcutModifiers {
    control: bool,
    shift: bool,
    alt: bool,
}

/// Coherent window/tray state published by the Win32 subclass.
///
/// The component retains `revision` and calls
/// [`lifecycle_snapshot_if_changed`] after a coalesced native wake. No raw
/// window procedure crosses directly into Reactor component state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowLifecycleSnapshot {
    pub revision: u64,
    pub registered: bool,
    pub visible: bool,
    pub minimized: bool,
    pub focused: bool,
    pub tray_icon_present: bool,
    pub close_to_tray_enabled: bool,
}

/// Installation failure with enough detail for the component to distinguish
/// a missing tray icon from a missing event-delivery subclass.
///
/// Once `core_installed` is true, lifecycle and `WM_APP_UI_WAKE` delivery are
/// usable even if Explorer rejected the optional notification-area icon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowHookInstallError {
    message: String,
    core_installed: bool,
}

impl WindowHookInstallError {
    fn core(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            core_installed: false,
        }
    }

    fn tray(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            core_installed: true,
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn core_installed(&self) -> bool {
        self.core_installed
    }
}

/// Return a coherent current lifecycle snapshot.
#[must_use]
pub fn lifecycle_snapshot() -> WindowLifecycleSnapshot {
    decode_lifecycle_word(LIFECYCLE_STATE.load(Ordering::Acquire))
}

/// Return a snapshot only after a transition newer than `last_revision`.
///
/// Revisions wrap after 2^56 state changes; equality is the only meaningful
/// comparison and therefore remains correct across wrapping.
#[must_use]
pub fn lifecycle_snapshot_if_changed(last_revision: u64) -> Option<WindowLifecycleSnapshot> {
    let snapshot = lifecycle_snapshot();
    (snapshot.revision != last_revision).then_some(snapshot)
}

/// Install the close-to-tray behavior and the tray icon on the given window.
///
/// # Errors
/// When the subclass or tray icon cannot be installed.
pub fn install(window: HWND, tooltip: &str) -> Result<(), WindowHookInstallError> {
    install_core(window, Some(tooltip))
}

/// Install the lifecycle, shortcut, and event-driven wake bridge without a
/// notification-area icon or close-to-tray behavior.
///
/// This is the safe path for the `WFDIAG_NO_TRAY` investigation switch. The
/// native subclass is still required because it owns `WM_APP_UI_WAKE`; merely
/// skipping [`install`] would strand worker events once polling is removed.
pub fn install_without_tray(window: HWND) -> Result<(), WindowHookInstallError> {
    // A remount can follow a tray-enabled window in tests or diagnostics.
    // Never leave close-to-tray armed when there is no icon to restore from.
    set_close_to_tray(false);
    install_core(window, None)
}

fn install_core(window: HWND, tooltip: Option<&str>) -> Result<(), WindowHookInstallError> {
    if !instance_support::register_main_window(window) {
        return Err(WindowHookInstallError::core(
            "The Reactor main window handle is stale or does not belong to this process"
                .to_string(),
        ));
    }

    // SAFETY: the window belongs to this thread; comctl32 subclassing chains
    // to the previous procedure for every message not handled below.
    let ok = unsafe { SetWindowSubclass(window, Some(tray_subclass_proc), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        // Do not let worker threads post wake messages to a window that has no
        // matching subclass; Windows would accept those messages, but nobody
        // would clear the coalescing flag after consuming them.
        instance_support::unregister_main_window(window);
        // A worker can race the tiny interval between HWND registration and
        // SetWindowSubclass. Its PostMessage succeeds but the unhooked window
        // cannot clear the coalescing bit, so explicitly make a later retry
        // eligible to post a fresh wake.
        UI_WAKE_PENDING.store(false, Ordering::Release);
        return Err(WindowHookInstallError::core(
            "Windows refused the native window subclass installation",
        ));
    }
    if let Err(error) = install_keyboard_hook(window) {
        // Shortcut capture is part of the core native integration. Roll back
        // the otherwise healthy top hook so Reactor's bounded startup retry
        // can install both pieces as one coherent generation.
        unsafe {
            let _ = RemoveWindowSubclass(window, Some(tray_subclass_proc), SUBCLASS_ID);
        }
        instance_support::unregister_main_window(window);
        UI_WAKE_PENDING.store(false, Ordering::Release);
        return Err(WindowHookInstallError::core(error));
    }
    // Refresh only after the subclass is live. `refresh_window_state` posts a
    // wake on the first registration; posting before SetWindowSubclass would
    // leave `UI_WAKE_PENDING` stuck if subclass installation failed.
    refresh_window_state(window);
    if let Some(tooltip) = tooltip {
        add_tray_icon(window, tooltip).map_err(WindowHookInstallError::tray)
    } else {
        Ok(())
    }
}

/// Update the close-to-tray policy (wired to the settings toggle).
pub fn set_close_to_tray(enabled: bool) {
    set_lifecycle_flag(FLAG_CLOSE_TO_TRAY, enabled);
}

/// Publish whether the command palette currently owns local navigation keys.
pub fn set_palette_open(open: bool) {
    PALETTE_OPEN.store(open, Ordering::Release);
}

/// Install the UI-thread callback invoked by [`post_ui_wake`].
///
/// The callback normally sends one lightweight drain message through
/// Reactor's local component sender. Replacing an existing callback is safe
/// during component remount; teardown should call [`clear_ui_wake_handler`].
pub fn set_ui_wake_handler(handler: impl Fn() + 'static) {
    UI_WAKE_HANDLER.with(|slot| *slot.borrow_mut() = Some(Rc::new(handler)));
    if native_signal_pending() {
        let _ = post_ui_wake();
    }
}

/// Remove the current UI-thread callback.
pub fn clear_ui_wake_handler() {
    UI_WAKE_HANDLER.with(|slot| slot.borrow_mut().take());
    UI_WAKE_PENDING.store(false, Ordering::Release);
}

/// Queue one event-driven callback on the Reactor window's UI thread.
///
/// This function is safe to call from backend workers. Repeated calls before
/// the window processes the message coalesce into a single wake.
#[must_use]
pub fn post_ui_wake() -> bool {
    // Coalesce via the pending flag, but never blindly trust it: the poster
    // that set it may still fail below (window not yet registered, PostMessage
    // refused) or not have posted yet. When a window is resolvable, post
    // anyway — a duplicate WM_APP_UI_WAKE is a harmless no-op drain, while a
    // swallowed wake strands workers until the next unrelated event.
    let pending = UI_WAKE_PENDING.swap(true, Ordering::AcqRel);
    // Worker notifications must never enumerate desktop windows. The UI
    // thread discovers and registers the exact Reactor window during hook
    // installation; its initial lifecycle transition then drains anything
    // queued before registration.
    let Some(window) = instance_support::registered_main_window_hwnd() else {
        if !pending {
            UI_WAKE_PENDING.store(false, Ordering::Release);
        }
        return false;
    };
    if unsafe { PostMessageW(Some(window), WM_APP_UI_WAKE, WPARAM(0), LPARAM(0)) }.is_err() {
        if !pending {
            UI_WAKE_PENDING.store(false, Ordering::Release);
        }
        return false;
    }
    true
}

fn dispatch_ui_wake() {
    // Clear first. Anything published while the callback/drain is running can
    // enqueue the next message and therefore cannot be stranded.
    UI_WAKE_PENDING.store(false, Ordering::Release);
    // Clone the UI-thread-only Rc and release the RefCell borrow before
    // invoking application code. This permits a callback to replace or clear
    // itself during component teardown without a re-entrant borrow panic.
    let handler = UI_WAKE_HANDLER.with(|slot| slot.borrow().clone());
    if let Some(handler) = handler {
        handler();
    }
}

fn native_signal_pending() -> bool {
    lifecycle_snapshot().revision != 0
        || TRAY_COMMAND.load(Ordering::Acquire) != TRAY_COMMAND_NONE
        || !shortcut_events().is_empty()
}

/// Remove the tray icon (called before a real exit).
pub fn remove_tray_icon(window: HWND) {
    if !lifecycle_flag_is_set(FLAG_TRAY_PRESENT) {
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
    set_lifecycle_flag(FLAG_TRAY_PRESENT, false);
}

/// Take and clear a pending tray command, if any.
#[must_use]
pub fn take_tray_command() -> u8 {
    TRAY_COMMAND.swap(TRAY_COMMAND_NONE, Ordering::SeqCst)
}

/// Captured intent behind `TRAY_COMMAND_SHOW`: hide if the window was visible
/// when its menu entry was built, restore otherwise. Values mirror the
/// command sentinel style.
pub const TRAY_MENU_INTENT_SHOW: u8 = 0;
/// See [`TRAY_MENU_INTENT_SHOW`].
pub const TRAY_MENU_INTENT_HIDE: u8 = 1;

static TRAY_MENU_INTENT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(TRAY_MENU_INTENT_SHOW);

/// Take and clear the captured show/hide intent for the pending
/// `TRAY_COMMAND_SHOW`.
#[must_use]
pub fn take_tray_menu_intent() -> u8 {
    TRAY_MENU_INTENT.swap(TRAY_MENU_INTENT_SHOW, Ordering::SeqCst)
}

/// Take the oldest captured shipping shortcut.
///
/// No raw window callback ever enters Reactor component state; the bounded
/// queue preserves rapid multi-key sequences until the component polls them.
#[must_use]
pub fn take_global_shortcut() -> Option<GlobalShortcutEvent> {
    // Never block: the queue is drained by a background poll loop that
    // retries on its next tick, so a contended miss costs nothing. The hook
    // side keeps the blocking lock, but its critical section is a push of a
    // few nanoseconds on the UI thread itself.
    let mut events = match SHORTCUT_EVENTS.try_lock() {
        Ok(events) => events,
        Err(std::sync::TryLockError::WouldBlock) => return None,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    };
    let encoded = pop_shortcut_event(&mut events)?;
    decode_shortcut_event(encoded)
}

fn publish_global_shortcut(command: GlobalShortcutCommand, editable_focused: bool) {
    let encoded = encode_shortcut_event(GlobalShortcutEvent {
        command,
        editable_focused,
    });
    if encoded == 0 {
        return;
    }
    let mut events = shortcut_events();
    push_shortcut_event(&mut events, encoded);
    drop(events);
    let _ = post_ui_wake();
}

fn push_shortcut_event(events: &mut VecDeque<u16>, encoded: u16) {
    if events.len() == SHORTCUT_QUEUE_CAPACITY {
        // Prefer the newest complete interaction when auto-repeat floods the
        // queue. Remaining entries retain strict FIFO order.
        let _ = events.pop_front();
    }
    events.push_back(encoded);
}

fn pop_shortcut_event(events: &mut VecDeque<u16>) -> Option<u16> {
    events.pop_front()
}

fn shortcut_events() -> std::sync::MutexGuard<'static, VecDeque<u16>> {
    SHORTCUT_EVENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn clear_global_shortcuts() {
    shortcut_events().clear();
}

fn encode_shortcut_event(event: GlobalShortcutEvent) -> u16 {
    let command = match event.command {
        GlobalShortcutCommand::TogglePalette => 1,
        GlobalShortcutCommand::Navigate(index @ 1..=6) => u16::from(index) + 1,
        GlobalShortcutCommand::Navigate(_) => return 0,
        GlobalShortcutCommand::ShowHelp => 8,
        GlobalShortcutCommand::QuickScan => 9,
        GlobalShortcutCommand::FullScan => 10,
        GlobalShortcutCommand::PalettePrevious => 11,
        GlobalShortcutCommand::PaletteNext => 12,
        GlobalShortcutCommand::PaletteExecute => 13,
        GlobalShortcutCommand::PaletteClose => 14,
    };
    command
        | if event.editable_focused {
            SHORTCUT_EDITABLE_FLAG
        } else {
            0
        }
}

fn decode_shortcut_event(encoded: u16) -> Option<GlobalShortcutEvent> {
    let command = match encoded & SHORTCUT_COMMAND_MASK {
        1 => GlobalShortcutCommand::TogglePalette,
        code @ 2..=7 => GlobalShortcutCommand::Navigate((code - 1) as u8),
        8 => GlobalShortcutCommand::ShowHelp,
        9 => GlobalShortcutCommand::QuickScan,
        10 => GlobalShortcutCommand::FullScan,
        11 => GlobalShortcutCommand::PalettePrevious,
        12 => GlobalShortcutCommand::PaletteNext,
        13 => GlobalShortcutCommand::PaletteExecute,
        14 => GlobalShortcutCommand::PaletteClose,
        _ => return None,
    };
    Some(GlobalShortcutEvent {
        command,
        editable_focused: encoded & SHORTCUT_EDITABLE_FLAG != 0,
    })
}

fn classify_global_shortcut(
    virtual_key: u32,
    modifiers: ShortcutModifiers,
) -> Option<GlobalShortcutCommand> {
    if modifiers.alt {
        return None;
    }
    if !modifiers.control && !modifiers.shift {
        return match virtual_key {
            0x26 => Some(GlobalShortcutCommand::PalettePrevious), // Up
            0x28 => Some(GlobalShortcutCommand::PaletteNext),     // Down
            0x0d => Some(GlobalShortcutCommand::PaletteExecute),  // Enter
            0x1b => Some(GlobalShortcutCommand::PaletteClose),    // Escape
            _ => None,
        };
    }
    if !modifiers.control {
        return None;
    }
    match (virtual_key, modifiers.shift) {
        (0x4b, false) => Some(GlobalShortcutCommand::TogglePalette), // K
        (key @ 0x31..=0x36, false) => Some(GlobalShortcutCommand::Navigate((key - 0x30) as u8)),
        (0xbf | 0x6f, false) => Some(GlobalShortcutCommand::ShowHelp), // OEM / or numpad /
        (0x51, true) => Some(GlobalShortcutCommand::QuickScan),        // Q
        (0x46, true) => Some(GlobalShortcutCommand::FullScan),         // F
        _ => None,
    }
}

fn key_is_down(virtual_key: i32) -> bool {
    (unsafe { GetKeyState(virtual_key) }) < 0
}

fn current_shortcut_modifiers() -> ShortcutModifiers {
    ShortcutModifiers {
        control: key_is_down(0x11),
        shift: key_is_down(0x10),
        alt: key_is_down(0x12),
    }
}

/// Publish a recognized chord and report whether XAML must not process the
/// same key. Palette-local navigation is consumed only while the palette owns
/// it; every application-wide chord continues through the native input path.
fn handle_key_down(virtual_key: u32, repeated: bool) -> bool {
    let Some(command) = classify_global_shortcut(virtual_key, current_shortcut_modifiers()) else {
        return false;
    };
    let palette_local = matches!(
        command,
        GlobalShortcutCommand::PalettePrevious
            | GlobalShortcutCommand::PaletteNext
            | GlobalShortcutCommand::PaletteExecute
            | GlobalShortcutCommand::PaletteClose
    );
    if palette_local && !PALETTE_OPEN.load(Ordering::Acquire) {
        return false;
    }
    // Arrow-key repeat is useful while traversing an open palette. Every
    // application-wide chord is edge-triggered so holding Ctrl+K cannot
    // oscillate the palette or launch the same scan more than once.
    if repeated && !palette_local {
        return false;
    }
    publish_global_shortcut(command, focused_control_is_editable());
    palette_local
}

fn install_keyboard_hook(top_window: HWND) -> Result<(), String> {
    let top_window = top_window.0 as isize;
    let current = KEYBOARD_HOOK_STATE.with(|slot| *slot.borrow());
    if current.hook != 0 {
        // The hook is owned by the UI thread, not by an HWND. A Reactor
        // top-window replacement on the same thread only changes foreground
        // scoping; installing a second hook would duplicate every chord.
        KEYBOARD_HOOK_STATE.with(|slot| slot.borrow_mut().top_window = top_window);
        return Ok(());
    }

    // SAFETY: a zero module handle is valid for a hook procedure in this
    // executable when the hook is restricted to the current UI thread.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD,
            Some(keyboard_hook_proc),
            None,
            GetCurrentThreadId(),
        )
    }
    .map_err(|error| format!("Windows refused the UI-thread keyboard hook: {error}"))?;

    KEYBOARD_HOOK_STATE.with(|slot| {
        *slot.borrow_mut() = KeyboardHookState {
            top_window,
            hook: hook.0 as isize,
        };
    });
    Ok(())
}

fn remove_keyboard_hook(top_window: HWND) {
    let top_window = top_window.0 as isize;
    let hook = KEYBOARD_HOOK_STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.top_window != top_window {
            return 0;
        }
        let hook = state.hook;
        *state = KeyboardHookState::default();
        hook
    });
    if hook != 0 {
        // SAFETY: top-window teardown runs on the thread that installed this
        // thread-specific hook.
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(hook as *mut std::ffi::c_void));
        }
    }
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Bit 31 is clear for key-down and set for key-up. Negative hook codes
    // must pass through untouched. Scope the chord to the foreground Reactor
    // window even though the hook itself is already confined to its UI thread.
    let bits = lparam.0 as usize;
    let key_down = (bits & (1_usize << 31)) == 0;
    let repeated = (bits & (1_usize << 30)) != 0;
    let owns_foreground = KEYBOARD_HOOK_STATE.with(|slot| {
        let state = slot.borrow();
        state.top_window != 0 && unsafe { GetForegroundWindow() }.0 as isize == state.top_window
    });
    // HC_ACTION is the only actionable notification. HC_NOREMOVE must chain:
    // the same message will be observed again when the pump actually removes
    // it, and publishing during both peeks would duplicate the shortcut.
    if code == 0 && key_down && owns_foreground && handle_key_down(wparam.0 as u32, repeated) {
        return LRESULT(1);
    }
    // SAFETY: the hook contract requires every event not exclusively consumed
    // by the open command palette to continue through the hook chain.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn focused_control_is_editable() -> bool {
    let focused = unsafe { GetFocus() };
    if focused.is_invalid() {
        return false;
    }
    let mut class_name = [0_u16; 128];
    let length = unsafe { GetClassNameW(focused, &mut class_name) };
    if length <= 0 {
        return false;
    }
    class_name_is_editable(&String::from_utf16_lossy(
        &class_name[..usize::try_from(length).unwrap_or_default()],
    ))
}

fn class_name_is_editable(class_name: &str) -> bool {
    let class_name = class_name.trim().to_ascii_lowercase();
    class_name == "edit"
        || class_name.starts_with("richedit")
        || class_name == "combobox"
        || class_name.contains("textbox")
        || class_name.contains("passwordbox")
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
    refresh_window_state(window);
}

/// Hide the window to the tray (tray menu "Hide to tray").
pub fn hide(window: HWND) {
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
    refresh_window_state(window);
}

fn add_tray_icon(window: HWND, tooltip: &str) -> Result<(), String> {
    // Remember the tooltip so a shell restart can re-add the icon (see
    // `tray_subclass_proc`). First tooltip wins; the app uses one constant.
    let _ = TRAY_TOOLTIP.set(tooltip.to_string());
    let mut data = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX),
        hWnd: window,
        uID: 1,
        // NIF_ICON is required for `hIcon` to be shown at all; without it the
        // notification area reserved a blank slot and the restore affordance
        // was invisible.
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: tray_icon(),
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
        set_lifecycle_flag(FLAG_TRAY_PRESENT, true);
        Ok(())
    } else {
        Err("Windows refused the tray icon".to_string())
    }
}

/// The process-wide tray icon. `Shell_NotifyIconW` copies the icon, so the
/// handle is loaded once and reused across add/remove cycles instead of
/// leaking one `ExtractIconW` result per install. Stored as a `usize` because
/// raw `HICON` wrappers are not `Sync`.
fn tray_icon() -> windows::Win32::UI::WindowsAndMessaging::HICON {
    static TRAY_ICON: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let handle = *TRAY_ICON.get_or_init(|| load_app_icon().0 as usize);
    windows::Win32::UI::WindowsAndMessaging::HICON(handle as *mut core::ffi::c_void)
}

static TRAY_TOOLTIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// `TaskbarCreated` is broadcast whenever the shell restarts (Explorer crash,
/// shell relaunch, some DPI changes). Without re-adding the icon,
/// close-to-tray would keep hiding the window with no restore affordance.
fn taskbar_created_message() -> u32 {
    static TASKBAR_CREATED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *TASKBAR_CREATED.get_or_init(|| {
        // SAFETY: registers a well-known window message; no side effects.
        unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
    })
}

/// Re-add the tray icon after a shell restart. A no-op when the icon was
/// never installed or the re-add is refused; in the latter case the lifecycle
/// flag stays clear so close-to-tray degrades to a real close (matching the
/// install-time failure policy) instead of hiding the window into nothing.
fn restore_tray_icon_after_shell_restart(window: HWND) {
    if !lifecycle_flag_is_set(FLAG_TRAY_PRESENT) {
        return;
    }
    remove_tray_icon(window);
    if let Some(tooltip) = TRAY_TOOLTIP.get() {
        let _ = add_tray_icon(window, tooltip);
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
            if should_hide_on_close(
                FORCE_CLOSE.swap(false, Ordering::SeqCst),
                lifecycle_flag_is_set(FLAG_CLOSE_TO_TRAY),
                lifecycle_flag_is_set(FLAG_TRAY_PRESENT),
                visible,
            ) {
                // Hide to the tray instead of closing; the tray menu's Exit
                // sets FORCE_CLOSE so the next close is real.
                unsafe {
                    let _ = ShowWindow(window, SW_HIDE);
                }
                refresh_window_state(window);
                return LRESULT(0);
            }
        }
        WM_SHOWWINDOW => {
            let shown = wparam.0 != 0;
            set_lifecycle_flag(FLAG_VISIBLE, shown);
            if !shown {
                set_lifecycle_flag(FLAG_FOCUSED, false);
            }
        }
        WM_SIZE => {
            set_lifecycle_flag(FLAG_MINIMIZED, wparam.0 as u32 == SIZE_MINIMIZED);
        }
        WM_ACTIVATE => {
            let activation = (wparam.0 & 0xffff) as u32;
            set_lifecycle_flag(FLAG_FOCUSED, activation != WA_INACTIVE);
        }
        WM_WINDOWPOSCHANGED => {
            // Covers visibility changes made through SetWindowPos paths that
            // do not emit WM_SHOWWINDOW, and records the actual outcome if
            // Windows foreground-lock policy rejects an activation request.
            refresh_window_state(window);
        }
        WM_APP_TRAY => {
            // lParam carries the mouse event for tray callback icons.
            let mouse_event = lparam.0 as u32;
            const WM_LBUTTONUP: u32 = 0x0202;
            const WM_RBUTTONUP: u32 = 0x0205;
            if mouse_event == WM_LBUTTONUP {
                // Left click always means "restore".
                TRAY_MENU_INTENT.store(TRAY_MENU_INTENT_SHOW, Ordering::SeqCst);
                TRAY_COMMAND.store(TRAY_COMMAND_SHOW, Ordering::SeqCst);
                let _ = post_ui_wake();
            } else if mouse_event == WM_RBUTTONUP {
                show_tray_menu(window);
            }
            return LRESULT(0);
        }
        message if message == taskbar_created_message() => {
            restore_tray_icon_after_shell_restart(window);
            // The shell's broadcast expects normal message processing.
            return unsafe { DefSubclassProc(window, message, wparam, lparam) };
        }
        WM_APP_UI_WAKE => {
            dispatch_ui_wake();
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
                let _ = post_ui_wake();
                return LRESULT(0);
            }
        }
        WM_NCDESTROY => {
            // A replaced top-level HWND can finish destroying after its
            // successor is registered. Only the current generation owns the
            // shared hook, tray, wake handler, shortcut queue, and lifecycle.
            if instance_support::is_registered_main_window(window) {
                remove_keyboard_hook(window);
                remove_tray_icon(window);
                instance_support::unregister_main_window(window);
                clear_global_shortcuts();
                clear_ui_wake_handler();
                update_lifecycle_flags(0, WINDOW_STATE_FLAGS);
            }
            // Win32 requires subclasses to remove themselves before the
            // underlying window storage disappears. All other messages keep
            // chaining through DefSubclassProc below.
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(tray_subclass_proc), SUBCLASS_ID);
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
        // Snapshot the intended action NOW: the command is drained later, and
        // re-deciding from live visibility at drain time can invert against
        // the label the user just read (e.g. a WM_SHOWWINDOW from the scan
        // restore path lands in between).
        TRAY_MENU_INTENT.store(
            if visible {
                TRAY_MENU_INTENT_HIDE
            } else {
                TRAY_MENU_INTENT_SHOW
            },
            Ordering::SeqCst,
        );
        let show_label: Vec<u16> = if visible {
            "Hide to tray"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect()
        } else {
            "Show".encode_utf16().chain(std::iter::once(0)).collect()
        };
        let quick_label: Vec<u16> = "Quick Scan"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let exit_label: Vec<u16> = "Exit".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the menu is owned by this scope; text pointers outlive
        // each AppendMenuW call.
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_SHOW as usize,
            PCWSTR(show_label.as_ptr()),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_QUICK_SCAN as usize,
            PCWSTR(quick_label.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_EXIT as usize,
            PCWSTR(exit_label.as_ptr()),
        );

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
            let command = chosen.0 as u8;
            if command == TRAY_COMMAND_QUICK_SCAN {
                // Match the shipping Tauri flow: surface the scan before the
                // component receives the command. This is safe while hidden
                // because install registered this exact HWND independently of
                // its visibility.
                restore(window);
            }
            TRAY_COMMAND.store(command, Ordering::SeqCst);
            let _ = post_ui_wake();
        }
    }
}

/// Arm a real close for the next WM_CLOSE, bypassing close-to-tray once.
pub fn request_forced_close() {
    FORCE_CLOSE.store(true, Ordering::SeqCst);
}

/// Disarm a close bypass when Reactor rejected the corresponding close
/// request. This prevents a later ordinary title-bar close from unexpectedly
/// exiting instead of honoring close-to-tray.
pub fn cancel_forced_close() {
    FORCE_CLOSE.store(false, Ordering::SeqCst);
}

fn should_hide_on_close(
    forced_close: bool,
    close_to_tray: bool,
    tray_present: bool,
    visible: bool,
) -> bool {
    !forced_close && close_to_tray && tray_present && visible
}

fn refresh_window_state(window: HWND) {
    let mut set = FLAG_REGISTERED;
    if unsafe { IsWindowVisible(window) }.as_bool() {
        set |= FLAG_VISIBLE;
    }
    if unsafe { IsIconic(window) }.as_bool() {
        set |= FLAG_MINIMIZED;
    }
    if unsafe { GetForegroundWindow() } == window {
        set |= FLAG_FOCUSED;
    }
    update_lifecycle_flags(set, WINDOW_STATE_FLAGS & !set);
}

fn lifecycle_flag_is_set(flag: u8) -> bool {
    (LIFECYCLE_STATE.load(Ordering::Acquire) as u8 & flag) != 0
}

fn set_lifecycle_flag(flag: u8, value: bool) {
    if value {
        update_lifecycle_flags(flag, 0);
    } else {
        update_lifecycle_flags(0, flag);
    }
}

fn update_lifecycle_flags(set: u8, clear: u8) -> bool {
    let mut current = LIFECYCLE_STATE.load(Ordering::Acquire);
    loop {
        let next = next_lifecycle_word(current, set, clear);
        if next == current {
            return false;
        }
        match LIFECYCLE_STATE.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let _ = post_ui_wake();
                return true;
            }
            Err(observed) => current = observed,
        }
    }
}

fn next_lifecycle_word(current: u64, set: u8, clear: u8) -> u64 {
    let current_flags = (current & STATE_FLAGS_MASK) as u8;
    let next_flags = (current_flags | set) & !clear;
    if next_flags == current_flags {
        return current;
    }
    let next_revision = (current >> STATE_REVISION_SHIFT).wrapping_add(1);
    (next_revision << STATE_REVISION_SHIFT) | u64::from(next_flags)
}

fn decode_lifecycle_word(word: u64) -> WindowLifecycleSnapshot {
    let flags = (word & STATE_FLAGS_MASK) as u8;
    WindowLifecycleSnapshot {
        revision: word >> STATE_REVISION_SHIFT,
        registered: flags & FLAG_REGISTERED != 0,
        visible: flags & FLAG_VISIBLE != 0,
        minimized: flags & FLAG_MINIMIZED != 0,
        focused: flags & FLAG_FOCUSED != 0,
        tray_icon_present: flags & FLAG_TRAY_PRESENT != 0,
        close_to_tray_enabled: flags & FLAG_CLOSE_TO_TRAY != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_word_updates_flags_and_revision_atomically() {
        let initial = next_lifecycle_word(0, FLAG_REGISTERED | FLAG_VISIBLE | FLAG_FOCUSED, 0);
        let shown = decode_lifecycle_word(initial);
        assert_eq!(shown.revision, 1);
        assert!(shown.registered);
        assert!(shown.visible);
        assert!(shown.focused);
        assert!(!shown.minimized);

        let hidden = next_lifecycle_word(initial, 0, FLAG_VISIBLE | FLAG_FOCUSED);
        let hidden = decode_lifecycle_word(hidden);
        assert_eq!(hidden.revision, 2);
        assert!(hidden.registered);
        assert!(!hidden.visible);
        assert!(!hidden.focused);
    }

    #[test]
    fn unchanged_lifecycle_state_does_not_advance_revision() {
        let current = next_lifecycle_word(0, FLAG_REGISTERED | FLAG_VISIBLE, 0);
        assert_eq!(
            next_lifecycle_word(current, FLAG_REGISTERED | FLAG_VISIBLE, FLAG_MINIMIZED),
            current
        );
    }

    #[test]
    fn tray_and_policy_are_part_of_the_coherent_snapshot() {
        let word = next_lifecycle_word(0, FLAG_TRAY_PRESENT | FLAG_CLOSE_TO_TRAY, 0);
        let snapshot = decode_lifecycle_word(word);
        assert!(snapshot.tray_icon_present);
        assert!(snapshot.close_to_tray_enabled);
        assert!(!snapshot.registered);
    }

    #[test]
    fn window_teardown_preserves_tray_policy_but_clears_window_state() {
        let word = next_lifecycle_word(
            0,
            WINDOW_STATE_FLAGS | FLAG_TRAY_PRESENT | FLAG_CLOSE_TO_TRAY,
            0,
        );
        let word = next_lifecycle_word(word, 0, WINDOW_STATE_FLAGS | FLAG_TRAY_PRESENT);
        let snapshot = decode_lifecycle_word(word);
        assert!(!snapshot.registered);
        assert!(!snapshot.visible);
        assert!(!snapshot.minimized);
        assert!(!snapshot.focused);
        assert!(!snapshot.tray_icon_present);
        assert!(snapshot.close_to_tray_enabled);
    }

    #[test]
    fn close_to_tray_never_hides_an_unrecoverable_window() {
        assert!(should_hide_on_close(false, true, true, true));
        assert!(!should_hide_on_close(false, true, false, true));
        assert!(!should_hide_on_close(true, true, true, true));
        assert!(!should_hide_on_close(false, true, true, false));
        assert!(!should_hide_on_close(false, false, true, true));
    }

    #[test]
    fn shipping_shortcuts_require_the_exact_modifier_shapes() {
        let ctrl = ShortcutModifiers {
            control: true,
            ..ShortcutModifiers::default()
        };
        let ctrl_shift = ShortcutModifiers {
            control: true,
            shift: true,
            alt: false,
        };
        assert_eq!(
            classify_global_shortcut(0x4b, ctrl),
            Some(GlobalShortcutCommand::TogglePalette)
        );
        for (key, index) in (0x31..=0x36).zip(1..=6) {
            assert_eq!(
                classify_global_shortcut(key, ctrl),
                Some(GlobalShortcutCommand::Navigate(index))
            );
        }
        assert_eq!(
            classify_global_shortcut(0xbf, ctrl),
            Some(GlobalShortcutCommand::ShowHelp)
        );
        assert_eq!(
            classify_global_shortcut(0x6f, ctrl),
            Some(GlobalShortcutCommand::ShowHelp)
        );
        assert_eq!(
            classify_global_shortcut(0x51, ctrl_shift),
            Some(GlobalShortcutCommand::QuickScan)
        );
        assert_eq!(
            classify_global_shortcut(0x46, ctrl_shift),
            Some(GlobalShortcutCommand::FullScan)
        );
        for (key, command) in [
            (0x26, GlobalShortcutCommand::PalettePrevious),
            (0x28, GlobalShortcutCommand::PaletteNext),
            (0x0d, GlobalShortcutCommand::PaletteExecute),
            (0x1b, GlobalShortcutCommand::PaletteClose),
        ] {
            assert_eq!(
                classify_global_shortcut(key, ShortcutModifiers::default()),
                Some(command)
            );
        }

        assert_eq!(
            classify_global_shortcut(0x4b, ShortcutModifiers::default()),
            None
        );
        assert_eq!(classify_global_shortcut(0x4b, ctrl_shift), None);
        assert_eq!(
            classify_global_shortcut(
                0x4b,
                ShortcutModifiers {
                    control: true,
                    shift: false,
                    alt: true,
                }
            ),
            None
        );
        assert_eq!(classify_global_shortcut(0x37, ctrl), None);
    }

    #[test]
    fn shortcut_handoff_preserves_command_and_editable_evidence() {
        for event in [
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::TogglePalette,
                editable_focused: true,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::Navigate(6),
                editable_focused: false,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::ShowHelp,
                editable_focused: true,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::QuickScan,
                editable_focused: false,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::FullScan,
                editable_focused: false,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::PalettePrevious,
                editable_focused: true,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteNext,
                editable_focused: true,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteExecute,
                editable_focused: true,
            },
            GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteClose,
                editable_focused: true,
            },
        ] {
            assert_eq!(
                decode_shortcut_event(encode_shortcut_event(event)),
                Some(event)
            );
        }
        assert_eq!(
            encode_shortcut_event(GlobalShortcutEvent {
                command: GlobalShortcutCommand::Navigate(7),
                editable_focused: false,
            }),
            0
        );
        assert_eq!(decode_shortcut_event(0), None);
    }

    #[test]
    fn shortcut_handoff_preserves_rapid_navigation_then_execution_order() {
        let mut queue = VecDeque::new();
        push_shortcut_event(
            &mut queue,
            encode_shortcut_event(GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteNext,
                editable_focused: true,
            }),
        );
        push_shortcut_event(
            &mut queue,
            encode_shortcut_event(GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteExecute,
                editable_focused: true,
            }),
        );
        assert_eq!(
            decode_shortcut_event(pop_shortcut_event(&mut queue).unwrap_or_default()),
            Some(GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteNext,
                editable_focused: true,
            })
        );
        assert_eq!(
            decode_shortcut_event(pop_shortcut_event(&mut queue).unwrap_or_default()),
            Some(GlobalShortcutEvent {
                command: GlobalShortcutCommand::PaletteExecute,
                editable_focused: true,
            })
        );
        assert_eq!(pop_shortcut_event(&mut queue), None);
    }

    #[test]
    fn shortcut_handoff_is_bounded_and_keeps_the_newest_events() {
        let previous = encode_shortcut_event(GlobalShortcutEvent {
            command: GlobalShortcutCommand::PalettePrevious,
            editable_focused: false,
        });
        let next = encode_shortcut_event(GlobalShortcutEvent {
            command: GlobalShortcutCommand::PaletteNext,
            editable_focused: false,
        });
        let mut queue = VecDeque::new();
        push_shortcut_event(&mut queue, previous);
        for _ in 0..SHORTCUT_QUEUE_CAPACITY {
            push_shortcut_event(&mut queue, next);
        }
        assert_eq!(queue.len(), SHORTCUT_QUEUE_CAPACITY);
        assert_eq!(
            decode_shortcut_event(pop_shortcut_event(&mut queue).unwrap_or_default())
                .map(|event| event.command),
            Some(GlobalShortcutCommand::PaletteNext)
        );
    }

    #[test]
    fn editable_hwnd_classes_are_detected_conservatively() {
        for class_name in ["Edit", "RICHEDIT50W", "ComboBox", "Windows.TextBox"] {
            assert!(class_name_is_editable(class_name), "{class_name}");
        }
        for class_name in [
            "Button",
            "WinUIDesktopWin32WindowClass",
            "Microsoft.UI.Content.DesktopChildSiteBridge",
        ] {
            assert!(!class_name_is_editable(class_name), "{class_name}");
        }
    }
}
