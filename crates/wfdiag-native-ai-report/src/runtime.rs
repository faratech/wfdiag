//! Off-UI report runtime: one worker thread owns a persistent Tokio runtime
//! that drives [`ReportService`].
//!
//! Provider routing policy, deterministic evidence, cache identity, and
//! duplicate suppression all live in the service; this runtime owns the
//! worker lifecycle, identity-scoped cancellation, and the projection of
//! service events onto one [`ReportWorkerEvent`] stream a host UI thread
//! drains. The host supplies only a [`ReportResolverFactory`] — the concrete,
//! platform-specific provider wiring for one generation — and a wake callback.

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{
    ActiveRequestSlot, ProviderUse, WorkerWake, build_worker_runtime, reap_worker,
    send_worker_event,
};
use wfdiag_native_ai_provider::{AIProvider, ProviderAvailability, SharedAiCache};
use wfdiag_native_history::ComparisonResult;

use crate::{
    ReportAck, ReportDeltaPayload, ReportDonePayload, ReportEmitter, ReportErrorPayload,
    ReportProviderResolver, ReportRequest, ReportScan, ReportService,
};

/// Immutable inputs for one report generation.
///
/// The host assembles this only after its provider-status and history
/// snapshots are ready, keeping those asynchronous dependencies out of the
/// report worker and making each command internally consistent.
#[derive(Clone, Debug)]
pub struct ReportGeneration {
    pub scan: ReportScan,
    pub provider: AIProvider,
    pub availability: ProviderAvailability,
    pub comparison: Option<ComparisonResult>,
    pub force_refresh: bool,
}

/// Host-owned provider wiring for one generation. The active provider and the
/// complete availability snapshot come from the same provider-status response,
/// so the report core's Phi-to-local policy cannot race a second,
/// independently ordered probe.
pub trait ReportResolverFactory: Send + 'static {
    fn resolver(
        &self,
        provider: AIProvider,
        availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver>;
}

/// Worker commands.
enum ReportCommand {
    Generate {
        request_id: u64,
        generation: Box<ReportGeneration>,
        cancel: CancellationToken,
    },
}

/// Typed worker events drained by the host component.
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
    #[must_use]
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

struct WorkerReportEmitter {
    request_id: u64,
    events: std_mpsc::Sender<ReportWorkerEvent>,
    wake: WorkerWake,
    state: Mutex<ReportEmissionState>,
    terminal: CancellationToken,
    active: ActiveRequestSlot,
}

impl WorkerReportEmitter {
    fn new(
        request_id: u64,
        events: std_mpsc::Sender<ReportWorkerEvent>,
        wake: WorkerWake,
        active: ActiveRequestSlot,
    ) -> Self {
        Self {
            request_id,
            events,
            wake,
            state: Mutex::new(ReportEmissionState::default()),
            terminal: CancellationToken::new(),
            active,
        }
    }

    fn publish(&self, event: ReportWorkerEvent) {
        send_worker_event(&self.events, &self.wake, event);
    }

