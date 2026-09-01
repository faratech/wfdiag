//! Off-UI chat runtime: one worker thread owns the conversation and drives the
//! shared turn engine.
//!
//! A host enqueues commands and drains typed [`ChatWorkerEvent`]s, so no Tokio
//! executor, provider probe, or streaming work ever runs on its UI thread. Two
//! ports keep this runtime framework- and platform-neutral: [`ProviderResolver`]
//! turns a provider choice into a concrete transport, and [`ChatTurnTools`]
//! turns the host's immutable per-turn evidence into the bounded read-only tool
//! surface. Everything else — turn dispatch, cancellation, conversation
//! context, the exactly-one-terminal-event guarantee, and the Auto
//! clean-failure retry hook — lives here.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::{AIProvider, compat_caps};

use crate::tools::summarize_tool_arguments;
use crate::workers::{ActiveRequestSlot, WorkerWake, reap_worker, send_worker_event};
use crate::{
    BoundedToolBackend, BoundedToolCatalog, BoundedToolExecutor, BoundedToolOperation, ChatEmitter,
    ChatMessage, ChatProvider, ChatRole, DeltaPayload, DonePayload, ErrorPayload, ProposalPayload,
    ProviderUse, ScanCoverage, ScanRequestPayload, ToolCall, ToolExecutor, ToolFuture, ToolPayload,
    TurnStatus, build_system_prompt, completed_scan_request_reason, plan_context, run_chat_turn,
    truncate_output,
};

/// Stable engine session id. A native shell keeps one conversation.
pub const CHAT_SESSION_ID: &str = "reactor-chat";

const TOOL_ACTIVITY_PREVIEW_CHARS: usize = 300;

/// Boxed backends are the natural shape for a per-turn host port, so they are
/// usable as the [`BoundedToolExecutor`]'s backend directly.
impl<B: BoundedToolBackend + ?Sized> BoundedToolBackend for Box<B> {
    fn execute(
        &self,
        operation: BoundedToolOperation,
        cancel: CancellationToken,
    ) -> ToolFuture<'_> {
        (**self).execute(operation, cancel)
    }
}

/// Concrete provider call resolved by the host from its secure settings.
/// Secrets remain inside `chat`; only the non-secret `config_fingerprint`
/// participates in cache identity.
pub struct ResolvedChatProvider {
    pub chat: Arc<dyn ChatProvider>,
    pub config_fingerprint: String,
    pub requested_model: Option<String>,
}

impl std::fmt::Debug for ResolvedChatProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedChatProvider")
            .field("config_fingerprint", &self.config_fingerprint)
            .field("requested_model", &self.requested_model)
            .finish_non_exhaustive()
    }
}

pub type ChatResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ResolvedChatProvider, String>> + Send + 'a>>;

/// Provider configuration boundary. The host owns credential reads, endpoint
/// discovery, and on-device availability; this runtime only asks for the
/// provider the UI already chose.
pub trait ProviderResolver: Send + 'static {
    /// Resolve one turn's transport. `cancel` lets a slow probe abandon early
    /// and lets an on-device provider observe cancellation while generating.
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_>;
}

/// Per-turn tool boundary. The host's evidence snapshot stays opaque here: it
/// is captured on the UI thread, moved to the worker, and only ever read back
/// through this port.
pub trait ChatTurnTools: Send + 'static {
    /// Immutable evidence for one turn, assembled by the host.
    type Evidence: Send + 'static;

    /// Bounded catalog of the tools this turn may expose to the model.
    fn catalog(&self, evidence: &Self::Evidence) -> BoundedToolCatalog;

    /// Read-only backend executing the catalog's validated operations.
    fn backend(
        &self,
        evidence: &Self::Evidence,
        max_result_chars: usize,
    ) -> Box<dyn BoundedToolBackend>;

    /// Bounded evidence text for providers that cannot call tools.
    fn prompt_evidence(&self, evidence: &Self::Evidence) -> String;

    /// Whether the user has enabled live network grounding.
    fn network_grounding_enabled(&self, evidence: &Self::Evidence) -> bool;

    /// How much evidence the current scan covers, for the Full Scan policy.
    fn scan_coverage(&self, evidence: &Self::Evidence) -> ScanCoverage;

    /// Scan session id attributed to a Full Scan request.
    fn scan_session_id(&self, evidence: &Self::Evidence) -> String;
}

/// Worker commands. Send resolves the concrete provider on the worker.
enum ChatCommand<E> {
    Send {
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        evidence: Box<E>,
        /// Created on the UI side, so cancellation does not wait behind the
        /// currently-running worker command.
        cancel: CancellationToken,
    },
    /// Clear the backend-owned conversation after any active turn has
    /// observed cancellation. This command is ordered behind the turn.
    Reset,
}

/// UI-facing lifecycle state for one model-requested tool call. The shared
/// engine calls its successful terminal state `completed`; this runtime
/// normalizes that wire spelling to the same `done` state used by the shipping
/// React UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatToolActivityState {
    Queued,
    Running,
    CancelRequested,
    Done,
    Cancelled,
    TimedOut,
    Failed,
}

