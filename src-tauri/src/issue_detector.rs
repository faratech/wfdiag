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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,
    Warning,
    Info,
}

pub struct IssueDetector;

impl IssueDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn detect_issues(&self, results: &std::collections::HashMap<String, TaskResult>) -> Vec<Issue> {
        let mut issues = Vec::new();

        self.detect_low_disk_space(results, &mut issues);
        self.detect_disk_fragmentation(results, &mut issues);
        self.detect_unsigned_drivers(results, &mut issues);
        self.detect_event_log_errors(results, &mut issues);
        self.detect_stopped_services(results, &mut issues);
        self.detect_high_cpu_usage(results, &mut issues);
        self.detect_high_memory_usage(results, &mut issues);
        self.detect_pending_windows_updates(results, &mut issues);
        self.detect_firewall_disabled(results, &mut issues);

        issues
    }

    '''    fn detect_low_disk_space(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
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
                                    issues.push(Issue {
                                        id: "low_disk_space".to_string(),
                                        category: "Storage".to_string(),
                                        severity: IssueSeverity::Critical,
                                        title: "Low Disk Space".to_string(),
                                        description: format!(
                                            "The disk '{}' is running low on space ({:.2}% free).",
                                            disk["Name"].as_str().unwrap_or("Unknown"),
                                            free_percent
                                        ),
                                        recommendation: "You should free up some disk space by deleting unnecessary files or moving them to another drive.".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn detect_disk_fragmentation(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("disk_fragmentation") {
            if result.success {
                if let Ok(disks) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for disk in disks {
                        if let Some(fragmentation) = disk["fragmentation_percent"].as_u64() {
                            if fragmentation > 20 {
                                issues.push(Issue {
                                    id: "disk_fragmentation".to_string(),
                                    category: "Storage".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "High Disk Fragmentation".to_string(),
                                    description: format!(
                                        "The disk '{}' has {}% fragmentation. High fragmentation can slow down disk performance.",
                                        disk["drive"].as_str().unwrap_or("Unknown"),
                                        fragmentation
                                    ),
                                    recommendation: "You should defragment your disk to improve performance. You can use the built-in Windows Disk Defragmenter tool.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }''

    fn detect_unsigned_drivers(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("drivers_list") {
            if result.success {
                if let Ok(drivers) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for driver in drivers {
                        if let Some(is_signed) = driver["IsSigned"].as_bool() {
                            if !is_signed {
                                issues.push(Issue {
                                    id: "unsigned_driver".to_string(),
                                    category: "Drivers".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "Unsigned Driver".to_string(),
                                    description: format!(
                                        "The driver '{}' is not signed. Unsigned drivers can cause system instability.",
                                        driver["DeviceName"].as_str().unwrap_or("Unknown")
                                    ),
                                    recommendation: "You should try to find a signed version of the driver from the manufacturer's website.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn detect_event_log_errors(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("event_logs") {
            if result.success {
                if let Ok(events) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    if !events.is_empty() {
                        issues.push(Issue {
                            id: "event_log_errors".to_string(),
                            category: "Logs".to_string(),
                            severity: IssueSeverity::Warning,
                            title: "Errors in Event Log".to_string(),
                            description: format!(
                                "There are {} errors in the event log. These errors can indicate problems with your system.",
                                events.len()
                            ),
                            recommendation: "You should review the event log for more information about the errors.".to_string(),
                        });
                    }
                }
            }
        }
    }

    fn detect_stopped_services(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("services") {
            if result.success {
                if let Ok(services) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for service in services {
                        if let (Some(state), Some(start_mode)) = (
                            service["State"].as_str(),
                            service["StartMode"].as_str(),
                        ) {
                            if start_mode == "Auto" && state != "Running" {
                                issues.push(Issue {
                                    id: "stopped_service".to_string(),
                                    category: "Services".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "Stopped Service".to_string(),
                                    description: format!(
                                        "The service '{}' is set to start automatically but is not currently running.",
                                        service["DisplayName"].as_str().unwrap_or("Unknown")
                                    ),
                                    recommendation: "You should try to start the service and investigate why it is not running.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn detect_high_cpu_usage(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("performance") {
            if result.success {
                if let Ok(performance) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let Some(cpu_usage) = performance["cpu_performance"]["PercentProcessorTime"].as_u64() {
                        if cpu_usage > 90 {
                            issues.push(Issue {
                                id: "high_cpu_usage".to_string(),
                                category: "Performance".to_string(),
                                severity: IssueSeverity::Warning,
                                title: "High CPU Usage".to_string(),
                                description: format!(
                                    "The CPU usage is currently at {}%. High CPU usage can cause system slowdowns.",
                                    cpu_usage
                                ),
                                recommendation: "You should check which processes are using the most CPU and investigate why.".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    '''    fn detect_high_memory_usage(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
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
                                issues.push(Issue {
                                    id: "high_memory_usage".to_string(),
                                    category: "Performance".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "High Memory Usage".to_string(),
                                    description: format!(
                                        "The memory usage is currently at {:.2}%. High memory usage can cause system slowdowns.",
                                        used_percent
                                    ),
                                    recommendation: "You should check which processes are using the most memory and investigate why.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    '''    fn detect_pending_windows_updates(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("windows_update") {
            if result.success {
                if let Ok(update_info) = serde_json::from_str::<serde_json::Value>(&result.output) {
                    if let Some(updates) = update_info["installed_updates"].as_array() {
                        let mut most_recent_update: Option<DateTime<Utc>> = None;

                        for update in updates {
                            if let Some(installed_on_str) = update["InstalledOn"].as_str() {
                                // Date format from WMI is M/D/YYYY
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
                                issues.push(Issue {
                                    id: "pending_windows_updates".to_string(),
                                    category: "System".to_string(),
                                    severity: IssueSeverity::Warning,
                                    title: "Pending Windows Updates".to_string(),
                                    description: format!(
                                        "Your last Windows update was on {}. Your system may be missing important security updates.",
                                        last_update_date.format("%Y-%m-%d")
                                    ),
                                    recommendation: "You should check for and install any pending Windows updates.".to_string(),
                                });
                            }
                        } else {
                             issues.push(Issue {
                                id: "unknown_windows_update_status".to_string(),
                                category: "System".to_string(),
                                severity: IssueSeverity::Info,
                                title: "Windows Update Status Unknown".to_string(),
                                description: "Could not determine the date of the last Windows update.".to_string(),
                                recommendation: "You should manually check for Windows updates to ensure your system is up to date.".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    fn detect_firewall_disabled(&self, results: &std::collections::HashMap<String, TaskResult>, issues: &mut Vec<Issue>) {
        if let Some(result) = results.get("firewall_status") {
            if result.success {
                if let Ok(firewalls) = serde_json::from_str::<Vec<serde_json::Value>>(&result.output) {
                    for firewall in firewalls {
                        if let Some(product_state) = firewall["productState"].as_u64() {
                            // 262144 = Disabled
                            if product_state == 262144 {
                                issues.push(Issue {
                                    id: "firewall_disabled".to_string(),
                                    category: "Security".to_string(),
                                    severity: IssueSeverity::Critical,
                                    title: "Firewall Disabled".to_string(),
                                    description: format!(
                                        "The firewall '{}' is disabled. This is a security risk.",
                                        firewall["displayName"].as_str().unwrap_or("Unknown")
                                    ),
                                    recommendation: "You should enable your firewall to protect your system from network threats.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}''''''
}