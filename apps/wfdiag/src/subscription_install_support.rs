//! Off-UI, confirmation-preserving subscription CLI installer for Reactor.
//!
//! This adapter owns no installation policy. It carries the UI's two explicit
//! approvals into [`SubscriptionInstallController`], keeps one cancellation
//! token per accepted request, and projects the shared controller's typed
//! progress and terminal outcomes onto a standard channel drained by the
//! Reactor component. Dropping the runtime cancels and joins its active
//! request, which in turn closes the shared installer's Windows Job Object.

#![deny(unsafe_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{ProcessSubscriptionModelCatalogSource, SubscriptionInstallController};
use wfdiag_native_ai_provider::ProcessSubscriptionCliStatusSource;

use crate::ui_wake_support::NotifySenderExt;

pub use wfdiag_native_ai_chat::{
    SubscriptionAuthProvider, SubscriptionInstallError, SubscriptionInstallFallbackReason,
    SubscriptionInstallMethod, SubscriptionInstallProgress, SubscriptionInstallRequest,
    SubscriptionInstallStage, SubscriptionInstallStatus,
};
// Kept as part of the adapter's public type surface for shells that consume
// the verified status without importing the shared chat crate directly.
#[allow(unused_imports)]
pub use wfdiag_native_ai_chat::SubscriptionAuthState;

type InstallFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<SubscriptionInstallStatus, SubscriptionInstallError>>
            + Send
            + 'a,
    >,
>;
type ProgressSink = Arc<dyn Fn(SubscriptionInstallProgress) + Send + Sync>;

trait InstallBackend: Send + Sync + 'static {
    fn install(
        &self,
        request: SubscriptionInstallRequest,
        cancellation: CancellationToken,
        progress: ProgressSink,
    ) -> InstallFuture<'_>;
}

#[derive(Debug, Default)]
struct SharedInstallBackend(SubscriptionInstallController);

impl InstallBackend for SharedInstallBackend {
    fn install(
        &self,
        request: SubscriptionInstallRequest,
        cancellation: CancellationToken,
        progress: ProgressSink,
    ) -> InstallFuture<'_> {
        Box::pin(async move {
            self.0
                .install(request, cancellation, move |event| progress(event))
                .await
        })
    }
}

/// Typed events drained on Reactor's UI thread. Every event carries the
/// request ID so a dialog epoch can reject stale progress and terminal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionInstallWorkerEvent {
    Ack {
        request_id: u64,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
    },
    Progress {
        request_id: u64,
        progress: SubscriptionInstallProgress,
    },
    Installed {
        request_id: u64,
        status: SubscriptionInstallStatus,
    },
    VendorFallbackConfirmationRequired {
        request_id: u64,
        provider: SubscriptionAuthProvider,
        reason: SubscriptionInstallFallbackReason,
    },
    Failed {
        request_id: u64,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
        message: String,
    },
    Cancelled {
        request_id: u64,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
    },
}

impl SubscriptionInstallWorkerEvent {
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Ack { request_id, .. }
            | Self::Progress { request_id, .. }
            | Self::Installed { request_id, .. }
            | Self::VendorFallbackConfirmationRequired { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Cancelled { request_id, .. } => *request_id,
        }
    }

    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Installed { .. }
                | Self::VendorFallbackConfirmationRequired { .. }
                | Self::Failed { .. }
                | Self::Cancelled { .. }
        )
    }
}

enum InstallCommand {
    Run {
        request_id: u64,
        request: SubscriptionInstallRequest,
        cancellation: CancellationToken,
    },
}

#[derive(Clone)]
struct ActiveRequest {
    request_id: u64,
    cancellation: CancellationToken,
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
    let cancellation = CancellationToken::new();
    *slot = Some(ActiveRequest {
        request_id,
        cancellation: cancellation.clone(),
    });
    Some(cancellation)
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
    let Some(cancellation) = slot
        .as_ref()
        .filter(|request| request.request_id == request_id)
        .map(|request| request.cancellation.clone())
    else {
        return false;
    };
    drop(slot);
    cancellation.cancel();
    true
}

fn cancel_any(active: &ActiveSlot) {
    let slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cancellation = slot.as_ref().map(|request| request.cancellation.clone());
    drop(slot);
    if let Some(cancellation) = cancellation {
        cancellation.cancel();
    }
}

struct WorkerState {
    backend: Arc<dyn InstallBackend>,
    events: std_mpsc::Sender<SubscriptionInstallWorkerEvent>,
    active: ActiveSlot,
}

