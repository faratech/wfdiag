//! Diagnostic scan orchestration: start, cancel, snapshot, and finalize.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::policy::{
    build_history_scan_record, diagnostic_output_snapshot, scan_kind_label,
    scan_policy_requests_auto_save, select_scan_tasks, shared_diagnostic_executor,
    system_identity_blocks_scan, take_startup_scan_when_ready,
};
use crate::app::state::{DiagnosticScanPolicy, DiagnosticSnapshot, TargetedDiagnosticOverlay};
use crate::app::tasks::{spawn_diagnostic_finalization_delay, spawn_history_save_wait};
use crate::platform::notifications;
use std::sync::Arc;
use std::time::Instant;
use wfdiag_native_diagnostics::{ScanEvidence, ScanKind};
use wfdiag_ui_core::{DiagnosticTaskResult, TaskProgressStatus, UiEvent};
use windows_reactor::*;

impl WfdiagShell {
    /// Queue the scan-completion toast, mirroring the shipping plugin's
    /// behavior when notifications are enabled.
    ///
    /// #206: the toast runs on one shared worker thread instead of a detached
    /// thread per scan, and its failure is no longer swallowed — the first one
    /// (from this dispatch or from an earlier queued toast) is reported once in
    /// the status line so a silently missing notification is explainable.
    pub(crate) fn notify_scan_completion(&mut self) {
        if self.deterministic_visual || !self.settings_snapshot.show_notifications {
            return;
        }
        let collected = self.diagnostic_results.len();
        let errors = self
            .diagnostic_results
            .iter()
            .filter(|result| !result.success)
            .count();
        if let Err(error) = notifications::request_scan_complete_toast(collected, errors) {
            self.report_notification_failure(error);
        }
    }

    /// Surface at most one notification failure for the session.
    ///
    /// Repeats are almost always the same cause, and a status line that keeps
    /// re-announcing a best-effort toast failure is worse than one that says
    /// it once.
    pub(crate) fn report_notification_failure(&mut self, error: String) {
        if self.notification_failure_reported {
            return;
        }
        self.notification_failure_reported = true;
        self.status = format!("Notification not shown · {error}");
    }

    pub(crate) fn maybe_begin_startup_scan(&mut self, context: &ComponentContext<Self>) {
        if take_startup_scan_when_ready(
            &mut self.startup_scan_gate,
            self.deterministic_visual,
            self.settings_loading,
            self.system_info_request_id,
            self.architecture_request_id,
        ) {
            self.begin_diagnostic_scan(ScanKind::Quick, context);
        }
    }

    pub(crate) fn invalidate_derived_diagnostic_content(&mut self) {
        self.invalidate_report_for_new_scan();
        self.invalidate_fix_plan();
        self.invalidate_issue_prioritization();
        if let Some(pending) = self.analysis_pending.take()
            && let Some(runtime) = self.analysis_runtime.as_ref()
        {
            let _ = runtime.cancel(pending.request_id);
        }
        self.diagnostic_analyses.clear();
    }

    pub(crate) fn diagnostics_busy(&self) -> bool {
        self.diagnostic_starting
            || self.diagnostic_running
            || self.diagnostic_cancelling
            || self.diagnostic_finalizing
    }

    pub(crate) fn update_diagnostic_counts(&mut self) {
        if let Some(overlay) = self.targeted_diagnostic_overlay.as_ref() {
            (self.diagnostic_completed, self.diagnostic_errors) = overlay.staged_counts();
        } else {
            self.diagnostic_completed = self.diagnostic_results.len();
            self.diagnostic_errors = self
                .diagnostic_results
                .iter()
                .filter(|result| !result.success)
                .count();
        }
        if let Some(started) = self.diagnostic_scan_start {
            self.diagnostic_duration_ms = started.elapsed().as_millis() as u64;
        }
    }

    pub(crate) fn capture_diagnostic_snapshot(&self) -> DiagnosticSnapshot {
        let task_ids = if self.diagnostic_expected_task_ids.is_empty() {
            self.diagnostic_results
                .iter()
                .map(|result| result.task_id.clone())
                .collect()
        } else {
            self.diagnostic_expected_task_ids.clone()
        };
        DiagnosticSnapshot {
            results: self.diagnostic_results.clone(),
            scan_kind: self.diagnostic_scan_kind,
            task_ids,
            session_id: self.diagnostic_session_id.clone().or_else(|| {
                self.diagnostic_results
                    .first()
                    .map(|result| result.session_id.clone())
            }),
            duration_ms: self.diagnostic_duration_ms,
            total: self.diagnostic_total,
            completed: self.diagnostic_completed,
            errors: self.diagnostic_errors,
        }
    }

