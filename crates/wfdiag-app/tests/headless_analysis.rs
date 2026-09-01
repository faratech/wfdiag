//! Per-task analysis, issue prioritisation, and the fix plan — including the
//! only route a plan may take toward execution: through the action broker's
//! own staging, naming nothing but catalog ids.

mod support;

use std::time::Duration;
use support::{Harness, ai_mocks, boot_ai_with};
use wfdiag_app::ports::mock::TaskScript;
use wfdiag_app::ports::mock_ai::ScriptedAnalysisOutcome;
use wfdiag_app::{
    ActionEvent, AnalysisEvent, AppCommand, AppEvent, FixPlanEvent, PrioritizationEvent,
};
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_ai_chat::ProviderUse;
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_issues::{FixPlanEntry, IssueStatus};
use wfdiag_native_remediation::RemediationTier;

/// A scan whose `logical_disk` output trips the low-disk detector, which is
/// what gives prioritisation and the fix plan a real detected issue to work
/// from.
fn boot_with_detected_issue(label: &str) -> Harness {
    let mocks = ai_mocks();
    mocks.executor.script(
        "logical_disk",
        TaskScript::ok(r#"[{"Name":"C:","FreeSpace":"5000000000","Size":"100000000000"}]"#),
    );
    let mut harness = boot_ai_with(label, mocks);
    harness.commit_scan();
    assert!(
        harness
            .service
            .snapshot()
            .issues
            .iter()
            .any(|issue| issue.id == "low_disk_space" && issue.status == IssueStatus::Detected),
        "the scripted evidence must produce a detected issue"
    );
    harness
}

#[test]
fn a_diagnostic_is_interpreted_without_reaching_the_network() {
    let mut harness = boot_with_detected_issue("analysis_task");
    harness.mocks.ai.analysis.script_analysis(
        "logical_disk",
        ScriptedAnalysisOutcome::completed("C: is nearly full."),
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::AnalyzeDiagnostic {
                task_id: "logical_disk".to_string(),
                force_refresh: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the interpretation", |event| {
        matches!(event, AppEvent::Analysis(AnalysisEvent::Completed { .. }))
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Analysis(AnalysisEvent::Started { .. })))
    );
    let interpretation = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Analysis(AnalysisEvent::Completed { interpretation, .. }) => {
                Some(interpretation.as_str())
            }
            _ => None,
        })
        .expect("the analysis completed");
    assert_eq!(interpretation, "C: is nearly full.");
    assert_eq!(
        harness.mocks.ai.analysis.grounding_flags(),
        [false],
        "live grounding is off by default and the request says so"
    );
    let snapshot = harness.service.snapshot();
    let entry = snapshot
        .ai
        .analyses
        .get("logical_disk")
        .expect("the read model carries the interpretation");
    assert!(!entry.busy);
    assert_eq!(entry.interpretation.as_deref(), Some("C: is nearly full."));
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn analysis_and_prioritization_share_one_slot_and_prioritization_ranks_detected_issues() {
    let mut harness = boot_with_detected_issue("analysis_priority");
    harness
        .mocks
        .ai
        .analysis
        .script_prioritization(ScriptedAnalysisOutcome::completed("1. Free disk space"));

    assert!(
        harness
            .service
            .dispatch(AppCommand::PrioritizeIssues {
                force_refresh: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the ranking", |event| {
        matches!(
            event,
            AppEvent::Prioritization(PrioritizationEvent::Completed { .. })
        )
    });
    let ranking = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Prioritization(PrioritizationEvent::Completed { ranking, .. }) => {
                Some(ranking.as_str())
            }
            _ => None,
        })
        .expect("prioritisation completed");
    assert_eq!(ranking, "1. Free disk space");
    assert_eq!(harness.mocks.ai.analysis.prioritized(), 1);
    assert_eq!(
        harness.service.snapshot().ai.prioritization.text.as_deref(),
        Some("1. Free disk space")
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_validated_fix_plan_names_only_catalog_ids_and_stages_through_the_broker() {
    let mut harness = boot_with_detected_issue("fix_plan");
    harness.mocks.ai.fix_plan.script(ValidatedFixPlan {
        entries: vec![FixPlanEntry {
            issue_id: "low_disk_space".to_string(),
            remediation_id: "open_disk_cleanup".to_string(),
            rationale: "C: is under 10% free".to_string(),
            tier: RemediationTier::OpenTool,
        }],
        notes: "One safe action.".to_string(),
        provider_use: ProviderUse::for_provider(AIProvider::Ollama, None),
        scan_fingerprint: String::new(),
        catalog_fingerprint: String::new(),
    });

    assert!(
        harness
            .service
            .dispatch(AppCommand::GenerateFixPlan)
            .is_accepted()
    );
    let events = harness.pump_for("the plan", |event| {
        matches!(event, AppEvent::FixPlan(FixPlanEvent::Completed { .. }))
    });
    let plan = events
        .iter()
        .find_map(|event| match event {
            AppEvent::FixPlan(FixPlanEvent::Completed { plan }) => Some(plan.clone()),
            _ => None,
        })
        .expect("a plan arrived");
    assert_eq!(plan.entries.len(), 1);
    let entry = &plan.entries[0];
    assert!(
        wfdiag_native_remediation::remediation::find(&entry.remediation_id).is_some(),
        "a plan may only ever name a compile-time catalog id"
    );

    // The only route from a plan to execution: the user stages the named id
    // through the broker, which revalidates it against live evidence.
    assert!(
        harness
            .service
            .dispatch(AppCommand::PrepareRemediation {
                remediation_id: entry.remediation_id.clone(),
                issue_id: Some(entry.issue_id.clone()),
            })
            .is_accepted()
    );
    let staged = harness.pump_for("the staged proposal", |event| {
        matches!(event, AppEvent::Action(ActionEvent::Proposal { .. }))
    });
    let proposal = staged
        .iter()
        .find_map(|event| match event {
            AppEvent::Action(ActionEvent::Proposal { proposal }) => Some(proposal.clone()),
            _ => None,
        })
        .expect("the broker staged the plan's action");
    assert_eq!(proposal.actions.len(), 1);
    assert_eq!(proposal.actions[0].remediation.id, "open_disk_cleanup");
    assert_eq!(
        proposal.actions[0].issue_id.as_deref(),
        Some("low_disk_space")
    );
    assert!(
        harness.mocks.ai.actions.executed().is_empty(),
        "staging is not execution"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_plan_generated_against_evidence_that_moved_on_is_discarded() {
    let mut harness = boot_with_detected_issue("fix_plan_stale");
    harness.mocks.ai.fix_plan.script(ValidatedFixPlan {
        entries: vec![FixPlanEntry {
            issue_id: "low_disk_space".to_string(),
            remediation_id: "open_disk_cleanup".to_string(),
            rationale: "stale".to_string(),
            tier: RemediationTier::OpenTool,
        }],
        notes: String::new(),
        provider_use: ProviderUse::for_provider(AIProvider::Ollama, None),
        // The scripted worker overwrites these from the request, so a plan is
        // stale only when the evidence changes underneath it.
        scan_fingerprint: String::new(),
        catalog_fingerprint: String::new(),
    });
    harness.mocks.ai.analysis.hold();

    assert!(
        harness
            .service
            .dispatch(AppCommand::GenerateFixPlan)
            .is_accepted()
    );
    harness.pump_for("the plan", |event| {
        matches!(event, AppEvent::FixPlan(FixPlanEvent::Completed { .. }))
    });
    assert!(harness.service.snapshot().ai.fix_plan.plan.is_some());

    // New evidence invalidates the plan it described.
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartScan {
                kind: wfdiag_native_diagnostics::ScanKind::Quick,
            })
            .is_accepted()
    );
    harness.pump_for("the invalidation", |event| {
        matches!(event, AppEvent::FixPlan(FixPlanEvent::Invalidated))
    });
    assert!(harness.service.snapshot().ai.fix_plan.plan.is_none());
    harness.shutdown(Duration::from_secs(2));
}
