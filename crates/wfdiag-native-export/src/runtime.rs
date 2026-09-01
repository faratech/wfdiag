use crate::{ExportPayload, ExportRequestKind, ExportTask, renderer::render_request};
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;
use wfdiag_native_issues::SharedScanEvidence;

/// One owned request queued from a native UI thread.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub request_id: u64,
    pub kind: ExportRequestKind,
    pub results: SharedScanEvidence,
}

/// Terminal response for one export request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportCompleted {
    pub request_id: u64,
    pub result: Result<ExportPayload, ExportError>,
}

#[derive(Debug)]
enum WorkerCommand {
    Generate(ExportRequest),
    Stop,
}

/// Rendering/runtime error independent of any UI framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    UnsupportedFormat(String),
    Serialization(String),
    Spawn(String),
    Disconnected,
    WorkerPanicked,
    ShutdownTimedOut,
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported export format: {format}")
            }
            Self::Serialization(reason) => {
                write!(formatter, "export serialization failed: {reason}")
            }
            Self::Spawn(reason) => write!(formatter, "failed to start export worker: {reason}"),
            Self::Disconnected => formatter.write_str("export worker is disconnected"),
            Self::WorkerPanicked => formatter.write_str("export worker panicked"),
            Self::ShutdownTimedOut => {
                formatter.write_str("export worker did not stop within the shutdown budget")
            }
        }
    }
}

impl std::error::Error for ExportError {}

/// Dedicated export worker with an unbounded, nonblocking request queue.
pub struct ExportRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    tasks: Arc<Vec<ExportTask>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for ExportRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportRuntime")
            .field("task_count", &self.tasks.len())
            .finish_non_exhaustive()
    }
}

impl ExportRuntime {
    /// Start an export worker with an immutable diagnostic task catalog.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Spawn`] if the worker thread cannot be created.
    pub fn start(
        tasks: Vec<ExportTask>,
    ) -> Result<(Self, mpsc::Receiver<ExportCompleted>), ExportError> {
        let tasks = Arc::new(tasks);
        let worker_tasks = Arc::clone(&tasks);
        let (command_tx, command_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("wfdiag-export".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        WorkerCommand::Generate(request) => {
                            let result = render_request(
                                &request.kind,
                                request.results.as_ref(),
                                &worker_tasks,
                            );
                            if reply_tx
                                .send(ExportCompleted {
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
            .map_err(|error| ExportError::Spawn(error.to_string()))?;

        Ok((
            Self {
                commands: command_tx,
                tasks,
                worker: Some(worker),
            },
            reply_rx,
        ))
    }

    /// Queue generation without formatting on the caller's thread.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Disconnected`] after worker shutdown.
    pub fn enqueue(&self, request: ExportRequest) -> Result<(), ExportError> {
        self.commands
            .send(WorkerCommand::Generate(request))
            .map_err(|_| ExportError::Disconnected)
    }

    /// Return the immutable export task catalog.
    #[must_use]
    pub fn tasks(&self) -> &[ExportTask] {
        &self.tasks
    }

    /// Bounded, ordered teardown from a non-UI shutdown path.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::ShutdownTimedOut`] when the worker had not exited
    /// within `budget`. The worker is still reaped on a detached thread.
    pub fn shutdown(mut self, budget: Duration) -> Result<(), ExportError> {
        if self.stop_and_join(budget) {
            Ok(())
        } else {
            Err(ExportError::ShutdownTimedOut)
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

impl Drop for ExportRuntime {
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
        .name("wfdiag-export-reaper".to_string())
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
    use crate::{
        ExportMetadata, ReportFormat, TaskResult, render_email, render_report, render_saved_report,
        render_support_package,
    };
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn shared_results() -> SharedScanEvidence {
        Arc::new(HashMap::from([(
            "os_info".to_string(),
            Arc::new(TaskResult {
                success: true,
                output: r#"{"edition":"Pro"}"#.to_string(),
                error: None,
                duration_ms: 5,
            }),
        )]))
    }

    #[test]
    fn stop_and_join_reaps_the_worker_and_drop_returns_promptly() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let (mut runtime, _replies) = ExportRuntime::start(tasks.clone()).unwrap();
        assert!(runtime.stop_and_join(Duration::from_secs(2)));
        // The worker thread really exited: its command receiver is gone, so
        // the next enqueue cannot be delivered.
        assert_eq!(
            runtime.enqueue(ExportRequest {
                request_id: 1,
                kind: ExportRequestKind::Report {
                    format: ReportFormat::Text,
                    include_raw: false,
                },
                results: shared_results(),
            }),
            Err(ExportError::Disconnected)
        );
        // The handle has already been taken, so a second call is a no-op.
        assert!(runtime.stop_and_join(Duration::from_millis(1)));

        let (fresh, _replies) = ExportRuntime::start(tasks).unwrap();
        let started = Instant::now();
        drop(fresh);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop blocked the caller: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn worker_matches_the_pure_renderer_and_preserves_request_id() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = shared_results();
        let expected = render_report(ReportFormat::Text, true, &results, &tasks).unwrap();
        let (runtime, replies) = ExportRuntime::start(tasks).unwrap();
        runtime
            .enqueue(ExportRequest {
                request_id: 42,
                kind: ExportRequestKind::Report {
                    format: ReportFormat::Text,
                    include_raw: true,
                },
                results,
            })
            .unwrap();
        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed.request_id, 42);
        assert_eq!(completed.result, Ok(ExportPayload::Report(expected)));
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn worker_routes_saved_report_metadata_and_preserves_request_id() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = shared_results();
        let metadata = ExportMetadata {
            generated: "8/30/2026, 1:02:03 PM".to_string(),
            local_date: "8/30/2026".to_string(),
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11 Pro (25H2)".to_string(),
            is_admin: true,
        };
        let expected =
            render_saved_report(ReportFormat::Text, true, &metadata, &results, &tasks).unwrap();
        let (runtime, replies) = ExportRuntime::start(tasks).unwrap();
        runtime
            .enqueue(ExportRequest {
                request_id: 43,
                kind: ExportRequestKind::SavedReport {
                    format: ReportFormat::Text,
                    include_raw: true,
                    metadata,
                },
                results,
            })
            .unwrap();
        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed.request_id, 43);
        assert_eq!(completed.result, Ok(ExportPayload::Report(expected)));
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn worker_routes_email_payload_without_putting_the_report_in_the_draft_body() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = shared_results();
        let metadata = ExportMetadata {
            generated: "8/30/2026, 1:02:03 PM".to_string(),
            local_date: "8/30/2026".to_string(),
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11 Pro (25H2)".to_string(),
            is_admin: true,
        };
        let expected = render_email(&metadata, &results, &tasks);
        let (runtime, replies) = ExportRuntime::start(tasks).unwrap();
        runtime
            .enqueue(ExportRequest {
                request_id: 44,
                kind: ExportRequestKind::Email { metadata },
                results,
            })
            .unwrap();

        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();

        assert_eq!(completed.request_id, 44);
        assert_eq!(completed.result, Ok(ExportPayload::Email(expected)));
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn worker_routes_one_atomic_support_package_payload() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = shared_results();
        let expected = render_support_package(true, &results, &tasks).unwrap();
        let (runtime, replies) = ExportRuntime::start(tasks).unwrap();
        runtime
            .enqueue(ExportRequest {
                request_id: 45,
                kind: ExportRequestKind::SupportPackage { include_raw: true },
                results,
            })
            .unwrap();

        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();

        assert_eq!(completed.request_id, 45);
        assert_eq!(
            completed.result,
            Ok(ExportPayload::SupportPackage(expected))
        );
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }
}
