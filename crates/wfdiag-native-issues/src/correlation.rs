//! Root cause before symptom. A small, reviewed table says which detected
//! rule is usually a consequence of which other detected rule, so the list
//! leads with the thing to fix and the symptom says what it follows from.
//! Pure; every id is checked against the catalog by a test.

use crate::issue_catalog::Issue;

/// One "usually caused by" relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Correlation {
    pub cause: &'static str,
    pub symptom: &'static str,
    /// Why, in one clause a home user can follow.
    pub note: &'static str,
}

/// The reviewed table. Order matters only among several detected causes of
/// one symptom: the first listed wins.
pub const CORRELATIONS: &[Correlation] = &[
    Correlation {
        cause: "no_internet",
        symptom: "gateway_unreachable",
        note: "there is no working network connection to reach the router over",
    },
    Correlation {
        cause: "no_internet",
        symptom: "dns_resolution_failing",
        note: "names cannot resolve without a connection",
    },
    Correlation {
        cause: "gateway_unreachable",
        symptom: "dns_resolution_failing",
        note: "DNS servers sit beyond the router",
    },
    Correlation {
        cause: "no_internet",
        symptom: "windows_update_failing",
        note: "Windows Update cannot download without a connection",
    },
    Correlation {
        cause: "gateway_unreachable",
        symptom: "windows_update_failing",
        note: "Windows Update cannot reach Microsoft through an unreachable router",
    },
    Correlation {
        cause: "windows_update_service_disabled",
        symptom: "windows_update_failing",
        note: "updates cannot install while the service is disabled",
    },
    Correlation {
        cause: "windows_update_service_disabled",
        symptom: "pending_windows_updates",
        note: "updates cannot install while the service is disabled",
    },
    Correlation {
        cause: "pending_reboot",
        symptom: "pending_windows_updates",
        note: "the installed updates finish at the restart",
    },
    Correlation {
        cause: "pending_reboot",
        symptom: "windows_update_failing",
        note: "an update waiting for a restart blocks the next one",
    },
    Correlation {
        cause: "low_disk_space",
        symptom: "space_consumers",
        note: "the breakdown shows what is using the space",
    },
    Correlation {
        cause: "low_disk_space",
        symptom: "page_file_pressure",
        note: "the page file cannot grow on a full disk",
    },
    Correlation {
        cause: "low_disk_space",
        symptom: "temp_files",
        note: "temporary files are part of what fills the disk",
    },
    Correlation {
        cause: "high_memory_usage",
        symptom: "page_file_pressure",
        note: "memory pressure spills into the page file",
    },
    Correlation {
        cause: "defender_disabled",
        symptom: "realtime_protection_off",
        note: "Defender is not running at all",
    },
    Correlation {
        cause: "defender_disabled",
        symptom: "defender_definitions_stale",
        note: "a stopped Defender does not update its definitions",
    },
    Correlation {
        cause: "defender_disabled",
        symptom: "defender_quick_scan_overdue",
        note: "a stopped Defender does not scan",
    },
    Correlation {
        cause: "smart_failure_predicted",
        symptom: "disk_unhealthy",
        note: "the drive itself reports it is failing",
    },
    Correlation {
        cause: "smart_failure_predicted",
        symptom: "disk_io_errors",
        note: "a failing drive produces read and write errors",
    },
    Correlation {
        cause: "disk_unhealthy",
        symptom: "disk_io_errors",
        note: "an unhealthy drive produces read and write errors",
    },
    Correlation {
        cause: "kernel_power_crashes",
        symptom: "unexpected_shutdowns",
        note: "each kernel-power event is an unexpected shutdown",
    },
    Correlation {
        cause: "bsod_recent",
        symptom: "unexpected_shutdowns",
        note: "a blue screen ends in an unexpected shutdown",
    },
    Correlation {
        cause: "whea_errors",
        symptom: "bsod_recent",
        note: "hardware errors are a common blue-screen cause",
    },
];

/// The detected issue `issue_id` is most likely a consequence of, with the
/// reason. `detected` must hold detected issues only.
#[must_use]
pub fn likely_cause<'a>(
    issue_id: &str,
    detected: &[&'a Issue],
) -> Option<(&'a Issue, &'static str)> {
    CORRELATIONS
        .iter()
        .filter(|relation| relation.symptom == issue_id)
        .find_map(|relation| {
            detected
                .iter()
                .copied()
                .find(|issue| issue.id == relation.cause && issue.id != issue_id)
                .map(|cause| (cause, relation.note))
        })
}

/// One sentence for the symptom's card: what it follows from and what to do.
#[must_use]
pub fn cause_hint(cause: &Issue, note: &str) -> String {
    format!(
        "Likely a consequence of \"{}\" ({note}); fix that first.",
        cause.title
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_catalog::{IssueSeverity, IssueStatus, catalog};

    #[test]
    fn every_relation_names_two_distinct_catalog_rules_and_has_no_cycle() {
        let ids: Vec<&str> = catalog().iter().map(|spec| spec.id).collect();
        for relation in CORRELATIONS {
            assert!(ids.contains(&relation.cause), "{}", relation.cause);
            assert!(ids.contains(&relation.symptom), "{}", relation.symptom);
            assert_ne!(relation.cause, relation.symptom);
            assert!(!relation.note.is_empty());
        }
        // No cycle: following cause links from any symptom terminates.
        for start in CORRELATIONS.iter().map(|relation| relation.symptom) {
            let mut current = start;
            let mut steps = 0;
            while let Some(relation) = CORRELATIONS
                .iter()
                .find(|relation| relation.symptom == current)
            {
                current = relation.cause;
                steps += 1;
                assert!(steps <= CORRELATIONS.len(), "cycle through {start}");
            }
        }
    }

    #[test]
    fn a_symptom_is_attributed_only_to_a_detected_cause() {
        let issue = |id: &str| Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: format!("Title {id}"),
            description: String::new(),
            recommendation: String::new(),
            detected: true,
            source_tasks: None,
            remediation: None,
        };
        let dns = issue("dns_resolution_failing");
        let gateway = issue("gateway_unreachable");
        let no_internet = issue("no_internet");
        assert!(likely_cause("dns_resolution_failing", &[&dns]).is_none());
        let (cause, note) = likely_cause("dns_resolution_failing", &[&dns, &gateway]).unwrap();
        assert_eq!(cause.id, "gateway_unreachable");
        assert!(note.contains("router"));
        // The first listed detected cause wins.
        let (cause, _) =
            likely_cause("dns_resolution_failing", &[&dns, &gateway, &no_internet]).unwrap();
        assert_eq!(cause.id, "no_internet");
        assert!(
            cause_hint(cause, "x").starts_with("Likely a consequence of \"Title no_internet\" (x)")
        );
    }
}