impl ChatToolActivityState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Failed => "failed",
        }
    }

    /// Original shared-engine spelling, retained for status-line text.
    #[must_use]
    pub const fn engine_status(self) -> &'static str {
        match self {
            Self::Done => "completed",
            state => state.as_str(),
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Cancelled | Self::TimedOut | Self::Failed
        )
    }

    const fn from_engine_status(status: &str) -> Option<Self> {
        match status.as_bytes() {
            b"queued" => Some(Self::Queued),
            b"started" | b"running" => Some(Self::Running),
            b"cancel_requested" => Some(Self::CancelRequested),
            b"completed" | b"done" => Some(Self::Done),
            b"cancelled" => Some(Self::Cancelled),
            b"timed_out" => Some(Self::TimedOut),
            b"failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Lossless projection of a shared-engine tool activity update.
///
/// `result_preview` is the live bounded preview supplied by the engine.
/// `model_output` / `model_error` are reconciled to the exact bounded text
/// inserted into provider history before the terminal worker event is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatToolActivity {
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    pub state: ChatToolActivityState,
    pub duration_ms: Option<u64>,
    pub result_preview: Option<String>,
    pub model_output: Option<String>,
    pub model_error: Option<String>,
}

impl ChatToolActivity {
    #[must_use]
    pub fn from_payload(payload: &ToolPayload) -> Self {
        let state = ChatToolActivityState::from_engine_status(&payload.status)
            .unwrap_or(ChatToolActivityState::Failed);
        let mut activity = Self {
            call_id: payload.call_id.clone(),
            tool: payload.tool.clone(),
            args_summary: payload.args_summary.clone(),
            state,
            duration_ms: payload.duration_ms,
            result_preview: payload.result_preview.clone(),
            model_output: None,
            model_error: None,
        };
        match state {
            ChatToolActivityState::Done => {
                activity.model_output.clone_from(&payload.result_preview);
            }
            ChatToolActivityState::CancelRequested
            | ChatToolActivityState::Cancelled
            | ChatToolActivityState::TimedOut
            | ChatToolActivityState::Failed => {
                activity.model_error.clone_from(&payload.result_preview);
            }
            ChatToolActivityState::Queued | ChatToolActivityState::Running => {}
        }
        if ChatToolActivityState::from_engine_status(&payload.status).is_none() {
            activity.model_error.get_or_insert_with(|| {
                format!("Unsupported tool activity status '{}'", payload.status)
            });
        }
        activity
    }

    /// Compact status text used by the current shell's status line.
    #[must_use]
    pub fn compatibility_summary(&self) -> String {
        format!("{} · {}", self.tool, self.state.engine_status())
    }
}

/// Ordered, call-id keyed activity history for one assistant turn. Repeated
/// queued/running/terminal updates replace their original slot rather than
/// producing duplicate chips or history rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatToolHistory {
    activities: Vec<ChatToolActivity>,
}

impl ChatToolHistory {
    #[must_use]
    pub fn activities(&self) -> &[ChatToolActivity] {
        &self.activities
    }

    /// Consuming accessor for callers that transfer the projection into their
    /// own view model.
    #[must_use]
    pub fn into_activities(self) -> Vec<ChatToolActivity> {
        self.activities
    }

    pub fn upsert(&mut self, activity: ChatToolActivity) {
        if let Some(existing) = self
            .activities
            .iter_mut()
            .find(|existing| existing.call_id == activity.call_id)
        {
            *existing = activity;
        } else {
            self.activities.push(activity);
        }
    }

    /// Adapter for call sites that still render `Vec<String>`.
    #[must_use]
    pub fn compatibility_summaries(&self) -> Vec<String> {
        self.activities
            .iter()
            .map(ChatToolActivity::compatibility_summary)
            .collect()
    }

    fn reconcile_model_messages(&mut self, messages: &[ChatMessage]) {
        for message in messages
            .iter()
            .filter(|message| message.role == ChatRole::Tool)
        {
            let Some(call_id) = message.tool_call_id.as_deref() else {
                continue;
            };
            let call = messages
                .iter()
                .filter(|message| message.role == ChatRole::Assistant)
                .flat_map(|message| message.tool_calls.iter())
                .find(|call| call.id == call_id);
            let position = self
                .activities
                .iter()
                .position(|activity| activity.call_id == call_id);
            let index = position.unwrap_or_else(|| {
                let tool = message
                    .tool_name
                    .clone()
                    .or_else(|| call.map(|call| call.name.clone()))
                    .unwrap_or_else(|| "unknown_tool".to_string());
                let args_summary = call.map_or_else(String::new, |call| {
                    summarize_tool_arguments(&call.arguments)
                });
                self.activities.push(ChatToolActivity {
                    call_id: call_id.to_string(),
                    tool,
                    args_summary,
                    state: if message.tool_result_is_error {
                        ChatToolActivityState::Failed
                    } else {
                        ChatToolActivityState::Done
                    },
                    duration_ms: None,
                    result_preview: Some(truncate_output(
                        &message.content,
                        TOOL_ACTIVITY_PREVIEW_CHARS,
                    )),
                    model_output: None,
                    model_error: None,
                });
                self.activities.len() - 1
            });
            let activity = &mut self.activities[index];
            if message.tool_result_is_error {
                activity.state = ChatToolActivityState::Failed;
            } else if !activity.state.is_terminal() {
                activity.state = ChatToolActivityState::Done;
            }
            if activity.state == ChatToolActivityState::Done && !message.tool_result_is_error {
                activity.model_output = Some(message.content.clone());
                activity.model_error = None;
            } else {
                activity.model_error = Some(message.content.clone());
                activity.model_output = None;
            }
        }
    }
}

