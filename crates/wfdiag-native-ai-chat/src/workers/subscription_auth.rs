//! Off-UI subscription account runtime for provider setup.
//!
//! Status, sign-in and sign-out run on a dedicated worker through the genuine
//! Codex/Claude CLIs. Mutations can begin only through explicit methods on this
//! handle; there is intentionally no install operation. Cancellation is
//! out-of-band so it can kill a login child without waiting behind it.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::ProcessSubscriptionCliStatusSource;
use wfdiag_native_settings::{AppSettings, SettingsService};

use crate::workers::{
    ActiveRequestSlot, WorkerWake, build_worker_runtime, reap_worker, send_worker_event,
};
use crate::{
    ProcessSubscriptionModelCatalogSource, SubscriptionAuthController, SubscriptionAuthError,
    SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionAuthStatus,
};

pub use crate::SubscriptionAuthState;

/// Typed account lifecycle events drained by the host component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionAuthWorkerEvent {
    Ack {
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
    },
    StatusLoaded {
        operation_id: u64,
        status: SubscriptionAuthStatus,
    },
    Completed {
        operation_id: u64,
        operation: SubscriptionAuthOperation,
        status: SubscriptionAuthStatus,
    },
    Failed {
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
        message: String,
    },
    Cancelled {
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
    },
}

impl SubscriptionAuthWorkerEvent {
    #[must_use]
    pub const fn operation_id(&self) -> u64 {
        match self {
            Self::Ack { operation_id, .. }
            | Self::StatusLoaded { operation_id, .. }
            | Self::Completed { operation_id, .. }
            | Self::Failed { operation_id, .. }
            | Self::Cancelled { operation_id, .. } => *operation_id,
        }
    }
}

enum AuthCommand {
    Run {
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
        draft_cli_path: Option<String>,
        cancellation: CancellationToken,
    },
}

struct WorkerState {
    settings: SettingsService,
    controller: SubscriptionAuthController,
    events: mpsc::Sender<SubscriptionAuthWorkerEvent>,
    wake: WorkerWake,
    active: ActiveRequestSlot,
}

impl WorkerState {
    async fn run(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
        draft_cli_path: Option<String>,
        cancellation: CancellationToken,
    ) {
        send_worker_event(
            &self.events,
            &self.wake,
            SubscriptionAuthWorkerEvent::Ack {
                operation_id,
                provider,
                operation,
            },
        );
        let settings = self.settings.load_nonsecret_settings().unwrap_or_default();
        let path = effective_cli_path(provider, draft_cli_path, &settings);
        let result = match operation {
            SubscriptionAuthOperation::Status => {
                self.controller
                    .status(provider, path.as_deref(), cancellation)
                    .await
            }
            SubscriptionAuthOperation::SignIn => {
                self.controller
                    .sign_in(provider, path.as_deref(), cancellation)
                    .await
            }
            SubscriptionAuthOperation::SignOut => {
                self.controller
                    .sign_out(provider, path.as_deref(), cancellation)
                    .await
            }
        };
        let shared_provider = provider.into();
        ProcessSubscriptionCliStatusSource::new().invalidate(shared_provider);
        if operation != SubscriptionAuthOperation::Status {
            ProcessSubscriptionModelCatalogSource::new().invalidate(shared_provider);
        }
        self.active.clear(operation_id);
        let event = match result {
            Ok(status) if operation == SubscriptionAuthOperation::Status => {
                SubscriptionAuthWorkerEvent::StatusLoaded {
                    operation_id,
                    status,
                }
            }
            Ok(status) => SubscriptionAuthWorkerEvent::Completed {
                operation_id,
                operation,
                status,
            },
            Err(SubscriptionAuthError::Cancelled { .. }) => {
                SubscriptionAuthWorkerEvent::Cancelled {
                    operation_id,
                    provider,
                    operation,
                }
            }
            Err(error) => SubscriptionAuthWorkerEvent::Failed {
                operation_id,
                provider,
                operation,
                message: error.to_string(),
            },
        };
        send_worker_event(&self.events, &self.wake, event);
    }
}

fn effective_cli_path(
    provider: SubscriptionAuthProvider,
    draft_cli_path: Option<String>,
    settings: &AppSettings,
) -> Option<String> {
    draft_cli_path
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .or_else(|| match provider {
            SubscriptionAuthProvider::Codex => settings.codex_cli_path.clone(),
            SubscriptionAuthProvider::ClaudeCode => settings.claude_cli_path.clone(),
        })
}

