//! The safe-fix planner: which detected issues can be fixed without asking,
//! which cannot and why. Pure and deterministic; the facade's automation
//! layer runs the plan through the broker, and the broker — not this module
//! — is what keeps a Repair-tier action from running unconfirmed.
//!
//! "Safe" means the catalog's `AutoSafe` tier with no restart. An
//! administrator-only safe fix is run only when the process is elevated;
//! a long-running one (a Defender scan) runs on its own rather than in a
//! batch, matching the broker's batch rules.

use crate::issue_catalog::Issue;
use crate::next_steps::{do_this_first, in_app_action};
use crate::projection::project_issues;
use std::collections::HashSet;
use wfdiag_remediation_catalog::{RemediationSummary, RemediationTier};

/// Why a detected issue is left for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    /// A Repair-tier fix: the user confirms it once.
    NeedsConfirmation,
    /// A safe fix that needs administrator rights this process lacks.
    NeedsAdministrator,
    /// The fix would need a restart; the user picks the moment.
    RequiresRestart,
    /// The remediation opens a Windows tool for the user to act in.
    OpensTool,
    /// The fix is a page in this app (the Processes page).
    InApp,
    /// No vetted fix exists; the recommendation is text.
    NoFix,
}

impl DeferReason {
    /// A short phrase for status lines and the assistant.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NeedsConfirmation => "needs your confirmation",
            Self::NeedsAdministrator => "needs administrator rights",
            Self::RequiresRestart => "needs a restart",
            Self::OpensTool => "opens a Windows tool",
            Self::InApp => "see the Processes page",
            Self::NoFix => "no built-in fix",
        }
    }
}

/// A detected issue the plan does not run automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredFix {
    pub issue_id: String,
    pub title: String,
    pub remediation_id: Option<String>,
    pub reason: DeferReason,
}

/// One fix the plan runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFixAction {
    pub issue_id: String,
    pub remediation_id: String,
    pub label: String,
}

/// The plan: `batch` runs as one proposal, each of `singles` on its own,
/// `deferred` waits for the user. Every list is in "do this first" order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafeFixPlan {
    pub batch: Vec<SafeFixAction>,
    pub singles: Vec<SafeFixAction>,
    pub deferred: Vec<DeferredFix>,
}

impl SafeFixPlan {
    /// How many fixes the plan runs.
    #[must_use]
    pub fn action_count(&self) -> usize {
        self.batch.len() + self.singles.len()
    }

    /// Every fix the plan runs, batch first.
    pub fn actions(&self) -> impl Iterator<Item = &SafeFixAction> {
        self.batch.iter().chain(self.singles.iter())
    }
}

/// Where a remediation may run without asking, or why it may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeLane {
    /// Safe, quick, non-admin: may share a proposal with others.
    Batch,
    /// Safe but long-running or administrator-only (while elevated): runs
    /// as its own proposal.
    Single,
}

/// Classify one remediation for automatic execution.
///
/// # Errors
/// The reason the remediation must wait for the user.
pub fn classify(remediation: &RemediationSummary, is_admin: bool) -> Result<SafeLane, DeferReason> {
    match remediation.tier {
        RemediationTier::OpenTool => Err(DeferReason::OpensTool),
        RemediationTier::Repair => Err(DeferReason::NeedsConfirmation),
        RemediationTier::AutoSafe => {
            if remediation.requires_restart {
                Err(DeferReason::RequiresRestart)
            } else if remediation.admin_required && !is_admin {
                Err(DeferReason::NeedsAdministrator)
            } else if remediation.batch_eligible {
                Ok(SafeLane::Batch)
            } else {
                Ok(SafeLane::Single)
            }
        }
    }
}

