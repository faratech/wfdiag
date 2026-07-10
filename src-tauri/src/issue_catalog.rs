//! Issue catalog: the single registry of every known issue wfdiag can detect.
//!
//! Architecture: metadata (ids, categories, titles, recommendations, the
//! remediation mapping) lives HERE in a static spec table; detection logic
//! stays as small pure functions in [`crate::issue_detector`] (bitfields and
//! date math don't belong in a data DSL). `detect_all` iterates the catalog,
//! so detectors can never drift from their metadata, and catalog invariants
//! are enforced by tests instead of hand-pinned id lists.

use crate::diagnostics::TaskResult;
use crate::issue_detector as det;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub category: String,
    pub severity: IssueSeverity,
    pub status: IssueStatus,
    pub title: String,
    pub description: String,
    pub recommendation: String,
    pub detected: bool,
    /// Diagnostic tasks this issue was derived from (frontend "Ask AI" uses
    /// these to attach the relevant raw data to the chat prompt)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tasks: Option<Vec<String>>,
    /// The vetted remediation for this issue, when one applies
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<crate::remediation::RemediationSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,
    Warning,
    Info,
    Ok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Detected,
    Ok,
    Skipped,
}

/// Everything a detector may consume. The clock and the temp-file count are
/// injected so every detector is deterministic in tests (no env probing, no
/// wall-clock reads inside detection logic).
pub struct DetectCtx<'a> {
    pub results: &'a HashMap<String, TaskResult>,
    pub now: Timestamp,
    /// Entry count of the user's temp directory; None = unknown (not detected)
    pub temp_file_count: Option<usize>,
}

/// A positive detection: dynamic description plus an optional severity
/// override (e.g. a count crossing a higher threshold upgrades to Critical).
pub struct Detection {
    pub severity: Option<IssueSeverity>,
    pub description: String,
}

impl Detection {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            severity: None,
            description: description.into(),
        }
    }

    pub fn with_severity(description: impl Into<String>, severity: IssueSeverity) -> Self {
        Self {
            severity: Some(severity),
            description: description.into(),
        }
    }
}

pub type DetectFn = fn(&DetectCtx) -> Option<Detection>;

pub struct IssueSpec {
    pub id: &'static str,
    pub category: &'static str,
    pub default_severity: IssueSeverity,
    /// Title when detected
    pub title: &'static str,
    /// Title/description when NOT detected
    pub ok_title: &'static str,
    pub ok_description: &'static str,
    pub recommendation: &'static str,
    /// Diagnostic task ids this detector reads (must exist in get_all_tasks)
    pub source_tasks: &'static [&'static str],
    /// Remediation from the remediation catalog, when one applies
    pub remediation_id: Option<&'static str>,
    pub detect: DetectFn,
}

