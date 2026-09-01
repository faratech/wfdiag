//! Issue detection orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::state::Page;
use crate::app::tasks::{spawn_issue_request_preparation, spawn_issue_wait};
use std::sync::Arc;
use wfdiag_native_diagnostics::SharedScanEvidence;
use wfdiag_native_issues::IssueDetectionCompleted;
use wfdiag_native_issues::projection::{
    PendingIssueDetection, PreparedIssueDetection, advance_nonzero_generation,
    pending_issue_preparation_is_current, project_issues, take_current_issue_completion,
};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn resume_issue_wait(&mut self, context: &ComponentContext<Self>) {
        let has_enqueued_current_request = self
            .issue_pending
            .as_ref()
            .is_some_and(|pending| self.issue_enqueued_request_id == Some(pending.request_id));
        if self.issue_wait.is_some() || !has_enqueued_current_request {
            return;
        }
        let Some(receiver) = self.issue_receiver.as_ref().map(Arc::clone) else {
            self.issue_wait = None;
            return;
        };
        self.issue_wait = Some(spawn_issue_wait(context, receiver));
    }

    pub(crate) fn stop_issue_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if let Some(task) = self.issue_wait.take() {
            task.cancel();
        }
        if let Some(task) = self.issue_prepare_task.take() {
            task.cancel();
        }
        self.issue_pending = None;
        self.issue_enqueued_request_id = None;
        self.issue_receiver = None;
        self.issue_runtime = None;
        self.issue_error = Some(reason.clone());
        if self.page == Page::Issues {
            self.status = reason;
        }
    }

    /// Re-run issue detection against the exact last committed native result
    /// map. The committed epoch changes only in `commit_issue_evidence`; a
    /// refresh advances the request id but deliberately retains that epoch.
    pub(crate) fn request_issue_detection(&mut self, context: &ComponentContext<Self>) -> bool {
        // Deterministic screenshots must not start the worker, read the clock,
        // or enumerate the user's temp directory.
        if self.deterministic_visual {
            return false;
        }
        let Some(session_id) = self.issue_source_session_id.clone() else {
            return false;
        };
        let Some(results) = self.issue_source_results.as_ref().map(Arc::clone) else {
            return false;
        };
        if self.issue_runtime.is_none() {
            if self.issue_error.is_none() {
                self.issue_error = Some("Native issue detection is unavailable".to_string());
            }
            return false;
        }

        let Some(request_id) = advance_nonzero_generation(&mut self.issue_request_id) else {
            self.stop_issue_delivery("Native issue request identity was exhausted");
            return false;
        };
        let pending = PendingIssueDetection {
            request_id,
            committed_epoch: self.issue_committed_epoch,
            session_id,
        };
        let superseded_prepare = self.issue_prepare_task.take();
        self.issue_pending = Some(pending.clone());
        self.issue_enqueued_request_id = None;
        self.issue_error = None;
        if let Some(task) = superseded_prepare {
            // The new guard is installed first, so a late cancellation message
            // from the old task cannot clear this request.
            task.cancel();
        }
        self.issue_prepare_task = Some(spawn_issue_request_preparation(context, pending, results));
        true
    }

    pub(crate) fn issue_preparation_is_current(&self, pending: &PendingIssueDetection) -> bool {
        pending_issue_preparation_is_current(
            self.issue_pending.as_ref(),
            pending,
            self.issue_committed_epoch,
            self.issue_source_session_id.as_deref(),
        )
    }

    pub(crate) fn apply_prepared_issue_request(
        &mut self,
        prepared: PreparedIssueDetection,
        context: &ComponentContext<Self>,
    ) {
        if !self.issue_preparation_is_current(&prepared.pending) {
            return;
        }
        self.issue_prepare_task = None;
        let request_id = prepared.pending.request_id;
        let enqueue_result = self
            .issue_runtime
            .as_ref()
            .ok_or_else(|| "native issue worker is unavailable".to_string())
            .and_then(|runtime| {
                runtime
                    .enqueue(prepared.request)
                    .map_err(|error| error.to_string())
            });
        match enqueue_result {
            Ok(()) => {
                self.issue_enqueued_request_id = Some(request_id);
                self.issue_error = None;
                self.resume_issue_wait(context);
            }
            Err(error) => {
                self.stop_issue_delivery(format!("Native issue detection stopped · {error}"));
            }
        }
    }

    pub(crate) fn apply_issue_preparation_failure(
        &mut self,
        pending: PendingIssueDetection,
        reason: &'static str,
    ) {
        if !self.issue_preparation_is_current(&pending) {
            return;
        }
        self.issue_prepare_task = None;
        self.issue_pending = None;
        self.issue_enqueued_request_id = None;
        self.issue_error = Some(reason.to_string());
        if self.page == Page::Issues {
            self.status = reason.to_string();
        }
    }

    /// Commit one complete authoritative diagnostic snapshot and immediately
    /// queue issue detection, before any optional history-save branch.
    pub(crate) fn commit_issue_evidence(
        &mut self,
        session_id: String,
        results: SharedScanEvidence,
        context: &ComponentContext<Self>,
    ) {
        let Some(committed_epoch) = advance_nonzero_generation(&mut self.issue_committed_epoch)
        else {
            self.stop_issue_delivery("Native issue evidence identity was exhausted");
            return;
        };
        debug_assert_eq!(committed_epoch, self.issue_committed_epoch);
        self.invalidate_issue_prioritization();
        self.issue_source_session_id = Some(session_id);
        self.issue_source_results = Some(results);
        self.reconcile_staged_action_reviews();
        // Keep the last successfully projected issues visible until the new
        // guarded worker response succeeds. Preparation/enqueue/delivery
        // failures therefore cannot blank previously useful evidence.
        self.issue_error = None;
        let _ = self.request_issue_detection(context);
    }

    pub(crate) fn apply_issue_completion(
        &mut self,
        completion: IssueDetectionCompleted,
        context: &ComponentContext<Self>,
    ) {
        self.issue_wait = None;
        let current_session_id = self.issue_source_session_id.clone();
        if take_current_issue_completion(
            &mut self.issue_pending,
            &completion,
            self.issue_committed_epoch,
            current_session_id.as_deref(),
        )
        .is_some()
        {
            self.issue_enqueued_request_id = None;
            self.invalidate_issue_prioritization();
            self.issues = completion.issues;
            self.invalidate_fix_plan();
            self.reconcile_staged_action_reviews();
            self.issue_projected_epoch = Some(self.issue_committed_epoch);
            self.issue_projected_session_id = current_session_id;
            self.issue_error = None;
            if self.page == Page::Issues && !self.diagnostics_busy() {
                self.status = project_issues(&self.issues).counts.summary_text();
            }
        }
        // A stale response can precede a newer queued response. Its guard must
        // remain pending and delivery must continue until that response lands.
        self.resume_issue_wait(context);
    }
}
