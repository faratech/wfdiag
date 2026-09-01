//! Framework-neutral action runtime: workers, run projection, cancellation.
//!
//! The UI can only stage catalog IDs into an immutable preview and later
//! approve that preview by its opaque ID. The worker owns the proposal store,
//! consumes each grant exactly once through [`crate::broker::ActionBroker`],
//! revalidates current scan/catalog/issue evidence at the execution boundary,
//! and is the only place that reaches [`RealCatalogExecutor`].

use std::collections::VecDeque;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::broker::{
    ActionApproval, ActionBroker, ActionGrant, ActionPrepareInput, ActionProposal, ActionSnapshot,
    AuthorizationError, AuthorizedActionExecutor, RealCatalogExecutor, now_ms, opaque_id,
};
use crate::remediation::{FixCompletionStatus, FixResult, RemediationSummary};

pub use wfdiag_ui_core::contract::ActionItemStatus;

const MAX_ACTION_HISTORY: usize = 50;

/// Notification invoked after a worker event is queued. Native shells use it
/// to schedule one UI-thread drain instead of polling each receiver.
pub type ActionWakeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Clone, Debug)]
pub struct ActionExecutionItem {
    pub remediation: RemediationSummary,
    pub issue_id: Option<String>,
    pub result: Result<FixResult, String>,
}

#[derive(Clone, Debug)]
pub struct ActionExecution {
    pub run_id: String,
    pub proposal_id: String,
    pub items: Vec<ActionExecutionItem>,
    /// Full terminal projection. `items` remains for the already-integrated
    /// per-item status path; new UI should render this summary instead.
    pub summary: ActionRunSummary,
}

/// Aggregate lifecycle for one approved proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRunStatus {
    Running,
    CancelRequested,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

impl ActionRunStatus {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

/// One action's live/terminal projection. `FixResult` is preserved verbatim,
/// including per-step succeeded/already-satisfied/failed/cancelled detail and
/// the requires-restart flag.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemRun {
    pub remediation_id: String,
    pub label: String,
    pub cancellable: bool,
    pub status: ActionItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<FixResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Addressable, rehydratable run state. This is the payload both shells
/// render, so its serialized shape is a pinned wire contract.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRunSummary {
    pub run_id: String,
    pub proposal_id: String,
    /// Backend-generated one-use authorization identifier. It is audit
    /// metadata, not a capability accepted by any entry point.
    pub authorization_id: String,
    pub status: ActionRunStatus,
    pub actions: Vec<ActionItemRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_index: Option<usize>,
    pub approved_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
}

impl ActionRunSummary {
    #[must_use]
    pub fn requires_restart(&self) -> bool {
        self.actions.iter().any(|action| {
            action
                .result
                .as_ref()
                .is_some_and(|result| result.requires_restart)
        })
    }
}

/// The initial projection for a freshly consumed grant. Both shells build
/// their run record from this so the two projections cannot drift.
#[must_use]
pub fn initial_run_summary(
    run_id: String,
    grant: &ActionGrant,
    approved_at_ms: u64,
) -> ActionRunSummary {
    let proposal = grant.proposal();
    ActionRunSummary {
        run_id,
        proposal_id: proposal.proposal_id.clone(),
        authorization_id: grant.authorization_id().to_string(),
        status: ActionRunStatus::Running,
        actions: proposal
            .actions
            .iter()
            .map(|action| ActionItemRun {
                remediation_id: action.remediation.id.clone(),
                label: action.remediation.label.clone(),
                cancellable: action.remediation.cancellable,
                status: ActionItemStatus::Pending,
                result: None,
                error: None,
            })
            .collect(),
        current_index: None,
        approved_at_ms,
        completed_at_ms: None,
        scan_fingerprint: proposal.scan_fingerprint.clone(),
        catalog_fingerprint: proposal.catalog_fingerprint.clone(),
    }
}

/// A late cancel must not rewrite work that already finished.
#[must_use]
pub fn cancellation_applies(actions: &[ActionItemRun], requested: bool) -> bool {
    let has_cancelled_item = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Cancelled);
    let has_unfinished_item = actions.iter().any(|action| {
        matches!(
            action.status,
            ActionItemStatus::Pending | ActionItemStatus::Running
        )
    });
    has_cancelled_item || (requested && has_unfinished_item)
}

/// Aggregate a finished run's per-item statuses into one terminal status.
#[must_use]
pub fn completed_run_status(actions: &[ActionItemRun], was_cancelled: bool) -> ActionRunStatus {
    if was_cancelled {
        return ActionRunStatus::Cancelled;
    }
    let succeeded = actions
        .iter()
        .filter(|action| action.status == ActionItemStatus::Succeeded)
        .count();
    let partial = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Partial);
    let failed = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Failed);
    if partial || (failed && succeeded > 0) {
        ActionRunStatus::Partial
    } else if failed {
        ActionRunStatus::Failed
    } else {
        ActionRunStatus::Succeeded
    }
}

/// Map one engine result onto its per-item projection status.
#[must_use]
pub const fn item_status_for(completion: FixCompletionStatus) -> ActionItemStatus {
    match completion {
        FixCompletionStatus::Succeeded => ActionItemStatus::Succeeded,
        FixCompletionStatus::Partial => ActionItemStatus::Partial,
        FixCompletionStatus::Failed => ActionItemStatus::Failed,
        FixCompletionStatus::Cancelled => ActionItemStatus::Cancelled,
    }
}

