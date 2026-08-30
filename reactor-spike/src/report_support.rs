//! Native AI scan report runtime for the Reactor shell.
//!
//! Mirrors the chat worker: one std thread owning a current-thread Tokio
//! runtime drives the shared [`wfdiag_native_ai_report::ReportService`] —
//! provider routing policy, deterministic evidence, cache identity, and
//! duplicate suppression all live in the crate. The WinUI thread only
//! enqueues commands and drains typed events.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use wfdiag_native_ai_chat::{ChatProvider, CompatChatProvider};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, CompatConfigPorts, FoundryEndpointSource, OllamaSource,
    SharedAiCache, parse_provider_preference, provider_config_fingerprint, resolve_compat_config,
};
use wfdiag_native_ai_report::{
    ReportAck, ReportDeltaPayload, ReportDonePayload, ReportEmitter, ReportErrorPayload,
    ReportFuture, ReportProviderResolver, ReportRequest, ReportService, ResolvedReportProvider,
};
use wfdiag_native_settings::SettingsService;

use crate::chat_support::ShellChatSource;

pub use wfdiag_native_ai_report::ReportScan;

/// Worker commands.
pub enum ReportCommand {
    Generate {
        request_id: u64,
        scan: ReportScan,
        provider: AIProvider,
        force_refresh: bool,
    },
    Cancel {
        request_id: u64,
    },
}

/// Typed worker events drained by the component.
#[derive(Clone)]
pub enum ReportWorkerEvent {
    Delta {
        request_id: u64,
        text: String,
    },
    Done {
        request_id: u64,
        provider: String,
        cached: bool,
    },
    Failed {
        request_id: u64,
        message: String,
    },
    Cancelled {
        request_id: u64,
    },
}

impl ReportWorkerEvent {
    /// The originating generate's identity, for stale-event rejection.
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Delta { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id } => *request_id,
        }
    }
}

struct WorkerReportEmitter {
    request_id: u64,
    events: std_mpsc::Sender<ReportWorkerEvent>,
}

impl ReportEmitter for WorkerReportEmitter {
    fn delta(&self, payload: &ReportDeltaPayload) {
        let _ = self.events.send(ReportWorkerEvent::Delta {
            request_id: self.request_id,
            text: payload.text.clone(),
        });
    }

    fn done(&self, payload: &ReportDonePayload) {
        let _ = self.events.send(ReportWorkerEvent::Done {
            request_id: self.request_id,
            provider: payload.provider.clone(),
            cached: false,
        });
    }

    fn error(&self, payload: &ReportErrorPayload) {
        let _ = self.events.send(ReportWorkerEvent::Failed {
            request_id: self.request_id,
            message: payload.message.clone(),
        });
    }
}

/// One report's routing inputs: the probed active provider comes from the
/// component's provider status; the Phi-to-local reroute is deliberately not
/// re-probed here (no local provider means the compat resolver reports the
/// gap), and comparison baselines arrive with a later increment.
struct TurnReportResolver {
    ports: Arc<CompatConfigPorts>,
    provider: AIProvider,
}

impl ReportProviderResolver for TurnReportResolver {
    fn preference(&self) -> AIProviderPreference {
        parse_provider_preference(&self.ports.settings.preferred_ai_provider)
    }

    fn determine_active(
        &self,
        _preference: AIProviderPreference,
    ) -> ReportFuture<'_, AIProvider> {
        Box::pin(async move { self.provider })
    }

    fn next_auto_local(
        &self,
        _preference: AIProviderPreference,
        _tried: &[AIProvider],
    ) -> ReportFuture<'_, Option<AIProvider>> {
        Box::pin(async { None })
    }

    fn resolve(
        &self,
        provider: AIProvider,
    ) -> ReportFuture<'_, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            let cfg = resolve_compat_config(provider, &self.ports).await?;
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
    in_flight: Option<(u64, String)>,
}

impl WorkerState {
    async fn run_generate(
        &mut self,
        request_id: u64,
        scan: ReportScan,
        provider: AIProvider,
        force_refresh: bool,
    ) {
        let ports = Arc::new(self.source.ports());
        let resolver = TurnReportResolver {
            ports,
            provider,
        };
        let emitter: Arc<dyn ReportEmitter> = Arc::new(WorkerReportEmitter {
            request_id,
            events: self.events.clone(),
        });
        let request = ReportRequest {
            scan,
            comparison: None,
            force_refresh,
            detection_now: wfdiag_native_issues::Timestamp::now(),
        };
        match self
            .service
            .generate(request, Arc::new(resolver), emitter)
            .await
        {
            Ok(ReportAck {
                report_id: _,
                cached: true,
                provider,
                ..
            }) => {
                // Cached answers return inline and emit no later events.
                let _ = self.events.send(ReportWorkerEvent::Done {
                    request_id,
                    provider,
                    cached: true,
                });
            }
            Ok(ReportAck {
                report_id,
                cached: false,
                ..
            }) => {
                self.in_flight = Some((request_id, report_id));
            }
            Err(message) => {
                let _ = self.events.send(ReportWorkerEvent::Failed { request_id, message });
            }
        }
    }
}

/// Cloneable handle the component holds on the UI thread.
pub struct NativeReportRuntime {
    commands: std_mpsc::Sender<ReportCommand>,
    worker: Option<JoinHandle<()>>,
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
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ReportWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ReportCommand>();
        let (events, event_rx) = std_mpsc::channel::<ReportWorkerEvent>();
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-report".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    return;
                };
                let mut state = WorkerState {
                    source: ShellChatSource::new(settings, foundry, ollama),
                    events,
                    service: ReportService::new(SharedAiCache::new(100)),
                    in_flight: None,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ReportCommand::Generate {
                            request_id,
                            scan,
                            provider,
                            force_refresh,
                        } => {
                            runtime.block_on(
                                state.run_generate(request_id, scan, provider, force_refresh),
                            );
                        }
                        ReportCommand::Cancel { request_id } => {
                            if let Some((pending, report_id)) = state.in_flight.as_ref()
                                && *pending == request_id
                            {
                                let report_id = report_id.clone();
                                let service = state.service.clone();
                                runtime.block_on(service.cancel(&report_id));
                                let _ = state.events.send(ReportWorkerEvent::Cancelled { request_id });
                            }
                        }
                    }
                }
            })?;
        Ok((
            Self {
                commands,
                worker: Some(worker),
            },
            event_rx,
        ))
    }

    pub fn generate(
        &self,
        request_id: u64,
        scan: ReportScan,
        provider: AIProvider,
        force_refresh: bool,
    ) {
        let _ = self.commands.send(ReportCommand::Generate {
            request_id,
            scan,
            provider,
            force_refresh,
        });
    }

    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        self.commands
            .send(ReportCommand::Cancel { request_id })
            .is_ok()
    }
}

impl Drop for NativeReportRuntime {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
