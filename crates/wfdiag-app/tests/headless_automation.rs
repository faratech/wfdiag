//! The automation layer: safe fixes run without a review, everything else
//! waits for the user. The broker under test is the shipping one; only
//! catalog execution is a recorder.

mod support;

use std::time::Duration;
use support::{Harness, ai_mocks, boot_ai_with};
use wfdiag_app::ports::AuditKind;
use wfdiag_app::ports::mock::{ScriptedExecutor, TaskScript};
use wfdiag_app::{
    ActionEvent, AppCommand, AppEvent, IssuesEvent, SafeFixOrigin, ScanEvent, SettingsEvent,
};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_issues::auto_fix::DeferReason;
use wfdiag_native_settings::SettingsUpdate;

/// Defender health with stale definitions: `defender_definitions_stale` maps
/// to `update_defender_signatures`, an `AutoSafe`, non-admin catalog action.
const STALE_DEFENDER: &str = r#"{"defender": {"AMRunningMode": "Normal", "RealTimeProtectionEnabled": true, "AntivirusSignatureAge": 12, "QuickScanAge": 3, "FullScanAge": 60}}"#;
const FRESH_DEFENDER: &str = r#"{"defender": {"AMRunningMode": "Normal", "RealTimeProtectionEnabled": true, "AntivirusSignatureAge": 1, "QuickScanAge": 3, "FullScanAge": 60}}"#;
/// Low disk: `low_disk_space` maps to `clear_temp_files`, a Repair.
const LOW_DISK: &str = r#"[{"Name":"C:","FreeSpace":"5000000000","Size":"100000000000"}]"#;
const HEALTHY_DISK: &str = r#"[{"Name":"C:","FreeSpace":"50000000000","Size":"100000000000"}]"#;

fn boot_with_findings(label: &str, defender: &str, disk: &str) -> Harness {
    let mut mocks = ai_mocks();
    // The default scripted catalog has no Defender task; add it so the
    // stale-definitions rule can fire.
    mocks.executor =
        ScriptedExecutor::with_tasks(&["os_info", "processor", "logical_disk", "defender_health"]);
    mocks
        .executor
        .script("defender_health", TaskScript::ok(defender));
    mocks.executor.script("logical_disk", TaskScript::ok(disk));
    let mut harness = boot_ai_with(label, mocks);
    harness.commit_scan();
    harness
}

fn enable(harness: &mut Harness, update: SettingsUpdate) {
    assert!(
        harness
            .service
            .dispatch(AppCommand::UpdateSetting(update))
            .is_accepted()
    );
    harness.pump_for("the setting", |event| {
        matches!(event, AppEvent::Settings(SettingsEvent::Updated { .. }))
    });
}

fn finished(events: &[AppEvent]) -> Option<(SafeFixOrigin, usize, usize)> {
    events.iter().find_map(|event| match event {
        AppEvent::Action(ActionEvent::SafeFixesFinished {
            origin,
            runs,
            succeeded,
        }) => Some((*origin, *runs, *succeeded)),
        _ => None,
    })
}

/// The planned remediation ids and the deferred `(issue, reason)` pairs.
type PlannedFixes = (Vec<String>, Vec<(String, DeferReason)>);

fn planned(events: &[AppEvent]) -> Option<PlannedFixes> {
    events.iter().find_map(|event| match event {
        AppEvent::Action(ActionEvent::SafeFixesPlanned {
            actions, deferred, ..
        }) => Some((
            actions
                .iter()
                .map(|action| action.remediation_id.clone())
                .collect(),
            deferred
                .iter()
                .map(|deferred| (deferred.issue_id.clone(), deferred.reason))
                .collect(),
        )),
        _ => None,
    })
}

fn no_review_was_opened(events: &[AppEvent]) -> bool {
    !events.iter().any(|event| {
        matches!(
            event,
            AppEvent::Action(
                ActionEvent::Proposal { .. } | ActionEvent::RepairConfirmationRequired { .. }
            )
        )
    })
}

