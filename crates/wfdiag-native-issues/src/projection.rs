//! Framework-neutral issue-state projection shared by the shells.
//!
//! Worker ownership and UI scheduling stay in the shell. This module owns the
//! rest of that boundary: immutable request preparation, stale-completion
//! guards, Store-compatible issue projections, and canonical remediation
//! snapshots.

use crate::{
    Issue, IssueDetectionCompleted, IssueDetectionRequest, IssueSeverity, IssueStatus,
    RemediationSummary, SharedScanEvidence, Timestamp, remediation_summaries,
};

/// Identity captured when an issue-detection request is enqueued.
///
/// A request id alone is insufficient: it can wrap, and a late response for
/// restored scan evidence must not replace issues derived from a newer
/// committed scan. `committed_epoch` and `session_id` identify that evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingIssueDetection {
    pub request_id: u64,
    pub committed_epoch: u64,
    pub session_id: String,
}

/// A worker request paired with the guard that must be retained by the shell.
#[derive(Clone, Debug)]
pub struct PreparedIssueDetection {
    pub pending: PendingIssueDetection,
    pub request: IssueDetectionRequest,
}

/// Build a worker request directly from the authoritative diagnostic result
/// map. `DiagnosticOutput` is a re-export of `TaskResult`, so callers should
/// move the map here without converting it through UI rows or JSON.
#[must_use]
pub fn prepare_issue_detection(
    request_id: u64,
    committed_epoch: u64,
    session_id: String,
    results: SharedScanEvidence,
    now: Timestamp,
    temp_file_count: Option<usize>,
) -> PreparedIssueDetection {
    PreparedIssueDetection {
        pending: PendingIssueDetection {
            request_id,
            committed_epoch,
            session_id,
        },
        request: IssueDetectionRequest {
            request_id,
            results,
            now,
            temp_file_count,
        },
    }
}

/// Advance a request or committed-evidence sequence while reserving zero as
/// the uninitialized value and never reusing an identity after exhaustion.
/// Refusing the practically unreachable overflow keeps an ancient worker
/// response from matching a new request after a numeric wrap.
#[must_use]
pub fn advance_nonzero_generation(sequence: &mut u64) -> Option<u64> {
    let next = sequence.checked_add(1)?;
    *sequence = next;
    Some(next)
}

/// Consume `pending` only when the completion belongs to the current
/// committed evidence. A stale response deliberately leaves a newer pending
/// request intact.
pub fn take_current_issue_completion(
    pending: &mut Option<PendingIssueDetection>,
    completion: &IssueDetectionCompleted,
    current_committed_epoch: u64,
    current_session_id: Option<&str>,
) -> Option<PendingIssueDetection> {
    let matches_current = pending.as_ref().is_some_and(|candidate| {
        candidate.request_id == completion.request_id
            && candidate.committed_epoch == current_committed_epoch
            && current_session_id == Some(candidate.session_id.as_str())
    });

    matches_current.then(|| pending.take()).flatten()
}

/// Validate an asynchronous request-preparation callback before its request is
/// enqueued. The callback must still be the shell's pending request and its
/// evidence identity must still be the current committed scan.
#[must_use]
pub fn pending_issue_preparation_is_current(
    pending: Option<&PendingIssueDetection>,
    prepared: &PendingIssueDetection,
    current_committed_epoch: u64,
    current_session_id: Option<&str>,
) -> bool {
    pending == Some(prepared)
        && prepared.committed_epoch == current_committed_epoch
        && current_session_id == Some(prepared.session_id.as_str())
}

/// Whether the visible issue projection was produced from the shell's current
/// committed diagnostic evidence. A new complete scan makes the old issues
/// stale until its guarded detection succeeds; Ctrl+R does not change this
/// identity because it re-evaluates the same evidence.
#[must_use]
pub fn issue_projection_matches_evidence(
    projected_epoch: Option<u64>,
    projected_session_id: Option<&str>,
    current_committed_epoch: u64,
    current_session_id: Option<&str>,
) -> bool {
    projected_epoch == Some(current_committed_epoch)
        && projected_session_id.is_some()
        && projected_session_id == current_session_id
}

