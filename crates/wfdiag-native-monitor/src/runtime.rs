use crate::{
    DiskInfo as NativeDiskInfo, MonitorEmission, MonitorEmitter, ProcessInfo as NativeProcessInfo,
    ProcessPage, ProcessQuery, SystemMonitor, SystemStats as NativeSystemStats,
};
use std::io;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use wfdiag_ui_core::{
    DiskStats, ProcessStats, SystemStats, TryPublishError, UiEvent, UiEventPublisher,
    UiEventReceiver, ui_event_bus,
};

/// Projects native samples into the coalescing UI event bus.
pub struct UiBusMonitorEmitter {
    publisher: UiEventPublisher,
}

impl UiBusMonitorEmitter {
    #[must_use]
    pub fn new(publisher: UiEventPublisher) -> Self {
        Self { publisher }
    }
}

impl MonitorEmitter for UiBusMonitorEmitter {
    fn accepts_system_stats(&self) -> bool {
        !self.publisher.is_closed()
    }

    fn emit_system_stats(&self, stats: &NativeSystemStats) -> MonitorEmission {
        if self.publisher.is_closed() {
            return MonitorEmission::Stop;
        }

        // Live samples have a dedicated latest-value slot, so capacity cannot
        // reject them. A close racing this call asks the collector to stop.
        match self
            .publisher
            .try_publish(UiEvent::SystemStats(stats.into()))
        {
            Ok(_) | Err(TryPublishError::Full(_)) => MonitorEmission::Continue,
            Err(TryPublishError::Closed(_)) => MonitorEmission::Stop,
        }
    }
}

impl From<&NativeDiskInfo> for DiskStats {
    fn from(value: &NativeDiskInfo) -> Self {
        Self {
            name: value.name.clone(),
            mount_point: value.mount_point.clone(),
            total_gb: value.total_gb,
            used_gb: value.used_gb,
            available_gb: value.available_gb,
            utilization: value.utilization,
            file_system: value.file_system.clone(),
            disk_type: value.disk_type.clone(),
        }
    }
}

impl From<&NativeProcessInfo> for ProcessStats {
    fn from(value: &NativeProcessInfo) -> Self {
        Self {
            pid: value.pid,
            name: value.name.clone(),
            cpu_percent: value.cpu_percent,
            memory_percent: value.memory_percent,
            memory_mb: value.memory_mb,
            gpu_percent: value.gpu_percent,
            npu_percent: value.npu_percent,
            status: value.status.clone(),
        }
    }
}

impl From<&NativeSystemStats> for SystemStats {
    fn from(value: &NativeSystemStats) -> Self {
        Self {
            cpu_utilization: value.cpu_utilization,
            per_cpu_utilization: value.per_cpu_utilization.clone(),
            cpu_frequency: value.cpu_frequency,
            memory_total_gb: value.memory_total_gb,
            memory_used_gb: value.memory_used_gb,
            memory_available_gb: value.memory_available_gb,
            memory_utilization: value.memory_utilization,
            swap_total_gb: value.swap_total_gb,
            swap_used_gb: value.swap_used_gb,
            swap_utilization: value.swap_utilization,
            storage_used_percent: value.storage_used_percent,
            disk_utilization: value.disk_utilization,
            disk_read_bytes: value.disk_read_bytes,
            disk_write_bytes: value.disk_write_bytes,
            disks: value.disks.iter().map(Into::into).collect(),
            network_upload_kb: value.network_upload_kb,
            network_download_kb: value.network_download_kb,
            gpu_available: value.gpu_available,
            gpu_name: value.gpu_name.clone(),
            gpu_utilization: value.gpu_utilization,
            gpu_memory_used_mb: value.gpu_memory_used_mb,
            gpu_memory_total_mb: value.gpu_memory_total_mb,
            npu_available: value.npu_available,
            npu_name: value.npu_name.clone(),
            npu_utilization: value.npu_utilization,
            npu_memory_used_mb: value.npu_memory_used_mb,
            npu_memory_total_mb: value.npu_memory_total_mb,
            top_processes: value.top_processes.iter().map(Into::into).collect(),
            timestamp: value.timestamp,
        }
    }
}