    pub(crate) fn apply_diagnostic_snapshot(&mut self, snapshot: DiagnosticSnapshot) {
        self.diagnostic_results = snapshot.results;
        self.diagnostic_scan_kind = snapshot.scan_kind;
        self.diagnostic_expected_task_ids = snapshot.task_ids;
        self.diagnostic_session_id = snapshot.session_id;
        self.diagnostic_duration_ms = snapshot.duration_ms;
        self.diagnostic_total = snapshot.total;
        self.diagnostic_completed = snapshot.completed;
        self.diagnostic_errors = snapshot.errors;
    }

    pub(crate) fn restore_previous_diagnostics(&mut self) {
        let previous = self
            .targeted_diagnostic_overlay
            .take()
            .map(TargetedDiagnosticOverlay::rollback)
            .or_else(|| self.previous_diagnostic_snapshot.take());
        if let Some(previous) = previous {
            self.apply_diagnostic_snapshot(previous);
        } else {
            self.diagnostic_results.clear();
            self.diagnostic_scan_kind = None;
            self.diagnostic_expected_task_ids.clear();
            self.diagnostic_session_id = None;
            self.diagnostic_duration_ms = 0;
            self.diagnostic_total = 0;
            self.diagnostic_completed = 0;
            self.diagnostic_errors = 0;
        }
    }

    pub(crate) fn commit_targeted_diagnostic_overlay(
        &mut self,
        transaction_session_id: &str,
        authoritative_results: &ScanEvidence,
        context: &ComponentContext<Self>,
    ) -> Result<(), String> {
        let overlay = self
            .targeted_diagnostic_overlay
            .as_ref()
            .ok_or_else(|| "targeted rerun transaction was unavailable".to_string())?;
        let output = authoritative_results
            .get(&overlay.target_task_id)
            .ok_or_else(|| format!("targeted rerun did not return `{}`", overlay.target_task_id))?;
        let replacement = DiagnosticTaskResult::new(
            transaction_session_id,
            overlay.target_task_id.clone(),
            Arc::clone(output),
        );
        let committed = overlay.commit(replacement, &self.diagnostic_catalog)?;
        let evidence_session_id = committed
            .session_id
            .clone()
            .or_else(|| {
                committed
                    .results
                    .first()
                    .map(|result| result.session_id.clone())
            })
            .unwrap_or_else(|| transaction_session_id.to_string());
        let issue_results = diagnostic_output_snapshot(&committed.results);

        self.invalidate_derived_diagnostic_content();
        self.targeted_diagnostic_overlay = None;
        self.previous_diagnostic_snapshot = None;
        self.apply_diagnostic_snapshot(committed);
        self.reset_diagnostic_activity();
        self.commit_issue_evidence(evidence_session_id, issue_results, context);
        self.status = format!(
            "Diagnostic rerun complete · {} collected · {} errors",
            self.diagnostic_completed, self.diagnostic_errors
        );
        self.resume_pending_ai_intent(context);
        Ok(())
    }

    pub(crate) fn reset_diagnostic_activity(&mut self) {
        if let Some(task) = self.diagnostic_finalization_task.take() {
            task.cancel();
        }
        if let Some(task) = self.diagnostic_history_save_task.take() {
            task.cancel();
        }
        self.diagnostic_starting = false;
        self.diagnostic_running = false;
        self.diagnostic_cancelling = false;
        self.diagnostic_finalizing = false;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_start_task = None;
        self.diagnostic_run_task = None;
        self.diagnostic_cancel_task = None;
        self.diagnostic_scan_policy = None;
        self.targeted_diagnostic_overlay = None;
        self.diagnostic_task_statuses.clear();
        self.diagnostic_current_task = None;
        self.diagnostic_scan_start = None;
    }

    pub(crate) fn finish_completed_diagnostic_scan(
        &mut self,
        history_error: Option<String>,
        context: &ComponentContext<Self>,
    ) {
        let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
        self.previous_diagnostic_snapshot = None;
        self.reset_diagnostic_activity();
        self.notify_scan_completion();
        if let Some(error) = history_error {
            self.history_error = Some(format!("Scan history was not saved: {error}"));
            self.status = format!(
                "{label} complete · {} collected · {} errors · history not saved",
                self.diagnostic_completed, self.diagnostic_errors
            );
        } else {
            self.status = format!(
                "{label} complete · {} collected · {} errors",
                self.diagnostic_completed, self.diagnostic_errors
            );
        }
        self.resume_pending_ai_intent(context);
    }

