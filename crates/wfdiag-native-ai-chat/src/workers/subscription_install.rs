//! Off-UI, confirmation-preserving subscription CLI installer.
//!
//! This adapter owns no installation policy. It carries the UI's two explicit
//! approvals into [`SubscriptionInstallController`], keeps one cancellation
//! token per accepted request, and projects the shared controller's typed
//! progress and terminal outcomes onto a standard channel drained by the host
//! component. Tearing the runtime down cancels its active request, which in
//! turn closes the shared installer's Windows Job Object.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::ProcessSubscriptionCliStatusSource;

use crate::workers::{
    ActiveRequestSlot, WorkerWake, build_worker_runtime, reap_worker, send_worker_event,
};
use crate::{ProcessSubscriptionModelCatalogSource, SubscriptionInstallController};

pub use crate::{
    SubscriptionAuthProvider, SubscriptionAuthState, SubscriptionInstallError,
    SubscriptionInstallFallbackReason, SubscriptionInstallMethod, SubscriptionInstallProgress,
    SubscriptionInstallRequest, SubscriptionInstallStage, SubscriptionInstallStatus,
};

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

/// Typed events drained on the host's UI thread. Every event carries the
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

struct WorkerState {
    backend: Arc<dyn InstallBackend>,
    events: mpsc::Sender<SubscriptionInstallWorkerEvent>,
    wake: WorkerWake,
    active: ActiveRequestSlot,
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
        send_worker_event(
            &self.events,
            &self.wake,
            SubscriptionInstallWorkerEvent::Ack {
                request_id,
                provider,
                method,
            },
        );
        let progress_events = self.events.clone();
        let progress_wake = Arc::clone(&self.wake);
        let progress: ProgressSink = Arc::new(move |progress| {
            send_worker_event(
                &progress_events,
                &progress_wake,
                SubscriptionInstallWorkerEvent::Progress {
                    request_id,
                    progress,
                },
            );
        });
        let result = self.backend.install(request, cancellation, progress).await;

        // A package manager can change the installation even when its final
        // status is failure. Force subsequent status/model refreshes to probe.
        let shared_provider = provider.into();
        ProcessSubscriptionCliStatusSource::new().invalidate(shared_provider);
        ProcessSubscriptionModelCatalogSource::new().invalidate(shared_provider);

        self.active.clear(request_id);
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
        send_worker_event(&self.events, &self.wake, terminal);
    }
}

/// UI-thread handle for explicit subscription CLI installs.
pub struct SubscriptionInstallRuntime {
    commands: Option<mpsc::Sender<InstallCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveRequestSlot,
}

impl SubscriptionInstallRuntime {
    /// Start a side-effect-free persistent worker. No probe or process occurs
    /// until one of the two explicit install methods is called.
    ///
    /// # Errors
    /// When the worker thread or its Tokio runtime cannot be created.
    pub fn start(
        wake: WorkerWake,
    ) -> std::io::Result<(Self, mpsc::Receiver<SubscriptionInstallWorkerEvent>)> {
        Self::start_with_backend(Arc::new(SharedInstallBackend::default()), wake)
    }

    fn start_with_backend(
        backend: Arc<dyn InstallBackend>,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, mpsc::Receiver<SubscriptionInstallWorkerEvent>)> {
        let (commands, command_rx) = mpsc::channel::<InstallCommand>();
        let (events, event_rx) = mpsc::channel::<SubscriptionInstallWorkerEvent>();
        let active = ActiveRequestSlot::new();
        let worker_active = active.clone();
        let runtime = build_worker_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-subscription-install".to_string())
            .spawn(move || {
                let state = WorkerState {
                    backend,
                    events,
                    wake,
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
    #[must_use]
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
    #[must_use]
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
        let Some(cancellation) = self.active.register(request_id, None) else {
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
            self.active.clear(request_id);
            false
        }
    }

    /// Cancel the matching request out-of-band. A stale request ID cannot
    /// cancel the current installer.
    #[must_use]
    pub fn cancel(&self, request_id: u64) -> bool {
        self.active.cancel(request_id)
    }

    /// Cancel any in-flight install, stop the worker, and wait up to `budget`.
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
        if let Some(cancellation) = self.active.take() {
            cancellation.cancel();
        }
        self.commands = None;
    }
}

impl Drop for SubscriptionInstallRuntime {
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
    use std::path::PathBuf;
    use std::sync::Mutex;
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
            SubscriptionInstallRuntime::start_with_backend(backend.clone(), no_wake()).unwrap();
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
            SubscriptionInstallRuntime::start_with_backend(backend.clone(), no_wake()).unwrap();
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
        let (runtime, events) =
            SubscriptionInstallRuntime::start_with_backend(backend, no_wake()).unwrap();
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
    fn stopping_the_runtime_cancels_and_joins_the_active_request() {
        let observed = Arc::new(AtomicBool::new(false));
        let backend = Arc::new(PendingBackend {
            cancellation_observed: Arc::clone(&observed),
        });
        let (mut runtime, events) =
            SubscriptionInstallRuntime::start_with_backend(backend, no_wake()).unwrap();
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

        // The bounded stop observes the cancelled install before returning;
        // Drop performs the same cancellation without waiting for the join.
        assert!(runtime.stop_and_join(Duration::from_secs(2)));
        assert!(observed.load(Ordering::Acquire));
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SubscriptionInstallWorkerEvent::Cancelled { request_id: 71, .. }
        ));
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
