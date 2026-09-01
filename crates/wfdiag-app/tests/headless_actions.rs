//! Remediation staging and execution.
//!
//! The broker under test is the shipping [`ActionBroker`] — fingerprints,
//! issue reconciliation, and the Repair gate are all real. Only catalog
//! execution is replaced, by a recorder that runs no command.

mod support;

use std::time::Duration;
use support::{Harness, ai_mocks, boot_ai_with};
use wfdiag_app::ports::mock::TaskScript;
use wfdiag_app::{ActionEvent, AppCommand, AppEvent, IssuesEvent};
use wfdiag_native_issues::IssueStatus;
use wfdiag_native_remediation::runtime::ActionRunStatus;

/// An `AutoSafe`, maintenance, non-admin catalog action.
const AUTO_SAFE: &str = "flush_dns";
/// A `Repair`, maintenance, non-admin catalog action.
const REPAIR: &str = "clear_temp_files";

fn boot_actions(label: &str) -> Harness {
    let mocks = ai_mocks();
    mocks.executor.script(
        "logical_disk",
        TaskScript::ok(r#"[{"Name":"C:","FreeSpace":"5000000000","Size":"100000000000"}]"#),
    );
    let mut harness = boot_ai_with(label, mocks);
    harness.commit_scan();
    harness
}

fn stage(harness: &mut Harness, remediation_id: &str) -> String {
    assert!(
        harness
            .service
            .dispatch(AppCommand::PrepareRemediation {
                remediation_id: remediation_id.to_string(),
                issue_id: None,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the staged proposal", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Proposal { .. }))
    });
    events
        .iter()
        .find_map(|event| match event {
            AppEvent::Action(ActionEvent::Proposal { proposal }) => {
                Some(proposal.proposal_id.clone())
            }
            _ => None,
        })
        .expect("a proposal was staged")
}

#[test]
fn an_auto_safe_maintenance_action_runs_through_the_recorder() {
    let mut harness = boot_actions("action_autosafe");
    let proposal_id = stage(&mut harness, AUTO_SAFE);

    assert!(
        harness
            .service
            .dispatch(AppCommand::ApproveAction {
                proposal_id,
                confirm_repair: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the run to finish", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Summary { .. }))
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Action(ActionEvent::Approved { .. }))),
        "the host is told a run started"
    );
    let summary = events
        .iter()
        .rev()
        .find_map(|event| match event {
            AppEvent::Action(ActionEvent::Summary { summary }) => Some(summary.clone()),
            _ => None,
        })
        .expect("the run reached a terminal state");
    assert_eq!(summary.status, ActionRunStatus::Succeeded);
    assert_eq!(harness.mocks.ai.actions.executed(), [AUTO_SAFE]);
    assert!(harness.service.snapshot().actions.active_run.is_none());
    assert_eq!(harness.service.snapshot().actions.history.len(), 1);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_repair_approved_with_mere_review_runs_nothing_and_asks_again() {
    let mut harness = boot_actions("action_repair_gate");
    let proposal_id = stage(&mut harness, REPAIR);

    assert!(
        harness
            .service
            .dispatch(AppCommand::ApproveAction {
                proposal_id: proposal_id.clone(),
                confirm_repair: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the repair gate", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::RepairConfirmationRequired { .. })
        )
    });
    assert!(
        harness.mocks.ai.actions.executed().is_empty(),
        "a Repair without its explicit confirmation must run nothing"
    );
    let returned = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Action(ActionEvent::RepairConfirmationRequired { proposal }) => {
                Some(proposal.clone())
            }
            _ => None,
        })
        .expect("the preview came back unconsumed");
    assert_eq!(returned.proposal_id, proposal_id);
    assert_eq!(
        harness
            .service
            .snapshot()
            .actions
            .repair_confirmation
            .as_ref()
            .map(|proposal| proposal.proposal_id.clone()),
        Some(proposal_id.clone()),
        "the preview stays reviewable on the confirmation surface"
    );

    // The second, repair-specific confirmation is what runs it.
    assert!(
        harness
            .service
            .dispatch(AppCommand::ApproveAction {
                proposal_id,
                confirm_repair: true,
            })
            .is_accepted()
    );
    harness.pump_for("the repair to run", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Summary { .. }))
    });
    assert_eq!(harness.mocks.ai.actions.executed(), [REPAIR]);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_preview_whose_evidence_moved_on_is_refused_and_never_runs() {
    let mut harness = boot_actions("action_stale");
    let proposal_id = stage(&mut harness, AUTO_SAFE);

    // A replacement scan changes the fingerprint the preview was bound to.
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartScan {
                kind: wfdiag_native_diagnostics::ScanKind::Quick,
            })
            .is_accepted()
    );
    harness.pump_until(
        |harness, _| !harness.service.snapshot().scan_busy(),
        "the replacement scan",
    );
    harness.pump_briefly();

    let outcome = harness.service.dispatch(AppCommand::ApproveAction {
        proposal_id,
        confirm_repair: false,
    });
    // Either the reconciliation already dropped the stale preview, or the
    // broker refuses it on the fresh snapshot. Both must run nothing.
    if outcome.is_accepted() {
        harness.pump_for("the refusal", |event| {
            matches!(event, AppEvent::Action(ActionEvent::Rejected { .. }))
        });
    }
    assert!(
        harness.mocks.ai.actions.executed().is_empty(),
        "a preview bound to evidence that is gone must never execute"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn issues_are_re_detected_after_a_successful_remediation() {
    let mut harness = boot_actions("action_refresh");
    assert!(
        harness
            .service
            .snapshot()
            .issues
            .iter()
            .any(|issue| issue.id == "low_disk_space" && issue.status == IssueStatus::Detected)
    );
    let proposal_id = stage(&mut harness, AUTO_SAFE);

    // The repair "fixed" the disk: the next detection pass sees free space.
    harness.mocks.executor.script(
        "logical_disk",
        TaskScript::ok(r#"[{"Name":"C:","FreeSpace":"90000000000","Size":"100000000000"}]"#),
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::ApproveAction {
                proposal_id,
                confirm_repair: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the post-run issue refresh", |event| {
        matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))
    });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Action(ActionEvent::Summary { .. }))),
        "the run finished before the refresh"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))),
        "returning to the Issues view must never show a known-stale projection"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn discarding_a_preview_removes_it_from_both_review_surfaces() {
    let mut harness = boot_actions("action_discard");
    let proposal_id = stage(&mut harness, AUTO_SAFE);
    assert!(harness.service.snapshot().actions.review.is_some());

    assert!(
        harness
            .service
            .dispatch(AppCommand::DiscardProposal {
                proposal_id: proposal_id.clone(),
            })
            .is_accepted()
    );
    assert!(harness.service.snapshot().actions.review.is_none());

    // An id the broker no longer holds cannot be approved.
    let outcome = harness.service.dispatch(AppCommand::ApproveAction {
        proposal_id,
        confirm_repair: false,
    });
    assert!(outcome.rejection().is_some());
    assert!(harness.mocks.ai.actions.executed().is_empty());
    harness.shutdown(Duration::from_secs(2));
}
