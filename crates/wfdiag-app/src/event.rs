//! The single event stream out of the application service.
//!
//! Events are facts, not instructions: every staleness comparison has already
//! happened inside [`crate::AppService::drain`], so an event a host receives is
//! by construction current. The [`AppEventReceiver`] is the host's handle to
//! that stream — it registers the wake callback and reports termination.

use crate::command::WorkerKind;
use crate::ids::RequestId;
use crate::ports::monitor::{NetworkConnection, ProcessPage};
use crate::snapshot::AppSnapshot;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_ai_chat::{
    ChatToolActivity, ChatToolHistory, ProviderUse, SubscriptionAuthOperation,
    SubscriptionAuthProvider, SubscriptionAuthStatus, SubscriptionInstallFallbackReason,
    SubscriptionInstallMethod, SubscriptionInstallProgress, SubscriptionInstallStatus,
};
use wfdiag_native_ai_provider::{AIProviderStatus, ModelCatalog};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_export::ExportPayload;
use wfdiag_native_history::{
    ComparisonResult, ComparisonSummary, ScanRecord, ScanSummary, TaskDiffDetail, TaskTrend,
};
use wfdiag_native_issues::Issue;
use wfdiag_native_remediation::broker::ActionProposal;
use wfdiag_native_remediation::runtime::ActionRunSummary;
use wfdiag_native_settings::AppSettings;
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use wfdiag_native_update::UpdateOutcome;
use wfdiag_ui_core::{SystemStats, TaskProgressStatus, UiWakeHandler};

/// Diagnostic-scan facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    /// The runtime created a session and execution began.
    Started {
        /// The session identity.
        session_id: String,
        /// Which scan is running.
        kind: ScanKind,
        /// How many tasks it will run.
        total: usize,
    },
    /// The session could not be created; previous evidence is visible again.
    StartFailed {
        /// The runtime's diagnostic.
        error: String,
    },
    /// One task changed state.
    Progress {
        /// The task.
        task_id: String,
        /// Its new state.
        status: TaskProgressStatus,
        /// The task's display name, when the runtime supplied one.
        task_name: Option<String>,
        /// Results collected so far.
        completed: usize,
        /// Tasks expected.
        total: usize,
    },
    /// One task's output arrived.
    TaskResult {
        /// The task.
        task_id: String,
        /// Whether it succeeded.
        success: bool,
        /// Results collected so far.
        completed: usize,
        /// Failures so far.
        errors: usize,
    },
    /// A complete replacement scan committed.
    Committed {
        /// The committed session.
        session_id: String,
        /// Results collected.
        completed: usize,
        /// Failures.
        errors: usize,
        /// Wall-clock duration.
        duration_ms: u64,
        /// Whether history auto-save is running for this scan.
        auto_save: bool,
    },
    /// A single-row rerun merged into the committed scan.
    TargetedCommitted {
        /// The committed session (the base scan's).
        session_id: String,
        /// The task that was replaced.
        task_id: String,
    },
    /// The rerun could not be merged; the base scan is visible again.
    TargetedFailed {
        /// Why the merge was refused.
        error: String,
    },
    /// The scan was stopped; previous evidence is visible again.
    Cancelled,
    /// The runtime accepted the cancellation. Queued tasks will skip; tasks
    /// already in flight still finish, so this is not the terminal event.
    CancelAcknowledged,
    /// The runtime refused the cancellation; the run continues.
    CancelFailed {
        /// The runtime's diagnostic.
        error: String,
    },
    /// The run failed; previous evidence is visible again.
    Failed {
        /// The runtime's diagnostic.
        error: String,
        /// Whether the user had asked to stop.
        stopped: bool,
    },
    /// The runtime returned a result set that is not the requested task set.
    Incomplete {
        /// Results returned.
        completed: usize,
        /// Tasks requested.
        expected: usize,
    },
    /// The scan finished, including optional history persistence.
    Finalized {
        /// The committed session.
        session_id: String,
        /// The auto-save result, when auto-save ran.
        history: Option<Result<(), String>>,
    },
}