/// A live update from the action runtime. The request ID correlates initial
/// authorization with the caller's pending-request identity; rehydrated
/// history intentionally contains only the stable summary.
#[derive(Clone, Debug)]
pub struct ActionRunEvent {
    pub request_id: u64,
    pub summary: ActionRunSummary,
}

/// Atomic state returned when a remounted view subscribes. Subscribe and
/// snapshot happen under one lock, so no run transition can be lost between
/// the history read and live delivery.
#[derive(Debug)]
pub struct ActionRuntimeSnapshot {
    pub pending_proposals: Vec<ActionProposal>,
    pub history: Vec<ActionRunSummary>,
    pub active_run: Option<ActionRunSummary>,
}

/// Typed worker events drained by the shell.
#[derive(Clone, Debug)]
pub enum ActionWorkerEvent {
    Prepared {
        request_id: u64,
        proposal: ActionProposal,
    },
    Done {
        request_id: u64,
        execution: ActionExecution,
    },
    Failed {
        request_id: u64,
        message: String,
    },
    NeedsRepairConfirmation {
        request_id: u64,
        proposal: ActionProposal,
    },
}

impl ActionWorkerEvent {
    /// The originating request's identity, for stale-event rejection.
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Prepared { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::NeedsRepairConfirmation { request_id, .. } => *request_id,
        }
    }
}

enum ActionCommand {
    Prepare {
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    },
    Approve {
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    },
    Discard {
        proposal_id: String,
    },
}

struct ActionRunRecord {
    request_id: u64,
    summary: ActionRunSummary,
    cancel: CancellationToken,
}

/// A consumed grant bound to one reserved run slot.
#[derive(Debug)]
pub struct AuthorizedRun {
    request_id: u64,
    run_id: String,
    grant: ActionGrant,
    cancel: CancellationToken,
}

impl AuthorizedRun {
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

/// Proposal store, bounded run history, and live subscribers.
#[derive(Default)]
pub struct ActionRuntimeState {
    broker: ActionBroker,
    runs: VecDeque<ActionRunRecord>,
    active_run_id: Option<String>,
    subscribers: Vec<std_mpsc::Sender<ActionRunEvent>>,
    wake: Option<ActionWakeHandler>,
}

impl ActionRuntimeState {
    fn publish(&mut self, event: &ActionRunEvent) {
        let wake = self.wake.clone();
        self.subscribers
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
        if let Some(wake) = wake {
            wake();
        }
    }

    fn publish_summary(&mut self, request_id: u64, summary: ActionRunSummary) {
        self.publish(&ActionRunEvent {
            request_id,
            summary,
        });
    }

