//! Native AI chat runtime for the Reactor shell.
//!
//! A single std worker thread owns a current-thread Tokio runtime, the
//! conversation state, and the shared [`wfdiag_native_ai_chat`] turn engine.
//! The WinUI thread only enqueues commands and drains typed events, so no
//! Tokio, provider probe, or streaming work ever runs on the UI thread.
//!
//! Provider transport and the model/tool loop are shared with the shipping
//! backend. Reactor contributes only immutable diagnostic evidence and
//! framework-neutral read-only tool ports.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{
    BoundedToolBackend, BoundedToolCatalog, BoundedToolExecutor, BoundedToolOperation, ChatEmitter,
    ChatMessage, ChatProvider, ChatRole, CompatChatProvider, DeltaPayload,
    DiagnosticToolDescriptor, DonePayload, ErrorPayload, ProposalPayload, ProviderUse,
    RemediationToolDescriptor, ScanCoverage, ScanRequestPayload, ToolExecutor, ToolFuture,
    ToolPayload, TurnStatus, build_system_prompt, completed_scan_request_reason, plan_context,
    run_chat_turn, search_windows_knowledge, truncate_output,
};
use wfdiag_native_ai_provider::{
    AIProvider, CompatConfigPorts, FoundryEndpointSource, OllamaSource,
    ProcessSubscriptionCliStatusSource, ProviderKeySource, SubscriptionCliStatusSource,
    SubscriptionConfigPorts, compat_caps, resolve_compat_config, resolve_subscription_config,
};
use wfdiag_native_diagnostics::{
    DiagnosticExecutor, DiagnosticTask, NativeDiagnosticExecutor, ScanKind,
};
use wfdiag_native_history::{NativeHistoryRuntime, ScanRecord, ScanSummary};
use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, RemediationSummary};
use wfdiag_native_phi::PhiChatProvider;
use wfdiag_native_settings::{ProviderKeyId, SettingsService};
use wfdiag_ui_core::{DiagnosticTaskResult, SystemStats};

use serde_json::json;

use crate::ui_wake_support::NotifySenderExt;

/// Stable engine session id. The Reactor shell keeps one conversation.
pub const CHAT_SESSION_ID: &str = "reactor-chat";

/// Immutable evidence captured by the component for one user turn. The tool
/// worker may read this value but has no reference back to mutable UI state.
#[derive(Clone, Debug, Default)]
pub struct ChatToolSnapshot {
    pub system_overview: Option<String>,
    pub scan: Option<ChatScanSnapshot>,
    /// Live current-scan projection used only as the left side of a history
    /// comparison. The history worker selects the newest stored different id.
    pub current_history_scan: Option<ScanRecord>,
    pub issues: Vec<Issue>,
    pub history: Vec<ScanSummary>,
    pub live_stats: Option<SystemStats>,
    pub remediations: Vec<RemediationSummary>,
    pub network_grounding_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ChatScanSnapshot {
    pub session_id: String,
    pub started_at: SystemTime,
    pub scan_kind: ScanKind,
    pub selected_tasks: Vec<String>,
    pub results: Vec<DiagnosticTaskResult>,
    pub running: bool,
}

/// Read-only platform ports owned by the Reactor adapter.
#[derive(Clone)]
pub struct ChatToolPorts {
    diagnostics: Arc<dyn DiagnosticExecutor>,
    history: Option<Arc<NativeHistoryRuntime>>,
}

impl ChatToolPorts {
    #[must_use]
    pub fn new(
        diagnostics: Arc<dyn DiagnosticExecutor>,
        history: Option<Arc<NativeHistoryRuntime>>,
    ) -> Self {
        Self {
            diagnostics,
            history,
        }
    }

    #[must_use]
    pub fn shipping(history: Option<Arc<NativeHistoryRuntime>>) -> Self {
        Self::new(Arc::new(NativeDiagnosticExecutor), history)
    }
}

/// Credential and endpoint ports for one resolved turn. Rebuilt per turn so
/// settings edits and saved keys apply to the very next message. Shared with
/// the report worker, which resolves the same provider set.
pub(crate) struct ShellChatSource {
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    subscription: Arc<dyn SubscriptionCliStatusSource>,
}

impl ShellChatSource {
    pub(crate) fn new(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
    ) -> Self {
        Self {
            settings,
            foundry,
            ollama,
            subscription: Arc::new(ProcessSubscriptionCliStatusSource::new()),
        }
    }

    pub(crate) fn ports(&self) -> CompatConfigPorts {
        CompatConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            keys: Arc::new(SettingsKeySource(self.settings.clone())),
            foundry: Arc::clone(&self.foundry),
            ollama: Arc::clone(&self.ollama),
        }
    }

    pub(crate) fn subscription_ports(&self) -> SubscriptionConfigPorts {
        SubscriptionConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            status: Arc::clone(&self.subscription),
        }
    }
}

struct SettingsKeySource(SettingsService);

impl ProviderKeySource for SettingsKeySource {
    fn load(&self, key: ProviderKeyId) -> Option<String> {
        self.0.load_provider_key(key).ok().flatten()
    }
}