/// Issue-detection facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssuesEvent {
    /// A new projection replaced the visible issues.
    Updated {
        /// The evidence this projection came from.
        session_id: String,
        /// The issues themselves.
        issues: Vec<Issue>,
    },
    /// Detection could not run or its reply was lost. The previously
    /// projected issues deliberately stay visible.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Which history request a failure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HistoryRequest {
    /// Listing stored scans.
    List,
    /// Loading one scan.
    Load,
    /// Comparing two scans.
    Compare,
    /// Comparing the committed scan with the newest stored one.
    CompareToLatest,
    /// Fetching one task's two outputs.
    TaskDiff,
    /// Saving a label.
    Label,
    /// Saving tags.
    Tags,
    /// Computing failure trends.
    Trends,
    /// Deleting every stored scan.
    Clear,
    /// Persisting a completed scan.
    AutoSave,
}

/// Scan-history facts.
#[derive(Clone, Debug)]
pub enum HistoryEvent {
    /// The stored scans, newest first.
    Listed {
        /// The summaries.
        scans: Vec<ScanSummary>,
    },
    /// One stored scan, in full.
    Loaded {
        /// The record.
        scan: Box<ScanRecord>,
    },
    /// A full comparison.
    Compared {
        /// The comparison.
        comparison: Box<ComparisonResult>,
    },
    /// A lightweight comparison.
    ComparedSummary {
        /// The summary comparison.
        comparison: Box<ComparisonSummary>,
    },
    /// The committed scan compared against the newest stored scan.
    ComparedToLatest {
        /// `None` when no other scan is stored.
        comparison: Option<Box<ComparisonResult>>,
    },
    /// Both outputs for one task.
    TaskDiff {
        /// The diff.
        diff: Box<TaskDiffDetail>,
    },
    /// Per-task failure trends.
    Trends {
        /// The trends.
        trends: Vec<TaskTrend>,
    },
    /// A label was saved.
    LabelSaved {
        /// The scan.
        scan_id: String,
    },
    /// Tags were saved.
    TagsSaved {
        /// The scan.
        scan_id: String,
    },
    /// A completed scan was persisted by the auto-save policy.
    ScanSaved {
        /// The stored scan id (the diagnostic session id).
        scan_id: String,
    },
    /// Every stored scan was deleted.
    Cleared,
    /// One history request failed.
    Failed {
        /// Which request.
        request: HistoryRequest,
        /// The diagnostic.
        error: String,
    },
}

/// Live-monitoring facts.
#[derive(Clone, Debug)]
pub enum MonitorEvent {
    /// A new telemetry sample.
    Stats(Box<SystemStats>),
    /// A requested process page.
    ProcessPage(Box<ProcessPage>),
    /// A newer process query replaced this one before it ran.
    ProcessPageSuperseded,
    /// The current network connections.
    NetworkConnections(Vec<NetworkConnection>),
    /// Sampling was paused or resumed.
    PausedChanged {
        /// Whether sampling is paused.
        paused: bool,
    },
    /// Live monitoring is not running.
    Unavailable {
        /// Why.
        reason: String,
    },
}

