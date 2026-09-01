//! Single-instance handoff.
//!
//! `Local\<identifier>` mutex decides primary/secondary at startup, before
//! any WinUI work; a secondary signals a named activation event and exits.
//! The primary registers the event with the Windows thread pool and posts one
//! coalesced UI wake, matching the single-instance plugin's focus behavior
//! without a permanent polling thread. The Win32 calls are localized here
//! with explicit SAFETY notes.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use super::window;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL, HWND, LPARAM, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentProcessId, INFINITE, OpenEventW,
    RegisterWaitForSingleObject, ResetEvent, SetEvent, UnregisterWait, WT_EXECUTEDEFAULT,
    WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SW_SHOW,
    SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PCWSTR};

/// HANDLE is a raw pointer; the event handle is process-lifetime and only
/// ever used for wait/signal calls, which are thread-safe.
struct EventHandle(HANDLE);
unsafe impl Send for EventHandle {}
unsafe impl Sync for EventHandle {}

static ACTIVATION_EVENT: OnceLock<EventHandle> = OnceLock::new();
static ACTIVATION_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVATION_WAIT_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Keep the primary's named-mutex handle alive for the process lifetime.
/// Secondary probes close their temporary handle immediately, which is also
/// what lets an accepted elevated relaunch observe the old process exiting.
struct MutexHandle(HANDLE);
unsafe impl Send for MutexHandle {}
unsafe impl Sync for MutexHandle {}

impl Drop for MutexHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

static INSTANCE_MUTEX: OnceLock<MutexHandle> = OnceLock::new();

/// Process-local identity of the Reactor-owned top-level window. HWND values
/// are process-local opaque handles, so an integer atomic is sufficient for
/// publication across the UI thread and the activation watcher. Every read is
/// revalidated with `IsWindow` + process ownership before use; Windows may
/// recycle a destroyed handle value.
static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Owning thread of the window in [`MAIN_WINDOW`], captured at registration.
///
/// #220: an HWND value alone is a weak identity — Windows recycles handle
/// values, and the old validation accepted any live, non-tool window of this
/// process. Pinning the creating thread adds a second dimension that a
/// recycled handle on some other GUI thread cannot satisfy.
static MAIN_WINDOW_THREAD: AtomicU32 = AtomicU32::new(0);

/// The outcome of the startup single-instance acquisition.
pub enum SingleInstanceDecision {
    /// This process owns the mutex; `InstanceWatch` observes activate
    /// requests from later launches.
    Primary(InstanceWatch),
    /// The lock could not be created at all — not "someone else owns it", but
    /// an SDDL or `CreateMutexW` failure that leaves ownership unknowable.
    /// The process runs as primary anyway and the caller must tell the user
    /// why (#188); `reason` is the Windows-level detail.
    PrimaryWithoutLock {
        watch: InstanceWatch,
        reason: String,
    },
    /// Another instance is already running and has been signalled to come to
    /// the foreground. The caller should exit.
    Secondary,
}

/// Acquire the single-instance mutex and prepare the activation event.
#[must_use]
pub fn acquire(identifier: &str) -> SingleInstanceDecision {
    match try_claim_mutex(identifier) {
        MutexClaim::Occupied => {
            signal_primary(identifier);
            SingleInstanceDecision::Secondary
        }
        MutexClaim::Acquired => initialize_primary(identifier),
        // #188: an indeterminate mutex result is NOT ownership, but it is also
        // not evidence that another instance exists. Treating it as
        // "secondary" made the app silently unlaunchable — no window, no
        // message, exit code 0. Run as primary and let `main` say so; the
        // failure mode is at worst a duplicate window, never a dead launch.
        MutexClaim::Failed(reason) => match initialize_primary(identifier) {
            SingleInstanceDecision::Primary(watch) => {
                SingleInstanceDecision::PrimaryWithoutLock { watch, reason }
            }
            other => other,
        },
    }
}