    fn record_grant(
        &mut self,
        request_id: u64,
        grant: ActionGrant,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, String> {
        if self.active_run_id.is_some() {
            return Err("Another system action is already running".to_string());
        }

        let run_id = opaque_id("run");
        let cancel = CancellationToken::new();
        let summary = initial_run_summary(run_id.clone(), &grant, approved_at_ms);
        self.active_run_id = Some(run_id.clone());
        self.runs.push_back(ActionRunRecord {
            request_id,
            summary: summary.clone(),
            cancel: cancel.clone(),
        });
        while self.runs.len() > MAX_ACTION_HISTORY {
            self.runs.pop_front();
        }
        self.publish_summary(request_id, summary);
        Ok(AuthorizedRun {
            request_id,
            run_id,
            grant,
            cancel,
        })
    }

    fn authorize_run(
        &mut self,
        request_id: u64,
        proposal_id: &str,
        snapshot: &ActionSnapshot,
        approval: ActionApproval,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, AuthorizationError> {
        // Reserve the one mutation slot before consuming the proposal. A
        // rejected concurrent approval therefore remains reviewable.
        if self.active_run_id.is_some() {
            return Err(AuthorizationError::Rejected(
                "Another system action is already running".to_string(),
            ));
        }
        let grant = self
            .broker
            .authorize(proposal_id, snapshot, approval, approved_at_ms)?;
        self.record_grant(request_id, grant, approved_at_ms)
            .map_err(AuthorizationError::Rejected)
    }

    fn run(&self, run_id: &str) -> Option<&ActionRunRecord> {
        self.runs
            .iter()
            .find(|record| record.summary.run_id == run_id)
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut ActionRunRecord> {
        self.runs
            .iter_mut()
            .find(|record| record.summary.run_id == run_id)
    }

    fn history(&self) -> Vec<ActionRunSummary> {
        self.runs
            .iter()
            .rev()
            .map(|record| record.summary.clone())
            .collect()
    }

    fn active_run(&self) -> Option<ActionRunSummary> {
        self.active_run_id
            .as_deref()
            .and_then(|run_id| self.run(run_id))
            .map(|record| record.summary.clone())
    }

    fn cancel_run(&mut self, run_id: &str) -> Result<ActionRunSummary, String> {
        let (request_id, summary) = {
            let record = self
                .run_mut(run_id)
                .ok_or_else(|| "Action run was not found".to_string())?;
            if record.summary.status.terminal() {
                return Ok(record.summary.clone());
            }
            if let Some(index) = record.summary.current_index
                && !record.summary.actions[index].cancellable
            {
                return Err("The current action cannot be stopped safely once started".to_string());
            }
            record.cancel.cancel();
            record.summary.status = ActionRunStatus::CancelRequested;
            (record.request_id, record.summary.clone())
        };
        self.publish_summary(request_id, summary.clone());
        Ok(summary)
    }

    fn fail_run(&mut self, run_id: &str, message: String) -> Option<ActionRunSummary> {
        let (request_id, summary) = {
            let record = self.run_mut(run_id)?;
            if record.summary.status.terminal() {
                return Some(record.summary.clone());
            }
            let failed_index = record.summary.current_index.or_else(|| {
                record
                    .summary
                    .actions
                    .iter()
                    .position(|action| action.status == ActionItemStatus::Pending)
            });
            if let Some(index) = failed_index {
                let action = &mut record.summary.actions[index];
                action.status = ActionItemStatus::Failed;
                action.error = Some(message);
            }
            for action in &mut record.summary.actions {
                if action.status == ActionItemStatus::Pending {
                    action.status = ActionItemStatus::Skipped;
                }
            }
            record.summary.status = completed_run_status(&record.summary.actions, false);
            record.summary.current_index = None;
            record.summary.completed_at_ms = Some(now_ms());
            (record.request_id, record.summary.clone())
        };
        if self.active_run_id.as_deref() == Some(run_id) {
            self.active_run_id = None;
        }
        self.publish_summary(request_id, summary.clone());
        Some(summary)
    }

    fn snapshot(&mut self) -> ActionRuntimeSnapshot {
        ActionRuntimeSnapshot {
            pending_proposals: self.broker.pending(now_ms()),
            history: self.history(),
            active_run: self.active_run(),
        }
    }
}

fn lock_state(state: &Mutex<ActionRuntimeState>) -> MutexGuard<'_, ActionRuntimeState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn send_and_wake<T>(
    sender: &std_mpsc::Sender<T>,
    wake: Option<&ActionWakeHandler>,
    event: T,
) -> bool {
    if sender.send(event).is_err() {
        return false;
    }
    if let Some(wake) = wake {
        wake();
    }
    true
}

/// Join `worker` on a detached thread rather than the calling (UI) thread: the
/// executor may still be inside one more blocking, non-cancellable step.
fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-action-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

async fn execute_authorized_run(
    state: &Arc<Mutex<ActionRuntimeState>>,
    authorized: AuthorizedRun,
    executor: &dyn AuthorizedActionExecutor,
) -> ActionExecution {
    let run_id = authorized.run_id.clone();
    let proposal_id = authorized.grant.proposal().proposal_id.clone();
    let action_count = authorized.grant.proposal().actions.len();
    let mut items = Vec::with_capacity(action_count);

    for index in 0..action_count {
        if authorized.cancel.is_cancelled() {
            break;
        }
        let action = authorized
            .grant
            .action(index)
            .expect("index came from the grant's own action count");
        {
            let mut state = lock_state(state);
            let update = state.run_mut(&run_id).map(|record| {
                record.summary.current_index = Some(index);
                record.summary.actions[index].status = ActionItemStatus::Running;
                (record.request_id, record.summary.clone())
            });
            if let Some((request_id, summary)) = update {
                state.publish_summary(request_id, summary);
            } else {
                break;
            }
        }

        let result = executor.execute(action, &authorized.cancel).await;
        let stop = {
            let mut state = lock_state(state);
            let update = state.run_mut(&run_id).map(|record| {
                let stop = match &result {
                    Ok(result) => {
                        record.summary.actions[index].status =
                            item_status_for(result.completion_status);
                        record.summary.actions[index].result = Some(result.clone());
                        result.completion_status != FixCompletionStatus::Succeeded
                    }
                    Err(error) => {
                        record.summary.actions[index].status = ActionItemStatus::Failed;
                        record.summary.actions[index].error = Some(error.clone());
                        true
                    }
                };
                (stop, record.request_id, record.summary.clone())
            });
            match update {
                Some((stop, request_id, summary)) => {
                    state.publish_summary(request_id, summary);
                    stop
                }
                None => true,
            }
        };
        items.push(ActionExecutionItem {
            remediation: action.preview().remediation.clone(),
            issue_id: action.preview().issue_id.clone(),
            result,
        });
        if stop {
            break;
        }
    }

    let summary = {
        let mut state = lock_state(state);
        let update = state.run_mut(&run_id).map(|record| {
            // A late cancel must not rewrite a fully-completed successful run.
            let was_cancelled =
                cancellation_applies(&record.summary.actions, authorized.cancel.is_cancelled());
            for action in &mut record.summary.actions {
                if action.status == ActionItemStatus::Pending {
                    action.status = if was_cancelled {
                        ActionItemStatus::Cancelled
                    } else {
                        ActionItemStatus::Skipped
                    };
                }
            }
            record.summary.status = completed_run_status(&record.summary.actions, was_cancelled);
            record.summary.current_index = None;
            record.summary.completed_at_ms = Some(now_ms());
            (record.request_id, record.summary.clone())
        });
        let (request_id, summary) = update.expect("authorized run must remain in bounded history");
        if state.active_run_id.as_deref() == Some(run_id.as_str()) {
            state.active_run_id = None;
        }
        state.publish_summary(request_id, summary.clone());
        summary
    };

    ActionExecution {
        run_id,
        proposal_id,
        items,
        summary,
    }
}

/// UI-thread handle for the native action runtime.
pub struct NativeActionRuntime {
    /// Option so Drop can release the sender BEFORE joining the worker.
    commands: Option<std_mpsc::Sender<ActionCommand>>,
    state: Arc<Mutex<ActionRuntimeState>>,
    command_worker: Option<JoinHandle<()>>,
    execution_worker: Option<JoinHandle<()>>,
}

impl NativeActionRuntime {
    /// Start the workers.
    ///
    /// `wake` is invoked after each queued event so a native shell can
    /// schedule one coalesced UI drain instead of polling; pass `None` in
    /// portable tests.
    ///
    /// # Errors
    /// When a worker thread cannot be spawned.
    #[allow(clippy::too_many_lines)] // Both worker loops wired in one place.
    pub fn start(
        wake: Option<ActionWakeHandler>,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ActionWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ActionCommand>();
        let (events, event_rx) = std_mpsc::channel::<ActionWorkerEvent>();
        let (executions, execution_rx) = std_mpsc::channel::<AuthorizedRun>();
        let state = Arc::new(Mutex::new(ActionRuntimeState {
            wake: wake.clone(),
            ..ActionRuntimeState::default()
        }));

        let command_state = Arc::clone(&state);
        let command_events = events.clone();
        let command_wake = wake.clone();
        let command_worker = std::thread::Builder::new()
            .name("wfdiag-actions".to_string())
            .spawn(move || {
                let wake = command_wake.as_ref();
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ActionCommand::Prepare {
                            request_id,
                            input,
                            snapshot,
                        } => match lock_state(&command_state).broker.prepare(
                            input,
                            &snapshot,
                            now_ms(),
                        ) {
                            Ok(proposal) => {
                                send_and_wake(
                                    &command_events,
                                    wake,
                                    ActionWorkerEvent::Prepared {
                                        request_id,
                                        proposal,
                                    },
                                );
                            }
                            Err(message) => {
                                send_and_wake(
                                    &command_events,
                                    wake,
                                    ActionWorkerEvent::Failed {
                                        request_id,
                                        message,
                                    },
                                );
                            }
                        },
                        ActionCommand::Approve {
                            request_id,
                            proposal_id,
                            snapshot,
                            approval,
                        } => {
                            let authorized = lock_state(&command_state).authorize_run(
                                request_id,
                                &proposal_id,
                                &snapshot,
                                approval,
                                now_ms(),
                            );
                            match authorized {
                                Ok(grant) => {
                                    if let Err(error) = executions.send(grant) {
                                        let run_id = error.0.run_id;
                                        let message =
                                            "The native remediation executor stopped".to_string();
                                        lock_state(&command_state)
                                            .fail_run(&run_id, message.clone());
                                        send_and_wake(
                                            &command_events,
                                            wake,
                                            ActionWorkerEvent::Failed {
                                                request_id,
                                                message,
                                            },
                                        );
                                    }
                                }
                                Err(AuthorizationError::RepairConfirmationRequired(proposal)) => {
                                    send_and_wake(
                                        &command_events,
                                        wake,
                                        ActionWorkerEvent::NeedsRepairConfirmation {
                                            request_id,
                                            proposal,
                                        },
                                    );
                                }
                                Err(AuthorizationError::Rejected(message)) => {
                                    send_and_wake(
                                        &command_events,
                                        wake,
                                        ActionWorkerEvent::Failed {
                                            request_id,
                                            message,
                                        },
                                    );
                                }
                            }
                        }
                        ActionCommand::Discard { proposal_id } => {
                            let _ = lock_state(&command_state).broker.discard(&proposal_id);
                        }
                    }
                }
            })?;

        let execution_state = Arc::clone(&state);
        let execution_worker = match std::thread::Builder::new()
            .name("wfdiag-action-executor".to_string())
            .spawn(move || {
                let wake = wake.as_ref();
                let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                while let Ok(authorized) = execution_rx.recv() {
                    let request_id = authorized.request_id;
                    if let Ok(tokio_runtime) = &tokio_runtime {
                        let execution = tokio_runtime.block_on(execute_authorized_run(
                            &execution_state,
                            authorized,
                            &RealCatalogExecutor,
                        ));
                        send_and_wake(
                            &events,
                            wake,
                            ActionWorkerEvent::Done {
                                request_id,
                                execution,
                            },
                        );
                    } else {
                        let message = "Could not start the native remediation executor".to_string();
                        lock_state(&execution_state).fail_run(&authorized.run_id, message.clone());
                        send_and_wake(
                            &events,
                            wake,
                            ActionWorkerEvent::Failed {
                                request_id,
                                message,
                            },
                        );
                    }
                }
            }) {
            Ok(worker) => worker,
            Err(error) => {
                drop(commands);
                let _ = command_worker.join();
                return Err(error);
            }
        };
        Ok((
            Self {
                commands: Some(commands),
                state,
                command_worker: Some(command_worker),
                execution_worker: Some(execution_worker),
            },
            event_rx,
        ))
    }