/// AI-provider facts.
#[derive(Clone, Debug)]
pub enum ProviderEvent {
    /// A fresh provider status.
    Status(Box<AIProviderStatus>),
    /// A preference was applied; the status is the post-mutation projection.
    PreferenceApplied {
        /// The applied preference wire id.
        preference: String,
        /// The status projected after the mutation.
        status: Box<AIProviderStatus>,
    },
    /// A preference was refused.
    PreferenceRejected {
        /// The user-facing reason.
        reason: String,
    },
    /// Cached AI responses were dropped.
    CacheCleared,
    /// The local Ollama model list.
    OllamaModels(Vec<String>),
    /// A live model-catalog fact.
    ModelCatalog(ModelCatalogEvent),
    /// A subscription-CLI account or installation fact.
    Subscription(Box<SubscriptionEvent>),
    /// A provider request failed.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Settings facts.
#[derive(Clone, Debug)]
pub enum SettingsEvent {
    /// The settings document was (re)loaded.
    Loaded {
        /// The document, with credential availability filled in.
        settings: Box<AppSettings>,
    },
    /// A complete document was persisted.
    Saved {
        /// The persisted document.
        settings: Box<AppSettings>,
    },
    /// One typed mutation was applied and persisted.
    Updated {
        /// The resulting document.
        settings: Box<AppSettings>,
    },
    /// Staged provider credentials were committed.
    CredentialsCommitted,
    /// A settings request failed.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Export-rendering facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportEvent {
    /// A rendered payload.
    Completed {
        /// The payload.
        payload: Box<ExportPayload>,
    },
    /// Rendering failed.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Update-channel facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// A completed check, including the deliberately silent outcomes.
    Checked(Box<UpdateOutcome>),
    /// The once-a-day throttle skipped the passive check.
    Throttled,
    /// The check could not be queued.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Host-identity facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemEvent {
    /// Host identity.
    Info(Box<SystemInfo>),
    /// CPU architecture.
    Architecture(Box<ArchitectureSnapshot>),
    /// An elevated relaunch was attempted.
    ElevationAttempted {
        /// Whether the elevated process started.
        restarted: bool,
    },
    /// A system request failed.
    Failed {
        /// The diagnostic.
        error: String,
    },
}

/// Agentic-chat facts.
///
/// Text arrives as [`ChatEvent::Delta`], **coalesced per drain**: however many
/// fragments a provider streamed between two [`crate::AppService::drain`]
/// calls, a host receives one delta. Exactly one terminal event
/// ([`ChatEvent::Done`], [`ChatEvent::Failed`], [`ChatEvent::Cancelled`]) ends
/// a turn, and an `Auto` fallback retry is invisible: the retried attempt is
/// the same logical turn.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChatEvent {
    /// A turn was accepted and is streaming from `provider`.
    Started {
        /// The provider handling this attempt, as a wire id.
        provider: String,
    },
    /// The turn is queued behind a prerequisite (provider probe, scan).
    Deferred {
        /// What the turn is waiting for.
        reason: String,
    },
    /// Coalesced assistant text produced since the previous drain.
    Delta {
        /// The text to append.
        text: String,
    },
    /// A model-requested tool changed state.
    ToolActivity {
        /// The activity that changed.
        activity: Box<ChatToolActivity>,
        /// The ordered history as of this update.
        history: Box<ChatToolHistory>,
    },
    /// The model staged a remediation. Nothing was executed: the host must
    /// still run the normal prepare/approve flow.
    ProposalStaged {
        /// The catalog id the model named.
        remediation_id: String,
        /// The detected issue it was staged against.
        issue_id: Option<String>,
    },
    /// The model asked for a Full Scan. No scan was started.
    FullScanRequested {
        /// The scan the request was attributed to.
        source_scan_id: String,
        /// Why the model wants more evidence.
        reason: String,
    },
    /// A local provider failed cleanly and continuing would cross into cloud
    /// execution. Answer with [`crate::AppCommand::CloudFallbackDecision`].
    CloudFallbackRequired {
        /// The cloud provider that would run next, as a wire id.
        candidate: String,
        /// The local provider's failure.
        reason: String,
    },
    /// The turn finished.
    Done {
        /// The provider that answered, as a wire id.
        provider: String,
        /// Trust and model attribution, including any fallback.
        provider_use: Box<ProviderUse>,
        /// The engine's finish reason.
        finish_reason: String,
        /// The reconciled tool history.
        tool_history: Box<ChatToolHistory>,
    },
    /// The turn failed. No further events follow for it.
    Failed {
        /// The diagnostic.
        message: String,
    },
    /// The turn was cancelled.
    Cancelled,
    /// The conversation was cleared.
    SessionReset,
}

/// AI scan-report facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportEvent {
    /// Generation began on `provider`.
    Started {
        /// The provider, as a wire id.
        provider: String,
    },
    /// The turn is queued behind a prerequisite.
    Deferred {
        /// What the report is waiting for.
        reason: String,
    },
    /// Coalesced report text produced since the previous drain.
    Delta {
        /// The text to append.
        text: String,
    },
    /// The cached report for this evidence, returned inline without streaming.
    Cached {
        /// The provider that originally produced it, as a wire id.
        provider: String,
        /// The complete report body.
        report: String,
    },
    /// Generation finished.
    Done {
        /// The provider, as a wire id.
        provider: String,
        /// The engine's finish reason.
        finish_reason: String,
        /// Trust and model attribution.
        provider_use: Box<ProviderUse>,
    },
    /// Generation failed.
    Failed {
        /// The diagnostic.
        message: String,
    },
    /// Generation was cancelled.
    Cancelled,
    /// A new scan replaced the evidence this report described.
    Invalidated,
}

