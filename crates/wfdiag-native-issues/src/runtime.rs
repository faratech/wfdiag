use crate::{
    DetectCtx, Issue, RemediationSummary, TaskResult, Timestamp, catalog, detect_all_with,
};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;

/// One immutable issue-detection request.
///
/// Time and temporary-directory state are explicit inputs so replaying the
/// same completed diagnostic evidence always produces the same output.
#[derive(Debug, Clone)]
pub struct IssueDetectionRequest {
    pub request_id: u64,
    pub results: HashMap<String, TaskResult>,
    pub now: Timestamp,
    pub temp_file_count: Option<usize>,
}

/// Worker response carrying the exact shipping Issue projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDetectionCompleted {
    pub request_id: u64,
    pub issues: Vec<Issue>,
}

#[derive(Debug)]
enum WorkerCommand {
    Detect(IssueDetectionRequest),
    Stop,
}

/// Errors establishing or communicating with the issue worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueRuntimeError {
    Spawn(String),
    Disconnected,
    WorkerPanicked,
    DuplicateRemediationSummary(String),
    MissingRemediationSummary {
        issue_id: &'static str,
        remediation_id: &'static str,
    },
}

impl fmt::Display for IssueRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(reason) => write!(formatter, "failed to start issue worker: {reason}"),
            Self::Disconnected => formatter.write_str("issue worker is disconnected"),
            Self::WorkerPanicked => formatter.write_str("issue worker panicked"),
            Self::DuplicateRemediationSummary(id) => {
                write!(formatter, "duplicate remediation summary '{id}'")
            }
            Self::MissingRemediationSummary {
                issue_id,
                remediation_id,
            } => write!(
                formatter,
                "issue '{issue_id}' references missing remediation summary '{remediation_id}'"
            ),
        }
    }
}

impl std::error::Error for IssueRuntimeError {}

/// Dedicated issue-detection worker suitable for a native UI thread.
///
/// `enqueue` only sends to an unbounded in-process channel; JSON parsing and
/// the complete catalog sweep happen on the worker thread. Remediation data is
/// immutable and read-only. This runtime exposes no execution operation.
pub struct IssueRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    remediation_summaries: Arc<HashMap<String, RemediationSummary>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for IssueRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssueRuntime")
            .field(
                "remediation_summary_count",
                &self.remediation_summaries.len(),
            )
            .finish_non_exhaustive()
    }
}

impl IssueRuntime {
    /// Start with the complete canonical read-only metadata catalog.
    ///
    /// This retains the same completeness/duplicate validation as [`Self::start`]
    /// and exposes no remediation execution capability.
    ///
    /// # Errors
    ///
    /// Returns the same invariant or worker-spawn errors as [`Self::start`].
    pub fn start_canonical()
    -> Result<(Self, mpsc::Receiver<IssueDetectionCompleted>), IssueRuntimeError> {
        Self::start(crate::remediation_summaries())
    }

