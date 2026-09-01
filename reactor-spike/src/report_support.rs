//! Native AI scan report runtime for the Reactor shell.
//!
//! Mirrors the chat worker: one std command thread owns a persistent Tokio
//! runtime that drives the shared [`wfdiag_native_ai_report::ReportService`] —
//! provider routing policy, deterministic evidence, cache identity, and
//! duplicate suppression all live in the crate. The WinUI thread only
//! enqueues commands and drains typed events.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{ChatProvider, CompatChatProvider, ProviderUse};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, CompatConfigPorts, FoundryEndpointSource, OllamaSource,
    ProviderAvailability, SharedAiCache, SubscriptionConfigPorts, next_auto_local_route,
    parse_provider_preference, provider_config_fingerprint, resolve_compat_config,
    resolve_subscription_config,
};
use wfdiag_native_ai_report::{
    ReportAck, ReportDeltaPayload, ReportDonePayload, ReportEmitter, ReportErrorPayload,
    ReportFuture, ReportProviderResolver, ReportRequest, ReportService, ResolvedReportProvider,
};
use wfdiag_native_history::ComparisonResult;
use wfdiag_native_phi::PhiChatProvider;
use wfdiag_native_settings::SettingsService;

use crate::chat_support::ShellChatSource;
use crate::ui_wake_support::NotifySenderExt;

pub use wfdiag_native_ai_report::ReportScan;

/// Immutable inputs for one native report generation.
///
/// The UI assembles this only after its provider-status and history snapshots
/// are ready, keeping those asynchronous dependencies out of the report
/// worker and making each command internally consistent.
#[derive(Clone, Debug)]
pub struct ReportGeneration {
    pub scan: ReportScan,
    pub provider: AIProvider,
    pub availability: ProviderAvailability,
    pub comparison: Option<ComparisonResult>,
    pub force_refresh: bool,
}

/// Worker commands.
pub enum ReportCommand {
    Generate {
        request_id: u64,
        generation: ReportGeneration,
        cancel: CancellationToken,
    },
}

/// Typed worker events drained by the component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportWorkerEvent {
    Ack {
        request_id: u64,
        provider: String,
        provider_use: ProviderUse,
    },
    Delta {
        request_id: u64,
        text: String,
    },
    Done {
        request_id: u64,
        provider: String,
        cached: bool,
        finish_reason: String,
        provider_use: ProviderUse,
        /// Cached reports are returned inline because the shared service does
        /// not emit delta events for cache hits.
        report: Option<String>,
    },
    Failed {
        request_id: u64,
        message: String,
        finish_reason: String,
    },
    Cancelled {
        request_id: u64,
        finish_reason: String,
    },
}

impl ReportWorkerEvent {
    /// The originating generate's identity, for stale-event rejection.
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Ack { request_id, .. }
            | Self::Delta { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id, .. } => *request_id,
        }
    }
}

#[derive(Default)]
struct ReportEmissionState {
    acknowledged: bool,
    terminal_sent: bool,
    pending_error: Option<String>,
    pending_events: Vec<ReportWorkerEvent>,
}

#[derive(Clone)]
struct ActiveReportRequest {
    request_id: u64,
    cancel: CancellationToken,
}

type ActiveReportSlot = Arc<Mutex<Option<ActiveReportRequest>>>;

fn active_slot() -> ActiveReportSlot {
    Arc::new(Mutex::new(None))
}

fn register_active_request(
    active: &ActiveReportSlot,
    request_id: u64,
) -> Option<CancellationToken> {
    let mut slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_some() {
        return None;
    }
    let cancel = CancellationToken::new();
    *slot = Some(ActiveReportRequest {
        request_id,
        cancel: cancel.clone(),
    });
    Some(cancel)
}

fn cancel_active_request(active: &ActiveReportSlot, request_id: u64) -> bool {
    let slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(cancel) = slot
        .as_ref()
        .filter(|request| request.request_id == request_id)
        .map(|request| request.cancel.clone())
    else {
        return false;
    };
    drop(slot);
    cancel.cancel();
    true
}

fn clear_active_request(active: &ActiveReportSlot, request_id: u64) {
    let mut slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|request| request.request_id == request_id)
    {
        *slot = None;
    }
}

