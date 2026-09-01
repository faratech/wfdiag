//! Deadlined worker replies, and the only polling thread in the crate.
//!
//! Several engine crates answer on a `tokio::sync::oneshot`. Awaiting one
//! needs an async context the host may not have, and the native shell's
//! workaround — a background task per request that slept 50 ms in a loop —
//! multiplied threads and still hung when a worker never answered.
//!
//! [`PendingReplies`] instead keeps every outstanding reply in one list that
//! [`crate::AppService::drain`] polls, and gives each a deadline: a reply that
//! never arrives is reported as a typed timeout instead of hanging.
//! [`ReplyWatcher`] is one thread that wakes the host every 50 ms *only while*
//! something is outstanding, and blocks on a condition variable otherwise.

use crate::command::WorkerKind;
use crate::event::EventQueue;
use crate::ids::RequestId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// Why a reply produced no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyFailure {
    /// The worker dropped the sender without answering.
    WorkerStopped,
    /// The deadline elapsed first.
    TimedOut,
}

impl std::fmt::Display for ReplyFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkerStopped => formatter.write_str("the worker stopped before replying"),
            Self::TimedOut => formatter.write_str("the worker did not reply in time"),
        }
    }
}

trait PendingReply<M> {
    fn worker(&self) -> WorkerKind;
    fn request(&self) -> RequestId;
    fn deadline(&self) -> Instant;
    fn poll(&mut self) -> Option<M>;
    fn expire(&mut self) -> Option<M>;
}

struct OneshotReply<T, M, F> {
    worker: WorkerKind,
    request: RequestId,
    deadline: Instant,
    receiver: oneshot::Receiver<T>,
    map: Option<F>,
    marker: std::marker::PhantomData<fn() -> M>,
}

impl<T, M, F> PendingReply<M> for OneshotReply<T, M, F>
where
    F: FnOnce(Result<T, ReplyFailure>) -> M,
{
    fn worker(&self) -> WorkerKind {
        self.worker
    }

    fn request(&self) -> RequestId {
        self.request
    }

    fn deadline(&self) -> Instant {
        self.deadline
    }

    fn poll(&mut self) -> Option<M> {
        match self.receiver.try_recv() {
            Ok(value) => self.map.take().map(|map| map(Ok(value))),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => self
                .map
                .take()
                .map(|map| map(Err(ReplyFailure::WorkerStopped))),
        }
    }

    fn expire(&mut self) -> Option<M> {
        self.map.take().map(|map| map(Err(ReplyFailure::TimedOut)))
    }
}

/// One elapsed deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReplyTimeout {
    /// Which worker owed the answer.
    pub(crate) worker: WorkerKind,
    /// The request that expired.
    pub(crate) request: RequestId,
}

/// Everything the polled worker replies produced in one pass.
pub(crate) struct ReplyBatch<M> {
    /// The mapped messages, answers and failures alike.
    pub(crate) messages: Vec<M>,
    /// The deadlines that elapsed in this pass.
    pub(crate) timeouts: Vec<ReplyTimeout>,
}

/// Every outstanding worker reply, with its deadline.
pub(crate) struct PendingReplies<M> {
    entries: Vec<Box<dyn PendingReply<M>>>,
    timeout: Duration,
}

