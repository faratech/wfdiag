use crate::{
    ArchitectureSnapshot, SystemError, SystemInfo, get_architecture_snapshot, get_system_info,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

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

    /// Bounded, ordered teardown from a non-UI shutdown path.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::ShutdownTimedOut`] when the worker had not exited
    /// within `budget`. The worker is still reaped on a detached thread.
    pub fn shutdown(mut self, budget: Duration) -> Result<(), SystemError> {
        if self.stop_and_join(budget) {
            Ok(())
        } else {
            Err(SystemError::ShutdownTimedOut)
        }
    }

    /// Stop the worker and wait up to `budget` for it to exit.
    ///
    /// Returns `false` when the worker was still running when the budget
    /// expired. Either way the handle has already been handed to a detached
    /// reaper, so the thread is joined eventually and the caller never blocks
    /// past `budget`. A second call after the handle is gone is a no-op that
    /// reports success.
    pub fn stop_and_join(&mut self, budget: Duration) -> bool {
        let _ = self.commands.send(WorkerCommand::Stop);
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (done, finished) = mpsc::channel();
        reap_worker(worker, Some(done));
        finished.recv_timeout(budget).is_ok()
    }
}

impl Drop for SystemRuntime {
    fn drop(&mut self) {
        // Never join inline: this runtime is owned by the UI shell, and a
        // queued unit of work would otherwise freeze the window at close.
        // Hand the handle to a detached reaper so the thread is still joined
        // instead of leaked (#203).
        let _ = self.commands.send(WorkerCommand::Stop);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker, None);
        }
    }
}

/// Join a stopped worker on a detached thread so the caller never blocks.
/// `done` (when supplied) receives one message once the join completes.
fn reap_worker(worker: thread::JoinHandle<()>, done: Option<mpsc::Sender<()>>) {
    let spawned = thread::Builder::new()
        .name("wfdiag-system-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
            if let Some(done) = done {
                let _ = done.send(());
            }
        });
    if spawned.is_err() {
        // Thread creation failed: the worker still exits on its own after
        // `Stop`, so leaking the handle is the only non-blocking option left.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessorArchitecture;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

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

    /// Blocks inside collection until released, standing in for a slow WMI
    /// or registry read on the real host.
    struct StallingProvider {
        release: Mutex<Option<mpsc::Receiver<()>>>,
    }

    impl SystemProvider for StallingProvider {
        fn architecture(&self) -> Result<ArchitectureSnapshot, SystemError> {
            if let Some(release) = self.release.lock().unwrap().take() {
                let _ = release.recv();
            }
            FakeProvider.architecture()
        }

        fn system_info(&self) -> Result<SystemInfo, SystemError> {
            FakeProvider.system_info()
        }
    }

    fn stalled_runtime() -> (
        SystemRuntime,
        mpsc::Receiver<SystemCompleted>,
        mpsc::Sender<()>,
    ) {
        let (release, receiver) = mpsc::channel();
        let (runtime, replies) = SystemRuntime::start_with_provider(Arc::new(StallingProvider {
            release: Mutex::new(Some(receiver)),
        }))
        .unwrap();
        runtime
            .enqueue(SystemRequest {
                request_id: 1,
                kind: SystemRequestKind::Architecture,
            })
            .unwrap();
        (runtime, replies, release)
    }

    #[test]
    fn drop_returns_promptly_behind_a_stalled_collection() {
        let (runtime, replies, release) = stalled_runtime();
        let started = Instant::now();
        drop(runtime);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop joined the stalled worker inline: {:?}",
            started.elapsed()
        );
        // The worker still finishes its queued request and exits on its own.
        release.send(()).unwrap();
        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed.request_id, 1);
    }

    #[test]
    fn stop_and_join_is_bounded_and_idempotent() {
        let (mut runtime, _replies, release) = stalled_runtime();
        let started = Instant::now();
        assert!(!runtime.stop_and_join(Duration::from_millis(100)));
        assert!(started.elapsed() < Duration::from_millis(500));
        release.send(()).unwrap();
        // The handle now belongs to the detached reaper, so a second call is
        // a no-op that reports success.
        assert!(runtime.stop_and_join(Duration::from_millis(1)));
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
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }
}
