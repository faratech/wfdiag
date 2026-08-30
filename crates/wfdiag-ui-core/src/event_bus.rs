//! Bounded, coalescing delivery from backend workers to a UI thread.
//!
//! Lossless events are accepted in FIFO order and never silently discarded.
//! [`UiEventSink::publish`] waits asynchronously for capacity, while
//! [`UiEventSink::try_publish`] returns the original event on a full queue.
//! Live system statistics use one fixed latest-value slot. Nonterminal task
//! progress uses one latest-value slot per `(session_id, task_id)`, bounded by
//! the configured capacity.

use crate::UiEvent;
use event_listener::{Event, Listener};
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

pub type PublishFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PublishOutcome, PublishError>> + Send + 'a>>;

/// Cloneable event sink interface used by framework-neutral backend services.
pub trait UiEventSink: Send + Sync {
    /// Publish an event, waiting asynchronously when the lossless FIFO is full.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError`] with ownership of the event when the receiver
    /// closes before accepting it.
    fn publish(&self, event: UiEvent) -> PublishFuture<'_>;

    /// Publish without waiting.
    ///
    /// # Errors
    ///
    /// Returns [`TryPublishError::Full`] with ownership of a lossless event
    /// when its lane has no capacity, or [`TryPublishError::Closed`] when the
    /// receiver no longer accepts events.
    fn try_publish(&self, event: UiEvent) -> Result<PublishOutcome, TryPublishError>;

    fn clone_box(&self) -> Box<dyn UiEventSink>;
}

impl Clone for Box<dyn UiEventSink> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Enqueued,
    Coalesced { replaced: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TryPublishError {
    Full(Box<UiEvent>),
    Closed(Box<UiEvent>),
}

impl TryPublishError {
    fn full(event: UiEvent) -> Self {
        Self::Full(Box::new(event))
    }

    fn closed(event: UiEvent) -> Self {
        Self::Closed(Box::new(event))
    }

    #[must_use]
    pub fn into_event(self) -> UiEvent {
        match self {
            Self::Full(event) | Self::Closed(event) => *event,
        }
    }
}

impl fmt::Display for TryPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("the bounded UI event bus is full"),
            Self::Closed(_) => formatter.write_str("the UI event receiver is closed"),
        }
    }
}

impl Error for TryPublishError {}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishError(Box<UiEvent>);

impl PublishError {
    fn closed(event: UiEvent) -> Self {
        Self(Box::new(event))
    }

    #[must_use]
    pub fn into_event(self) -> UiEvent {
        *self.0
    }

    #[must_use]
    pub fn event(&self) -> &UiEvent {
        &self.0
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the UI event receiver is closed")
    }
}

impl Error for PublishError {}

#[derive(Debug)]
struct SequencedEvent {
    sequence: u64,
    event: UiEvent,
}

#[derive(Debug, Default)]
struct CoalescedSlots {
    system_stats: Option<SequencedEvent>,
    task_progress: HashMap<TaskKey, SequencedEvent>,
}

impl CoalescedSlots {
    fn len(&self) -> usize {
        usize::from(self.system_stats.is_some()) + self.task_progress.len()
    }

