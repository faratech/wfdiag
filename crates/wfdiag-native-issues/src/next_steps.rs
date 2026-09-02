//! "Do this first": the detected issues a user should act on first, ranked
//! by severity and by how directly the vetted remediation fixes them. Pure
//! and deterministic — the same projection always yields the same list.

use crate::issue_catalog::{Issue, IssueSeverity};
use crate::projection::IssueProjection;
use wfdiag_remediation_catalog::{RemediationSummary, RemediationTier};

/// How many "do this first" steps the Issues page shows.
pub const DO_THIS_FIRST_LIMIT: usize = 3;

/// A fix that lives inside this app rather than in the remediation catalog:
/// the rules about resource pressure send the user to the Processes page,
/// sorted so the culprit is the first row, instead of to Task Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InAppAction {
    ProcessesByCpu,
    ProcessesByMemory,
}

impl InAppAction {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProcessesByCpu => "Show top processes by CPU",
            Self::ProcessesByMemory => "Show top processes by memory",
        }
    }
}

/// The in-app action for a rule, keyed by issue id. Only rules without a
/// catalog remediation have one (see `in_app_actions_belong_to_rules_without_a_remediation`).
#[must_use]
pub fn in_app_action(issue_id: &str) -> Option<InAppAction> {
    match issue_id {
        "high_cpu_usage" => Some(InAppAction::ProcessesByCpu),
        "high_memory_usage" | "page_file_pressure" => Some(InAppAction::ProcessesByMemory),
        _ => None,
    }
}

/// One "do this first" row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextStep {
    pub issue_id: String,
    pub title: String,
    pub severity: IssueSeverity,
    /// The vetted remediation to offer, when the issue has one.
    pub remediation: Option<RemediationSummary>,
    /// The in-app destination to offer instead, when the rule has one.
    pub in_app: Option<InAppAction>,
    pub needs_admin: bool,
    /// Why it is on the list ("Critical · one-click safe fix").
    pub why: String,
}

const fn severity_rank(severity: IssueSeverity) -> u8 {
    match severity {
        IssueSeverity::Critical => 0,
        IssueSeverity::Warning => 1,
        IssueSeverity::Info => 2,
        IssueSeverity::Ok => 3,
    }
}

const fn tier_rank(tier: RemediationTier) -> u8 {
    match tier {
        RemediationTier::AutoSafe => 0,
        RemediationTier::Repair => 1,
        RemediationTier::OpenTool => 2,
    }
}

fn next_step(issue: &Issue) -> NextStep {
    let remediation = issue.remediation.clone();
    let needs_admin = remediation
        .as_ref()
        .is_some_and(|remediation| remediation.admin_required);
    let severity = match issue.severity {
        IssueSeverity::Critical => "Critical",
        IssueSeverity::Warning => "Warning",
        IssueSeverity::Info | IssueSeverity::Ok => "Worth a look",
    };
    let in_app = in_app_action(&issue.id);
    let action = match remediation.as_ref().map(|remediation| remediation.tier) {
        Some(RemediationTier::AutoSafe) => "one-click safe fix",
        Some(RemediationTier::Repair) => "built-in repair, confirmation required",
        Some(RemediationTier::OpenTool) => "opens the right Windows tool",
        None if in_app.is_some() => "see the culprits on the Processes page",
        None => "follow the recommendation",
    };
    NextStep {
        issue_id: issue.id.clone(),
        title: issue.title.clone(),
        severity: issue.severity,
        remediation,
        in_app,
        needs_admin,
        why: format!("{severity} · {action}"),
    }
}

