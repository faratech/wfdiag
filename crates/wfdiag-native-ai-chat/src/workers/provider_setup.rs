//! Off-UI provider setup and live model-catalog runtime.
//!
//! The host submits one typed draft and drains typed events. Settings and
//! DPAPI credential reads, local-provider discovery, HTTP requests, and
//! subscription CLI processes all execute on a dedicated worker. This module
//! exposes no login, logout, install, or model-execution operation.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::{
    AIProvider, FoundryEndpointSource, ModelCatalogRequest, ModelCatalogService, OllamaSource,
    SettingsProviderKeySource, SubscriptionModelCatalogSource,
};
use wfdiag_native_settings::SettingsService;

use crate::ProcessSubscriptionModelCatalogSource;
use crate::workers::{
    ActiveRequestSlot, WorkerWake, build_worker_runtime, reap_worker, send_worker_event,
};

pub use wfdiag_native_ai_provider::ModelCatalog;

/// Events drained by the host component on its UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSetupWorkerEvent {
    Ack {
        request_id: u64,
        provider: AIProvider,
    },
    ModelsLoaded {
        request_id: u64,
        provider: AIProvider,
        catalog: ModelCatalog,
    },
    Failed {
        request_id: u64,
        provider: AIProvider,
        message: String,
    },
    Cancelled {
        request_id: u64,
        provider: AIProvider,
    },
}

impl ProviderSetupWorkerEvent {
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Ack { request_id, .. }
            | Self::ModelsLoaded { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id, .. } => *request_id,
        }
    }
}

enum ProviderSetupCommand {
    ListModels {
        request_id: u64,
        request: ModelCatalogRequest,
        cancel: CancellationToken,
    },
}

#[derive(Clone)]
struct ProviderSetupPorts {
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    subscriptions: Arc<dyn SubscriptionModelCatalogSource>,
}

impl ProviderSetupPorts {
    fn service(&self) -> ModelCatalogService {
        ModelCatalogService::new(
            self.settings.load_nonsecret_settings().unwrap_or_default(),
            Arc::new(SettingsProviderKeySource(self.settings.clone())),
            Arc::clone(&self.foundry),
            Arc::clone(&self.ollama),
            Arc::clone(&self.subscriptions),
        )
    }
}

struct WorkerState {
    ports: ProviderSetupPorts,
    events: mpsc::Sender<ProviderSetupWorkerEvent>,
    wake: WorkerWake,
    active: ActiveRequestSlot,
}

impl WorkerState {
    async fn list_models(
        &self,
        request_id: u64,
        request: ModelCatalogRequest,
        cancel: CancellationToken,
    ) {
        let provider = request.provider;
        send_worker_event(
            &self.events,
            &self.wake,
            ProviderSetupWorkerEvent::Ack {
                request_id,
                provider,
            },
        );
        let service = self.ports.service();
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            result = service.list(request) => Some(result),
        };
        self.active.clear(request_id);
        let event = match result {
            None => ProviderSetupWorkerEvent::Cancelled {
                request_id,
                provider,
            },
            Some(Ok(catalog)) => ProviderSetupWorkerEvent::ModelsLoaded {
                request_id,
                provider,
                catalog,
            },
            Some(Err(message)) => ProviderSetupWorkerEvent::Failed {
                request_id,
                provider,
                message,
            },
        };
        send_worker_event(&self.events, &self.wake, event);
    }
}

/// UI-thread handle for provider setup model discovery.
pub struct ProviderSetupRuntime {
    commands: Option<mpsc::Sender<ProviderSetupCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveRequestSlot,
}

impl ProviderSetupRuntime {
    /// Start the persistent worker. Construction has no discovery, auth, or
    /// install side effect; processes/network are touched only by
    /// [`Self::list_models`].
    ///
    /// # Errors
    /// When the worker thread or its Tokio runtime cannot be created.
    pub fn start(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, mpsc::Receiver<ProviderSetupWorkerEvent>)> {
        Self::start_with_subscriptions(
            settings,
            foundry,
            ollama,
            Arc::new(ProcessSubscriptionModelCatalogSource::new()),
            wake,
        )
    }

