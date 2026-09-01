use crate::{
    ComparisonResult, ComparisonSummary, DiagnosticTask, ScanRecord, ScanStorage, ScanSummary,
    TaskDiffDetail, TaskTrend,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub type HistoryReply<T> = oneshot::Receiver<Result<T, String>>;

/// Settings and task metadata providers for one history worker.
///
/// Both callbacks are evaluated lazily by `ScanStorage`: retention changes
/// take effect on the next save, while diagnostic labels used for comparisons
/// always reflect the current executor catalog.
#[derive(Clone)]
pub struct HistoryRuntimeConfig {
    pub storage_dir: PathBuf,
    retention_provider: Arc<dyn Fn() -> (bool, u32) + Send + Sync>,
    task_catalog_provider: Arc<dyn Fn() -> Vec<DiagnosticTask> + Send + Sync>,
}

impl std::fmt::Debug for HistoryRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryRuntimeConfig")
            .field("storage_dir", &self.storage_dir)
            .finish_non_exhaustive()
    }
}

impl HistoryRuntimeConfig {
    #[must_use]
    pub fn new<R, C>(storage_dir: PathBuf, retention_provider: R, task_catalog_provider: C) -> Self
    where
        R: Fn() -> (bool, u32) + Send + Sync + 'static,
        C: Fn() -> Vec<DiagnosticTask> + Send + Sync + 'static,
    {
        Self {
            storage_dir,
            retention_provider: Arc::new(retention_provider),
            task_catalog_provider: Arc::new(task_catalog_provider),
        }
    }

    /// Use the exact shipping history path and default 30-scan retention.
    /// A native settings service should prefer [`Self::new`] and inject its
    /// live retention callback.
    ///
    /// # Errors
    ///
    /// Returns the same structured string error as `ScanStorage` when
    /// `APPDATA` is unavailable.
    pub fn shipping_defaults() -> Result<Self, String> {
        Ok(Self::new(
            ScanStorage::default_storage_directory()?,
            || (true, 30),
            Vec::new,
        ))
    }
}

enum HistoryCommand {
    Save {
        scan: ScanRecord,
        reply: oneshot::Sender<Result<(), String>>,
    },
    List {
        reply: oneshot::Sender<Result<Vec<ScanSummary>, String>>,
    },
    Load {
        scan_id: String,
        reply: oneshot::Sender<Result<ScanRecord, String>>,
    },
    Compare {
        current_id: String,
        previous_id: String,
        reply: oneshot::Sender<Result<ComparisonResult, String>>,
    },
    CompareCurrentToLatest {
        // Arc: the record carries every result body and must not be copied
        // into the command channel just to be handed to storage.
        current: std::sync::Arc<ScanRecord>,
        reply: oneshot::Sender<Result<Option<ComparisonResult>, String>>,
    },
    CompareSummary {
        current_id: String,
        previous_id: String,
        reply: oneshot::Sender<Result<ComparisonSummary, String>>,
    },
    TaskDiff {
        current_id: String,
        previous_id: String,
        task_id: String,
        reply: oneshot::Sender<Result<TaskDiffDetail, String>>,
    },
    UpdateTags {
        scan_id: String,
        tags: Vec<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    UpdateLabel {
        scan_id: String,
        label: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Trends {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<TaskTrend>, String>>,
    },
    Clear {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRuntimeError {
    WorkerStopped,
}

impl std::fmt::Display for HistoryRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkerStopped => formatter.write_str("native history worker stopped"),
        }
    }
}

impl std::error::Error for HistoryRuntimeError {}

fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-history-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

/// Dedicated scan-history worker for native UI shells.
///
/// Every request method only allocates a Tokio oneshot and enqueues on an
/// unbounded control channel. DPAPI, JSON and filesystem operations never run
/// on the caller (normally `WinUI`) thread. Receivers may be awaited by any
/// application worker and their results marshalled through `DispatcherQueue`.
pub struct NativeHistoryRuntime {
    commands: mpsc::UnboundedSender<HistoryCommand>,
    worker: Option<JoinHandle<()>>,
}

impl NativeHistoryRuntime {
    /// Start the dedicated worker and synchronously verify storage can open.
    ///
    /// # Errors
    ///
    /// Returns if the thread/runtime cannot be created, initialization fails,
    /// or the worker does not acknowledge startup within five seconds.
    #[allow(clippy::too_many_lines)]
    pub fn start(config: HistoryRuntimeConfig) -> io::Result<Self> {
        let (commands, mut command_receiver) = mpsc::unbounded_channel();
        let (startup_sender, startup_receiver) = std_mpsc::sync_channel(1);

        let worker = std::thread::Builder::new()
            .name("wfdiag-native-history".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_sender.send(Err(error.to_string()));
                        return;
                    }
                };