impl WorkerState {
    async fn install(
        &self,
        request_id: u64,
        request: SubscriptionInstallRequest,
        cancellation: CancellationToken,
    ) {
        let provider = request.provider;
        let method = request.method;
        let _ = self
            .events
            .send_and_wake(SubscriptionInstallWorkerEvent::Ack {
                request_id,
                provider,
                method,
            });
        let progress_events = self.events.clone();
        let progress: ProgressSink = Arc::new(move |progress| {
            let _ = progress_events.send_and_wake(SubscriptionInstallWorkerEvent::Progress {
                request_id,
                progress,
            });
        });
        let result = self.backend.install(request, cancellation, progress).await;

        // A package manager can change the installation even when its final
        // status is failure. Force subsequent status/model refreshes to probe.
        let shared_provider = provider.into();
        ProcessSubscriptionCliStatusSource::new().invalidate(shared_provider);
        ProcessSubscriptionModelCatalogSource::new().invalidate(shared_provider);

        clear_active(&self.active, request_id);
        let terminal = match result {
            Ok(status) => SubscriptionInstallWorkerEvent::Installed { request_id, status },
            Err(SubscriptionInstallError::VendorFallbackConfirmationRequired {
                reason, ..
            }) => SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                request_id,
                provider,
                reason,
            },
            Err(SubscriptionInstallError::Cancelled { .. }) => {
                SubscriptionInstallWorkerEvent::Cancelled {
                    request_id,
                    provider,
                    method,
                }
            }
            Err(error) => SubscriptionInstallWorkerEvent::Failed {
                request_id,
                provider,
                method,
                message: error.to_string(),
            },
        };
        debug_assert!(terminal.is_terminal());
        let _ = self.events.send_and_wake(terminal);
    }
}

fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

/// UI-thread handle for explicit subscription CLI installs.
pub struct SubscriptionInstallRuntime {
    commands: Option<std_mpsc::Sender<InstallCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveSlot,
}

impl SubscriptionInstallRuntime {
    /// Start a side-effect-free persistent worker. No probe or process occurs
    /// until one of the two explicit install methods is called.
    pub fn start() -> std::io::Result<(Self, std_mpsc::Receiver<SubscriptionInstallWorkerEvent>)> {
        Self::start_with_backend(Arc::new(SharedInstallBackend::default()))
    }

