use crate::diagnostics::TaskResult;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub category: String,
    pub severity: IssueSeverity,
    pub title: String,
    pub description: String,
    pub recommendation: String,
    pub detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,
    Warning,
    Info,
    Ok,
}

pub struct IssueDetector;

impl IssueDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn detect_issues(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Vec<Issue> {
        // Check each issue type and add to list with status
        vec![
            self.check_disk_space(results),
            self.check_disk_fragmentation(results),
            self.check_unsigned_drivers(results),
            self.check_event_log_errors(results),
            self.check_stopped_services(results),
            self.check_high_cpu_usage(results),
            self.check_high_memory_usage(results),
            self.check_pending_windows_updates(results),
            self.check_firewall_status(results),
            self.check_temp_files(),
            self.check_dns_cache(results),
        ]
    }

    fn check_disk_space(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("logical_disk")
            && result.success
            && let Ok(disks) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
        {
            for disk in disks {
                if let (Some(free_space), Some(size)) =
                    (disk["FreeSpace"].as_u64(), disk["Size"].as_u64())
                    && size > 0
                {
                    let free_percent = (free_space as f64 / size as f64) * 100.0;
                    if free_percent < 10.0 {
                        return Issue {
                            id: "low_disk_space".to_string(),
                            category: "Storage".to_string(),
                            severity: IssueSeverity::Critical,
                            title: "Low Disk Space".to_string(),
                            description: format!(
                                "The disk '{}' is running low on space ({:.2}% free).",
                                disk["Name"].as_str().unwrap_or("Unknown"),
                                free_percent
                            ),
                            recommendation: "Free up disk space by deleting unnecessary files."
                                .to_string(),
                            detected: true,
                        };
                    }
                }
            }
        }
        Issue {
            id: "low_disk_space".to_string(),
            category: "Storage".to_string(),
            severity: IssueSeverity::Ok,
            title: "Disk Space".to_string(),
            description: "All disks have adequate free space (>10%).".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_disk_fragmentation(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("disk_fragmentation")
            && result.success
            && let Ok(disks) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
        {
            for disk in disks {
                if let Some(fragmentation) = disk["fragmentation_percent"].as_u64()
                    && fragmentation > 20
                {
                    return Issue {
                        id: "disk_fragmentation".to_string(),
                        category: "Storage".to_string(),
                        severity: IssueSeverity::Warning,
                        title: "High Disk Fragmentation".to_string(),
                        description: format!(
                            "The disk '{}' has {}% fragmentation.",
                            disk["drive"].as_str().unwrap_or("Unknown"),
                            fragmentation
                        ),
                        recommendation: "Defragment your disk to improve performance.".to_string(),
                        detected: true,
                    };
                }
            }
        }
        Issue {
            id: "disk_fragmentation".to_string(),
            category: "Storage".to_string(),
            severity: IssueSeverity::Ok,
            title: "Disk Fragmentation".to_string(),
            description: "Disk fragmentation is within normal levels (<20%).".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_unsigned_drivers(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        let mut unsigned_count = 0;
        if let Some(result) = results.get("drivers_list")
            && result.success
            && let Ok(drivers) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
        {
            for driver in &drivers {
                if let Some(is_signed) = driver["IsSigned"].as_bool()
                    && !is_signed
                {
                    unsigned_count += 1;
                }
            }
            if unsigned_count > 0 {
                return Issue {
                    id: "unsigned_drivers".to_string(),
                    category: "Drivers".to_string(),
                    severity: IssueSeverity::Warning,
                    title: "Unsigned Drivers Detected".to_string(),
                    description: format!(
                        "Found {} unsigned driver(s) that could cause instability.",
                        unsigned_count
                    ),
                    recommendation: "Update drivers from manufacturer websites.".to_string(),
                    detected: true,
                };
            }
        }
        Issue {
            id: "unsigned_drivers".to_string(),
            category: "Drivers".to_string(),
            severity: IssueSeverity::Ok,
            title: "Driver Signatures".to_string(),
            description: "All drivers are properly signed.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_event_log_errors(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("event_logs")
            && result.success
            && let Ok(events) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
            && !events.is_empty()
        {
            return Issue {
                id: "event_log_errors".to_string(),
                category: "Logs".to_string(),
                severity: IssueSeverity::Warning,
                title: "Event Log Errors".to_string(),
                description: format!("Found {} error(s) in system event logs.", events.len()),
                recommendation: "Review event logs for details.".to_string(),
                detected: true,
            };
        }
        Issue {
            id: "event_log_errors".to_string(),
            category: "Logs".to_string(),
            severity: IssueSeverity::Ok,
            title: "Event Logs".to_string(),
            description: "No critical errors found in event logs.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_stopped_services(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        let mut stopped_count = 0;
        if let Some(result) = results.get("services")
            && result.success
            && let Ok(services) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
        {
            for service in &services {
                if let (Some(state), Some(start_mode)) =
                    (service["State"].as_str(), service["StartMode"].as_str())
                    && start_mode == "Auto"
                    && state != "Running"
                {
                    stopped_count += 1;
                }
            }
            if stopped_count > 0 {
                return Issue {
                    id: "stopped_services".to_string(),
                    category: "Services".to_string(),
                    severity: IssueSeverity::Warning,
                    title: "Stopped Services".to_string(),
                    description: format!("{} automatic service(s) are not running.", stopped_count),
                    recommendation: "Start stopped services or investigate why they failed."
                        .to_string(),
                    detected: true,
                };
            }
        }
        Issue {
            id: "stopped_services".to_string(),
            category: "Services".to_string(),
            severity: IssueSeverity::Ok,
            title: "Windows Services".to_string(),
            description: "All automatic services are running normally.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_high_cpu_usage(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("performance")
            && result.success
            && let Ok(performance) = serde_json::from_str::<serde_json::Value>(&result.output)
            && let Some(cpu_usage) = performance["cpu_performance"]["PercentProcessorTime"].as_u64()
            && cpu_usage > 90
        {
            return Issue {
                id: "high_cpu_usage".to_string(),
                category: "Performance".to_string(),
                severity: IssueSeverity::Warning,
                title: "High CPU Usage".to_string(),
                description: format!("CPU usage is at {}%.", cpu_usage),
                recommendation: "Check Task Manager for resource-intensive processes.".to_string(),
                detected: true,
            };
        }
        Issue {
            id: "high_cpu_usage".to_string(),
            category: "Performance".to_string(),
            severity: IssueSeverity::Ok,
            title: "CPU Usage".to_string(),
            description: "CPU usage is within normal range (<90%).".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_high_memory_usage(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("performance")
            && result.success
            && let Ok(performance) = serde_json::from_str::<serde_json::Value>(&result.output)
            && let (Some(total_memory), Some(available_memory)) = (
                performance["memory_performance"]["TotalVisibleMemorySize"].as_u64(),
                performance["memory_performance"]["FreePhysicalMemory"].as_u64(),
            )
            && total_memory > 0
        {
            let used_memory = total_memory - available_memory;
            let used_percent = (used_memory as f64 / total_memory as f64) * 100.0;
            if used_percent > 90.0 {
                return Issue {
                    id: "high_memory_usage".to_string(),
                    category: "Performance".to_string(),
                    severity: IssueSeverity::Warning,
                    title: "High Memory Usage".to_string(),
                    description: format!("Memory usage is at {:.1}%.", used_percent),
                    recommendation: "Close unnecessary programs to free memory.".to_string(),
                    detected: true,
                };
            }
        }
        Issue {
            id: "high_memory_usage".to_string(),
            category: "Performance".to_string(),
            severity: IssueSeverity::Ok,
            title: "Memory Usage".to_string(),
            description: "Memory usage is within normal range (<90%).".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_pending_windows_updates(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("windows_update")
            && result.success
            && let Ok(update_info) = serde_json::from_str::<serde_json::Value>(&result.output)
            && let Some(updates) = update_info["installed_updates"].as_array()
        {
            let mut most_recent_update: Option<Timestamp> = None;
            for update in updates {
                if let Some(installed_on_str) = update["InstalledOn"].as_str()
                    && let Some(ts) = Self::parse_windows_date(installed_on_str)
                    && most_recent_update.is_none_or(|t| ts > t)
                {
                    most_recent_update = Some(ts);
                }
            }
            if let Some(last_update_date) = most_recent_update {
                let now = Timestamp::now();
                let thirty_days = Duration::from_secs(30 * 24 * 60 * 60);
                if last_update_date.is_before(&now, thirty_days) {
                    return Issue {
                        id: "pending_windows_updates".to_string(),
                        category: "System".to_string(),
                        severity: IssueSeverity::Warning,
                        title: "Outdated Windows Updates".to_string(),
                        description: format!(
                            "Last update was {}.",
                            last_update_date
                                .to_iso_string()
                                .split('T')
                                .next()
                                .unwrap_or("unknown")
                        ),
                        recommendation: "Check for and install pending updates.".to_string(),
                        detected: true,
                    };
                }
            }
        }
        Issue {
            id: "pending_windows_updates".to_string(),
            category: "System".to_string(),
            severity: IssueSeverity::Ok,
            title: "Windows Updates".to_string(),
            description: "System is up to date.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    /// Parse a Windows date format (MM/DD/YYYY) to a Timestamp
    fn parse_windows_date(date_str: &str) -> Option<Timestamp> {
        let parts: Vec<&str> = date_str.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let month: u32 = parts[0].parse().ok()?;
        let day: u32 = parts[1].parse().ok()?;
        let year: i32 = parts[2].parse().ok()?;

        // Create ISO format and parse
        let iso = format!("{:04}-{:02}-{:02}T00:00:00Z", year, month, day);
        Timestamp::from_iso_string(&iso).ok()
    }

    fn check_firewall_status(
        &self,
        results: &std::collections::HashMap<String, TaskResult>,
    ) -> Issue {
        if let Some(result) = results.get("firewall_status")
            && result.success
            && let Ok(firewalls) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output)
        {
            for firewall in &firewalls {
                // Windows Security Center productState is a packed bitfield, NOT a
                // single enum value. The enabled/disabled state is the second byte
                // (bits 8-15): the 0x10 bit set => ON; 0x00/0x01 => OFF. The old
                // `== 262144` exact match missed most real disabled states (which
                // vary by provider and Windows version).
                if let Some(product_state) = firewall["productState"].as_u64() {
                    let enabled_byte = (product_state >> 8) & 0xFF;
                    let firewall_on = (enabled_byte & 0x10) != 0;
                    if !firewall_on {
                        return Issue {
                            id: "firewall_disabled".to_string(),
                            category: "Security".to_string(),
                            severity: IssueSeverity::Critical,
                            title: "Firewall Disabled".to_string(),
                            description: format!(
                                "'{}' is disabled.",
                                firewall["displayName"].as_str().unwrap_or("Firewall")
                            ),
                            recommendation: "Enable firewall for network protection.".to_string(),
                            detected: true,
                        };
                    }
                }
            }
        }
        Issue {
            id: "firewall_disabled".to_string(),
            category: "Security".to_string(),
            severity: IssueSeverity::Ok,
            title: "Firewall Status".to_string(),
            description: "Firewall is enabled and protecting your system.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_temp_files(&self) -> Issue {
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            let count = entries.count();
            if count > 100 {
                return Issue {
                    id: "temp_files".to_string(),
                    category: "Performance".to_string(),
                    severity: IssueSeverity::Warning,
                    title: "Excessive Temporary Files".to_string(),
                    description: format!("Found {} files in temp directory.", count),
                    recommendation: "Clean temporary files to free disk space.".to_string(),
                    detected: true,
                };
            }
        }
        Issue {
            id: "temp_files".to_string(),
            category: "Performance".to_string(),
            severity: IssueSeverity::Ok,
            title: "Temporary Files".to_string(),
            description: "Temporary files are within normal limits.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }

    fn check_dns_cache(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("network_adapter")
            && (!result.success || result.output.contains("error"))
        {
            return Issue {
                id: "dns_cache".to_string(),
                category: "Network".to_string(),
                severity: IssueSeverity::Info,
                title: "DNS Cache May Need Refresh".to_string(),
                description: "Network connectivity issues detected.".to_string(),
                recommendation: "Clear DNS cache if experiencing issues.".to_string(),
                detected: true,
            };
        }
        Issue {
            id: "dns_cache".to_string(),
            category: "Network".to_string(),
            severity: IssueSeverity::Ok,
            title: "DNS Cache".to_string(),
            description: "DNS resolution is working normally.".to_string(),
            recommendation: "No action needed.".to_string(),
            detected: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ok_result(output: &str) -> TaskResult {
        TaskResult {
            success: true,
            output: output.to_string(),
            error: None,
            duration_ms: 1,
        }
    }

    fn results_with(task_id: &str, output: &str) -> HashMap<String, TaskResult> {
        HashMap::from([(task_id.to_string(), ok_result(output))])
    }

    #[test]
    fn low_disk_space_detected_below_ten_percent() {
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": 5000000000, "Size": 100000000000}]"#,
        );
        let issue = IssueDetector::new().check_disk_space(&results);
        assert!(issue.detected);
        assert!(matches!(issue.severity, IssueSeverity::Critical));
        assert!(issue.description.contains("C:"));
    }

    #[test]
    fn adequate_disk_space_not_flagged() {
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": 50000000000, "Size": 100000000000}]"#,
        );
        let issue = IssueDetector::new().check_disk_space(&results);
        assert!(!issue.detected);
    }

    #[test]
    fn malformed_disk_output_does_not_false_positive() {
        let results = results_with("logical_disk", "not json at all");
        assert!(!IssueDetector::new().check_disk_space(&results).detected);
        // Zero-size disk must not divide by zero or flag
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "X:", "FreeSpace": 0, "Size": 0}]"#,
        );
        assert!(!IssueDetector::new().check_disk_space(&results).detected);
    }

    #[test]
    fn stopped_auto_service_detected() {
        let results = results_with(
            "services",
            r#"[{"Name": "wuauserv", "State": "Stopped", "StartMode": "Auto"},
                {"Name": "Spooler", "State": "Running", "StartMode": "Auto"}]"#,
        );
        let issue = IssueDetector::new().check_stopped_services(&results);
        assert!(issue.detected);
        assert!(issue.description.contains('1'));
    }

    #[test]
    fn manual_stopped_service_not_flagged() {
        let results = results_with(
            "services",
            r#"[{"Name": "Fax", "State": "Stopped", "StartMode": "Manual"}]"#,
        );
        assert!(
            !IssueDetector::new()
                .check_stopped_services(&results)
                .detected
        );
    }

    #[test]
    fn high_memory_usage_threshold() {
        // 95% used -> flagged
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalVisibleMemorySize": 100, "FreePhysicalMemory": 5}}"#,
        );
        assert!(
            IssueDetector::new()
                .check_high_memory_usage(&results)
                .detected
        );
        // 50% used -> fine
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalVisibleMemorySize": 100, "FreePhysicalMemory": 50}}"#,
        );
        assert!(
            !IssueDetector::new()
                .check_high_memory_usage(&results)
                .detected
        );
    }

    #[test]
    fn high_cpu_usage_threshold() {
        let results = results_with(
            "performance",
            r#"{"cpu_performance": {"PercentProcessorTime": 97}}"#,
        );
        assert!(IssueDetector::new().check_high_cpu_usage(&results).detected);
        let results = results_with(
            "performance",
            r#"{"cpu_performance": {"PercentProcessorTime": 40}}"#,
        );
        assert!(!IssueDetector::new().check_high_cpu_usage(&results).detected);
    }

    #[test]
    fn firewall_product_state_bitfield() {
        // Second byte 0x10 bit set => firewall ON (e.g. 0x1000 = 4096)
        let results = results_with(
            "firewall_status",
            r#"[{"displayName": "Windows Firewall", "productState": 4096}]"#,
        );
        assert!(
            !IssueDetector::new()
                .check_firewall_status(&results)
                .detected
        );
        // 0x0100: second byte 0x01 => OFF
        let results = results_with(
            "firewall_status",
            r#"[{"displayName": "Windows Firewall", "productState": 256}]"#,
        );
        let issue = IssueDetector::new().check_firewall_status(&results);
        assert!(issue.detected);
        assert!(matches!(issue.severity, IssueSeverity::Critical));
    }

    #[test]
    fn parse_windows_date_formats() {
        assert!(IssueDetector::parse_windows_date("6/12/2026").is_some());
        assert!(IssueDetector::parse_windows_date("12/31/2025").is_some());
        assert!(IssueDetector::parse_windows_date("2026-06-12").is_none());
        assert!(IssueDetector::parse_windows_date("garbage").is_none());
        assert!(IssueDetector::parse_windows_date("6/12").is_none());
    }

    #[test]
    fn empty_results_yield_no_detections() {
        let issues = IssueDetector::new().detect_issues(&HashMap::new());
        // temp_files probes the real temp dir, so exclude it from the assertion
        for issue in issues.iter().filter(|i| i.id != "temp_files") {
            assert!(
                !issue.detected,
                "issue '{}' detected on an empty result set",
                issue.id
            );
        }
        // Every detector reports exactly once
        let mut ids: Vec<_> = issues.iter().map(|i| i.id.as_str()).collect();
        ids.sort_unstable();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate issue ids emitted");
    }
}