    /// Stage catalog IDs into a reviewable immutable proposal.
    ///
    /// Returns `false` only when the worker queue is unavailable.
    #[must_use]
    pub fn prepare(
        &self,
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    ) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Prepare {
                    request_id,
                    input,
                    snapshot,
                })
                .is_ok()
        })
    }

    /// Approve an opaque proposal against a freshly captured authoritative
    /// snapshot. Repair proposals require [`ActionApproval::RepairConfirmed`].
    ///
    /// Returns `false` only when the worker queue is unavailable.
    #[must_use]
    pub fn approve(
        &self,
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    ) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Approve {
                    request_id,
                    proposal_id,
                    snapshot,
                    approval,
                })
                .is_ok()
        })
    }

    /// Drop an unused proposal after the review UI is dismissed.
    #[must_use]
    pub fn discard(&self, proposal_id: String) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Discard { proposal_id })
                .is_ok()
        })
    }

    /// Signal cancellation without waiting behind the currently executing
    /// remediation. Cancellation is accepted only before a non-cancellable
    /// catalog action starts.
    ///
    /// # Errors
    /// When the run is unknown or its current action cannot be stopped safely.
    pub fn cancel(&self, run_id: &str) -> Result<ActionRunSummary, String> {
        lock_state(&self.state).cancel_run(run_id)
    }

    /// Current projection for one addressable run.
    #[must_use]
    pub fn get_status(&self, run_id: &str) -> Option<ActionRunSummary> {
        lock_state(&self.state)
            .run(run_id)
            .map(|record| record.summary.clone())
    }

    /// Newest-first bounded in-memory audit history, retained across view
    /// unmount/remount for the lifetime of the native application.
    #[must_use]
    pub fn list_history(&self) -> Vec<ActionRunSummary> {
        lock_state(&self.state).history()
    }

    /// Unconsumed, unexpired proposals for staged-action rehydration.
    #[must_use]
    pub fn list_pending_proposals(&self) -> Vec<ActionProposal> {
        lock_state(&self.state).broker.pending(now_ms())
    }

    /// Atomically attach a live observer and capture pending/history/active
    /// state. The caller can render the snapshot, then drain events without a
    /// race window between those operations.
    #[must_use]
    pub fn subscribe_run_events(
        &self,
    ) -> (std_mpsc::Receiver<ActionRunEvent>, ActionRuntimeSnapshot) {
        let (sender, receiver) = std_mpsc::channel();
        let mut state = lock_state(&self.state);
        state.subscribers.push(sender);
        let snapshot = state.snapshot();
        (receiver, snapshot)
    }
}

