//! The automation layer's state: what it is running, for whom, and what it
//! has already tried. Pure; the service drives it.
//!
//! Two modes: **replan** recomputes the safe-fix plan from the live issue
//! projection after every run (so a fix that cleared two issues is not run
//! twice, and a fix that failed is not retried), and **fixed** runs a list
//! decided up front (an assistant-staged action, the safe part of a fix
//! plan). Either way every proposal goes through the broker with the plain
//! review approval, which is what keeps Repair-tier actions out.

use std::collections::{HashSet, VecDeque};
use wfdiag_native_issues::auto_fix::{DeferReason, DeferredFix, SafeFixPlan, SafeLane, classify};
use wfdiag_native_issues::{Issue, IssueStatus, RemediationSummary};
use wfdiag_native_remediation::broker::ActionRequest;

use crate::event::SafeFixOrigin;

/// How the next group of actions is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationMode {
    /// Recompute the plan from the current projection before each group.
    Replan,
    /// Run these groups, in order, then stop.
    Fixed(VecDeque<Vec<ActionRequest>>),
}

/// The automation layer's state. `origin` is `None` while idle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Automation {
    /// Who asked; `None` while idle.
    pub origin: Option<SafeFixOrigin>,
    /// How the next group is chosen.
    pub mode: AutomationMode,
    /// A prepare or approve the automation asked for is in flight, so the
    /// reply must not be shown for review.
    pub pending: bool,
    /// A run succeeded and the issue projection is being refreshed; the
    /// next group waits for it so its proposal is built on current evidence.
    pub await_projection: bool,
    /// Remediation ids already attempted in this session (never retried).
    pub attempted: HashSet<String>,
    /// Proposals that ran to a terminal state.
    pub runs: usize,
    /// Proposals in which at least one action succeeded.
    pub succeeded: usize,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            origin: None,
            mode: AutomationMode::Replan,
            pending: false,
            await_projection: false,
            attempted: HashSet::new(),
            runs: 0,
            succeeded: 0,
        }
    }
}

impl Automation {
    /// Begin a session.
    pub fn start(&mut self, origin: SafeFixOrigin, mode: AutomationMode) {
        *self = Self {
            origin: Some(origin),
            mode,
            ..Self::default()
        };
    }

    /// Whether a session is in progress.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.origin.is_some()
    }

    /// End the session, returning who asked.
    pub fn finish(&mut self) -> Option<SafeFixOrigin> {
        let origin = self.origin.take();
        *self = Self::default();
        origin
    }

    /// The next group to stage, or `None` when the session is complete.
    /// `plan` is the plan for the *current* projection (replan mode only).
    pub fn next_group(&mut self, plan: &SafeFixPlan) -> Option<Vec<ActionRequest>> {
        let group = match &mut self.mode {
            AutomationMode::Fixed(groups) => groups.pop_front()?,
            AutomationMode::Replan => {
                let batch: Vec<ActionRequest> = plan
                    .batch
                    .iter()
                    .filter(|action| !self.attempted.contains(&action.remediation_id))
                    .map(request)
                    .collect();
                if batch.is_empty() {
                    let single = plan
                        .singles
                        .iter()
                        .find(|action| !self.attempted.contains(&action.remediation_id))?;
                    vec![request(single)]
                } else {
                    batch
                }
            }
        };
        for action in &group {
            self.attempted.insert(action.remediation_id.clone());
        }
        Some(group)
    }
}

fn request(action: &wfdiag_native_issues::auto_fix::SafeFixAction) -> ActionRequest {
    ActionRequest {
        remediation_id: action.remediation_id.clone(),
        issue_id: Some(action.issue_id.clone()),
    }
}

