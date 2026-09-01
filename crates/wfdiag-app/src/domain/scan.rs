//! Pure scan-session state machine.
//!
//! This is the portable form of the native shell's diagnostic state (its
//! `DiagnosticSnapshot`, `TargetedDiagnosticOverlay`, and the
//! `begin`/`started`/`finished`/`cancel` message handlers). It performs no
//! I/O: the caller owns the runtime and feeds it worker outcomes.
//!
//! Two transactions live here:
//!
//! * A **replacement scan** (quick or full) stashes the currently committed
//!   snapshot in `previous` and clears the visible results when the session
//!   starts. Any failure path restores `previous` untouched.
//! * A **targeted rerun** of exactly one task keeps the committed snapshot
//!   visible for the whole run and stages the single replacement in an
//!   overlay. The overlay commits by merging exactly one row into the base;
//!   every failure path rolls back to the base with nothing else disturbed.

use std::collections::HashMap;
use std::time::Instant;
use wfdiag_native_diagnostics::{DiagnosticTask, ScanKind};
use wfdiag_native_issues::{ScanEvidence, SharedScanEvidence};
use wfdiag_ui_core::{DiagnosticTaskResult, TaskProgress, TaskProgressStatus};

/// Everything the UI shows about one committed or in-flight scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanSnapshot {
    /// Per-task results in catalog order.
    pub results: Vec<DiagnosticTaskResult>,
    /// The kind of scan that produced `results`, when one has run.
    pub scan_kind: Option<ScanKind>,
    /// The task ids the scan was asked to collect.
    pub task_ids: Vec<String>,
    /// The diagnostic session id that produced `results`.
    pub session_id: Option<String>,
    /// Wall-clock duration of the run that produced `results`.
    pub duration_ms: u64,
    /// Number of tasks expected.
    pub total: usize,
    /// Number of tasks whose result has arrived.
    pub completed: usize,
    /// Number of arrived results that failed.
    pub errors: usize,
}

impl ScanSnapshot {
    /// True when no scan has ever committed evidence.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.results.is_empty() && self.session_id.is_none()
    }

    /// The session id, falling back to the one carried by the first result.
    #[must_use]
    pub fn effective_session_id(&self) -> Option<String> {
        self.session_id
            .clone()
            .or_else(|| self.results.first().map(|result| result.session_id.clone()))
    }

    /// The evidence map matching `results`, ready for issue detection or
    /// export without a second deep copy of any task output.
    #[must_use]
    pub fn evidence(&self) -> SharedScanEvidence {
        std::sync::Arc::new(
            self.results
                .iter()
                .map(|result| {
                    (
                        result.task_id.clone(),
                        std::sync::Arc::clone(&result.result),
                    )
                })
                .collect(),
        )
    }
}

/// The single-row rerun transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedOverlay {
    target_task_id: String,
    base: ScanSnapshot,
    staged_result: Option<DiagnosticTaskResult>,
}

impl TargetedOverlay {
    /// Open an overlay when a committed session is being re-run for exactly
    /// one task. Any other shape returns `None` and the caller performs a
    /// normal replacement transaction instead.
    #[must_use]
    pub fn for_committed_session(
        scan_kind: ScanKind,
        task_ids: &[String],
        base: ScanSnapshot,
    ) -> Option<Self> {
        let [target_task_id] = task_ids else {
            return None;
        };
        (scan_kind == ScanKind::Targeted && base.session_id.is_some()).then(|| Self {
            target_task_id: target_task_id.clone(),
            base,
            staged_result: None,
        })
    }

    /// The task this rerun is allowed to replace.
    #[must_use]
    pub fn target_task_id(&self) -> &str {
        &self.target_task_id
    }

    /// Stage a streamed result. Anything but the target task is ignored.
    pub fn stage(&mut self, result: DiagnosticTaskResult) {
        if result.task_id == self.target_task_id {
            self.staged_result = Some(result);
        }
    }

    /// Progress counters for the rerun itself, not the base snapshot.
    #[must_use]
    pub fn staged_counts(&self) -> (usize, usize) {
        self.staged_result
            .as_ref()
            .map_or((0, 0), |result| (1, usize::from(!result.success)))
    }