/// Wait for the current instance to exit during an administrator handoff.
///
/// The elevated child starts before `ShellExecuteExW` returns to the original
/// process. A normal single-instance check would therefore classify it as a
/// secondary and exit before the original can close. This bounded wait keeps
/// the elevated child alive until the original releases its process-owned
/// mutex, without permitting two interactive instances.
#[must_use]
pub fn acquire_for_relaunch(
    identifier: &str,
    timeout: std::time::Duration,
) -> SingleInstanceDecision {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match try_claim_mutex(identifier) {
            MutexClaim::Acquired => return initialize_primary(identifier),
            // Same reasoning as `acquire` (#188). The unelevated original is
            // already on its way out, so the duplicate-window risk this once
            // guarded against does not apply to the hand-off.
            MutexClaim::Failed(reason) => match initialize_primary(identifier) {
                SingleInstanceDecision::Primary(watch) => {
                    return SingleInstanceDecision::PrimaryWithoutLock { watch, reason };
                }
                other => return other,
            },
            MutexClaim::Occupied if std::time::Instant::now() >= deadline => {
                // #189: the original copy never let go. This used to end the
                // elevated process with no output at all, so an approved UAC
                // prompt produced nothing the user could see.
                super::crash::show_startup_warning(&format!(
                    "WindowsForum Diagnostics could not take over from the copy that is \
                     already running.\n\n\
                     It stayed open for the full {} second hand-off, so this elevated \
                     copy is closing to avoid running two instances at once.\n\n\
                     Close the running WindowsForum Diagnostics window and choose \
                     \"Restart as administrator\" again.",
                    timeout.as_secs()
                ));
                signal_primary(identifier);
                return SingleInstanceDecision::Secondary;
            }
            MutexClaim::Occupied => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(50)));
            }
        }
    }
}

enum MutexClaim {
    Acquired,
    Occupied,
    /// The claim could not be evaluated; carries the user-facing detail.
    Failed(String),
}

fn try_claim_mutex(identifier: &str) -> MutexClaim {
    let mutex_name = wide(&format!("Local\\{identifier}-single-instance"));
    let security = match NamedObjectSecurity::for_interactive_session() {
        Ok(security) => security,
        Err(error) => return MutexClaim::Failed(error),
    };
    // SAFETY: the name and security descriptor are valid for the duration of
    // CreateMutexW. GetLastError is cleared/read immediately around the call.
    unsafe { windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0)) };
    let handle = match unsafe {
        CreateMutexW(
            Some(&security.attributes),
            false,
            PCWSTR(mutex_name.as_ptr()),
        )
    } {
        Ok(handle) => handle,
        Err(error) => {
            return MutexClaim::Failed(format!("Windows refused the instance lock: {error}"));
        }
    };
    let already_exists =
        unsafe { windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS };
    if already_exists {
        // This probe must not keep the old process's named mutex alive. The
        // elevated wait loop retries with a fresh, promptly closed handle.
        unsafe {
            let _ = CloseHandle(handle);
        }
        MutexClaim::Occupied
    } else {
        if let Err(extra) = INSTANCE_MUTEX.set(MutexHandle(handle)) {
            drop(extra);
        }
        MutexClaim::Acquired
    }
}

fn initialize_primary(identifier: &str) -> SingleInstanceDecision {
    let mut wait_handle = None;
    let event_name = wide(&format!("Local\\{identifier}-activate"));
    if let Ok(security) = NamedObjectSecurity::for_interactive_session()
        && let Ok(event) = unsafe {
            // Manual reset: the signal must survive until either the thread-pool
            // callback or the polling fallback claims it. An auto-reset event can
            // be consumed by the pool's internal wait while its callback is still
            // queued, losing the activation entirely (the secondary has already
            // exited by then).
            CreateEventW(
                Some(&security.attributes),
                true,
                false,
                PCWSTR(event_name.as_ptr()),
            )
        }
    {
        let _ = ACTIVATION_EVENT.set(EventHandle(event));
        let mut registered = HANDLE::default();
        // SAFETY: the activation event remains process-owned in
        // `ACTIVATION_EVENT`. The callback captures no borrowed context and
        // the returned wait handle is unregistered by `InstanceWatch::drop`.
        if unsafe {
            RegisterWaitForSingleObject(
                &mut registered,
                event,
                Some(on_activation_event),
                None,
                INFINITE,
                WT_EXECUTEDEFAULT,
            )
        }
        .is_ok()
        {
            ACTIVATION_WAIT_REGISTERED.store(true, Ordering::Release);
            wait_handle = Some(registered);
        }
    }
    SingleInstanceDecision::Primary(InstanceWatch { wait_handle })
}