    /// Start the worker with a snapshot of the canonical remediation catalog.
    ///
    /// Every remediation referenced by an issue must resolve before a worker
    /// starts, preventing a native shell from silently losing action metadata.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for missing or duplicate summaries, or a
    /// spawn error when the operating system cannot create the worker thread.
    pub fn start(
        remediation_summaries: Vec<RemediationSummary>,
    ) -> Result<(Self, mpsc::Receiver<IssueDetectionCompleted>), IssueRuntimeError> {
        let mut by_id = HashMap::with_capacity(remediation_summaries.len());
        for summary in remediation_summaries {
            let id = summary.id.clone();
            if by_id.insert(id.clone(), summary).is_some() {
                return Err(IssueRuntimeError::DuplicateRemediationSummary(id));
            }
        }
        for spec in catalog() {
            if let Some(remediation_id) = spec.remediation_id
                && !by_id.contains_key(remediation_id)
            {
                return Err(IssueRuntimeError::MissingRemediationSummary {
                    issue_id: spec.id,
                    remediation_id,
                });
            }
        }

        let remediation_summaries = Arc::new(by_id);
        let summaries_for_worker = Arc::clone(&remediation_summaries);
        let (command_tx, command_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("wfdiag-issue-detection".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        WorkerCommand::Detect(request) => {
                            let ctx = DetectCtx {
                                results: &request.results,
                                now: request.now,
                                temp_file_count: request.temp_file_count,
                            };
                            let resolve = |id: &str| summaries_for_worker.get(id).cloned();
                            let issues = detect_all_with(&ctx, &resolve);
                            if reply_tx
                                .send(IssueDetectionCompleted {
                                    request_id: request.request_id,
                                    issues,
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
            .map_err(|error| IssueRuntimeError::Spawn(error.to_string()))?;

        Ok((
            Self {
                commands: command_tx,
                remediation_summaries,
                worker: Some(worker),
            },
            reply_rx,
        ))
    }

    /// Queue detection without performing parsing or catalog work on the
    /// caller's thread.
    ///
    /// # Errors
    ///
    /// Returns [`IssueRuntimeError::Disconnected`] after worker shutdown.
    pub fn enqueue(&self, request: IssueDetectionRequest) -> Result<(), IssueRuntimeError> {
        self.commands
            .send(WorkerCommand::Detect(request))
            .map_err(|_| IssueRuntimeError::Disconnected)
    }

    /// Read one immutable remediation summary for native issue rendering.
    #[must_use]
    pub fn remediation_summary(&self, id: &str) -> Option<RemediationSummary> {
        self.remediation_summaries.get(id).cloned()
    }

    /// Return the complete read-only remediation snapshot in stable id order.
    #[must_use]
    pub fn remediation_summaries(&self) -> Vec<RemediationSummary> {
        let mut summaries: Vec<_> = self.remediation_summaries.values().cloned().collect();
        summaries.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        summaries
    }

    /// Stop and join the worker from a non-UI shutdown path.
    ///
    /// # Errors
    ///
    /// Returns [`IssueRuntimeError::WorkerPanicked`] if the worker panicked.
    pub fn shutdown(mut self) -> Result<(), IssueRuntimeError> {
        let _ = self.commands.send(WorkerCommand::Stop);
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            return Err(IssueRuntimeError::WorkerPanicked);
        }
        Ok(())
    }
}

impl Drop for IssueRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueStatus;
    use std::time::Duration;

    fn complete_remediation_catalog() -> Vec<RemediationSummary> {
        crate::remediation_summaries()
    }

    fn low_disk_request(request_id: u64) -> IssueDetectionRequest {
        IssueDetectionRequest {
            request_id,
            results: HashMap::from([(
                "logical_disk".to_string(),
                TaskResult {
                    success: true,
                    output: r#"[{"Name":"C:","FreeSpace":5,"Size":100}]"#.to_string(),
                    error: None,
                    duration_ms: 3,
                },
            )]),
            now: Timestamp::from_secs(1_781_264_000),
            temp_file_count: Some(1),
        }
    }

    #[test]
    fn worker_is_deterministic_and_embeds_the_supplied_summary() {
        let summaries = complete_remediation_catalog();
        let by_id: HashMap<_, _> = summaries
            .iter()
            .cloned()
            .map(|summary| (summary.id.clone(), summary))
            .collect();
        let direct_request = low_disk_request(0);
        let direct_ctx = DetectCtx {
            results: &direct_request.results,
            now: direct_request.now,
            temp_file_count: direct_request.temp_file_count,
        };
        let resolve = |id: &str| by_id.get(id).cloned();
        let direct = detect_all_with(&direct_ctx, &resolve);

        let (runtime, replies) = IssueRuntime::start_canonical().unwrap();
        runtime.enqueue(low_disk_request(10)).unwrap();
        runtime.enqueue(low_disk_request(11)).unwrap();

        let first = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(first.request_id, 10);
        assert_eq!(second.request_id, 11);
        assert_eq!(first.issues, second.issues);
        assert_eq!(first.issues, direct);

        let low_disk = first
            .issues
            .iter()
            .find(|issue| issue.id == "low_disk_space")
            .unwrap();
        assert_eq!(low_disk.status, IssueStatus::Detected);
        assert_eq!(
            low_disk.remediation.as_ref().map(|value| value.id.as_str()),
            Some("open_disk_cleanup")
        );
        runtime.shutdown().unwrap();
    }

    #[test]
    fn worker_refuses_an_incomplete_or_duplicate_remediation_snapshot() {
        assert!(matches!(
            IssueRuntime::start(Vec::new()),
            Err(IssueRuntimeError::MissingRemediationSummary { .. })
        ));
        let mut summaries = complete_remediation_catalog();
        summaries.push(summaries[0].clone());
        assert!(matches!(
            IssueRuntime::start(summaries),
            Err(IssueRuntimeError::DuplicateRemediationSummary(_))
        ));
    }

    #[test]
    fn issue_json_matches_the_shipping_field_contract() {
        let (runtime, replies) = IssueRuntime::start_canonical().unwrap();
        runtime.enqueue(low_disk_request(1)).unwrap();
        let response = replies.recv_timeout(Duration::from_secs(2)).unwrap();
        let issue = response
            .issues
            .iter()
            .find(|issue| issue.id == "low_disk_space")
            .unwrap();
        let value = serde_json::to_value(issue).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": "low_disk_space",
                "category": "Storage",
                "severity": "Critical",
                "status": "detected",
                "title": "Low Disk Space",
                "description": "The disk 'C:' is running low on space (5.00% free).",
                "recommendation": "Free up disk space by deleting unnecessary files.",
                "detected": true,
                "source_tasks": ["logical_disk"],
                "remediation": {
                    "id": "open_disk_cleanup",
                    "label": "Open Disk Cleanup",
                    "description": "Opens Windows Disk Cleanup (cleanmgr.exe) to pick what to remove.",
                    "tier": "open_tool",
                    "admin_required": false,
                    "requires_restart": false,
                    "long_running": false,
                    "maintenance": false,
                    "batch_eligible": false,
                    "cancellable": false
                }
            })
        );
        runtime.shutdown().unwrap();
    }
}
