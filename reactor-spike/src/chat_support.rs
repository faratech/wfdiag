//! Native AI chat runtime for the Reactor shell.
//!
//! A single std worker thread owns a current-thread Tokio runtime, the
//! conversation state, and the shared [`wfdiag_native_ai_chat`] turn engine.
//! The WinUI thread only enqueues commands and drains typed events, so no
//! Tokio, provider probe, or streaming work ever runs on the UI thread.
//!
//! Provider transport follows the extraction boundary: the chat-completions
//! providers (cloud OpenAI, Foundry Local, Ollama, custom endpoints) stream
//! through the shared client; providers whose transports still live behind
//! the shipping backend resolve to a clear "not provided yet" error.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{
    ChatEmitter, ChatMessage, ChatRole, CompatChatProvider, DeltaPayload, DonePayload,
    ErrorPayload, ProviderUse, ToolExecutor, ToolFuture, ToolPayload, TurnStatus,
    build_system_prompt, plan_context, run_chat_turn,
};
use wfdiag_native_ai_provider::{
    AIProvider, CompatConfigPorts, FoundryEndpointSource, OllamaSource, ProviderKeySource,
    compat_caps, resolve_compat_config,
};
use wfdiag_native_settings::{ProviderKeyId, SettingsService};

use serde_json::json;

/// Stable engine session id. The Reactor shell keeps one conversation.
pub const CHAT_SESSION_ID: &str = "reactor-chat";

/// Credential and endpoint ports for one resolved turn. Rebuilt per turn so
/// settings edits and saved keys apply to the very next message. Shared with
/// the report worker, which resolves the same provider set.
pub(crate) struct ShellChatSource {
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
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
        /// System snapshot for the read-only `get_system_overview` tool.
        overview: Option<String>,
    },
    Cancel,
}

/// Typed worker events drained by the component.
#[derive(Clone)]
pub enum ChatWorkerEvent {
    Delta {
        request_id: u64,
        text: String,
    },
    ToolActivity {
        request_id: u64,
        summary: String,
    },
    Done {
        request_id: u64,
        provider: String,
    },
    Failed {
        request_id: u64,
        message: String,
    },
    Cancelled {
        request_id: u64,
    },
}

impl ChatWorkerEvent {
    /// The originating send's identity, used for stale-event rejection.
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Delta { request_id, .. }
            | Self::ToolActivity { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id } => *request_id,
        }
    }
}

struct WorkerEmitter {
    request_id: u64,
    events: std_mpsc::Sender<ChatWorkerEvent>,
}

impl ChatEmitter for WorkerEmitter {
    fn delta(&self, payload: &DeltaPayload) {
        let _ = self.events.send(ChatWorkerEvent::Delta {
            request_id: self.request_id,
            text: payload.text.clone(),
        });
    }

    fn tool(&self, payload: &ToolPayload) {
        let _ = self.events.send(ChatWorkerEvent::ToolActivity {
            request_id: self.request_id,
            summary: format!("{} · {}", payload.tool, payload.status),
        });
    }

    fn done(&self, payload: &DonePayload) {
        let _ = self.events.send(ChatWorkerEvent::Done {
            request_id: self.request_id,
            provider: payload.provider.clone(),
        });
    }

    fn error(&self, payload: &ErrorPayload) {
        let _ = self.events.send(ChatWorkerEvent::Failed {
            request_id: self.request_id,
            message: payload.message.clone(),
        });
    }
}

/// The report path's no-tool executor: chat tools are a separate extraction
/// and the engine enforces read-only budgets around them.
struct NoTools;

impl ToolExecutor for NoTools {
    fn execute<'a>(&'a self, _call: &'a wfdiag_native_ai_chat::ToolCall, _cancel: CancellationToken)
    -> ToolFuture<'a> {
        Box::pin(async { Err("Native chat has no tools yet".to_string()) })
    }
}

struct WorkerState {
    source: ShellChatSource,
    events: std_mpsc::Sender<ChatWorkerEvent>,
    messages: Vec<ChatMessage>,
    cancel: Option<CancellationToken>,
    overview: Option<String>,
    overview_executor: SystemOverviewExecutor,
}