fn cancel_any_active_request(active: &ActiveReportSlot) {
    let slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cancel = slot.as_ref().map(|request| request.cancel.clone());
    drop(slot);
    if let Some(cancel) = cancel {
        cancel.cancel();
    }
}

struct WorkerReportEmitter {
    request_id: u64,
    events: std_mpsc::Sender<ReportWorkerEvent>,
    state: Mutex<ReportEmissionState>,
    terminal: CancellationToken,
    active: ActiveReportSlot,
}

impl WorkerReportEmitter {
    fn new(
        request_id: u64,
        events: std_mpsc::Sender<ReportWorkerEvent>,
        active: ActiveReportSlot,
    ) -> Self {
        Self {
            request_id,
            events,
            state: Mutex::new(ReportEmissionState::default()),
            terminal: CancellationToken::new(),
            active,
        }
    }

    fn acknowledged(&self, ack: &ReportAck) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.acknowledged || (state.terminal_sent && state.pending_events.is_empty()) {
            return;
        }
        state.acknowledged = true;
        let ack_event = ReportWorkerEvent::Ack {
            request_id: self.request_id,
            provider: ack.provider.clone(),
            provider_use: ack.provider_use.clone(),
        };
        let _ = self.events.send_and_wake(ack_event);
        for event in std::mem::take(&mut state.pending_events) {
            let _ = self.events.send_and_wake(event);
        }
    }

    fn finish_once(&self, event: ReportWorkerEvent) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        state.terminal_sent = true;
        let _ = self.events.send_and_wake(event);
        drop(state);
        self.terminal.cancel();
        clear_active_request(&self.active, self.request_id);
    }

    /// Complete an uncached stream, buffering its terminal event until the
    /// acknowledgement is visible. `ReportService` spawns the provider task
    /// before returning its ack, so a fast provider can otherwise finish on
    /// the runtime worker before the command thread publishes attribution.
    fn finish_stream_once(&self, make_event: impl FnOnce(Option<String>) -> ReportWorkerEvent) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        let event = make_event(state.pending_error.take());
        state.terminal_sent = true;
        if state.acknowledged {
            let _ = self.events.send_and_wake(event);
        } else {
            state.pending_events.push(event);
        }
        drop(state);

        self.terminal.cancel();
        clear_active_request(&self.active, self.request_id);
    }

    fn fail_once(&self, message: String, finish_reason: &str) {
        self.finish_once(ReportWorkerEvent::Failed {
            request_id: self.request_id,
            message,
            finish_reason: finish_reason.to_string(),
        });
    }

    fn cancel_once(&self) {
        self.finish_once(ReportWorkerEvent::Cancelled {
            request_id: self.request_id,
            finish_reason: "cancelled".to_string(),
        });
    }
}

impl ReportEmitter for WorkerReportEmitter {
    fn delta(&self, payload: &ReportDeltaPayload) {
        let event = ReportWorkerEvent::Delta {
            request_id: self.request_id,
            text: payload.text.clone(),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        if state.acknowledged {
            let _ = self.events.send_and_wake(event);
        } else {
            state.pending_events.push(event);
        }
    }

    fn done(&self, payload: &ReportDonePayload) {
        self.finish_stream_once(|pending_error| match payload.finish_reason.as_str() {
            "cancelled" => ReportWorkerEvent::Cancelled {
                request_id: self.request_id,
                finish_reason: payload.finish_reason.clone(),
            },
            "error" | "refusal" => ReportWorkerEvent::Failed {
                request_id: self.request_id,
                message: pending_error
                    .unwrap_or_else(|| "The AI report did not complete".to_string()),
                finish_reason: payload.finish_reason.clone(),
            },
            _ => ReportWorkerEvent::Done {
                request_id: self.request_id,
                provider: payload.provider.clone(),
                cached: false,
                finish_reason: payload.finish_reason.clone(),
                provider_use: payload.provider_use.clone(),
                report: None,
            },
        });
    }

    fn error(&self, payload: &ReportErrorPayload) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.terminal_sent {
            state.pending_error = Some(payload.message.clone());
        }
    }
}

