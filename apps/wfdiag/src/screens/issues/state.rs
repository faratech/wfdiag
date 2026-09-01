//! The Issues screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::app::state::{FixPlanActionSelection, IssuePrioritizationDisplay};
use std::collections::HashSet;
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_issues::{Issue, RemediationSummary};
use wfdiag_native_remediation::runtime::ActionRunSummary;

/// Everything the Issues page renders.
#[derive(Default)]
pub(crate) struct IssuesScreen {
    pub(crate) issues: Vec<Issue>,
    pub(crate) maintenance: Vec<RemediationSummary>,
    pub(crate) error: Option<String>,
    pub(crate) refreshing: bool,
    /// The evidence the visible issue projection came from, so the page can
    /// grey out a projection that a newer scan has already replaced.
    pub(crate) projected_session_id: Option<String>,
    pub(crate) prioritization: IssuePrioritizationDisplay,
    pub(crate) fix_plan: Option<ValidatedFixPlan>,
    pub(crate) fix_plan_busy: bool,
    pub(crate) fix_plan_error: Option<String>,

    // ---- remediation runs ------------------------------------------------
    pub(crate) active_run: Option<ActionRunSummary>,
    pub(crate) run_history: Vec<ActionRunSummary>,
    pub(crate) expanded_runs: HashSet<String>,
    pub(crate) busy: bool,
}

impl IssuesScreen {
    /// Whether the visible projection describes the visible scan.
    ///
    /// The engine only publishes issues for evidence it has committed, so
    /// "current" is simply: the projection names the visible scan.
    pub(crate) fn projection_current(&self, visible_session_id: Option<&str>) -> bool {
        self.projected_session_id.as_deref() == visible_session_id
    }
}

/// Everything the Issues page can ask for.
#[derive(Clone)]
pub(crate) enum IssuesMsg {
    RunRemediation(String),
    AskAiAboutIssue(String),
    Prioritize,
    CancelPrioritization,
    ProposeFixPlan,
    CancelFixPlan,
    ReviewFixPlanActions(FixPlanActionSelection),
    CancelActionRun,
    RunExpandedChanged { run_id: String, expanded: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projection_is_current_only_while_it_names_the_visible_scan() {
        let screen = IssuesScreen {
            projected_session_id: Some("session-3".to_string()),
            ..IssuesScreen::default()
        };
        assert!(screen.projection_current(Some("session-3")));
        assert!(!screen.projection_current(Some("session-4")));
        assert!(!screen.projection_current(None));
        assert!(IssuesScreen::default().projection_current(None));
    }
}