unsafe extern "system" fn on_activation_event(_context: *mut c_void, timed_out: bool) {
    if timed_out {
        return;
    }
    let Some(EventHandle(event)) = ACTIVATION_EVENT.get() else {
        return;
    };
    // Re-arm before publishing: a SetEvent arriving after this reset fires a
    // fresh callback, while a signal consumed here is published below. Reset
    // and publish are both idempotent, so concurrent callbacks are harmless.
    // SAFETY: event is a valid process-owned handle.
    let _ = unsafe { ResetEvent(*event) };
    ACTIVATION_PENDING.store(true, Ordering::Release);
    let _ = window::post_ui_wake();
}

fn signal_primary(identifier: &str) {
    let event_name = wide(&format!("Local\\{identifier}-activate"));
    // Signal the primary's activation event. Create it if the primary has not
    // made it yet (the narrow startup race); the short-lived secondary keeps
    // that handle alive until it exits.
    let opened = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name.as_ptr())) };
    match opened {
        Ok(handle) => unsafe {
            let _ = SetEvent(handle);
            let _ = CloseHandle(handle);
        },
        Err(_) => {
            if let Ok(security) = NamedObjectSecurity::for_interactive_session()
                && let Ok(handle) = unsafe {
                    // Manual reset, matching the primary's event: see
                    // `initialize_primary`.
                    CreateEventW(
                        Some(&security.attributes),
                        true,
                        false,
                        PCWSTR(event_name.as_ptr()),
                    )
                }
            {
                unsafe {
                    let _ = SetEvent(handle);
                    // #220: close it. The previous code leaked this handle on
                    // purpose, hoping to keep the event object alive until the
                    // primary opened it — but the only caller that reached
                    // here exited within microseconds anyway, so process
                    // teardown closed it regardless and the "protection" was
                    // imaginary. Now that a lock failure can continue as
                    // primary (#188), a stray handle to the activation event
                    // would be a genuine process-lifetime leak in a process
                    // that keeps running.
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

/// Owns the self-relative descriptor referenced by `attributes`.
///
/// `Local\` already scopes the object to the current Terminal Services
/// session. Within that boundary the DACL admits interactive tokens (plus
/// SYSTEM), and the mandatory label is medium integrity. This covers both a
/// same-user split token and over-the-shoulder UAC, where `runas` uses a
/// different administrator SID, without exposing the object to other sessions.
struct NamedObjectSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl NamedObjectSecurity {
    fn for_interactive_session() -> Result<Self, String> {
        let sddl = named_object_sddl();
        let sddl = wide(sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SDDL revision 1 is the only documented revision accepted by this
        // conversion API.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|error| format!("Could not create named-object security descriptor: {error}"))?;
        if descriptor.is_invalid() {
            return Err("Windows returned an empty named-object security descriptor".to_string());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .unwrap_or(u32::MAX),
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: BOOL(0),
            },
        })
    }
}

impl Drop for NamedObjectSecurity {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.descriptor.0)));
        }
    }
}

fn named_object_sddl() -> &'static str {
    // CreateMutexW/CreateEventW request all access when opening an existing
    // named object, so the ACE must grant GA. IU is session-scoped in
    // combination with the Local\ namespace and survives alternate-credential
    // elevation; SY keeps service-side diagnostics possible.
    "D:P(A;;GA;;;IU)(A;;GA;;;SY)S:(ML;;NW;;;ME)"
}

/// True when a secondary launch requested activation.
///
/// Claims the signal: either the thread-pool callback published it (and reset
/// the event) or this poll observes the still-signaled manual-reset event
/// directly. The event is reset here after a successful claim so the poll
/// cannot report the same signal twice. A racing callback may also publish
/// `ACTIVATION_PENDING` for the same signal, making one subsequent poll return
/// true again — a harmless duplicate window activation, never a lost one.
#[must_use]
pub fn activation_requested() -> bool {
    if ACTIVATION_PENDING.swap(false, Ordering::AcqRel) {
        return true;
    }
    let Some(EventHandle(event)) = ACTIVATION_EVENT.get() else {
        return false;
    };
    // SAFETY: event is process-owned for the primary's lifetime.
    if unsafe { WaitForSingleObject(*event, 0) } == WAIT_OBJECT_0 {
        unsafe {
            let _ = ResetEvent(*event);
        }
        return true;
    }
    false
}

