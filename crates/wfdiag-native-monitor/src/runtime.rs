use crate::{
    DiskInfo as NativeDiskInfo, MonitorEmission, MonitorEmitter, MonitorProfile,
    ProcessInfo as NativeProcessInfo, ProcessPage, ProcessQuery, SystemMonitor,
    SystemStats as NativeSystemStats,
};
use std::io;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
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
    ControlWake,
    ProcessWake,
    Shutdown,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ControlSnapshot {
    running: bool,
    refresh: bool,
}

/// Coalesces rapid lifecycle and refresh changes into one worker wake.
///
/// The desired lifecycle is latest-wins. Refresh is a single edge and is
/// discarded while paused, so a hidden page cannot keep system sampling alive.
struct CoalescedControl {
    desired_running: AtomicBool,
    refresh_requested: AtomicBool,
    wake_pending: AtomicBool,
}

impl CoalescedControl {
    fn new(running: bool) -> Self {
        Self {
            desired_running: AtomicBool::new(running),
            refresh_requested: AtomicBool::new(false),
            wake_pending: AtomicBool::new(false),
        }
    }

    /// Returns true when the caller must enqueue a worker wake.
    fn set_running(&self, running: bool) -> bool {
        let changed = self.desired_running.swap(running, Ordering::AcqRel) != running;
        if !running {
            self.refresh_requested.store(false, Ordering::Release);
        }
        changed && !self.wake_pending.swap(true, Ordering::AcqRel)
    }

    /// Returns true when the caller must enqueue a worker wake.
    fn request_refresh(&self) -> bool {
        if !self.desired_running.load(Ordering::Acquire) {
            return false;
        }
        let first_refresh = !self.refresh_requested.swap(true, Ordering::AcqRel);
        first_refresh && !self.wake_pending.swap(true, Ordering::AcqRel)
    }

    fn take_for_wake(&self) -> ControlSnapshot {
        // Clear first so a racing producer schedules another wake. The extra
        // wake can be redundant, but no newer lifecycle state is ever lost.
        self.wake_pending.store(false, Ordering::Release);
        let running = self.desired_running.load(Ordering::Acquire);
        let refresh = self.refresh_requested.swap(false, Ordering::AcqRel) && running;
        ControlSnapshot { running, refresh }
    }

    fn clear_wake(&self) {
        self.wake_pending.store(false, Ordering::Release);
    }
}

struct ProcessRequest {
    query: ProcessQuery,
    reply: oneshot::Sender<ProcessQueryOutcome>,
}

/// Outcome of a queued process query.
///
/// The queue keeps only the newest request; a replaced query is answered with
/// [`ProcessQueryOutcome::Superseded`] instead of silently closing its
/// channel, so callers can tell "your query was superseded" (routine rapid
/// requery) from "the monitor worker stopped" (a real failure).
#[derive(Debug)]
pub enum ProcessQueryOutcome {
    /// The requested page.
    Page(ProcessPage),
    /// A newer query replaced this one before it could run.
    Superseded,
}

/// One queued process enumeration at most. Replacing an obsolete request
/// answers it with [`ProcessQueryOutcome::Superseded`], immediately releasing
/// the cancelled Reactor task. An enumeration already executing may finish,
/// but it can be followed by only the newest requested query rather than an
/// unbounded stale backlog.
#[derive(Default)]
struct LatestProcessRequest {
    latest: Mutex<Option<ProcessRequest>>,
    wake_pending: AtomicBool,
}

impl LatestProcessRequest {
    fn submit(&self, request: ProcessRequest) -> bool {
        let previous = self
            .latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .replace(request);
        if let Some(previous) = previous {
            let _ = previous.reply.send(ProcessQueryOutcome::Superseded);
        }
        !self.wake_pending.swap(true, Ordering::AcqRel)
    }

    fn take_for_wake(&self) -> Option<ProcessRequest> {
        // Clear before taking: a concurrent submit then schedules a fresh wake
        // and either replaces this not-yet-taken request or fills the empty
        // slot for the next iteration. Both outcomes retain the newest query.
        self.wake_pending.store(false, Ordering::Release);
        self.latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }

