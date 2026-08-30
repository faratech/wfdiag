//! Pure issue-state support for the Reactor shell.
//!
//! Worker ownership and UI scheduling stay in `main.rs`. This module owns the
//! framework-neutral parts of that boundary: immutable request preparation,
//! stale-completion guards, Store-compatible issue projections, canonical
//! remediation snapshots, and the deterministic 2.5.8 visual fixture.

use std::collections::HashMap;

use wfdiag_native_issues::{
    Issue, IssueDetectionCompleted, IssueDetectionRequest, IssueSeverity, IssueStatus,
    RemediationSummary, TaskResult, Timestamp, catalog, remediation_summaries,
};

/// Identity captured when an issue-detection request is enqueued.
///
/// A request id alone is insufficient: it can wrap, and a late response for
/// restored scan evidence must not replace issues derived from a newer
/// committed scan. `committed_epoch` and `session_id` identify that evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingIssueDetection {
    pub request_id: u64,
    pub committed_epoch: u64,
    pub session_id: String,
}

/// A worker request paired with the guard that must be retained by the shell.
#[derive(Clone, Debug)]
pub(crate) struct PreparedIssueDetection {
    pub pending: PendingIssueDetection,
    pub request: IssueDetectionRequest,
}

/// Build a worker request directly from the authoritative diagnostic result
/// map. `DiagnosticOutput` is a re-export of `TaskResult`, so callers should
/// move the map here without converting it through UI rows or JSON.
#[must_use]
pub(crate) fn prepare_issue_detection(
    request_id: u64,
    committed_epoch: u64,
    session_id: String,
    results: HashMap<String, TaskResult>,
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
pub(crate) fn advance_nonzero_generation(sequence: &mut u64) -> Option<u64> {
    let next = sequence.checked_add(1)?;
    *sequence = next;
    Some(next)
}

/// Consume `pending` only when the completion belongs to the current
/// committed evidence. A stale response deliberately leaves a newer pending
/// request intact.
pub(crate) fn take_current_issue_completion(
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
pub(crate) fn pending_issue_preparation_is_current(
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
pub(crate) fn issue_projection_matches_evidence(
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
pub(crate) struct IssueCounts {
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
        let mut summary = format!(
            "{} {issue_noun} {needs} attention · detected by the latest scan",
            self.detected
        );
        if self.critical > 0 {
            summary.push_str(&format!(" · {} critical", self.critical));
        }
        if self.warnings > 0 {
            let warning_noun = if self.warnings == 1 {
                "warning"
            } else {
                "warnings"
            };
            summary.push_str(&format!(" · {} {warning_noun}", self.warnings));
        }
        summary
    }
}

/// Stable partitions over one immutable issue result set.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct IssueProjection<'a> {
    pub detected: Vec<&'a Issue>,
    pub passed: Vec<&'a Issue>,
    pub unknown: Vec<&'a Issue>,
    pub counts: IssueCounts,
}

/// Match the Store's compatibility projection. `detected` and the explicit
/// status are both honored; legacy `Skipped` is grouped with `Unknown` and is
/// never presented as a passed check.
#[must_use]
pub(crate) fn project_issues(issues: &[Issue]) -> IssueProjection<'_> {
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
pub(crate) struct IssueMetadataSnapshot {
    pub remediations: Vec<RemediationSummary>,
    pub maintenance: Vec<RemediationSummary>,
}

#[must_use]
pub(crate) fn canonical_issue_metadata_snapshot() -> IssueMetadataSnapshot {
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

const FIXTURE_258_UNKNOWN_IDS: [&str; 11] = [
    "disk_fragmentation",
    "unsigned_drivers",
    "event_log_errors",
    "pending_windows_updates",
    "smart_failure_predicted",
    "disk_unhealthy",
    "dism_corruption",
    "bsod_recent",
    "defender_disabled",
    "battery_degraded",
    "outdated_drivers",
];

fn fixture_detected_details(issue_id: &str) -> Option<(&'static str, IssueSeverity)> {
    match issue_id {
        "temp_files" => Some((
            "Found 1069 files in temp directory.",
            IssueSeverity::Warning,
        )),
        "device_manager_errors" => Some((
            "1 device(s) report a Device Manager problem code: HID Custom Sensor (all are user-disabled devices).",
            IssueSeverity::Info,
        )),
        "page_file_pressure" => Some((
            "Page file nearly exhausted: 3.1% of 92835 MB available.",
            IssueSeverity::Warning,
        )),
        _ => None,
    }
}

fn source_tasks(source_tasks: &[&str]) -> Option<Vec<String>> {
    (!source_tasks.is_empty()).then(|| {
        source_tasks
            .iter()
            .map(|task_id| (*task_id).to_string())
            .collect()
    })
}

/// Derive the populated Store 2.5.8 visual state from the shipping issue and
/// remediation catalogs. This keeps ids, titles, recommendations, source-task
/// links, action metadata, ordering, and the 28-item total synchronized with
/// production while pinning only the screenshot-specific outcomes.
#[must_use]
pub(crate) fn fixture_258_issues() -> Vec<Issue> {
    let metadata = canonical_issue_metadata_snapshot();

    catalog()
        .iter()
        .map(|spec| {
            let common_source_tasks = source_tasks(spec.source_tasks);
            if let Some((description, severity)) = fixture_detected_details(spec.id) {
                return Issue {
                    id: spec.id.to_string(),
                    category: spec.category.to_string(),
                    severity,
                    status: IssueStatus::Detected,
                    title: spec.title.to_string(),
                    description: description.to_string(),
                    recommendation: spec.recommendation.to_string(),
                    detected: true,
                    source_tasks: common_source_tasks,
                    remediation: spec.remediation_id.and_then(|remediation_id| {
                        metadata
                            .remediations
                            .iter()
                            .find(|summary| summary.id == remediation_id)
                            .cloned()
                    }),
                };
            }

            if FIXTURE_258_UNKNOWN_IDS.contains(&spec.id) {
                let reason = spec.source_tasks.first().map_or_else(
                    || "the required diagnostic data was unavailable".to_string(),
                    |task_id| format!("diagnostic '{task_id}' was not run"),
                );
                return Issue {
                    id: spec.id.to_string(),
                    category: spec.category.to_string(),
                    severity: IssueSeverity::Info,
                    status: IssueStatus::Unknown,
                    title: spec.ok_title.to_string(),
                    description: format!("Couldn't verify this check: {reason}"),
                    recommendation: "Retry the required diagnostic tasks, or restart as administrator for access-restricted checks.".to_string(),
                    detected: false,
                    source_tasks: common_source_tasks,
                    remediation: None,
                };
            }

            Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: IssueSeverity::Ok,
                status: IssueStatus::Ok,
                title: spec.ok_title.to_string(),
                description: spec.ok_description.to_string(),
                recommendation: "No action needed.".to_string(),
                detected: false,
                source_tasks: common_source_tasks,
                remediation: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use wfdiag_native_diagnostics::DiagnosticOutput;

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
        let result = DiagnosticOutput {
            success: true,
            output: "{\"ok\":true}".to_string(),
            error: None,
            duration_ms: 17,
        };
        let results: HashMap<String, DiagnosticOutput> =
            HashMap::from([("logical_disk".to_string(), result.clone())]);
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
        assert_eq!(prepared.request.results.len(), 1);
        assert_eq!(prepared.request.now.timestamp(), 1_788_076_800);
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
    fn visual_fixture_is_catalog_complete_ordered_and_deterministic() {
        let first = fixture_258_issues();
        let second = fixture_258_issues();
        assert_eq!(first, second);
        assert_eq!(first.len(), catalog().len());
        assert_eq!(
            first
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<Vec<_>>(),
            catalog().iter().map(|spec| spec.id).collect::<Vec<_>>()
        );
        assert_eq!(
            first
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            catalog().len()
        );
    }

    #[test]
    fn visual_fixture_retains_store_counts_text_and_detected_metadata() {
        let issues = fixture_258_issues();
        let projection = project_issues(&issues);
        assert_eq!(
            projection.counts,
            IssueCounts {
                total: 28,
                detected: 3,
                critical: 0,
                warnings: 2,
                passed: 14,
                unknown: 11,
            }
        );
        assert_eq!(projection.counts.nav_badge_count(), Some(3));
        assert_eq!(
            projection.counts.summary_text(),
            "3 issues need attention · detected by the latest scan · 2 warnings"
        );
        assert_eq!(
            projection
                .detected
                .iter()
                .map(|issue| issue.id.as_str())
                .collect::<Vec<_>>(),
            ["temp_files", "device_manager_errors", "page_file_pressure"]
        );

        let canonical = remediation_summaries();
        for issue in &projection.detected {
            let remediation_id = catalog()
                .iter()
                .find(|spec| spec.id == issue.id)
                .and_then(|spec| spec.remediation_id)
                .unwrap();
            assert_eq!(
                issue.remediation.as_ref(),
                canonical
                    .iter()
                    .find(|summary| summary.id == remediation_id)
            );
        }
        assert_eq!(
            projection.detected[1].severity,
            IssueSeverity::Info,
            "the user-disabled fixture device remains informational"
        );
    }

    #[test]
    fn visual_fixture_passed_and_unknown_rows_use_production_projection_metadata() {
        let issues = fixture_258_issues();
        for issue in &issues {
            let spec = catalog().iter().find(|spec| spec.id == issue.id).unwrap();
            let expected_sources = source_tasks(spec.source_tasks);
            assert_eq!(issue.category, spec.category);
            assert_eq!(issue.source_tasks, expected_sources);

            if issue.status == IssueStatus::Ok {
                assert_eq!(issue.title, spec.ok_title);
                assert_eq!(issue.description, spec.ok_description);
                assert_eq!(issue.recommendation, "No action needed.");
                assert!(issue.remediation.is_none());
            } else if issue.status == IssueStatus::Unknown {
                assert_eq!(issue.title, spec.ok_title);
                assert!(
                    issue
                        .description
                        .starts_with("Couldn't verify this check: ")
                );
                assert_eq!(issue.severity, IssueSeverity::Info);
                assert!(issue.remediation.is_none());
            }
        }
    }
}
