use crate::{
    ArchitectureSnapshot, SystemError, SystemInfo, get_architecture_snapshot, get_system_info,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;

/// Read-only query queued by a native shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemRequestKind {
    Architecture,
    SystemInfo,
}

/// Owned request suitable for enqueueing from a `WinUI` dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemRequest {
    pub request_id: u64,
    pub kind: SystemRequestKind,
}

/// Typed successful payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SystemPayload {
    Architecture(ArchitectureSnapshot),
    SystemInfo(SystemInfo),
}

/// Terminal response for one queued request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCompleted {
    pub request_id: u64,
    pub result: Result<SystemPayload, SystemError>,
}

/// Injectable collection boundary used by both the worker and deterministic
/// tests. Implementations must remain read-only.
pub trait SystemProvider: Send + Sync + 'static {
    /// Collect the architecture projection.
    ///
    /// # Errors
    ///
    /// Returns a provider-specific collection error.
    fn architecture(&self) -> Result<ArchitectureSnapshot, SystemError>;

    /// Collect the host system-information projection.
    ///
    /// # Errors
    ///
    /// Returns a provider-specific collection error.
    fn system_info(&self) -> Result<SystemInfo, SystemError>;
}

/// Shipping Windows/portable host collector.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeSystemProvider;

impl SystemProvider for NativeSystemProvider {
    fn architecture(&self) -> Result<ArchitectureSnapshot, SystemError> {
        get_architecture_snapshot()
    }

    fn system_info(&self) -> Result<SystemInfo, SystemError> {
        get_system_info()
    }
}

enum WorkerCommand {
    Query(SystemRequest),
    Stop,
}

/// Dedicated read-only collector with a nonblocking, unbounded request queue.
pub struct SystemRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    worker: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for SystemRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemRuntime")
            .finish_non_exhaustive()
    }
}

impl SystemRuntime {
    /// Start the shipping native collector.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::Spawn`] when the worker thread cannot start.
    pub fn start() -> Result<(Self, mpsc::Receiver<SystemCompleted>), SystemError> {
        Self::start_with_provider(Arc::new(NativeSystemProvider))
    }

    /// Start with an injected read-only provider.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::Spawn`] when the worker thread cannot start.
    pub fn start_with_provider(
        provider: Arc<dyn SystemProvider>,
    ) -> Result<(Self, mpsc::Receiver<SystemCompleted>), SystemError> {
        let (command_tx, command_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("wfdiag-system".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        WorkerCommand::Query(request) => {
                            let result = match request.kind {
                                SystemRequestKind::Architecture => {
                                    provider.architecture().map(SystemPayload::Architecture)
                                }
                                SystemRequestKind::SystemInfo => {
                                    provider.system_info().map(SystemPayload::SystemInfo)
                                }
                            };
                            if reply_tx
                                .send(SystemCompleted {
                                    request_id: request.request_id,
                                    result,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        WorkerCommand::Stop => break,
                    }
                }
            })
            .map_err(|error| SystemError::Spawn(error.to_string()))?;

        Ok((
            Self {
                commands: command_tx,
                worker: Some(worker),
            },
            reply_rx,
        ))
    }

    /// Enqueue a request without performing collection on the caller thread.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::Disconnected`] after shutdown.
    pub fn enqueue(&self, request: SystemRequest) -> Result<(), SystemError> {
        self.commands
            .send(WorkerCommand::Query(request))
            .map_err(|_| SystemError::Disconnected)
    }

    /// Stop and join from a non-UI shutdown path.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::WorkerPanicked`] if the worker panicked.
    pub fn shutdown(mut self) -> Result<(), SystemError> {
        let _ = self.commands.send(WorkerCommand::Stop);
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            return Err(SystemError::WorkerPanicked);
        }
        Ok(())
    }
}

impl Drop for SystemRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessorArchitecture;
    use std::time::Duration;

    struct FakeProvider;

    impl SystemProvider for FakeProvider {
        fn architecture(&self) -> Result<ArchitectureSnapshot, SystemError> {
            Ok(ArchitectureSnapshot {
                process_architecture: ProcessorArchitecture::Amd64.to_u16(),
                process_architecture_name: "x64".to_string(),
                native_architecture: ProcessorArchitecture::Arm64.to_u16(),
                native_architecture_name: "ARM64".to_string(),
                is_emulated: true,
                page_size: 4096,
                processor_count: 12,
                emulation_status: "x64 app running on ARM64 hardware".to_string(),
            })
        }

        fn system_info(&self) -> Result<SystemInfo, SystemError> {
            Ok(SystemInfo {
                computer_name: "FAKE-PC".to_string(),
                os_version: "Windows 11 Pro (25H2)".to_string(),
                is_admin: false,
            })
        }
    }

    #[test]
    fn worker_preserves_ids_order_and_typed_contracts() {
        let (runtime, replies) =
            SystemRuntime::start_with_provider(Arc::new(FakeProvider)).unwrap();
        runtime
            .enqueue(SystemRequest {
                request_id: 7,
                kind: SystemRequestKind::Architecture,
            })
            .unwrap();
        runtime
            .enqueue(SystemRequest {
                request_id: 8,
                kind: SystemRequestKind::SystemInfo,
            })
            .unwrap();

        let architecture = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(architecture.request_id, 7);
        assert!(matches!(
            architecture.result,
            Ok(SystemPayload::Architecture(ArchitectureSnapshot {
                native_architecture: 12,
                is_emulated: true,
                ..
            }))
        ));

        let system = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(system.request_id, 8);
        assert_eq!(
            system.result,
            Ok(SystemPayload::SystemInfo(SystemInfo {
                computer_name: "FAKE-PC".to_string(),
                os_version: "Windows 11 Pro (25H2)".to_string(),
                is_admin: false,
            }))
        );
        runtime.shutdown().unwrap();
    }
}