/// The full issue catalog. Order is display order (roughly: severity-prone
/// hardware/security first, hygiene last).
pub fn catalog() -> &'static [IssueSpec] {
    &[
        IssueSpec {
            id: "low_disk_space",
            category: "Storage",
            default_severity: IssueSeverity::Critical,
            title: "Low Disk Space",
            ok_title: "Disk Space",
            ok_description: "All disks have adequate free space (>10%).",
            recommendation: "Free up disk space by deleting unnecessary files.",
            source_tasks: &["logical_disk"],
            remediation_id: Some("open_disk_cleanup"),
            detect: det::detect_low_disk_space,
        },
        IssueSpec {
            id: "disk_fragmentation",
            category: "Storage",
            default_severity: IssueSeverity::Warning,
            title: "High Disk Fragmentation",
            ok_title: "Disk Fragmentation",
            ok_description: "Disk fragmentation is within normal levels (<20%).",
            recommendation: "Defragment your disk to improve performance.",
            source_tasks: &["disk_fragmentation"],
            remediation_id: Some("open_defrag"),
            detect: det::detect_disk_fragmentation,
        },
        IssueSpec {
            id: "unsigned_drivers",
            category: "Drivers",
            default_severity: IssueSeverity::Warning,
            title: "Unsigned Drivers Detected",
            ok_title: "Driver Signatures",
            ok_description: "All drivers are properly signed.",
            recommendation: "Update drivers from manufacturer websites.",
            source_tasks: &["drivers_list"],
            remediation_id: None,
            detect: det::detect_unsigned_drivers,
        },
        IssueSpec {
            id: "event_log_errors",
            category: "Logs",
            default_severity: IssueSeverity::Warning,
            title: "Event Log Errors",
            ok_title: "Event Logs",
            ok_description: "No critical errors found in event logs.",
            recommendation: "Review event logs for details.",
            source_tasks: &["event_logs"],
            remediation_id: None,
            detect: det::detect_event_log_errors,
        },
        IssueSpec {
            id: "stopped_services",
            category: "Services",
            default_severity: IssueSeverity::Warning,
            title: "Stopped Services",
            ok_title: "Core Windows Services",
            ok_description: "Supported core automatic services are running normally.",
            recommendation: "Start stopped core services or investigate why they failed.",
            source_tasks: &["services"],
            remediation_id: Some("start_critical_services"),
            detect: det::detect_stopped_services,
        },
        IssueSpec {
            id: "high_cpu_usage",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "High CPU Usage",
            ok_title: "CPU Usage",
            ok_description: "CPU usage is within normal range (<90%).",
            recommendation: "Check Task Manager for resource-intensive processes.",
            source_tasks: &["performance"],
            remediation_id: Some("open_task_manager"),
            detect: det::detect_high_cpu_usage,
        },
        IssueSpec {
            id: "high_memory_usage",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "High Memory Usage",
            ok_title: "Memory Usage",
            ok_description: "Memory usage is within normal range (<90%).",
            recommendation: "Close unnecessary programs to free memory.",
            source_tasks: &["performance"],
            remediation_id: Some("open_task_manager"),
            detect: det::detect_high_memory_usage,
        },
        IssueSpec {
            id: "pending_windows_updates",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Outdated Windows Updates",
            ok_title: "Windows Updates",
            ok_description: "System is up to date.",
            recommendation: "Check for and install pending updates.",
            source_tasks: &["windows_update"],
            remediation_id: Some("open_windows_update"),
            detect: det::detect_pending_windows_updates,
        },
        IssueSpec {
            id: "firewall_disabled",
            category: "Security",
            default_severity: IssueSeverity::Critical,
            title: "Firewall Disabled",
            ok_title: "Firewall Status",
            ok_description: "Firewall is enabled and protecting your system.",
            recommendation: "Enable firewall for network protection.",
            source_tasks: &["firewall_status"],
            remediation_id: Some("open_security_center"),
            detect: det::detect_firewall_disabled,
        },
        IssueSpec {
            id: "temp_files",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "Excessive Temporary Files",
            ok_title: "Temporary Files",
            ok_description: "Temporary files are within normal limits.",
            recommendation: "Clean temporary files to free disk space.",
            source_tasks: &[],
            remediation_id: Some("clear_temp_files"),
            detect: det::detect_temp_files,
        },
        IssueSpec {
            id: "dns_misconfigured",
            category: "Network",
            default_severity: IssueSeverity::Warning,
            title: "DNS Not Configured",
            ok_title: "DNS Configuration",
            ok_description: "Network adapters have DNS servers configured.",
            recommendation: "Set DNS servers on the adapter (or re-enable DHCP) — without \
                             them name resolution fails even though the network is up.",
            source_tasks: &["network_adapter"],
            remediation_id: None,
            detect: det::detect_dns_misconfigured,
        },
        IssueSpec {
            id: "smart_failure_predicted",
            category: "Storage",
            default_severity: IssueSeverity::Critical,
            title: "Disk Failure Predicted (SMART)",
            ok_title: "Disk SMART Status",
            ok_description: "No disk is predicting failure.",
            recommendation: "Back up your data immediately and replace the disk.",
            source_tasks: &["chkdsk"],
            remediation_id: None,
            detect: det::detect_smart_failure_predicted,
        },
        IssueSpec {
            id: "disk_unhealthy",
            category: "Storage",
            default_severity: IssueSeverity::Warning,
            title: "Disk Health Degraded",
            ok_title: "Disk Health",
            ok_description: "All physical disks report healthy status.",
            recommendation: "Back up important data and monitor the disk closely.",
            source_tasks: &["chkdsk"],
            remediation_id: None,
            detect: det::detect_disk_unhealthy,
        },
        IssueSpec {
            id: "dism_corruption",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Windows Component Store Corruption",
            ok_title: "Windows Image Health",
            ok_description: "The Windows component store is healthy.",
            recommendation: "Run the DISM repair to restore the component store.",
            source_tasks: &["dism_health"],
            remediation_id: Some("dism_restorehealth"),
            detect: det::detect_dism_corruption,
        },
        IssueSpec {
            id: "bsod_recent",
            category: "Debug",
            default_severity: IssueSeverity::Critical,
            title: "Recent Blue Screen Crashes",
            ok_title: "Blue Screen Crashes",
            ok_description: "No crash dumps from the last 30 days.",
            recommendation: "Copy the minidumps (Diagnostics > Debug) and analyze them, or share them on WindowsForum for help.",
            source_tasks: &["minidump"],
            remediation_id: None,
            detect: det::detect_bsod_recent,
        },
        IssueSpec {
            id: "kernel_power_crashes",
            category: "System",
            default_severity: IssueSeverity::Critical,
            title: "Power-Loss Crashes (Kernel-Power 41)",
            ok_title: "Kernel-Power Events",
            ok_description: "No power-loss crash events in the last 7 days.",
            recommendation: "Check PSU/power cabling, overheating and driver crashes; review what happened right before each event.",
            source_tasks: &["event_codes_critical"],
            remediation_id: None,
            detect: det::detect_kernel_power_crashes,
        },
        IssueSpec {
            id: "unexpected_shutdowns",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Unexpected Shutdowns",
            ok_title: "Shutdown Events",
            ok_description: "No unexpected shutdowns in the last 7 days.",
            recommendation: "Correlate the times with power events or crashes; check Reliability Monitor.",
            source_tasks: &["event_codes_critical"],
            remediation_id: None,
            detect: det::detect_unexpected_shutdowns,
        },
        IssueSpec {
            id: "disk_io_errors",
            category: "Storage",
            default_severity: IssueSeverity::Warning,
            title: "Disk I/O Errors Logged",
            ok_title: "Disk I/O Events",
            ok_description: "No disk I/O error events in the last 7 days.",
            recommendation: "Back up the affected disk, check SATA/NVMe cabling and run the disk health check.",
            source_tasks: &["event_codes_critical"],
            remediation_id: None,
            detect: det::detect_disk_io_errors,
        },
        IssueSpec {
            id: "whea_errors",
            category: "Hardware",
            default_severity: IssueSeverity::Warning,
            title: "Hardware Errors (WHEA)",
            ok_title: "Hardware Error Events",
            ok_description: "No WHEA hardware error events in the last 7 days.",
            recommendation: "Check CPU/RAM stability (disable overclocks, run memtest) and update firmware.",
            source_tasks: &["event_codes_critical"],
            remediation_id: None,
            detect: det::detect_whea_errors,
        },
        IssueSpec {
            id: "service_crash_loops",
            category: "Services",
            default_severity: IssueSeverity::Warning,
            title: "Services Crashing Repeatedly",
            ok_title: "Service Stability",
            ok_description: "No repeated service crashes in the last 7 days.",
            recommendation: "Identify the crashing service in the event log and reinstall or update its application.",
            source_tasks: &["event_codes_critical"],
            remediation_id: None,
            detect: det::detect_service_crash_loops,
        },
        IssueSpec {
            id: "device_manager_errors",
            category: "Hardware",
            default_severity: IssueSeverity::Warning,
            title: "Devices With Problems",
            ok_title: "Device Status",
            ok_description: "No devices report Device Manager problem codes.",
            recommendation: "Open Device Manager and update or reinstall the flagged drivers.",
            source_tasks: &["device_errors"],
            remediation_id: Some("open_device_manager"),
            detect: det::detect_device_manager_errors,
        },
        IssueSpec {
            id: "defender_disabled",
            category: "Security",
            default_severity: IssueSeverity::Critical,
            title: "Antivirus Disabled",
            ok_title: "Antivirus Status",
            ok_description: "An antivirus product is active.",
            recommendation: "Enable Windows Security (or your AV product) immediately.",
            source_tasks: &["defender_status"],
            remediation_id: Some("open_security_center"),
            detect: det::detect_defender_disabled,
        },
        IssueSpec {
            id: "pending_reboot",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Restart Pending",
            ok_title: "Restart State",
            ok_description: "No restart is pending.",
            recommendation: "Restart Windows to finish applying updates or driver changes.",
            source_tasks: &["pending_reboot"],
            remediation_id: Some("restart_system"),
            detect: det::detect_pending_reboot,
        },
        IssueSpec {
            id: "page_file_pressure",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "Page File Nearly Full",
            ok_title: "Page File",
            ok_description: "Page file usage is within normal limits.",
            recommendation: "Close memory-heavy programs or increase the page file size.",
            source_tasks: &["performance"],
            remediation_id: Some("open_task_manager"),
            detect: det::detect_page_file_pressure,
        },
        IssueSpec {
            id: "battery_degraded",
            category: "Hardware",
            default_severity: IssueSeverity::Warning,
            title: "Battery Significantly Degraded",
            ok_title: "Battery Health",
            ok_description: "Battery health is acceptable (or no battery present).",
            recommendation: "Consider replacing the battery; reduce charge cycles by avoiding deep discharges.",
            source_tasks: &["battery_report"],
            remediation_id: None,
            detect: det::detect_battery_degraded,
        },
        IssueSpec {
            id: "startup_bloat",
            category: "Performance",
            default_severity: IssueSeverity::Info,
            title: "Many Startup Programs",
            ok_title: "Startup Programs",
            ok_description: "Startup program count is reasonable.",
            recommendation: "Disable unneeded startup entries in Task Manager's Startup tab.",
            source_tasks: &["startup_command"],
            remediation_id: Some("open_task_manager"),
            detect: det::detect_startup_bloat,
        },
        IssueSpec {
            id: "outdated_drivers",
            category: "Drivers",
            default_severity: IssueSeverity::Warning,
            title: "Outdated Display/Network Drivers",
            ok_title: "Driver Age",
            ok_description: "Display and network drivers are reasonably current.",
            recommendation: "Download current drivers from the GPU/NIC manufacturer.",
            source_tasks: &["drivers_list"],
            remediation_id: None,
            detect: det::detect_outdated_drivers,
        },
        IssueSpec {
            id: "hosts_file_hijack",
            category: "Security",
            default_severity: IssueSeverity::Critical,
            title: "Hosts File Hijack Suspected",
            ok_title: "Hosts File",
            ok_description: "No suspicious hosts-file redirections.",
            recommendation: "Review the hosts file and run a full antivirus scan; do not log into redirected sites.",
            source_tasks: &["hosts_file"],
            remediation_id: None,
            detect: det::detect_hosts_file_hijack,
        },
    ]
}

