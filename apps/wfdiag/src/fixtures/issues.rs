//! The deterministic Store 2.5.8 issue fixture.
//!
//! Ids, titles, recommendations, source-task links, action metadata, ordering,
//! and the item total come from the shipping catalogs; only the
//! screenshot-specific outcomes are pinned here.

use wfdiag_native_issues::projection::canonical_issue_metadata_snapshot;
use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, catalog};

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
    use super::{fixture_258_issues, source_tasks};
    use std::collections::HashSet;
    use wfdiag_native_issues::projection::{IssueCounts, project_issues};
    use wfdiag_native_issues::{IssueSeverity, IssueStatus, catalog, remediation_summaries};

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
