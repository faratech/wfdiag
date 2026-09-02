//! Issue catalog: the single registry of every known issue wfdiag can detect.
//!
//! Architecture: metadata (ids, categories, titles, recommendations, the
//! remediation mapping) lives HERE in a static spec table; detection logic
//! stays as small pure functions in [`crate::issue_detector`] (bitfields and
//! date math don't belong in a data DSL). `detect_all` iterates the catalog,
//! so detectors can never drift from their metadata, and catalog invariants
//! are enforced by tests instead of hand-pinned id lists.

#[cfg(test)]
use crate::diagnostics::TaskResult;
use crate::diagnostics::TaskResultLookup;
use crate::issue_detector as det;
use crate::remediation::RemediationSummary;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub remediation: Option<RemediationSummary>,
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
    /// The diagnostic did not provide enough trustworthy evidence to decide.
    Unknown,
    /// Legacy wire value retained for old stored scans. New detections use
    /// `Unknown` instead.
    Skipped,
}

/// Everything a detector may consume. The clock and the temp-file count are
/// injected so every detector is deterministic in tests (no env probing, no
/// wall-clock reads inside detection logic).
pub struct DetectCtx<'a> {
    pub results: &'a dyn TaskResultLookup,
    pub now: Timestamp,
    /// Entry count of the user's temp directory; None = unknown (not detected)
    pub temp_file_count: Option<usize>,
}

/// A positive detection: dynamic description plus an optional severity
/// override (e.g. a count crossing a higher threshold upgrades to Critical)
/// and an optional evidence-dependent remediation (e.g. the fix for the
/// largest reclaimable disk consumer).
pub struct Detection {
    pub severity: Option<IssueSeverity>,
    pub description: String,
    /// A remediation chosen from the evidence. It is honoured only when the
    /// spec lists it (its default or one of `alternate_remediations`);
    /// anything else silently falls back to the spec's default, so a detector
    /// can never widen the closed remediation set.
    pub remediation_id: Option<&'static str>,
}

/// The result of evaluating one catalog rule. A detector may only report
/// `Clear` after its source task passed schema/evidence validation; missing,
/// malformed, partial, or access-denied evidence is always `Unknown`.
pub enum DetectionOutcome {
    Detected(Detection),
    Clear,
    Unknown(String),
}