    fn acknowledged(&self, ack: &ReportAck) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.acknowledged || (state.terminal_sent && state.pending_events.is_empty()) {
            return;
        }
        state.acknowledged = true;
        let ack_event = ReportWorkerEvent::Ack {
            request_id: self.request_id,
            provider: ack.provider.clone(),
            provider_use: ack.provider_use.clone(),
        };
        self.publish(ack_event);
        for event in std::mem::take(&mut state.pending_events) {
            self.publish(event);
        }
    }

    fn finish_once(&self, event: ReportWorkerEvent) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        state.terminal_sent = true;
        self.publish(event);
        drop(state);
        self.terminal.cancel();
        self.active.clear(self.request_id);
    }

    /// Complete an uncached stream, buffering its terminal event until the
    /// acknowledgement is visible. `ReportService` spawns the provider task
    /// before returning its ack, so a fast provider can otherwise finish on
    /// the runtime worker before the command thread publishes attribution.
    fn finish_stream_once(&self, make_event: impl FnOnce(Option<String>) -> ReportWorkerEvent) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        let event = make_event(state.pending_error.take());
        state.terminal_sent = true;
        if state.acknowledged {
            self.publish(event);
        } else {
            state.pending_events.push(event);
        }
        drop(state);

        self.terminal.cancel();
        self.active.clear(self.request_id);
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.terminal_sent {
            return;
        }
        if state.acknowledged {
            self.publish(event);
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
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

struct WorkerState {
    resolvers: Box<dyn ReportResolverFactory>,
    events: std_mpsc::Sender<ReportWorkerEvent>,
    wake: WorkerWake,
    service: ReportService,
    active: ActiveRequestSlot,
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
            Arc::clone(&self.wake),
            self.active.clone(),
        ));
        let request = service_request(scan, comparison, force_refresh);
        if cancel.is_cancelled() {
            emitter.cancel_once();
            return;
        }
        // Building the resolver reads settings and credentials, so a
        // cancellation that lands during it is observed on the way out.
        let resolver = self.resolvers.resolver(provider, availability);
        if cancel.is_cancelled() {
            emitter.cancel_once();
            return;
        }
        let report_emitter: Arc<dyn ReportEmitter> = emitter.clone();
        let generated = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            result = self.service.generate(request, resolver, report_emitter) => {
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

/// UI-thread handle for the native report worker.
pub struct NativeReportRuntime {
    /// Option so teardown can release the sender BEFORE joining the worker.
    commands: Option<std_mpsc::Sender<ReportCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveRequestSlot,
}

impl NativeReportRuntime {
    /// Start the worker.
    ///
    /// # Errors
    /// When the worker thread or its Tokio runtime cannot be created.
    pub fn start(
        resolvers: Box<dyn ReportResolverFactory>,
        cache: SharedAiCache,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ReportWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ReportCommand>();
        let (events, event_rx) = std_mpsc::channel::<ReportWorkerEvent>();
        let active = ActiveRequestSlot::new();
        let worker_active = active.clone();
        // ReportService::generate returns before its spawned streaming task.
        // Keep one runtime alive for the whole worker lifetime so that task is
        // polled while this command thread waits for the next send. Cancellation
        // is signalled directly through the active request token.
        let runtime = build_worker_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-report".to_string())
            .spawn(move || {
                let state = WorkerState {
                    resolvers,
                    events,
                    wake,
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
                            runtime.block_on(state.run_generate(request_id, *generation, cancel));
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

    /// Queue one report generation. Returns `false` while another report is
    /// in flight or after the worker has stopped.
    #[must_use]
    pub fn generate(&self, request_id: u64, generation: ReportGeneration) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancel) = self.active.register(request_id, None) else {
            return false;
        };
        if commands
            .send(ReportCommand::Generate {
                request_id,
                generation: Box::new(generation),
                cancel: cancel.clone(),
            })
            .is_ok()
        {
            true
        } else {
            cancel.cancel();
            self.active.clear(request_id);
            false
        }
    }

    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        self.active.cancel(request_id)
    }

    /// Cancel any in-flight report, stop the worker, and wait up to `budget`.
    ///
    /// Returns `false` when the worker was still running when the budget
    /// expired; the handle has already been handed to a detached reaper either
    /// way, so the caller never blocks past `budget`.
    pub fn stop_and_join(&mut self, budget: Duration) -> bool {
        self.cancel_and_release();
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (done, finished) = std_mpsc::channel();
        reap_worker(worker, Some(done));
        finished.recv_timeout(budget).is_ok()
    }

    fn cancel_and_release(&mut self) {
        if let Some(cancel) = self.active.take() {
            cancel.cancel();
        }
        self.commands = None;
    }
}

impl Drop for NativeReportRuntime {
    fn drop(&mut self) {
        self.cancel_and_release();
        if let Some(worker) = self.worker.take() {
            // An in-flight request that ignores cancellation (a hung vendor
            // CLI, a slow provider probe) must not extend graceful close.
            reap_worker(worker, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_ai_chat::no_wake;
    use wfdiag_native_ai_provider::{AIProviderPreference, next_auto_local_route};
    use wfdiag_native_history::{ScanSummary, Timestamp};
    use wfdiag_native_issues::SharedScanEvidence;

    fn emitter(
        request_id: u64,
        active: &ActiveRequestSlot,
    ) -> (WorkerReportEmitter, std_mpsc::Receiver<ReportWorkerEvent>) {
        let (events, receiver) = std_mpsc::channel();
        let _reserved = active
            .register(request_id, None)
            .expect("the slot must admit the request");
        (
            WorkerReportEmitter::new(request_id, events, no_wake(), active.clone()),
            receiver,
        )
    }

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
        let active = ActiveRequestSlot::new();
        let (emitter, receiver) = emitter(9, &active);
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
            active.is_idle(),
            "a terminal error must release the active request"
        );
    }

    #[test]
    fn preparation_failure_releases_active_without_late_ack() {
        let active = ActiveRequestSlot::new();
        let (emitter, receiver) = emitter(10, &active);
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
        assert!(active.is_idle());
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
        let active = ActiveRequestSlot::new();
        let (emitter, receiver) = emitter(14, &active);
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
        assert!(active.is_idle());
    }

    #[test]
    fn fast_stream_events_are_published_after_acknowledgement() {
        let active = ActiveRequestSlot::new();
        let (emitter, receiver) = emitter(15, &active);
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
        let active = ActiveRequestSlot::new();
        let cancel = active.register(21, None).expect("register request");
        assert!(active.register(22, None).is_none());
        assert!(!active.cancel(22));
        assert!(!cancel.is_cancelled());

        assert!(active.cancel(21));
        assert!(cancel.is_cancelled());
        assert!(
            !active.is_idle(),
            "the slot remains reserved until its terminal event"
        );

        active.clear(22);
        assert!(!active.is_idle());
        active.clear(21);
        assert!(active.is_idle());
    }

    #[test]
    fn failed_command_queue_releases_the_registered_request() {
        let (commands, command_receiver) = std_mpsc::channel();
        drop(command_receiver);
        let active = ActiveRequestSlot::new();
        let runtime = NativeReportRuntime {
            commands: Some(commands),
            worker: None,
            active: active.clone(),
        };

        assert!(!runtime.generate(
            23,
            ReportGeneration {
                scan: ReportScan {
                    session_id: "current".to_string(),
                    results: SharedScanEvidence::default(),
                },
                provider: AIProvider::OpenAI,
                availability: ProviderAvailability::default(),
                comparison: None,
                force_refresh: false,
            }
        ));
        assert!(active.is_idle());
    }

    #[test]
    fn dropping_runtime_cancels_and_clears_the_active_request() {
        let (commands, _command_receiver) = std_mpsc::channel();
        let active = ActiveRequestSlot::new();
        let cancel = active.register(24, None).expect("register request");
        let runtime = NativeReportRuntime {
            commands: Some(commands),
            worker: None,
            active: active.clone(),
        };

        drop(runtime);

        assert!(cancel.is_cancelled());
        assert!(active.is_idle());
    }

    #[test]
    fn service_request_keeps_latest_comparison_and_refresh_policy() {
        let request = service_request(
            ReportScan {
                session_id: "current".to_string(),
                results: SharedScanEvidence::default(),
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
            crate::choose_report_provider(AIProviderPreference::Auto, AIProvider::PhiSilica, next,),
            AIProvider::FoundryLocal
        );
    }

    #[test]
    fn persistent_runtime_drives_spawned_work_between_commands() {
        let runtime = build_worker_runtime().unwrap();
        let (completed, receiver) = std_mpsc::channel();
        runtime.spawn(async move {
            let _ = completed.send(());
        });

        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the persistent runtime should keep polling spawned report work");
    }
}