#[test]
fn run_safe_fixes_runs_the_safe_fix_defers_the_repair_and_re_detects() {
    let mut harness = boot_with_findings("automation_user", STALE_DEFENDER, LOW_DISK);
    // The fix "worked": the next detection pass sees fresh definitions.
    harness
        .mocks
        .executor
        .script("defender_health", TaskScript::ok(FRESH_DEFENDER));

    let outcome = harness.service.dispatch(AppCommand::RunSafeFixes);
    assert!(
        outcome.is_accepted(),
        "{outcome:?}; detected: {:?}",
        harness
            .service
            .snapshot()
            .issues
            .iter()
            .filter(|issue| issue.detected)
            .map(|issue| (
                issue.id.clone(),
                issue.remediation.as_ref().map(|r| r.id.clone())
            ))
            .collect::<Vec<_>>()
    );
    let events = harness.pump_for("the automation to finish", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    let (actions, deferred) = planned(&events).expect("a plan was announced");
    assert_eq!(actions, ["update_defender_signatures"]);
    assert_eq!(
        deferred,
        [("low_disk_space".to_string(), DeferReason::NeedsConfirmation)],
        "a Repair is reported, never run"
    );
    assert_eq!(
        harness.mocks.ai.actions.executed(),
        ["update_defender_signatures"]
    );
    assert!(
        no_review_was_opened(&events),
        "a safe fix runs without a review surface"
    );
    assert_eq!(finished(&events), Some((SafeFixOrigin::User, 1, 1)));
    // The run was verified: the fixed issue's source task was re-collected
    // and the fresh projection no longer detects it, while the deferred
    // repair is still on the list for the user.
    let verified = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Action(ActionEvent::Verified {
                resolved,
                unresolved,
                ..
            }) => Some((resolved.clone(), unresolved.clone())),
            _ => None,
        })
        .expect("the fix was verified against fresh evidence");
    assert_eq!(
        verified,
        (vec!["defender_definitions_stale".to_string()], vec![])
    );
    // The audit trail has the plan, the run, its verification and the end.
    let kinds: Vec<AuditKind> = harness
        .mocks
        .audit
        .entries()
        .iter()
        .map(|entry| entry.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            AuditKind::SafeFixesPlanned,
            AuditKind::RunFinished,
            AuditKind::Verified,
            AuditKind::SafeFixesFinished,
        ]
    );
    let run = &harness.mocks.audit.entries()[1];
    assert_eq!(run.origin, Some(SafeFixOrigin::User));
    assert_eq!(
        run.detail["run"]["actions"][0]["remediationId"],
        "update_defender_signatures"
    );
    let issues = &harness.service.snapshot().issues;
    assert!(
        issues
            .iter()
            .any(|issue| issue.id == "defender_definitions_stale" && !issue.detected)
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.id == "low_disk_space" && issue.detected)
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn nothing_safe_to_run_is_reported_not_faked() {
    let mut harness = boot_with_findings("automation_nothing", FRESH_DEFENDER, LOW_DISK);
    let outcome = harness.service.dispatch(AppCommand::RunSafeFixes);
    assert!(
        matches!(outcome, wfdiag_app::DispatchOutcome::Ignored { .. }),
        "{outcome:?}"
    );
    let events = harness.pump_for("the empty plan", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    let (actions, deferred) = planned(&events).unwrap();
    assert!(actions.is_empty());
    assert_eq!(deferred[0].1, DeferReason::NeedsConfirmation);
    assert!(harness.mocks.ai.actions.executed().is_empty());
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn after_scan_automation_runs_only_when_switched_on_and_only_for_new_scans() {
    let mut harness = boot_with_findings("automation_after_scan", STALE_DEFENDER, HEALTHY_DISK);
    // Off by default: the committed scan ran nothing.
    assert!(harness.mocks.ai.actions.executed().is_empty());

    enable(&mut harness, SettingsUpdate::AutoFixSafeIssues(true));
    // A refresh of the same scan never starts automation.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RefreshIssues)
            .is_accepted()
    );
    let events = harness.pump_for("the refresh", |event| {
        matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))
    });
    assert!(planned(&events).is_none(), "a refresh is not a new scan");

    assert!(
        harness
            .service
            .dispatch(AppCommand::StartScan {
                kind: ScanKind::Quick,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the after-scan automation", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    assert_eq!(
        finished(&events).map(|(origin, runs, _)| (origin, runs)),
        Some((SafeFixOrigin::AfterScan, 1))
    );
    assert_eq!(
        harness.mocks.ai.actions.executed(),
        ["update_defender_signatures"]
    );
    assert!(no_review_was_opened(&events));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))),
        "the scan itself still finalized normally"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_assistant_only_acts_with_standing_permission_and_never_on_a_repair() {
    let mut harness = boot_with_findings("automation_assistant", STALE_DEFENDER, LOW_DISK);

    // Permission off: the staged action goes to the review surface.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RunAssistantRemediation {
                remediation_id: "update_defender_signatures".to_string(),
                issue_id: Some("defender_definitions_stale".to_string()),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the review", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Proposal { .. }))
    });
    assert!(planned(&events).is_none());
    assert!(harness.mocks.ai.actions.executed().is_empty());
    let proposal_id = harness
        .service
        .snapshot()
        .actions
        .review
        .as_ref()
        .map(|proposal| proposal.proposal_id.clone())
        .expect("staged for the user");
    assert!(
        harness
            .service
            .dispatch(AppCommand::DiscardProposal { proposal_id })
            .is_accepted()
    );
    harness.pump_for("the discard", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Discarded { .. }))
    });

    // Permission on: the safe fix runs, the repair still asks.
    enable(&mut harness, SettingsUpdate::AssistantMayRunSafeFixes(true));
    assert!(
        harness
            .service
            .dispatch(AppCommand::RunAssistantRemediation {
                remediation_id: "update_defender_signatures".to_string(),
                issue_id: Some("defender_definitions_stale".to_string()),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the assistant's fix", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    assert_eq!(
        finished(&events).map(|(origin, runs, _)| (origin, runs)),
        Some((SafeFixOrigin::Assistant, 1))
    );
    assert_eq!(
        harness.mocks.ai.actions.executed(),
        ["update_defender_signatures"]
    );
    assert!(no_review_was_opened(&events));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Action(ActionEvent::Verified { .. }))),
        "the assistant's fix was verified like any other"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::RunAssistantRemediation {
                remediation_id: "clear_temp_files".to_string(),
                issue_id: Some("low_disk_space".to_string()),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the repair's review", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Proposal { .. }))
    });
    assert!(
        planned(&events).is_none(),
        "a Repair the assistant stages is never automated"
    );
    assert_eq!(
        harness.mocks.ai.actions.executed(),
        ["update_defender_signatures"],
        "nothing else ran"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_failing_safe_fix_stops_the_session_and_is_not_retried() {
    let mut harness = boot_with_findings("automation_failure", STALE_DEFENDER, HEALTHY_DISK);
    harness
        .mocks
        .ai
        .actions
        .executor
        .fail_on("update_defender_signatures");
    assert!(
        harness
            .service
            .dispatch(AppCommand::RunSafeFixes)
            .is_accepted()
    );
    let events = harness.pump_for("the automation to finish", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    let (_, runs, succeeded) = finished(&events).unwrap();
    assert_eq!((runs, succeeded), (1, 0));
    assert_eq!(
        harness.mocks.ai.actions.executed(),
        ["update_defender_signatures"],
        "attempted exactly once"
    );
    // Asking again re-plans from the still-detected issue and tries once more
    // (a new session), but never loops inside one session.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RunSafeFixes)
            .is_accepted()
    );
    harness.pump_for("the second session", |event| {
        matches!(
            event,
            AppEvent::Action(ActionEvent::SafeFixesFinished { .. })
        )
    });
    assert_eq!(harness.mocks.ai.actions.executed().len(), 2);
    harness.shutdown(Duration::from_secs(2));
}