/// Per-task AI analysis facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnalysisEvent {
    /// Analysis began.
    Started {
        /// The task being explained.
        task_id: String,
        /// The provider, as a wire id.
        provider: String,
        /// Whether the answer came from the response cache.
        cached: bool,
    },
    /// An interpretation arrived.
    Completed {
        /// The task.
        task_id: String,
        /// The interpretation.
        interpretation: String,
        /// Whether it came from the response cache.
        cached: bool,
        /// Trust and model attribution.
        provider_use: Box<ProviderUse>,
    },
    /// Analysis failed.
    Failed {
        /// The task.
        task_id: String,
        /// The diagnostic.
        message: String,
        /// Whether retrying could succeed.
        retryable: bool,
    },
    /// Analysis was cancelled.
    Cancelled {
        /// The task.
        task_id: String,
    },
    /// A new scan replaced the evidence the analyses described.
    Invalidated,
}

/// AI issue-prioritisation facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrioritizationEvent {
    /// Prioritisation began.
    Started {
        /// The provider, as a wire id.
        provider: String,
    },
    /// A ranking arrived.
    Completed {
        /// The model's ranking text.
        ranking: String,
        /// Whether it came from the response cache.
        cached: bool,
    },
    /// Prioritisation failed.
    Failed {
        /// The diagnostic.
        message: String,
        /// Whether retrying could succeed.
        retryable: bool,
    },
    /// Prioritisation was cancelled.
    Cancelled,
    /// New issues replaced the projection the ranking described.
    Invalidated,
}

/// AI fix-plan facts. Every plan is validated against the remediation catalog
/// before it reaches a host: a plan can only ever name catalog ids.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FixPlanEvent {
    /// Generation began.
    Started {
        /// The provider, as a wire id.
        provider: String,
    },
    /// A validated plan arrived.
    Completed {
        /// The plan, reduced to current catalog references.
        plan: Box<ValidatedFixPlan>,
    },
    /// Generation failed.
    Failed {
        /// The diagnostic.
        message: String,
        /// Whether retrying could succeed.
        retryable: bool,
    },
    /// Generation was cancelled.
    Cancelled,
    /// New evidence or new issues replaced what the plan described.
    Invalidated,
}

/// Remediation-execution facts.
///
/// The Repair gate lives inside the broker, not here: approving a proposal
/// that contains a `Repair`-tier action with mere review produces
/// [`ActionEvent::RepairConfirmationRequired`] and runs nothing.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ActionEvent {
    /// An immutable preview was staged for review.
    Proposal {
        /// The preview.
        proposal: Box<ActionProposal>,
    },
    /// The preview contains a Repair-tier action and was **not** consumed.
    /// Show it again and approve with `confirm_repair`.
    RepairConfirmationRequired {
        /// The still-reviewable preview.
        proposal: Box<ActionProposal>,
    },
    /// A grant was minted and a run started.
    Approved {
        /// The run.
        run_id: String,
        /// The initial run projection.
        summary: Box<ActionRunSummary>,
    },
    /// A live run transition.
    Run {
        /// The current run projection.
        summary: Box<ActionRunSummary>,
    },
    /// A run reached a terminal state.
    Summary {
        /// The terminal run projection.
        summary: Box<ActionRunSummary>,
    },
    /// A prepare or approve was refused.
    Rejected {
        /// The user-facing reason.
        message: String,
    },
    /// An unused preview was dismissed.
    Discarded {
        /// The preview that is gone.
        proposal_id: String,
    },
}

/// Live model-catalog facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelCatalogEvent {
    /// A refresh began.
    Started {
        /// The provider, as a wire id.
        provider: String,
    },
    /// A refresh was skipped because one ran recently.
    Throttled {
        /// The provider, as a wire id.
        provider: String,
        /// The catalog still on record, when there is one.
        last: Option<Box<ModelCatalog>>,
    },
    /// A fresh catalog arrived.
    Loaded {
        /// The provider, as a wire id.
        provider: String,
        /// The catalog.
        catalog: Box<ModelCatalog>,
    },
    /// A refresh failed. `last` is the catalog a host should keep showing.
    Failed {
        /// The provider, as a wire id.
        provider: String,
        /// The diagnostic.
        error: String,
        /// The previous catalog, when one is on record.
        last: Option<Box<ModelCatalog>>,
    },
    /// A refresh was cancelled.
    Cancelled {
        /// The provider, as a wire id.
        provider: String,
    },
}