enum RuntimeCommand {
    Pause,
    Resume,
    Refresh,
    ListProcesses {
        query: ProcessQuery,
        reply: oneshot::Sender<ProcessPage>,
    },
    Shutdown,
}

fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-monitor-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

/// Owns the Tokio worker used by the native collector.
///
/// Control methods are nonblocking and can be called directly from the `WinUI`
/// thread. Dropping the runtime requests shutdown; a detached reaper joins the
/// worker so slow Windows providers cannot stall the UI thread during exit.
pub struct NativeMonitorRuntime {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
    worker: Option<JoinHandle<()>>,
}

impl NativeMonitorRuntime {
    /// Start native collection and return the UI-thread receiver.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the telemetry worker or its Tokio runtime cannot
    /// be started, or if initialization does not finish within five seconds.
    pub fn start(include_process_adapter_stats: bool) -> io::Result<(Self, UiEventReceiver)> {
        let capacity = NonZeroUsize::try_from(32usize)
            .map_err(|_| io::Error::other("native monitoring event capacity must be non-zero"))?;
        let (publisher, receiver) = ui_event_bus(capacity);
        let (commands, mut command_receiver) = mpsc::unbounded_channel();
        let (startup_sender, startup_receiver) = std_mpsc::sync_channel(1);

        let worker = std::thread::Builder::new()
            .name("wfdiag-native-monitor".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_time()
                    .thread_name("wfdiag-monitor-async")
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_sender.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = startup_sender.send(Ok::<(), String>(()));

                runtime.block_on(async move {
                    let emitter = Arc::new(UiBusMonitorEmitter::new(publisher));
                    let monitor = SystemMonitor::with_emitter(emitter.clone());
                    monitor
                        .start_monitoring(include_process_adapter_stats)
                        .await;

                    while let Some(command) = command_receiver.recv().await {
                        match command {
                            RuntimeCommand::Pause => monitor.stop_monitoring().await,
                            RuntimeCommand::Resume => {
                                monitor
                                    .start_monitoring(include_process_adapter_stats)
                                    .await;
                            }
                            RuntimeCommand::Refresh => {
                                let stats = monitor.get_current_stats().await;
                                let _ = emitter.emit_system_stats(&stats);
                            }
                            RuntimeCommand::ListProcesses { query, reply } => {
                                let page = monitor.list_processes(query).await;
                                let _ = reply.send(page);
                            }
                            RuntimeCommand::Shutdown => break,
                        }
                    }

                    monitor.stop_monitoring().await;
                });
            })?;

        match startup_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                let _ = worker.join();
                return Err(io::Error::other(message));
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                let _ = commands.send(RuntimeCommand::Shutdown);
                reap_worker(worker);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native monitoring runtime did not initialize within five seconds",
                ));
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return Err(io::Error::other(
                    "native monitoring worker exited before initialization",
                ));
            }
        }

        Ok((
            Self {
                commands,
                worker: Some(worker),
            },
            receiver,
        ))
    }

    /// Stop sampling without blocking the UI thread.
    #[must_use]
    pub fn pause(&self) -> bool {
        self.commands.send(RuntimeCommand::Pause).is_ok()
    }

    /// Resume one-second native sampling.
    #[must_use]
    pub fn resume(&self) -> bool {
        self.commands.send(RuntimeCommand::Resume).is_ok()
    }

    /// Request an immediate native sample.
    #[must_use]
    pub fn refresh(&self) -> bool {
        self.commands.send(RuntimeCommand::Refresh).is_ok()
    }

    /// Queue an on-demand full process query on the monitor worker.
    ///
    /// The returned receiver is awaitable from a Reactor `ComponentTask` and
    /// never blocks the WinUI thread. Process enumeration stays off the
    /// one-second telemetry event path and reuses the collector's CPU history.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the worker has already stopped.
    pub fn request_processes(
        &self,
        query: ProcessQuery,
    ) -> io::Result<oneshot::Receiver<ProcessPage>> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(RuntimeCommand::ListProcesses { query, reply })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "monitor worker stopped"))?;
        Ok(receiver)
    }
}

impl Drop for NativeMonitorRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(RuntimeCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker);
        }
    }
}
