//! Staged remediation review: fingerprints, reconciliation, and the surface a
//! refused approval returns to.
//!
//! The security boundary itself is
//! [`wfdiag_native_remediation::broker::ActionBroker`]: nothing here can
//! authorise anything. What lives here is the *shell-side* half of the
//! transaction — how the authoritative snapshot is built from committed
//! evidence, whether a staged preview still describes that evidence, and where
//! a preview goes when an approval bounces.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use wfdiag_native_issues::{Issue, IssueStatus};
use wfdiag_native_remediation::RemediationTier;
use wfdiag_native_remediation::broker::{
    ActionProposal, ActionSnapshot, DetectedIssueRemediation, current_action_catalog_fingerprint,
};
use wfdiag_ui_core::DiagnosticTaskResult;

/// Which review surface a staged preview belongs to.
///
/// A refused approval must return to the surface it came from: bouncing a
/// Repair confirmation back to the first review dialog would ask the user to
/// approve the same preview twice with the weaker approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewSurface {
    /// The first, immutable-preview review.
    Review,
    /// The Repair-specific second confirmation.
    RepairConfirmation,
}

/// A staged preview plus the surface it must return to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedReview {
    /// The preview.
    pub proposal: ActionProposal,
    /// Where it came from.
    pub surface: ReviewSurface,
}

/// Fingerprint the committed evidence a proposal is bound to.
///
/// The session id and the issue generation are hashed first, then every task
/// result in **task-id order** so that thread scheduling cannot change the
/// value. A proposal whose fingerprint no longer matches describes evidence
/// that is gone, and the broker refuses to authorise it.
#[must_use]
pub fn scan_fingerprint(
    session_id: Option<&str>,
    issue_generation: u64,
    results: &[DiagnosticTaskResult],
) -> String {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    issue_generation.hash(&mut hasher);
    let mut ordered: Vec<&DiagnosticTaskResult> = results.iter().collect();
    ordered.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    for result in ordered {
        result.session_id.hash(&mut hasher);
        result.task_id.hash(&mut hasher);
        result.success.hash(&mut hasher);
        result.output.hash(&mut hasher);
        result.error.hash(&mut hasher);
    }
    format!(
        "{}:{:016x}",
        session_id.unwrap_or("no-scan"),
        hasher.finish()
    )
}

/// Project the detected issues into the broker's reconciliation view.
///
/// Only `Detected` issues can authorise a remediation: an issue that is `Ok`,
/// `Unknown`, or `Skipped` is not evidence of a problem to repair.
#[must_use]
pub fn detected_issue_remediations(issues: &[Issue]) -> Vec<DetectedIssueRemediation> {
    issues
        .iter()
        .filter(|issue| issue.status == IssueStatus::Detected)
        .map(|issue| DetectedIssueRemediation {
            issue_id: issue.id.clone(),
            remediation_id: issue
                .remediation
                .as_ref()
                .map(|remediation| remediation.id.clone()),
        })
        .collect()
}

/// Build the authoritative snapshot a prepare or approve is validated against.
#[must_use]
pub fn build_snapshot(
    session_id: Option<&str>,
    issue_generation: u64,
    results: &[DiagnosticTaskResult],
    issues: &[Issue],
    is_admin: bool,
) -> ActionSnapshot {
    ActionSnapshot {
        scan_fingerprint: scan_fingerprint(session_id, issue_generation, results),
        catalog_fingerprint: current_action_catalog_fingerprint(),
        detected_issues: detected_issue_remediations(issues),
        is_admin,
    }
}

/// Whether a staged preview still describes `snapshot`.
///
/// An action bound to an issue survives only while that exact issue still maps
/// to that exact remediation. An action bound to no issue is a maintenance
/// action, and survives only while the catalog still says so.
#[must_use]
pub fn proposal_matches(proposal: &ActionProposal, snapshot: &ActionSnapshot) -> bool {
    proposal.scan_fingerprint == snapshot.scan_fingerprint
        && proposal.catalog_fingerprint == snapshot.catalog_fingerprint
        && proposal.actions.iter().all(|action| {
            action
                .issue_id
                .as_deref()
                .map_or(action.remediation.maintenance, |issue_id| {
                    snapshot.detected_issues.iter().any(|issue| {
                        issue.issue_id == issue_id
                            && issue.remediation_id.as_deref()
                                == Some(action.remediation.id.as_str())
                    })
                })
        })
}

