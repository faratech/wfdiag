//! Off-UI worker runtimes and the plumbing they share.
//!
//! Each worker owns one std thread plus a Tokio runtime, accepts commands over
//! a `std::sync::mpsc` channel, and publishes typed events on a second one. A
//! host supplies a [`WorkerWake`] callback so its UI thread can be nudged to
//! drain those events instead of polling, tracks the single in-flight request
//! through an [`ActiveRequestSlot`], and tears a worker down through the
//! bounded `stop_and_join` on each handle. Nothing here depends on a UI
//! framework.

pub mod provider_setup;
pub mod subscription_auth;
pub mod subscription_install;

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use tokio_util::sync::CancellationToken;

/// Thread-safe notification invoked after a worker event is accepted. Native
/// shells use it to schedule one coalesced UI-thread drain; a host with no UI
/// (tests, headless callers) passes a no-op.
pub type WorkerWake = Arc<dyn Fn() + Send + Sync>;

/// A [`WorkerWake`] that does nothing.
#[must_use]
pub fn no_wake() -> WorkerWake {
    Arc::new(|| {})
}

/// Deliver one worker event and, only on success, schedule a single UI wake.
///
/// Returns `false` once the consumer is gone: the event was not delivered, and
/// a component that can no longer receive it is not woken.
pub fn send_worker_event<T>(events: &mpsc::Sender<T>, wake: &WorkerWake, event: T) -> bool {
    if events.send(event).is_err() {
        return false;
    }
    wake();
    true
}

/// The single in-flight request a worker admits, with identity-scoped
/// cancellation.
///
/// Cancellation is deliberately out-of-band: it runs on the caller's thread
/// against the stored token rather than queueing behind the slow HTTP request,
/// vendor CLI, or provider probe it is meant to interrupt. A stale request id
/// can never cancel the current request.
#[derive(Clone, Default)]
pub struct ActiveRequestSlot(Arc<Mutex<Option<(u64, CancellationToken)>>>);

impl std::fmt::Debug for ActiveRequestSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ActiveRequestSlot")
            .field(&self.active_request())
            .finish()
    }
}

impl ActiveRequestSlot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve the slot for `request_id` and return its cancellation token.
    ///
    /// Returns `None` while another request holds the slot, unless `replaces`
    /// names exactly the request that does — the atomic hand-off a retry needs
    /// when the worker has not yet cleared the attempt it just finished.
    #[must_use]
    pub fn register(&self, request_id: u64, replaces: Option<u64>) -> Option<CancellationToken> {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((active_request_id, _)) = slot.as_ref()
            && replaces != Some(*active_request_id)
        {
            return None;
        }
        let cancel = CancellationToken::new();
        *slot = Some((request_id, cancel.clone()));
        Some(cancel)
    }

    /// Release the slot if `request_id` still holds it.
    pub fn clear(&self, request_id: u64) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(active_request_id, _)| *active_request_id == request_id)
        {
            *slot = None;
        }
    }

    /// Cancel `request_id` if it still holds the slot. The reservation is kept
    /// until the request publishes its terminal event.
    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(cancel) = slot
            .as_ref()
            .filter(|(active_request_id, _)| *active_request_id == request_id)
            .map(|(_, cancel)| cancel.clone())
        else {
            return false;
        };
        drop(slot);
        cancel.cancel();
        true
    }

    /// Cancel whatever holds the slot, keeping the reservation.
    pub fn cancel_any(&self) -> bool {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let cancel = slot.as_ref().map(|(_, cancel)| cancel.clone());
        drop(slot);
        if let Some(cancel) = cancel {
            cancel.cancel();
            return true;
        }
        false
    }

    /// Empty the slot and return the token it held, without cancelling it.
    ///
    /// Teardown uses this so the token is cancelled *after* the slot lock is
    /// released: `CancellationToken::cancel` runs registered wakers
    /// synchronously, and a waker that touches this same slot would deadlock
    /// against the guard.
    #[must_use]
    pub fn take(&self) -> Option<CancellationToken> {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.take().map(|(_, cancel)| cancel)
    }

    /// Identity of the request currently holding the slot.
    #[must_use]
    pub fn active_request(&self) -> Option<u64> {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.as_ref().map(|(request_id, _)| *request_id)
    }

    /// Whether no request holds the slot.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.active_request().is_none()
    }
}

/// Join a stopped worker on a detached thread so the caller never blocks.
///
/// `done` (when supplied) receives one message once the join completes, which
/// is how a bounded `stop_and_join` waits without risking an unbounded stall on
/// the UI thread: an in-flight vendor CLI or provider probe that ignores
/// cancellation keeps running on the reaper, not on the caller.
pub fn reap_worker(worker: JoinHandle<()>, done: Option<mpsc::Sender<()>>) {
    let spawned = std::thread::Builder::new()
        .name("wfdiag-ai-worker-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
            if let Some(done) = done {
                let _ = done.send(());
            }
        });
    if spawned.is_err() {
        // Thread creation failed: the worker still exits on its own once its
        // command channel closes, so leaking the handle is the only
        // non-blocking option left.
    }
}

/// Build the single-worker-thread Tokio runtime the request workers use.
///
/// Multi-threaded so a provider task spawned by a request keeps being polled
/// while the command thread waits for the next command.
///
/// # Errors
/// When the runtime's thread or IO/time drivers cannot be created.
pub fn build_worker_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn successful_delivery_notifies_exactly_after_the_value_is_visible() {
        let (sender, receiver) = mpsc::channel();
        let receiver = Mutex::new(receiver);
        let observed = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&observed);
        let wake: WorkerWake = Arc::new(move || {
            let visible = receiver
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_recv()
                .is_ok();
            seen.store(usize::from(visible), Ordering::SeqCst);
        });

        assert!(send_worker_event(&sender, &wake, 42_u8));
        assert_eq!(observed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn disconnected_delivery_does_not_notify() {
        let (sender, receiver) = mpsc::channel::<u8>();
        drop(receiver);
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let wake: WorkerWake = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        assert!(!send_worker_event(&sender, &wake, 7));
        assert_eq!(woken.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn busy_slot_fails_closed_and_cancellation_is_identity_scoped() {
        let slot = ActiveRequestSlot::new();
        let first = slot
            .register(21, None)
            .expect("an idle slot admits a request");
        assert!(slot.register(22, None).is_none());
        assert!(!slot.cancel(22), "a stale id cannot cancel the current one");
        assert!(!first.is_cancelled());

        assert!(slot.cancel(21));
        assert!(first.is_cancelled());
        assert_eq!(
            slot.active_request(),
            Some(21),
            "the slot stays reserved until its terminal event"
        );

        slot.clear(22);
        assert!(!slot.is_idle());
        slot.clear(21);
        assert!(slot.is_idle());
    }

    #[test]
    fn a_named_replacement_takes_over_the_slot_atomically() {
        let slot = ActiveRequestSlot::new();
        let _first = slot.register(7, None).expect("register");
        assert!(slot.register(8, Some(6)).is_none());
        let second = slot.register(8, Some(7)).expect("named replacement");
        assert_eq!(slot.active_request(), Some(8));

        assert!(slot.cancel_any());
        assert!(second.is_cancelled());
        let taken = slot.take().expect("take returns the held token");
        assert!(taken.is_cancelled());
        assert!(slot.is_idle());
        assert!(!slot.cancel_any());
        assert!(slot.take().is_none());
    }
}