/// Run every catalog detector against the context. Always returns one Issue
/// per spec (detected or ok) — the stat strip and AI tools rely on the full
/// sweep.
pub fn detect_all(ctx: &DetectCtx) -> Vec<Issue> {
    catalog()
        .iter()
        .map(|spec| match (spec.detect)(ctx) {
            Some(detection) => Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: detection.severity.unwrap_or(spec.default_severity),
                status: IssueStatus::Detected,
                title: spec.title.to_string(),
                description: detection.description,
                recommendation: spec.recommendation.to_string(),
                detected: true,
                source_tasks: source_tasks_for_issue(spec),
                remediation: spec.remediation_id.and_then(crate::remediation::summary),
            },
            None => Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: if source_tasks_available(ctx, spec) {
                    IssueSeverity::Ok
                } else {
                    IssueSeverity::Info
                },
                status: if source_tasks_available(ctx, spec) {
                    IssueStatus::Ok
                } else {
                    IssueStatus::Skipped
                },
                title: spec.ok_title.to_string(),
                description: if source_tasks_available(ctx, spec) {
                    spec.ok_description.to_string()
                } else {
                    "Required diagnostic data was not available for this check.".to_string()
                },
                recommendation: if source_tasks_available(ctx, spec) {
                    "No action needed.".to_string()
                } else {
                    "Run the required diagnostic tasks, or restart as administrator for admin-only checks.".to_string()
                },
                detected: false,
                source_tasks: source_tasks_for_issue(spec),
                remediation: None,
            },
        })
        .collect()
}