/// Whether any action in a preview is Repair-tier, and therefore needs the
/// second, repair-specific confirmation.
#[must_use]
pub fn contains_repair(proposal: &ActionProposal) -> bool {
    proposal
        .actions
        .iter()
        .any(|action| action.remediation.tier == RemediationTier::Repair)
}

/// Whether a preview cannot run because it needs rights this host lacks.
#[must_use]
pub fn admin_blocked(proposal: &ActionProposal, is_admin: bool) -> bool {
    !is_admin
        && proposal
            .actions
            .iter()
            .any(|action| action.remediation.admin_required)
}

/// Which of the two staged reviews a new snapshot invalidates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaleReviews {
    /// The first-review preview is stale.
    pub review: bool,
    /// The Repair-confirmation preview is stale.
    pub repair: bool,
}

impl StaleReviews {
    /// Whether anything is stale.
    #[must_use]
    pub const fn any(self) -> bool {
        self.review || self.repair
    }
}

/// Evaluate both staged surfaces against one freshly captured snapshot.
///
/// Both are judged against the *same* snapshot: taking two would let evidence
/// land between them and leave one surface reconciled against evidence the
/// other never saw.
#[must_use]
pub fn stale_reviews(
    review: Option<&ActionProposal>,
    repair: Option<&ActionProposal>,
    snapshot: &ActionSnapshot,
) -> StaleReviews {
    StaleReviews {
        review: review.is_some_and(|proposal| !proposal_matches(proposal, snapshot)),
        repair: repair.is_some_and(|proposal| !proposal_matches(proposal, snapshot)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReviewSurface, StagedReview, admin_blocked, build_snapshot, contains_repair,
        detected_issue_remediations, proposal_matches, scan_fingerprint, stale_reviews,
    };
    use std::sync::Arc;
    use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, TaskResult};
    use wfdiag_native_remediation::broker::{ActionPreview, ActionProposal, ApprovalScope};
    use wfdiag_native_remediation::{RemediationTier, remediation};
    use wfdiag_ui_core::DiagnosticTaskResult;

    fn result(task_id: &str, output: &str) -> DiagnosticTaskResult {
        DiagnosticTaskResult::new(
            "session-1",
            task_id,
            Arc::new(TaskResult {
                success: true,
                output: output.to_string(),
                error: None,
                duration_ms: 1,
            }),
        )
    }

    fn summary(id: &str) -> wfdiag_native_issues::RemediationSummary {
        remediation::find(id)
            .expect("the catalog contains this id")
            .summary()
    }

    fn issue(id: &str, remediation_id: &str, status: IssueStatus) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Storage".to_string(),
            severity: IssueSeverity::Warning,
            status,
            title: "Low disk space".to_string(),
            description: "C: is nearly full".to_string(),
            recommendation: "Free space".to_string(),
            detected: status == IssueStatus::Detected,
            source_tasks: None,
            remediation: Some(summary(remediation_id)),
        }
    }

    fn proposal(snapshot: &wfdiag_native_remediation::broker::ActionSnapshot) -> ActionProposal {
        let spec = remediation::find("open_disk_cleanup").expect("catalog id");
        ActionProposal {
            proposal_id: "prop-1".to_string(),
            approval_scope: ApprovalScope::Exact,
            actions: vec![ActionPreview {
                remediation: spec.summary(),
                issue_id: Some("low_disk_space".to_string()),
                steps: spec.preview_steps(),
            }],
            scan_fingerprint: snapshot.scan_fingerprint.clone(),
            catalog_fingerprint: snapshot.catalog_fingerprint.clone(),
            created_at_ms: 0,
            expires_at_ms: u64::MAX,
        }
    }

    #[test]
    fn the_fingerprint_ignores_result_order_but_not_result_content() {
        let issues = Vec::new();
        let forward = [result("a", "one"), result("b", "two")];
        let reversed = [result("b", "two"), result("a", "one")];
        assert_eq!(
            scan_fingerprint(Some("session-1"), 1, &forward),
            scan_fingerprint(Some("session-1"), 1, &reversed),
            "task ordering is scheduling noise, not evidence"
        );
        let changed = [result("a", "one"), result("b", "CHANGED")];
        assert_ne!(
            scan_fingerprint(Some("session-1"), 1, &forward),
            scan_fingerprint(Some("session-1"), 1, &changed)
        );
        assert_ne!(
            scan_fingerprint(Some("session-1"), 1, &forward),
            scan_fingerprint(Some("session-1"), 2, &forward),
            "a new issue generation is new evidence"
        );
        assert!(scan_fingerprint(None, 0, &[]).starts_with("no-scan:"));
        let _ = build_snapshot(Some("session-1"), 1, &forward, &issues, false);
    }

    #[test]
    fn only_detected_issues_can_authorise_a_remediation() {
        let issues = [
            issue("low_disk_space", "open_disk_cleanup", IssueStatus::Detected),
            issue("healthy", "open_disk_cleanup", IssueStatus::Ok),
        ];
        let projected = detected_issue_remediations(&issues);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].issue_id, "low_disk_space");
        assert_eq!(
            projected[0].remediation_id.as_deref(),
            Some("open_disk_cleanup")
        );
    }

    #[test]
    fn a_proposal_goes_stale_when_its_issue_or_evidence_changes() {
        let results = [result("disk_space", "full")];
        let issues = [issue(
            "low_disk_space",
            "open_disk_cleanup",
            IssueStatus::Detected,
        )];
        let snapshot = build_snapshot(Some("session-1"), 1, &results, &issues, true);
        let proposal = proposal(&snapshot);
        assert!(proposal_matches(&proposal, &snapshot));

        let newer = build_snapshot(Some("session-2"), 2, &results, &issues, true);
        assert!(
            !proposal_matches(&proposal, &newer),
            "new evidence invalidates"
        );

        let resolved = build_snapshot(Some("session-1"), 1, &results, &[], true);
        assert!(
            !proposal_matches(&proposal, &resolved),
            "the issue that authorised the action is gone"
        );
    }

    #[test]
    fn both_staged_surfaces_are_judged_against_one_snapshot() {
        let results = [result("disk_space", "full")];
        let issues = [issue(
            "low_disk_space",
            "open_disk_cleanup",
            IssueStatus::Detected,
        )];
        let snapshot = build_snapshot(Some("session-1"), 1, &results, &issues, true);
        let staged = proposal(&snapshot);
        assert_eq!(
            stale_reviews(Some(&staged), Some(&staged), &snapshot),
            super::StaleReviews::default()
        );
        let newer = build_snapshot(Some("session-2"), 2, &results, &issues, true);
        let stale = stale_reviews(Some(&staged), Some(&staged), &newer);
        assert!(stale.review && stale.repair && stale.any());
        assert!(!stale_reviews(None, None, &newer).any());
    }

    #[test]
    fn the_repair_tier_and_admin_gates_read_the_preview_not_the_catalog_id() {
        let results = [result("disk_space", "full")];
        let issues = [issue(
            "low_disk_space",
            "open_disk_cleanup",
            IssueStatus::Detected,
        )];
        let snapshot = build_snapshot(Some("session-1"), 1, &results, &issues, true);
        let mut staged = proposal(&snapshot);
        assert!(!contains_repair(&staged));

        let repair = remediation::remediations()
            .iter()
            .find(|spec| spec.tier == RemediationTier::Repair)
            .expect("the catalog ships repair-tier actions");
        staged.actions[0].remediation = repair.summary();
        staged.actions[0].issue_id = None;
        assert!(contains_repair(&staged));
        assert_eq!(
            admin_blocked(&staged, false),
            repair.admin_required,
            "a repair that needs admin is blocked without it"
        );
        assert!(!admin_blocked(&staged, true));

        let review = StagedReview {
            proposal: staged,
            surface: ReviewSurface::RepairConfirmation,
        };
        assert_eq!(review.surface, ReviewSurface::RepairConfirmation);
    }
}