/// Counts shared by the Issues summary, collapsible sections, and nav badge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IssueCounts {
    pub total: usize,
    pub detected: usize,
    pub critical: usize,
    pub warnings: usize,
    pub passed: usize,
    pub unknown: usize,
}

impl IssueCounts {
    /// Store behavior: the badge represents detected problems only. Unknown
    /// and skipped checks never turn into a warning badge.
    #[must_use]
    pub const fn nav_badge_count(self) -> Option<usize> {
        if self.detected == 0 {
            None
        } else {
            Some(self.detected)
        }
    }

    /// Exact Store 2.5.8 summary wording and singular/plural behavior.
    #[must_use]
    pub fn summary_text(self) -> String {
        if self.detected == 0 {
            if self.unknown > 0 {
                return format!(
                    "No issues detected in completed checks · {} checks passed · {} couldn’t verify",
                    self.passed, self.unknown
                );
            }
            return format!(
                "All clear — no issues detected · {} checks passed",
                self.passed
            );
        }

        let issue_noun = if self.detected == 1 {
            "issue"
        } else {
            "issues"
        };
        let needs = if self.detected == 1 { "needs" } else { "need" };
        let critical = if self.critical > 0 {
            format!(" · {} critical", self.critical)
        } else {
            String::new()
        };
        let warnings = if self.warnings > 0 {
            let warning_noun = if self.warnings == 1 {
                "warning"
            } else {
                "warnings"
            };
            format!(" · {} {warning_noun}", self.warnings)
        } else {
            String::new()
        };
        format!(
            "{} {issue_noun} {needs} attention · detected by the latest scan{critical}{warnings}",
            self.detected
        )
    }
}

/// Stable partitions over one immutable issue result set.
#[derive(Debug, Eq, PartialEq)]
pub struct IssueProjection<'a> {
    pub detected: Vec<&'a Issue>,
    pub passed: Vec<&'a Issue>,
    pub unknown: Vec<&'a Issue>,
    pub counts: IssueCounts,
}

/// One issue that became Critical since the previous projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCriticalIssue {
    pub id: String,
    pub title: String,
}

/// The issues that are Critical now and were explicitly *not* Critical in
/// the baseline: a clear check, or a detected Warning/Info that escalated.
/// A baseline that could not decide (Unknown, Skipped, missing) stays silent,
/// so the first Full Scan after Quick Scans never announces the admin-only
/// rules it merely turned on.
#[must_use]
pub fn newly_critical(baseline: &[Issue], current: &[Issue]) -> Vec<NewCriticalIssue> {
    current
        .iter()
        .filter(|issue| {
            (issue.detected || issue.status == IssueStatus::Detected)
                && issue.severity == IssueSeverity::Critical
        })
        .filter(|issue| {
            baseline
                .iter()
                .find(|previous| previous.id == issue.id)
                .is_some_and(|previous| match previous.status {
                    IssueStatus::Ok => true,
                    IssueStatus::Detected => previous.severity != IssueSeverity::Critical,
                    IssueStatus::Unknown | IssueStatus::Skipped => false,
                })
        })
        .map(|issue| NewCriticalIssue {
            id: issue.id.clone(),
            title: issue.title.clone(),
        })
        .collect()
}

/// A friendlier reason for an Unknown check whose source is the opt-in
/// connectivity test: "not run" only because the user has not turned it on.
#[must_use]
pub fn unknown_check_hint(issue: &Issue, network_tests_enabled: bool) -> Option<&'static str> {
    if network_tests_enabled || issue.status != IssueStatus::Unknown {
        return None;
    }
    issue
        .source_tasks
        .as_ref()
        .is_some_and(|tasks| tasks.iter().any(|task| task == "network_path"))
        .then_some("Connectivity tests are off. Turn them on in Settings to run this check (it sends three small probes).")
}