/// The first read-only tool: returns the shell-provided system snapshot.
/// The model can only ever read this text — it never reaches an argv.
struct SystemOverviewExecutor {
    overview: Option<String>,
}

impl ToolExecutor for SystemOverviewExecutor {
    fn execute<'a>(&'a self, call: &'a wfdiag_native_ai_chat::ToolCall, _cancel: CancellationToken)
    -> ToolFuture<'a> {
        Box::pin(async move {
            if call.name != "get_system_overview" {
                return Err(format!("Unknown tool '{}'", call.name));
            }
            Ok(self.overview.clone().unwrap_or_else(|| {
                "System information is not available in this session.".to_string()
            }))
        })
    }
}

/// Tool contract exposed to tool-capable providers when a snapshot exists.
#[must_use]
pub fn system_overview_spec() -> wfdiag_native_ai_chat::ToolSpec {
    wfdiag_native_ai_chat::ToolSpec {
        name: "get_system_overview".to_string(),
        description: "Return a snapshot of this PC's identity and operating system:                       computer name, Windows edition/version, CPU architecture, and                       elevation. Use it whenever the answer depends on the user's                       hardware or Windows version."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

impl WorkerState {
    async fn run_turn(
        &mut self,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        overview: Option<String>,
    ) {
        let _ = &self.overview_executor;
        let ports = self.source.ports();
        let cfg = match resolve_compat_config(provider, &ports).await {
            Ok(cfg) => cfg,
            Err(message) => {
                let _ = self.events.send(ChatWorkerEvent::Failed { request_id, message });
                return;
            }
        };
        self.overview = overview;
        let caps = compat_caps(provider);
        let plan = plan_context(caps.context_budget_chars);
        let tools_enabled = caps.supports_tools && self.overview.is_some();
        let system = build_system_prompt(tools_enabled, false, None, &plan);
        let message_id = format!("chat_{request_id}");
        let chat = CompatChatProvider { provider, cfg };
        let mut provider_use = ProviderUse::for_provider(provider, None);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            content: prompt,
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            tool_result_is_error: false,
            provider_replay: None,
        });
        let emitter = WorkerEmitter {
            request_id,
            events: self.events.clone(),
        };
        let outcome = run_chat_turn(
            &mut provider_use,
            caps,
            &chat,
            CHAT_SESSION_ID,
            &message_id,
            &mut self.messages,
            &system,
            &if tools_enabled {
                vec![system_overview_spec()]
            } else {
                Vec::new()
            },
            match tools_enabled {
                true => &self.overview_executor,
                false => &NoTools as &dyn ToolExecutor,
            },
            &emitter,
            cancel,
            false,
        )
        .await;
        self.cancel = None;
        match outcome {
            Ok(TurnStatus::Completed { .. }) => {
                let _ = self.events.send(ChatWorkerEvent::Done {
                    request_id,
                    provider: provider_use.provider_id.clone(),
                });
            }
            Ok(TurnStatus::Cancelled) => {
                self.truncate_failed_turn();
                let _ = self.events.send(ChatWorkerEvent::Cancelled { request_id });
            }
            Ok(TurnStatus::Error) | Err(_) => {
                self.truncate_failed_turn();
            }
        }
    }

    /// Drop the trailing user message when no assistant reply was recorded,
    /// so a retried question does not duplicate context.
    fn truncate_failed_turn(&mut self) {
        if self.messages.last().is_some_and(|message| message.role == ChatRole::User) {
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
    worker: Option<JoinHandle<()>>,
}

impl NativeChatRuntime {
    /// Prepare the worker channel pair and spawn the OS thread.
    ///
    /// # Errors
    /// When the worker thread cannot be spawned.
    pub fn start(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ChatWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ChatCommand>();
        let (events, event_rx) = std_mpsc::channel::<ChatWorkerEvent>();
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-chat".to_string())
            .spawn(move || {
                let mut state = WorkerState {
                    source: ShellChatSource::new(settings, foundry, ollama),
                    events,
                    messages: Vec::new(),
                    cancel: None,
                    overview: None,
                    overview_executor: SystemOverviewExecutor { overview: None },
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ChatCommand::Send {
                            request_id,
                            prompt,
                            provider,
                            overview,
                        } => {
                            // The Tokio runtime exists only while a turn runs; an
                            // idle runtime's IO/time drivers were observed to keep
                            // the WinUI dispatcher from finishing window teardown.
                            let runtime = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build();
                            if let Ok(runtime) = runtime {
                                runtime.block_on(state.run_turn(request_id, prompt, provider, overview));
                            }
                        }
                        ChatCommand::Cancel => {
                            if let Some(cancel) = state.cancel.as_ref() {
                                cancel.cancel();
                            }
                        }
                    }
                }
            })?;
        Ok((
            Self {
                commands: Some(commands),
                worker: Some(worker),
            },
            event_rx,
        ))
    }

    pub fn send(
        &self,
        request_id: u64,
        prompt: String,
        provider: AIProvider,
        overview: Option<String>,
    ) {
        if let Some(commands) = self.commands.as_ref() {
            let _ = commands.send(ChatCommand::Send {
                request_id,
                prompt,
                provider,
                overview,
            });
        }
    }

    #[must_use]
    pub fn cancel(&self) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|commands| commands.send(ChatCommand::Cancel).is_ok())
    }
}