    fn take_all(&mut self, target: &mut Vec<SequencedEvent>) {
        target.extend(self.system_stats.take());
        target.extend(self.task_progress.drain().map(|(_, event)| event));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskKey {
    session_id: String,
    task_id: String,
}

impl TaskKey {
    fn from_event(event: &UiEvent) -> Option<Self> {
        let UiEvent::TaskProgress(progress) = event else {
            return None;
        };
        Some(Self {
            session_id: progress.session_id.clone(),
            task_id: progress.task_id.clone(),
        })
    }
}

#[derive(Debug)]
struct BusState {
    accepting: bool,
    publishers: usize,
    next_sequence: u64,
    lossless: VecDeque<SequencedEvent>,
    coalesced: CoalescedSlots,
}

impl BusState {
    fn sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        if self.next_sequence == 0 {
            self.next_sequence = 1;
        }
        sequence
    }

    fn pending_len(&self) -> usize {
        self.lossless.len() + self.coalesced.len()
    }
}

#[derive(Debug)]
struct Shared {
    capacity: usize,
    state: Mutex<BusState>,
    items_available: Event,
    lossless_capacity_available: Event,
    progress_capacity_available: Event,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, BusState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn notify_closed(&self) {
        self.items_available.notify(usize::MAX);
        self.lossless_capacity_available.notify(usize::MAX);
        self.progress_capacity_available.notify(usize::MAX);
    }
}

#[derive(Debug, Clone, Copy)]
enum Lane {
    SystemStats,
    TaskProgress,
    Lossless,
}

fn lane(event: &UiEvent) -> Lane {
    match event {
        UiEvent::SystemStats(_) => Lane::SystemStats,
        UiEvent::TaskProgress(progress) if !progress.is_terminal() => Lane::TaskProgress,
        UiEvent::TaskProgress(_)
        | UiEvent::DiagnosticResult(_)
        | UiEvent::Chat(_)
        | UiEvent::Report(_)
        | UiEvent::ActionStatus(_)
        | UiEvent::QuickScan(_) => Lane::Lossless,
    }
}

fn supersede_matching_progress(state: &mut BusState, event: &UiEvent) -> bool {
    let UiEvent::TaskProgress(terminal) = event else {
        return false;
    };
    if !terminal.is_terminal() {
        return false;
    }

    state
        .coalesced
        .task_progress
        .remove(&TaskKey {
            session_id: terminal.session_id.clone(),
            task_id: terminal.task_id.clone(),
        })
        .is_some()
}

/// Sending half of a bounded UI event bus.
pub struct UiEventPublisher {
    shared: Arc<Shared>,
}

impl fmt::Debug for UiEventPublisher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UiEventPublisher")
            .field("capacity", &self.shared.capacity)
            .finish_non_exhaustive()
    }
}