/// Whether Windows thread-pool delivery replaced the legacy polling path.
#[must_use]
pub fn activation_wake_registered() -> bool {
    ACTIVATION_WAIT_REGISTERED.load(Ordering::Acquire)
}

/// Own the Windows thread-pool registration for the activation event.
///
/// No dedicated thread is created by the application. Dropping this guard
/// unregisters future callbacks; any callback already executing only touches
/// process-lifetime atomics and may safely finish.
#[derive(Default)]
pub struct InstanceWatch {
    wait_handle: Option<HANDLE>,
}

impl Drop for InstanceWatch {
    fn drop(&mut self) {
        if let Some(wait_handle) = self.wait_handle.take() {
            // SAFETY: this is the exact wait returned during primary
            // initialization. The callback owns no borrowed context, so an
            // already-running invocation can finish independently.
            unsafe {
                let _ = UnregisterWait(wait_handle);
            }
        }
        ACTIVATION_WAIT_REGISTERED.store(false, Ordering::Release);
    }
}

/// Register the exact Reactor-owned top-level window.
///
/// `window::install` calls this as soon as Reactor exposes the HWND.
/// Keeping the handle independent of visibility is essential: a window hidden
/// by close-to-tray is absent from a visible-window enumeration but remains the
/// same valid window that tray and single-instance activation must restore.
///
/// Returns false for a stale/foreign/tool/system HWND and leaves the prior
/// valid registration untouched.
#[must_use]
pub fn register_main_window(window: HWND) -> bool {
    let Some(thread) = main_window_candidate_thread(window) else {
        return false;
    };
    // Publish the owning thread first so any reader that observes the new
    // HWND can already observe the thread id it has to match (#220).
    MAIN_WINDOW_THREAD.store(thread, Ordering::Release);
    MAIN_WINDOW.store(hwnd_to_raw(window), Ordering::Release);
    true
}

/// Clear the cached window only when it still names `window`.
///
/// Called from the subclass's `WM_NCDESTROY` path so a late teardown from an
/// old window can never clear a newly registered replacement window.
pub fn unregister_main_window(window: HWND) {
    let raw = hwnd_to_raw(window);
    if MAIN_WINDOW
        .compare_exchange(raw, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        MAIN_WINDOW_THREAD.store(0, Ordering::Release);
    }
}

/// Whether the atomic registration still names this exact HWND.
///
/// Unlike [`registered_main_window_hwnd`], this does not revalidate a window
/// that is already inside `WM_NCDESTROY`; it is a generation comparison used
/// solely to keep a late old-window teardown from clearing successor state.
#[must_use]
pub(crate) fn is_registered_main_window(window: HWND) -> bool {
    MAIN_WINDOW.load(Ordering::Acquire) == hwnd_to_raw(window)
}