    fn start_with_backend(
        backend: Arc<dyn InstallBackend>,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<SubscriptionInstallWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<InstallCommand>();
        let (events, event_rx) = std_mpsc::channel::<SubscriptionInstallWorkerEvent>();
        let active = active_slot();
        let worker_active = Arc::clone(&active);
        let runtime = build_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-subscription-install".to_string())
            .spawn(move || {
                let state = WorkerState {
                    backend,
                    events,
                    active: worker_active,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        InstallCommand::Run {
                            request_id,
                            request,
                            cancellation,
                        } => runtime.block_on(state.install(request_id, request, cancellation)),
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

    /// Queue the allowlisted winget package after the first confirmation.
    pub fn install_with_winget(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
    ) -> bool {
        self.send(
            request_id,
            SubscriptionInstallRequest::winget(provider, confirmed),
        )
    }

    /// Queue the mutable vendor bootstrap. Both installation confirmation and
    /// the distinct fallback confirmation remain explicit inputs.
    pub fn install_with_vendor_fallback(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> bool {
        self.send(
            request_id,
            SubscriptionInstallRequest::vendor_fallback(provider, confirmed, fallback_confirmed),
        )
    }

    fn send(&self, request_id: u64, request: SubscriptionInstallRequest) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancellation) = register_active(&self.active, request_id) else {
            return false;
        };
        if commands
            .send(InstallCommand::Run {
                request_id,
                request,
                cancellation: cancellation.clone(),
            })
            .is_ok()
        {
            true
        } else {
            cancellation.cancel();
            clear_active(&self.active, request_id);
            false
        }
    }

    /// Cancel the matching request out-of-band. A stale request ID cannot
    /// cancel the current installer.
    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        cancel_active(&self.active, request_id)
    }
}

impl Drop for SubscriptionInstallRuntime {
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ScriptedBackend {
        requests: Mutex<Vec<SubscriptionInstallRequest>>,
        result: Result<SubscriptionInstallStatus, SubscriptionInstallError>,
        emit_progress: bool,
    }

    impl InstallBackend for ScriptedBackend {
        fn install(
            &self,
            request: SubscriptionInstallRequest,
            _cancellation: CancellationToken,
            progress: ProgressSink,
        ) -> InstallFuture<'_> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(request);
                if self.emit_progress {
                    progress(SubscriptionInstallProgress {
                        provider: request.provider,
                        method: request.method,
                        stage: SubscriptionInstallStage::InstallingWinget,
                    });
                }
                self.result.clone()
            })
        }
    }

    struct PendingBackend {
        cancellation_observed: Arc<AtomicBool>,
    }

    impl InstallBackend for PendingBackend {
        fn install(
            &self,
            request: SubscriptionInstallRequest,
            cancellation: CancellationToken,
            _progress: ProgressSink,
        ) -> InstallFuture<'_> {
            let observed = Arc::clone(&self.cancellation_observed);
            Box::pin(async move {
                cancellation.cancelled().await;
                observed.store(true, Ordering::Release);
                Err(SubscriptionInstallError::Cancelled {
                    provider: request.provider,
                    method: request.method,
                })
            })
        }
    }

    fn installed(provider: SubscriptionAuthProvider) -> SubscriptionInstallStatus {
        let path = match provider {
            SubscriptionAuthProvider::Codex => PathBuf::from(r"C:\Tools\codex.exe"),
            SubscriptionAuthProvider::ClaudeCode => PathBuf::from(r"C:\Tools\claude.exe"),
        };
        SubscriptionInstallStatus {
            provider,
            path,
            state: SubscriptionAuthState::SignedOut,
        }
    }

    #[test]
    fn success_preserves_request_identity_progress_and_verified_status() {
        let backend = Arc::new(ScriptedBackend {
            requests: Mutex::new(Vec::new()),
            result: Ok(installed(SubscriptionAuthProvider::Codex)),
            emit_progress: true,
        });
        let (runtime, events) =
            SubscriptionInstallRuntime::start_with_backend(backend.clone()).unwrap();
        assert!(runtime.install_with_winget(41, SubscriptionAuthProvider::Codex, true));

        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Ack {
                request_id: 41,
                provider: SubscriptionAuthProvider::Codex,
                method: SubscriptionInstallMethod::Winget,
            }
        );
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Progress {
                request_id: 41,
                progress: SubscriptionInstallProgress {
                    stage: SubscriptionInstallStage::InstallingWinget,
                    ..
                }
            }
        ));
        let terminal = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(terminal.request_id(), 41);
        assert!(terminal.is_terminal());
        assert_eq!(
            terminal,
            SubscriptionInstallWorkerEvent::Installed {
                request_id: 41,
                status: installed(SubscriptionAuthProvider::Codex),
            }
        );
        assert_eq!(
            *backend.requests.lock().unwrap(),
            [SubscriptionInstallRequest::winget(
                SubscriptionAuthProvider::Codex,
                true,
            )]
        );
    }

    #[test]
    fn fallback_remains_structured_and_requires_both_explicit_flags() {
        let provider = SubscriptionAuthProvider::ClaudeCode;
        let backend = Arc::new(ScriptedBackend {
            requests: Mutex::new(Vec::new()),
            result: Err(
                SubscriptionInstallError::VendorFallbackConfirmationRequired {
                    provider,
                    reason: SubscriptionInstallFallbackReason::ExplicitApprovalMissing,
                },
            ),
            emit_progress: false,
        });
        let (runtime, events) =
            SubscriptionInstallRuntime::start_with_backend(backend.clone()).unwrap();
        assert!(runtime.install_with_vendor_fallback(52, provider, true, false));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Ack { request_id: 52, .. }
        ));
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                request_id: 52,
                provider,
                reason: SubscriptionInstallFallbackReason::ExplicitApprovalMissing,
            }
        );
        assert_eq!(
            *backend.requests.lock().unwrap(),
            [SubscriptionInstallRequest::vendor_fallback(
                provider, true, false,
            )]
        );
    }

    #[test]
    fn cancellation_is_out_of_band_and_stale_ids_fail_closed() {
        let observed = Arc::new(AtomicBool::new(false));
        let backend = Arc::new(PendingBackend {
            cancellation_observed: Arc::clone(&observed),
        });
        let (runtime, events) = SubscriptionInstallRuntime::start_with_backend(backend).unwrap();
        assert!(runtime.install_with_winget(61, SubscriptionAuthProvider::Codex, true));
        assert!(!runtime.install_with_winget(62, SubscriptionAuthProvider::Codex, true));
        assert!(!runtime.cancel(62));
        assert!(runtime.cancel(61));

        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Ack { request_id: 61, .. }
        ));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Cancelled { request_id: 61, .. }
        ));
        assert!(observed.load(Ordering::Acquire));
    }

    #[test]
    fn dropping_runtime_cancels_and_joins_the_active_request() {
        let observed = Arc::new(AtomicBool::new(false));
        let backend = Arc::new(PendingBackend {
            cancellation_observed: Arc::clone(&observed),
        });
        let (runtime, events) = SubscriptionInstallRuntime::start_with_backend(backend).unwrap();
        assert!(runtime.install_with_vendor_fallback(
            71,
            SubscriptionAuthProvider::ClaudeCode,
            true,
            true,
        ));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Ack { request_id: 71, .. }
        ));

        drop(runtime);
        assert!(observed.load(Ordering::Acquire));
    }

    #[test]
    fn event_helpers_make_stale_filtering_and_terminal_detection_explicit() {
        let progress = SubscriptionInstallWorkerEvent::Progress {
            request_id: 88,
            progress: SubscriptionInstallProgress {
                provider: SubscriptionAuthProvider::Codex,
                method: SubscriptionInstallMethod::Winget,
                stage: SubscriptionInstallStage::Verifying,
            },
        };
        assert_eq!(progress.request_id(), 88);
        assert!(!progress.is_terminal());

        let failed = SubscriptionInstallWorkerEvent::Failed {
            request_id: 89,
            provider: SubscriptionAuthProvider::Codex,
            method: SubscriptionInstallMethod::Winget,
            message: "sanitized".to_string(),
        };
        assert_eq!(failed.request_id(), 89);
        assert!(failed.is_terminal());
    }
}