    pub(crate) fn begin_completed_diagnostic_finalization(
        &mut self,
        session_id: String,
        context: &ComponentContext<Self>,
    ) {
        self.previous_diagnostic_snapshot = None;
        self.diagnostic_running = false;
        let committed_after_stop = self.diagnostic_cancel_requested || self.diagnostic_cancelling;
        self.diagnostic_cancelling = false;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_run_task = None;
        self.diagnostic_current_task = None;

        let auto_save = scan_policy_requests_auto_save(self.diagnostic_scan_policy.as_ref());
        if !auto_save || committed_after_stop {
            self.finish_completed_diagnostic_scan(None, context);
            return;
        }

        self.diagnostic_finalizing = true;
        self.status = format!(
            "Finalizing {}…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        self.diagnostic_finalization_task =
            Some(spawn_diagnostic_finalization_delay(context, session_id));
    }

    pub(crate) fn begin_completed_scan_history_save(
        &mut self,
        session_id: String,
        context: &ComponentContext<Self>,
    ) {
        self.diagnostic_finalization_task = None;
        if !self.diagnostic_finalizing || self.diagnostic_session_id.as_deref() != Some(&session_id)
        {
            return;
        }

        let Some(history_tag) = self
            .diagnostic_scan_policy
            .as_ref()
            .map(|policy| policy.history_tag.clone())
        else {
            self.finish_completed_diagnostic_scan(
                Some("the scan-start history policy was unavailable".to_string()),
                context,
            );
            return;
        };
        self.update_diagnostic_counts();
        let scan = build_history_scan_record(
            session_id.clone(),
            &self.system_info,
            &self.diagnostic_results,
            self.diagnostic_duration_ms,
            history_tag,
        );
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.finish_completed_diagnostic_scan(
                Some("native scan history is unavailable".to_string()),
                context,
            );
            return;
        };
        let reply = match runtime.request_save(scan) {
            Ok(reply) => reply,
            Err(error) => {
                self.finish_completed_diagnostic_scan(Some(error.to_string()), context);
                return;
            }
        };

        self.status = format!(
            "Saving {} history…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        self.diagnostic_history_save_task =
            Some(spawn_history_save_wait(context, session_id, reply));
    }

    pub(crate) fn begin_diagnostic_scan(
        &mut self,
        scan_kind: ScanKind,
        context: &ComponentContext<Self>,
    ) {
        let task_ids = select_scan_tasks(
            &self.diagnostic_catalog,
            scan_kind,
            self.is_admin,
            self.settings_snapshot.quick_scan_tasks.as_deref(),
        );
        self.begin_diagnostic_scan_with_tasks(scan_kind, task_ids, context);
    }

    pub(crate) fn begin_targeted_diagnostic_scan(
        &mut self,
        task_id: &str,
        context: &ComponentContext<Self>,
    ) {
        let Some(task) = self
            .diagnostic_catalog
            .iter()
            .find(|task| task.id == task_id)
        else {
            self.status = "That diagnostic task is no longer available".to_string();
            return;
        };
        if task.admin_required && !self.is_admin {
            self.status = format!("{} requires administrator access", task.name);
            return;
        }
        self.begin_diagnostic_scan_with_tasks(ScanKind::Targeted, vec![task.id.clone()], context);
    }

    pub(crate) fn begin_diagnostic_scan_with_tasks(
        &mut self,
        scan_kind: ScanKind,
        task_ids: Vec<String>,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual {
            // Screenshot fixtures must never launch WMI, commands, or mutate the
            // captured Store 2.5.8 state.
            self.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        if self.diagnostics_busy() {
            let label = self
                .diagnostic_scan_kind
                .map_or("Diagnostic scan", scan_kind_label);
            self.status = format!("{label} is already running");
            return;
        }
        if system_identity_blocks_scan(self.deterministic_visual, self.system_info_request_id) {
            self.status = "Detecting administrator access…".to_string();
            return;
        }
        if self.settings_loading {
            self.status = "Loading scan settings…".to_string();
            return;
        }

        let Some(runtime) = self.diagnostic_runtime.clone() else {
            self.status = "Native diagnostics are unavailable".to_string();
            return;
        };
        if task_ids.is_empty() {
            self.status = format!("{} has no available tasks", scan_kind_label(scan_kind));
            return;
        }

        let previous = self.capture_diagnostic_snapshot();
        self.previous_diagnostic_snapshot = None;
        self.targeted_diagnostic_overlay = TargetedDiagnosticOverlay::for_committed_session(
            scan_kind,
            &task_ids,
            previous.clone(),
        );
        let policy = DiagnosticScanPolicy::snapshot(
            &self.settings_snapshot,
            scan_kind,
            self.targeted_diagnostic_overlay.is_some(),
        );
        if self.targeted_diagnostic_overlay.is_none() {
            // Replacement scans invalidate derived content as soon as their
            // transaction starts. A targeted rerun defers that invalidation
            // until its single authoritative replacement commits so every
            // failure path leaves the prior evidence and projections intact.
            self.invalidate_derived_diagnostic_content();
            self.previous_diagnostic_snapshot = Some(previous);
        }
        let task_count = task_ids.len();
        self.diagnostic_expected_task_ids = task_ids.clone();
        self.diagnostic_task_statuses = task_ids
            .iter()
            .cloned()
            .map(|task_id| (task_id, TaskProgressStatus::Queued))
            .collect();
        self.diagnostic_scan_kind = Some(scan_kind);
        self.diagnostic_scan_policy = Some(policy);
        self.diagnostic_total = task_count;
        self.diagnostic_completed = 0;
        self.diagnostic_errors = 0;
        self.diagnostic_duration_ms = 0;
        self.diagnostic_current_task = None;
        self.diagnostic_starting = true;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_scan_start = Some(Instant::now());
        self.status = format!("Starting {}…", scan_kind_label(scan_kind));

        self.diagnostic_start_task = Some(context.spawn_background_with_rejection(
            move |_| match shared_diagnostic_executor() {
                Ok(executor) => match executor.block_on(runtime.start_session(task_ids, scan_kind))
                {
                    Ok(session_id) => Message::DiagnosticSessionStarted {
                        session_id,
                        scan_kind,
                        task_count,
                    },
                    Err(error) => Message::DiagnosticSessionStartFailed {
                        error: error.to_string(),
                    },
                },
                Err(error) => Message::DiagnosticSessionStartFailed { error },
            },
            Message::DiagnosticSessionStartFailed {
                error: "the Reactor background queue rejected the scan request".to_string(),
            },
        ));
    }

    pub(crate) fn launch_diagnostic_run(
        &mut self,
        session_id: String,
        context: &ComponentContext<Self>,
    ) {
        let Some(runtime) = self.diagnostic_runtime.clone() else {
            self.restore_previous_diagnostics();
            self.reset_diagnostic_activity();
            self.status = "Native diagnostics stopped before the scan began".to_string();
            return;
        };
        let Some(max_concurrent_tasks) = self
            .diagnostic_scan_policy
            .as_ref()
            .map(|policy| policy.max_concurrent_tasks)
        else {
            self.restore_previous_diagnostics();
            self.reset_diagnostic_activity();
            self.status = "Scan settings were unavailable after startup".to_string();
            return;
        };
        let run_session_id = session_id.clone();
        self.diagnostic_run_task = Some(context.spawn_background_with_rejection(
            move |_| {
                match shared_diagnostic_executor() {
                    Ok(executor) => match executor
                        .block_on(runtime.run_session(run_session_id.clone(), max_concurrent_tasks))
                    {
                        Ok(result) if result.cancelled => Message::DiagnosticRunFinished {
                            session_id: result.session_id,
                            cancelled: true,
                            authoritative_results: Err("scan was cancelled".to_string()),
                        },
                        Ok(result) => Message::DiagnosticRunFinished {
                            session_id: result.session_id,
                            cancelled: false,
                            authoritative_results: Ok(result.evidence),
                        },
                        Err(error) => Message::DiagnosticRunFinished {
                            session_id: run_session_id,
                            cancelled: false,
                            authoritative_results: Err(error.to_string()),
                        },
                    },
                    Err(error) => Message::DiagnosticRunFinished {
                        session_id: run_session_id,
                        cancelled: false,
                        authoritative_results: Err(error),
                    },
                }
            },
            Message::DiagnosticRunRejected,
        ));
    }

    pub(crate) fn request_diagnostic_cancel(&mut self, context: &ComponentContext<Self>) {
        if self.diagnostic_starting {
            self.diagnostic_cancel_requested = true;
            self.status = "Stopping scan as soon as startup completes…".to_string();
            return;
        }
        if self.diagnostic_finalizing {
            if let Some(task) = self.diagnostic_finalization_task.take() {
                task.cancel();
                self.finish_completed_diagnostic_scan(None, context);
            }
            return;
        }
        if !self.diagnostic_running || self.diagnostic_cancelling {
            return;
        }
        let (Some(runtime), Some(session_id)) = (
            self.diagnostic_runtime.clone(),
            self.diagnostic_session_id.clone(),
        ) else {
            self.status = "The active diagnostic session could not be identified".to_string();
            return;
        };

        self.diagnostic_cancelling = true;
        self.diagnostic_cancel_requested = true;
        self.status = format!(
            "Stopping {} after in-flight checks finish…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        let cancel_session_id = session_id.clone();
        let rejection_session_id = session_id.clone();
        self.diagnostic_cancel_task = Some(context.spawn_background_with_rejection(
            move |_| match shared_diagnostic_executor() {
                Ok(executor) => {
                    let error = executor
                        .block_on(runtime.cancel_session(&cancel_session_id))
                        .err()
                        .map(|error| error.to_string());
                    Message::DiagnosticCancelFinished {
                        session_id: cancel_session_id,
                        error,
                    }
                }
                Err(error) => Message::DiagnosticCancelFinished {
                    session_id: cancel_session_id,
                    error: Some(error),
                },
            },
            Message::DiagnosticCancelRejected {
                session_id: rejection_session_id,
            },
        ));
    }

    pub(crate) fn apply_diagnostic_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::TaskProgress(progress) => {
                if self.diagnostic_session_id.as_deref() != Some(&progress.session_id) {
                    return;
                }
                let task_id = progress.task_id.clone();
                let task_status = progress.status;
                self.diagnostic_task_statuses
                    .insert(task_id.clone(), task_status);
                let task_name = progress.task_name.unwrap_or_else(|| {
                    self.diagnostic_catalog
                        .iter()
                        .find(|task| task.id == task_id)
                        .map_or_else(|| task_id.clone(), |task| task.name.clone())
                });
                if task_status == TaskProgressStatus::Running {
                    self.diagnostic_current_task = Some(task_name.clone());
                }
                self.update_diagnostic_counts();
                if self.diagnostic_running {
                    let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                    self.status = if self.diagnostic_cancelling {
                        format!("Stopping {label} · {task_name}")
                    } else {
                        format!(
                            "{label} · {} of {} collected · {task_name}",
                            self.diagnostic_completed, self.diagnostic_total
                        )
                    };
                }
            }
            UiEvent::DiagnosticResult(result) => {
                if self.diagnostic_session_id.as_deref() != Some(&result.session_id) {
                    return;
                }
                self.diagnostic_task_statuses.insert(
                    result.task_id.clone(),
                    if result.success {
                        TaskProgressStatus::Completed
                    } else {
                        TaskProgressStatus::Failed
                    },
                );
                if let Some(overlay) = self.targeted_diagnostic_overlay.as_mut() {
                    overlay.stage(result);
                    self.update_diagnostic_counts();
                    let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                    self.status = format!(
                        "{label} · {} of {} collected · {} errors",
                        self.diagnostic_completed, self.diagnostic_total, self.diagnostic_errors
                    );
                    return;
                }
                if let Some(existing) = self
                    .diagnostic_results
                    .iter_mut()
                    .find(|existing| existing.task_id == result.task_id)
                {
                    *existing = result;
                } else {
                    self.diagnostic_results.push(result);
                }
                // Catalog order, computed once per event instead of one
                // linear catalog scan per result inside the key closure.
                let catalog_order: std::collections::HashMap<&str, usize> = self
                    .diagnostic_catalog
                    .iter()
                    .enumerate()
                    .map(|(index, task)| (task.id.as_str(), index))
                    .collect();
                self.diagnostic_results.sort_by_key(|result| {
                    catalog_order
                        .get(result.task_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                });
                self.update_diagnostic_counts();
                let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                self.status = if self.diagnostic_running {
                    format!(
                        "{label} · {} of {} collected · {} errors",
                        self.diagnostic_completed, self.diagnostic_total, self.diagnostic_errors
                    )
                } else {
                    format!(
                        "{label} complete · {} collected · {} errors",
                        self.diagnostic_completed, self.diagnostic_errors
                    )
                };
            }
            _ => {}
        }
    }
}
