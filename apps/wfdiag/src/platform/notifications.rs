//! Best-effort scan-completion toast.
//!
//! The AUMID matches the Store package identity so the packaged shell's
//! toasts are attributed correctly. An unpackaged candidate has no
//! Start-menu shortcut backing that AUMID, and Windows drops the toast
//! silently in that case — the same graceful degradation the shipping
//! plugin path shows when permission is missing. Failures are reported to
//! the caller and never block the scan-completion flow.
//!
//! #206: toasts run on ONE lazily started, named worker thread rather than a
//! fresh detached `std::thread::spawn` per scan, and a failure is no longer
//! discarded by a `let _ =` at the call site. The worker parks the first
//! failure here and wakes the UI thread, which surfaces it once in the status
//! line — a user who has notifications switched on and never sees one now
//! learns why.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

/// One queued toast.
struct ToastRequest {
    collected: usize,
    errors: usize,
}

/// The worker's inbox, or the reason it could not be started.
static TOAST_QUEUE: OnceLock<Result<SyncSender<ToastRequest>, String>> = OnceLock::new();

/// First worker-side failure not yet shown to the user.
static PENDING_FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// Lock-free guard for [`take_toast_failure`], which the UI thread calls on
/// every native wake.
static HAS_PENDING_FAILURE: AtomicBool = AtomicBool::new(false);

/// Bound on queued toasts. A stuck notifier must not let repeated scans grow
/// an unbounded backlog of stale "scan complete" messages.
const TOAST_QUEUE_DEPTH: usize = 4;

/// Queue the scan-completion toast on the shared notification thread.
///
/// Returns immediately; the WinRT work happens on the worker.
///
/// # Errors
/// When the worker thread could not be started or its queue is saturated.
/// A failure raised by the toast itself arrives later through
/// [`take_toast_failure`].
pub fn request_scan_complete_toast(collected: usize, errors: usize) -> Result<(), String> {
    match TOAST_QUEUE.get_or_init(start_toast_worker) {
        Ok(sender) => sender
            .try_send(ToastRequest { collected, errors })
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    "Notifications are backing up and this one was dropped".to_string()
                }
                TrySendError::Disconnected(_) => {
                    "The notification worker stopped unexpectedly".to_string()
                }
            }),
        Err(error) => Err(error.clone()),
    }
}

/// Take the first unreported toast failure, if any.
///
/// The UI thread polls this from its native-wake drain; the worker posts a
/// wake as soon as it records one, so the status line updates promptly rather
/// than waiting for the next scan.
#[must_use]
pub fn take_toast_failure() -> Option<String> {
    if !HAS_PENDING_FAILURE.swap(false, Ordering::AcqRel) {
        return None;
    }
    PENDING_FAILURE.lock().ok().and_then(|mut slot| slot.take())
}

fn start_toast_worker() -> Result<SyncSender<ToastRequest>, String> {
    let (sender, receiver) = sync_channel::<ToastRequest>(TOAST_QUEUE_DEPTH);
    std::thread::Builder::new()
        .name("wfdiag-reactor-toast".to_string())
        .spawn(move || {
            // Parks on the channel between scans; the process exiting is what
            // ends it, exactly as the per-scan detached threads ended before.
            for request in receiver {
                if let Err(error) = show_scan_complete_toast(request.collected, request.errors) {
                    record_failure(error);
                }
            }
        })
        .map(|_handle| sender)
        .map_err(|error| format!("Could not start the notification worker: {error}"))
}

/// Hold at most one unreported failure. A repeated failure is almost always
/// the same cause, and the shell latches its own "already said this" flag, so
/// overwriting a pending message would only churn the status line.
fn record_failure(error: String) {
    let Ok(mut slot) = PENDING_FAILURE.lock() else {
        return;
    };
    if slot.is_some() {
        return;
    }
    *slot = Some(error);
    drop(slot);
    HAS_PENDING_FAILURE.store(true, Ordering::Release);
    let _ = super::window::post_ui_wake();
}

/// Show the scan-completion toast with the Store 2.5.8 wording.
///
/// # Errors
/// When WinRT activation or the notifier call fails (reported, never fatal).
fn show_scan_complete_toast(collected: usize, errors: usize) -> Result<(), String> {
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