impl Detection {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            severity: None,
            description: description.into(),
            remediation_id: None,
        }
    }

    pub fn with_severity(description: impl Into<String>, severity: IssueSeverity) -> Self {
        Self {
            severity: Some(severity),
            description: description.into(),
            remediation_id: None,
        }
    }

    /// Point the issue at an evidence-dependent remediation.
    #[must_use]
    pub fn with_remediation(mut self, remediation_id: &'static str) -> Self {
        self.remediation_id = Some(remediation_id);
        self
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
    /// Further catalog remediations a detector may select from the evidence
    /// (see [`Detection::with_remediation`]). Closed and catalog-checked.
    pub alternate_remediations: &'static [&'static str],
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
            recommendation: "Free up space: clear temporary files and empty the Recycle Bin first, then remove large downloads you no longer need.",
            source_tasks: &["logical_disk"],
            remediation_id: Some("clear_temp_files"),
            alternate_remediations: &[
                "empty_recycle_bin",
                "windows_update_reset",
                "clear_windows_temp",
                "open_downloads_folder",
                "open_storage_settings",
                "open_disk_cleanup",
            ],
            detect: det::detect_low_disk_space,
        },
        IssueSpec {
            id: "space_consumers",
            category: "Storage",
            default_severity: IssueSeverity::Info,
            title: "Space You Can Reclaim",
            ok_title: "Disk Space Breakdown",
            ok_description: "No single reclaimable item on the system drive is large enough to matter.",
            recommendation: "Start with the largest item: the suggested action opens the right folder or tool, or clears it safely. Hibernation and page files are normal and are left alone.",
            source_tasks: &["disk_usage"],
            remediation_id: Some("open_storage_settings"),
            alternate_remediations: &[
                "open_downloads_folder",
                "clear_temp_files",
                "clear_windows_temp",
                "empty_recycle_bin",
                "windows_update_reset",
                "open_disk_cleanup",
            ],
            detect: det::detect_space_consumers,
        },
        IssueSpec {
            id: "disk_fragmentation",
            category: "Storage",
            default_severity: IssueSeverity::Warning,
            title: "High Disk Fragmentation",
            ok_title: "Disk Fragmentation",
            ok_description: "Disk fragmentation is within normal levels (<20%).",
            recommendation: "Optimize the drive. Windows normally does this weekly; running it now brings the schedule up to date.",
            source_tasks: &["disk_fragmentation"],
            remediation_id: Some("optimize_drives"),
            alternate_remediations: &["open_defrag"],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_stopped_services,
        },
        IssueSpec {
            id: "high_cpu_usage",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "High CPU Usage",
            ok_title: "CPU Usage",
            ok_description: "CPU usage is within normal range (<90%).",
            recommendation: "See which programs are using the processor on the Processes page and close the ones you do not need.",
            source_tasks: &["performance"],
            remediation_id: None,
            alternate_remediations: &[],
            detect: det::detect_high_cpu_usage,
        },
        IssueSpec {
            id: "high_memory_usage",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "High Memory Usage",
            ok_title: "Memory Usage",
            ok_description: "Memory usage is within normal range (<90%).",
            recommendation: "See which programs are using the most memory on the Processes page and close the ones you do not need.",
            source_tasks: &["performance"],
            remediation_id: None,
            alternate_remediations: &[],
            detect: det::detect_high_memory_usage,
        },
        IssueSpec {
            id: "pending_windows_updates",
            category: "System",
            default_severity: IssueSeverity::Info,
            title: "Windows Reports Pending Updates",
            ok_title: "Windows Update History",
            ok_description: "Installed update history was collected. Open Windows Update for the authoritative pending-update status.",
            recommendation: "Open Windows Update to review and install the pending updates.",
            source_tasks: &["windows_update"],
            remediation_id: Some("open_windows_update"),
            alternate_remediations: &[],
            detect: det::detect_pending_windows_updates,
        },
        IssueSpec {
            id: "windows_update_failing",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Windows Update Keeps Failing",
            ok_title: "Windows Update Installs",
            ok_description: "No Windows Update installation failed in the last 30 days without a later success.",
            recommendation: "Run the suggested fix, then open Windows Update and check for updates again. If the same code returns, note it when asking for help.",
            source_tasks: &["windows_update_events"],
            remediation_id: Some("windows_update_reset"),
            alternate_remediations: &[
                "dism_restorehealth",
                "enable_windows_update_service",
                "open_disk_cleanup",
                "clear_temp_files",
                "open_network_settings",
                "restart_system",
            ],
            detect: det::detect_windows_update_failing,
        },
        IssueSpec {
            id: "windows_update_service_disabled",
            category: "Services",
            default_severity: IssueSeverity::Warning,
            title: "Windows Update Service Is Disabled",
            ok_title: "Windows Update Service",
            ok_description: "The Windows Update service (wuauserv) is allowed to start.",
            recommendation: "Enable the Windows Update service so security updates can install again; some 'update blocker' tools disable it silently.",
            source_tasks: &["services"],
            remediation_id: Some("enable_windows_update_service"),
            alternate_remediations: &[],
            detect: det::detect_windows_update_service_disabled,
        },
        IssueSpec {
            id: "firewall_disabled",
            category: "Security",
            default_severity: IssueSeverity::Critical,
            title: "Firewall Disabled",
            ok_title: "Firewall Status",
            ok_description: "Firewall is enabled and protecting your system.",
            recommendation: "Turn the firewall on for every network profile.",
            source_tasks: &["firewall_status"],
            remediation_id: Some("enable_firewall"),
            alternate_remediations: &["open_security_center"],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_dns_misconfigured,
        },
        IssueSpec {
            id: "no_internet",
            category: "Network",
            default_severity: IssueSeverity::Warning,
            title: "No Internet Connection",
            ok_title: "Internet Connection",
            ok_description: "The connectivity test reached the internet.",
            recommendation: "Check the cable or Wi-Fi and renew the network address. If the router answers but the internet does not, the problem is on the provider's side.",
            source_tasks: &["network_path"],
            remediation_id: Some("renew_ip_lease"),
            alternate_remediations: &["open_network_settings", "network_reset"],
            detect: det::detect_no_internet,
        },
        IssueSpec {
            id: "gateway_unreachable",
            category: "Network",
            default_severity: IssueSeverity::Warning,
            title: "Router Not Reachable",
            ok_title: "Router Reachability",
            ok_description: "The default gateway answered, or the internet beyond it did.",
            recommendation: "Renew the network address, then reconnect to Wi-Fi or reseat the cable and restart the router. 'Reset network stack' is the next step if the adapter is the problem.",
            source_tasks: &["network_path"],
            remediation_id: Some("renew_ip_lease"),
            alternate_remediations: &["open_network_settings", "network_reset"],
            detect: det::detect_gateway_unreachable,
        },
        IssueSpec {
            id: "dns_resolution_failing",
            category: "Network",
            default_severity: IssueSeverity::Warning,
            title: "Web Addresses Do Not Resolve",
            ok_title: "DNS Resolution",
            ok_description: "The connectivity test resolved a well-known name.",
            recommendation: "Flush the DNS cache first; if names still fail, set the adapter's DNS server to 1.1.1.1 or 8.8.8.8 in Network settings, or run 'Reset network stack' from Maintenance.",
            source_tasks: &["network_path"],
            remediation_id: Some("flush_dns"),
            alternate_remediations: &[],
            detect: det::detect_dns_resolution_failing,
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_dism_corruption,
        },
        IssueSpec {
            id: "bsod_recent",
            category: "Debug",
            default_severity: IssueSeverity::Critical,
            title: "Recent Blue Screen Crashes",
            ok_title: "Blue Screen Crashes",
            ok_description: "No crash dumps from the last 30 days.",
            recommendation: "Start with the next step for the decoded cause. To get help, copy the minidumps (Diagnostics > Debug) and share them on WindowsForum together with the stop code.",
            source_tasks: &["minidump"],
            remediation_id: None,
            alternate_remediations: &[
                "open_device_manager",
                "open_memory_diagnostic",
                "schedule_memory_diagnostic",
                "sfc_scannow",
            ],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_defender_disabled,
        },
        IssueSpec {
            id: "realtime_protection_off",
            category: "Security",
            default_severity: IssueSeverity::Critical,
            title: "Real-Time Protection Is Off",
            ok_title: "Real-Time Protection",
            ok_description: "Microsoft Defender's real-time protection is on, or another antivirus product is in charge.",
            recommendation: "Turn real-time protection back on in Windows Security; if it keeps turning off, run a full scan — some malware disables it.",
            source_tasks: &["defender_health"],
            remediation_id: Some("open_security_center"),
            alternate_remediations: &[],
            detect: det::detect_realtime_protection_off,
        },
        IssueSpec {
            id: "defender_definitions_stale",
            category: "Security",
            default_severity: IssueSeverity::Warning,
            title: "Antivirus Definitions Are Out of Date",
            ok_title: "Antivirus Definitions",
            ok_description: "Microsoft Defender's security intelligence was updated within the last week.",
            recommendation: "Update Microsoft Defender's protection definitions; if that fails, run the Windows Update fix.",
            source_tasks: &["defender_health"],
            remediation_id: Some("update_defender_signatures"),
            alternate_remediations: &["open_security_center", "windows_update_reset"],
            detect: det::detect_defender_definitions_stale,
        },
        IssueSpec {
            id: "defender_quick_scan_overdue",
            category: "Security",
            default_severity: IssueSeverity::Info,
            title: "No Antivirus Scan This Month",
            ok_title: "Recent Antivirus Scan",
            ok_description: "Microsoft Defender ran a scan within the last 30 days.",
            recommendation: "Run a Microsoft Defender quick scan; it takes a few minutes.",
            source_tasks: &["defender_health"],
            remediation_id: Some("defender_quick_scan"),
            alternate_remediations: &["open_security_center"],
            detect: det::detect_defender_quick_scan_overdue,
        },
        IssueSpec {
            id: "pending_reboot",
            category: "System",
            default_severity: IssueSeverity::Warning,
            title: "Windows Restart Required",
            ok_title: "Restart Requirement",
            ok_description: "Windows Update and component servicing do not report a required restart.",
            recommendation: "Restart Windows to complete the operation identified above. If the same marker remains afterward, re-run this check before restarting again.",
            source_tasks: &["pending_reboot"],
            remediation_id: Some("restart_system"),
            alternate_remediations: &[],
            detect: det::detect_pending_reboot,
        },
        IssueSpec {
            id: "page_file_pressure",
            category: "Performance",
            default_severity: IssueSeverity::Warning,
            title: "Page File Nearly Full",
            ok_title: "Page File",
            ok_description: "Page file usage is within normal limits.",
            recommendation: "Close memory-heavy programs (the Processes page shows them) or add memory.",
            source_tasks: &["performance"],
            remediation_id: None,
            alternate_remediations: &[],
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
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_startup_bloat,
        },
        IssueSpec {
            id: "outdated_drivers",
            category: "Drivers",
            default_severity: IssueSeverity::Info,
            title: "Older Display/Network Driver Package",
            ok_title: "Driver Package Age",
            ok_description: "No display or network driver package is more than five years old.",
            recommendation: "Driver age alone does not prove a problem. Check the PC or device manufacturer's support page if you are troubleshooting a related symptom.",
            source_tasks: &["drivers_list"],
            remediation_id: None,
            alternate_remediations: &[],
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
            alternate_remediations: &[],
            detect: det::detect_hosts_file_hijack,
        },
    ]
}