/// Worker commands. Send resolves the concrete provider on the worker.
pub enum ChatCommand {
    Send {
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        tools: Box<ChatToolSnapshot>,
        /// Created on the UI side, so cancellation does not wait behind the
        /// currently-running worker command.
        cancel: CancellationToken,
    },
    /// Clear the backend-owned conversation after any active turn has
    /// observed cancellation. This command is ordered behind the turn.
    Reset,
}

/// UI-facing lifecycle state for one model-requested tool call. The shared
/// engine calls its successful terminal state `completed`; Reactor normalizes
/// that wire spelling to the same `done` state used by the shipping React UI.
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

    /// Original shared-engine spelling, retained for the existing status-line
    /// text while `main.rs` moves from strings to the structured projection.
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

/// Lossless Reactor projection of a shared-engine tool activity update.
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

    /// Existing compact status text used by the current shell.
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

    // Consuming compatibility accessor for callers that transfer the
    // projection into their own view model. The current shell renders by
    // reference, but this remains part of the support module's public API.
    #[allow(dead_code)]
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

    /// Temporary adapter for call sites that still render `Vec<String>`.
    #[must_use]
    #[allow(dead_code)] // compatibility API; Reactor now renders structured activities
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

const TOOL_ACTIVITY_PREVIEW_CHARS: usize = 300;