/// The diagnostic tasks that would decide the unverified checks in `unknown`:
/// their source tasks, deduplicated, without the privacy-gated connectivity
/// test while it is switched off.
#[must_use]
pub fn missing_check_tasks(unknown: &[&Issue], network_tests_enabled: bool) -> Vec<String> {
    let mut tasks: Vec<String> = Vec::new();
    for issue in unknown {
        for task in issue.source_tasks.iter().flatten() {
            if !network_tests_enabled && task == "network_path" {
                continue;
            }
            if !tasks.contains(task) {
                tasks.push(task.clone());
            }
        }
    }
    tasks
}

/// Match the Store's compatibility projection. `detected` and the explicit
/// status are both honored; legacy `Skipped` is grouped with `Unknown` and is
/// never presented as a passed check.
#[must_use]
pub fn project_issues(issues: &[Issue]) -> IssueProjection<'_> {
    let mut detected = Vec::new();
    let mut passed = Vec::new();
    let mut unknown = Vec::new();
    let mut critical = 0;
    let mut warnings = 0;

    for issue in issues {
        if issue.detected || issue.status == IssueStatus::Detected {
            if issue.severity == IssueSeverity::Critical {
                critical += 1;
            } else if issue.severity == IssueSeverity::Warning {
                warnings += 1;
            }
            detected.push(issue);
        } else if matches!(issue.status, IssueStatus::Unknown | IssueStatus::Skipped) {
            unknown.push(issue);
        } else {
            passed.push(issue);
        }
    }

    let counts = IssueCounts {
        total: issues.len(),
        detected: detected.len(),
        critical,
        warnings,
        passed: passed.len(),
        unknown: unknown.len(),
    };
    IssueProjection {
        detected,
        passed,
        unknown,
        counts,
    }
}

/// One canonical read-only metadata snapshot for worker startup and UI
/// maintenance rendering. Both vectors preserve the shipping catalog order,
/// and maintenance entries retain every summary field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueMetadataSnapshot {
    pub remediations: Vec<RemediationSummary>,
    pub maintenance: Vec<RemediationSummary>,
}