/// Typed worker events drained by the host component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatWorkerEvent {
    Delta {
        request_id: u64,
        text: String,
    },
    ToolActivity {
        request_id: u64,
        activity: ChatToolActivity,
        history: ChatToolHistory,
        /// Compatibility projection for the current status-line consumer.
        summary: String,
    },
    Proposal {
        request_id: u64,
        remediation_id: String,
        issue_id: Option<String>,
    },
    FullScanRequested {
        request_id: u64,
        source_scan_id: String,
        reason: String,
    },
    Done {
        request_id: u64,
        provider: String,
        provider_use: ProviderUse,
        finish_reason: String,
        tool_history: ChatToolHistory,
    },
    /// A clean first-request failure with no emitted text or tool work. The
    /// host may retry this logical turn against its next snapshot-based
    /// provider candidate without duplicating conversation history.
    RetryableFailure {
        request_id: u64,
        message: String,
        provider_use: ProviderUse,
        tool_history: ChatToolHistory,
    },
    Failed {
        request_id: u64,
        message: String,
        finish_reason: String,
        tool_history: ChatToolHistory,
    },
    Cancelled {
        request_id: u64,
        finish_reason: String,
        tool_history: ChatToolHistory,
    },
}

impl ChatWorkerEvent {
    /// The originating send's identity, used for stale-event rejection.
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Delta { request_id, .. }
            | Self::ToolActivity { request_id, .. }
            | Self::Proposal { request_id, .. }
            | Self::FullScanRequested { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::RetryableFailure { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id, .. } => *request_id,
        }
    }

    /// The activity update carried by a live tool event.
    #[must_use]
    pub const fn tool_activity(&self) -> Option<&ChatToolActivity> {
        match self {
            Self::ToolActivity { activity, .. } => Some(activity),
            _ => None,
        }
    }

    /// The latest ordered history snapshot. Live tool events carry the state
    /// as of that update; terminal events carry the model-history-reconciled
    /// final snapshot.
    #[must_use]
    pub const fn tool_history(&self) -> Option<&ChatToolHistory> {
        match self {
            Self::ToolActivity { history, .. } => Some(history),
            Self::Done { tool_history, .. }
            | Self::RetryableFailure { tool_history, .. }
            | Self::Failed { tool_history, .. }
            | Self::Cancelled { tool_history, .. } => Some(tool_history),
            _ => None,
        }
    }
}

#[derive(Default)]
struct ChatEmissionState {
    terminal_flushed: bool,
    terminal: Option<ChatWorkerEvent>,
    pending_error: Option<String>,
    tool_history: ChatToolHistory,
}

struct WorkerEmitter {
    request_id: u64,
    events: std_mpsc::Sender<ChatWorkerEvent>,
    wake: WorkerWake,
    state: Mutex<ChatEmissionState>,
}

impl WorkerEmitter {
    fn new(request_id: u64, events: std_mpsc::Sender<ChatWorkerEvent>, wake: WorkerWake) -> Self {
        Self {
            request_id,
            events,
            wake,
            state: Mutex::new(ChatEmissionState::default()),
        }
    }

    fn publish(&self, event: ChatWorkerEvent) {
        send_worker_event(&self.events, &self.wake, event);
    }

    fn fail_once(&self, message: String, finish_reason: &str) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.terminal_flushed || state.terminal.is_some() {
            return;
        }
        state.terminal = Some(ChatWorkerEvent::Failed {
            request_id: self.request_id,
            message,
            finish_reason: finish_reason.to_string(),
            tool_history: ChatToolHistory::default(),
        });
    }

    fn retryable_failure_once(&self, message: String, provider_use: ProviderUse) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.terminal_flushed || state.terminal.is_some() {
            return;
        }
        state.terminal = Some(ChatWorkerEvent::RetryableFailure {
            request_id: self.request_id,
            message,
            provider_use,
            tool_history: ChatToolHistory::default(),
        });
    }

    fn cancel_once(&self, provider: AIProvider) {
        self.done(&DonePayload {
            session_id: CHAT_SESSION_ID.to_string(),
            message_id: format!("chat_{}", self.request_id),
            finish_reason: "cancelled".to_string(),
            provider: provider.to_string(),
            provider_use: ProviderUse::for_provider(provider, None),
            tool_call_count: 0,
        });
    }

    fn flush_terminal(&self) {
        let terminal = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.terminal_flushed {
                return;
            }
            state.terminal_flushed = true;
            let history = state.tool_history.clone();
            state.terminal.take().map(|mut terminal| {
                match &mut terminal {
                    ChatWorkerEvent::Done { tool_history, .. }
                    | ChatWorkerEvent::RetryableFailure { tool_history, .. }
                    | ChatWorkerEvent::Failed { tool_history, .. }
                    | ChatWorkerEvent::Cancelled { tool_history, .. } => {
                        *tool_history = history;
                    }
                    ChatWorkerEvent::Delta { .. }
                    | ChatWorkerEvent::ToolActivity { .. }
                    | ChatWorkerEvent::Proposal { .. }
                    | ChatWorkerEvent::FullScanRequested { .. } => {}
                }
                terminal
            })
        };
        if let Some(terminal) = terminal {
            self.publish(terminal);
        }
    }

    fn reconcile_model_messages(&self, messages: &[ChatMessage]) {
        if let Ok(mut state) = self.state.lock() {
            state.tool_history.reconcile_model_messages(messages);
        }
    }
}