/// Run every catalog detector against the context. Always returns one Issue
/// per spec. A clear result is emitted only when the source evidence is both
/// successful and complete enough for that particular rule.
#[cfg(test)]
pub(crate) fn detect_all(ctx: &DetectCtx) -> Vec<Issue> {
    detect_all_with(ctx, &crate::remediation::summary)
}

/// Run every catalog detector using an injected read-only remediation-summary
/// resolver. Native shells use this entry point so issue detection stays
/// independent from the action broker and cannot execute a remediation.
pub fn detect_all_with(
    ctx: &DetectCtx,
    remediation_summary: &dyn Fn(&str) -> Option<RemediationSummary>,
) -> Vec<Issue> {
    catalog()
        .iter()
        .map(|spec| match evaluate(spec, ctx) {
            DetectionOutcome::Detected(detection) => Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: detection.severity.unwrap_or(spec.default_severity),
                status: IssueStatus::Detected,
                title: spec.title.to_string(),
                description: detection.description,
                recommendation: spec.recommendation.to_string(),
                detected: true,
                source_tasks: source_tasks_for_issue(spec),
                remediation: chosen_remediation(spec, detection.remediation_id)
                    .and_then(remediation_summary),
            },
            DetectionOutcome::Clear => Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: IssueSeverity::Ok,
                status: IssueStatus::Ok,
                title: spec.ok_title.to_string(),
                description: spec.ok_description.to_string(),
                recommendation: "No action needed.".to_string(),
                detected: false,
                source_tasks: source_tasks_for_issue(spec),
                remediation: None,
            },
            DetectionOutcome::Unknown(reason) => Issue {
                id: spec.id.to_string(),
                category: spec.category.to_string(),
                severity: IssueSeverity::Info,
                status: IssueStatus::Unknown,
                title: spec.ok_title.to_string(),
                description: format!("Couldn't verify this check: {}", reason),
                recommendation: "Retry the required diagnostic tasks, or restart as administrator for access-restricted checks.".to_string(),
                detected: false,
                source_tasks: source_tasks_for_issue(spec),
                remediation: None,
            },
        })
        .collect()
}