    fn start_with_subscriptions(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        subscriptions: Arc<dyn SubscriptionModelCatalogSource>,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, mpsc::Receiver<ProviderSetupWorkerEvent>)> {
        let (commands, command_rx) = mpsc::channel::<ProviderSetupCommand>();
        let (events, event_rx) = mpsc::channel::<ProviderSetupWorkerEvent>();
        let active = ActiveRequestSlot::new();
        let worker_active = active.clone();
        let runtime = build_worker_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-provider-setup".to_string())
            .spawn(move || {
                let state = WorkerState {
                    ports: ProviderSetupPorts {
                        settings,
                        foundry,
                        ollama,
                        subscriptions,
                    },
                    events,
                    wake,
                    active: worker_active,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ProviderSetupCommand::ListModels {
                            request_id,
                            request,
                            cancel,
                        } => runtime.block_on(state.list_models(request_id, request, cancel)),
                    }
                }
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

    /// Queue one live catalog request. Returns `false` if another request is
    /// active or the worker has stopped. The draft API key is moved directly
    /// to the worker and is never included in a worker event.
    #[must_use]
    pub fn list_models(&self, request_id: u64, request: ModelCatalogRequest) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancel) = self.active.register(request_id, None) else {
            return false;
        };
        if commands
            .send(ProviderSetupCommand::ListModels {
                request_id,
                request,
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

    /// Cancel directly, without queueing behind a slow HTTP or CLI call.
    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        self.active.cancel(request_id)
    }

    /// Cancel any in-flight request, stop the worker, and wait up to `budget`.
    ///
    /// Returns `false` when the worker was still running when the budget
    /// expired; the handle has already been handed to a detached reaper either
    /// way, so the caller never blocks past `budget`.
    pub fn stop_and_join(&mut self, budget: Duration) -> bool {
        self.cancel_and_release();
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (done, finished) = mpsc::channel();
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

impl Drop for ProviderSetupRuntime {
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
    use crate::workers::no_wake;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use wfdiag_native_ai_provider::{BackendFuture, SubscriptionCli};
    use wfdiag_native_settings::{
        AllowAllSettings, CredentialStorage, ProviderKeyId, SettingsError, SettingsStorage,
    };

    #[derive(Default)]
    struct MemorySettings;

    impl SettingsStorage for MemorySettings {
        fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
            Ok(None)
        }

        fn save(&self, _serialized: &[u8]) -> Result<(), SettingsError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryCredentials(Mutex<HashMap<ProviderKeyId, String>>);

    impl CredentialStorage for MemoryCredentials {
        fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
            self.0.lock().unwrap().insert(provider, key.to_string());
            Ok(())
        }

        fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
            Ok(self.0.lock().unwrap().get(&provider).cloned())
        }

        fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
            self.0.lock().unwrap().remove(&provider);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeFoundry;

    impl FoundryEndpointSource for FakeFoundry {
        fn probe(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
    }

    #[derive(Default)]
    struct FakeOllama;

    impl OllamaSource for FakeOllama {
        fn discover(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { configured })
        }

        fn list_models(&self, _endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async {
                Ok(vec![
                    "llama3.2:latest".to_string(),
                    "phi4:latest".to_string(),
                ])
            })
        }
    }

    #[derive(Default)]
    struct PendingSubscriptions;

    impl SubscriptionModelCatalogSource for PendingSubscriptions {
        fn list_models(
            &self,
            _provider: SubscriptionCli,
            _configured_path: Option<String>,
        ) -> BackendFuture<'_, Result<ModelCatalog, String>> {
            Box::pin(std::future::pending())
        }
    }

    fn settings() -> SettingsService {
        SettingsService::new(
            Arc::new(MemorySettings),
            Arc::new(MemoryCredentials::default()),
            Arc::new(AllowAllSettings),
        )
    }

    fn runtime() -> (
        ProviderSetupRuntime,
        mpsc::Receiver<ProviderSetupWorkerEvent>,
    ) {
        ProviderSetupRuntime::start_with_subscriptions(
            settings(),
            Arc::new(FakeFoundry),
            Arc::new(FakeOllama),
            Arc::new(PendingSubscriptions),
            no_wake(),
        )
        .unwrap()
    }

    #[test]
    fn model_discovery_runs_off_thread_and_emits_typed_catalog() {
        let (runtime, events) = runtime();
        assert!(runtime.list_models(
            41,
            ModelCatalogRequest {
                provider: AIProvider::Ollama,
                draft_endpoint: Some("http://ollama.test".to_string()),
                ..ModelCatalogRequest::default()
            }
        ));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            ProviderSetupWorkerEvent::Ack {
                request_id: 41,
                provider: AIProvider::Ollama
            }
        ));
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            ProviderSetupWorkerEvent::ModelsLoaded {
                request_id,
                provider,
                catalog,
            } => {
                assert_eq!(request_id, 41);
                assert_eq!(provider, AIProvider::Ollama);
                assert_eq!(catalog.default_model.as_deref(), Some("llama3.2:latest"));
                assert_eq!(catalog.models.len(), 2);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn cancellation_is_out_of_band_and_busy_requests_fail_closed() {
        let (runtime, events) = runtime();
        assert!(runtime.list_models(7, ModelCatalogRequest::new(AIProvider::CodexCli)));
        assert!(!runtime.list_models(8, ModelCatalogRequest::new(AIProvider::Ollama)));
        assert!(runtime.cancel(7));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            ProviderSetupWorkerEvent::Ack { request_id: 7, .. }
        ));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            ProviderSetupWorkerEvent::Cancelled { request_id: 7, .. }
        ));
    }

    #[test]
    fn stop_and_join_cancels_the_pending_request_and_is_idempotent() {
        let (mut runtime, events) = runtime();
        assert!(runtime.list_models(9, ModelCatalogRequest::new(AIProvider::CodexCli)));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            ProviderSetupWorkerEvent::Ack { request_id: 9, .. }
        ));

        assert!(runtime.stop_and_join(Duration::from_secs(2)));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            ProviderSetupWorkerEvent::Cancelled { request_id: 9, .. }
        ));
        assert!(!runtime.list_models(10, ModelCatalogRequest::new(AIProvider::Ollama)));
        assert!(runtime.stop_and_join(Duration::from_millis(1)));
    }
}