impl ChatEmitter for WorkerEmitter {
    fn delta(&self, payload: &DeltaPayload) {
        self.publish(ChatWorkerEvent::Delta {
            request_id: self.request_id,
            text: payload.text.clone(),
        });
    }

    fn tool(&self, payload: &ToolPayload) {
        let activity = ChatToolActivity::from_payload(payload);
        let summary = activity.compatibility_summary();
        let history = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.tool_history.upsert(activity.clone());
            state.tool_history.clone()
        };
        self.publish(ChatWorkerEvent::ToolActivity {
            request_id: self.request_id,
            activity,
            history,
            summary,
        });
    }

    fn done(&self, payload: &DonePayload) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.terminal_flushed || state.terminal.is_some() {
            return;
        }
        let pending_error = state.pending_error.take();
        let event = match payload.finish_reason.as_str() {
            "cancelled" => ChatWorkerEvent::Cancelled {
                request_id: self.request_id,
                finish_reason: payload.finish_reason.clone(),
                tool_history: ChatToolHistory::default(),
            },
            "error" | "refusal" => ChatWorkerEvent::Failed {
                request_id: self.request_id,
                message: pending_error
                    .unwrap_or_else(|| "The AI request did not complete".to_string()),
                finish_reason: payload.finish_reason.clone(),
                tool_history: ChatToolHistory::default(),
            },
            _ => ChatWorkerEvent::Done {
                request_id: self.request_id,
                provider: payload.provider.clone(),
                provider_use: payload.provider_use.clone(),
                finish_reason: payload.finish_reason.clone(),
                tool_history: ChatToolHistory::default(),
            },
        };
        state.terminal = Some(event);
    }

    fn error(&self, payload: &ErrorPayload) {
        if let Ok(mut state) = self.state.lock()
            && !state.terminal_flushed
            && state.terminal.is_none()
        {
            state.pending_error = Some(payload.message.clone());
        }
    }

    fn proposal(&self, payload: &ProposalPayload) {
        let Some(remediation_id) = payload
            .proposal
            .get("remediationId")
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        let issue_id = payload
            .proposal
            .get("issueId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        self.publish(ChatWorkerEvent::Proposal {
            request_id: self.request_id,
            remediation_id: remediation_id.to_string(),
            issue_id,
        });
    }

    fn scan_request(&self, payload: &ScanRequestPayload) {
        self.publish(ChatWorkerEvent::FullScanRequested {
            request_id: self.request_id,
            source_scan_id: payload.source_scan_id.clone(),
            reason: payload.reason.clone(),
        });
    }
}

/// Non-tool providers receive bounded evidence in the system prompt and can
/// never issue a tool call.
struct NoTools;

impl ToolExecutor for NoTools {
    fn execute<'a>(&'a self, _call: &'a ToolCall, _cancel: CancellationToken) -> ToolFuture<'a> {
        Box::pin(async { Err("This provider does not support tools".to_string()) })
    }
}

struct WorkerState<R: ProviderResolver, T: ChatTurnTools> {
    resolver: R,
    tools: T,
    events: std_mpsc::Sender<ChatWorkerEvent>,
    wake: WorkerWake,
    messages: Vec<ChatMessage>,
}

impl<R: ProviderResolver, T: ChatTurnTools> WorkerState<R, T> {
    #[allow(clippy::too_many_arguments)] // One physical attempt's full identity.
    async fn run_turn(
        &mut self,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        evidence: T::Evidence,
        cancel: CancellationToken,
    ) {
        let emitter = WorkerEmitter::new(request_id, self.events.clone(), Arc::clone(&self.wake));
        if cancel.is_cancelled() {
            emitter.cancel_once(provider);
            emitter.flush_terminal();
            return;
        }
        let Some(resolved) = self
            .resolve_or_finish(provider, fallback_from, allow_fallback, &cancel, &emitter)
            .await
        else {
            return;
        };
        let caps = compat_caps(provider);
        let plan = plan_context(caps.context_budget_chars);
        let tools_enabled = caps.supports_tools;
        let scan_evidence = if tools_enabled {
            None
        } else {
            Some(self.tools.prompt_evidence(&evidence))
        };
        let system = build_system_prompt(
            tools_enabled,
            self.tools.network_grounding_enabled(&evidence),
            scan_evidence.as_deref(),
            &plan,
        );
        let message_id = format!("chat_{request_id}");
        let mut provider_use = ProviderUse::for_provider(provider, fallback_from)
            .with_requested_model(resolved.requested_model.as_deref());
        let catalog = self.tools.catalog(&evidence);
        let specs = if tools_enabled {
            catalog.specs()
        } else {
            Vec::new()
        };
        let bounded_executor = BoundedToolExecutor::new(
            catalog,
            self.tools.backend(&evidence, plan.tool_result_chars),
        );
        let no_tools = NoTools;
        let tool_executor: &dyn ToolExecutor = if tools_enabled {
            &bounded_executor
        } else {
            &no_tools
        };
        let turn_message_start = self.messages.len();
        self.messages.push(ChatMessage::user(prompt));
        let outcome = run_chat_turn(
            &mut provider_use,
            caps,
            resolved.chat.as_ref(),
            CHAT_SESSION_ID,
            &message_id,
            &mut self.messages,
            &system,
            &specs,
            tool_executor,
            &emitter,
            cancel.clone(),
            allow_fallback,
        )
        .await;
        match outcome {
            Ok(TurnStatus::Completed { .. }) => {
                if let Some(reason) = completed_scan_request_reason(
                    tools_enabled,
                    self.tools.scan_coverage(&evidence),
                    &self.messages,
                ) {
                    emitter.scan_request(&ScanRequestPayload {
                        session_id: CHAT_SESSION_ID.to_string(),
                        message_id,
                        source_scan_id: self.tools.scan_session_id(&evidence),
                        kind: "full".to_string(),
                        reason,
                        question: "Would you like me to run the Full Scan?".to_string(),
                    });
                }
            }
            Ok(TurnStatus::Cancelled) => {
                self.truncate_failed_turn();
            }
            Ok(TurnStatus::Error) => {
                self.truncate_failed_turn();
                emitter.fail_once("The AI request did not complete".to_string(), "error");
            }
            Err(message) => {
                self.truncate_failed_turn();
                emitter.retryable_failure_once(message, provider_use);
            }
        }
        emitter.reconcile_model_messages(&self.messages[turn_message_start..]);
        emitter.flush_terminal();
    }