/// Return the registered main window even while it is hidden or minimized.
///
/// Before Reactor's window hook is installed, fall back to a one-time
/// top-level enumeration and cache the result. Visible candidates are
/// preferred during that bootstrap search, but visibility is never a validity
/// requirement and is never used for subsequent lookups.
///
/// #220: the enumeration is a last resort, never a shortcut. The registered
/// HWND always wins, and the search itself now ranks candidates by window
/// class instead of taking whichever window Windows happened to hand back
/// first — a process gets top-level IME/marshal windows for free on every GUI
/// thread, and those passed the old process+style filter.
#[must_use]
pub fn main_window_hwnd() -> Option<HWND> {
    if let Some(window) = registered_main_window() {
        return Some(window);
    }

    struct Collector {
        current_process: u32,
        /// Highest-ranked candidate so far: WinUI class beats visibility, and
        /// visibility only breaks ties between windows of the same class.
        best: Option<(u8, HWND)>,
    }
    unsafe extern "system" fn on_window(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam points at our Collector for the duration of the
        // enumeration; the OS hands back the exact pointer we passed.
        let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
        let mut process_id = 0_u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id != collector.current_process {
            return BOOL(1);
        }
        // Skip tool windows (transient popups); prefer the main app window.
        let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        if is_tool_window_style(ex_style) {
            return BOOL(1);
        }
        // Owned windows are dialogs and popups of some other window of ours
        // (the crash MessageBox, a ContentDialog host); the shell's top-level
        // window is unowned.
        if !unsafe { GetWindow(window, GW_OWNER) }
            .unwrap_or_default()
            .0
            .is_null()
        {
            return BOOL(1);
        }
        let class_name = window_class_name(window);
        if class_is_transient_system_window(&class_name) {
            return BOOL(1);
        }

        let rank = main_window_rank(
            class_is_reactor_main_window(&class_name),
            unsafe { IsWindowVisible(window) }.as_bool(),
        );
        if collector.best.is_none_or(|(best, _)| rank > best) {
            collector.best = Some((rank, window));
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
    if let Some((_, window)) = collector.best {
        // The enumeration already applied the same ownership/style/class
        // filters; registration revalidates the handle in case destruction
        // raced it, and records the owning thread.
        if register_main_window(window) {
            return Some(window);
        }
    }
    None
}

/// Restore and foreground this process's registered main window.
///
/// `SetForegroundWindow` is best-effort because Windows foreground-lock
/// policy may reject focus stealing; restoring/showing the window remains
/// useful in that case.
pub fn activate_main_window() {
    let _ = try_activate_main_window();
}

/// Restore and foreground the registered main window, reporting whether a
/// valid window was available. This is useful to focused lifecycle tests and
/// callers that need to surface an activation failure; the compatibility
/// wrapper above preserves the existing fire-and-forget component API.
#[must_use]
pub fn try_activate_main_window() -> bool {
    if let Some(window) = main_window_hwnd() {
        unsafe {
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            } else {
                let _ = ShowWindow(window, SW_SHOW);
            }
            let _ = SetForegroundWindow(window);
        }
        true
    } else {
        false
    }
}

fn registered_main_window() -> Option<HWND> {
    let raw = MAIN_WINDOW.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    let expected_thread = MAIN_WINDOW_THREAD.load(Ordering::Acquire);
    let window = hwnd_from_raw(raw);
    // #220: matching the handle value is not enough — Windows recycles them.
    // The window must still be ours AND still live on the thread it was
    // registered from.
    if main_window_candidate_thread(window) == Some(expected_thread) && expected_thread != 0 {
        Some(window)
    } else {
        // Compare-exchange avoids clearing a replacement registered between
        // the load and this stale-handle validation.
        unregister_main_window(window);
        None
    }
}

/// Return only an already-registered Reactor window.
///
/// Unlike [`main_window_hwnd`], this never performs desktop enumeration, so
/// backend worker wakeups can call it without turning an early startup burst
/// into repeated cross-process window walks.
#[must_use]
pub(crate) fn registered_main_window_hwnd() -> Option<HWND> {
    registered_main_window()
}

/// Validate `window` as this process's shell window, returning its owning
/// thread id.
///
/// #220: the checks are process ownership, tool-window style, AND window
/// class. The class filter is what keeps the per-GUI-thread IME/marshal
/// windows Windows creates for us out of the candidate set; they are
/// top-level, unowned, non-tool windows of this very process, so nothing
/// cheaper distinguishes them.
fn main_window_candidate_thread(window: HWND) -> Option<u32> {
    if window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
        return None;
    }
    let mut process_id = 0_u32;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    if thread_id == 0 || process_id != unsafe { GetCurrentProcessId() } {
        return None;
    }
    let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
    if is_tool_window_style(ex_style) {
        return None;
    }
    if class_is_transient_system_window(&window_class_name(window)) {
        return None;
    }
    Some(thread_id)
}

fn is_tool_window_style(ex_style: isize) -> bool {
    (ex_style & WS_EX_TOOLWINDOW.0 as isize) != 0
}