                let retention = Arc::clone(&config.retention_provider);
                let catalog = Arc::clone(&config.task_catalog_provider);
                let storage = match ScanStorage::new_in(
                    config.storage_dir,
                    move || retention(),
                    move || catalog(),
                ) {
                    Ok(storage) => storage,
                    Err(error) => {
                        let _ = startup_sender.send(Err(error));
                        return;
                    }
                };
                let _ = startup_sender.send(Ok::<(), String>(()));

                runtime.block_on(async move {
                    while let Some(command) = command_receiver.recv().await {
                        match command {
                            HistoryCommand::Save { scan, reply } => {
                                let _ = reply.send(storage.save_scan(&scan));
                            }
                            HistoryCommand::List { reply } => {
                                let _ = reply.send(storage.list_scans());
                            }
                            HistoryCommand::Load { scan_id, reply } => {
                                let _ = reply.send(storage.load_scan(&scan_id));
                            }
                            HistoryCommand::Compare {
                                current_id,
                                previous_id,
                                reply,
                            } => {
                                let _ =
                                    reply.send(storage.compare_scans(&current_id, &previous_id));
                            }
                            HistoryCommand::CompareCurrentToLatest { current, reply } => {
                                let _ =
                                    reply.send(storage.compare_current_to_latest_stored(current));
                            }
                            HistoryCommand::CompareSummary {
                                current_id,
                                previous_id,
                                reply,
                            } => {
                                let _ = reply
                                    .send(storage.compare_scans_summary(&current_id, &previous_id));
                            }
                            HistoryCommand::TaskDiff {
                                current_id,
                                previous_id,
                                task_id,
                                reply,
                            } => {
                                let _ = reply.send(storage.scan_task_diff(
                                    &current_id,
                                    &previous_id,
                                    &task_id,
                                ));
                            }
                            HistoryCommand::UpdateTags {
                                scan_id,
                                tags,
                                reply,
                            } => {
                                let _ = reply.send(storage.update_tags(&scan_id, tags));
                            }
                            HistoryCommand::UpdateLabel {
                                scan_id,
                                label,
                                reply,
                            } => {
                                let _ = reply.send(storage.update_label(&scan_id, label));
                            }
                            HistoryCommand::Trends { limit, reply } => {
                                let _ = reply.send(storage.task_failure_trends(limit));
                            }
                            HistoryCommand::Clear { reply } => {
                                let _ = reply.send(storage.clear_history());
                            }
                            HistoryCommand::Shutdown => break,
                        }
                    }
                });
            })?;

        match startup_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                commands,
                worker: Some(worker),
            }),
            Ok(Err(message)) => {
                let _ = worker.join();
                Err(io::Error::other(message))
            }
            Err(error) => {
                let _ = commands.send(HistoryCommand::Shutdown);
                reap_worker(worker);
                Err(io::Error::new(io::ErrorKind::TimedOut, error.to_string()))
            }
        }
    }

    fn send(&self, command: HistoryCommand) -> Result<(), HistoryRuntimeError> {
        self.commands
            .send(command)
            .map_err(|_| HistoryRuntimeError::WorkerStopped)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_save(&self, scan: ScanRecord) -> Result<HistoryReply<()>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::Save { scan, reply })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_list(&self) -> Result<HistoryReply<Vec<ScanSummary>>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::List { reply })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_load(
        &self,
        scan_id: impl Into<String>,
    ) -> Result<HistoryReply<ScanRecord>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::Load {
            scan_id: scan_id.into(),
            reply,
        })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_compare(
        &self,
        current_id: impl Into<String>,
        previous_id: impl Into<String>,
    ) -> Result<HistoryReply<ComparisonResult>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::Compare {
            current_id: current_id.into(),
            previous_id: previous_id.into(),
            reply,
        })?;
        Ok(receiver)
    }

    /// Compare a live scan snapshot against the newest persisted scan other
    /// than the current one (scan ids are unique per run). This is the
    /// report-generation baseline policy; it works even when the current scan
    /// has not itself been saved.
    ///
    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_compare_current_to_latest(
        &self,
        current: std::sync::Arc<ScanRecord>,
    ) -> Result<HistoryReply<Option<ComparisonResult>>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::CompareCurrentToLatest { current, reply })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_compare_summary(
        &self,
        current_id: impl Into<String>,
        previous_id: impl Into<String>,
    ) -> Result<HistoryReply<ComparisonSummary>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::CompareSummary {
            current_id: current_id.into(),
            previous_id: previous_id.into(),
            reply,
        })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_task_diff(
        &self,
        current_id: impl Into<String>,
        previous_id: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Result<HistoryReply<TaskDiffDetail>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::TaskDiff {
            current_id: current_id.into(),
            previous_id: previous_id.into(),
            task_id: task_id.into(),
            reply,
        })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_update_tags(
        &self,
        scan_id: impl Into<String>,
        tags: Vec<String>,
    ) -> Result<HistoryReply<()>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::UpdateTags {
            scan_id: scan_id.into(),
            tags,
            reply,
        })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_update_label(
        &self,
        scan_id: impl Into<String>,
        label: Option<String>,
    ) -> Result<HistoryReply<()>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::UpdateLabel {
            scan_id: scan_id.into(),
            label,
            reply,
        })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_trends(
        &self,
        limit: usize,
    ) -> Result<HistoryReply<Vec<TaskTrend>>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::Trends { limit, reply })?;
        Ok(receiver)
    }

    /// # Errors
    /// Returns [`HistoryRuntimeError::WorkerStopped`] if the worker exited.
    pub fn request_clear(&self) -> Result<HistoryReply<()>, HistoryRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.send(HistoryCommand::Clear { reply })?;
        Ok(receiver)
    }
}