impl Drop for NativeActionRuntime {
    fn drop(&mut self) {
        // Release the command sender BEFORE reaping: the command worker
        // drains, exits, and its closure drops the last `executions` sender,
        // which is what disconnects the execution worker's receive loop.
        // Without that disconnect the executor reaper would wait on a worker
        // that can never observe shutdown.
        self.commands = None;
        // Best-effort cancellation of an in-flight run. A step already started
        // and flagged non-cancellable keeps running to completion, which is
        // exactly why the joins happen on a detached reaper rather than here.
        let active_run_id = lock_state(&self.state).active_run_id.clone();
        if let Some(run_id) = active_run_id {
            // "Not found" and "cannot be stopped safely" are both fine here.
            let _ = lock_state(&self.state).cancel_run(&run_id);
        }
        // Independent reapers keep graceful close off the UI thread: the
        // executor may be inside one more blocking step even after
        // cancellation, and the command worker exits on its own once the
        // sender above drops, disconnecting the executor's receive loop.
        if let Some(worker) = self.command_worker.take() {
            reap_worker(worker);
        }
        if let Some(worker) = self.execution_worker.take() {
            reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::ACTION_PROPOSAL_TTL_MS;
    use crate::broker::{
        ActionPreview, ActionRequest, ApprovalScope, current_action_catalog_fingerprint,
    };
    use crate::remediation::{RemediationStepResult, RemediationStepStatus, find};
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;

    fn snapshot(scan_fingerprint: &str, is_admin: bool) -> ActionSnapshot {
        ActionSnapshot {
            scan_fingerprint: scan_fingerprint.to_string(),
            catalog_fingerprint: current_action_catalog_fingerprint(),
            detected_issues: Vec::new(),
            is_admin,
        }
    }

    fn request(remediation_id: &str, issue_id: Option<&str>) -> ActionPrepareInput {
        ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id: remediation_id.to_string(),
                issue_id: issue_id.map(str::to_string),
            }],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        }
    }

    fn preview(remediation_id: &str) -> ActionPreview {
        let spec = find(remediation_id).expect("test remediation must exist");
        ActionPreview {
            remediation: spec.summary(),
            issue_id: None,
            steps: spec.preview_steps(),
        }
    }

    /// A grant for run-projection tests. It uses the broker's test-only
    /// constructor so these fixtures can cover tier combinations the approval
    /// path refuses; the gate itself is tested in `broker`.
    fn grant(remediation_ids: &[&str]) -> ActionGrant {
        ActionGrant::for_tests(remediation_ids.iter().map(|id| preview(id)).collect())
    }

    fn fix_result(
        completion_status: FixCompletionStatus,
        requires_restart: bool,
        steps: Vec<RemediationStepResult>,
    ) -> FixResult {
        FixResult {
            success: matches!(completion_status, FixCompletionStatus::Succeeded),
            message: format!("{completion_status:?}"),
            actions_taken: steps.iter().map(|step| step.action.clone()).collect(),
            requires_restart,
            completion_status,
            steps,
        }
    }

    fn step(action: &str, status: RemediationStepStatus) -> RemediationStepResult {
        RemediationStepResult {
            action: action.to_string(),
            status,
            detail: Some(format!("{action} detail")),
        }
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    struct SequenceExecutor {
        outcomes: Mutex<VecDeque<Result<FixResult, String>>>,
        calls: Mutex<Vec<String>>,
    }

    impl SequenceExecutor {
        fn new(outcomes: impl IntoIterator<Item = Result<FixResult, String>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl AuthorizedActionExecutor for SequenceExecutor {
        fn execute<'a>(
            &'a self,
            action: crate::broker::AuthorizedAction<'a>,
            _cancel: &'a CancellationToken,
        ) -> crate::broker::ExecutionFuture<'a> {
            self.calls
                .lock()
                .unwrap()
                .push(action.remediation_id().to_string());
            let outcome = self
                .outcomes
                .lock()
                .unwrap()
                .pop_front()
                .expect("test executor needs one outcome per call");
            Box::pin(async move { outcome })
        }
    }

    #[derive(Default)]
    struct RecordingExecutor {
        calls: Mutex<Vec<String>>,
    }

    impl AuthorizedActionExecutor for RecordingExecutor {
        fn execute<'a>(
            &'a self,
            action: crate::broker::AuthorizedAction<'a>,
            _cancel: &'a CancellationToken,
        ) -> crate::broker::ExecutionFuture<'a> {
            let remediation_id = action.remediation_id().to_string();
            self.calls.lock().unwrap().push(remediation_id.clone());
            let result = FixResult {
                success: true,
                message: "recorded".to_string(),
                actions_taken: vec![remediation_id],
                requires_restart: false,
                completion_status: FixCompletionStatus::Succeeded,
                steps: Vec::new(),
            };
            Box::pin(async move { Ok(result) })
        }
    }

    struct WaitForCancellationExecutor {
        started: Mutex<Option<std_mpsc::Sender<()>>>,
    }

    impl AuthorizedActionExecutor for WaitForCancellationExecutor {
        fn execute<'a>(
            &'a self,
            action: crate::broker::AuthorizedAction<'a>,
            cancel: &'a CancellationToken,
        ) -> crate::broker::ExecutionFuture<'a> {
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let label = action.remediation_id().to_string();
            Box::pin(async move {
                cancel.cancelled().await;
                Ok(fix_result(
                    FixCompletionStatus::Cancelled,
                    false,
                    vec![step(&label, RemediationStepStatus::Cancelled)],
                ))
            })
        }
    }

    #[test]
    fn the_grant_fixture_carries_the_expected_preview() {
        let grant = grant(&["flush_dns"]);
        assert_eq!(grant.proposal().approval_scope, ApprovalScope::Exact);
        assert_eq!(
            grant.proposal().actions[0].steps,
            preview("flush_dns").steps
        );
        assert_eq!(
            grant.proposal().expires_at_ms,
            1_000 + ACTION_PROPOSAL_TTL_MS
        );
        assert_eq!(grant.actions().len(), 1);
        assert_eq!(grant.action(0).unwrap().remediation_id(), "flush_dns");
        assert!(grant.action(1).is_none());
    }

    #[test]
    fn structured_progress_preserves_steps_partial_and_restart_semantics() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let (updates, update_rx) = std_mpsc::channel();
        let authorized = {
            let mut state = lock_state(&state);
            state.subscribers.push(updates);
            state
                .record_grant(42, grant(&["flush_dns", "clear_icon_cache"]), 2_000)
                .unwrap()
        };
        let initial = update_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(initial.request_id, 42);
        assert_eq!(initial.summary.status, ActionRunStatus::Running);
        assert_eq!(initial.summary.current_index, None);
        assert!(
            initial
                .summary
                .actions
                .iter()
                .all(|action| action.status == ActionItemStatus::Pending)
        );

        let executor = SequenceExecutor::new([
            Ok(fix_result(
                FixCompletionStatus::Succeeded,
                false,
                vec![step("Flush DNS", RemediationStepStatus::AlreadySatisfied)],
            )),
            Ok(fix_result(
                FixCompletionStatus::Partial,
                true,
                vec![
                    step("Stop Explorer", RemediationStepStatus::Succeeded),
                    step("Delete cache", RemediationStepStatus::Failed),
                ],
            )),
        ]);
        let execution = block_on(execute_authorized_run(&state, authorized, &executor));

        assert_eq!(execution.summary.status, ActionRunStatus::Partial);
        assert_eq!(execution.summary.current_index, None);
        assert_eq!(
            execution
                .summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [ActionItemStatus::Succeeded, ActionItemStatus::Partial]
        );
        assert_eq!(
            execution.summary.actions[0].result.as_ref().unwrap().steps[0].status,
            RemediationStepStatus::AlreadySatisfied
        );
        assert_eq!(
            execution.summary.actions[1].result.as_ref().unwrap().steps[1].status,
            RemediationStepStatus::Failed
        );
        assert!(execution.summary.requires_restart());
        assert!(lock_state(&state).active_run_id.is_none());

        let events = update_rx.try_iter().collect::<Vec<_>>();
        assert!(events.iter().any(|event| {
            event.summary.current_index == Some(0)
                && event.summary.actions[0].status == ActionItemStatus::Running
        }));
        assert!(events.iter().any(|event| {
            event.summary.current_index == Some(1)
                && event.summary.actions[1].status == ActionItemStatus::Running
        }));
        assert_eq!(
            events.last().unwrap().summary.status,
            ActionRunStatus::Partial
        );
    }

    #[test]
    fn failure_after_success_is_partial_and_later_actions_are_skipped() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(
                7,
                grant(&["flush_dns", "clear_icon_cache", "open_task_manager"]),
                2_000,
            )
            .unwrap();
        let executor = SequenceExecutor::new([
            Ok(fix_result(
                FixCompletionStatus::Succeeded,
                false,
                vec![step("Flush DNS", RemediationStepStatus::Succeeded)],
            )),
            Err("cache command failed".to_string()),
        ]);
        let execution = block_on(execute_authorized_run(&state, authorized, &executor));

        assert_eq!(execution.summary.status, ActionRunStatus::Partial);
        assert_eq!(
            execution
                .summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [
                ActionItemStatus::Succeeded,
                ActionItemStatus::Failed,
                ActionItemStatus::Skipped,
            ]
        );
        assert_eq!(
            execution.summary.actions[1].error.as_deref(),
            Some("cache command failed")
        );
        assert_eq!(execution.items.len(), 2);
        assert_eq!(
            executor.calls.lock().unwrap().as_slice(),
            ["flush_dns", "clear_icon_cache"]
        );
    }

    #[test]
    fn infrastructure_failure_after_completed_work_is_partial() {
        let mut state = ActionRuntimeState::default();
        let authorized = state
            .record_grant(
                71,
                grant(&["flush_dns", "clear_icon_cache", "open_task_manager"]),
                2_000,
            )
            .unwrap();
        {
            let record = state.run_mut(&authorized.run_id).unwrap();
            record.summary.actions[0].status = ActionItemStatus::Succeeded;
            record.summary.current_index = Some(1);
            record.summary.actions[1].status = ActionItemStatus::Running;
        }

        let summary = state
            .fail_run(&authorized.run_id, "executor stopped".to_string())
            .unwrap();

        assert_eq!(summary.status, ActionRunStatus::Partial);
        assert_eq!(
            summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [
                ActionItemStatus::Succeeded,
                ActionItemStatus::Failed,
                ActionItemStatus::Skipped,
            ]
        );
        assert_eq!(
            summary.actions[1].error.as_deref(),
            Some("executor stopped")
        );
        assert!(state.active_run_id.is_none());
    }

    #[test]
    fn cancellation_before_execution_marks_every_unfinished_action_cancelled() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(8, grant(&["flush_dns", "clear_icon_cache"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        let cancelled = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(cancelled.status, ActionRunStatus::CancelRequested);
        let executor = RecordingExecutor::default();
        let execution = block_on(execute_authorized_run(&state, authorized, &executor));

        assert_eq!(execution.summary.status, ActionRunStatus::Cancelled);
        assert!(
            execution
                .summary
                .actions
                .iter()
                .all(|action| action.status == ActionItemStatus::Cancelled)
        );
        assert!(execution.items.is_empty());
        assert!(executor.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn in_flight_cancel_is_immediate_and_publishes_cancel_requested_then_terminal() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let (subscriber, updates) = std_mpsc::channel();
        let authorized = {
            let mut state = lock_state(&state);
            state.subscribers.push(subscriber);
            state.record_grant(9, grant(&["flush_dns"]), 2_000).unwrap()
        };
        let run_id = authorized.run_id.clone();
        let (started_tx, started_rx) = std_mpsc::channel();
        let execution_state = Arc::clone(&state);
        let execution_thread = std::thread::spawn(move || {
            let executor = WaitForCancellationExecutor {
                started: Mutex::new(Some(started_tx)),
            };
            block_on(execute_authorized_run(
                &execution_state,
                authorized,
                &executor,
            ))
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancel_projection = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(cancel_projection.status, ActionRunStatus::CancelRequested);
        let execution = execution_thread.join().unwrap();
        assert_eq!(execution.summary.status, ActionRunStatus::Cancelled);
        assert_eq!(
            execution.summary.actions[0].status,
            ActionItemStatus::Cancelled
        );

        let statuses = updates
            .try_iter()
            .map(|event| event.summary.status)
            .collect::<Vec<_>>();
        assert!(statuses.contains(&ActionRunStatus::CancelRequested));
        assert_eq!(statuses.last(), Some(&ActionRunStatus::Cancelled));
    }

    #[test]
    fn cancellation_rejects_a_started_non_cancellable_action() {
        let mut state = ActionRuntimeState::default();
        let authorized = state
            .record_grant(10, grant(&["sfc_scannow"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        {
            let record = state.run_mut(&run_id).unwrap();
            record.summary.current_index = Some(0);
            record.summary.actions[0].status = ActionItemStatus::Running;
        }
        let error = state.cancel_run(&run_id).unwrap_err();
        assert!(error.contains("cannot be stopped safely"));
        assert!(!authorized.cancel.is_cancelled());
        assert_eq!(
            state.run(&run_id).unwrap().summary.status,
            ActionRunStatus::Running
        );
    }

    #[test]
    fn late_cancel_does_not_rewrite_a_successful_terminal_run() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(11, grant(&["flush_dns"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        let executor = RecordingExecutor::default();
        let execution = block_on(execute_authorized_run(&state, authorized, &executor));
        assert_eq!(execution.summary.status, ActionRunStatus::Succeeded);

        let after_cancel = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(after_cancel.status, ActionRunStatus::Succeeded);
        assert_eq!(after_cancel.actions[0].status, ActionItemStatus::Succeeded);
    }

    #[test]
    fn concurrent_run_rejection_does_not_consume_the_second_proposal() {
        let current = snapshot("scan-a", true);
        let mut state = ActionRuntimeState::default();
        let first = state
            .broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        let second = state
            .broker
            .prepare(request("flush_dns", None), &current, 1_001)
            .unwrap();
        let active = state
            .authorize_run(
                1,
                &first.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_002,
            )
            .unwrap();

        let rejected = state.authorize_run(
            2,
            &second.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_003,
        );
        assert!(matches!(
            rejected,
            Err(AuthorizationError::Rejected(message)) if message.contains("already running")
        ));
        assert!(
            state
                .broker
                .pending(1_004)
                .iter()
                .any(|proposal| proposal.proposal_id == second.proposal_id)
        );

        state.fail_run(&active.run_id, "test completion".to_string());
        assert!(
            state
                .authorize_run(
                    2,
                    &second.proposal_id,
                    &current,
                    ActionApproval::Reviewed,
                    1_005,
                )
                .is_ok()
        );
    }

    /// #185: the worker path refuses a Repair without the second confirmation
    /// and never reserves a run slot for it.
    #[test]
    fn worker_authorization_refuses_an_unconfirmed_repair() {
        let current = snapshot("scan-a", true);
        let mut state = ActionRuntimeState::default();
        let proposal = state
            .broker
            .prepare(request("sfc_scannow", None), &current, 1_000)
            .unwrap();

        let refused = state.authorize_run(
            1,
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_001,
        );
        assert!(matches!(
            refused,
            Err(AuthorizationError::RepairConfirmationRequired(review)) if review == proposal
        ));
        assert!(state.active_run_id.is_none());
        assert!(state.runs.is_empty());

        state
            .authorize_run(
                2,
                &proposal.proposal_id,
                &current,
                ActionApproval::RepairConfirmed,
                1_002,
            )
            .unwrap();
        assert!(state.active_run_id.is_some());
    }

    #[test]
    fn history_is_newest_first_bounded_and_atomic_snapshot_keeps_pending_state() {
        let mut state = ActionRuntimeState::default();
        let current = snapshot("scan-a", true);
        let pending = state
            .broker
            .prepare(request("flush_dns", None), &current, now_ms())
            .unwrap();
        let mut run_ids = Vec::new();
        for index in 0..=MAX_ACTION_HISTORY {
            let index = u64::try_from(index).unwrap();
            let authorized = state
                .record_grant(index, grant(&["flush_dns"]), index)
                .unwrap();
            run_ids.push(authorized.run_id.clone());
            state.fail_run(&authorized.run_id, format!("failure {index}"));
        }
        assert_eq!(state.runs.len(), MAX_ACTION_HISTORY);
        assert!(state.run(&run_ids[0]).is_none());

        let (subscriber, updates) = std_mpsc::channel();
        state.subscribers.push(subscriber);
        let captured = state.snapshot();
        assert_eq!(captured.history.len(), MAX_ACTION_HISTORY);
        assert_eq!(captured.history[0].run_id, *run_ids.last().unwrap());
        assert_eq!(captured.history.last().unwrap().run_id, run_ids[1]);
        assert!(captured.active_run.is_none());
        assert!(
            captured
                .pending_proposals
                .iter()
                .any(|proposal| proposal.proposal_id == pending.proposal_id)
        );

        let live = state
            .record_grant(999, grant(&["flush_dns"]), 9_999)
            .unwrap();
        let update = updates.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(update.request_id, 999);
        assert_eq!(update.summary.run_id, live.run_id);
        assert_eq!(update.summary.status, ActionRunStatus::Running);
    }

    fn item(status: ActionItemStatus) -> ActionItemRun {
        ActionItemRun {
            remediation_id: "test".to_string(),
            label: "Test".to_string(),
            cancellable: true,
            status,
            result: None,
            error: None,
        }
    }

    #[test]
    fn aggregate_status_table_matches_shipping_semantics() {
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Succeeded)], false),
            ActionRunStatus::Succeeded
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Failed)], false),
            ActionRunStatus::Failed
        );
        assert_eq!(
            completed_run_status(
                &[
                    item(ActionItemStatus::Succeeded),
                    item(ActionItemStatus::Failed),
                ],
                false,
            ),
            ActionRunStatus::Partial
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Partial)], false),
            ActionRunStatus::Partial
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Pending)], true),
            ActionRunStatus::Cancelled
        );
        assert!(!cancellation_applies(
            &[item(ActionItemStatus::Succeeded)],
            true
        ));
        assert!(cancellation_applies(
            &[
                item(ActionItemStatus::Succeeded),
                item(ActionItemStatus::Pending),
            ],
            true,
        ));
    }

    #[test]
    fn run_summary_wire_shape_is_camel_case_and_skips_empty_optionals() {
        let summary = ActionRunSummary {
            run_id: "run_1".to_string(),
            proposal_id: "proposal_1".to_string(),
            authorization_id: "grant_1".to_string(),
            status: ActionRunStatus::CancelRequested,
            actions: vec![item(ActionItemStatus::Running)],
            current_index: None,
            approved_at_ms: 5,
            completed_at_ms: None,
            scan_fingerprint: "scan".to_string(),
            catalog_fingerprint: "catalog".to_string(),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert_eq!(
            json,
            r#"{"runId":"run_1","proposalId":"proposal_1","authorizationId":"grant_1","status":"cancel_requested","actions":[{"remediationId":"test","label":"Test","cancellable":true,"status":"running"}],"approvedAtMs":5,"scanFingerprint":"scan","catalogFingerprint":"catalog"}"#
        );
    }

    #[test]
    fn public_runtime_rehydrates_pending_proposals_without_executing_them() {
        let (runtime, legacy_events) = NativeActionRuntime::start(None).unwrap();
        let (run_events, initial) = runtime.subscribe_run_events();
        assert!(initial.pending_proposals.is_empty());
        assert!(initial.history.is_empty());
        assert!(initial.active_run.is_none());
        assert!(runtime.prepare(123, request("flush_dns", None), snapshot("scan-a", true)));
        let prepared = match legacy_events.recv_timeout(Duration::from_secs(2)) {
            Ok(ActionWorkerEvent::Prepared { proposal, .. }) => proposal,
            Ok(other) => panic!("expected Prepared, got {other:?}"),
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for preparation"),
            Err(RecvTimeoutError::Disconnected) => panic!("action worker disconnected"),
        };
        assert!(
            runtime
                .list_pending_proposals()
                .iter()
                .any(|proposal| proposal.proposal_id == prepared.proposal_id)
        );
        assert!(runtime.list_history().is_empty());
        assert!(run_events.try_recv().is_err());
        assert!(runtime.discard(prepared.proposal_id));
    }

    #[test]
    fn the_wake_handler_fires_for_every_queued_worker_event() {
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let (runtime, events) = NativeActionRuntime::start(Some(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })))
        .unwrap();
        assert!(runtime.prepare(1, request("flush_dns", None), snapshot("scan-a", true)));
        events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(wakes.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    }
}