    /// Resolve the turn's transport, or publish the one terminal event a
    /// failed resolution earns: cancellation wins over a clean failure, and a
    /// clean failure is retryable only while fallback is still allowed.
    async fn resolve_or_finish(
        &self,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        cancel: &CancellationToken,
        emitter: &WorkerEmitter,
    ) -> Option<ResolvedChatProvider> {
        match self.resolver.resolve(provider, cancel.clone()).await {
            Ok(resolved) => Some(resolved),
            Err(message) => {
                if cancel.is_cancelled() {
                    emitter.cancel_once(provider);
                } else if allow_fallback {
                    emitter.retryable_failure_once(
                        message,
                        ProviderUse::for_provider(provider, fallback_from),
                    );
                } else {
                    emitter.fail_once(message, "error");
                }
                emitter.flush_terminal();
                None
            }
        }
    }

    /// Drop the trailing user message when no assistant reply was recorded,
    /// so a retried question does not duplicate context.
    fn truncate_failed_turn(&mut self) {
        if self
            .messages
            .last()
            .is_some_and(|message| message.role == ChatRole::User)
        {
            self.messages.pop();
        }
    }
}

/// Handle the host holds on its UI thread.
pub struct NativeChatRuntime<T: ChatTurnTools> {
    /// Option so teardown can release the sender BEFORE joining the worker;
    /// joining while the sender is alive would deadlock (recv never
    /// disconnects on the shutting-down UI thread).
    commands: Option<std_mpsc::Sender<ChatCommand<T::Evidence>>>,
    active: ActiveRequestSlot,
    worker: Option<JoinHandle<()>>,
}

