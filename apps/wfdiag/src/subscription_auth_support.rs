//! Off-UI subscription account runtime for Reactor provider setup.
//!
//! Status, sign-in and sign-out run on a dedicated worker through the genuine
//! Codex/Claude CLIs. Mutations can begin only through explicit methods on
//! this handle; there is intentionally no install operation. Cancellation is
//! out-of-band so it can kill a login child without waiting behind it.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{
    ProcessSubscriptionModelCatalogSource, SubscriptionAuthController, SubscriptionAuthError,
    SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionAuthStatus,
};
use wfdiag_native_ai_provider::ProcessSubscriptionCliStatusSource;
use wfdiag_native_settings::{AppSettings, SettingsService};

use crate::ui_wake_support::NotifySenderExt;

pub use wfdiag_native_ai_chat::SubscriptionAuthState;

/// Typed account lifecycle events drained by the Reactor component.
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

#[derive(Clone)]
struct ActiveOperation {
    operation_id: u64,
    cancellation: CancellationToken,
}

type ActiveSlot = Arc<Mutex<Option<ActiveOperation>>>;

fn active_slot() -> ActiveSlot {
    Arc::new(Mutex::new(None))
}

fn register_active(active: &ActiveSlot, operation_id: u64) -> Option<CancellationToken> {
    let mut slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_some() {
        return None;
    }
    let cancellation = CancellationToken::new();
    *slot = Some(ActiveOperation {
        operation_id,
        cancellation: cancellation.clone(),
    });
    Some(cancellation)
}

fn clear_active(active: &ActiveSlot, operation_id: u64) {
    let mut slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|operation| operation.operation_id == operation_id)
    {
        *slot = None;
    }
}

fn cancel_active(active: &ActiveSlot, operation_id: u64) -> bool {
    let slot = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(cancellation) = slot
        .as_ref()
        .filter(|operation| operation.operation_id == operation_id)
        .map(|operation| operation.cancellation.clone())
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
    let cancellation = slot
        .as_ref()
        .map(|operation| operation.cancellation.clone());
    drop(slot);
    if let Some(cancellation) = cancellation {
        cancellation.cancel();
    }
}

struct WorkerState {
    settings: SettingsService,
    controller: SubscriptionAuthController,
    events: std_mpsc::Sender<SubscriptionAuthWorkerEvent>,
    active: ActiveSlot,
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
        let _ = self.events.send_and_wake(SubscriptionAuthWorkerEvent::Ack {
            operation_id,
            provider,
            operation,
        });
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
        clear_active(&self.active, operation_id);
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
        let _ = self.events.send_and_wake(event);
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

fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

/// UI-thread handle for explicit subscription account actions.
pub struct SubscriptionAuthRuntime {
    commands: Option<std_mpsc::Sender<AuthCommand>>,
    worker: Option<JoinHandle<()>>,
    active: ActiveSlot,
}

impl SubscriptionAuthRuntime {
    /// Start a side-effect-free persistent worker.
    pub fn start(
        settings: SettingsService,
    ) -> std::io::Result<(Self, std_mpsc::Receiver<SubscriptionAuthWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<AuthCommand>();
        let (events, event_rx) = std_mpsc::channel::<SubscriptionAuthWorkerEvent>();
        let active = active_slot();
        let worker_active = Arc::clone(&active);
        let runtime = build_runtime()?;
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-subscription-auth".to_string())
            .spawn(move || {
                let state = WorkerState {
                    settings,
                    controller: SubscriptionAuthController::new(),
                    events,
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
        let Some(cancellation) = register_active(&self.active, operation_id) else {
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
            clear_active(&self.active, operation_id);
            false
        }
    }

    /// Cancel the matching operation directly rather than queueing behind it.
    #[must_use]
    pub fn cancel(&self, operation_id: u64) -> bool {
        cancel_active(&self.active, operation_id)
    }
}

impl Drop for SubscriptionAuthRuntime {
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