/// The detected issues a user should act on first, most valuable first:
/// severity, then whether a vetted remediation exists, then built-in fixes
/// before tool handoffs, then catalog order (the projection's order).
#[must_use]
pub fn do_this_first(projection: &IssueProjection<'_>, limit: usize) -> Vec<NextStep> {
    let mut ranked: Vec<(usize, &Issue)> =
        projection.detected.iter().copied().enumerate().collect();
    ranked.sort_by_key(|(position, issue)| {
        (
            severity_rank(issue.severity),
            u8::from(issue.remediation.is_none() && in_app_action(&issue.id).is_none()),
            issue
                .remediation
                .as_ref()
                .map_or(u8::MAX, |remediation| tier_rank(remediation.tier)),
            *position,
        )
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, issue)| next_step(issue))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_catalog::IssueStatus;
    use crate::projection::project_issues;

    fn issue(id: &str, severity: IssueSeverity, status: IssueStatus) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity,
            status,
            title: format!("Issue {id}"),
            description: String::new(),
            recommendation: String::new(),
            detected: status == IssueStatus::Detected,
            source_tasks: None,
            remediation: None,
        }
    }

    fn with_remediation(mut issue: Issue, id: &str, tier: RemediationTier, admin: bool) -> Issue {
        issue.remediation = Some(RemediationSummary {
            id: id.to_string(),
            label: id.to_string(),
            description: String::new(),
            tier,
            admin_required: admin,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            batch_eligible: false,
            cancellable: false,
        });
        issue
    }

    #[test]
    fn do_this_first_prefers_built_in_fixes_over_tool_handoffs() {
        let issues = vec![
            with_remediation(
                issue("tool", IssueSeverity::Critical, IssueStatus::Detected),
                "open_disk_cleanup",
                RemediationTier::OpenTool,
                false,
            ),
            issue("no_fix", IssueSeverity::Critical, IssueStatus::Detected),
            with_remediation(
                issue("repair", IssueSeverity::Critical, IssueStatus::Detected),
                "sfc_scannow",
                RemediationTier::Repair,
                true,
            ),
            with_remediation(
                issue("warn", IssueSeverity::Warning, IssueStatus::Detected),
                "flush_dns",
                RemediationTier::AutoSafe,
                false,
            ),
            issue("info", IssueSeverity::Info, IssueStatus::Detected),
            issue("ok", IssueSeverity::Ok, IssueStatus::Ok),
        ];
        let steps = do_this_first(&project_issues(&issues), DO_THIS_FIRST_LIMIT);
        assert_eq!(
            steps
                .iter()
                .map(|step| step.issue_id.as_str())
                .collect::<Vec<_>>(),
            ["repair", "tool", "no_fix"]
        );
        assert!(steps[0].needs_admin);
        assert_eq!(
            steps[0].why,
            "Critical · built-in repair, confirmation required"
        );
        assert_eq!(steps[2].why, "Critical · follow the recommendation");
        let all = do_this_first(&project_issues(&issues), 10);
        assert_eq!(all.len(), 5, "clear checks are never steps");
        assert_eq!(all[3].issue_id, "warn");
        assert_eq!(all[4].why, "Worth a look · follow the recommendation");
    }

    #[test]
    fn in_app_actions_belong_to_rules_without_a_remediation() {
        let mut seen = 0;
        for spec in crate::issue_catalog::catalog() {
            if let Some(action) = in_app_action(spec.id) {
                seen += 1;
                assert!(
                    spec.remediation_id.is_none(),
                    "{}: an in-app action and a catalog remediation would compete for one button",
                    spec.id
                );
                assert!(!action.label().is_empty());
            }
        }
        assert_eq!(
            seen, 3,
            "high_cpu_usage, high_memory_usage, page_file_pressure"
        );
        assert!(in_app_action("low_disk_space").is_none());
    }

    #[test]
    fn a_rule_with_an_in_app_action_ranks_like_one_with_a_fix() {
        let issues = vec![
            issue(
                "unsigned_drivers",
                IssueSeverity::Warning,
                IssueStatus::Detected,
            ),
            issue(
                "high_cpu_usage",
                IssueSeverity::Warning,
                IssueStatus::Detected,
            ),
        ];
        let steps = do_this_first(&project_issues(&issues), DO_THIS_FIRST_LIMIT);
        assert_eq!(steps[0].issue_id, "high_cpu_usage");
        assert_eq!(steps[0].in_app, Some(InAppAction::ProcessesByCpu));
        assert_eq!(
            steps[0].why,
            "Warning · see the culprits on the Processes page"
        );
        assert_eq!(steps[1].in_app, None);
    }
}