/// The remediation an issue carries: the detector's evidence-based choice when
/// the spec declares it, else the spec's default. Never anything else.
fn chosen_remediation(spec: &IssueSpec, override_id: Option<&'static str>) -> Option<&'static str> {
    override_id
        .filter(|id| spec.remediation_id == Some(id) || spec.alternate_remediations.contains(id))
        .or(spec.remediation_id)
}

fn evaluate(spec: &IssueSpec, ctx: &DetectCtx) -> DetectionOutcome {
    if let Err(reason) = validate_evidence(spec, ctx) {
        return DetectionOutcome::Unknown(reason);
    }
    match (spec.detect)(ctx) {
        Some(detection) => DetectionOutcome::Detected(detection),
        None => DetectionOutcome::Clear,
    }
}

fn source_tasks_for_issue(spec: &IssueSpec) -> Option<Vec<String>> {
    if spec.source_tasks.is_empty() {
        None
    } else {
        Some(spec.source_tasks.iter().map(|s| s.to_string()).collect())
    }
}

/// The Defender health row must name its running mode and carry the field a
/// rule reads, in a usable shape.
fn defender_health_field(
    health: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    usable: impl Fn(&serde_json::Value) -> bool,
) -> bool {
    health
        .get("defender")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|defender| {
            defender
                .get("AMRunningMode")
                .and_then(serde_json::Value::as_str)
                .is_some()
                && defender.get(field).is_some_and(usable)
        })
}