#[must_use]
pub fn canonical_issue_metadata_snapshot() -> IssueMetadataSnapshot {
    let remediations = remediation_summaries();
    let maintenance = remediations
        .iter()
        .filter(|summary| summary.maintenance)
        .cloned()
        .collect();
    IssueMetadataSnapshot {
        remediations,
        maintenance,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IssueCounts, PendingIssueDetection, advance_nonzero_generation,
        canonical_issue_metadata_snapshot, issue_projection_matches_evidence, missing_check_tasks,
        pending_issue_preparation_is_current, prepare_issue_detection, project_issues,
        take_current_issue_completion,
    };
    use super::{newly_critical, unknown_check_hint};
    use crate::{
        Issue, IssueDetectionCompleted, IssueSeverity, IssueStatus, TaskResult, Timestamp,
        remediation_summaries,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_issue(id: &str, status: IssueStatus, severity: IssueSeverity, detected: bool) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity,
            status,
            title: id.to_string(),
            description: "description".to_string(),
            recommendation: "recommendation".to_string(),
            detected,
            source_tasks: None,
            remediation: None,
        }
    }

    fn completion(request_id: u64) -> IssueDetectionCompleted {
        IssueDetectionCompleted {
            request_id,
            issues: Vec::new(),
        }
    }

    #[test]
    fn generation_is_nonzero_unique_and_refuses_wrap() {
        let mut generation = 0;
        assert_eq!(advance_nonzero_generation(&mut generation), Some(1));
        assert_eq!(advance_nonzero_generation(&mut generation), Some(2));

        generation = u64::MAX;
        assert_eq!(advance_nonzero_generation(&mut generation), None);
        assert_eq!(generation, u64::MAX);
    }

    #[test]
    fn request_preparation_moves_the_authoritative_map_and_pairs_the_guard() {
        let result = Arc::new(TaskResult {
            success: true,
            output: "{\"ok\":true}".to_string(),
            error: None,
            duration_ms: 17,
        });
        let results = Arc::new(HashMap::from([(
            "logical_disk".to_string(),
            Arc::clone(&result),
        )]));
        let prepared = prepare_issue_detection(
            41,
            9,
            "session-9".to_string(),
            results,
            Timestamp::from_secs(1_788_076_800),
            Some(1_069),
        );

        assert_eq!(
            prepared.pending,
            PendingIssueDetection {
                request_id: 41,
                committed_epoch: 9,
                session_id: "session-9".to_string(),
            }
        );
        assert_eq!(prepared.request.request_id, 41);
        assert_eq!(prepared.request.results.get("logical_disk"), Some(&result));
        assert!(Arc::ptr_eq(
            prepared.request.results.get("logical_disk").unwrap(),
            &result
        ));
        assert_eq!(prepared.request.results.len(), 1);
        assert_eq!(prepared.request.now.secs, 1_788_076_800);
        assert_eq!(prepared.request.temp_file_count, Some(1_069));
    }

    #[test]
    fn exact_request_epoch_and_session_match_consumes_pending() {
        let mut pending = Some(PendingIssueDetection {
            request_id: 7,
            committed_epoch: 12,
            session_id: "scan-12".to_string(),
        });

        let matched =
            take_current_issue_completion(&mut pending, &completion(7), 12, Some("scan-12"));

        assert_eq!(matched.unwrap().session_id, "scan-12");
        assert!(pending.is_none());
    }

    #[test]
    fn every_stale_guard_dimension_rejects_without_clearing_newer_pending() {
        let expected = PendingIssueDetection {
            request_id: 7,
            committed_epoch: 12,
            session_id: "scan-12".to_string(),
        };
        let stale_cases = [
            (completion(6), 12, Some("scan-12")),
            (completion(7), 11, Some("scan-12")),
            (completion(7), 12, Some("scan-11")),
            (completion(7), 12, None),
        ];

        for (response, epoch, session_id) in stale_cases {
            let mut pending = Some(expected.clone());
            assert!(
                take_current_issue_completion(&mut pending, &response, epoch, session_id).is_none()
            );
            assert_eq!(pending, Some(expected.clone()));
        }
    }

    #[test]
    fn completion_without_a_pending_request_is_stale() {
        let mut pending = None;
        assert!(
            take_current_issue_completion(&mut pending, &completion(1), 1, Some("scan-1"))
                .is_none()
        );
        assert!(pending.is_none());
    }

    #[test]
    fn prepared_request_requires_pending_request_epoch_and_session_identity() {
        let expected = PendingIssueDetection {
            request_id: 7,
            committed_epoch: 12,
            session_id: "scan-12".to_string(),
        };
        assert!(pending_issue_preparation_is_current(
            Some(&expected),
            &expected,
            12,
            Some("scan-12"),
        ));

        for (pending, prepared, epoch, session) in [
            (
                Some(expected.clone()),
                PendingIssueDetection {
                    request_id: 6,
                    ..expected.clone()
                },
                12,
                Some("scan-12"),
            ),
            (
                Some(expected.clone()),
                PendingIssueDetection {
                    committed_epoch: 11,
                    ..expected.clone()
                },
                12,
                Some("scan-12"),
            ),
            (
                Some(expected.clone()),
                PendingIssueDetection {
                    session_id: "scan-11".to_string(),
                    ..expected.clone()
                },
                12,
                Some("scan-12"),
            ),
            (
                Some(expected.clone()),
                expected.clone(),
                11,
                Some("scan-12"),
            ),
            (
                Some(expected.clone()),
                expected.clone(),
                12,
                Some("scan-11"),
            ),
            (Some(expected.clone()), expected.clone(), 12, None),
            (None, expected.clone(), 12, Some("scan-12")),
        ] {
            assert!(!pending_issue_preparation_is_current(
                pending.as_ref(),
                &prepared,
                epoch,
                session,
            ));
        }
    }

    #[test]
    fn visible_projection_tracks_the_exact_committed_evidence() {
        assert!(issue_projection_matches_evidence(
            Some(12),
            Some("scan-12"),
            12,
            Some("scan-12"),
        ));
        for (epoch, projected_session, current_epoch, current_session) in [
            (None, Some("scan-12"), 12, Some("scan-12")),
            (Some(11), Some("scan-12"), 12, Some("scan-12")),
            (Some(12), Some("scan-11"), 12, Some("scan-12")),
            (Some(12), Some("scan-12"), 12, None),
        ] {
            assert!(!issue_projection_matches_evidence(
                epoch,
                projected_session,
                current_epoch,
                current_session,
            ));
        }
    }

    #[test]
    fn issue_projection_matches_store_compatibility_rules() {
        let issues = vec![
            test_issue(
                "flag-detected",
                IssueStatus::Ok,
                IssueSeverity::Critical,
                true,
            ),
            test_issue(
                "status-detected",
                IssueStatus::Detected,
                IssueSeverity::Warning,
                false,
            ),
            test_issue(
                "informational",
                IssueStatus::Detected,
                IssueSeverity::Info,
                true,
            ),
            test_issue("passed", IssueStatus::Ok, IssueSeverity::Ok, false),
            test_issue("unknown", IssueStatus::Unknown, IssueSeverity::Info, false),
            test_issue(
                "legacy-skipped",
                IssueStatus::Skipped,
                IssueSeverity::Ok,
                false,
            ),
        ];

        let projection = project_issues(&issues);

        assert_eq!(
            projection
                .detected
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<Vec<_>>(),
            ["flag-detected", "status-detected", "informational"]
        );
        assert_eq!(projection.passed[0].id, "passed");
        assert_eq!(
            projection
                .unknown
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<Vec<_>>(),
            ["unknown", "legacy-skipped"]
        );
        assert_eq!(
            projection.counts,
            IssueCounts {
                total: 6,
                detected: 3,
                critical: 1,
                warnings: 1,
                passed: 1,
                unknown: 2,
            }
        );
        assert_eq!(projection.counts.nav_badge_count(), Some(3));
    }

    #[test]
    fn summary_and_badge_handle_clear_unknown_and_singular_states() {
        assert_eq!(
            IssueCounts {
                passed: 28,
                ..IssueCounts::default()
            }
            .summary_text(),
            "All clear — no issues detected · 28 checks passed"
        );
        assert_eq!(
            IssueCounts {
                passed: 14,
                unknown: 11,
                ..IssueCounts::default()
            }
            .summary_text(),
            "No issues detected in completed checks · 14 checks passed · 11 couldn’t verify"
        );
        let one = IssueCounts {
            detected: 1,
            critical: 1,
            warnings: 1,
            ..IssueCounts::default()
        };
        assert_eq!(
            one.summary_text(),
            "1 issue needs attention · detected by the latest scan · 1 critical · 1 warning"
        );
        assert_eq!(one.nav_badge_count(), Some(1));
        assert_eq!(IssueCounts::default().nav_badge_count(), None);
    }

    #[test]
    fn canonical_maintenance_snapshot_preserves_order_and_every_field() {
        let snapshot = canonical_issue_metadata_snapshot();
        let canonical = remediation_summaries();
        let expected_maintenance = canonical
            .iter()
            .filter(|summary| summary.maintenance)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(snapshot.remediations, canonical);
        assert_eq!(snapshot.maintenance, expected_maintenance);
        assert_eq!(
            snapshot
                .maintenance
                .iter()
                .map(|summary| summary.id.as_str())
                .collect::<Vec<_>>(),
            [
                "flush_dns",
                "clear_icon_cache",
                "empty_recycle_bin",
                "clear_temp_files",
                "windows_update_reset",
                "dism_restorehealth",
                "sfc_scannow",
                "network_reset",
                "optimize_drives",
                "clear_windows_temp",
                "defender_quick_scan",
            ]
        );
        assert!(
            snapshot
                .maintenance
                .iter()
                .all(|summary| summary.maintenance)
        );
        assert!(
            snapshot
                .maintenance
                .iter()
                .any(|summary| summary.admin_required && summary.long_running)
        );
        assert!(
            snapshot
                .maintenance
                .iter()
                .any(|summary| summary.requires_restart && summary.cancellable)
        );
    }

    #[test]
    fn newly_critical_counts_escalations_and_ignores_undecided_baselines() {
        let issue = |id: &str, severity: IssueSeverity, status: IssueStatus| Issue {
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
        };
        let baseline = vec![
            issue("was_ok", IssueSeverity::Ok, IssueStatus::Ok),
            issue("was_warning", IssueSeverity::Warning, IssueStatus::Detected),
            issue("was_unknown", IssueSeverity::Info, IssueStatus::Unknown),
            issue(
                "was_critical",
                IssueSeverity::Critical,
                IssueStatus::Detected,
            ),
        ];
        let current = vec![
            issue("was_ok", IssueSeverity::Critical, IssueStatus::Detected),
            issue(
                "was_warning",
                IssueSeverity::Critical,
                IssueStatus::Detected,
            ),
            issue(
                "was_unknown",
                IssueSeverity::Critical,
                IssueStatus::Detected,
            ),
            issue(
                "was_critical",
                IssueSeverity::Critical,
                IssueStatus::Detected,
            ),
            issue("brand_new", IssueSeverity::Critical, IssueStatus::Detected),
            issue("warning_now", IssueSeverity::Warning, IssueStatus::Detected),
        ];
        assert_eq!(
            newly_critical(&baseline, &current)
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<Vec<_>>(),
            ["was_ok", "was_warning"]
        );
        assert!(newly_critical(&[], &current).is_empty());
    }

    #[test]
    fn unknown_check_hint_only_names_the_opt_in_probe() {
        let mut issue = Issue {
            id: "no_internet".to_string(),
            category: "Network".to_string(),
            severity: IssueSeverity::Info,
            status: IssueStatus::Unknown,
            title: "Internet Connection".to_string(),
            description: "Couldn't verify this check: diagnostic 'network_path' was not run"
                .to_string(),
            recommendation: String::new(),
            detected: false,
            source_tasks: Some(vec!["network_path".to_string()]),
            remediation: None,
        };
        assert!(unknown_check_hint(&issue, false).is_some());
        assert!(unknown_check_hint(&issue, true).is_none());
        issue.source_tasks = Some(vec!["services".to_string()]);
        assert!(unknown_check_hint(&issue, false).is_none());
        issue.source_tasks = Some(vec!["network_path".to_string()]);
        issue.status = IssueStatus::Ok;
        assert!(unknown_check_hint(&issue, false).is_none());
    }

    #[test]
    fn missing_check_tasks_are_the_unverified_rules_sources_minus_gated_probes() {
        let unknown = |id: &str, tasks: &[&str]| Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Unknown,
            title: id.to_string(),
            description: String::new(),
            recommendation: String::new(),
            detected: false,
            source_tasks: Some(tasks.iter().map(|task| (*task).to_string()).collect()),
            remediation: None,
        };
        let a = unknown("bsod_recent", &["minidump"]);
        let b = unknown("space_consumers", &["disk_usage", "logical_disk"]);
        let c = unknown("no_internet", &["network_path"]);
        assert_eq!(
            missing_check_tasks(&[&a, &b, &c], false),
            ["minidump", "disk_usage", "logical_disk"]
        );
        assert_eq!(missing_check_tasks(&[&c], true), ["network_path"]);
    }
}