    /// Abandon the rerun, restoring the untouched committed snapshot.
    #[must_use]
    pub fn rollback(self) -> ScanSnapshot {
        self.base
    }

    /// Merge the authoritative replacement into the base snapshot.
    ///
    /// # Errors
    ///
    /// Returns a message when the runtime returned a different task than the
    /// one this overlay was opened for.
    pub fn commit(
        &self,
        replacement: DiagnosticTaskResult,
        catalog: &[DiagnosticTask],
    ) -> Result<ScanSnapshot, String> {
        let results = merge_targeted_result(
            self.base.results.clone(),
            &self.target_task_id,
            replacement,
            self.base.session_id.as_deref(),
            catalog,
        )?;
        let mut task_ids = self.base.task_ids.clone();
        if !task_ids.iter().any(|id| id == &self.target_task_id) {
            task_ids.push(self.target_task_id.clone());
        }
        let completed = results.len();
        let errors = results.iter().filter(|result| !result.success).count();
        Ok(ScanSnapshot {
            results,
            scan_kind: self.base.scan_kind,
            total: task_ids.len(),
            task_ids,
            session_id: self.base.session_id.clone(),
            duration_ms: self.base.duration_ms,
            completed,
            errors,
        })
    }
}

/// Replace exactly one row of a committed result list, preserving catalog
/// order for known tasks and prior order for unknown ones.
///
/// # Errors
///
/// Returns a message when `replacement` is not the requested task.
pub fn merge_targeted_result(
    mut prior: Vec<DiagnosticTaskResult>,
    target_task_id: &str,
    mut replacement: DiagnosticTaskResult,
    base_session_id: Option<&str>,
    catalog: &[DiagnosticTask],
) -> Result<Vec<DiagnosticTaskResult>, String> {
    if replacement.task_id != target_task_id {
        return Err(format!(
            "targeted rerun returned `{}` instead of `{target_task_id}`",
            replacement.task_id
        ));
    }
    let replacement_index = prior
        .iter()
        .position(|result| result.task_id == target_task_id);
    if let Some(session_id) = base_session_id {
        replacement.session_id = session_id.to_string();
    }
    let prior_order: HashMap<String, usize> = prior
        .iter()
        .enumerate()
        .map(|(index, result)| (result.task_id.clone(), index))
        .collect();
    if let Some(index) = replacement_index {
        prior[index] = replacement;
    } else {
        prior.push(replacement);
    }
    let catalog_order = catalog_order(catalog);
    prior.sort_by_key(|result| {
        catalog_order
            .get(result.task_id.as_str())
            .copied()
            .map_or_else(
                || {
                    (
                        1,
                        prior_order
                            .get(&result.task_id)
                            .copied()
                            .unwrap_or(usize::MAX),
                    )
                },
                |index| (0, index),
            )
    });
    Ok(prior)
}

fn catalog_order(catalog: &[DiagnosticTask]) -> HashMap<&str, usize> {
    catalog
        .iter()
        .enumerate()
        .map(|(index, task)| (task.id.as_str(), index))
        .collect()
}

