//! Action review presentation for the remediation confirm surfaces.

#![deny(unsafe_code)]

use crate::app::policy::{action_proposal_contains_repair, action_proposal_schedules_restart};
use wfdiag_native_remediation::broker::{ActionProposal, ApprovalScope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionReviewPresentation {
    pub(crate) title: String,
    pub(crate) primary_label: String,
    pub(crate) admin_blocked: bool,
    pub(crate) schedules_restart: bool,
    pub(crate) requires_restart: bool,
    pub(crate) long_running: bool,
    pub(crate) can_stop: bool,
    pub(crate) batch: bool,
}

pub(crate) fn action_review_presentation(
    proposal: &ActionProposal,
    is_admin: bool,
) -> ActionReviewPresentation {
    let batch = proposal.approval_scope == ApprovalScope::Batch;
    let admin_blocked = !is_admin
        && proposal
            .actions
            .iter()
            .any(|action| action.remediation.admin_required);
    let schedules_restart = action_proposal_schedules_restart(proposal);
    let repair = action_proposal_contains_repair(proposal);
    let title = if batch {
        format!("Review {} actions", proposal.actions.len())
    } else {
        proposal.actions.first().map_or_else(
            || "Review remediation".to_string(),
            |action| action.remediation.label.clone(),
        )
    };
    let primary_label = if admin_blocked {
        "Restart as Administrator".to_string()
    } else if batch {
        format!("Run these {} actions", proposal.actions.len())
    } else if schedules_restart {
        "Schedule restart".to_string()
    } else if repair {
        "Run repair once".to_string()
    } else {
        "Run once".to_string()
    };
    ActionReviewPresentation {
        title,
        primary_label,
        admin_blocked,
        schedules_restart,
        requires_restart: proposal
            .actions
            .iter()
            .any(|action| action.remediation.requires_restart),
        long_running: proposal
            .actions
            .iter()
            .any(|action| action.remediation.long_running),
        can_stop: proposal
            .actions
            .iter()
            .all(|action| action.remediation.cancellable),
        batch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::policy::action_proposal_matches_snapshot;
    use wfdiag_native_remediation::broker::{ActionSnapshot, DetectedIssueRemediation};
    use wfdiag_native_remediation::remediation;

    fn action_proposal(
        remediation_id: &str,
        issue_id: Option<&str>,
        approval_scope: ApprovalScope,
    ) -> ActionProposal {
        let spec = remediation::find(remediation_id).expect("test remediation exists");
        ActionProposal {
            proposal_id: format!("proposal-{remediation_id}"),
            approval_scope,
            actions: vec![wfdiag_native_remediation::broker::ActionPreview {
                remediation: spec.summary(),
                issue_id: issue_id.map(str::to_string),
                steps: spec.preview_steps(),
            }],
            scan_fingerprint: "scan-1".to_string(),
            catalog_fingerprint: "catalog-1".to_string(),
            created_at_ms: 10,
            expires_at_ms: 20,
        }
    }

    #[test]
    fn action_review_labels_and_admin_gate_match_the_shipping_contract() {
        let restart = action_proposal("restart_system", None, ApprovalScope::Exact);
        let restart_ui = action_review_presentation(&restart, true);
        assert_eq!(restart_ui.primary_label, "Schedule restart");
        assert!(restart_ui.schedules_restart);

        let repair = action_proposal("dism_restorehealth", None, ApprovalScope::Exact);
        let repair_ui = action_review_presentation(&repair, true);
        assert_eq!(repair_ui.primary_label, "Run repair once");
        assert!(!repair_ui.admin_blocked);
        let blocked_ui = action_review_presentation(&repair, false);
        assert_eq!(blocked_ui.primary_label, "Restart as Administrator");
        assert!(blocked_ui.admin_blocked);

        let mut batch = action_proposal("flush_dns", None, ApprovalScope::Batch);
        batch.actions.push(
            action_proposal("clear_temp_files", None, ApprovalScope::Exact)
                .actions
                .remove(0),
        );
        let batch_ui = action_review_presentation(&batch, true);
        assert_eq!(batch_ui.title, "Review 2 actions");
        assert_eq!(batch_ui.primary_label, "Run these 2 actions");
    }

    #[test]
    fn staged_action_review_rejects_changed_evidence_catalog_and_issue_mapping() {
        let proposal = action_proposal("flush_dns", Some("dns-issue"), ApprovalScope::Exact);
        let snapshot = ActionSnapshot {
            scan_fingerprint: proposal.scan_fingerprint.clone(),
            catalog_fingerprint: proposal.catalog_fingerprint.clone(),
            detected_issues: vec![DetectedIssueRemediation {
                issue_id: "dns-issue".to_string(),
                remediation_id: Some("flush_dns".to_string()),
            }],
            is_admin: true,
        };
        assert!(action_proposal_matches_snapshot(&proposal, &snapshot));

        let mut changed_scan = snapshot.clone();
        changed_scan.scan_fingerprint = "scan-2".to_string();
        assert!(!action_proposal_matches_snapshot(&proposal, &changed_scan));

        let mut changed_catalog = snapshot.clone();
        changed_catalog.catalog_fingerprint = "catalog-2".to_string();
        assert!(!action_proposal_matches_snapshot(
            &proposal,
            &changed_catalog
        ));

        let mut changed_mapping = snapshot;
        changed_mapping.detected_issues[0].remediation_id = Some("clear_temp_files".to_string());
        assert!(!action_proposal_matches_snapshot(
            &proposal,
            &changed_mapping
        ));
    }
}