/// Subscription-CLI account and installation facts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionEvent {
    /// An account operation began.
    Started {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which operation.
        operation: SubscriptionAuthOperation,
    },
    /// An account status arrived.
    Status {
        /// The status.
        status: Box<SubscriptionAuthStatus>,
    },
    /// A sign-in or sign-out completed.
    Completed {
        /// Which operation.
        operation: SubscriptionAuthOperation,
        /// The status afterwards.
        status: Box<SubscriptionAuthStatus>,
    },
    /// An account operation failed.
    Failed {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which operation.
        operation: SubscriptionAuthOperation,
        /// The sanitized diagnostic.
        error: String,
    },
    /// An account operation was cancelled.
    Cancelled {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which operation.
        operation: SubscriptionAuthOperation,
    },
    /// An installation began.
    InstallStarted {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which method.
        method: SubscriptionInstallMethod,
    },
    /// An installation reached a new stage.
    InstallProgress {
        /// The stage.
        progress: Box<SubscriptionInstallProgress>,
    },
    /// The vendor bootstrap needs its own second confirmation.
    InstallFallbackRequired {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Why the winget path is unusable.
        reason: SubscriptionInstallFallbackReason,
    },
    /// An installation finished and was verified.
    Installed {
        /// The verified post-install state.
        status: Box<SubscriptionInstallStatus>,
    },
    /// An installation failed.
    InstallFailed {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which method.
        method: SubscriptionInstallMethod,
        /// The sanitized diagnostic.
        error: String,
    },
    /// An installation was cancelled; its process tree was killed.
    InstallCancelled {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Which method.
        method: SubscriptionInstallMethod,
    },
}

/// Everything the engine reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AppEvent {
    /// The service finished starting. The snapshot already carries the
    /// persisted settings, so a first frame can use the right theme.
    Started {
        /// The initial read model.
        snapshot: Box<AppSnapshot>,
    },
    /// A diagnostic-scan fact.
    Scan(ScanEvent),
    /// An issue-detection fact.
    Issues(IssuesEvent),
    /// A scan-history fact.
    History(HistoryEvent),
    /// A live-monitoring fact.
    Monitor(MonitorEvent),
    /// An AI-provider fact.
    Provider(ProviderEvent),
    /// A settings fact.
    Settings(SettingsEvent),
    /// An export-rendering fact.
    Export(ExportEvent),
    /// An update-channel fact.
    Update(UpdateEvent),
    /// A host-identity fact.
    System(SystemEvent),
    /// An agentic-chat fact.
    Chat(ChatEvent),
    /// An AI scan-report fact.
    Report(ReportEvent),
    /// A per-task AI analysis fact.
    Analysis(AnalysisEvent),
    /// An AI fix-plan fact.
    FixPlan(FixPlanEvent),
    /// An AI issue-prioritisation fact.
    Prioritization(PrioritizationEvent),
    /// A remediation-execution fact.
    Action(ActionEvent),
    /// A worker stopped. `panicked` is true when it stopped without being
    /// asked to, which is the only signal the worker crates expose.
    WorkerStopped {
        /// Which worker.
        worker: WorkerKind,
        /// Whether it stopped on its own.
        panicked: bool,
    },
    /// A worker reply did not arrive inside its deadline. The domain's
    /// pending state is cleared; nothing hangs.
    ReplyTimedOut {
        /// Which worker owed the reply.
        worker: WorkerKind,
        /// The request that timed out.
        request: RequestId,
    },
    /// The event stream is finished. No further events will be produced.
    Terminated,
}

#[derive(Debug)]
struct EventQueueState {
    queue: VecDeque<AppEvent>,
    dropped: u64,
    terminated: bool,
}

