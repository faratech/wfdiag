use serde::{Deserialize, Serialize};
use crate::diagnostics::TaskResult;
use chrono::{DateTime, Utc};

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

    pub fn detect_issues(&self, results: &std::collections::HashMap<String, TaskResult>) -> Vec<Issue> {
        let mut issues = Vec::new();

        // Check each issue type and add to list with status
        issues.push(self.check_disk_space(results));
        issues.push(self.check_disk_fragmentation(results));
        issues.push(self.check_unsigned_drivers(results));
        issues.push(self.check_event_log_errors(results));
        issues.push(self.check_stopped_services(results));
        issues.push(self.check_high_cpu_usage(results));
        issues.push(self.check_high_memory_usage(results));
        issues.push(self.check_pending_windows_updates(results));
        issues.push(self.check_firewall_status(results));
        issues.push(self.check_temp_files());
        issues.push(self.check_dns_cache(results));

        issues
    }

    fn check_disk_space(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("logical_disk") {
            if result.success {
                if let Ok(disks) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for disk in disks {
                        if let (Some(free_space), Some(size)) = (
                            disk["FreeSpace"].as_u64(),
                            disk["Size"].as_u64(),
                        ) {
                            if size > 0 {
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
                                        recommendation: "Free up disk space by deleting unnecessary files.".to_string(),
                                        detected: true,
                                    };
                                }
                            }
                        }
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

    fn check_disk_fragmentation(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("disk_fragmentation") {
            if result.success {
                if let Ok(disks) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for disk in disks {
                        if let Some(fragmentation) = disk["fragmentation_percent"].as_u64() {
                            if fragmentation > 20 {
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

    fn check_unsigned_drivers(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        let mut unsigned_count = 0;
        if let Some(result) = results.get("drivers_list") {
            if result.success {
                if let Ok(drivers) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for driver in &drivers {
                        if let Some(is_signed) = driver["IsSigned"].as_bool() {
                            if !is_signed {
                                unsigned_count += 1;
                            }
                        }
                    }
                    if unsigned_count > 0 {
                        return Issue {
                            id: "unsigned_drivers".to_string(),
                            category: "Drivers".to_string(),
                            severity: IssueSeverity::Warning,
                            title: "Unsigned Drivers Detected".to_string(),
                            description: format!("Found {} unsigned driver(s) that could cause instability.", unsigned_count),
                            recommendation: "Update drivers from manufacturer websites.".to_string(),
                            detected: true,
                        };
                    }
                }
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

    fn check_event_log_errors(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("event_logs") {
            if result.success {
                if let Ok(events) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    if !events.is_empty() {
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
                }
            }
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

    fn check_stopped_services(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        let mut stopped_count = 0;
        if let Some(result) = results.get("services") {
            if result.success {
                if let Ok(services) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for service in &services {
                        if let (Some(state), Some(start_mode)) = (
                            service["State"].as_str(),
                            service["StartMode"].as_str(),
                        ) {
                            if start_mode == "Auto" && state != "Running" {
                                stopped_count += 1;
                            }
                        }
                    }
                    if stopped_count > 0 {
                        return Issue {
                            id: "stopped_services".to_string(),
                            category: "Services".to_string(),
                            severity: IssueSeverity::Warning,
                            title: "Stopped Services".to_string(),
                            description: format!("{} automatic service(s) are not running.", stopped_count),
                            recommendation: "Start stopped services or investigate why they failed.".to_string(),
                            detected: true,
                        };
                    }
                }
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

    fn check_high_cpu_usage(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("performance") {
            if result.success {
                if let Ok(performance) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let Some(cpu_usage) = performance["cpu_performance"]["PercentProcessorTime"].as_u64() {
                        if cpu_usage > 90 {
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
                    }
                }
            }
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

    fn check_high_memory_usage(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("performance") {
            if result.success {
                if let Ok(performance) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let (Some(total_memory), Some(available_memory)) = (
                        performance["memory_performance"]["TotalVisibleMemorySize"].as_u64(),
                        performance["memory_performance"]["FreePhysicalMemory"].as_u64(),
                    ) {
                        if total_memory > 0 {
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
                    }
                }
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

    fn check_pending_windows_updates(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("windows_update") {
            if result.success {
                if let Ok(update_info) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let Some(updates) = update_info["installed_updates"].as_array() {
                        let mut most_recent_update: Option<DateTime<Utc>> = None;
                        for update in updates {
                            if let Some(installed_on_str) = update["InstalledOn"].as_str() {
                                if let Ok(datetime) = DateTime::parse_from_str(&format!("{} 00:00:00 +0000", installed_on_str), "%m/%d/%Y %H:%M:%S %z") {
                                    let datetime_utc = datetime.with_timezone(&Utc);
                                    if most_recent_update.is_none() || datetime_utc > most_recent_update.unwrap() {
                                        most_recent_update = Some(datetime_utc);
                                    }
                                }
                            }
                        }
                        if let Some(last_update_date) = most_recent_update {
                            let thirty_days_ago = Utc::now() - chrono::Duration::days(30);
                            if last_update_date < thirty_days_ago {
                                return Issue {
                                    id: "pending_windows_updates".to_string(),
                                    category: "System".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "Outdated Windows Updates".to_string(),
                                    description: format!("Last update was {}.", last_update_date.format("%Y-%m-%d")),
                                    recommendation: "Check for and install pending updates.".to_string(),
                                    detected: true,
                                };
                            }
                        }
                    }
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

    fn check_firewall_status(&self, results: &std::collections::HashMap<String, TaskResult>) -> Issue {
        if let Some(result) = results.get("firewall_status") {
            if result.success {
                if let Ok(firewalls) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for firewall in &firewalls {
                        if let Some(product_state) = firewall["productState"].as_u64() {
                            if product_state == 262144 {
                                return Issue {
                                    id: "firewall_disabled".to_string(),
                                    category: "Security".to_string(),
                                    severity: IssueSeverity::Critical,
                                    title: "Firewall Disabled".to_string(),
                                    description: format!("'{}' is disabled.", firewall["displayName"].as_str().unwrap_or("Firewall")),
                                    recommendation: "Enable firewall for network protection.".to_string(),
                                    detected: true,
                                };
                            }
                        }
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
        if let Some(result) = results.get("network_adapter") {
            if !result.success || result.output.contains("error") {
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