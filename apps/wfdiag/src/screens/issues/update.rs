//! How the Issues screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::policy::{action_run_status_text, rejection_text};
use crate::app::screen::{Effect, ScreenCx};
use crate::app::state::{FixPlanActionSelection, Page};
use crate::fixtures::visual::LiveTestFixture;
use crate::screens::issues::state::{IssuesMsg, IssuesScreen};
use wfdiag_app::{
    ActionEvent, AppCommand, AppEvent, DispatchOutcome, FixPlanEvent, IssuesEvent,
    PrioritizationEvent, ScanEvent,
};
use wfdiag_native_issues::projection::project_issues;
use wfdiag_native_remediation::broker::ActionRequest;
use wfdiag_native_remediation::remediation;

impl IssuesScreen {
    pub(crate) fn update(&mut self, message: IssuesMsg, cx: &mut ScreenCx<'_>) {
        match message {
            IssuesMsg::RunRemediation(remediation_id) => self.run_remediation(remediation_id, cx),
            IssuesMsg::AskAiAboutIssue(issue_id) => self.ask_ai_about_issue(&issue_id, cx),
            IssuesMsg::Prioritize => {
                self.begin_prioritization(self.prioritization.text.is_some(), cx);
            }
            IssuesMsg::CancelPrioritization => {
                if cx.dispatch(AppCommand::CancelAnalysis).is_accepted() {
                    cx.status("Cancelling issue prioritization…");
                }
            }
            IssuesMsg::ProposeFixPlan => self.begin_fix_plan(cx),
            IssuesMsg::CancelFixPlan => {
                if cx.dispatch(AppCommand::CancelFixPlan).is_accepted() {
                    cx.status("Cancelling the AI fix plan…");
                }
            }
            IssuesMsg::ReviewFixPlanActions(selection) => self.prepare_selection(selection, cx),
            IssuesMsg::CancelActionRun => self.cancel_action_run(cx),
            IssuesMsg::RunExpandedChanged { run_id, expanded } => {
                let visible = self
                    .active_run
                    .as_ref()
                    .is_some_and(|run| run.run_id == run_id)
                    || self.run_history.iter().any(|run| run.run_id == run_id);
                if visible {
                    if expanded {
                        self.expanded_runs.insert(run_id);
                    } else {
                        self.expanded_runs.remove(&run_id);
                    }
                }
            }
        }
    }

    /// Re-run detection against the latest committed evidence.
    pub(crate) fn request_refresh(&mut self, cx: &mut ScreenCx<'_>) -> bool {
        if cx.shell.deterministic_visual {
            return false;
        }
        let accepted = cx.dispatch(AppCommand::RefreshIssues).is_accepted();
        self.refreshing = accepted;
        accepted
    }

    pub(crate) fn begin_prioritization(&mut self, force_refresh: bool, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual {
            cx.status("Visual fixture mode · issue prioritization is disabled");
            return;
        }
        match cx.dispatch(AppCommand::PrioritizeIssues { force_refresh }) {
            DispatchOutcome::Accepted { .. } => cx.status("Prioritizing detected issues…"),
            DispatchOutcome::Ignored { detail } => cx.status(detail.to_string()),
            outcome => cx.report_rejection(&outcome),
        }
    }