/// Project the runtime's authoritative evidence map into catalog-ordered rows.
#[must_use]
pub fn authoritative_rows(
    session_id: &str,
    evidence: &ScanEvidence,
    catalog: &[DiagnosticTask],
) -> Vec<DiagnosticTaskResult> {
    let order = catalog_order(catalog);
    let mut results: Vec<DiagnosticTaskResult> = evidence
        .iter()
        .map(|(task_id, result)| {
            DiagnosticTaskResult::new(session_id, task_id, std::sync::Arc::clone(result))
        })
        .collect();
    results.sort_by_key(|result| {
        order
            .get(result.task_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    results
}

/// Whether the runtime returned exactly the requested task set.
#[must_use]
pub fn evidence_is_complete(evidence: &ScanEvidence, expected_task_ids: &[String]) -> bool {
    let expected: std::collections::HashSet<&str> =
        expected_task_ids.iter().map(String::as_str).collect();
    expected.len() == expected_task_ids.len()
        && evidence.len() == expected_task_ids.len()
        && evidence
            .keys()
            .all(|task_id| expected.contains(task_id.as_str()))
}

/// Where the scan machine currently is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanPhase {
    /// Nothing is running; `snapshot` is the committed evidence.
    #[default]
    Idle,
    /// A session has been requested but the runtime has not answered.
    Starting,
    /// The runtime is executing tasks.
    Running,
    /// Cancellation was requested; in-flight tasks are still finishing.
    Cancelling,
    /// The run completed and optional history persistence is pending.
    Finalizing,
}

/// Per-scan policy captured from settings when the scan starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanPolicy {
    /// Whether the completed scan should be written to history.
    pub auto_save: bool,
    /// Executor concurrency for this run.
    pub max_concurrent_tasks: usize,
    /// The history tag written with the saved record.
    pub history_tag: String,
}

/// The terminal decision for one finished run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The completion belongs to a session that is no longer visible.
    Stale,
    /// The scan was stopped; the previous snapshot is visible again.
    Cancelled,
    /// The run failed; the previous snapshot is visible again.
    Failed {
        /// The runtime's diagnostic.
        error: String,
        /// Whether the user had asked to stop before the failure.
        stopped: bool,
    },
    /// The runtime returned a result set that is not the requested task set.
    Incomplete {
        /// How many results arrived.
        completed: usize,
        /// How many were requested.
        expected: usize,
    },
    /// A complete replacement scan committed.
    Committed {
        /// The committed session id.
        session_id: String,
        /// The authoritative evidence, shared with the committed snapshot.
        evidence: SharedScanEvidence,
        /// Whether history auto-save should run for this scan.
        auto_save: bool,
    },
    /// A single-row rerun committed into the existing snapshot.
    TargetedCommitted {
        /// The session id of the committed evidence (the base session).
        session_id: String,
        /// The one task that was replaced.
        task_id: String,
        /// The merged evidence for the whole snapshot.
        evidence: SharedScanEvidence,
    },
    /// The rerun could not be merged; the base snapshot is visible again.
    TargetedFailed {
        /// Why the merge was refused.
        error: String,
    },
}

/// The portable scan session machine.
#[derive(Debug, Default)]
pub struct ScanState {
    snapshot: ScanSnapshot,
    phase: ScanPhase,
    previous: Option<ScanSnapshot>,
    overlay: Option<TargetedOverlay>,
    policy: Option<ScanPolicy>,
    statuses: HashMap<String, TaskProgressStatus>,
    current_task: Option<String>,
    cancel_requested: bool,
    started_at: Option<Instant>,
}

impl ScanState {
    /// A machine with no committed evidence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The committed (or in-flight) snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &ScanSnapshot {
        &self.snapshot
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> ScanPhase {
        self.phase
    }

    /// The policy captured when the current scan started.
    #[must_use]
    pub const fn policy(&self) -> Option<&ScanPolicy> {
        self.policy.as_ref()
    }

    /// Per-task progress for the current scan.
    #[must_use]
    pub const fn statuses(&self) -> &HashMap<String, TaskProgressStatus> {
        &self.statuses
    }

    /// The task currently reported as running.
    #[must_use]
    pub fn current_task(&self) -> Option<&str> {
        self.current_task.as_deref()
    }