impl<M> std::fmt::Debug for PendingReplies<M> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingReplies")
            .field("outstanding", &self.entries.len())
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl<M> PendingReplies<M> {
    pub(crate) const fn new(timeout: Duration) -> Self {
        Self {
            entries: Vec::new(),
            timeout,
        }
    }

    /// Register one reply. `map` turns the answer — or the failure — into the
    /// message the service guards and translates.
    pub(crate) fn register<T, F>(
        &mut self,
        worker: WorkerKind,
        request: RequestId,
        receiver: oneshot::Receiver<T>,
        map: F,
    ) where
        T: 'static,
        M: 'static,
        F: FnOnce(Result<T, ReplyFailure>) -> M + 'static,
    {
        self.entries.push(Box::new(OneshotReply {
            worker,
            request,
            deadline: Instant::now() + self.timeout,
            receiver,
            map: Some(map),
            marker: std::marker::PhantomData,
        }));
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Poll every outstanding reply once.
    pub(crate) fn poll(&mut self, now: Instant) -> ReplyBatch<M> {
        let mut messages = Vec::new();
        let mut timeouts = Vec::new();
        let mut retained: Vec<Box<dyn PendingReply<M>>> = Vec::with_capacity(self.entries.len());
        for mut entry in self.entries.drain(..) {
            if let Some(message) = entry.poll() {
                messages.push(message);
                continue;
            }
            if now >= entry.deadline() {
                timeouts.push(ReplyTimeout {
                    worker: entry.worker(),
                    request: entry.request(),
                });
                if let Some(message) = entry.expire() {
                    messages.push(message);
                }
                continue;
            }
            retained.push(entry);
        }
        self.entries = retained;
        ReplyBatch { messages, timeouts }
    }

    /// Fail every outstanding reply immediately (used during shutdown).
    pub(crate) fn drain_as_stopped(&mut self) -> Vec<M> {
        self.entries
            .drain(..)
            .filter_map(|mut entry| entry.expire())
            .collect()
    }
}

#[derive(Debug, Default)]
struct WatchState {
    pending: usize,
    stop: bool,
}

/// The shared "is anything outstanding" signal the watcher waits on.
#[derive(Debug, Default)]
pub(crate) struct WorkSignal {
    state: Mutex<WatchState>,
    changed: Condvar,
}

impl WorkSignal {
    fn lock(&self) -> std::sync::MutexGuard<'_, WatchState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Publish how many units of work are outstanding.
    pub(crate) fn set_pending(&self, pending: usize) {
        let mut state = self.lock();
        if state.pending != pending {
            state.pending = pending;
            drop(state);
            self.changed.notify_all();
        }
    }

    fn stop(&self) {
        let mut state = self.lock();
        state.stop = true;
        drop(state);
        self.changed.notify_all();
    }
}

/// One thread that wakes the host while replies are outstanding.
pub(crate) struct ReplyWatcher {
    signal: Arc<WorkSignal>,
    running: Arc<AtomicBool>,
    finished: std::sync::mpsc::Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ReplyWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplyWatcher")
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ReplyWatcher {
    pub(crate) fn start(queue: &Arc<EventQueue>, interval: Duration) -> Self {
        let signal = Arc::new(WorkSignal::default());
        let running = Arc::new(AtomicBool::new(true));
        let worker_signal = Arc::clone(&signal);
        let worker_running = Arc::clone(&running);
        let worker_queue = Arc::clone(queue);
        let (finished_tx, finished) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("wfdiag-app-reply-watcher".to_string())
            .spawn(move || {
                loop {
                    let mut state = worker_signal.lock();
                    while !state.stop && state.pending == 0 {
                        state = worker_signal
                            .changed
                            .wait(state)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if state.stop {
                        break;
                    }
                    let (state, _timeout) = worker_signal
                        .changed
                        .wait_timeout(state, interval)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let stop = state.stop;
                    drop(state);
                    if stop {
                        break;
                    }
                    worker_queue.wake();
                }
                worker_running.store(false, Ordering::Release);
                let _ = finished_tx.send(());
            })
            .ok();
        if worker.is_none() {
            running.store(false, Ordering::Release);
        }
        Self {
            signal,
            running,
            finished,
            worker,
        }
    }

    pub(crate) fn signal(&self) -> Arc<WorkSignal> {
        Arc::clone(&self.signal)
    }

    /// Stop the watcher and wait at most `budget` for it to exit.
    pub(crate) fn stop(&mut self, budget: Duration) -> bool {
        self.signal.stop();
        let Some(worker) = self.worker.take() else {
            return true;
        };
        if self.finished.recv_timeout(budget).is_err() {
            // Never block past the budget: hand the handle to a detached
            // reaper so the thread is still joined instead of leaked.
            std::thread::Builder::new()
                .name("wfdiag-app-watcher-reaper".to_string())
                .spawn(move || {
                    let _ = worker.join();
                })
                .ok();
            return false;
        }
        worker.join().is_ok()
    }
}

impl Drop for ReplyWatcher {
    fn drop(&mut self) {
        self.signal.stop();
        if let Some(worker) = self.worker.take() {
            std::thread::Builder::new()
                .name("wfdiag-app-watcher-reaper".to_string())
                .spawn(move || {
                    let _ = worker.join();
                })
                .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingReplies, ReplyFailure, ReplyWatcher};
    use crate::command::WorkerKind;
    use crate::event::EventQueue;
    use crate::ids::RequestId;
    use std::time::{Duration, Instant};
    use tokio::sync::oneshot;

    fn map_message(result: Result<u8, ReplyFailure>) -> String {
        match result {
            Ok(value) => format!("ok:{value}"),
            Err(failure) => format!("err:{failure:?}"),
        }
    }

    #[test]
    fn a_reply_that_lands_is_delivered_once() {
        let mut replies = PendingReplies::new(Duration::from_secs(30));
        let (sender, receiver) = oneshot::channel();
        replies.register(
            WorkerKind::System,
            RequestId::from_raw(1),
            receiver,
            map_message,
        );
        assert!(replies.poll(Instant::now()).messages.is_empty());
        sender.send(7).expect("receiver is alive");
        let batch = replies.poll(Instant::now());
        assert_eq!(batch.messages, ["ok:7"]);
        assert_eq!(replies.len(), 0);
        assert!(replies.poll(Instant::now()).messages.is_empty());
    }

    #[test]
    fn a_dropped_sender_reports_the_worker_stopping() {
        let mut replies = PendingReplies::new(Duration::from_secs(30));
        let (sender, receiver) = oneshot::channel::<u8>();
        replies.register(
            WorkerKind::History,
            RequestId::from_raw(2),
            receiver,
            map_message,
        );
        drop(sender);
        let batch = replies.poll(Instant::now());
        assert_eq!(batch.messages, ["err:WorkerStopped"]);
        assert!(batch.timeouts.is_empty());
    }

    #[test]
    fn an_elapsed_deadline_produces_a_typed_timeout_instead_of_a_hang() {
        let mut replies = PendingReplies::new(Duration::from_millis(0));
        let (_sender, receiver) = oneshot::channel::<u8>();
        replies.register(
            WorkerKind::Provider,
            RequestId::from_raw(3),
            receiver,
            map_message,
        );
        let batch = replies.poll(Instant::now() + Duration::from_millis(1));
        assert_eq!(batch.messages, ["err:TimedOut"]);
        assert_eq!(batch.timeouts.len(), 1);
        assert_eq!(batch.timeouts[0].worker, WorkerKind::Provider);
        assert_eq!(batch.timeouts[0].request, RequestId::from_raw(3));
        assert_eq!(replies.len(), 0);
    }

    #[test]
    fn the_watcher_starts_stops_and_never_outlives_its_budget() {
        let queue = EventQueue::new(8);
        let mut watcher = ReplyWatcher::start(&queue, Duration::from_millis(5));
        let signal = watcher.signal();
        signal.set_pending(1);
        std::thread::sleep(Duration::from_millis(30));
        signal.set_pending(0);
        assert!(watcher.stop(Duration::from_secs(2)));
    }
}