/// Split a list of intended actions (a fix plan, an assistant request) into
/// the groups the automation may run and the ones the user must approve.
///
/// `resolve` maps an action to its catalog summary and, for an issue-bound
/// action, confirms the issue is currently detected with that remediation.
#[must_use]
pub fn fixed_groups(
    actions: &[ActionRequest],
    issues: &[Issue],
    remediations: &[RemediationSummary],
    is_admin: bool,
    max_batch: usize,
) -> (VecDeque<Vec<ActionRequest>>, Vec<DeferredFix>) {
    let mut batch = Vec::new();
    let mut singles = Vec::new();
    let mut deferred = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for action in actions {
        if !seen.insert(action.remediation_id.as_str()) {
            continue;
        }
        let issue = action
            .issue_id
            .as_deref()
            .and_then(|issue_id| issues.iter().find(|issue| issue.id == issue_id));
        let title = issue.map_or_else(
            || action.remediation_id.clone(),
            |issue| issue.title.clone(),
        );
        let deferred_as = |reason: DeferReason| DeferredFix {
            issue_id: action.issue_id.clone().unwrap_or_default(),
            title: title.clone(),
            remediation_id: Some(action.remediation_id.clone()),
            reason,
        };
        let Some(remediation) = remediations
            .iter()
            .find(|remediation| remediation.id == action.remediation_id)
        else {
            deferred.push(deferred_as(DeferReason::NoFix));
            continue;
        };
        let bound_ok = match issue {
            Some(issue) => {
                issue.status == IssueStatus::Detected
                    && issue
                        .remediation
                        .as_ref()
                        .is_some_and(|mapped| mapped.id == remediation.id)
            }
            None => action.issue_id.is_none() && remediation.maintenance,
        };
        if !bound_ok {
            deferred.push(deferred_as(DeferReason::NoFix));
            continue;
        }
        match classify(remediation, is_admin) {
            Ok(SafeLane::Batch) if batch.len() < max_batch => batch.push(action.clone()),
            Ok(SafeLane::Batch | SafeLane::Single) => singles.push(vec![action.clone()]),
            Err(reason) => deferred.push(deferred_as(reason)),
        }
    }
    let mut groups = VecDeque::new();
    if !batch.is_empty() {
        groups.push_back(batch);
    }
    groups.extend(singles);
    (groups, deferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_issues::auto_fix::SafeFixAction;
    use wfdiag_native_issues::{IssueSeverity, remediation_summaries};

    fn action(issue: &str, remediation: &str) -> SafeFixAction {
        SafeFixAction {
            issue_id: issue.to_string(),
            remediation_id: remediation.to_string(),
            label: remediation.to_string(),
        }
    }

    #[test]
    fn replan_never_retries_an_attempted_remediation() {
        let mut automation = Automation::default();
        automation.start(SafeFixOrigin::User, AutomationMode::Replan);
        let plan = SafeFixPlan {
            batch: vec![
                action("a", "flush_dns"),
                action("b", "update_defender_signatures"),
            ],
            singles: vec![action("c", "defender_quick_scan")],
            deferred: vec![],
        };
        let first = automation.next_group(&plan).unwrap();
        assert_eq!(first.len(), 2);
        let second = automation.next_group(&plan).unwrap();
        assert_eq!(second[0].remediation_id, "defender_quick_scan");
        assert!(
            automation.next_group(&plan).is_none(),
            "everything was attempted"
        );
        assert_eq!(automation.finish(), Some(SafeFixOrigin::User));
        assert!(!automation.active());
    }

    #[test]
    fn fixed_groups_keep_only_bound_safe_actions() {
        let remediations = remediation_summaries();
        let summary = |id: &str| {
            remediations
                .iter()
                .find(|summary| summary.id == id)
                .cloned()
                .unwrap()
        };
        let issue = |id: &str, remediation: &str| Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: format!("Issue {id}"),
            description: String::new(),
            recommendation: String::new(),
            detected: true,
            source_tasks: None,
            remediation: Some(summary(remediation)),
        };
        let issues = vec![
            issue("defender_definitions_stale", "update_defender_signatures"),
            issue("low_disk_space", "clear_temp_files"),
        ];
        let request = |remediation: &str, issue: Option<&str>| ActionRequest {
            remediation_id: remediation.to_string(),
            issue_id: issue.map(str::to_string),
        };
        let (groups, deferred) = fixed_groups(
            &[
                request(
                    "update_defender_signatures",
                    Some("defender_definitions_stale"),
                ),
                request("clear_temp_files", Some("low_disk_space")),
                request("flush_dns", None),
                request("flush_dns", Some("dns_resolution_failing")),
                request("sfc_scannow", None),
            ],
            &issues,
            &remediations,
            false,
            5,
        );
        let groups: Vec<Vec<String>> = groups
            .into_iter()
            .map(|group| group.into_iter().map(|a| a.remediation_id).collect())
            .collect();
        assert_eq!(groups, [vec!["update_defender_signatures", "flush_dns"]]);
        assert_eq!(
            deferred
                .iter()
                .map(|d| (d.remediation_id.as_deref(), d.reason))
                .collect::<Vec<_>>(),
            [
                (Some("clear_temp_files"), DeferReason::NeedsConfirmation),
                (Some("sfc_scannow"), DeferReason::NeedsConfirmation),
            ]
        );
    }
}