impl<T: ChatTurnTools> NativeChatRuntime<T> {
    /// Start the worker with the host's provider and tool ports. `wake` is
    /// invoked after each event is queued so a UI thread can drain without
    /// polling.
    ///
    /// # Errors
    /// When the worker thread cannot be spawned.
    pub fn start<R: ProviderResolver>(
        resolver: R,
        tools: T,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ChatWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ChatCommand<T::Evidence>>();
        let (events, event_rx) = std_mpsc::channel::<ChatWorkerEvent>();
        let active = ActiveRequestSlot::new();
        let worker_active = active.clone();
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-chat".to_string())
            .spawn(move || {
                let mut state = WorkerState {
                    resolver,
                    tools,
                    events,
                    wake,
                    messages: Vec::new(),
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ChatCommand::Send {
                            request_id,
                            prompt,
                            provider,
                            fallback_from,
                            allow_fallback,
                            evidence,
                            cancel,
                        } => {
                            // The Tokio runtime exists only while a turn runs; an
                            // idle runtime's IO/time drivers were observed to keep
                            // the WinUI dispatcher from finishing window teardown.
                            // Deliberately rebuilt per turn (2026-08-31 audit): a
                            // worker-owned persistent runtime would reintroduce
                            // that teardown hang, and the build cost is noise
                            // next to a multi-second network turn.
                            let runtime = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build();
                            if let Ok(runtime) = runtime {
                                runtime.block_on(state.run_turn(
                                    request_id,
                                    prompt,
                                    provider,
                                    fallback_from,
                                    allow_fallback,
                                    *evidence,
                                    cancel,
                                ));
                                runtime.shutdown_timeout(Duration::from_secs(1));
                            } else {
                                let emitter = WorkerEmitter::new(
                                    request_id,
                                    state.events.clone(),
                                    Arc::clone(&state.wake),
                                );
                                emitter.fail_once(
                                    "The native AI chat runtime could not start".to_string(),
                                    "error",
                                );
                                emitter.flush_terminal();
                            }
                            worker_active.clear(request_id);
                        }
                        ChatCommand::Reset => state.messages.clear(),
                    }
                }
            })?;
        Ok((
            Self {
                commands: Some(commands),
                active,
                worker: Some(worker),
            },
            event_rx,
        ))
    }

    /// Queue one physical attempt for a logical chat turn. `fallback_from`
    /// keeps final attribution pinned to the first provider, while
    /// `allow_fallback` permits only a clean first-request failure to return
    /// as [`ChatWorkerEvent::RetryableFailure`].
    #[must_use]
    pub fn send_attempt(
        &self,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        evidence: T::Evidence,
    ) -> bool {
        self.queue_attempt(
            None,
            request_id,
            prompt,
            provider,
            fallback_from,
            allow_fallback,
            evidence,
        )
    }

    /// Queue the next physical attempt immediately after a clean failure.
    /// The worker may still be clearing `previous_request_id` when the UI
    /// receives that terminal event, so replacement is an explicit atomic
    /// operation instead of a timing-dependent retry.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // The retried attempt's full identity.
    pub fn retry_attempt(
        &self,
        previous_request_id: u64,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        evidence: T::Evidence,
    ) -> bool {
        self.queue_attempt(
            Some(previous_request_id),
            request_id,
            prompt,
            provider,
            fallback_from,
            allow_fallback,
            evidence,
        )
    }

    #[allow(clippy::too_many_arguments)] // One physical attempt's full identity.
    fn queue_attempt(
        &self,
        replaces_request_id: Option<u64>,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        evidence: T::Evidence,
    ) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancel) = self.active.register(request_id, replaces_request_id) else {
            return false;
        };
        if commands
            .send(ChatCommand::Send {
                request_id,
                prompt,
                provider,
                fallback_from,
                allow_fallback,
                evidence: Box::new(evidence),
                cancel,
            })
            .is_ok()
        {
            true
        } else {
            self.active.clear(request_id);
            false
        }
    }

    /// Cancel the turn currently held by the slot.
    #[must_use]
    pub fn cancel(&self) -> bool {
        self.active.cancel_any()
    }

    /// Cancel the active turn and clear backend-owned history in queue order.
    #[must_use]
    pub fn new_session(&self) -> bool {
        let _ = self.cancel();
        self.commands
            .as_ref()
            .is_some_and(|commands| commands.send(ChatCommand::Reset).is_ok())
    }

    /// Cancel any active turn, stop the worker, and wait up to `budget`.
    ///
    /// Returns `false` when the worker was still running when the budget
    /// expired. Either way the handle has already been handed to a detached
    /// reaper, so the caller never blocks past `budget` behind a turn that
    /// ignores cancellation. A second call is a no-op that reports success.
    pub fn stop_and_join(&mut self, budget: Duration) -> bool {
        self.cancel_and_release();
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (done, finished) = std_mpsc::channel();
        reap_worker(worker, Some(done));
        finished.recv_timeout(budget).is_ok()
    }

    /// Take the active turn's token and clear the slot BEFORE cancelling:
    /// `cancel()` runs registered wakers synchronously, and a waker that
    /// touches this same slot mutex would deadlock against the guard. The
    /// command sender is released next so the worker's `recv()` disconnects.
    fn cancel_and_release(&mut self) {
        if let Some(cancel) = self.active.take() {
            cancel.cancel();
        }
        self.commands = None;
    }
}