fn validate_evidence(spec: &IssueSpec, ctx: &DetectCtx) -> Result<(), String> {
    if spec.id == "temp_files" {
        return ctx
            .temp_file_count
            .map(|_| ())
            .ok_or_else(|| "the temporary-file count was unavailable".to_string());
    }

    let mut parsed = HashMap::new();
    for task_id in spec.source_tasks {
        let result = ctx
            .results
            .get_task_result(task_id)
            .ok_or_else(|| format!("diagnostic '{}' was not run", task_id))?;
        if !result.success {
            return Err(result
                .error
                .as_deref()
                .filter(|message| !message.trim().is_empty())
                .map(|message| format!("diagnostic '{}' failed: {}", task_id, message))
                .unwrap_or_else(|| format!("diagnostic '{}' failed", task_id)));
        }
        if let Some(error) = result
            .error
            .as_deref()
            .filter(|error| !error.trim().is_empty())
        {
            return Err(format!("diagnostic '{}' reported: {}", task_id, error));
        }
        if result.output.trim().is_empty() {
            return Err(format!("diagnostic '{}' returned no data", task_id));
        }
        let value: serde_json::Value = serde_json::from_str(&result.output)
            .map_err(|_| format!("diagnostic '{}' returned malformed data", task_id))?;
        if let Some(error) = value
            .as_object()
            .and_then(|object| object.get("error"))
            .and_then(serde_json::Value::as_str)
            .filter(|error| !error.trim().is_empty())
        {
            return Err(format!("diagnostic '{}' reported: {}", task_id, error));
        }
        parsed.insert(*task_id, value);
    }

    let source = |task_id: &str| {
        parsed
            .get(task_id)
            .ok_or_else(|| format!("diagnostic '{}' returned no usable data", task_id))
    };
    let array = |task_id: &str| {
        source(task_id)?
            .as_array()
            .ok_or_else(|| format!("diagnostic '{}' returned an unexpected shape", task_id))
    };
    let object = |task_id: &str| {
        source(task_id)?
            .as_object()
            .ok_or_else(|| format!("diagnostic '{}' returned an unexpected shape", task_id))
    };
    let nonempty_array = |task_id: &str| {
        let values = array(task_id)?;
        if values.is_empty() {
            Err(format!("diagnostic '{}' returned no records", task_id))
        } else {
            Ok(values)
        }
    };
    let numeric = |value: &serde_json::Value| {
        value
            .as_u64()
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<u64>().ok())
            })
            .is_some()
    };

    let incomplete = || "the diagnostic data was incomplete for this check".to_string();
    let complete = match spec.id {
        "low_disk_space" => nonempty_array("logical_disk")?.iter().all(|disk| {
            disk.get("Name")
                .and_then(serde_json::Value::as_str)
                .is_some()
                && disk.get("FreeSpace").is_some_and(&numeric)
                && disk.get("Size").is_some_and(|size| {
                    numeric(size)
                        && size
                            .as_u64()
                            .or_else(|| {
                                size.as_str()
                                    .and_then(|text| text.trim().parse::<u64>().ok())
                            })
                            .is_some_and(|size| size > 0)
                })
        }),
        "space_consumers" => {
            let usage = object("disk_usage")?;
            usage.get("total_bytes").is_some_and(&numeric)
                && usage
                    .get("consumers")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|consumers| {
                        consumers.iter().all(|consumer| {
                            consumer
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .is_some()
                                && consumer.get("bytes").is_some_and(&numeric)
                        })
                    })
        }
        "disk_fragmentation" => nonempty_array("disk_fragmentation")?
            .iter()
            .all(|disk| disk.get("fragmentation_percent").is_some_and(&numeric)),
        "unsigned_drivers" => nonempty_array("drivers_list")?.iter().all(|driver| {
            driver
                .get("IsSigned")
                .and_then(serde_json::Value::as_bool)
                .is_some()
        }),
        "event_log_errors" | "startup_bloat" => array(spec.source_tasks[0])?
            .iter()
            .all(serde_json::Value::is_object),
        "stopped_services" => {
            let services = nonempty_array("services")?;
            ["wuauserv", "bits", "spooler", "themes", "audiosrv"]
                .iter()
                .all(|expected| {
                    services.iter().any(|service| {
                        service
                            .get("Name")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
                            && service
                                .get("StartMode")
                                .and_then(serde_json::Value::as_str)
                                .is_some()
                            && service
                                .get("State")
                                .and_then(serde_json::Value::as_str)
                                .is_some()
                    })
                })
        }
        "high_cpu_usage" => object("performance")?
            .get("cpu_performance")
            .and_then(serde_json::Value::as_object)
            .and_then(|cpu| cpu.get("LoadPercentage"))
            .is_some_and(&numeric),
        "high_memory_usage" => object("performance")?
            .get("memory_performance")
            .and_then(serde_json::Value::as_object)
            .and_then(|memory| memory.get("UsedPercent"))
            .is_some_and(&numeric),
        "page_file_pressure" => object("performance")?
            .get("memory_performance")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|memory| {
                memory.get("TotalPageFileMBytes").is_some_and(&numeric)
                    && memory.get("AvailablePageFileMBytes").is_some_and(&numeric)
            }),
        "pending_windows_updates" => object("windows_update")?
            .get("installed_updates")
            .and_then(serde_json::Value::as_array)
            .is_some(),
        "windows_update_failing" => {
            let events = object("windows_update_events")?;
            events
                .get("failures")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|failures| {
                    failures.iter().all(|failure| {
                        failure
                            .get("error_code")
                            .and_then(serde_json::Value::as_str)
                            .is_some()
                            && failure.get("time_secs").is_some_and(&numeric)
                    })
                })
                && events
                    .get("successes_after_last_failure")
                    .is_some_and(&numeric)
        }
        "windows_update_service_disabled" => nonempty_array("services")?.iter().any(|service| {
            service
                .get("Name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("wuauserv"))
                && service
                    .get("StartMode")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        }),
        "no_internet" | "gateway_unreachable" | "dns_resolution_failing" => object("network_path")?
            .get("verdict")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "realtime_protection_off" => defender_health_field(
            object("defender_health")?,
            "RealTimeProtectionEnabled",
            |value| value.as_bool().is_some(),
        ),
        "defender_definitions_stale" => defender_health_field(
            object("defender_health")?,
            "AntivirusSignatureAge",
            &numeric,
        ),
        "defender_quick_scan_overdue" => {
            defender_health_field(object("defender_health")?, "QuickScanAge", &numeric)
        }
        "firewall_disabled" | "defender_disabled" => nonempty_array(spec.source_tasks[0])?
            .iter()
            .all(|product| product.get("productState").is_some_and(&numeric)),
        "dns_misconfigured" => nonempty_array("network_adapter")?
            .iter()
            .all(serde_json::Value::is_object),
        "smart_failure_predicted" => nonempty_array_from_object(object("chkdsk")?, "disks")?
            .iter()
            .all(|disk| disk.get("OperationalStatus").is_some()),
        "disk_unhealthy" => nonempty_array_from_object(object("chkdsk")?, "disks")?
            .iter()
            .all(|disk| {
                disk.get("HealthStatus").is_some()
                    || disk.get("HealthStatusText").is_some()
                    || disk.get("OperationalStatus").is_some()
            }),
        "dism_corruption" => object("dism_health")?
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| {
                matches!(
                    status.to_ascii_lowercase().as_str(),
                    "healthy" | "repairable" | "corrupted"
                )
            }),
        "bsod_recent" => object("minidump")?
            .get("dumps")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|dumps| {
                dumps
                    .iter()
                    .all(|dump| dump.get("created").is_some_and(&numeric))
            }),
        "kernel_power_crashes"
        | "unexpected_shutdowns"
        | "disk_io_errors"
        | "whea_errors"
        | "service_crash_loops" => object("event_codes_critical")?
            .get("events")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|events| {
                events.iter().all(|event| {
                    event
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                        && event.get("code").is_some_and(&numeric)
                        && event.get("count").is_some_and(&numeric)
                })
            }),
        "device_manager_errors" => array("device_errors")?
            .iter()
            .all(|device| device.get("ConfigManagerErrorCode").is_some_and(&numeric)),
        "pending_reboot" => {
            let reboot = object("pending_reboot")?;
            let pending = reboot
                .get("pending")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(incomplete)?;
            let restart_required = match reboot.get("restart_required") {
                Some(value) => Some(value.as_bool().ok_or_else(incomplete)?),
                None => None,
            };
            let reasons = reboot
                .get("reasons")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(incomplete)?;
            let mut high_confidence = false;
            let mut deferred_only = false;
            for reason in reasons {
                match reason.as_str().ok_or_else(incomplete)? {
                    "component_based_servicing" | "windows_update" => high_confidence = true,
                    // Accepted only for compatibility with stored scans from
                    // versions that treated PFRO as a restart reason.
                    "pending_file_rename" | "pending_file_operations" => deferred_only = true,
                    other => {
                        return Err(format!(
                            "the restart diagnostic returned an unrecognized marker: {other}"
                        ));
                    }
                }
            }

            if let Some(required) = restart_required {
                if pending != required || required != high_confidence {
                    return Err(
                        "the restart diagnostic returned contradictory marker state".to_string()
                    );
                }
            } else if high_confidence != pending {
                // The one tolerated legacy exception is PFRO-only data with
                // pending=true. The detector suppresses that old false alarm.
                if !(pending && !high_confidence && deferred_only) {
                    return Err(
                        "the restart diagnostic returned contradictory marker state".to_string()
                    );
                }
            }
            if pending && reasons.is_empty() {
                return Err(
                    "the restart diagnostic did not identify the restart source".to_string()
                );
            }

            if let Some(deferred) = reboot.get("deferred_file_operations") {
                let deferred = deferred.as_object().ok_or_else(incomplete)?;
                let deferred_pending = deferred
                    .get("pending")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or_else(incomplete)?;
                let operation_count = deferred
                    .get("operation_count")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(incomplete)?;
                if deferred_pending != (operation_count > 0) {
                    return Err("the deferred-file-operation count was inconsistent".to_string());
                }
            }
            true
        }
        "battery_degraded" => object("battery_report")?
            .get("battery_summary")
            .and_then(serde_json::Value::as_object)
            .is_some(),
        "outdated_drivers" => {
            let drivers = nonempty_array("drivers_list")?;
            drivers
                .iter()
                .filter(|driver| {
                    driver
                        .get("DeviceClass")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|class| {
                            class.eq_ignore_ascii_case("DISPLAY")
                                || class.eq_ignore_ascii_case("NET")
                        })
                })
                .all(|driver| {
                    driver
                        .get("DriverDate")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                        && driver
                            .get("DeviceName")
                            .and_then(serde_json::Value::as_str)
                            .is_some()
                })
        }
        "hosts_file_hijack" => object("hosts_file")?
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| {
                entries.iter().all(|entry| {
                    entry
                        .get("ip")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                        && entry
                            .get("hostname")
                            .and_then(serde_json::Value::as_str)
                            .is_some()
                })
            }),
        _ => true,
    };

    complete.then_some(()).ok_or_else(incomplete)
}