impl Clone for UiEventPublisher {
    fn clone(&self) -> Self {
        {
            let mut state = self.shared.state();
            state.publishers = state.publishers.saturating_add(1);
        }
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for UiEventPublisher {
    fn drop(&mut self) {
        let closed = {
            let mut state = self.shared.state();
            state.publishers = state.publishers.saturating_sub(1);
            if state.publishers == 0 {
                state.accepting = false;
                true
            } else {
                false
            }
        };
        if closed {
            self.shared.notify_closed();
        }
    }
}

impl UiEventPublisher {
    /// Publish an event, asynchronously waiting for bounded-lane capacity.
    ///
    /// Coalescible updates return immediately when replacing an existing key.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError`] with ownership of the event if the receiver
    /// closes before accepting it.
    pub async fn publish(&self, event: UiEvent) -> Result<PublishOutcome, PublishError> {
        self.publish_owned(event).await
    }

    /// Publish an event without waiting for bounded-lane capacity.
    ///
    /// # Errors
    ///
    /// Returns [`TryPublishError`] with ownership of an event that could not be
    /// accepted.
    pub fn try_publish(&self, event: UiEvent) -> Result<PublishOutcome, TryPublishError> {
        self.try_publish_owned(event)
    }

    /// True once the receiver has stopped accepting new events.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        !self.shared.state().accepting
    }

    /// Stop all current and future publishers. Already accepted events remain
    /// available to the receiver.
    pub fn close(&self) {
        {
            let mut state = self.shared.state();
            state.accepting = false;
        }
        self.shared.notify_closed();
    }

    async fn publish_owned(&self, event: UiEvent) -> Result<PublishOutcome, PublishError> {
        let lane = lane(&event);
        let mut pending = Some(event);

        loop {
            let wait_for_capacity = {
                let mut state = self.shared.state();
                if !state.accepting {
                    return Err(PublishError::closed(
                        pending.take().expect("pending event must exist"),
                    ));
                }

                match lane {
                    Lane::SystemStats => {
                        let sequence = state.sequence();
                        let replacement = SequencedEvent {
                            sequence,
                            event: pending.take().expect("pending event must exist"),
                        };
                        let replaced = state.coalesced.system_stats.replace(replacement).is_some();
                        drop(state);
                        self.shared.items_available.notify(1);
                        return Ok(PublishOutcome::Coalesced { replaced });
                    }
                    Lane::TaskProgress => {
                        let key = TaskKey::from_event(
                            pending.as_ref().expect("pending event must exist"),
                        )
                        .expect("task-progress lane must contain task progress");
                        let replacing = state.coalesced.task_progress.contains_key(&key);
                        if !replacing && state.coalesced.task_progress.len() >= self.shared.capacity
                        {
                            self.shared.progress_capacity_available.listen()
                        } else {
                            let sequence = state.sequence();
                            let replacement = SequencedEvent {
                                sequence,
                                event: pending.take().expect("pending event must exist"),
                            };
                            let replaced = state
                                .coalesced
                                .task_progress
                                .insert(key, replacement)
                                .is_some();
                            drop(state);
                            self.shared.items_available.notify(1);
                            return Ok(PublishOutcome::Coalesced { replaced });
                        }
                    }
                    Lane::Lossless if state.lossless.len() < self.shared.capacity => {
                        let event = pending.take().expect("pending event must exist");
                        let released_progress = supersede_matching_progress(&mut state, &event);
                        let sequence = state.sequence();
                        state.lossless.push_back(SequencedEvent { sequence, event });
                        drop(state);
                        if released_progress {
                            self.shared.progress_capacity_available.notify_additional(1);
                        }
                        self.shared.items_available.notify(1);
                        return Ok(PublishOutcome::Enqueued);
                    }
                    Lane::Lossless => self.shared.lossless_capacity_available.listen(),
                }
            };

            wait_for_capacity.await;
        }
    }

    fn try_publish_owned(&self, event: UiEvent) -> Result<PublishOutcome, TryPublishError> {
        let lane = lane(&event);
        let mut state = self.shared.state();
        if !state.accepting {
            return Err(TryPublishError::closed(event));
        }

        let (outcome, released_progress) = match lane {
            Lane::SystemStats => {
                let sequence = state.sequence();
                let replaced = state
                    .coalesced
                    .system_stats
                    .replace(SequencedEvent { sequence, event })
                    .is_some();
                (PublishOutcome::Coalesced { replaced }, false)
            }
            Lane::TaskProgress => {
                let key = TaskKey::from_event(&event)
                    .expect("task-progress lane must contain task progress");
                if !state.coalesced.task_progress.contains_key(&key)
                    && state.coalesced.task_progress.len() >= self.shared.capacity
                {
                    return Err(TryPublishError::full(event));
                }
                let sequence = state.sequence();
                let replaced = state
                    .coalesced
                    .task_progress
                    .insert(key, SequencedEvent { sequence, event })
                    .is_some();
                (PublishOutcome::Coalesced { replaced }, false)
            }
            Lane::Lossless if state.lossless.len() < self.shared.capacity => {
                let released_progress = supersede_matching_progress(&mut state, &event);
                let sequence = state.sequence();
                state.lossless.push_back(SequencedEvent { sequence, event });
                (PublishOutcome::Enqueued, released_progress)
            }
            Lane::Lossless => return Err(TryPublishError::full(event)),
        };

        drop(state);
        if released_progress {
            self.shared.progress_capacity_available.notify_additional(1);
        }
        self.shared.items_available.notify(1);
        Ok(outcome)
    }
}

impl UiEventSink for UiEventPublisher {
    fn publish(&self, event: UiEvent) -> PublishFuture<'_> {
        Box::pin(UiEventPublisher::publish(self, event))
    }

    fn try_publish(&self, event: UiEvent) -> Result<PublishOutcome, TryPublishError> {
        UiEventPublisher::try_publish(self, event)
    }

    fn clone_box(&self) -> Box<dyn UiEventSink> {
        Box::new(self.clone())
    }
}

/// UI-thread half of the event bus.
pub struct UiEventReceiver {
    shared: Arc<Shared>,
}

impl fmt::Debug for UiEventReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UiEventReceiver")
            .field("capacity", &self.shared.capacity)
            .field("pending", &self.pending_len())
            .finish_non_exhaustive()
    }
}

impl UiEventReceiver {
    /// Drain every event currently accepted by the bus.
    ///
    /// The returned batch is ordered by acceptance sequence. Its maximum size
    /// is `2 * lossless_capacity + 1`: the lossless FIFO, one progress slot
    /// per active task up to the same bound, and one system-stats slot.
    #[must_use]
    pub fn drain(&self) -> Vec<UiEvent> {
        let (mut drained, released_lossless, released_progress) = {
            let mut state = self.shared.state();
            let released_lossless = state.lossless.len();
            let released_progress = state.coalesced.task_progress.len();
            let mut drained = Vec::with_capacity(state.pending_len());
            state.coalesced.take_all(&mut drained);
            drained.extend(state.lossless.drain(..));
            (drained, released_lossless, released_progress)
        };

        if released_lossless != 0 {
            self.shared
                .lossless_capacity_available
                .notify_additional(released_lossless);
        }
        if released_progress != 0 {
            self.shared
                .progress_capacity_available
                .notify_additional(released_progress);
        }
        drained.sort_by_key(|item| item.sequence);
        drained.into_iter().map(|item| item.event).collect()
    }