impl<T: ChatTurnTools> Drop for NativeChatRuntime<T> {
    fn drop(&mut self) {
        self.cancel_and_release();
        if let Some(worker) = self.worker.take() {
            // A turn that ignores cancellation (a vendor CLI that will not
            // die) must not extend the host's graceful-close path.
            reap_worker(worker, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workers::no_wake;
    use serde_json::json;

    struct NoEvidenceTools;

    impl ChatTurnTools for NoEvidenceTools {
        type Evidence = ();

        fn catalog(&self, _evidence: &Self::Evidence) -> BoundedToolCatalog {
            crate::tools::tool_catalog([], [])
        }

        fn backend(
            &self,
            _evidence: &Self::Evidence,
            _max_result_chars: usize,
        ) -> Box<dyn BoundedToolBackend> {
            unreachable!("no test turn reaches tool execution")
        }

        fn prompt_evidence(&self, _evidence: &Self::Evidence) -> String {
            String::new()
        }

        fn network_grounding_enabled(&self, _evidence: &Self::Evidence) -> bool {
            false
        }

        fn scan_coverage(&self, _evidence: &Self::Evidence) -> ScanCoverage {
            ScanCoverage::None
        }

        fn scan_session_id(&self, _evidence: &Self::Evidence) -> String {
            String::new()
        }
    }

    fn done_payload(finish_reason: &str) -> DonePayload {
        DonePayload {
            session_id: CHAT_SESSION_ID.to_string(),
            message_id: "chat_7".to_string(),
            finish_reason: finish_reason.to_string(),
            provider: "openai".to_string(),
            provider_use: ProviderUse::for_provider(AIProvider::OpenAI, None),
            tool_call_count: 0,
        }
    }

    fn tool_payload(
        call_id: &str,
        status: &str,
        duration_ms: Option<u64>,
        result_preview: Option<&str>,
    ) -> ToolPayload {
        ToolPayload {
            session_id: CHAT_SESSION_ID.to_string(),
            message_id: "chat_7".to_string(),
            call_id: call_id.to_string(),
            tool: "get_scan_summary".to_string(),
            args_summary: "scope: current".to_string(),
            status: status.to_string(),
            duration_ms,
            result_preview: result_preview.map(str::to_string),
        }
    }

    #[test]
    fn typed_activity_normalizes_every_shipping_lifecycle_state() {
        let cases = [
            ("queued", ChatToolActivityState::Queued, false, false),
            ("running", ChatToolActivityState::Running, false, false),
            (
                "cancel_requested",
                ChatToolActivityState::CancelRequested,
                false,
                true,
            ),
            ("completed", ChatToolActivityState::Done, true, false),
            ("cancelled", ChatToolActivityState::Cancelled, false, true),
            ("timed_out", ChatToolActivityState::TimedOut, false, true),
            ("failed", ChatToolActivityState::Failed, false, true),
        ];
        for (wire_status, expected, has_output, has_error) in cases {
            let preview = (has_output || has_error).then_some("bounded model-visible result");
            let payload = tool_payload("call-identity", wire_status, Some(37), preview);
            let activity = ChatToolActivity::from_payload(&payload);
            assert_eq!(activity.call_id, "call-identity");
            assert_eq!(activity.tool, "get_scan_summary");
            assert_eq!(activity.args_summary, "scope: current");
            assert_eq!(activity.state, expected);
            assert_eq!(activity.duration_ms, Some(37));
            assert_eq!(activity.model_output.is_some(), has_output);
            assert_eq!(activity.model_error.is_some(), has_error);
        }

        let started =
            ChatToolActivity::from_payload(&tool_payload("legacy-started", "started", None, None));
        assert_eq!(started.state, ChatToolActivityState::Running);
    }

    #[test]
    fn activity_history_upserts_by_call_identity_without_reordering() {
        let mut history = ChatToolHistory::default();
        history.upsert(ChatToolActivity::from_payload(&tool_payload(
            "call-a", "queued", None, None,
        )));
        history.upsert(ChatToolActivity::from_payload(&tool_payload(
            "call-b", "queued", None, None,
        )));
        history.upsert(ChatToolActivity::from_payload(&tool_payload(
            "call-a",
            "completed",
            Some(18),
            Some("A result"),
        )));

        assert_eq!(history.activities().len(), 2);
        assert_eq!(history.activities()[0].call_id, "call-a");
        assert_eq!(history.activities()[0].state, ChatToolActivityState::Done);
        assert_eq!(history.activities()[0].duration_ms, Some(18));
        assert_eq!(history.activities()[1].call_id, "call-b");
        assert_eq!(
            history.compatibility_summaries(),
            [
                "get_scan_summary · completed".to_string(),
                "get_scan_summary · queued".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_history_reconciles_exact_model_output_and_error() {
        let mut history = ChatToolHistory::default();
        history.upsert(ChatToolActivity::from_payload(&tool_payload(
            "call-ok",
            "completed",
            Some(12),
            Some("short preview"),
        )));
        let messages = vec![
            ChatMessage::assistant_with_tools(
                "",
                vec![
                    ToolCall {
                        id: "call-ok".to_string(),
                        name: "get_scan_summary".to_string(),
                        arguments: json!({"scope":"current", "reason":"hidden"}),
                    },
                    ToolCall {
                        id: "call-rejected".to_string(),
                        name: "run_diagnostic".to_string(),
                        arguments: json!({"task_id":"os_info", "reason":"hidden"}),
                    },
                ],
            ),
            ChatMessage::tool_result(
                "call-ok",
                "get_scan_summary",
                "the exact bounded output inserted into model context",
            ),
            ChatMessage::tool_error(
                "call-rejected",
                "run_diagnostic",
                "the exact rejection inserted into model context",
            ),
        ];

        history.reconcile_model_messages(&messages);

        let completed = &history.activities()[0];
        assert_eq!(completed.result_preview.as_deref(), Some("short preview"));
        assert_eq!(
            completed.model_output.as_deref(),
            Some("the exact bounded output inserted into model context")
        );
        assert_eq!(completed.model_error, None);
        let rejected = &history.activities()[1];
        assert_eq!(rejected.call_id, "call-rejected");
        assert_eq!(rejected.tool, "run_diagnostic");
        assert_eq!(rejected.args_summary, "task_id: os_info");
        assert_eq!(rejected.state, ChatToolActivityState::Failed);
        assert_eq!(
            rejected.model_error.as_deref(),
            Some("the exact rejection inserted into model context")
        );
        assert_eq!(rejected.model_output, None);
    }

    #[test]
    fn worker_events_carry_live_and_terminal_structured_history() {
        let (events, receiver) = std_mpsc::channel();
        let emitter = WorkerEmitter::new(7, events, no_wake());
        emitter.tool(&tool_payload("call-1", "queued", None, None));
        emitter.tool(&tool_payload(
            "call-1",
            "completed",
            Some(9),
            Some("preview"),
        ));
        emitter.reconcile_model_messages(&[ChatMessage::tool_result(
            "call-1",
            "get_scan_summary",
            "exact model output",
        )]);
        emitter.done(&done_payload("stop"));
        emitter.flush_terminal();

        let queued = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            queued.tool_activity().map(|activity| activity.state),
            Some(ChatToolActivityState::Queued)
        );
        assert_eq!(queued.tool_history().unwrap().activities().len(), 1);

        let completed = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            completed.tool_activity().map(|activity| activity.state),
            Some(ChatToolActivityState::Done)
        );
        assert_eq!(
            completed.tool_history().unwrap().activities()[0].duration_ms,
            Some(9)
        );

        let terminal = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let final_activity = &terminal.tool_history().unwrap().activities()[0];
        assert_eq!(final_activity.state, ChatToolActivityState::Done);
        assert_eq!(
            final_activity.model_output.as_deref(),
            Some("exact model output")
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn error_plus_done_flushes_exactly_one_terminal_event() {
        let (events, receiver) = std_mpsc::channel();
        let emitter = WorkerEmitter::new(7, events, no_wake());
        emitter.error(&ErrorPayload {
            session_id: CHAT_SESSION_ID.to_string(),
            message_id: "chat_7".to_string(),
            message: "provider failed".to_string(),
        });
        emitter.done(&done_payload("error"));
        emitter.done(&done_payload("error"));
        emitter.flush_terminal();
        emitter.flush_terminal();

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ChatWorkerEvent::Failed {
                request_id: 7,
                message: "provider failed".to_string(),
                finish_reason: "error".to_string(),
                tool_history: ChatToolHistory::default(),
            }
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn clean_failure_flushes_one_retryable_event_with_fallback_attribution() {
        let (events, receiver) = std_mpsc::channel();
        let emitter = WorkerEmitter::new(8, events, no_wake());
        let provider_use =
            ProviderUse::for_provider(AIProvider::FoundryLocal, Some(AIProvider::PhiSilica));
        emitter.retryable_failure_once("provider unavailable".to_string(), provider_use.clone());
        emitter.fail_once("must not replace retry".to_string(), "error");
        emitter.flush_terminal();
        emitter.flush_terminal();

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ChatWorkerEvent::RetryableFailure {
                request_id: 8,
                message: "provider unavailable".to_string(),
                provider_use,
                tool_history: ChatToolHistory::default(),
            }
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn retry_atomically_replaces_the_just_completed_active_request() {
        let (commands, receiver) = std_mpsc::channel();
        let active = ActiveRequestSlot::new();
        let _previous = active
            .register(7, None)
            .expect("register the first attempt");
        let runtime = NativeChatRuntime::<NoEvidenceTools> {
            commands: Some(commands),
            active: active.clone(),
            worker: None,
        };
        assert!(runtime.retry_attempt(
            7,
            8,
            "retry me".to_string(),
            AIProvider::Ollama,
            Some(AIProvider::PhiSilica),
            false,
            (),
        ));
        assert_eq!(active.active_request(), Some(8));
        match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
            ChatCommand::Send {
                request_id,
                provider,
                fallback_from,
                ..
            } => {
                assert_eq!(request_id, 8);
                assert_eq!(provider, AIProvider::Ollama);
                assert_eq!(fallback_from, Some(AIProvider::PhiSilica));
            }
            ChatCommand::Reset => panic!("retry must queue a send"),
        }
    }

    #[test]
    fn cancellation_is_immediate_and_flushes_one_terminal_event() {
        let (events, receiver) = std_mpsc::channel();
        let emitter = WorkerEmitter::new(9, events, no_wake());
        emitter.cancel_once(AIProvider::PhiSilica);
        emitter.done(&done_payload("stop"));
        emitter.flush_terminal();
        emitter.flush_terminal();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ChatWorkerEvent::Cancelled {
                request_id: 9,
                finish_reason: "cancelled".to_string(),
                tool_history: ChatToolHistory::default(),
            }
        );
        assert!(receiver.try_recv().is_err());

        let (commands, _receiver) = std_mpsc::channel::<ChatCommand<()>>();
        let active = ActiveRequestSlot::new();
        let cancel = active.register(9, None).expect("register the active turn");
        let runtime = NativeChatRuntime::<NoEvidenceTools> {
            commands: Some(commands),
            active,
            worker: None,
        };
        assert!(runtime.cancel());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn stop_and_join_is_bounded_and_idempotent() {
        struct NoProviders;

        impl ProviderResolver for NoProviders {
            fn resolve(
                &self,
                _provider: AIProvider,
                _cancel: CancellationToken,
            ) -> ChatResolveFuture<'_> {
                Box::pin(async { Err("no provider".to_string()) })
            }
        }

        let (mut runtime, events) =
            NativeChatRuntime::start(NoProviders, NoEvidenceTools, no_wake()).unwrap();
        assert!(runtime.send_attempt(1, "hello".to_string(), AIProvider::OpenAI, None, false, (),));
        assert_eq!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            ChatWorkerEvent::Failed {
                request_id: 1,
                message: "no provider".to_string(),
                finish_reason: "error".to_string(),
                tool_history: ChatToolHistory::default(),
            }
        );

        assert!(runtime.stop_and_join(Duration::from_secs(2)));
        // The worker really exited: its command receiver is gone.
        assert!(!runtime.send_attempt(2, "again".to_string(), AIProvider::OpenAI, None, false, ()));
        // The handle has already been taken, so a second call is a no-op.
        assert!(runtime.stop_and_join(Duration::from_millis(1)));
    }
}
