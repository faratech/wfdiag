//! Remediation action orchestration: proposal, approval, and runs.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::ActionReviewSurface;
use crate::app::policy::{action_proposal_matches_snapshot, action_run_status_text};
use crate::app::state::{FixPlanActionSelection, PendingActionApproval};
use crate::app::tasks::spawn_relaunch_as_admin;
use crate::fixtures::visual::LiveTestFixture;
use std::collections::HashSet;
use wfdiag_native_issues::projection::advance_nonzero_generation;
use wfdiag_native_remediation::broker::{
    ActionApproval, ActionPrepareInput, ActionProposal, ActionRequest, ActionSnapshot,
    DetectedIssueRemediation, current_action_catalog_fingerprint,
};
use wfdiag_native_remediation::runtime::ActionRunSummary;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn action_snapshot(&self) -> ActionSnapshot {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.issue_source_session_id.hash(&mut hasher);
        self.issue_committed_epoch.hash(&mut hasher);
        let mut results = self.diagnostic_results.iter().collect::<Vec<_>>();
        results.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        for result in results {
            result.session_id.hash(&mut hasher);
            result.task_id.hash(&mut hasher);
            result.success.hash(&mut hasher);
            result.output.hash(&mut hasher);
            result.error.hash(&mut hasher);
        }
        let scan_fingerprint = format!(
            "{}:{:016x}",
            self.issue_source_session_id.as_deref().unwrap_or("no-scan"),
            hasher.finish()
        );
        ActionSnapshot {
            scan_fingerprint,
            catalog_fingerprint: current_action_catalog_fingerprint(),
            detected_issues: self
                .issues
                .iter()
                .filter(|issue| issue.status == wfdiag_native_issues::IssueStatus::Detected)
                .map(|issue| DetectedIssueRemediation {
                    issue_id: issue.id.clone(),
                    remediation_id: issue
                        .remediation
                        .as_ref()
                        .map(|remediation| remediation.id.clone()),
                })
                .collect(),
            is_admin: self.is_admin,
        }
    }

    pub(crate) fn restore_action_approval(&mut self, pending: PendingActionApproval) {
        match pending.return_surface {
            ActionReviewSurface::Review => self.action_review = Some(pending.proposal),
            ActionReviewSurface::RepairConfirmation => {
                self.repair_confirm = Some(pending.proposal);
            }
        }
    }

    pub(crate) fn restore_action_approval_if_current(
        &mut self,
        pending: PendingActionApproval,
    ) -> bool {
        let snapshot = self.action_snapshot();
        let broker_still_has_proposal = self.action_runtime.as_ref().is_some_and(|runtime| {
            runtime
                .list_pending_proposals()
                .iter()
                .any(|proposal| proposal.proposal_id == pending.proposal.proposal_id)
        });
        if broker_still_has_proposal
            && action_proposal_matches_snapshot(&pending.proposal, &snapshot)
        {
            self.restore_action_approval(pending);
            true
        } else {
            if broker_still_has_proposal && let Some(runtime) = self.action_runtime.as_ref() {
                let _ = runtime.discard(pending.proposal.proposal_id);
            }
            false
        }
    }

    pub(crate) fn discard_action_proposal_or_restore(&mut self, pending: PendingActionApproval) {
        let discarded = self
            .action_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.discard(pending.proposal.proposal_id.clone()));
        if !discarded {
            self.restore_action_approval(pending);
            self.status =
                "Could not discard the staged action · native remediation broker unavailable"
                    .to_string();
        }
    }

    pub(crate) fn reconcile_staged_action_reviews(&mut self) {
        let snapshot = self.action_snapshot();
        let stale_review = self
            .action_review
            .as_ref()
            .is_some_and(|proposal| !action_proposal_matches_snapshot(proposal, &snapshot));
        let stale_repair = self
            .repair_confirm
            .as_ref()
            .is_some_and(|proposal| !action_proposal_matches_snapshot(proposal, &snapshot));
        let stale = [
            stale_review.then(|| self.action_review.take()).flatten(),
            stale_repair.then(|| self.repair_confirm.take()).flatten(),
        ];
        if let Some(runtime) = self.action_runtime.as_ref() {
            for proposal in stale.into_iter().flatten() {
                let _ = runtime.discard(proposal.proposal_id);
            }
        }
    }

    pub(crate) fn request_admin_relaunch(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual
            && self.live_test_fixture != Some(LiveTestFixture::AdminRelaunch)
        {
            self.status = "Visual fixture mode · elevation is disabled".to_string();
            return;
        }
        if self.admin_relaunch_task.is_none() {
            self.admin_relaunch_task = Some(spawn_relaunch_as_admin(context));
            self.status = "Restarting with administrator rights…".to_string();
        }
    }

    /// Stage an immutable, catalog-derived action preview. No catalog entry
    /// can execute until the user approves the opaque proposal ID returned by
    /// the worker and the worker revalidates a fresh authoritative snapshot.
    pub(crate) fn prepare_remediation(
        &mut self,
        remediation_id: String,
        issue_id: Option<String>,
        context: &ComponentContext<Self>,
    ) {
        let snapshot = self.action_snapshot();
        self.prepare_action_selection(
            FixPlanActionSelection {
                actions: vec![ActionRequest {
                    remediation_id,
                    issue_id,
                }],
                expected_scan_fingerprint: snapshot.scan_fingerprint,
                expected_catalog_fingerprint: snapshot.catalog_fingerprint,
            },
            context,
        );
    }

    pub(crate) fn prepare_action_selection(
        &mut self,
        selection: FixPlanActionSelection,
        context: &ComponentContext<Self>,
    ) {
        if self
            .live_test_fixture
            .is_some_and(|fixture| !fixture.permits_actions(&selection.actions))
        {
            self.status = "The validation fixture rejected an action outside its closed allowlist"
                .to_string();
            return;
        }
        if self.action_pending.is_some() {
            self.status = "A remediation request is already active…".to_string();
            return;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.action_request_id) else {
            self.status = "Native remediation request identity was exhausted".to_string();
            return;
        };
        let snapshot = self.action_snapshot();
        let Some(runtime) = self.action_runtime.as_ref() else {
            self.status = "Native remediation is unavailable".to_string();
            return;
        };
        let action_count = selection.actions.len();
        let input = ActionPrepareInput {
            actions: selection.actions,
            expected_scan_fingerprint: Some(selection.expected_scan_fingerprint),
            expected_catalog_fingerprint: Some(selection.expected_catalog_fingerprint),
        };
        if !runtime.prepare(request_id, input, snapshot) {
            self.status = "The native remediation preview queue is unavailable".to_string();
            return;
        }
        self.action_pending = Some(request_id);
        self.action_pending_approval = None;
        self.status = format!(
            "Preparing a review for {action_count} vetted action{}…",
            if action_count == 1 { "" } else { "s" }
        );
        self.resume_action_wait(context);
    }

    pub(crate) fn approve_action_proposal(
        &mut self,
        proposal: ActionProposal,
        approval: ActionApproval,
        return_surface: ActionReviewSurface,
        context: &ComponentContext<Self>,
    ) {
        let pending = PendingActionApproval {
            proposal,
            return_surface,
        };
        if self.action_pending.is_some() {
            self.restore_action_approval(pending);
            self.status = "A remediation request is already active…".to_string();
            return;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.action_request_id) else {
            self.restore_action_approval(pending);
            self.status = "Native remediation request identity was exhausted".to_string();
            return;
        };
        let snapshot = self.action_snapshot();
        let Some(runtime) = self.action_runtime.as_ref() else {
            self.restore_action_approval(pending);
            self.status = "Native remediation is unavailable".to_string();
            return;
        };
        if !runtime.approve(
            request_id,
            pending.proposal.proposal_id.clone(),
            snapshot,
            approval,
        ) {
            self.restore_action_approval(pending);
            self.status = "The native remediation approval queue is unavailable".to_string();
            return;
        }
        self.action_pending = Some(request_id);
        self.action_pending_approval = Some(pending);
        self.status = "Revalidating the reviewed remediation…".to_string();
        self.resume_action_wait(context);
    }

    pub(crate) fn resume_action_wait(&mut self, _context: &ComponentContext<Self>) {
        self.action_wait = None;
    }

    pub(crate) fn resume_action_run_wait(&mut self, _context: &ComponentContext<Self>) {
        self.action_run_wait = None;
    }

    pub(crate) fn apply_action_run_summary(&mut self, summary: ActionRunSummary) {
        let run_id = summary.run_id.clone();
        let first_observation = self
            .action_active_run
            .as_ref()
            .is_none_or(|run| run.run_id != run_id)
            && !self
                .action_run_history
                .iter()
                .any(|run| run.run_id == run_id);
        if first_observation {
            // New runs start open so partial/failure details are immediately
            // visible. Subsequent renders preserve the user's explicit
            // Expander choice instead of deriving it again from live/history.
            self.action_expanded_runs.insert(run_id.clone());
        }
        if summary.status.terminal() {
            if self
                .action_active_run
                .as_ref()
                .map(|run| run.run_id.as_str())
                == Some(run_id.as_str())
            {
                self.action_active_run = None;
            }
            self.action_run_history
                .retain(|existing| existing.run_id != run_id);
            self.action_run_history.insert(0, summary.clone());
            self.action_run_history.truncate(50);
        } else {
            self.action_active_run = Some(summary.clone());
        }
        let visible_ids = self
            .action_active_run
            .iter()
            .chain(self.action_run_history.iter())
            .map(|run| run.run_id.as_str())
            .collect::<HashSet<_>>();
        self.action_expanded_runs
            .retain(|expanded| visible_ids.contains(expanded.as_str()));
        self.status = action_run_status_text(&summary);
    }
}