    /// Whether a targeted-rerun transaction is open.
    #[must_use]
    pub const fn has_overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// Whether a scan occupies the machine.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        !matches!(self.phase, ScanPhase::Idle)
    }

    /// Whether the user has asked for the current scan to stop.
    #[must_use]
    pub const fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    /// Open a scan transaction.
    ///
    /// Returns `true` when the transaction invalidates derived content
    /// immediately (every replacement scan). A targeted rerun returns `false`:
    /// it defers invalidation to its commit so a failure leaves the previous
    /// evidence and every projection derived from it intact.
    pub fn begin(
        &mut self,
        scan_kind: ScanKind,
        task_ids: Vec<String>,
        policy: ScanPolicy,
    ) -> bool {
        let previous = self.snapshot.clone();
        self.previous = None;
        self.overlay =
            TargetedOverlay::for_committed_session(scan_kind, &task_ids, previous.clone());
        let replacement = self.overlay.is_none();
        if replacement {
            self.previous = Some(previous);
        }
        self.statuses = task_ids
            .iter()
            .cloned()
            .map(|task_id| (task_id, TaskProgressStatus::Queued))
            .collect();
        self.snapshot.task_ids = task_ids;
        self.snapshot.scan_kind = Some(scan_kind);
        self.snapshot.total = self.snapshot.task_ids.len();
        self.snapshot.completed = 0;
        self.snapshot.errors = 0;
        self.snapshot.duration_ms = 0;
        self.policy = Some(policy);
        self.phase = ScanPhase::Starting;
        self.cancel_requested = false;
        self.current_task = None;
        self.started_at = Some(Instant::now());
        replacement
    }

    /// Accept the runtime's session id.
    ///
    /// Returns `false` for a stale start completion, which must never replace
    /// newer visible evidence.
    pub fn session_started(
        &mut self,
        session_id: String,
        scan_kind: ScanKind,
        total: usize,
    ) -> bool {
        if self.phase != ScanPhase::Starting || self.snapshot.scan_kind != Some(scan_kind) {
            return false;
        }
        self.phase = ScanPhase::Running;
        self.snapshot.total = total;
        self.snapshot.completed = 0;
        self.snapshot.errors = 0;
        self.snapshot.duration_ms = 0;
        self.snapshot.session_id = Some(session_id);
        self.current_task = None;
        if self.overlay.is_none() {
            self.snapshot.results.clear();
        }
        true
    }

    /// The session could not be created; restore the previous snapshot.
    pub fn start_failed(&mut self) {
        self.restore_previous();
        self.reset();
    }

    /// Apply one streamed progress event. Returns `false` when it belongs to
    /// another session.
    pub fn apply_progress(&mut self, progress: &TaskProgress, catalog: &[DiagnosticTask]) -> bool {
        if self.snapshot.session_id.as_deref() != Some(progress.session_id.as_str()) {
            return false;
        }
        self.statuses
            .insert(progress.task_id.clone(), progress.status);
        if progress.status == TaskProgressStatus::Running {
            let name = progress.task_name.clone().unwrap_or_else(|| {
                catalog
                    .iter()
                    .find(|task| task.id == progress.task_id)
                    .map_or_else(|| progress.task_id.clone(), |task| task.name.clone())
            });
            self.current_task = Some(name);
        }
        self.update_counts();
        true
    }

    /// Apply one streamed task result. Returns `false` when it belongs to
    /// another session.
    pub fn apply_result(
        &mut self,
        result: DiagnosticTaskResult,
        catalog: &[DiagnosticTask],
    ) -> bool {
        if self.snapshot.session_id.as_deref() != Some(result.session_id.as_str()) {
            return false;
        }
        self.statuses.insert(
            result.task_id.clone(),
            if result.success {
                TaskProgressStatus::Completed
            } else {
                TaskProgressStatus::Failed
            },
        );
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.stage(result);
            self.update_counts();
            return true;
        }
        if let Some(existing) = self
            .snapshot
            .results
            .iter_mut()
            .find(|existing| existing.task_id == result.task_id)
        {
            *existing = result;
        } else {
            self.snapshot.results.push(result);
        }
        let order = catalog_order(catalog);
        self.snapshot.results.sort_by_key(|result| {
            order
                .get(result.task_id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        self.update_counts();
        true
    }

    /// Record that the user asked to stop.
    ///
    /// Returns `true` when the caller should send a cancel request to the
    /// runtime; a scan that is still starting only records the intent and
    /// cancels once its session exists.
    pub fn request_cancel(&mut self) -> bool {
        match self.phase {
            ScanPhase::Starting => {
                self.cancel_requested = true;
                false
            }
            ScanPhase::Running => {
                self.cancel_requested = true;
                self.phase = ScanPhase::Cancelling;
                true
            }
            ScanPhase::Idle | ScanPhase::Cancelling | ScanPhase::Finalizing => false,
        }
    }

    /// Cancellation was refused by the runtime; the run continues.
    pub fn cancel_failed(&mut self) {
        if self.phase == ScanPhase::Cancelling {
            self.phase = ScanPhase::Running;
        }
        self.cancel_requested = false;
    }

    /// Decide what one finished run means for the visible evidence.
    pub fn finish_run(
        &mut self,
        session_id: &str,
        cancelled: bool,
        evidence: Result<SharedScanEvidence, String>,
        catalog: &[DiagnosticTask],
    ) -> RunOutcome {
        if self.snapshot.session_id.as_deref() != Some(session_id) {
            return RunOutcome::Stale;
        }
        self.update_counts();
        if cancelled {
            self.restore_previous();
            self.reset();
            return RunOutcome::Cancelled;
        }
        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(error) => {
                let stopped = self.cancel_requested || self.phase == ScanPhase::Cancelling;
                self.restore_previous();
                self.reset();
                return RunOutcome::Failed { error, stopped };
            }
        };
        let expected = self.snapshot.task_ids.len();
        if !evidence_is_complete(&evidence, &self.snapshot.task_ids) {
            let completed = evidence.len();
            self.restore_previous();
            self.reset();
            return RunOutcome::Incomplete {
                completed,
                expected,
            };
        }

        if let Some(overlay) = self.overlay.as_ref() {
            let Some(output) = evidence.get(overlay.target_task_id()) else {
                let error = format!(
                    "targeted rerun did not return `{}`",
                    overlay.target_task_id()
                );
                self.restore_previous();
                self.reset();
                return RunOutcome::TargetedFailed { error };
            };
            let replacement = DiagnosticTaskResult::new(
                session_id,
                overlay.target_task_id().to_string(),
                std::sync::Arc::clone(output),
            );
            let target_task_id = overlay.target_task_id().to_string();
            return match overlay.commit(replacement, catalog) {
                Ok(committed) => {
                    let committed_session = committed
                        .effective_session_id()
                        .unwrap_or_else(|| session_id.to_string());
                    let merged = committed.evidence();
                    self.snapshot = committed;
                    self.overlay = None;
                    self.previous = None;
                    self.reset();
                    RunOutcome::TargetedCommitted {
                        session_id: committed_session,
                        task_id: target_task_id,
                        evidence: merged,
                    }
                }
                Err(error) => {
                    self.restore_previous();
                    self.reset();
                    RunOutcome::TargetedFailed { error }
                }
            };
        }

        self.snapshot.results = authoritative_rows(session_id, &evidence, catalog);
        self.update_counts();
        self.previous = None;
        let committed_after_stop = self.cancel_requested || self.phase == ScanPhase::Cancelling;
        let auto_save = self
            .policy
            .as_ref()
            .is_some_and(|policy| policy.auto_save && !committed_after_stop);
        self.phase = ScanPhase::Finalizing;
        self.cancel_requested = false;
        self.current_task = None;
        if !auto_save {
            self.reset();
        }
        RunOutcome::Committed {
            session_id: session_id.to_string(),
            evidence,
            auto_save,
        }
    }

    /// History persistence finished (or was skipped); leave the finalizing
    /// phase. Returns `false` when the acknowledgement is stale.
    pub fn finish_finalization(&mut self, session_id: &str) -> bool {
        if self.phase != ScanPhase::Finalizing
            || self.snapshot.session_id.as_deref() != Some(session_id)
        {
            return false;
        }
        self.reset();
        true
    }

    /// Adopt an externally loaded snapshot (a history scan opened by the
    /// user). Refused while a scan is in flight.
    pub fn adopt(&mut self, snapshot: ScanSnapshot) -> bool {
        if self.is_busy() {
            return false;
        }
        self.previous = None;
        self.overlay = None;
        self.snapshot = snapshot;
        true
    }

    fn restore_previous(&mut self) {
        let previous = self
            .overlay
            .take()
            .map(TargetedOverlay::rollback)
            .or_else(|| self.previous.take());
        self.snapshot = previous.unwrap_or_default();
    }

    fn reset(&mut self) {
        self.phase = ScanPhase::Idle;
        self.policy = None;
        self.overlay = None;
        self.statuses.clear();
        self.current_task = None;
        self.cancel_requested = false;
        self.started_at = None;
    }

    fn update_counts(&mut self) {
        if let Some(overlay) = self.overlay.as_ref() {
            (self.snapshot.completed, self.snapshot.errors) = overlay.staged_counts();
        } else {
            self.snapshot.completed = self.snapshot.results.len();
            self.snapshot.errors = self
                .snapshot
                .results
                .iter()
                .filter(|result| !result.success)
                .count();
        }
        if let Some(started) = self.started_at {
            self.snapshot.duration_ms =
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RunOutcome, ScanPhase, ScanPolicy, ScanState, TargetedOverlay};
    use std::collections::HashMap;
    use std::sync::Arc;
    use wfdiag_native_diagnostics::{DiagnosticTask, ScanKind};
    use wfdiag_native_issues::TaskResult;
    use wfdiag_ui_core::DiagnosticTaskResult;

    fn catalog() -> Vec<DiagnosticTask> {
        ["os_info", "processor", "logical_disk"]
            .into_iter()
            .map(|id| DiagnosticTask {
                id: id.to_string(),
                name: format!("Task {id}"),
                description: String::new(),
                category: "Test".to_string(),
                admin_required: false,
            })
            .collect()
    }

    fn row(session: &str, task: &str, success: bool) -> DiagnosticTaskResult {
        DiagnosticTaskResult::new(
            session,
            task,
            Arc::new(TaskResult {
                success,
                output: format!("{task}-output"),
                error: None,
                duration_ms: 1,
            }),
        )
    }

    fn evidence(session: &str, tasks: &[(&str, bool)]) -> wfdiag_native_issues::SharedScanEvidence {
        let _ = session;
        Arc::new(
            tasks
                .iter()
                .map(|(task, success)| {
                    (
                        (*task).to_string(),
                        Arc::new(TaskResult {
                            success: *success,
                            output: format!("{task}-output"),
                            error: None,
                            duration_ms: 1,
                        }),
                    )
                })
                .collect::<HashMap<_, _>>(),
        )
    }

    fn policy() -> ScanPolicy {
        ScanPolicy {
            auto_save: true,
            max_concurrent_tasks: 5,
            history_tag: "Quick Scan".to_string(),
        }
    }

    fn committed_quick_scan() -> ScanState {
        let mut state = ScanState::new();
        assert!(state.begin(
            ScanKind::Quick,
            vec!["os_info".to_string(), "processor".to_string()],
            policy(),
        ));
        assert!(state.session_started("scan_1".to_string(), ScanKind::Quick, 2));
        let outcome = state.finish_run(
            "scan_1",
            false,
            Ok(evidence(
                "scan_1",
                &[("os_info", true), ("processor", true)],
            )),
            &catalog(),
        );
        assert!(matches!(outcome, RunOutcome::Committed { .. }));
        assert!(state.finish_finalization("scan_1"));
        state
    }

    #[test]
    fn a_replacement_scan_commits_catalog_ordered_evidence() {
        let state = committed_quick_scan();
        assert_eq!(state.phase(), ScanPhase::Idle);
        let ids: Vec<&str> = state
            .snapshot()
            .results
            .iter()
            .map(|result| result.task_id.as_str())
            .collect();
        assert_eq!(ids, ["os_info", "processor"]);
        assert_eq!(state.snapshot().completed, 2);
        assert_eq!(state.snapshot().errors, 0);
    }

    #[test]
    fn a_stale_start_completion_cannot_replace_newer_evidence() {
        let mut state = committed_quick_scan();
        assert!(!state.session_started("scan_stale".to_string(), ScanKind::Full, 9));
        assert_eq!(state.snapshot().session_id.as_deref(), Some("scan_1"));
    }

    #[test]
    fn a_targeted_rerun_replaces_exactly_one_row_and_keeps_the_rest() {
        let mut state = committed_quick_scan();
        let deferred = state.begin(
            ScanKind::Targeted,
            vec!["processor".to_string()],
            ScanPolicy {
                auto_save: false,
                ..policy()
            },
        );
        assert!(!deferred, "a targeted rerun defers invalidation to commit");
        assert!(state.has_overlay());
        assert!(state.session_started("scan_2".to_string(), ScanKind::Targeted, 1));
        // The base snapshot stays visible while the rerun is in flight.
        assert_eq!(state.snapshot().results.len(), 2);

        assert!(state.apply_result(row("scan_2", "processor", false), &catalog()));
        assert_eq!(state.snapshot().completed, 1);
        assert_eq!(state.snapshot().errors, 1);

        let outcome = state.finish_run(
            "scan_2",
            false,
            Ok(evidence("scan_2", &[("processor", false)])),
            &catalog(),
        );
        let RunOutcome::TargetedCommitted {
            session_id,
            task_id,
            evidence,
        } = outcome
        else {
            panic!("the rerun should commit into the base snapshot")
        };
        assert_eq!(session_id, "scan_1");
        assert_eq!(task_id, "processor");
        assert_eq!(evidence.len(), 2);
        assert_eq!(state.snapshot().results.len(), 2);
        assert!(!state.snapshot().results[1].success, "processor replaced");
        assert!(state.snapshot().results[0].success, "os_info untouched");
        // Every row keeps the committed session identity.
        assert!(
            state
                .snapshot()
                .results
                .iter()
                .all(|result| result.session_id == "scan_1")
        );
        assert!(!state.has_overlay());
    }

    #[test]
    fn a_failed_targeted_rerun_rolls_back_to_the_untouched_base() {
        let mut state = committed_quick_scan();
        state.begin(
            ScanKind::Targeted,
            vec!["processor".to_string()],
            ScanPolicy {
                auto_save: false,
                ..policy()
            },
        );
        assert!(state.session_started("scan_2".to_string(), ScanKind::Targeted, 1));
        let outcome = state.finish_run("scan_2", false, Err("boom".to_string()), &catalog());
        assert_eq!(
            outcome,
            RunOutcome::Failed {
                error: "boom".to_string(),
                stopped: false
            }
        );
        assert_eq!(state.snapshot().session_id.as_deref(), Some("scan_1"));
        assert_eq!(state.snapshot().results.len(), 2);
        assert!(state.snapshot().results.iter().all(|row| row.success));
        assert_eq!(state.phase(), ScanPhase::Idle);
    }

    #[test]
    fn an_incomplete_result_set_restores_the_previous_snapshot() {
        let mut state = committed_quick_scan();
        state.begin(
            ScanKind::Full,
            vec!["os_info".to_string(), "logical_disk".to_string()],
            policy(),
        );
        assert!(state.session_started("scan_3".to_string(), ScanKind::Full, 2));
        let outcome = state.finish_run(
            "scan_3",
            false,
            Ok(evidence("scan_3", &[("os_info", true)])),
            &catalog(),
        );
        assert_eq!(
            outcome,
            RunOutcome::Incomplete {
                completed: 1,
                expected: 2
            }
        );
        assert_eq!(state.snapshot().session_id.as_deref(), Some("scan_1"));
        assert_eq!(state.snapshot().results.len(), 2);
    }

    #[test]
    fn cancelling_restores_the_previous_snapshot_and_skips_auto_save() {
        let mut state = committed_quick_scan();
        state.begin(ScanKind::Full, vec!["os_info".to_string()], policy());
        assert!(state.session_started("scan_4".to_string(), ScanKind::Full, 1));
        assert!(state.request_cancel());
        assert_eq!(state.phase(), ScanPhase::Cancelling);
        let outcome = state.finish_run("scan_4", true, Err("cancelled".to_string()), &catalog());
        assert_eq!(outcome, RunOutcome::Cancelled);
        assert_eq!(state.snapshot().session_id.as_deref(), Some("scan_1"));
    }

    #[test]
    fn a_scan_that_completes_after_stop_commits_without_auto_saving() {
        let mut state = committed_quick_scan();
        state.begin(ScanKind::Full, vec!["os_info".to_string()], policy());
        assert!(state.session_started("scan_5".to_string(), ScanKind::Full, 1));
        assert!(state.request_cancel());
        let outcome = state.finish_run(
            "scan_5",
            false,
            Ok(evidence("scan_5", &[("os_info", true)])),
            &catalog(),
        );
        let RunOutcome::Committed { auto_save, .. } = outcome else {
            panic!("a natural completion that raced Stop still commits")
        };
        assert!(!auto_save, "a scan stopped by the user is not auto-saved");
        assert_eq!(state.phase(), ScanPhase::Idle);
    }

    #[test]
    fn an_overlay_refuses_to_open_without_a_committed_session() {
        assert!(
            TargetedOverlay::for_committed_session(
                ScanKind::Targeted,
                &["processor".to_string()],
                super::ScanSnapshot::default(),
            )
            .is_none()
        );
    }
}

/// The Store 2.5.8 Quick Scan task set.
pub const QUICK_SCAN_TASK_IDS: [&str; 17] = [
    "comp_system",
    "os_info",
    "processor",
    "physical_memory",
    "disk_drive",
    "logical_disk",
    "network_adapter",
    "systeminfo",
    "pending_reboot",
    "device_errors",
    "defender_status",
    "event_codes_critical",
    "services",
    "performance",
    "startup_command",
    "hosts_file",
    "firewall_status",
];

/// Cheap, non-admin issue sources always unioned into a customised Quick Scan
/// so issue detection keeps working when the user trims the task list.
pub const QUICK_DETECTION_SOURCE_TASK_IDS: [&str; 11] = [
    "logical_disk",
    "network_adapter",
    "pending_reboot",
    "device_errors",
    "defender_status",
    "event_codes_critical",
    "services",
    "performance",
    "startup_command",
    "hosts_file",
    "firewall_status",
];

/// Choose the tasks one scan runs.
///
/// A full or targeted scan drops admin-only tasks when the process is not
/// elevated. A quick scan uses the user's custom set when they have one, always
/// unioned with the cheap issue sources.
#[must_use]
pub fn select_scan_tasks(
    catalog: &[DiagnosticTask],
    scan_kind: ScanKind,
    is_admin: bool,
    custom_quick_tasks: Option<&[String]>,
) -> Vec<String> {
    let custom_quick_tasks = custom_quick_tasks.filter(|tasks| !tasks.is_empty());
    catalog
        .iter()
        .filter(|task| match scan_kind {
            ScanKind::Quick => custom_quick_tasks.map_or_else(
                || QUICK_SCAN_TASK_IDS.contains(&task.id.as_str()),
                |tasks| {
                    tasks.iter().any(|task_id| task_id == &task.id)
                        || QUICK_DETECTION_SOURCE_TASK_IDS.contains(&task.id.as_str())
                },
            ),
            ScanKind::Full | ScanKind::Targeted => is_admin || !task.admin_required,
        })
        .map(|task| task.id.clone())
        .collect()
}

#[cfg(test)]
mod selection_tests {
    use super::select_scan_tasks;
    use wfdiag_native_diagnostics::{DiagnosticTask, ScanKind};

    fn task(id: &str, admin_required: bool) -> DiagnosticTask {
        DiagnosticTask {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            category: "Test".to_string(),
            admin_required,
        }
    }

    #[test]
    fn a_full_scan_drops_admin_tasks_without_elevation() {
        let catalog = vec![task("os_info", false), task("bsod_dumps", true)];
        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Full, false, None),
            ["os_info"]
        );
        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Full, true, None).len(),
            2
        );
    }

    #[test]
    fn a_custom_quick_scan_still_includes_the_issue_sources() {
        let catalog = vec![
            task("os_info", false),
            task("logical_disk", false),
            task("processor", false),
        ];
        let custom = vec!["processor".to_string()];
        let selected = select_scan_tasks(&catalog, ScanKind::Quick, false, Some(&custom));
        assert!(selected.contains(&"processor".to_string()));
        assert!(
            selected.contains(&"logical_disk".to_string()),
            "cheap issue sources are always unioned in"
        );
        assert!(!selected.contains(&"os_info".to_string()));
    }

    #[test]
    fn the_default_quick_scan_uses_the_shipping_task_set() {
        let catalog = vec![task("os_info", false), task("unlisted", false)];
        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Quick, true, None),
            ["os_info"]
        );
    }
}