/// The shared event storage behind [`AppEventReceiver`].
#[derive(Debug)]
pub(crate) struct EventQueue {
    capacity: usize,
    state: Mutex<EventQueueState>,
    wake: Mutex<Option<UiWakeHandler>>,
}

impl EventQueue {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            state: Mutex::new(EventQueueState {
                queue: VecDeque::new(),
                dropped: 0,
                terminated: false,
            }),
            wake: Mutex::new(None),
        })
    }

    fn state(&self) -> MutexGuard<'_, EventQueueState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queue one event. The oldest event is dropped when the queue is full;
    /// the drop count is reported by [`AppEventReceiver::dropped`].
    pub(crate) fn push(&self, event: AppEvent) {
        let mut state = self.state();
        if state.queue.len() >= self.capacity {
            state.queue.pop_front();
            state.dropped = state.dropped.saturating_add(1);
        }
        if matches!(event, AppEvent::Terminated) {
            state.terminated = true;
        }
        state.queue.push_back(event);
    }

    pub(crate) fn take(&self) -> Vec<AppEvent> {
        let mut state = self.state();
        state.queue.drain(..).collect()
    }

    pub(crate) fn wake(&self) {
        let handler = self
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(handler) = handler {
            handler.call();
        }
    }
}

/// The host's handle on the event stream.
///
/// The service pumps workers inside [`crate::AppService::drain`], which takes
/// the queued events and returns them. This handle exists so the host can
/// register its wake callback and observe termination from another thread; its
/// own [`Self::drain`] takes whatever is queued *without* pumping, which is
/// what a host does when it was woken but does not own the service.
#[derive(Clone)]
pub struct AppEventReceiver {
    queue: Arc<EventQueue>,
}

impl fmt::Debug for AppEventReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppEventReceiver")
            .field("pending", &self.pending_len())
            .field("terminated", &self.is_terminated())
            .finish_non_exhaustive()
    }
}

impl AppEventReceiver {
    pub(crate) fn new(queue: Arc<EventQueue>) -> Self {
        Self { queue }
    }

    /// Install the callback workers use to wake the host.
    ///
    /// This is `wfdiag_ui_core::UiWakeHandler`, the same type the diagnostic
    /// and monitor event buses take, so a host writes one callback.
    ///
    /// Registering immediately wakes once if events are already queued, so a
    /// host that installs its handler late cannot miss a batch.
    pub fn set_wake_handler(&self, handler: UiWakeHandler) {
        *self
            .queue
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handler);
        if self.pending_len() != 0 || self.is_terminated() {
            self.queue.wake();
        }
    }

    /// Remove the wake callback without stopping the stream.
    pub fn clear_wake_handler(&self) {
        self.queue
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }

    /// Take every queued event without pumping the workers.
    #[must_use]
    pub fn drain(&self) -> Vec<AppEvent> {
        self.queue.take()
    }

    /// How many events are queued.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.queue.state().queue.len()
    }

    /// How many events the bounded queue had to drop.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.queue.state().dropped
    }

    /// Whether [`AppEvent::Terminated`] has been produced.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.queue.state().terminated
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEvent, AppEventReceiver, ChatEvent, EventQueue, UiWakeHandler};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_queue_is_bounded_and_reports_what_it_dropped() {
        let queue = EventQueue::new(2);
        let receiver = AppEventReceiver::new(Arc::clone(&queue));
        for _ in 0..4 {
            queue.push(AppEvent::Chat(ChatEvent::Cancelled));
        }
        assert_eq!(receiver.pending_len(), 2);
        assert_eq!(receiver.dropped(), 2);
        assert_eq!(receiver.drain().len(), 2);
        assert_eq!(receiver.pending_len(), 0);
    }

    #[test]
    fn installing_a_wake_handler_late_still_wakes_for_queued_events() {
        let queue = EventQueue::new(8);
        let receiver = AppEventReceiver::new(Arc::clone(&queue));
        queue.push(AppEvent::Chat(ChatEvent::Cancelled));
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        receiver.set_wake_handler(UiWakeHandler::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn termination_is_observable_from_the_receiver() {
        let queue = EventQueue::new(8);
        let receiver = AppEventReceiver::new(Arc::clone(&queue));
        assert!(!receiver.is_terminated());
        queue.push(AppEvent::Terminated);
        assert!(receiver.is_terminated());
    }
}