impl Drop for NativeChatRuntime {
    fn drop(&mut self) {
        // Release the command sender first so the worker's recv()
        // disconnects; joining before that deadlocks the shutting-down UI
        // thread (the graceful-close hang root cause).
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Poll cadence for the component's chat wait task (mirrors the other
/// backend wait loops).
pub const CHAT_WAIT_POLL: Duration = Duration::from_millis(100);

#[cfg(test)]
mod tests {
    use super::*;

    /// The turn boundary must keep the conversation clean: a user message
    /// with no assistant reply is retried, not duplicated.
    #[test]
    fn failed_turn_drops_trailing_user_message() {
        let (events, _rx) = std_mpsc::channel();
        let mut state = WorkerState {
            source: ShellChatSource {
                settings: SettingsService::new(
                    Arc::new(wfdiag_native_settings::ShippingSettingsStorage::at_path(
                        std::env::temp_dir().join("wfdiag-chat-test-unused.json"),
                    )),
                    Arc::new(wfdiag_native_settings::WindowsDpapiCredentialStorage::new()),
                    Arc::new(wfdiag_native_settings::AllowAllSettings),
                ),
                foundry: unreachable_foundry(),
                ollama: unreachable_ollama(),
            },
            events,
            overview: None,
            overview_executor: SystemOverviewExecutor { overview: None },
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "why is disk full?".to_string(),
                tool_calls: Vec::new(),
                tool_call_id: None,
                tool_name: None,
                tool_result_is_error: false,
                provider_replay: None,
            }],
            cancel: None,
        };
        state.truncate_failed_turn();
        assert!(state.messages.is_empty());
        // An assistant reply already recorded means the turn completed and
        // nothing is dropped.
        state.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: "here is why".to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            tool_result_is_error: false,
            provider_replay: None,
        });
        state.truncate_failed_turn();
        assert_eq!(state.messages.len(), 1);
    }

    fn unreachable_foundry() -> Arc<dyn FoundryEndpointSource> {
        struct No;
        impl FoundryEndpointSource for No {
            fn probe(
                &self,
                _configured: Option<String>,
            ) -> wfdiag_native_ai_provider::BackendFuture<'_, Option<String>> {
                Box::pin(async { None })
            }
        }
        Arc::new(No)
    }

    fn unreachable_ollama() -> Arc<dyn OllamaSource> {
        struct No;
        impl OllamaSource for No {
            fn discover(
                &self,
                _configured: Option<String>,
            ) -> wfdiag_native_ai_provider::BackendFuture<'_, Option<String>> {
                Box::pin(async { None })
            }
            fn list_models(
                &self,
                _endpoint: String,
            ) -> wfdiag_native_ai_provider::BackendFuture<'_, Result<Vec<String>, String>> {
                Box::pin(async { Ok(Vec::new()) })
            }
        }
        Arc::new(No)
    }
}
