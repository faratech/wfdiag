//! Native remediation action runtime for the Reactor shell.
//!
//! Thin worker over the shared [`wfdiag_native_remediation`] engine: one std
//! thread owning a current-thread Tokio runtime executes authorized catalog
//! entries off the WinUI thread. Tier gating is enforced twice — the UI only
//! dispatches Repair after an explicit dialog confirmation, and the engine's
//! own gate would reject an unauthorized Repair regardless.

#![deny(unsafe_code)]

use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;

use tokio_util::sync::CancellationToken;
use wfdiag_native_remediation::remediation::{
    FixResult, RealRunner, RemediationTier, execute_authorized, find,
};

/// Worker commands.
pub enum ActionCommand {
    Execute {
        request_id: u64,
        remediation_id: String,
        confirmed: bool,
    },
}

/// Typed worker events drained by the component.
#[derive(Clone)]
pub enum ActionWorkerEvent {
    Done {
        request_id: u64,
        result: FixResult,
    },
    Failed {
        request_id: u64,
        message: String,
    },
    NeedsConfirmation {
        request_id: u64,
        remediation_id: String,
    },
}

impl ActionWorkerEvent {
    /// The originating execute's identity, for stale-event rejection.
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::NeedsConfirmation { request_id, .. } => *request_id,
        }
    }
}

/// Cloneable handle the component holds on the UI thread.
pub struct NativeActionRuntime {
    /// Option so Drop can release the sender BEFORE joining the worker.
    commands: Option<std_mpsc::Sender<ActionCommand>>,
    worker: Option<JoinHandle<()>>,
}

impl NativeActionRuntime {
    /// Start the worker.
    ///
    /// # Errors
    /// When the worker thread cannot be spawned.
    pub fn start() -> std::io::Result<(Self, std_mpsc::Receiver<ActionWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ActionCommand>();
        let (events, event_rx) = std_mpsc::channel::<ActionWorkerEvent>();
        let worker = std::thread::Builder::new()
            .name("wfdiag-reactor-actions".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    let Ok(runtime) = runtime else {
                        continue;
                    };
                    let ActionCommand::Execute {
                        request_id,
                        remediation_id,
                        confirmed,
                    } = command;
                    let spec = find(&remediation_id);
                    let Some(spec) = spec else {
                        let _ = events.send(ActionWorkerEvent::Failed {
                            request_id,
                            message: format!("Unknown remediation '{remediation_id}'"),
                        });
                        continue;
                    };
                    if spec.tier == RemediationTier::Repair && !confirmed {
                        // The UI confirmation dialog is the only path here;
                        // treat a direct dispatch as a gate hit, not an error.
                        let _ = events.send(ActionWorkerEvent::NeedsConfirmation {
                            request_id,
                            remediation_id,
                        });
                        continue;
                    }
                    let outcome = runtime.block_on(execute_authorized(
                        &remediation_id,
                        &RealRunner,
                        &CancellationToken::new(),
                    ));
                    match outcome {
                        Ok(result) => {
                            let _ = events.send(ActionWorkerEvent::Done { request_id, result });
                        }
                        Err(message) => {
                            let _ = events.send(ActionWorkerEvent::Failed { request_id, message });
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

    pub fn execute(&self, request_id: u64, remediation_id: String, confirmed: bool) {
        if let Some(commands) = self.commands.as_ref() {
            let _ = commands.send(ActionCommand::Execute {
                request_id,
                remediation_id,
                confirmed,
            });
        }
    }
}

impl Drop for NativeActionRuntime {
    fn drop(&mut self) {
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