/// UI-thread handle for explicit subscription account actions.
pub struct SubscriptionAuthRuntime {
    commands: Option<mpsc::Sender<AuthCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveRequestSlot,
}

impl SubscriptionAuthRuntime {
    /// Start a side-effect-free persistent worker.
    ///
    /// # Errors
    /// When the worker thread or its Tokio runtime cannot be created.
    pub fn start(
        settings: SettingsService,
        wake: WorkerWake,
    ) -> std::io::Result<(Self, mpsc::Receiver<SubscriptionAuthWorkerEvent>)> {
        let (commands, command_rx) = mpsc::channel::<AuthCommand>();
        let (events, event_rx) = mpsc::channel::<SubscriptionAuthWorkerEvent>();
        let active = ActiveRequestSlot::new();
        let worker_active = active.clone();
        let runtime = build_worker_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-subscription-auth".to_string())
            .spawn(move || {
                let state = WorkerState {
                    settings,
                    controller: SubscriptionAuthController::new(),
                    events,
                    wake,
                    active: worker_active,
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        AuthCommand::Run {
                            operation_id,
                            provider,
                            operation,
                            draft_cli_path,
                            cancellation,
                        } => runtime.block_on(state.run(
                            operation_id,
                            provider,
                            operation,
                            draft_cli_path,
                            cancellation,
                        )),
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

    /// Queue a read-only status refresh.
    #[must_use]
    pub fn request_status(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<String>,
    ) -> bool {
        self.send(
            operation_id,
            provider,
            SubscriptionAuthOperation::Status,
            draft_cli_path,
        )
    }

    /// Explicitly start the vendor's own sign-in flow.
    #[must_use]
    pub fn start_sign_in(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<String>,
    ) -> bool {
        self.send(
            operation_id,
            provider,
            SubscriptionAuthOperation::SignIn,
            draft_cli_path,
        )
    }

    /// Explicitly ask the vendor CLI to sign out.
    #[must_use]
    pub fn start_sign_out(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<String>,
    ) -> bool {
        self.send(
            operation_id,
            provider,
            SubscriptionAuthOperation::SignOut,
            draft_cli_path,
        )
    }

    fn send(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
        draft_cli_path: Option<String>,
    ) -> bool {
        let Some(commands) = self.commands.as_ref() else {
            return false;
        };
        let Some(cancellation) = self.active.register(operation_id, None) else {
            return false;
        };
        if commands
            .send(AuthCommand::Run {
                operation_id,
                provider,
                operation,
                draft_cli_path,
                cancellation: cancellation.clone(),
            })
            .is_ok()
        {
            true
        } else {
            cancellation.cancel();
            self.active.clear(operation_id);
            false
        }
    }

    /// Cancel the matching operation directly rather than queueing behind it.
    #[must_use]
    pub fn cancel(&self, operation_id: u64) -> bool {
        self.active.cancel(operation_id)
    }

    /// Cancel any in-flight operation, stop the worker, and wait up to
    /// `budget`.
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

impl Drop for SubscriptionAuthRuntime {
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

    #[test]
    fn draft_path_wins_and_blank_draft_uses_stored_path() {
        let settings = AppSettings {
            codex_cli_path: Some("C:\\Stored\\codex.exe".to_string()),
            claude_cli_path: Some("C:\\Stored\\claude.exe".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(
            effective_cli_path(
                SubscriptionAuthProvider::Codex,
                Some(" C:\\Draft\\codex.exe ".to_string()),
                &settings,
            )
            .as_deref(),
            Some("C:\\Draft\\codex.exe")
        );
        assert_eq!(
            effective_cli_path(
                SubscriptionAuthProvider::ClaudeCode,
                Some("  ".to_string()),
                &settings,
            )
            .as_deref(),
            Some("C:\\Stored\\claude.exe")
        );
    }

    #[test]
    fn event_identity_is_stable_for_stale_result_rejection() {
        let event = SubscriptionAuthWorkerEvent::Cancelled {
            operation_id: 77,
            provider: SubscriptionAuthProvider::Codex,
            operation: SubscriptionAuthOperation::SignIn,
        };
        assert_eq!(event.operation_id(), 77);
    }
}