impl Drop for NativeHistoryRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(HistoryCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TaskResult, Timestamp};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "wfdiag_history_runtime_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn scan(id: &str, timestamp: &str, success: bool) -> ScanRecord {
        ScanRecord {
            id: id.into(),
            timestamp: Timestamp::from_iso_string(timestamp).expect("timestamp"),
            computer_name: "TEST-PC".into(),
            os_version: "Windows 11".into(),
            is_admin: false,
            results: HashMap::from([(
                "os_info".into(),
                Arc::new(TaskResult {
                    success,
                    output: if success { "ok" } else { "failed" }.into(),
                    error: None,
                    duration_ms: 5,
                }),
            )]),
            task_count: 1,
            success_count: usize::from(success),
            failure_count: usize::from(!success),
            duration_ms: 5,
            label: None,
            tags: Vec::new(),
        }
    }

    fn task_catalog() -> Vec<DiagnosticTask> {
        vec![DiagnosticTask {
            id: "os_info".into(),
            name: "Operating System".into(),
            description: "Operating system details".into(),
            category: "System".into(),
            admin_required: false,
        }]
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_roundtrips_lists_compares_updates_and_clears() {
        let directory = temp_dir("roundtrip");
        let runtime = NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
            directory.clone(),
            || (true, 30),
            task_catalog,
        ))
        .expect("start runtime");

        runtime
            .request_save(scan("older", "2026-06-12T10:00:00Z", true))
            .expect("queue save")
            .await
            .expect("worker reply")
            .expect("save older");
        runtime
            .request_save(scan("newer", "2026-06-12T11:00:00Z", false))
            .expect("queue save")
            .await
            .expect("worker reply")
            .expect("save newer");

        let summaries = runtime
            .request_list()
            .expect("queue list")
            .await
            .expect("worker reply")
            .expect("list");
        assert_eq!(
            summaries
                .iter()
                .map(|scan| scan.id.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );

        let comparison = runtime
            .request_compare("newer", "older")
            .expect("queue comparison")
            .await
            .expect("worker reply")
            .expect("compare");
        assert_eq!(comparison.new_failures.len(), 1);
        assert_eq!(comparison.new_failures[0].task_name, "Operating System");

        runtime
            .request_update_label("newer", Some("  After update  ".into()))
            .expect("queue label")
            .await
            .expect("worker reply")
            .expect("label");
        runtime
            .request_update_tags("newer", vec!["regression".into()])
            .expect("queue tags")
            .await
            .expect("worker reply")
            .expect("tags");
        let loaded = runtime
            .request_load("newer")
            .expect("queue load")
            .await
            .expect("worker reply")
            .expect("load");
        assert_eq!(loaded.label.as_deref(), Some("After update"));
        assert_eq!(loaded.tags, ["regression"]);

        let trends = runtime
            .request_trends(30)
            .expect("queue trends")
            .await
            .expect("worker reply")
            .expect("trends");
        assert_eq!(trends[0].failed, 1);
        assert_eq!(trends[0].seen_in, 2);

        runtime
            .request_clear()
            .expect("queue clear")
            .await
            .expect("worker reply")
            .expect("clear");
        assert!(
            runtime
                .request_list()
                .expect("queue list")
                .await
                .expect("worker reply")
                .expect("list")
                .is_empty()
        );
        drop(runtime);
        std::fs::remove_dir_all(directory).ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_comparison_uses_newest_other_session_and_handles_empty_history() {
        let directory = temp_dir("live-comparison");
        let runtime = NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
            directory.clone(),
            || (true, 30),
            task_catalog,
        ))
        .expect("start runtime");
        for stored in [
            scan("older", "2026-06-12T10:00:00Z", true),
            scan("newer", "2026-06-12T11:00:00Z", false),
        ] {
            runtime
                .request_save(stored)
                .expect("queue save")
                .await
                .expect("worker reply")
                .expect("save scan");
        }

        let live_comparison = runtime
            .request_compare_current_to_latest(std::sync::Arc::new(scan("live", "2026-06-12T12:00:00Z", true)))
            .expect("queue live comparison")
            .await
            .expect("worker reply")
            .expect("compare live scan")
            .expect("newest baseline");
        assert_eq!(live_comparison.current_scan.id, "live");
        assert_eq!(live_comparison.previous_scan.id, "newer");
        assert_eq!(live_comparison.new_successes.len(), 1);

        let autosaved_current = runtime
            .request_compare_current_to_latest(std::sync::Arc::new(scan("newer", "2026-06-12T12:00:00Z", false)))
            .expect("queue autosaved-current comparison")
            .await
            .expect("worker reply")
            .expect("compare autosaved current")
            .expect("older baseline");
        assert_eq!(autosaved_current.previous_scan.id, "older");

        runtime
            .request_clear()
            .expect("queue clear")
            .await
            .expect("worker reply")
            .expect("clear");
        let empty = runtime
            .request_compare_current_to_latest(std::sync::Arc::new(scan("after-clear", "2026-06-12T13:00:00Z", true)))
            .expect("queue empty comparison")
            .await
            .expect("worker reply")
            .expect("compare empty history");
        assert!(empty.is_none());

        drop(runtime);
        std::fs::remove_dir_all(directory).ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn retention_policy_is_injected_and_enforced() {
        let directory = temp_dir("retention");
        let runtime = NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
            directory.clone(),
            || (true, 2),
            task_catalog,
        ))
        .expect("start runtime");
        for (id, time) in [
            ("first", "2026-06-12T10:00:00Z"),
            ("second", "2026-06-12T11:00:00Z"),
            ("third", "2026-06-12T12:00:00Z"),
        ] {
            runtime
                .request_save(scan(id, time, true))
                .expect("queue save")
                .await
                .expect("worker reply")
                .expect("save");
        }
        let summaries = runtime
            .request_list()
            .expect("queue list")
            .await
            .expect("worker reply")
            .expect("list");
        assert_eq!(
            summaries
                .iter()
                .map(|scan| scan.id.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "second"]
        );
        drop(runtime);
        std::fs::remove_dir_all(directory).ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disabled_retention_acknowledges_without_writing() {
        let directory = temp_dir("disabled");
        let runtime = NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
            directory.clone(),
            || (false, 30),
            task_catalog,
        ))
        .expect("start runtime");
        runtime
            .request_save(scan("ignored", "2026-06-12T10:00:00Z", true))
            .expect("queue save")
            .await
            .expect("worker reply")
            .expect("save is a no-op");
        assert!(
            runtime
                .request_list()
                .expect("queue list")
                .await
                .expect("worker reply")
                .expect("list")
                .is_empty()
        );
        drop(runtime);
        std::fs::remove_dir_all(directory).ok();
    }
}