    /// Wait until at least one event is available or all publishers close.
    pub async fn wait_for_events(&self) {
        loop {
            let listener = {
                let state = self.shared.state();
                if state.pending_len() != 0 || !state.accepting {
                    return;
                }
                self.shared.items_available.listen()
            };
            listener.await;
        }
    }

    /// Block until events arrive, the bus closes, or `timeout` elapses.
    ///
    /// This is intended for native UI frameworks whose scope-owned worker
    /// callback is synchronous. Returning `false` on timeout lets that worker
    /// observe its framework cancellation token without polling the bus lock.
    #[must_use]
    pub fn wait_for_events_timeout(&self, timeout: Duration) -> bool {
        loop {
            let listener = {
                let state = self.shared.state();
                if state.pending_len() != 0 || !state.accepting {
                    return true;
                }
                self.shared.items_available.listen()
            };
            if listener.wait_timeout(timeout).is_none() {
                return false;
            }
        }
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.shared.state().pending_len()
    }

    #[must_use]
    pub fn lossless_len(&self) -> usize {
        self.shared.state().lossless.len()
    }

    #[must_use]
    pub fn lossless_capacity(&self) -> usize {
        self.shared.capacity
    }

    /// True once the receiver no longer accepts new events.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        !self.shared.state().accepting
    }

    /// True once publishing has closed and every accepted event was drained.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        let state = self.shared.state();
        !state.accepting && state.pending_len() == 0
    }

    /// Stop every publisher while preserving already accepted events for a
    /// final drain.
    pub fn close(&self) {
        {
            let mut state = self.shared.state();
            state.accepting = false;
        }
        self.shared.notify_closed();
    }
}

impl Drop for UiEventReceiver {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state();
            state.accepting = false;
            state.lossless.clear();
            state.coalesced = CoalescedSlots::default();
        }
        self.shared.notify_closed();
    }
}