    fn begin_fix_plan(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual {
            cx.status("Visual fixture mode · AI fix planning is disabled");
            return;
        }
        match cx.dispatch(AppCommand::GenerateFixPlan) {
            DispatchOutcome::Accepted { .. } => {}
            DispatchOutcome::Ignored { detail } => cx.status(detail.to_string()),
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.fix_plan_error = Some(message.clone());
                    cx.status(message);
                }
            }
        }
    }

    fn ask_ai_about_issue(&mut self, issue_id: &str, cx: &mut ScreenCx<'_>) {
        let Some(issue) = self.issues.iter().find(|issue| issue.id == issue_id) else {
            return;
        };
        let remediation = issue.remediation.as_ref().map_or_else(
            || "No vetted remediation is mapped to this issue.".to_string(),
            |remediation| {
                format!(
                    "Vetted remediation: {} (catalog id {}).",
                    remediation.label, remediation.id
                )
            },
        );
        let prompt = format!(
            "Explain this detected Windows issue and give safe next steps using the scan evidence and read-only tools. Issue id: {}. Title: {}. Category: {}. Severity: {:?}. Description: {}. Recommendation: {}. {} Do not claim any repair was run; stage only vetted catalog actions if useful.",
            issue.id,
            issue.title,
            issue.category,
            issue.severity,
            issue.description,
            issue.recommendation,
            remediation,
        );
        cx.effect(Effect::AskAi { prompt });
    }

    fn run_remediation(&mut self, remediation_id: String, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual
            && cx.shell.live_test_fixture != Some(LiveTestFixture::DeviceManager)
        {
            cx.status("Visual fixture mode · remediation is disabled");
            return;
        }
        let Some(spec) = remediation::find(&remediation_id) else {
            cx.status(format!("Unknown remediation '{remediation_id}'"));
            return;
        };
        let issue_id = self
            .issues
            .iter()
            .find(|issue| {
                issue.status == wfdiag_native_issues::IssueStatus::Detected
                    && issue.remediation.as_ref().map(|item| item.id.as_str())
                        == Some(remediation_id.as_str())
            })
            .map(|issue| issue.id.clone());
        if issue_id.is_none() && !spec.maintenance {
            cx.status(format!(
                "'{}' is no longer mapped to a detected issue",
                spec.label
            ));
            return;
        }
        self.prepare_remediation(remediation_id, issue_id, cx);
    }

    pub(crate) fn prepare_remediation(
        &mut self,
        remediation_id: String,
        issue_id: Option<String>,
        cx: &mut ScreenCx<'_>,
    ) {
        if cx.shell.live_test_fixture.is_some_and(|fixture| {
            !fixture.permits_actions(std::slice::from_ref(&ActionRequest {
                remediation_id: remediation_id.clone(),
                issue_id: issue_id.clone(),
            }))
        }) {
            cx.status("The validation fixture rejected an action outside its closed allowlist");
            return;
        }
        match cx.dispatch(AppCommand::PrepareRemediation {
            remediation_id,
            issue_id,
        }) {
            DispatchOutcome::Accepted { .. } => {
                cx.status("Preparing a review for 1 vetted action…");
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    fn prepare_selection(&mut self, selection: FixPlanActionSelection, cx: &mut ScreenCx<'_>) {
        if cx
            .shell
            .live_test_fixture
            .is_some_and(|fixture| !fixture.permits_actions(&selection.actions))
        {
            cx.status("The validation fixture rejected an action outside its closed allowlist");
            return;
        }
        let action_count = selection.actions.len();
        match cx.dispatch(AppCommand::PrepareRemediations {
            actions: selection.actions,
            expected_scan_fingerprint: Some(selection.expected_scan_fingerprint),
            expected_catalog_fingerprint: Some(selection.expected_catalog_fingerprint),
        }) {
            DispatchOutcome::Accepted { .. } => {
                cx.status(format!(
                    "Preparing a review for {action_count} vetted action{}…",
                    if action_count == 1 { "" } else { "s" }
                ));
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    fn cancel_action_run(&mut self, cx: &mut ScreenCx<'_>) {
        let Some(run_id) = self.active_run.as_ref().map(|run| run.run_id.clone()) else {
            cx.status("No remediation run is active");
            return;
        };
        let outcome = cx.dispatch(AppCommand::CancelAction { run_id });
        if let Some(reason) = outcome.rejection() {
            cx.status(format!(
                "Could not stop remediation · {}",
                rejection_text(reason)
            ));
        }
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        match event {
            AppEvent::Issues(event) => self.on_issues_event(event, cx),
            AppEvent::Scan(ScanEvent::Committed { .. } | ScanEvent::TargetedCommitted { .. }) => {
                // Committing evidence re-runs detection inside the engine.
                self.refreshing = true;
            }
            AppEvent::Prioritization(event) => self.on_prioritization_event(event, cx),
            AppEvent::FixPlan(event) => Self::on_fix_plan_event(event, cx),
            AppEvent::Action(event) => self.on_action_event(event, cx),
            _ => {}
        }
    }

    fn on_issues_event(&mut self, event: &IssuesEvent, cx: &mut ScreenCx<'_>) {
        match event {
            IssuesEvent::Updated { session_id, .. } => {
                self.refreshing = false;
                self.projected_session_id = Some(session_id.clone());
                if cx.shell.page == Page::Issues && !cx.scan.busy {
                    cx.status(project_issues(&self.issues).counts.summary_text());
                }
            }
            IssuesEvent::Failed { error } => {
                self.refreshing = false;
                if cx.shell.page == Page::Issues {
                    cx.status(error.clone());
                }
            }
        }
    }

    fn on_prioritization_event(&mut self, event: &PrioritizationEvent, cx: &mut ScreenCx<'_>) {
        match event {
            PrioritizationEvent::Started { .. } => {
                cx.status("The AI provider is prioritizing detected issues…");
            }
            PrioritizationEvent::Completed { cached, .. } => {
                let provider = self
                    .prioritization
                    .provider_use
                    .as_ref()
                    .map_or_else(|| "AI".to_string(), |use_| use_.provider_id.clone());
                cx.status(if *cached {
                    format!("Issue prioritization ready · {provider} · cached")
                } else {
                    format!("Issue prioritization ready · {provider}")
                });
            }
            PrioritizationEvent::Failed { message, .. } => {
                cx.status(format!("Issue prioritization failed · {message}"));
            }
            PrioritizationEvent::Cancelled => cx.status("Issue prioritization cancelled"),
            PrioritizationEvent::Invalidated => {
                cx.status("Ignored stale issue prioritization after evidence changed");
            }
            _ => {}
        }
    }

    fn on_fix_plan_event(event: &FixPlanEvent, cx: &mut ScreenCx<'_>) {
        match event {
            FixPlanEvent::Started { provider } => {
                cx.status(format!("Preparing a vetted fix plan with {provider}…"));
            }
            FixPlanEvent::Completed { plan } => {
                let entry_count = plan.entries.len();
                cx.status(format!(
                    "Fix plan ready · {entry_count} vetted action{} · {}",
                    if entry_count == 1 { "" } else { "s" },
                    plan.provider_use.provider_id
                ));
            }
            FixPlanEvent::Failed { message, .. } => cx.status(message.clone()),
            FixPlanEvent::Cancelled => cx.status("AI fix plan cancelled"),
            _ => {}
        }
    }

    fn on_action_event(&mut self, event: &ActionEvent, cx: &mut ScreenCx<'_>) {
        match event {
            ActionEvent::Proposal { proposal } => {
                let action_count = proposal.actions.len();
                cx.status(format!(
                    "Review {action_count} vetted remediation action{}",
                    if action_count == 1 { "" } else { "s" }
                ));
            }
            ActionEvent::RepairConfirmationRequired { .. } => {
                cx.status("Explicit repair confirmation is required");
            }
            ActionEvent::Approved { summary, .. } | ActionEvent::Run { summary } => {
                self.expanded_runs.insert(summary.run_id.clone());
                cx.status(action_run_status_text(summary));
            }
            ActionEvent::Summary { summary } => {
                let succeeded = summary
                    .actions
                    .iter()
                    .any(|action| action.result.as_ref().is_some_and(|result| result.success));
                cx.status(action_run_status_text(summary));
                if succeeded && !cx.shell.deterministic_visual {
                    // The fix may have changed what detection sees. Refresh
                    // the authoritative projection even when the user left the
                    // Issues page while a long repair ran.
                    cx.dispatch(AppCommand::RefreshIssues);
                    self.refreshing = true;
                }
            }
            ActionEvent::Rejected { message } => cx.status(message.clone()),
            _ => {}
        }
    }
}
