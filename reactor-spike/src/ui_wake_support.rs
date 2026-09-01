//! Process-wide, UI-framework-neutral notification hook for Reactor workers.
//!
//! Worker modules are also built by portable unit-test targets, so they must
//! not call Win32 directly. The Windows executable installs one callback that
//! posts a coalesced native window message; tests leave the hook uninstalled
//! and event delivery remains a no-op beyond the channel send itself.

use std::sync::mpsc::{SendError, Sender};
use std::sync::{Arc, OnceLock};

type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

static UI_WAKE: OnceLock<WakeCallback> = OnceLock::new();

/// Install the process-wide native wake callback.
///
/// Reactor owns one application component for the process lifetime, so the
/// callback is intentionally single-assignment.
pub fn install(callback: impl Fn() + Send + Sync + 'static) -> Result<(), &'static str> {
    UI_WAKE
        .set(Arc::new(callback))
        .map_err(|_| "the Reactor UI wake callback is already installed")
}

/// Notify the UI that at least one worker channel can be drained.
pub fn notify() {
    if let Some(callback) = UI_WAKE.get() {
        callback();
    }
}

/// Extension used by Reactor-specific worker channels. Successful delivery
/// schedules one coalesced native UI wake; failed sends preserve the standard
/// channel error and do not wake a component that can no longer receive it.
pub trait NotifySenderExt<T> {
    fn send_and_wake(&self, event: T) -> Result<(), SendError<T>>;
}

impl<T> NotifySenderExt<T> for Sender<T> {
    fn send_and_wake(&self, event: T) -> Result<(), SendError<T>> {
        send_and_notify(self, event, notify)
    }
}

fn send_and_notify<T>(
    sender: &Sender<T>,
    event: T,
    notifier: impl FnOnce(),
) -> Result<(), SendError<T>> {
    sender.send(event)?;
    notifier();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::mpsc;

    #[test]
    fn successful_delivery_notifies_exactly_after_the_value_is_visible() {
        let (sender, receiver) = mpsc::channel();
        let observed = Cell::new(None);

        send_and_notify(&sender, 42_u8, || observed.set(receiver.try_recv().ok())).unwrap();

        assert_eq!(observed.get(), Some(42));
    }

    #[test]
    fn disconnected_delivery_preserves_the_value_and_does_not_notify() {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let notified = Cell::new(false);

        let error = send_and_notify(&sender, String::from("pending"), || notified.set(true))
            .expect_err("a disconnected channel must reject the event");

        assert_eq!(error.0, "pending");
        assert!(!notified.get());
    }
}