    fn clear(&self) {
        self.wake_pending.store(false, Ordering::Release);
        self.latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
    }
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
    control: Arc<CoalescedControl>,
    process_requests: Arc<LatestProcessRequest>,
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
        Self::start_with_profile(MonitorProfile::Legacy {
            include_process_adapter_stats,
        })
    }

    /// Start native collection with an explicit telemetry demand profile.
    ///
    /// Native shells should use `MonitorProfile::SystemOnly`; that profile
    /// guarantees that one-second telemetry performs no process enumeration.
    /// `start` remains the legacy Tauri-compatible behavior.
    pub fn start_with_profile(profile: MonitorProfile) -> io::Result<(Self, UiEventReceiver)> {
        let capacity = NonZeroUsize::try_from(32usize)
            .map_err(|_| io::Error::other("native monitoring event capacity must be non-zero"))?;
        let (publisher, receiver) = ui_event_bus(capacity);
        let (commands, mut command_receiver) = mpsc::unbounded_channel();
        let control = Arc::new(CoalescedControl::new(true));
        let worker_control = Arc::clone(&control);
        let process_requests = Arc::new(LatestProcessRequest::default());
        let worker_process_requests = Arc::clone(&process_requests);
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
                    let monitor = Arc::new(SystemMonitor::with_emitter(emitter.clone()));
                    monitor.start_monitoring_with_profile(profile).await;
                    let mut running = true;

                    while let Some(command) = command_receiver.recv().await {
                        match command {
                            RuntimeCommand::ControlWake => {
                                let desired = worker_control.take_for_wake();
                                let resumed = desired.running && !running;
                                if desired.running != running {
                                    if desired.running {
                                        monitor.start_monitoring_with_profile(profile).await;
                                    } else {
                                        monitor.stop_monitoring().await;
                                    }
                                    running = desired.running;
                                }

                                // Starting seeds and immediately emits a sample;
                                // fold any queued refresh into that transition.
                                if desired.refresh && running && !resumed {
                                    let stats = monitor.get_current_stats().await;
                                    let _ = emitter.emit_system_stats(&stats);
                                }
                            }
                            RuntimeCommand::ProcessWake => {
                                if let Some(request) = worker_process_requests.take_for_wake() {
                                    // Enumerate on its own task so a
                                    // multi-second process snapshot cannot
                                    // delay ControlWake handling: pause() and
                                    // stop_monitoring() must take effect
                                    // promptly even while a query is in
                                    // flight. Concurrent enumerations still
                                    // serialize inside SystemMonitor's snapshot
                                    // refresh lock, and the UI rejects stale
                                    // pages by request id.
                                    let monitor = Arc::clone(&monitor);
                                    tokio::spawn(async move {
                                        let page = monitor.list_processes(request.query).await;
                                        let _ = request.reply.send(ProcessQueryOutcome::Page(page));
                                    });
                                }
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
                control,
                process_requests,
                worker: Some(worker),
            },
            receiver,
        ))
    }

    /// Stop sampling without blocking the UI thread.
    #[must_use]
    pub fn pause(&self) -> bool {
        self.send_control_wake(self.control.set_running(false))
    }

    /// Resume one-second native sampling.
    #[must_use]
    pub fn resume(&self) -> bool {
        self.send_control_wake(self.control.set_running(true))
    }

    /// Request an immediate native sample.
    #[must_use]
    pub fn refresh(&self) -> bool {
        self.send_control_wake(self.control.request_refresh())
    }

    fn send_control_wake(&self, required: bool) -> bool {
        if !required {
            return !self.commands.is_closed();
        }
        if self.commands.send(RuntimeCommand::ControlWake).is_ok() {
            true
        } else {
            self.control.clear_wake();
            false
        }
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
    ) -> io::Result<oneshot::Receiver<ProcessQueryOutcome>> {
        let (reply, receiver) = oneshot::channel();
        if self
            .process_requests
            .submit(ProcessRequest { query, reply })
            && self.commands.send(RuntimeCommand::ProcessWake).is_err()
        {
            self.process_requests.clear();
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "monitor worker stopped",
            ));
        }
        Ok(receiver)
    }
}

impl Drop for NativeMonitorRuntime {
    fn drop(&mut self) {
        self.control.set_running(false);
        self.process_requests.clear();
        let _ = self.commands.send(RuntimeCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProcessSortDirection, ProcessSortKey};

    fn query(search: &str) -> ProcessQuery {
        ProcessQuery {
            search: search.to_string(),
            sort_by: ProcessSortKey::CpuPercent,
            sort_direction: ProcessSortDirection::Desc,
            offset: 0,
            limit: 100,
        }
    }

    #[test]
    fn pending_process_requests_are_latest_wins_with_one_wake() {
        let slot = LatestProcessRequest::default();
        let (first_reply, mut first_receiver) = oneshot::channel();
        assert!(slot.submit(ProcessRequest {
            query: query("first"),
            reply: first_reply,
        }));

        let (latest_reply, _latest_receiver) = oneshot::channel();
        assert!(!slot.submit(ProcessRequest {
            query: query("latest"),
            reply: latest_reply,
        }));
        assert!(matches!(
            first_receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));

        let pending = slot.take_for_wake().expect("latest request remains queued");
        assert_eq!(pending.query.search, "latest");
        drop(pending);

        let (next_reply, _next_receiver) = oneshot::channel();
        assert!(slot.submit(ProcessRequest {
            query: query("next"),
            reply: next_reply,
        }));
    }

    #[test]
    fn lifecycle_commands_are_latest_wins_and_idempotent() {
        let control = CoalescedControl::new(true);

        assert!(!control.set_running(true), "duplicate resume is a no-op");
        assert!(control.set_running(false), "pause schedules one wake");
        assert!(
            !control.set_running(true),
            "resume replaces the queued pause without a second wake"
        );
        assert_eq!(
            control.take_for_wake(),
            ControlSnapshot {
                running: true,
                refresh: false,
            }
        );
        assert!(!control.set_running(true), "processed resume stays a no-op");
    }

    #[test]
    fn refreshes_coalesce_and_pause_discards_them() {
        let control = CoalescedControl::new(true);

        assert!(control.request_refresh());
        assert!(
            !control.request_refresh(),
            "only one refresh wake is queued"
        );
        assert_eq!(
            control.take_for_wake(),
            ControlSnapshot {
                running: true,
                refresh: true,
            }
        );

        assert!(control.request_refresh());
        assert!(!control.set_running(false), "the refresh wake is reused");
        assert!(
            !control.request_refresh(),
            "paused sampling rejects refresh"
        );
        assert_eq!(
            control.take_for_wake(),
            ControlSnapshot {
                running: false,
                refresh: false,
            }
        );
    }
}