fn source_tasks_for_issue(spec: &IssueSpec) -> Option<Vec<String>> {
    if spec.source_tasks.is_empty() {
        None
    } else {
        Some(spec.source_tasks.iter().map(|s| s.to_string()).collect())
    }
}

fn source_tasks_available(ctx: &DetectCtx, spec: &IssueSpec) -> bool {
    if spec.id == "temp_files" {
        return ctx.temp_file_count.is_some();
    }
    spec.source_tasks.iter().all(|task_id| {
        ctx.results
            .get(*task_id)
            .is_some_and(|result| result.success)
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Fixed clock for deterministic detector tests.
    pub fn fixed_now() -> Timestamp {
        Timestamp::from_iso_string("2026-06-12T12:00:00Z").unwrap()
    }

    pub fn ctx<'a>(results: &'a HashMap<String, TaskResult>) -> DetectCtx<'a> {
        DetectCtx {
            results,
            now: fixed_now(),
            temp_file_count: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<&str> = catalog().iter().map(|s| s.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate issue ids in catalog");
    }

    #[test]
    fn source_tasks_reference_real_diagnostic_tasks() {
        let task_ids: Vec<String> = crate::diagnostics::get_all_tasks()
            .into_iter()
            .map(|t| t.id)
            .collect();
        for spec in catalog() {
            for task in spec.source_tasks {
                assert!(
                    task_ids.iter().any(|id| id == task),
                    "issue '{}' references unknown task '{}'",
                    spec.id,
                    task
                );
            }
        }
    }

    #[test]
    fn empty_context_yields_full_sweep_with_no_detections() {
        let results = HashMap::new();
        let ctx = DetectCtx {
            results: &results,
            now: test_support::fixed_now(),
            temp_file_count: None, // injected: no env probing in detectors
        };
        let issues = detect_all(&ctx);
        assert_eq!(issues.len(), catalog().len());
        for issue in &issues {
            assert!(
                !issue.detected,
                "issue '{}' detected on an empty result set",
                issue.id
            );
            assert_eq!(issue.status, IssueStatus::Skipped);
            assert_eq!(issue.severity, IssueSeverity::Info);
        }
    }

    #[test]
    fn successful_source_task_without_detection_is_ok() {
        let mut results = HashMap::new();
        results.insert(
            "logical_disk".to_string(),
            TaskResult {
                success: true,
                output: r#"[{"Name":"C:","FreeSpace":90000000000,"Size":100000000000}]"#.into(),
                error: None,
                duration_ms: 1,
            },
        );
        let ctx = test_support::ctx(&results);
        let issue = detect_all(&ctx)
            .into_iter()
            .find(|issue| issue.id == "low_disk_space")
            .expect("issue exists");

        assert_eq!(issue.status, IssueStatus::Ok);
        assert_eq!(issue.severity, IssueSeverity::Ok);
    }
}
