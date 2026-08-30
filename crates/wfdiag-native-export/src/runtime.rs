use crate::{ExportPayload, ExportRequestKind, ExportTask, TaskResult, renderer::render_request};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;

/// One owned request queued from a native UI thread.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub request_id: u64,
    pub kind: ExportRequestKind,
    pub results: Arc<HashMap<String, TaskResult>>,
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
                            let result =
                                render_request(&request.kind, &request.results, &worker_tasks);
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

    /// Stop and join from a non-UI shutdown path.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::WorkerPanicked`] if the worker panicked.
    pub fn shutdown(mut self) -> Result<(), ExportError> {
        let _ = self.commands.send(WorkerCommand::Stop);
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            return Err(ExportError::WorkerPanicked);
        }
        Ok(())
    }
}

impl Drop for ExportRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExportMetadata, ReportFormat, render_report, render_saved_report};
    use std::time::Duration;

    #[test]
    fn worker_matches_the_pure_renderer_and_preserves_request_id() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = HashMap::from([(
            "os_info".to_string(),
            TaskResult {
                success: true,
                output: r#"{"edition":"Pro"}"#.to_string(),
                error: None,
                duration_ms: 5,
            },
        )]);
        let expected = render_report(ReportFormat::Text, true, &results, &tasks).unwrap();
        let (runtime, replies) = ExportRuntime::start(tasks).unwrap();
        runtime
            .enqueue(ExportRequest {
                request_id: 42,
                kind: ExportRequestKind::Report {
                    format: ReportFormat::Text,
                    include_raw: true,
                },
                results: Arc::new(results),
            })
            .unwrap();
        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed.request_id, 42);
        assert_eq!(completed.result, Ok(ExportPayload::Report(expected)));
        runtime.shutdown().unwrap();
    }

    #[test]
    fn worker_routes_saved_report_metadata_and_preserves_request_id() {
        let tasks = vec![ExportTask::new("os_info", "Operating System", "System")];
        let results = HashMap::from([(
            "os_info".to_string(),
            TaskResult {
                success: true,
                output: r#"{"edition":"Pro"}"#.to_string(),
                error: None,
                duration_ms: 5,
            },
        )]);
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
                results: Arc::new(results),
            })
            .unwrap();
        let completed = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed.request_id, 43);
        assert_eq!(completed.result, Ok(ExportPayload::Report(expected)));
        runtime.shutdown().unwrap();
    }
}