/// Construct a bus with a bounded lossless FIFO, an equally bounded keyed
/// progress lane, and one system-stats slot. Exactly one receiver is returned
/// by construction.
#[must_use]
pub fn ui_event_bus(lossless_capacity: NonZeroUsize) -> (UiEventPublisher, UiEventReceiver) {
    let shared = Arc::new(Shared {
        capacity: lossless_capacity.get(),
        state: Mutex::new(BusState {
            accepting: true,
            publishers: 1,
            next_sequence: 1,
            lossless: VecDeque::with_capacity(lossless_capacity.get()),
            coalesced: CoalescedSlots::default(),
        }),
        items_available: Event::new(),
        lossless_capacity_available: Event::new(),
        progress_capacity_available: Event::new(),
    });

    (
        UiEventPublisher {
            shared: Arc::clone(&shared),
        },
        UiEventReceiver { shared },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionRunStatus, ActionStatus, ChatDelta, ChatDone, ChatEvent, ChatFinishReason,
        DiagnosticTaskResult, ProviderExecutionClass, ProviderUse, QuickScanRequest,
        QuickScanSource, ReportDelta, ReportEvent, SystemStats, TaskProgress, TaskProgressStatus,
    };
    use std::time::Duration;

    fn capacity(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("test capacity is nonzero")
    }

    fn provider_use() -> ProviderUse {
        ProviderUse {
            provider_id: "openai".into(),
            execution_class: ProviderExecutionClass::ApiCloud,
            fallback_from: None,
            requested_model: Some("gpt-5".into()),
            actual_models: vec!["gpt-5.2".into()],
        }
    }

    fn delta(text: &str) -> UiEvent {
        UiEvent::Chat(ChatEvent::Delta(ChatDelta {
            session_id: "session-1".into(),
            message_id: "message-1".into(),
            text: text.into(),
        }))
    }

    fn done() -> UiEvent {
        UiEvent::Chat(ChatEvent::Done(ChatDone {
            session_id: "session-1".into(),
            message_id: "message-1".into(),
            finish_reason: ChatFinishReason::Stop,
            provider: "openai".into(),
            provider_use: provider_use(),
            tool_call_count: 0,
        }))
    }

    fn progress(task_id: &str, status: TaskProgressStatus) -> UiEvent {
        UiEvent::TaskProgress(TaskProgress {
            session_id: "scan-1".into(),
            task_id: task_id.into(),
            status,
            task_name: Some(format!("Task {task_id}")),
            success: status
                .is_terminal()
                .then_some(status == TaskProgressStatus::Completed),
        })
    }

    fn diagnostic_result(task_id: &str) -> UiEvent {
        UiEvent::DiagnosticResult(DiagnosticTaskResult {
            session_id: "scan-1".into(),
            task_id: task_id.into(),
            success: true,
            output: "{}".into(),
            error: None,
            duration_ms: 1,
        })
    }

    fn stats(timestamp: i64) -> UiEvent {
        let utilization = f32::from(i16::try_from(timestamp).expect("small test timestamp"));
        UiEvent::SystemStats(SystemStats {
            cpu_utilization: utilization,
            per_cpu_utilization: vec![utilization],
            cpu_frequency: 4_000,
            memory_total_gb: 32.0,
            memory_used_gb: 10.0,
            memory_available_gb: 22.0,
            memory_utilization: 31.25,
            swap_total_gb: 4.0,
            swap_used_gb: 0.5,
            swap_utilization: 12.5,
            storage_used_percent: 50.0,
            disk_utilization: 50.0,
            disk_read_bytes: 100,
            disk_write_bytes: 200,
            disks: Vec::new(),
            network_upload_kb: 1.0,
            network_download_kb: 2.0,
            gpu_available: false,
            gpu_name: None,
            gpu_utilization: None,
            gpu_memory_used_mb: 0.0,
            gpu_memory_total_mb: 0.0,
            npu_available: false,
            npu_name: None,
            npu_utilization: None,
            npu_memory_used_mb: 0.0,
            npu_memory_total_mb: 0.0,
            top_processes: Vec::new(),
            timestamp,
        })
    }

    fn action_status(status: ActionRunStatus) -> UiEvent {
        UiEvent::ActionStatus(ActionStatus {
            run_id: "run-1".into(),
            proposal_id: "proposal-1".into(),
            authorization_id: "authorization-1".into(),
            status,
            actions: Vec::new(),
            current_index: None,
            approved_at_ms: 10,
            completed_at_ms: status.is_terminal().then_some(20),
            scan_fingerprint: "scan-fingerprint".into(),
            catalog_fingerprint: "catalog-fingerprint".into(),
        })
    }

    #[test]
    fn chat_and_terminal_events_keep_fifo_order() {
        let (publisher, receiver) = ui_event_bus(capacity(8));
        let events = vec![
            delta("first"),
            UiEvent::Report(ReportEvent::Delta(ReportDelta {
                report_id: "report-1".into(),
                text: "second".into(),
            })),
            action_status(ActionRunStatus::Running),
            diagnostic_result("cpu"),
            progress("cpu", TaskProgressStatus::Completed),
            done(),
            UiEvent::QuickScan(QuickScanRequest {
                request_id: "request-1".into(),
                requested_at_ms: 42,
                source: QuickScanSource::Tray,
            }),
        ];

        for event in &events {
            assert_eq!(
                publisher.try_publish(event.clone()),
                Ok(PublishOutcome::Enqueued)
            );
        }
        assert_eq!(receiver.drain(), events);
    }

    #[test]
    fn blocking_wait_times_out_then_observes_sample_and_close() {
        let (publisher, receiver) = ui_event_bus(capacity(2));

        assert!(!receiver.wait_for_events_timeout(Duration::from_millis(1)));
        publisher.try_publish(stats(7)).unwrap();
        assert!(receiver.wait_for_events_timeout(Duration::from_secs(1)));
        assert_eq!(receiver.drain(), vec![stats(7)]);

        publisher.close();
        assert!(receiver.wait_for_events_timeout(Duration::from_secs(1)));
        assert!(receiver.is_terminated());
    }

    #[test]
    fn latest_monitoring_and_each_tasks_nonterminal_progress_are_coalesced() {
        let (publisher, receiver) = ui_event_bus(capacity(4));

        assert_eq!(
            publisher.try_publish(stats(1)),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );
        assert_eq!(
            publisher.try_publish(stats(2)),
            Ok(PublishOutcome::Coalesced { replaced: true })
        );
        assert_eq!(
            publisher.try_publish(progress("cpu", TaskProgressStatus::Running)),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );
        assert_eq!(
            publisher.try_publish(progress("cpu", TaskProgressStatus::Queued)),
            Ok(PublishOutcome::Coalesced { replaced: true })
        );
        assert_eq!(
            publisher.try_publish(progress("memory", TaskProgressStatus::Running)),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );

        assert_eq!(receiver.pending_len(), 3);
        assert_eq!(
            receiver.drain(),
            vec![
                stats(2),
                progress("cpu", TaskProgressStatus::Queued),
                progress("memory", TaskProgressStatus::Running),
            ]
        );
    }

    #[test]
    fn terminal_progress_supersedes_only_the_matching_tasks_stale_progress() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Running))
            .unwrap();
        publisher
            .try_publish(progress("memory", TaskProgressStatus::Running))
            .unwrap();
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Completed))
            .unwrap();

        assert_eq!(receiver.pending_len(), 2);
        assert_eq!(
            receiver.drain(),
            vec![
                progress("memory", TaskProgressStatus::Running),
                progress("cpu", TaskProgressStatus::Completed),
            ]
        );
    }

    #[test]
    fn keyed_progress_capacity_returns_the_undelivered_task() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Running))
            .unwrap();

        let memory = progress("memory", TaskProgressStatus::Running);
        let error = publisher.try_publish(memory.clone()).unwrap_err();
        assert_eq!(error, TryPublishError::Full(Box::new(memory.clone())));
        assert_eq!(
            receiver.drain(),
            vec![progress("cpu", TaskProgressStatus::Running)]
        );
        assert_eq!(
            publisher.try_publish(error.into_event()),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );
        assert_eq!(receiver.drain(), vec![memory]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_progress_releases_keyed_capacity_without_waiting_for_a_drain() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Running))
            .unwrap();
        publisher
            .try_publish(progress("memory", TaskProgressStatus::Running))
            .unwrap();

        let blocked_publisher = publisher.clone();
        let mut blocked = tokio::spawn(async move {
            blocked_publisher
                .publish(progress("disk", TaskProgressStatus::Running))
                .await
        });
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut blocked)
                .await
                .is_err()
        );

        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Completed))
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), blocked)
                .await
                .unwrap()
                .unwrap(),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );
        assert_eq!(
            receiver.drain(),
            vec![
                progress("memory", TaskProgressStatus::Running),
                progress("cpu", TaskProgressStatus::Completed),
                progress("disk", TaskProgressStatus::Running),
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn consecutive_same_lane_releases_wake_additional_waiters() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Running))
            .unwrap();
        publisher
            .try_publish(progress("memory", TaskProgressStatus::Running))
            .unwrap();

        let mut waiters = Vec::new();
        for task_id in ["disk", "network", "gpu"] {
            let blocked_publisher = publisher.clone();
            waiters.push(tokio::spawn(async move {
                blocked_publisher
                    .publish(progress(task_id, TaskProgressStatus::Running))
                    .await
            }));
            // Fix registration order and ensure every publisher reaches its
            // capacity listener before either slot is released.
            tokio::task::yield_now().await;
        }
        assert!(waiters.iter().all(|waiter| !waiter.is_finished()));

        // Release two progress keys without yielding to the first notified
        // waiter. Event::notify(1) is non-additive and would collapse these
        // into one wake; notify_additional(1) wakes one publisher per release.
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Completed))
            .unwrap();
        publisher
            .try_publish(progress("memory", TaskProgressStatus::Completed))
            .unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert_eq!(
            waiters.iter().filter(|waiter| waiter.is_finished()).count(),
            2,
            "each released progress key must wake an additional waiter"
        );

        receiver.close();
        for waiter in waiters {
            let _ = waiter.await;
        }
    }

    #[test]
    fn full_try_publish_returns_the_undelivered_event() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher.try_publish(delta("first")).unwrap();

        let second = delta("second");
        let error = publisher.try_publish(second.clone()).unwrap_err();
        assert_eq!(error, TryPublishError::Full(Box::new(second.clone())));
        assert_eq!(error.into_event(), second);
        assert_eq!(receiver.drain(), vec![delta("first")]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_publish_applies_backpressure_then_resumes() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher.try_publish(delta("first")).unwrap();

        let blocked_publisher = publisher.clone();
        let mut blocked =
            tokio::spawn(async move { blocked_publisher.publish(delta("second")).await });
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut blocked)
                .await
                .is_err()
        );

        assert_eq!(receiver.drain(), vec![delta("first")]);
        assert_eq!(blocked.await.unwrap(), Ok(PublishOutcome::Enqueued));
        assert_eq!(receiver.drain(), vec![delta("second")]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mixed_lane_waiters_receive_only_their_capacity_wakeups() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher.try_publish(delta("lossless-occupied")).unwrap();
        publisher
            .try_publish(progress("cpu", TaskProgressStatus::Running))
            .unwrap();

        // Register two lossless waiters first. With one shared capacity Event,
        // draining one slot from each lane could wake both lossless waiters and
        // leave the progress waiter asleep even though its lane has capacity.
        let lossless_publisher_a = publisher.clone();
        let lossless_a = tokio::spawn(async move {
            lossless_publisher_a
                .publish(delta("lossless-waiter-a"))
                .await
        });
        tokio::task::yield_now().await;

        let lossless_publisher_b = publisher.clone();
        let lossless_b = tokio::spawn(async move {
            lossless_publisher_b
                .publish(delta("lossless-waiter-b"))
                .await
        });
        tokio::task::yield_now().await;

        let progress_publisher = publisher.clone();
        let mut progress_waiter = tokio::spawn(async move {
            progress_publisher
                .publish(progress("memory", TaskProgressStatus::Running))
                .await
        });
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut progress_waiter)
                .await
                .is_err()
        );

        assert_eq!(
            receiver.drain(),
            vec![
                delta("lossless-occupied"),
                progress("cpu", TaskProgressStatus::Running),
            ]
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut progress_waiter)
                .await
                .expect("progress waiter was not woken for progress capacity")
                .unwrap(),
            Ok(PublishOutcome::Coalesced { replaced: false })
        );

        // One lossless waiter may have occupied the released FIFO slot. Closing
        // wakes whichever lossless waiter remains, keeping test cleanup finite.
        receiver.close();
        let _ = lossless_a.await;
        let _ = lossless_b.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn coalesced_values_bypass_a_full_lossless_queue() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher.try_publish(delta("first")).unwrap();

        assert_eq!(
            publisher.publish(stats(7)).await,
            Ok(PublishOutcome::Coalesced { replaced: false })
        );
        assert_eq!(receiver.drain(), vec![delta("first"), stats(7)]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn receiver_close_preserves_accepted_events_and_rejects_new_ones() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        publisher.try_publish(delta("accepted")).unwrap();
        receiver.close();

        let rejected = done();
        let error = publisher.publish(rejected.clone()).await.unwrap_err();
        assert_eq!(error.event(), &rejected);
        assert!(publisher.is_closed());
        assert!(receiver.is_closed());
        assert!(!receiver.is_terminated());
        assert_eq!(receiver.drain(), vec![delta("accepted")]);
        assert!(receiver.is_terminated());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_receiver_wakes_blocked_publishers_with_their_event() {
        let (publisher, receiver) = ui_event_bus(capacity(1));
        publisher.try_publish(delta("first")).unwrap();

        let blocked_publisher = publisher.clone();
        let blocked_event = done();
        let expected = blocked_event.clone();
        let blocked = tokio::spawn(async move { blocked_publisher.publish(blocked_event).await });
        tokio::task::yield_now().await;
        drop(receiver);

        assert!(publisher.is_closed());
        assert_eq!(blocked.await.unwrap().unwrap_err().into_event(), expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_last_publisher_closes_after_pending_values_drain() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        publisher.try_publish(stats(9)).unwrap();
        drop(publisher);

        receiver.wait_for_events().await;
        assert!(receiver.is_closed());
        assert_eq!(receiver.drain(), vec![stats(9)]);
        assert!(receiver.is_terminated());
    }

    #[test]
    fn boxed_sinks_are_cloneable() {
        let (publisher, receiver) = ui_event_bus(capacity(2));
        let sink: Box<dyn UiEventSink> = Box::new(publisher);
        let clone = sink.clone();

        sink.try_publish(delta("one")).unwrap();
        clone.try_publish(delta("two")).unwrap();
        assert_eq!(receiver.drain(), vec![delta("one"), delta("two")]);
    }
}