/// Plan the safe fixes for the detected issues in `issues`.
///
/// One remediation runs once even when several issues map to it; the batch
/// holds at most `max_batch` actions and the rest run singly.
#[must_use]
pub fn safe_fix_plan(issues: &[Issue], is_admin: bool, max_batch: usize) -> SafeFixPlan {
    let projection = project_issues(issues);
    let ranked = do_this_first(&projection, usize::MAX);
    let mut plan = SafeFixPlan::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for step in &ranked {
        let Some(issue) = issues.iter().find(|issue| issue.id == step.issue_id) else {
            continue;
        };
        let Some(remediation) = issue.remediation.as_ref() else {
            let reason = if in_app_action(&issue.id).is_some() {
                DeferReason::InApp
            } else {
                DeferReason::NoFix
            };
            plan.deferred.push(DeferredFix {
                issue_id: issue.id.clone(),
                title: issue.title.clone(),
                remediation_id: None,
                reason,
            });
            continue;
        };
        if !seen.insert(remediation.id.as_str()) {
            continue;
        }
        let action = SafeFixAction {
            issue_id: issue.id.clone(),
            remediation_id: remediation.id.clone(),
            label: remediation.label.clone(),
        };
        match classify(remediation, is_admin) {
            Ok(SafeLane::Batch) if plan.batch.len() < max_batch => plan.batch.push(action),
            Ok(SafeLane::Batch | SafeLane::Single) => plan.singles.push(action),
            Err(reason) => plan.deferred.push(DeferredFix {
                issue_id: issue.id.clone(),
                title: issue.title.clone(),
                remediation_id: Some(remediation.id.clone()),
                reason,
            }),
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_catalog::{IssueSeverity, IssueStatus};
    use wfdiag_remediation_catalog as catalog;

    fn detected(id: &str, severity: IssueSeverity, remediation: Option<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity,
            status: IssueStatus::Detected,
            title: format!("Issue {id}"),
            description: String::new(),
            recommendation: String::new(),
            detected: true,
            source_tasks: None,
            remediation: remediation.map(|id| catalog::find(id).expect("catalog id").summary()),
        }
    }

    #[test]
    fn only_auto_safe_fixes_run_and_repairs_are_deferred_with_a_reason() {
        let issues = vec![
            detected(
                "low_disk_space",
                IssueSeverity::Critical,
                Some("clear_temp_files"),
            ),
            detected(
                "dns_resolution_failing",
                IssueSeverity::Warning,
                Some("flush_dns"),
            ),
            detected(
                "device_manager_errors",
                IssueSeverity::Warning,
                Some("open_device_manager"),
            ),
            detected("high_cpu_usage", IssueSeverity::Warning, None),
            detected("unsigned_drivers", IssueSeverity::Info, None),
        ];
        let plan = safe_fix_plan(&issues, false, 5);
        assert_eq!(
            plan.batch
                .iter()
                .map(|a| a.remediation_id.as_str())
                .collect::<Vec<_>>(),
            ["flush_dns"]
        );
        assert!(plan.singles.is_empty());
        assert_eq!(
            plan.deferred
                .iter()
                .map(|d| (d.issue_id.as_str(), d.reason))
                .collect::<Vec<_>>(),
            [
                ("low_disk_space", DeferReason::NeedsConfirmation),
                ("device_manager_errors", DeferReason::OpensTool),
                ("high_cpu_usage", DeferReason::InApp),
                ("unsigned_drivers", DeferReason::NoFix),
            ]
        );
        assert_eq!(plan.action_count(), 1);
    }

    #[test]
    fn admin_only_and_long_running_safe_fixes_run_singly_and_only_when_allowed() {
        let issues = vec![
            detected(
                "firewall_disabled",
                IssueSeverity::Critical,
                Some("enable_firewall"),
            ),
            detected(
                "defender_quick_scan_overdue",
                IssueSeverity::Info,
                Some("defender_quick_scan"),
            ),
            detected(
                "defender_definitions_stale",
                IssueSeverity::Warning,
                Some("update_defender_signatures"),
            ),
        ];
        let standard = safe_fix_plan(&issues, false, 5);
        assert_eq!(
            standard.batch[0].remediation_id,
            "update_defender_signatures"
        );
        assert_eq!(standard.singles[0].remediation_id, "defender_quick_scan");
        assert_eq!(standard.deferred[0].reason, DeferReason::NeedsAdministrator);
        assert_eq!(
            standard.deferred[0].remediation_id.as_deref(),
            Some("enable_firewall")
        );

        let elevated = safe_fix_plan(&issues, true, 5);
        assert!(elevated.deferred.is_empty());
        assert_eq!(
            elevated
                .singles
                .iter()
                .map(|a| a.remediation_id.as_str())
                .collect::<Vec<_>>(),
            ["enable_firewall", "defender_quick_scan"],
            "critical first, then the long scan"
        );
    }

    #[test]
    fn a_remediation_shared_by_two_issues_runs_once_and_the_batch_is_capped() {
        let issues = vec![
            detected("no_internet", IssueSeverity::Critical, Some("flush_dns")),
            detected(
                "dns_resolution_failing",
                IssueSeverity::Warning,
                Some("flush_dns"),
            ),
            detected(
                "defender_definitions_stale",
                IssueSeverity::Warning,
                Some("update_defender_signatures"),
            ),
        ];
        let plan = safe_fix_plan(&issues, false, 1);
        assert_eq!(plan.batch.len(), 1);
        assert_eq!(plan.batch[0].issue_id, "no_internet");
        assert_eq!(plan.singles[0].remediation_id, "update_defender_signatures");
        assert_eq!(plan.action_count(), 2);
    }

    #[test]
    fn every_catalog_entry_classifies_consistently_with_the_broker_batch_rule() {
        for metadata in catalog::catalog() {
            let summary = metadata.summary();
            match classify(&summary, false) {
                Ok(SafeLane::Batch) => assert!(summary.batch_eligible, "{}", summary.id),
                Ok(SafeLane::Single) => {
                    assert!(!summary.batch_eligible && summary.tier == RemediationTier::AutoSafe);
                }
                Err(reason) => assert!(
                    summary.tier != RemediationTier::AutoSafe
                        || reason == DeferReason::NeedsAdministrator
                        || reason == DeferReason::RequiresRestart,
                    "{}: {reason:?}",
                    summary.id
                ),
            }
        }
    }
}