/// The registered class of `window`, or an empty string when Windows refuses.
///
/// An empty name is deliberately *not* disqualifying: an unknown class still
/// passes the transient-window filter, so a future Reactor class rename
/// degrades to the old process+style behaviour instead of losing the window.
fn window_class_name(window: HWND) -> String {
    let mut buffer = [0_u16; 128];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or_default()])
}

/// WinUI 3 desktop windows — the shell's own top-level window included — all
/// register this class.
const REACTOR_MAIN_WINDOW_CLASS: &str = "WinUIDesktopWin32WindowClass";

fn class_is_reactor_main_window(class_name: &str) -> bool {
    class_name
        .trim()
        .eq_ignore_ascii_case(REACTOR_MAIN_WINDOW_CLASS)
}

/// Top-level windows Windows creates on this process's behalf, never the app.
fn class_is_transient_system_window(class_name: &str) -> bool {
    let class_name = class_name.trim().to_ascii_lowercase();
    class_name == "ime"
        || class_name == "default ime"
        || class_name == "msctfime ui"
        || class_name == "cicmarshalwndclass"
        || class_name == "tooltips_class32"
        || class_name.starts_with("xaml_windowedpopup")
}

/// Rank a bootstrap-search candidate: class evidence dominates visibility.
fn main_window_rank(reactor_class: bool, visible: bool) -> u8 {
    u8::from(reactor_class) * 2 + u8::from(visible)
}

fn hwnd_to_raw(window: HWND) -> isize {
    window.0 as isize
}

fn hwnd_from_raw(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_object_strings_are_nul_terminated_without_losing_unicode() {
        let value = wide("Local\\wfdiag-测试");
        assert_eq!(value.last(), Some(&0));
        assert_eq!(
            String::from_utf16_lossy(&value[..value.len() - 1]),
            "Local\\wfdiag-测试"
        );
    }

    #[test]
    fn named_objects_are_interactive_session_scoped_and_medium_integrity() {
        assert_eq!(
            named_object_sddl(),
            "D:P(A;;GA;;;IU)(A;;GA;;;SY)S:(ML;;NW;;;ME)"
        );
    }

    #[test]
    fn tool_window_filter_uses_ws_ex_toolwindow_not_topmost() {
        const WS_EX_TOPMOST_VALUE: isize = 0x0000_0008;
        assert!(!is_tool_window_style(WS_EX_TOPMOST_VALUE));
        assert!(is_tool_window_style(WS_EX_TOOLWINDOW.0 as isize));
    }

    #[test]
    fn hwnd_integer_round_trip_preserves_the_opaque_value() {
        let raw = 0x1234_isize;
        assert_eq!(hwnd_to_raw(hwnd_from_raw(raw)), raw);
    }

    #[test]
    fn per_thread_system_windows_are_never_main_window_candidates() {
        for rejected in [
            "IME",
            "Default IME",
            "MSCTFIME UI",
            "CicMarshalWndClass",
            "tooltips_class32",
            "Xaml_WindowedPopupClass",
            "  default ime  ",
        ] {
            assert!(class_is_transient_system_window(rejected), "{rejected:?}");
            assert!(!class_is_reactor_main_window(rejected), "{rejected:?}");
        }
        for accepted in [REACTOR_MAIN_WINDOW_CLASS, "", "SomeFutureReactorClass"] {
            assert!(!class_is_transient_system_window(accepted), "{accepted:?}");
        }
    }

    #[test]
    fn reactor_class_match_is_case_insensitive_but_exact() {
        assert!(class_is_reactor_main_window("WinUIDesktopWin32WindowClass"));
        assert!(class_is_reactor_main_window(
            " winuidesktopwin32windowclass "
        ));
        assert!(!class_is_reactor_main_window(
            "WinUIDesktopWin32WindowClass2"
        ));
        assert!(!class_is_reactor_main_window(""));
    }

    #[test]
    fn window_class_outranks_visibility_in_the_bootstrap_search() {
        // A hidden Reactor window must still beat a visible unknown window:
        // close-to-tray hides the real window, and the enumeration is exactly
        // where that used to pick the wrong one (#220).
        assert!(main_window_rank(true, false) > main_window_rank(false, true));
        assert!(main_window_rank(true, true) > main_window_rank(true, false));
        assert!(main_window_rank(false, true) > main_window_rank(false, false));
    }
}
