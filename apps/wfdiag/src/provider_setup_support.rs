//! Off-UI provider setup and live model-catalog runtime for Reactor.
//!
//! The component submits one typed draft and drains typed events. Settings
//! and DPAPI credential reads, local-provider discovery, HTTP requests, and
//! subscription CLI processes all execute on a dedicated worker. This module
//! exposes no login, logout, install, or model-execution operation.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::ProcessSubscriptionModelCatalogSource;
use wfdiag_native_ai_provider::{
    AIProvider, FoundryEndpointSource, ModelCatalogRequest, ModelCatalogService, OllamaSource,
    SettingsProviderKeySource, SubscriptionModelCatalogSource,
};
use wfdiag_native_settings::SettingsService;

use crate::ui_wake_support::NotifySenderExt;

pub use wfdiag_native_ai_provider::ModelCatalog;

/// Events drained by the Reactor component on its UI thread.
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
struct ActiveRequest {
    request_id: u64,
    cancel: CancellationToken,
}

type ActiveSlot = Arc<Mutex<Option<ActiveRequest>>>;

fn active_slot() -> ActiveSlot {
    Arc::new(Mutex::new(None))
}

fn register_active(active: &ActiveSlot, request_id: u64) -> Option<CancellationToken> {
    let mut slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_some() {
        return None;
    }
    let cancel = CancellationToken::new();
    *slot = Some(ActiveRequest {
        request_id,
        cancel: cancel.clone(),
    });
    Some(cancel)
}

fn clear_active(active: &ActiveSlot, request_id: u64) {
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

fn cancel_active(active: &ActiveSlot, request_id: u64) -> bool {
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

fn cancel_any(active: &ActiveSlot) {
    let slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cancel = slot.as_ref().map(|request| request.cancel.clone());
    drop(slot);
    if let Some(cancel) = cancel {
        cancel.cancel();
    }
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
    events: std_mpsc::Sender<ProviderSetupWorkerEvent>,
    active: ActiveSlot,
}

impl WorkerState {
    async fn list_models(
        &self,
        request_id: u64,
        request: ModelCatalogRequest,
        cancel: CancellationToken,
    ) {
        let provider = request.provider;
        let _ = self.events.send_and_wake(ProviderSetupWorkerEvent::Ack {
            request_id,
            provider,
        });
        let service = self.ports.service();
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            result = service.list(request) => Some(result),
        };
        clear_active(&self.active, request_id);
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
        let _ = self.events.send_and_wake(event);
    }
}

fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

/// UI-thread handle for provider setup model discovery.
pub struct ProviderSetupRuntime {
    commands: Option<std_mpsc::Sender<ProviderSetupCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveSlot,
}

impl ProviderSetupRuntime {
    /// Start the persistent worker. Construction has no discovery, auth, or
    /// install side effect; processes/network are touched only by
    /// [`Self::list_models`].
    pub fn start(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ProviderSetupWorkerEvent>)> {
        Self::start_with_subscriptions(
            settings,
            foundry,
            ollama,
            Arc::new(ProcessSubscriptionModelCatalogSource::new()),
        )
    }

    fn start_with_subscriptions(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        subscriptions: Arc<dyn SubscriptionModelCatalogSource>,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<ProviderSetupWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ProviderSetupCommand>();
        let (events, event_rx) = std_mpsc::channel::<ProviderSetupWorkerEvent>();
        let active = active_slot();
        let worker_active = Arc::clone(&active);
        let runtime = build_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-provider-setup".to_string())
            .spawn(move || {
                let state = WorkerState {
                    ports: ProviderSetupPorts {
                        settings,
                        foundry,
                        ollama,
                        subscriptions,
                    },
                    events,
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
    pub fn list_models(&self, request_id: u64, request: ModelCatalogRequest) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancel) = register_active(&self.active, request_id) else {
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
            clear_active(&self.active, request_id);
            false
        }
    }

    /// Cancel directly, without queueing behind a slow HTTP or CLI call.
    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        cancel_active(&self.active, request_id)
    }
}

impl Drop for ProviderSetupRuntime {
    fn drop(&mut self) {
        cancel_any(&self.active);
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
    use std::collections::HashMap;
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

    #[test]
    fn model_discovery_runs_off_thread_and_emits_typed_catalog() {
        let (runtime, events) = ProviderSetupRuntime::start_with_subscriptions(
            settings(),
            Arc::new(FakeFoundry),
            Arc::new(FakeOllama),
            Arc::new(PendingSubscriptions),
        )
        .unwrap();
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
        let (runtime, events) = ProviderSetupRuntime::start_with_subscriptions(
            settings(),
            Arc::new(FakeFoundry),
            Arc::new(FakeOllama),
            Arc::new(PendingSubscriptions),
        )
        .unwrap();
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
}