fn nonempty_array_from_object<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<&'a Vec<serde_json::Value>, String> {
    let values = object
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "the diagnostic data had an unexpected shape".to_string())?;
    if values.is_empty() {
        Err("the diagnostic returned no records for this check".to_string())
    } else {
        Ok(values)
    }
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
    fn issue_linked_remediations_resolve_exactly_once_in_the_canonical_catalog() {
        for spec in catalog() {
            for remediation_id in spec
                .remediation_id
                .iter()
                .chain(spec.alternate_remediations.iter())
            {
                let matches = crate::remediation_catalog()
                    .iter()
                    .filter(|metadata| metadata.id == *remediation_id)
                    .count();
                assert_eq!(
                    matches, 1,
                    "issue '{}' must resolve remediation '{}' exactly once",
                    spec.id, remediation_id
                );
            }
            let mut seen = std::collections::HashSet::new();
            for remediation_id in spec.alternate_remediations {
                assert!(
                    seen.insert(*remediation_id) && spec.remediation_id != Some(*remediation_id),
                    "issue '{}' lists remediation '{}' twice",
                    spec.id,
                    remediation_id
                );
            }
        }
    }

    fn spec_with_alternates() -> IssueSpec {
        IssueSpec {
            id: "test_rule",
            category: "Test",
            default_severity: IssueSeverity::Warning,
            title: "Test",
            ok_title: "Test",
            ok_description: "Test",
            recommendation: "Test",
            source_tasks: &[],
            remediation_id: Some("open_disk_cleanup"),
            alternate_remediations: &["empty_recycle_bin"],
            detect: |_| None,
        }
    }

    #[test]
    fn override_in_alternates_is_embedded_in_issue() {
        let spec = spec_with_alternates();
        assert_eq!(
            chosen_remediation(&spec, Some("empty_recycle_bin")),
            Some("empty_recycle_bin")
        );
        assert_eq!(
            chosen_remediation(&spec, Some("open_disk_cleanup")),
            Some("open_disk_cleanup")
        );
        assert_eq!(chosen_remediation(&spec, None), Some("open_disk_cleanup"));
    }

    /// 2.6: a rule opens another program only where the fix genuinely needs
    /// the user's judgement (which device, which update, which files). Every
    /// other rule with a remediation runs a built-in fix, and CPU/memory
    /// rules point at this app's own Processes page.
    #[test]
    fn tool_handoffs_are_defaults_only_where_no_safe_built_in_fix_exists() {
        const HANDOFF_DEFAULTS: [&str; 6] = [
            "space_consumers",
            "pending_windows_updates",
            "device_manager_errors",
            "defender_disabled",
            "realtime_protection_off",
            "startup_bloat",
        ];
        for spec in catalog() {
            let Some(remediation_id) = spec.remediation_id else {
                continue;
            };
            let tier = wfdiag_remediation_catalog::find(remediation_id)
                .unwrap_or_else(|| panic!("{}: unknown remediation", spec.id))
                .tier;
            assert_eq!(
                tier == wfdiag_remediation_catalog::RemediationTier::OpenTool,
                HANDOFF_DEFAULTS.contains(&spec.id),
                "{}: default '{remediation_id}' has tier {tier:?}",
                spec.id
            );
        }
    }

    #[test]
    fn override_outside_alternates_falls_back_to_spec_default() {
        let spec = spec_with_alternates();
        assert_eq!(
            chosen_remediation(&spec, Some("sfc_scannow")),
            Some("open_disk_cleanup"),
            "a detector cannot widen the closed remediation set"
        );
        let no_default = IssueSpec {
            remediation_id: None,
            alternate_remediations: &[],
            ..spec_with_alternates()
        };
        assert_eq!(chosen_remediation(&no_default, Some("sfc_scannow")), None);
    }

    #[test]
    fn empty_context_yields_full_sweep_with_no_detections() {
        let results: HashMap<String, TaskResult> = HashMap::new();
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
            assert_eq!(issue.status, IssueStatus::Unknown);
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

    #[test]
    fn deferred_file_operations_never_get_actionable_restart_copy() {
        let mut results = HashMap::new();
        results.insert(
            "pending_reboot".to_string(),
            TaskResult {
                success: true,
                // Legacy shape deliberately says pending=true: the detector
                // must still reject PFRO as an actionable restart source.
                output: r#"{"pending":true,"reasons":["pending_file_rename"],"deferred_file_operations":{"pending":true,"operation_count":48}}"#.into(),
                error: None,
                duration_ms: 1,
            },
        );
        let issue = issue_for(&results, "pending_reboot");
        assert_eq!(issue.status, IssueStatus::Ok);
        assert!(!issue.detected);
        assert!(issue.remediation.is_none());
        assert_eq!(issue.recommendation, "No action needed.");
    }

    #[test]
    fn contradictory_or_unrecognized_restart_evidence_is_unknown() {
        for output in [
            r#"{"pending":true,"restart_required":true,"reasons":[]}"#,
            r#"{"pending":false,"restart_required":false,"reasons":["windows_update"]}"#,
            r#"{"pending":false,"restart_required":false,"reasons":["future_marker"]}"#,
            r#"{"pending":false,"restart_required":false,"reasons":[7]}"#,
        ] {
            let mut results = HashMap::new();
            results.insert(
                "pending_reboot".to_string(),
                TaskResult {
                    success: true,
                    output: output.into(),
                    error: None,
                    duration_ms: 1,
                },
            );
            let issue = issue_for(&results, "pending_reboot");
            assert_eq!(issue.status, IssueStatus::Unknown, "output: {output}");
            assert!(!issue.detected);
            assert!(issue.remediation.is_none());
        }
    }

    fn issue_for(results: &HashMap<String, TaskResult>, id: &str) -> Issue {
        detect_all(&test_support::ctx(results))
            .into_iter()
            .find(|issue| issue.id == id)
            .expect("catalog issue exists")
    }

    #[test]
    fn malformed_successful_output_is_unknown_not_ok() {
        let mut results = HashMap::new();
        results.insert(
            "logical_disk".to_string(),
            TaskResult {
                success: true,
                output: "not json".into(),
                error: None,
                duration_ms: 1,
            },
        );
        assert_eq!(
            issue_for(&results, "low_disk_space").status,
            IssueStatus::Unknown
        );
    }

    #[test]
    fn partial_successful_output_is_unknown_not_ok() {
        let mut results = HashMap::new();
        results.insert(
            "logical_disk".to_string(),
            TaskResult {
                success: true,
                output: r#"[{"Name":"C:","Size":100000000000}]"#.into(),
                error: None,
                duration_ms: 1,
            },
        );
        assert_eq!(
            issue_for(&results, "low_disk_space").status,
            IssueStatus::Unknown
        );
    }

    #[test]
    fn embedded_access_error_is_unknown_not_ok() {
        let mut results = HashMap::new();
        results.insert(
            "hosts_file".to_string(),
            TaskResult {
                success: true,
                output: r#"{"error":"Access is denied"}"#.into(),
                error: None,
                duration_ms: 1,
            },
        );
        let issue = issue_for(&results, "hosts_file_hijack");
        assert_eq!(issue.status, IssueStatus::Unknown);
        assert!(issue.description.contains("Access is denied"));
    }

    #[test]
    fn empty_required_records_are_unknown_not_ok() {
        let mut results = HashMap::new();
        results.insert(
            "firewall_status".to_string(),
            TaskResult {
                success: true,
                output: "[]".into(),
                error: None,
                duration_ms: 1,
            },
        );
        assert_eq!(
            issue_for(&results, "firewall_disabled").status,
            IssueStatus::Unknown
        );
    }
}