fn cached_report_event(request_id: u64, ack: &ReportAck) -> ReportWorkerEvent {
    ReportWorkerEvent::Done {
        request_id,
        provider: ack.provider.clone(),
        cached: true,
        // ReportService only caches clean `stop` completions.
        finish_reason: "stop".to_string(),
        provider_use: ack.provider_use.clone(),
        report: ack.report.clone(),
    }
}

/// One report's routing inputs. The active provider and complete availability
/// snapshot come from the same provider-status response, so the report core's
/// Phi-to-local policy cannot race a second, independently ordered probe.
struct TurnReportResolver {
    ports: Arc<CompatConfigPorts>,
    subscription_ports: Arc<SubscriptionConfigPorts>,
    provider: AIProvider,
    availability: ProviderAvailability,
}

impl ReportProviderResolver for TurnReportResolver {
    fn preference(&self) -> AIProviderPreference {
        parse_provider_preference(&self.ports.settings.preferred_ai_provider)
    }

    fn determine_active(&self, _preference: AIProviderPreference) -> ReportFuture<'_, AIProvider> {
        Box::pin(async move { self.provider })
    }

    fn next_auto_local(
        &self,
        preference: AIProviderPreference,
        tried: &[AIProvider],
    ) -> ReportFuture<'_, Option<AIProvider>> {
        let next = next_auto_local_route(preference, tried, self.availability);
        Box::pin(async move { next })
    }

    fn resolve(
        &self,
        provider: AIProvider,
    ) -> ReportFuture<'_, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            if provider == AIProvider::PhiSilica {
                return Ok(ResolvedReportProvider {
                    chat: Arc::new(PhiChatProvider),
                    config_fingerprint: "provider=phi_silica;runtime=windows_ai".to_string(),
                    requested_model: None,
                });
            }
            let cfg = match provider {
                AIProvider::CodexCli | AIProvider::ClaudeCode => {
                    resolve_subscription_config(provider, &self.subscription_ports).await?
                }
                _ => resolve_compat_config(provider, &self.ports).await?,
            };
            let requested_model = cfg.model.clone();
            let config_fingerprint = provider_config_fingerprint(provider, &cfg);
            let chat: Arc<dyn ChatProvider> = Arc::new(CompatChatProvider { provider, cfg });
            Ok(ResolvedReportProvider {
                chat,
                config_fingerprint,
                requested_model,
            })
        })
    }
}

struct WorkerState {
    source: ShellChatSource,
    events: std_mpsc::Sender<ReportWorkerEvent>,
    service: ReportService,
    active: ActiveReportSlot,
}

fn service_request(
    scan: ReportScan,
    comparison: Option<ComparisonResult>,
    force_refresh: bool,
) -> ReportRequest {
    ReportRequest {
        scan,
        comparison,
        force_refresh,
        detection_now: wfdiag_native_issues::Timestamp::now(),
    }
}

impl WorkerState {
    async fn run_generate(
        &self,
        request_id: u64,
        generation: ReportGeneration,
        cancel: CancellationToken,
    ) {
        let ReportGeneration {
            scan,
            provider,
            availability,
            comparison,
            force_refresh,
        } = generation;
        let emitter = Arc::new(WorkerReportEmitter::new(
            request_id,
            self.events.clone(),
            Arc::clone(&self.active),
        ));
        let request = service_request(scan, comparison, force_refresh);
        if cancel.is_cancelled() {
            emitter.cancel_once();
            return;
        }
        let ports = Arc::new(self.source.ports());
        if cancel.is_cancelled() {
            emitter.cancel_once();
            return;
        }
        let subscription_ports = Arc::new(self.source.subscription_ports());
        if cancel.is_cancelled() {
            emitter.cancel_once();
            return;
        }
        let resolver = TurnReportResolver {
            ports,
            subscription_ports,
            provider,
            availability,
        };
        let report_emitter: Arc<dyn ReportEmitter> = emitter.clone();
        let generated = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            result = self.service.generate(request, Arc::new(resolver), report_emitter) => {
                Some(result)
            }
        };
        let Some(generated) = generated else {
            emitter.cancel_once();
            return;
        };