fn summarize_tool_arguments(arguments: &serde_json::Value) -> String {
    let summary = arguments.as_object().map_or_else(
        || arguments.to_string(),
        |map| {
            map.iter()
                .filter_map(|(key, value)| {
                    if key == "reason" {
                        return None;
                    }
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    Some(format!("{key}: {value}"))
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    summary.chars().take(80).collect()
}

/// Typed worker events drained by the component.
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
        /// Compatibility projection for the current `main.rs` consumer.
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
    /// component may retry this logical turn against its next snapshot-based
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
    #[allow(dead_code)] // typed-event compatibility; main currently destructures variants
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
    #[allow(dead_code)] // typed-event compatibility; main currently destructures variants
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
    state: Mutex<ChatEmissionState>,
}

impl WorkerEmitter {
    fn new(request_id: u64, events: std_mpsc::Sender<ChatWorkerEvent>) -> Self {
        Self {
            request_id,
            events,
            state: Mutex::new(ChatEmissionState::default()),
        }
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
            let _ = self.events.send_and_wake(terminal);
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
        let _ = self.events.send_and_wake(ChatWorkerEvent::Delta {
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
        let _ = self.events.send_and_wake(ChatWorkerEvent::ToolActivity {
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
        let _ = self.events.send_and_wake(ChatWorkerEvent::Proposal {
            request_id: self.request_id,
            remediation_id: remediation_id.to_string(),
            issue_id,
        });
    }

    fn scan_request(&self, payload: &ScanRequestPayload) {
        let _ = self
            .events
            .send_and_wake(ChatWorkerEvent::FullScanRequested {
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
    fn execute<'a>(
        &'a self,
        _call: &'a wfdiag_native_ai_chat::ToolCall,
        _cancel: CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async { Err("This provider does not support tools".to_string()) })
    }
}

struct WorkerState {
    source: ShellChatSource,
    events: std_mpsc::Sender<ChatWorkerEvent>,
    messages: Vec<ChatMessage>,
    tools: ChatToolPorts,
}

struct ReactorToolBackend {
    snapshot: ChatToolSnapshot,
    ports: ChatToolPorts,
    max_result_chars: usize,
}

impl BoundedToolBackend for ReactorToolBackend {
    fn execute<'a>(
        &'a self,
        operation: BoundedToolOperation,
        cancel: CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            match operation {
                BoundedToolOperation::RunDiagnostic { task_id, .. } => {
                    let execution = self.ports.diagnostics.execute(task_id.clone());
                    let output = tokio::select! {
                        biased;
                        () = cancel.cancelled() => return Err("Diagnostic cancelled".to_string()),
                        output = execution => output,
                    };
                    if output.success {
                        Ok(bounded_readable_text(&output.output, self.max_result_chars))
                    } else {
                        Ok(truncate_output(
                            &format!(
                                "Task {task_id} COLLECTION ERROR: {}",
                                output.error.unwrap_or_else(|| "unknown error".to_string())
                            ),
                            self.max_result_chars,
                        ))
                    }
                }
                BoundedToolOperation::SearchWindowsKnowledge { query } => {
                    if !self.snapshot.network_grounding_enabled {
                        Err("Network grounding is disabled in Settings".to_string())
                    } else {
                        search_windows_knowledge(&query, self.max_result_chars, &cancel).await
                    }
                }
                BoundedToolOperation::GetScanSummary => Ok(truncate_output(
                    &scan_summary_text(&self.snapshot),
                    self.max_result_chars,
                )),
                BoundedToolOperation::RequestFullScan { reason } => {
                    request_full_scan_envelope(self.snapshot.scan.as_ref(), &reason)
                }
                BoundedToolOperation::GetDetectedIssues => Ok(truncate_output(
                    &detected_issues_text(&self.snapshot.issues),
                    self.max_result_chars,
                )),
                BoundedToolOperation::CompareWithPreviousScan => {
                    self.compare_with_previous_scan(&cancel).await
                }
                BoundedToolOperation::GetLiveStats => Ok(self.snapshot.live_stats.as_ref().map_or_else(
                    || "Monitoring is not running — live values unavailable. Suggest the user open the Monitor tab to start it.".to_string(),
                    |stats| bounded_readable_text(
                        &serde_json::to_string(stats).unwrap_or_default(),
                        self.max_result_chars,
                    ),
                )),
                BoundedToolOperation::ListRemediations => Ok(truncate_output(
                    &remediation_list_text(&self.snapshot.remediations),
                    self.max_result_chars,
                )),
                BoundedToolOperation::ListScanHistory => self.list_scan_history(&cancel).await,
                BoundedToolOperation::StageRemediation {
                    remediation_id,
                    issue_id,
                } => stage_remediation_envelope(
                    &self.snapshot,
                    &remediation_id,
                    issue_id.as_deref(),
                ),
            }
        })
    }
}

impl ReactorToolBackend {
    async fn compare_with_previous_scan(
        &self,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        let Some(current) = self.snapshot.current_history_scan.clone() else {
            return Ok(
                "Comparison is unavailable because the current scan has not been projected into history format."
                    .to_string(),
            );
        };
        let Some(history) = self.ports.history.as_ref() else {
            return Ok("Scan history storage is unavailable on this system.".to_string());
        };
        let comparison = history
            .request_compare_current_to_latest(std::sync::Arc::new(current))
            .map_err(|error| error.to_string())?;
        let comparison = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("History comparison cancelled".to_string()),
            result = comparison => result.map_err(|_| "Scan history worker stopped".to_string())??,
        };
        let Some(comparison) = comparison else {
            return Ok(
                "No different stored scan is available to compare with the current scan."
                    .to_string(),
            );
        };
        let task_ids = |changes: &[wfdiag_native_history::TaskChange]| {
            changes
                .iter()
                .map(|change| change.task_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        Ok(truncate_output(
            &format!(
                "Compared {} (current) with {} (previous): {} change(s).\nNew collection errors: {}\nNewly collected: {}",
                comparison.current_scan.timestamp,
                comparison.previous_scan.timestamp,
                comparison.total_changes,
                if comparison.new_failures.is_empty() {
                    "none".to_string()
                } else {
                    task_ids(&comparison.new_failures)
                },
                if comparison.new_successes.is_empty() {
                    "none".to_string()
                } else {
                    task_ids(&comparison.new_successes)
                },
            ),
            self.max_result_chars,
        ))
    }

    async fn list_scan_history(&self, cancel: &CancellationToken) -> Result<String, String> {
        let scans = if let Some(history) = self.ports.history.as_ref() {
            let reply = history.request_list().map_err(|error| error.to_string())?;
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err("Scan history request cancelled".to_string()),
                result = reply => result.map_err(|_| "Scan history worker stopped".to_string())??,
            }
        } else {
            self.snapshot.history.clone()
        };
        Ok(truncate_output(
            &scan_history_text(&scans),
            self.max_result_chars,
        ))
    }
}

fn tool_catalog(
    tasks: &[DiagnosticTask],
    remediations: &[RemediationSummary],
) -> BoundedToolCatalog {
    BoundedToolCatalog::new(
        tasks
            .iter()
            .map(|task| DiagnosticToolDescriptor {
                id: task.id.clone(),
                description: task.description.clone(),
            })
            .collect(),
        remediations
            .iter()
            .map(|remediation| RemediationToolDescriptor {
                id: remediation.id.clone(),
            })
            .collect(),
    )
}

fn bounded_readable_text(value: &str, max_chars: usize) -> String {
    let rendered = serde_json::from_str::<serde_json::Value>(value).map_or_else(
        |_| value.to_string(),
        |parsed| serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| value.to_string()),
    );
    truncate_output(&rendered, max_chars)
}

fn scan_coverage(scan: Option<&ChatScanSnapshot>) -> ScanCoverage {
    let Some(scan) = scan else {
        return ScanCoverage::None;
    };
    if scan.running || scan.results.len() < scan.selected_tasks.len() {
        return ScanCoverage::InProgress;
    }
    match scan.scan_kind {
        ScanKind::Quick => ScanCoverage::Quick,
        ScanKind::Full => ScanCoverage::Full,
        ScanKind::Targeted => ScanCoverage::Targeted,
    }
}

fn scan_summary_text(snapshot: &ChatToolSnapshot) -> String {
    let overview = snapshot
        .system_overview
        .as_deref()
        .map_or_else(String::new, |overview| {
            format!("SYSTEM OVERVIEW\n{overview}\n")
        });
    let Some(scan) = snapshot.scan.as_ref() else {
        return format!(
            "{overview}SCAN_SCOPE kind=none state=empty selected=0 completed=0. No scan data is available yet."
        );
    };
    let kind = match scan.scan_kind {
        ScanKind::Quick => "quick",
        ScanKind::Full => "full",
        ScanKind::Targeted => "targeted",
    };
    let state = if scan.running { "running" } else { "complete" };
    let collected = scan.results.iter().filter(|result| result.success).count();
    let failures = scan.results.len().saturating_sub(collected);
    let age_minutes = scan
        .started_at
        .elapsed()
        .map_or(0, |duration| duration.as_secs() / 60);
    let mut lines = vec![format!(
        "{overview}SCAN_SCOPE kind={kind} state={state} selected={} completed={}. Scan from {age_minutes} minute(s) ago: {collected} collected, {failures} collection failures.",
        scan.selected_tasks.len(),
        scan.results.len(),
    )];
    let mut results = scan.results.iter().collect::<Vec<_>>();
    results.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    lines.extend(results.into_iter().map(|result| {
        if result.success {
            format!("{}: COLLECTED — {}", result.task_id, result.output)
        } else {
            format!(
                "{}: COLLECTION ERROR ({})",
                result.task_id,
                result.error.as_deref().unwrap_or("unknown error")
            )
        }
    }));
    lines.join("\n")
}

fn detected_issues_text(issues: &[Issue]) -> String {
    if issues.is_empty() {
        return "Detected issues: none. No rule-based issue evidence is available for this scan."
            .to_string();
    }
    let severity = |severity| match severity {
        IssueSeverity::Critical => "CRITICAL",
        IssueSeverity::Warning => "WARNING",
        IssueSeverity::Info => "INFO",
        IssueSeverity::Ok => "OK",
    };
    let mut detected = Vec::new();
    let mut unknown = Vec::new();
    for issue in issues {
        match issue.status {
            IssueStatus::Detected => detected.push(format!(
                "Issue ID: {} | Remediation ID: {} | Severity: {} | {} — {} | Recommendation: {}",
                issue.id,
                issue
                    .remediation
                    .as_ref()
                    .map_or("none", |remediation| remediation.id.as_str()),
                severity(issue.severity),
                issue.title,
                issue.description,
                issue.recommendation,
            )),
            IssueStatus::Unknown | IssueStatus::Skipped => unknown.push(format!(
                "Issue ID: {} | Status: UNKNOWN | {} — {}",
                issue.id, issue.title, issue.description
            )),
            IssueStatus::Ok => {}
        }
    }
    let mut sections = vec![if detected.is_empty() {
        "Detected issues: none.".to_string()
    } else {
        format!(
            "{} issue(s) detected:\n{}",
            detected.len(),
            detected.join("\n")
        )
    }];
    if !unknown.is_empty() {
        sections.push(format!(
            "{} check(s) could not be verified:\n{}",
            unknown.len(),
            unknown.join("\n")
        ));
    }
    sections.join("\n\n")
}

fn scan_history_text(scans: &[ScanSummary]) -> String {
    if scans.is_empty() {
        return "No stored scans yet.".to_string();
    }
    scans
        .iter()
        .take(10)
        .map(|scan| {
            format!(
                "{} | {} | {} collected / {} collection errors{}",
                scan.id,
                scan.timestamp,
                scan.success_count,
                scan.failure_count,
                if scan.tags.is_empty() {
                    String::new()
                } else {
                    format!(" | tags: {}", scan.tags.join(", "))
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn remediation_list_text(remediations: &[RemediationSummary]) -> String {
    if remediations.is_empty() {
        return "No vetted remediations are available.".to_string();
    }
    remediations
        .iter()
        .map(|remediation| {
            format!(
                "{} | {} | {:?}{} | {}",
                remediation.id,
                remediation.label,
                remediation.tier,
                if remediation.admin_required {
                    " | admin"
                } else {
                    ""
                },
                remediation.description,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn request_full_scan_envelope(
    scan: Option<&ChatScanSnapshot>,
    reason: &str,
) -> Result<String, String> {
    match scan_coverage(scan) {
        ScanCoverage::Quick | ScanCoverage::Targeted => serde_json::to_string(&json!({
            "kind": "scan_request",
            "scanKind": "full",
            "reason": reason,
            "notice": "Confirmation requested only. The Full Scan has not started."
        }))
        .map_err(|error| format!("Could not serialize the Full Scan request: {error}")),
        ScanCoverage::None => Ok(
            "No scan evidence is available. Complete the automatic Quick Scan first.".to_string(),
        ),
        ScanCoverage::InProgress => {
            Ok("A scan is still in progress. Wait for it to finish.".to_string())
        }
        ScanCoverage::Full => {
            Ok("The current session already contains a Full Scan. Use its evidence.".to_string())
        }
    }
}

fn stage_remediation_envelope(
    snapshot: &ChatToolSnapshot,
    remediation_id: &str,
    issue_id: Option<&str>,
) -> Result<String, String> {
    let remediation = snapshot
        .remediations
        .iter()
        .find(|remediation| remediation.id == remediation_id)
        .ok_or_else(|| format!("Unknown remediation '{remediation_id}'"))?;
    if let Some(issue_id) = issue_id {
        let issue = snapshot
            .issues
            .iter()
            .find(|issue| issue.id == issue_id && issue.status == IssueStatus::Detected)
            .ok_or_else(|| format!("Detected issue '{issue_id}' is not present"))?;
        if issue.remediation.as_ref().map(|item| item.id.as_str()) != Some(remediation_id) {
            return Err(format!(
                "Remediation '{remediation_id}' is not mapped to issue '{issue_id}'"
            ));
        }
    } else if !remediation.maintenance {
        return Err(format!(
            "Remediation '{remediation_id}' requires its detected issue id"
        ));
    }
    serde_json::to_string(&json!({
        "kind": "staged_action_proposal",
        "proposal": {
            "remediationId": remediation_id,
            "issueId": issue_id,
            "label": remediation.label,
        },
        "notice": "Staged only. Awaiting the user's exact approval; nothing was executed."
    }))
    .map_err(|error| format!("Could not serialize staged proposal: {error}"))
}

struct ResolvedChat {
    chat: Box<dyn ChatProvider>,
    requested_model: Option<String>,
}

impl WorkerState {
    async fn resolve_chat(
        &self,
        provider: AIProvider,
        cancel: &CancellationToken,
    ) -> Result<ResolvedChat, String> {
        if provider == AIProvider::PhiSilica {
            return Ok(ResolvedChat {
                chat: Box::new(PhiChatProvider),
                requested_model: None,
            });
        }
        if provider == AIProvider::None {
            return Err("No AI provider is available".to_string());
        }
        let resolution = async {
            match provider {
                AIProvider::CodexCli | AIProvider::ClaudeCode => {
                    let ports = self.source.subscription_ports();
                    resolve_subscription_config(provider, &ports).await
                }
                _ => {
                    let ports = self.source.ports();
                    resolve_compat_config(provider, &ports).await
                }
            }
        };
        let cfg = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("AI request cancelled".to_string()),
            cfg = resolution => cfg?,
        };
        let requested_model = cfg.model.clone();
        Ok(ResolvedChat {
            chat: Box::new(CompatChatProvider { provider, cfg }),
            requested_model,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_turn(
        &mut self,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        tools: ChatToolSnapshot,
        cancel: CancellationToken,
    ) {
        let emitter = WorkerEmitter::new(request_id, self.events.clone());
        if cancel.is_cancelled() {
            emitter.cancel_once(provider);
            emitter.flush_terminal();
            return;
        }
        let resolved = match self.resolve_chat(provider, &cancel).await {
            Ok(resolved) => resolved,
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
                return;
            }
        };
        let caps = compat_caps(provider);
        let plan = plan_context(caps.context_budget_chars);
        let tools_enabled = caps.supports_tools;
        let evidence = if tools_enabled {
            None
        } else {
            Some(format!(
                "{}\n\n{}",
                scan_summary_text(&tools),
                detected_issues_text(&tools.issues)
            ))
        };
        let system = build_system_prompt(
            tools_enabled,
            tools.network_grounding_enabled,
            evidence.as_deref(),
            &plan,
        );
        let message_id = format!("chat_{request_id}");
        let mut provider_use = ProviderUse::for_provider(provider, fallback_from)
            .with_requested_model(resolved.requested_model.as_deref());
        let turn_message_start = self.messages.len();
        self.messages.push(ChatMessage::user(prompt));
        let catalog = tool_catalog(
            &self.tools.diagnostics.available_tasks(),
            &tools.remediations,
        );
        let specs = if tools_enabled {
            catalog.specs()
        } else {
            Vec::new()
        };
        let bounded_executor = BoundedToolExecutor::new(
            catalog,
            ReactorToolBackend {
                snapshot: tools.clone(),
                ports: self.tools.clone(),
                max_result_chars: plan.tool_result_chars,
            },
        );
        let no_tools = NoTools;
        let tool_executor: &dyn ToolExecutor = if tools_enabled {
            &bounded_executor
        } else {
            &no_tools
        };
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
                    scan_coverage(tools.scan.as_ref()),
                    &self.messages,
                ) {
                    emitter.scan_request(&ScanRequestPayload {
                        session_id: CHAT_SESSION_ID.to_string(),
                        message_id,
                        source_scan_id: tools
                            .scan
                            .as_ref()
                            .map_or_else(String::new, |scan| scan.session_id.clone()),
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

/// Cloneable handle the component holds on the UI thread.
pub struct NativeChatRuntime {
    /// Option so Drop can release the sender BEFORE joining the worker;
    /// joining while the sender is alive would deadlock (recv never
    /// disconnects on the shutting-down UI thread).
    commands: Option<std_mpsc::Sender<ChatCommand>>,
    active: Arc<Mutex<Option<(u64, CancellationToken)>>>,
    worker: Option<JoinHandle<()>>,
}

fn clear_active_turn(active: &Mutex<Option<(u64, CancellationToken)>>, request_id: u64) {
    if let Ok(mut active) = active.lock()
        && active
            .as_ref()
            .is_some_and(|(active_request_id, _)| *active_request_id == request_id)
    {
        *active = None;
    }
}

impl NativeChatRuntime {
    /// Start with explicit read-only diagnostic/history ports.
    pub fn start_with_ports(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        tools: ChatToolPorts,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ChatWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ChatCommand>();
        let (events, event_rx) = std_mpsc::channel::<ChatWorkerEvent>();
        let active = Arc::new(Mutex::new(None));
        let worker_active = Arc::clone(&active);
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-chat".to_string())
            .spawn(move || {
                let mut state = WorkerState {
                    source: ShellChatSource::new(settings, foundry, ollama),
                    events,
                    messages: Vec::new(),
                    tools,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ChatCommand::Send {
                            request_id,
                            prompt,
                            provider,
                            fallback_from,
                            allow_fallback,
                            tools,
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
                                    *tools,
                                    cancel,
                                ));
                                runtime.shutdown_timeout(Duration::from_secs(1));
                            } else {
                                let emitter = WorkerEmitter::new(request_id, state.events.clone());
                                emitter.fail_once(
                                    "The native AI chat runtime could not start".to_string(),
                                    "error",
                                );
                                emitter.flush_terminal();
                            }
                            clear_active_turn(&worker_active, request_id);
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
        tools: ChatToolSnapshot,
    ) -> bool {
        self.queue_attempt(
            None,
            request_id,
            prompt,
            provider,
            fallback_from,
            allow_fallback,
            tools,
        )
    }

    /// Queue the next physical attempt immediately after a clean failure.
    /// The worker may still be clearing `previous_request_id` when the UI
    /// receives that terminal event, so replacement is an explicit atomic
    /// operation instead of a timing-dependent retry.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn retry_attempt(
        &self,
        previous_request_id: u64,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        tools: ChatToolSnapshot,
    ) -> bool {
        self.queue_attempt(
            Some(previous_request_id),
            request_id,
            prompt,
            provider,
            fallback_from,
            allow_fallback,
            tools,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_attempt(
        &self,
        replaces_request_id: Option<u64>,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        fallback_from: Option<AIProvider>,
        allow_fallback: bool,
        tools: ChatToolSnapshot,
    ) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let cancel = CancellationToken::new();
        let Ok(mut active) = self.active.lock() else {
            return false;
        };
        if let Some((active_request_id, _)) = active.as_ref()
            && replaces_request_id != Some(*active_request_id)
        {
            return false;
        }
        *active = Some((request_id, cancel.clone()));
        drop(active);
        if commands
            .send(ChatCommand::Send {
                request_id,
                prompt,
                provider,
                fallback_from,
                allow_fallback,
                tools: Box::new(tools),
                cancel,
            })
            .is_ok()
        {
            true
        } else {
            clear_active_turn(&self.active, request_id);
            false
        }
    }

    #[must_use]
    pub fn cancel(&self) -> bool {
        self.active.lock().is_ok_and(|active| {
            active.as_ref().is_some_and(|(_, cancel)| {
                cancel.cancel();
                true
            })
        })
    }

    /// Cancel the active turn and clear backend-owned history in queue order.
    #[must_use]
    pub fn new_session(&self) -> bool {
        let _ = self.cancel();
        self.commands
            .as_ref()
            .is_some_and(|commands| commands.send(ChatCommand::Reset).is_ok())
    }
}

impl Drop for NativeChatRuntime {
    fn drop(&mut self) {
        // Take the active turn's token and clear the slot BEFORE cancelling:
        // cancel() runs registered wakers synchronously, and a waker that
        // touches this same slot mutex would deadlock against the guard.
        let active_cancel = self.active.lock().ok().and_then(|mut slot| {
            let cancel = slot.as_ref().map(|(_, cancel)| cancel.clone());
            *slot = None;
            cancel
        });
        if let Some(cancel) = active_cancel {
            cancel.cancel();
        }
        // Release the command sender first so the worker's recv()
        // disconnects; joining before that deadlocks the shutting-down UI
        // thread (the graceful-close hang root cause).
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            // A turn that ignores cancellation (a vendor CLI that will not
            // die) must not extend the graceful-close path.
            crate::teardown_support::reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wfdiag_native_ai_chat::ToolCall;
    use wfdiag_native_diagnostics::{DiagnosticFuture, DiagnosticOutput};
    use wfdiag_native_history::HistoryRuntimeConfig;

    #[derive(Default)]
    struct FixedDiagnostics {
        calls: AtomicUsize,
    }

    impl DiagnosticExecutor for FixedDiagnostics {
        fn available_tasks(&self) -> Vec<DiagnosticTask> {
            vec![DiagnosticTask {
                id: "os_info".to_string(),
                name: "Operating system".to_string(),
                description: "Collect Windows version information".to_string(),
                category: "System".to_string(),
                admin_required: false,
            }]
        }

        fn execute(&self, task_id: String) -> DiagnosticFuture<'_> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                DiagnosticOutput {
                    success: true,
                    output: format!(r#"{{"task":"{task_id}","build":26100}}"#),
                    error: None,
                    duration_ms: 4,
                }
            })
        }
    }

    fn disk_cleanup() -> RemediationSummary {
        wfdiag_native_issues::remediation_summaries()
            .into_iter()
            .find(|summary| summary.id == "open_disk_cleanup")
            .expect("canonical remediation must exist")
    }

    fn low_disk_issue(remediation: RemediationSummary) -> Issue {
        Issue {
            id: "low_disk_space".to_string(),
            category: "Storage".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: "Low disk space".to_string(),
            description: "C: has little free space".to_string(),
            recommendation: "Review Disk Cleanup".to_string(),
            detected: true,
            source_tasks: Some(vec!["disk_space".to_string()]),
            remediation: Some(remediation),
        }
    }

    fn backend_snapshot() -> (Arc<FixedDiagnostics>, ReactorToolBackend) {
        let diagnostics = Arc::new(FixedDiagnostics::default());
        let remediation = disk_cleanup();
        let backend = ReactorToolBackend {
            snapshot: ChatToolSnapshot {
                system_overview: Some("Windows 11 · ARM64".to_string()),
                scan: Some(ChatScanSnapshot {
                    session_id: "scan-1".to_string(),
                    started_at: SystemTime::now(),
                    scan_kind: ScanKind::Quick,
                    selected_tasks: vec!["os_info".to_string()],
                    results: vec![DiagnosticTaskResult::new(
                        "scan-1",
                        "os_info",
                        Arc::new(wfdiag_native_issues::TaskResult {
                            success: true,
                            output: "Windows 11".to_string(),
                            error: None,
                            duration_ms: 3,
                        }),
                    )],
                    running: false,
                }),
                issues: vec![low_disk_issue(remediation.clone())],
                history: vec![ScanSummary {
                    id: "stored-1".to_string(),
                    timestamp: wfdiag_native_history::Timestamp { secs: 1 },
                    computer_name: "TEST-PC".to_string(),
                    task_count: 1,
                    success_count: 1,
                    failure_count: 0,
                    duration_ms: 3,
                    label: None,
                    tags: vec!["Quick Scan".to_string()],
                }],
                remediations: vec![remediation],
                ..ChatToolSnapshot::default()
            },
            ports: ChatToolPorts::new(diagnostics.clone(), None),
            max_result_chars: 2_000,
        };
        (diagnostics, backend)
    }

    fn execute(
        executor: &impl ToolExecutor,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, String> {
        let call = ToolCall {
            id: "call-1".to_string(),
            name: name.to_string(),
            arguments,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(executor.execute(&call, CancellationToken::new()))
    }

    #[test]
    fn reactor_exposes_exactly_the_canonical_ten_tools() {
        let remediation = disk_cleanup();
        let catalog = tool_catalog(
            &FixedDiagnostics::default().available_tasks(),
            &[remediation],
        );
        let names = catalog
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "run_diagnostic",
                "search_windows_knowledge",
                "get_scan_summary",
                "request_full_scan",
                "get_detected_issues",
                "compare_with_previous_scan",
                "get_live_stats",
                "list_remediations",
                "list_scan_history",
                "stage_remediation",
            ]
        );
        assert!(!names.iter().any(|name| name == "get_system_overview"));
    }

    #[test]
    fn representative_read_only_tools_return_bounded_snapshot_data() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = tool_catalog(
            &diagnostics.available_tasks(),
            &backend.snapshot.remediations,
        );
        let executor = BoundedToolExecutor::new(catalog, backend);

        let summary = execute(&executor, "get_scan_summary", json!({})).unwrap();
        assert!(summary.contains("SCAN_SCOPE kind=quick"));
        assert!(summary.contains("Windows 11 · ARM64"));

        let issues = execute(&executor, "get_detected_issues", json!({})).unwrap();
        assert!(issues.contains("low_disk_space"));
        assert!(issues.contains("open_disk_cleanup"));

        let remediations = execute(&executor, "list_remediations", json!({})).unwrap();
        assert!(remediations.contains("open_disk_cleanup"));

        let history = execute(&executor, "list_scan_history", json!({})).unwrap();
        assert!(history.contains("stored-1"));

        let comparison = execute(&executor, "compare_with_previous_scan", json!({})).unwrap();
        assert!(comparison.contains("current scan has not been projected"));

        let diagnostic = execute(
            &executor,
            "run_diagnostic",
            json!({"task_id":"os_info", "reason":"verify build"}),
        )
        .unwrap();
        assert!(diagnostic.contains("\"build\": 26100"));
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn full_scan_and_remediation_calls_only_stage_typed_ui_requests() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = tool_catalog(
            &diagnostics.available_tasks(),
            &backend.snapshot.remediations,
        );
        let executor = BoundedToolExecutor::new(catalog, backend);

        let scan = execute(
            &executor,
            "request_full_scan",
            json!({"reason":"need broader driver evidence"}),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&scan).unwrap()["kind"],
            "scan_request"
        );

        let proposal = execute(
            &executor,
            "stage_remediation",
            json!({
                "remediation_id":"open_disk_cleanup",
                "issue_id":"low_disk_space"
            }),
        )
        .unwrap();
        let proposal = serde_json::from_str::<serde_json::Value>(&proposal).unwrap();
        assert_eq!(proposal["kind"], "staged_action_proposal");
        assert_eq!(proposal["proposal"]["remediationId"], "open_disk_cleanup");
        assert!(
            proposal["notice"]
                .as_str()
                .unwrap()
                .contains("nothing was executed")
        );
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn canonical_parser_rejects_arbitrary_commands_and_catalog_ids() {
        let (diagnostics, backend) = backend_snapshot();
        let catalog = tool_catalog(
            &diagnostics.available_tasks(),
            &backend.snapshot.remediations,
        );
        let executor = BoundedToolExecutor::new(catalog, backend);

        assert!(
            execute(
                &executor,
                "run_diagnostic",
                json!({"task_id":"powershell", "reason":"run arbitrary command"}),
            )
            .unwrap_err()
            .contains("Unknown diagnostic")
        );
        assert!(
            execute(&executor, "get_live_stats", json!({"command":"whoami"}))
                .unwrap_err()
                .contains("does not accept")
        );
        assert_eq!(diagnostics.calls.load(Ordering::SeqCst), 0);
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
        let emitter = WorkerEmitter::new(7, events);
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

    fn history_scan(id: &str, secs: i64, success: bool) -> ScanRecord {
        ScanRecord {
            id: id.to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs },
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: false,
            results: HashMap::from([(
                "os_info".to_string(),
                Arc::new(wfdiag_native_history::TaskResult {
                    success,
                    output: if success {
                        "collected".to_string()
                    } else {
                        "collection failed".to_string()
                    },
                    error: (!success).then(|| "access denied".to_string()),
                    duration_ms: 3,
                }),
            )]),
            task_count: 1,
            success_count: usize::from(success),
            failure_count: usize::from(!success),
            duration_ms: 3,
            label: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn history_tools_query_the_worker_and_compare_live_current_to_latest() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wfdiag-reactor-chat-history-{}-{unique}",
            std::process::id()
        ));
        let history = Arc::new(
            NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
                directory.clone(),
                || (true, 30),
                Vec::new,
            ))
            .unwrap(),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                history
                    .request_save(history_scan("stored", 1, false))
                    .unwrap(),
            )
            .unwrap()
            .unwrap();

        let diagnostics = Arc::new(FixedDiagnostics::default());
        let remediation = disk_cleanup();
        let backend = ReactorToolBackend {
            snapshot: ChatToolSnapshot {
                current_history_scan: Some(history_scan("live", 2, true)),
                history: vec![ScanSummary {
                    id: "stale-ui-copy".to_string(),
                    timestamp: wfdiag_native_history::Timestamp { secs: 0 },
                    computer_name: "TEST-PC".to_string(),
                    task_count: 0,
                    success_count: 0,
                    failure_count: 0,
                    duration_ms: 0,
                    label: None,
                    tags: Vec::new(),
                }],
                remediations: vec![remediation.clone()],
                ..ChatToolSnapshot::default()
            },
            ports: ChatToolPorts::new(diagnostics.clone(), Some(Arc::clone(&history))),
            max_result_chars: 2_000,
        };
        let executor = BoundedToolExecutor::new(
            tool_catalog(&diagnostics.available_tasks(), &[remediation]),
            backend,
        );

        let list = execute(&executor, "list_scan_history", json!({})).unwrap();
        assert!(list.contains("stored"));
        assert!(!list.contains("stale-ui-copy"));
        let comparison = execute(&executor, "compare_with_previous_scan", json!({})).unwrap();
        assert!(comparison.contains("1 change(s)"));
        assert!(comparison.contains("Newly collected: os_info"));

        drop(executor);
        drop(history);
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn error_plus_done_flushes_exactly_one_terminal_event() {
        let (events, receiver) = std_mpsc::channel();
        let emitter = WorkerEmitter::new(7, events);
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
        let emitter = WorkerEmitter::new(8, events);
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
        let previous_cancel = CancellationToken::new();
        let active = Arc::new(Mutex::new(Some((7, previous_cancel))));
        let runtime = NativeChatRuntime {
            commands: Some(commands),
            active: Arc::clone(&active),
            worker: None,
        };
        assert!(runtime.retry_attempt(
            7,
            8,
            "retry me".to_string(),
            AIProvider::Ollama,
            Some(AIProvider::PhiSilica),
            false,
            ChatToolSnapshot::default(),
        ));
        assert_eq!(
            active
                .lock()
                .unwrap()
                .as_ref()
                .map(|(request_id, _)| *request_id),
            Some(8)
        );
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
        let emitter = WorkerEmitter::new(9, events);
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

        let (commands, _receiver) = std_mpsc::channel();
        let cancel = CancellationToken::new();
        let runtime = NativeChatRuntime {
            commands: Some(commands),
            active: Arc::new(Mutex::new(Some((9, cancel.clone())))),
            worker: None,
        };
        assert!(runtime.cancel());
        assert!(cancel.is_cancelled());
    }
}