        match generated {
            Ok(ack @ ReportAck { cached: true, .. }) => {
                // Cached answers return inline and emit no later events.
                emitter.acknowledged(&ack);
                emitter.finish_once(cached_report_event(request_id, &ack));
            }
            Ok(ack @ ReportAck { cached: false, .. }) => {
                emitter.acknowledged(&ack);
                let service = self.service.clone();
                let terminal = emitter.terminal.clone();
                let report_id = ack.report_id;
                tokio::spawn(async move {
                    tokio::select! {
                        biased;
                        () = terminal.cancelled() => {}
                        () = cancel.cancelled() => {
                            service.cancel(&report_id).await;
                            // The normal cancellation path emits `done` before
                            // cleanup wakes ReportService::cancel. Keep a
                            // terminal fallback for a provider task that exits
                            // abnormally without delivering that event.
                            if !terminal.is_cancelled() {
                                emitter.cancel_once();
                            }
                        }
                    }
                });
            }
            Err(message) => {
                emitter.fail_once(message, "error");
            }
        }
    }
}

fn build_report_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

/// UI-thread handle for the native report worker.
pub struct NativeReportRuntime {
    /// Option so Drop can release the sender BEFORE joining the worker.
    commands: Option<std_mpsc::Sender<ReportCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveReportSlot,
}

impl NativeReportRuntime {
    /// Start the worker.
    ///
    /// # Errors
    /// When the worker thread cannot be spawned.
    pub fn start(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        cache: SharedAiCache,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ReportWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ReportCommand>();
        let (events, event_rx) = std_mpsc::channel::<ReportWorkerEvent>();
        let active = active_slot();
        let worker_active = Arc::clone(&active);
        // ReportService::generate returns before its spawned streaming task.
        // Keep one runtime alive for the whole worker lifetime so that task is
        // polled while this command thread waits for the next send. Cancellation
        // is signalled directly through the active request token.
        let runtime = build_report_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-report".to_string())
            .spawn(move || {
                let state = WorkerState {
                    source: ShellChatSource::new(settings, foundry, ollama),
                    events,
                    service: ReportService::new(cache),
                    active: worker_active,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ReportCommand::Generate {
                            request_id,
                            generation,
                            cancel,
                        } => {
                            runtime.block_on(state.run_generate(request_id, generation, cancel));
                        }
                    }
                }
                // Do not let a spawned provider task hold native window
                // teardown indefinitely after the command channel closes.
                runtime.shutdown_timeout(Duration::from_secs(1));
            })?;
        Ok((
            Self {
                commands: Some(commands),
                worker: Some(worker),
                active,
            },
            event_rx,
        ))
    }

    pub fn generate(&self, request_id: u64, generation: ReportGeneration) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancel) = register_active_request(&self.active, request_id) else {
            return false;
        };
        if commands
            .send(ReportCommand::Generate {
                request_id,
                generation,
                cancel: cancel.clone(),
            })
            .is_ok()
        {
            true
        } else {
            cancel.cancel();
            clear_active_request(&self.active, request_id);
            false
        }
    }

    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        cancel_active_request(&self.active, request_id)
    }
}

impl Drop for NativeReportRuntime {
    fn drop(&mut self) {
        cancel_any_active_request(&self.active);
        self.commands = None;
        {
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *active = None;
        }
        if let Some(worker) = self.worker.take() {
            // An in-flight request that ignores cancellation (a hung vendor
            // CLI, a slow provider probe) must not extend graceful close.
            crate::teardown_support::reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_history::{ScanSummary, Timestamp};

    fn done_payload(finish_reason: &str) -> ReportDonePayload {
        ReportDonePayload {
            report_id: "report-9".to_string(),
            finish_reason: finish_reason.to_string(),
            provider: "openai".to_string(),
            provider_use: ProviderUse::for_provider(AIProvider::OpenAI, None),
        }
    }

    fn summary(id: &str) -> ScanSummary {
        ScanSummary {
            id: id.to_string(),
            timestamp: Timestamp::from_secs(1_788_112_800),
            computer_name: "TEST-PC".to_string(),
            task_count: 1,
            success_count: 1,
            failure_count: 0,
            duration_ms: 5,
            label: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn report_error_and_done_become_one_terminal_event() {
        let (events, receiver) = std_mpsc::channel();
        let active = active_slot();
        register_active_request(&active, 9).expect("register request");
        let emitter = WorkerReportEmitter::new(9, events, Arc::clone(&active));
        let ack = ReportAck {
            report_id: "report-9".to_string(),
            cached: false,
            provider: "openai".to_string(),
            provider_use: ProviderUse::for_provider(AIProvider::OpenAI, None),
            report: None,
        };
        emitter.error(&ReportErrorPayload {
            report_id: "report-9".to_string(),
            message: "report provider failed".to_string(),
        });
        emitter.done(&done_payload("error"));
        emitter.done(&done_payload("error"));
        assert!(receiver.try_recv().is_err());
        emitter.acknowledged(&ack);

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Ack {
                request_id: 9,
                provider: "openai".to_string(),
                provider_use: ack.provider_use,
            }
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Failed {
                request_id: 9,
                message: "report provider failed".to_string(),
                finish_reason: "error".to_string(),
            }
        );
        assert!(receiver.try_recv().is_err());
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none(),
            "a terminal error must release the active request"
        );
    }

    #[test]
    fn preparation_failure_releases_active_without_late_ack() {
        let (events, receiver) = std_mpsc::channel();
        let active = active_slot();
        register_active_request(&active, 10).expect("register request");
        let emitter = WorkerReportEmitter::new(10, events, Arc::clone(&active));
        emitter.fail_once("provider configuration failed".to_string(), "error");
        emitter.acknowledged(&ReportAck {
            report_id: "report-10".to_string(),
            cached: false,
            provider: "openai".to_string(),
            provider_use: ProviderUse::for_provider(AIProvider::OpenAI, None),
            report: None,
        });

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Failed {
                request_id: 10,
                message: "provider configuration failed".to_string(),
                finish_reason: "error".to_string(),
            }
        );
        assert!(receiver.try_recv().is_err());
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn cached_ack_keeps_the_inline_report_body() {
        let provider_use = ProviderUse::for_provider(AIProvider::OpenAI, None)
            .with_requested_model(Some("gpt-5-nano"));
        let ack = ReportAck {
            report_id: "cached-12".to_string(),
            cached: true,
            provider: "openai".to_string(),
            provider_use: provider_use.clone(),
            report: Some("## Health summary\nHealthy".to_string()),
        };
        let event = cached_report_event(12, &ack);

        assert_eq!(
            event,
            ReportWorkerEvent::Done {
                request_id: 12,
                provider: "openai".to_string(),
                cached: true,
                finish_reason: "stop".to_string(),
                provider_use,
                report: Some("## Health summary\nHealthy".to_string()),
            }
        );
    }

    #[test]
    fn acknowledgement_and_done_preserve_provider_attribution() {
        let (events, receiver) = std_mpsc::channel();
        let active = active_slot();
        register_active_request(&active, 14).expect("register request");
        let emitter = WorkerReportEmitter::new(14, events, Arc::clone(&active));
        let mut provider_use =
            ProviderUse::for_provider(AIProvider::FoundryLocal, Some(AIProvider::PhiSilica))
                .with_requested_model(Some("phi-4-mini"));
        let ack = ReportAck {
            report_id: "report-14".to_string(),
            cached: false,
            provider: "foundry_local".to_string(),
            provider_use: provider_use.clone(),
            report: None,
        };
        emitter.acknowledged(&ack);
        provider_use.set_actual_models(["phi-4-mini-instruct".to_string()]);
        emitter.done(&ReportDonePayload {
            report_id: "report-14".to_string(),
            finish_reason: "stop".to_string(),
            provider: "foundry_local".to_string(),
            provider_use: provider_use.clone(),
        });

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Ack {
                request_id: 14,
                provider: "foundry_local".to_string(),
                provider_use: ack.provider_use,
            }
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Done {
                request_id: 14,
                provider: "foundry_local".to_string(),
                cached: false,
                finish_reason: "stop".to_string(),
                provider_use,
                report: None,
            }
        );
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn fast_stream_events_are_published_after_acknowledgement() {
        let (events, receiver) = std_mpsc::channel();
        let active = active_slot();
        register_active_request(&active, 15).expect("register request");
        let emitter = WorkerReportEmitter::new(15, events, Arc::clone(&active));
        let provider_use = ProviderUse::for_provider(AIProvider::Gemini, None)
            .with_requested_model(Some("gemini-2.5-flash"));
        let ack = ReportAck {
            report_id: "report-15".to_string(),
            cached: false,
            provider: "gemini".to_string(),
            provider_use: provider_use.clone(),
            report: None,
        };

        emitter.delta(&ReportDeltaPayload {
            report_id: "report-15".to_string(),
            text: "Healthy".to_string(),
        });
        emitter.done(&ReportDonePayload {
            report_id: "report-15".to_string(),
            finish_reason: "stop".to_string(),
            provider: "gemini".to_string(),
            provider_use: provider_use.clone(),
        });
        assert!(receiver.try_recv().is_err());

        emitter.acknowledged(&ack);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Ack {
                request_id: 15,
                provider: "gemini".to_string(),
                provider_use: provider_use.clone(),
            }
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Delta {
                request_id: 15,
                text: "Healthy".to_string(),
            }
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ReportWorkerEvent::Done {
                request_id: 15,
                provider: "gemini".to_string(),
                cached: false,
                finish_reason: "stop".to_string(),
                provider_use,
                report: None,
            }
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn cancellation_is_direct_identity_scoped_and_keeps_slot_until_terminal() {
        let active = active_slot();
        let cancel = register_active_request(&active, 21).expect("register request");
        assert!(register_active_request(&active, 22).is_none());
        assert!(!cancel_active_request(&active, 22));
        assert!(!cancel.is_cancelled());

        assert!(cancel_active_request(&active, 21));
        assert!(cancel.is_cancelled());
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some(),
            "the slot remains reserved until its terminal event"
        );

        clear_active_request(&active, 22);
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
        );
        clear_active_request(&active, 21);
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn failed_command_queue_releases_the_registered_request() {
        let (commands, command_receiver) = std_mpsc::channel();
        drop(command_receiver);
        let active = active_slot();
        let runtime = NativeReportRuntime {
            commands: Some(commands),
            worker: None,
            active: Arc::clone(&active),
        };

        assert!(!runtime.generate(
            23,
            ReportGeneration {
                scan: ReportScan {
                    session_id: "current".to_string(),
                    results: Default::default(),
                },
                provider: AIProvider::OpenAI,
                availability: ProviderAvailability::default(),
                comparison: None,
                force_refresh: false,
            }
        ));
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn dropping_runtime_cancels_and_clears_the_active_request() {
        let (commands, _command_receiver) = std_mpsc::channel();
        let active = active_slot();
        let cancel = register_active_request(&active, 24).expect("register request");
        let runtime = NativeReportRuntime {
            commands: Some(commands),
            worker: None,
            active: Arc::clone(&active),
        };

        drop(runtime);

        assert!(cancel.is_cancelled());
        assert!(
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn service_request_keeps_latest_comparison_and_refresh_policy() {
        let request = service_request(
            ReportScan {
                session_id: "current".to_string(),
                results: Default::default(),
            },
            Some(ComparisonResult {
                current_scan: summary("current"),
                previous_scan: summary("previous"),
                total_changes: 0,
                new_failures: Vec::new(),
                new_successes: Vec::new(),
                status_unchanged: Vec::new(),
            }),
            true,
        );

        assert_eq!(request.scan.session_id, "current");
        assert_eq!(
            request
                .comparison
                .as_ref()
                .map(|comparison| comparison.previous_scan.id.as_str()),
            Some("previous")
        );
        assert!(request.force_refresh);
    }

    #[test]
    fn auto_phi_report_reroutes_to_next_available_local_provider() {
        let availability = ProviderAvailability {
            phi: true,
            foundry: true,
            ollama: true,
            ..Default::default()
        };
        let next = next_auto_local_route(
            AIProviderPreference::Auto,
            &[AIProvider::PhiSilica],
            availability,
        );

        assert_eq!(next, Some(AIProvider::FoundryLocal));
        assert_eq!(
            wfdiag_native_ai_report::choose_report_provider(
                AIProviderPreference::Auto,
                AIProvider::PhiSilica,
                next,
            ),
            AIProvider::FoundryLocal
        );
    }

    #[test]
    fn persistent_runtime_drives_spawned_work_between_commands() {
        let runtime = build_report_runtime().unwrap();
        let (completed, receiver) = std_mpsc::channel();
        runtime.spawn(async move {
            let _ = completed.send(());
        });

        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the persistent runtime should keep polling spawned report work");
    }
}
