//! Pure detection functions for the issue catalog.
//!
//! Each function takes a [`DetectCtx`] and returns `Some(Detection)` when the
//! issue is present. Metadata (ids, titles, recommendations, remediation
//! mapping) lives in [`crate::issue_catalog`] — keep these functions about
//! parsing diagnostic output and thresholds only. Everything here must be
//! deterministic: time comes from `ctx.now`, environment state (temp-file
//! count) is injected by the caller.

use crate::issue_catalog::{DetectCtx, Detection, IssueSeverity};
use crate::timestamp::Timestamp;
use serde_json::Value;
use std::time::Duration;

/// Parse the diagnostic output for `task_id` as a JSON array, if the task
/// succeeded. The common shape for WMI-backed tasks.
fn task_array(ctx: &DetectCtx, task_id: &str) -> Option<Vec<Value>> {
    let result = ctx.results.get(task_id)?;
    if !result.success {
        return None;
    }
    serde_json::from_str::<Vec<Value>>(&result.output).ok()
}

/// Parse the diagnostic output for `task_id` as a JSON object.
fn task_object(ctx: &DetectCtx, task_id: &str) -> Option<Value> {
    let result = ctx.results.get(task_id)?;
    if !result.success {
        return None;
    }
    serde_json::from_str::<Value>(&result.output).ok()
}

/// Read a JSON value as u64 whether it arrived as a number OR a numeric
/// string. WMI marshals CIM `uint64` properties (e.g. `Win32_LogicalDisk`'s
/// `Size`/`FreeSpace`) as `VT_BSTR`, so they reach us as JSON strings — a
/// plain `.as_u64()` silently returns `None` on real hardware.
fn json_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn detect_low_disk_space(ctx: &DetectCtx) -> Option<Detection> {
    for disk in task_array(ctx, "logical_disk")? {
        if let (Some(free_space), Some(size)) =
            (json_u64(&disk["FreeSpace"]), json_u64(&disk["Size"]))
            && size > 0
        {
            let free_percent = (free_space as f64 / size as f64) * 100.0;
            if free_percent < 10.0 {
                return Some(Detection::new(format!(
                    "The disk '{}' is running low on space ({:.2}% free).",
                    disk["Name"].as_str().unwrap_or("Unknown"),
                    free_percent
                )));
            }
        }
    }
    None
}

pub fn detect_disk_fragmentation(ctx: &DetectCtx) -> Option<Detection> {
    for disk in task_array(ctx, "disk_fragmentation")? {
        if let Some(fragmentation) = disk["fragmentation_percent"].as_u64()
            && fragmentation > 20
        {
            return Some(Detection::new(format!(
                "The disk '{}' has {}% fragmentation.",
                disk["drive"].as_str().unwrap_or("Unknown"),
                fragmentation
            )));
        }
    }
    None
}

pub fn detect_unsigned_drivers(ctx: &DetectCtx) -> Option<Detection> {
    let drivers = task_array(ctx, "drivers_list")?;
    let unsigned_count = drivers
        .iter()
        .filter(|d| d["IsSigned"].as_bool() == Some(false))
        .count();
    if unsigned_count > 0 {
        return Some(Detection::new(format!(
            "Found {} unsigned driver(s) that could cause instability.",
            unsigned_count
        )));
    }
    None
}

pub fn detect_event_log_errors(ctx: &DetectCtx) -> Option<Detection> {
    let events = task_array(ctx, "event_logs")?;
    if events.is_empty() {
        return None;
    }
    Some(Detection::new(format!(
        "Found {} error(s) in system event logs.",
        events.len()
    )))
}

pub fn detect_stopped_services(ctx: &DetectCtx) -> Option<Detection> {
    let services = task_array(ctx, "services")?;
    let stopped_count = services
        .iter()
        .filter(|s| {
            s["StartMode"].as_str() == Some("Auto")
                && s["State"].as_str().is_some_and(|state| state != "Running")
        })
        .count();
    if stopped_count > 0 {
        return Some(Detection::new(format!(
            "{} automatic service(s) are not running.",
            stopped_count
        )));
    }
    None
}

/// FIXED in the catalog refactor: the performance task emits
/// `cpu_performance.LoadPercentage` (native_diagnostics get_performance_data),
/// not `PercentProcessorTime` — the old detector never fired on real data.
pub fn detect_high_cpu_usage(ctx: &DetectCtx) -> Option<Detection> {
    let performance = task_object(ctx, "performance")?;
    let cpu_usage = performance["cpu_performance"]["LoadPercentage"].as_u64()?;
    if cpu_usage > 90 {
        return Some(Detection::new(format!("CPU usage is at {}%.", cpu_usage)));
    }
    None
}

/// FIXED in the catalog refactor: the performance task emits
/// `memory_performance.UsedPercent` directly (dwMemoryLoad), not
/// `TotalVisibleMemorySize`/`FreePhysicalMemory` — the old detector never
/// fired on real data.
pub fn detect_high_memory_usage(ctx: &DetectCtx) -> Option<Detection> {
    let performance = task_object(ctx, "performance")?;
    let used_percent = performance["memory_performance"]["UsedPercent"].as_u64()?;
    if used_percent > 90 {
        return Some(Detection::new(format!(
            "Memory usage is at {}%.",
            used_percent
        )));
    }
    None
}

pub fn detect_pending_windows_updates(ctx: &DetectCtx) -> Option<Detection> {
    let update_info = task_object(ctx, "windows_update")?;
    let updates = update_info["installed_updates"].as_array()?;

    let mut most_recent_update: Option<Timestamp> = None;
    for update in updates {
        if let Some(installed_on_str) = update["InstalledOn"].as_str()
            && let Some(ts) = parse_windows_date(installed_on_str)
            && most_recent_update.is_none_or(|t| ts > t)
        {
            most_recent_update = Some(ts);
        }
    }

    let last_update_date = most_recent_update?;
    let thirty_days = Duration::from_secs(30 * 24 * 60 * 60);
    if last_update_date.is_before(&ctx.now, thirty_days) {
        return Some(Detection::new(format!(
            "Last update was {}.",
            last_update_date
                .to_iso_string()
                .split('T')
                .next()
                .unwrap_or("unknown")
        )));
    }
    None
}

pub fn detect_firewall_disabled(ctx: &DetectCtx) -> Option<Detection> {
    let mut disabled_products = Vec::new();
    let mut enabled_seen = false;
    for firewall in task_array(ctx, "firewall_status")? {
        // Windows Security Center productState is a packed bitfield, NOT a
        // single enum value. The enabled/disabled state is the second byte
        // (bits 8-15): the 0x10 bit set => ON; 0x00/0x01 => OFF.
        if let Some(product_state) = firewall["productState"].as_u64() {
            let enabled_byte = (product_state >> 8) & 0xFF;
            let firewall_on = (enabled_byte & 0x10) != 0;
            if firewall_on {
                enabled_seen = true;
            } else {
                disabled_products.push(
                    firewall["displayName"]
                        .as_str()
                        .unwrap_or("Firewall")
                        .to_string(),
                );
            }
        }
    }
    if !enabled_seen && !disabled_products.is_empty() {
        return Some(Detection::new(format!(
            "{} disabled.",
            disabled_products.join(", ")
        )));
    }
    None
}

pub fn detect_temp_files(ctx: &DetectCtx) -> Option<Detection> {
    let count = ctx.temp_file_count?;
    if count > 100 {
        return Some(Detection::new(format!(
            "Found {} files in temp directory.",
            count
        )));
    }
    None
}

/// Replaces the old `dns_cache` check (which keyed on the literal string
/// "error" appearing anywhere in the adapter output — meaningless). Real
/// condition: an adapter that has a default gateway (i.e. is the active
/// route) but no DNS servers configured — the network is up but name
/// resolution will fail.
pub fn detect_dns_misconfigured(ctx: &DetectCtx) -> Option<Detection> {
    for adapter in task_array(ctx, "network_adapter")? {
        let has_gateway = adapter["DefaultIPGateway"]
            .as_array()
            .is_some_and(|g| !g.is_empty());
        if !has_gateway {
            continue;
        }
        let has_dns = adapter["DNSServerSearchOrder"]
            .as_array()
            .is_some_and(|d| !d.is_empty());
        if !has_dns {
            return Some(Detection::with_severity(
                format!(
                    "Adapter '{}' has a gateway but no DNS servers configured.",
                    adapter["Description"].as_str().unwrap_or("Unknown")
                ),
                IssueSeverity::Warning,
            ));
        }
    }
    None
}

// ============================================================================
// Disk & system health
// ============================================================================

pub fn detect_smart_failure_predicted(ctx: &DetectCtx) -> Option<Detection> {
    let health = task_object(ctx, "chkdsk")?;
    for disk in health["disks"].as_array()? {
        // MSFT_PhysicalDisk.OperationalStatus is a UInt16[] array (CIM), not a
        // scalar — WMI renders it as a JSON array. 5 = Predictive Failure (SMART).
        let predictive = match &disk["OperationalStatus"] {
            Value::Array(states) => states.iter().any(|s| json_u64(s) == Some(5)),
            other => json_u64(other) == Some(5),
        };
        if predictive {
            return Some(Detection::new(format!(
                "Disk '{}' is predicting imminent failure (SMART). Back up your data NOW.",
                disk["Model"].as_str().unwrap_or("Unknown")
            )));
        }
    }
    None
}

pub fn detect_disk_unhealthy(ctx: &DetectCtx) -> Option<Detection> {
    let health = task_object(ctx, "chkdsk")?;
    for disk in health["disks"].as_array()? {
        // MSFT_PhysicalDisk HealthStatus: 0 Healthy, 1 Warning, 2 Unhealthy
        match json_u64(&disk["HealthStatus"]) {
            Some(1) => {
                return Some(Detection::new(format!(
                    "Disk '{}' reports degraded health.",
                    disk["Model"].as_str().unwrap_or("Unknown")
                )));
            }
            Some(2) => {
                return Some(Detection::with_severity(
                    format!(
                        "Disk '{}' reports UNHEALTHY status. Back up your data immediately.",
                        disk["Model"].as_str().unwrap_or("Unknown")
                    ),
                    IssueSeverity::Critical,
                ));
            }
            _ => {}
        }

        let health_text = disk["HealthStatusText"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if health_text == "warning" {
            return Some(Detection::new(format!(
                "Disk '{}' reports degraded health.",
                disk["Model"].as_str().unwrap_or("Unknown")
            )));
        }
        if health_text == "unhealthy" {
            return Some(Detection::with_severity(
                format!(
                    "Disk '{}' reports UNHEALTHY status. Back up your data immediately.",
                    disk["Model"].as_str().unwrap_or("Unknown")
                ),
                IssueSeverity::Critical,
            ));
        }

        let operational_codes: Vec<u64> = match &disk["OperationalStatus"] {
            Value::Array(states) => states.iter().filter_map(json_u64).collect(),
            other => json_u64(other).into_iter().collect(),
        };
        if operational_codes.contains(&6) {
            return Some(Detection::with_severity(
                format!(
                    "Disk '{}' reports an operational error. Back up your data immediately.",
                    disk["Model"].as_str().unwrap_or("Unknown")
                ),
                IssueSeverity::Critical,
            ));
        }
        if operational_codes.iter().any(|code| matches!(*code, 3 | 4)) {
            return Some(Detection::new(format!(
                "Disk '{}' reports degraded operational status.",
                disk["Model"].as_str().unwrap_or("Unknown")
            )));
        }
    }
    if health["errors_found"].as_bool() == Some(true) {
        return Some(Detection::new(
            "Disk health check reported errors.".to_string(),
        ));
    }
    None
}

pub fn detect_dism_corruption(ctx: &DetectCtx) -> Option<Detection> {
    let dism = task_object(ctx, "dism_health")?;
    let status = dism["status"].as_str().unwrap_or("");
    if status.eq_ignore_ascii_case("corrupted") {
        return Some(Detection::with_severity(
            "The Windows component store is corrupted.".to_string(),
            IssueSeverity::Critical,
        ));
    }
    if status.eq_ignore_ascii_case("repairable") || dism["repairable"].as_bool() == Some(true) {
        return Some(Detection::new(
            "The Windows component store has repairable corruption.",
        ));
    }
    None
}

pub fn detect_bsod_recent(ctx: &DetectCtx) -> Option<Detection> {
    let minidump = task_object(ctx, "minidump")?;
    let dumps = minidump["dumps"].as_array()?;
    let thirty_days_ago = ctx.now.secs - 30 * 24 * 3600;
    let recent = dumps
        .iter()
        .filter(|d| {
            d["created"]
                .as_i64()
                .is_some_and(|created| created >= thirty_days_ago)
        })
        .count();
    if recent > 0 {
        return Some(Detection::new(format!(
            "{} blue-screen crash dump(s) from the last 30 days.",
            recent
        )));
    }
    None
}

// ============================================================================
// Critical event codes (source task: event_codes_critical)
// ============================================================================

/// Total count + most recent sample for a source (optionally a code subset)
/// from the aggregated event_codes_critical output.
fn event_code_count(ctx: &DetectCtx, source: &str, codes: &[u64]) -> Option<(u64, String)> {
    let data = task_object(ctx, "event_codes_critical")?;
    let mut total = 0u64;
    let mut sample = String::new();
    for event in data["events"].as_array()? {
        if event["source"].as_str() != Some(source) {
            continue;
        }
        let code = event["code"].as_u64().unwrap_or(0);
        if !codes.is_empty() && !codes.contains(&code) {
            continue;
        }
        total += event["count"].as_u64().unwrap_or(0);
        if sample.is_empty()
            && let Some(message) = event["sample_message"].as_str()
        {
            sample = message.to_string();
        }
    }
    if total > 0 {
        Some((total, sample))
    } else {
        None
    }
}

pub fn detect_kernel_power_crashes(ctx: &DetectCtx) -> Option<Detection> {
    let (count, _) = event_code_count(ctx, "Microsoft-Windows-Kernel-Power", &[41])?;
    Some(Detection::new(format!(
        "{} Kernel-Power 41 event(s) in the last 7 days - the system lost power or crashed without a clean shutdown.",
        count
    )))
}

pub fn detect_unexpected_shutdowns(ctx: &DetectCtx) -> Option<Detection> {
    let (count, _) = event_code_count(ctx, "EventLog", &[6008])?;
    Some(Detection::new(format!(
        "{} unexpected shutdown(s) (event 6008) in the last 7 days.",
        count
    )))
}

pub fn detect_disk_io_errors(ctx: &DetectCtx) -> Option<Detection> {
    let (count, _) = event_code_count(ctx, "disk", &[7, 51, 153])?;
    let severity = if count >= 5 {
        Some(IssueSeverity::Critical)
    } else {
        None
    };
    Some(Detection {
        severity,
        description: format!(
            "{} disk I/O error event(s) (codes 7/51/153) in the last 7 days. Check cables and back up the affected disk.",
            count
        ),
    })
}

pub fn detect_whea_errors(ctx: &DetectCtx) -> Option<Detection> {
    let (count, _) = event_code_count(ctx, "Microsoft-Windows-WHEA-Logger", &[])?;
    let severity = if count >= 5 {
        Some(IssueSeverity::Critical)
    } else {
        None
    };
    Some(Detection {
        severity,
        description: format!(
            "{} hardware error event(s) (WHEA) in the last 7 days - possible CPU, memory or bus fault.",
            count
        ),
    })
}

pub fn detect_service_crash_loops(ctx: &DetectCtx) -> Option<Detection> {
    let (count, sample) = event_code_count(ctx, "Service Control Manager", &[7031, 7034])?;
    if count < 3 {
        return None;
    }
    let mut description = format!(
        "{} service crash event(s) (7031/7034) in the last 7 days.",
        count
    );
    if !sample.is_empty() {
        let head: String = sample.chars().take(120).collect();
        description.push_str(&format!(" Latest: {}", head));
    }
    Some(Detection::new(description))
}

// ============================================================================
// Devices, security, system state
// ============================================================================

pub fn detect_device_manager_errors(ctx: &DetectCtx) -> Option<Detection> {
    let devices = task_array(ctx, "device_errors")?;
    if devices.is_empty() {
        return None;
    }
    // ConfigManagerErrorCode 22 = device disabled by the user - informational
    let all_disabled = devices
        .iter()
        .all(|d| d["ConfigManagerErrorCode"].as_u64() == Some(22));
    let names: Vec<&str> = devices
        .iter()
        .filter_map(|d| d["Name"].as_str())
        .take(3)
        .collect();
    let description = format!(
        "{} device(s) report a Device Manager problem code{}{}",
        devices.len(),
        if names.is_empty() { "" } else { ": " },
        names.join(", ")
    );
    if all_disabled {
        Some(Detection::with_severity(
            format!("{} (all are user-disabled devices).", description),
            IssueSeverity::Info,
        ))
    } else {
        Some(Detection::new(format!("{}.", description)))
    }
}

pub fn detect_defender_disabled(ctx: &DetectCtx) -> Option<Detection> {
    let products = task_array(ctx, "defender_status")?;
    if products.is_empty() {
        return None; // unknown, not "disabled"
    }
    // Any single enabled AV protects the machine; flag only when ALL are off.
    let any_enabled = products.iter().any(|p| {
        p["productState"]
            .as_u64()
            .is_some_and(|state| ((state >> 8) & 0x10) != 0)
    });
    if any_enabled {
        return None;
    }
    let names: Vec<&str> = products
        .iter()
        .filter_map(|p| p["displayName"].as_str())
        .take(3)
        .collect();
    Some(Detection::new(format!(
        "No active antivirus: {} report(s) disabled state.",
        if names.is_empty() {
            "installed product(s)".to_string()
        } else {
            names.join(", ")
        }
    )))
}

pub fn detect_pending_reboot(ctx: &DetectCtx) -> Option<Detection> {
    let reboot = task_object(ctx, "pending_reboot")?;
    if reboot["pending"].as_bool() != Some(true) {
        return None;
    }
    let reasons: Vec<&str> = reboot["reasons"]
        .as_array()
        .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    // PendingFileRenameOperations alone is a noisy marker (installers leave
    // it behind constantly) - informational only.
    let pfro_only = reasons == ["pending_file_rename"];
    let description = format!(
        "Windows is waiting for a restart ({}).",
        if reasons.is_empty() {
            "unspecified".to_string()
        } else {
            reasons.join(", ")
        }
    );
    if pfro_only {
        Some(Detection::with_severity(description, IssueSeverity::Info))
    } else {
        Some(Detection::new(description))
    }
}

pub fn detect_page_file_pressure(ctx: &DetectCtx) -> Option<Detection> {
    let performance = task_object(ctx, "performance")?;
    let total = performance["memory_performance"]["TotalPageFileMBytes"].as_u64()?;
    let available = performance["memory_performance"]["AvailablePageFileMBytes"].as_u64()?;
    if total == 0 {
        return None;
    }
    let available_percent = (available as f64 / total as f64) * 100.0;
    if available_percent < 10.0 {
        return Some(Detection::new(format!(
            "Page file nearly exhausted: {:.1}% of {} MB available.",
            available_percent, total
        )));
    }
    None
}

pub fn detect_battery_degraded(ctx: &DetectCtx) -> Option<Detection> {
    let report = task_object(ctx, "battery_report")?;
    let summary = &report["battery_summary"];
    let health = summary["battery_health_percentage"].as_f64();
    let status = summary["battery_health_status"].as_str();
    // Desktops have no battery fields - absent data is fine
    if health.is_none() && status.is_none() {
        return None;
    }
    let poor = status.is_some_and(|s| s.eq_ignore_ascii_case("poor"));
    if poor || health.is_some_and(|h| h < 60.0) {
        return Some(Detection::new(format!(
            "Battery holds {}% of its design capacity.",
            health
                .map(|h| format!("{:.0}", h))
                .unwrap_or_else(|| "under 60".to_string())
        )));
    }
    None
}

pub fn detect_startup_bloat(ctx: &DetectCtx) -> Option<Detection> {
    let entries = task_array(ctx, "startup_command")?;
    if entries.len() > 15 {
        return Some(Detection::new(format!(
            "{} programs launch at startup, which slows boot and idles resources.",
            entries.len()
        )));
    }
    None
}

pub fn detect_outdated_drivers(ctx: &DetectCtx) -> Option<Detection> {
    let drivers = task_array(ctx, "drivers_list")?;
    let five_years_ago = ctx.now.secs - 5 * 365 * 24 * 3600;
    let mut outdated: Vec<String> = Vec::new();
    for driver in &drivers {
        let class_matches = driver["DeviceClass"]
            .as_str()
            .is_some_and(|c| c.eq_ignore_ascii_case("DISPLAY") || c.eq_ignore_ascii_case("NET"));
        if !class_matches {
            continue;
        }
        if let Some(date) = driver["DriverDate"].as_str()
            && let Some(ts) = crate::timestamp::parse_wmi_datetime(date)
            && ts.secs < five_years_ago
        {
            outdated.push(
                driver["DeviceName"]
                    .as_str()
                    .unwrap_or("Unknown device")
                    .to_string(),
            );
        }
    }
    if outdated.is_empty() {
        return None;
    }
    outdated.truncate(3);
    Some(Detection::new(format!(
        "Display/network drivers older than 5 years: {}.",
        outdated.join(", ")
    )))
}

/// Domains whose hosts-file redirection is a classic malware/hijack sign.
const HIJACK_WATCHED_DOMAINS: &[&str] = &[
    "google.com",
    "bing.com",
    "microsoft.com",
    "windowsupdate.com",
    "live.com",
    "office.com",
    "facebook.com",
    "github.com",
    "mozilla.org",
    "avast.com",
    "bitdefender.com",
    "kaspersky.com",
    "malwarebytes.com",
];

pub fn detect_hosts_file_hijack(ctx: &DetectCtx) -> Option<Detection> {
    let hosts = task_object(ctx, "hosts_file")?;
    for entry in hosts["entries"].as_array()? {
        let (Some(ip), Some(hostname)) = (entry["ip"].as_str(), entry["hostname"].as_str()) else {
            continue;
        };
        let loopback = matches!(ip, "127.0.0.1" | "::1" | "0.0.0.0");
        if loopback {
            continue;
        }
        let hostname_lower = hostname.to_ascii_lowercase();
        let watched = HIJACK_WATCHED_DOMAINS.iter().any(|domain| {
            hostname_lower == *domain || hostname_lower.ends_with(&format!(".{}", domain))
        });
        if watched {
            return Some(Detection::new(format!(
                "hosts file redirects '{}' to {} - a common malware hijack. Review the hosts file before trusting any login page.",
                hostname, ip
            )));
        }
    }
    None
}

/// Parse a Windows date format (MM/DD/YYYY) to a Timestamp
pub(crate) fn parse_windows_date(date_str: &str) -> Option<Timestamp> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::TaskResult;
    use crate::issue_catalog::test_support::ctx;
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
        // WMI marshals uint64 Size/FreeSpace as STRINGS — the real shape, and
        // the one the original numeric-literal test failed to exercise.
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": "5000000000", "Size": "100000000000"}]"#,
        );
        let detection = detect_low_disk_space(&ctx(&results)).expect("should detect");
        assert!(detection.description.contains("C:"));
    }

    #[test]
    fn low_disk_space_accepts_numeric_json_too() {
        // Defensive: a provider that returns real JSON numbers must still work.
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": 5000000000, "Size": 100000000000}]"#,
        );
        assert!(detect_low_disk_space(&ctx(&results)).is_some());
    }

    #[test]
    fn adequate_disk_space_not_flagged() {
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": 50000000000, "Size": 100000000000}]"#,
        );
        assert!(detect_low_disk_space(&ctx(&results)).is_none());
    }

    #[test]
    fn malformed_disk_output_does_not_false_positive() {
        let results = results_with("logical_disk", "not json at all");
        assert!(detect_low_disk_space(&ctx(&results)).is_none());
        // Zero-size disk must not divide by zero or flag
        let results = results_with(
            "logical_disk",
            r#"[{"Name": "X:", "FreeSpace": 0, "Size": 0}]"#,
        );
        assert!(detect_low_disk_space(&ctx(&results)).is_none());
    }

    #[test]
    fn stopped_auto_service_detected() {
        let results = results_with(
            "services",
            r#"[{"Name": "wuauserv", "State": "Stopped", "StartMode": "Auto"},
                {"Name": "Spooler", "State": "Running", "StartMode": "Auto"}]"#,
        );
        let detection = detect_stopped_services(&ctx(&results)).expect("should detect");
        assert!(detection.description.contains('1'));
    }

    #[test]
    fn manual_stopped_service_not_flagged() {
        let results = results_with(
            "services",
            r#"[{"Name": "Fax", "State": "Stopped", "StartMode": "Manual"}]"#,
        );
        assert!(detect_stopped_services(&ctx(&results)).is_none());
    }

    #[test]
    fn high_memory_usage_uses_the_real_used_percent_field() {
        // Regression: the old detector read TotalVisibleMemorySize /
        // FreePhysicalMemory, which the performance task never emits — it was
        // dead on real data. The REAL shape carries UsedPercent directly.
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalMBytes": 16384, "AvailableMBytes": 819, "UsedPercent": 95}}"#,
        );
        assert!(detect_high_memory_usage(&ctx(&results)).is_some());
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalMBytes": 16384, "AvailableMBytes": 8192, "UsedPercent": 50}}"#,
        );
        assert!(detect_high_memory_usage(&ctx(&results)).is_none());
        // The OLD (wrong) shape must no longer trigger anything
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalVisibleMemorySize": 100, "FreePhysicalMemory": 5}}"#,
        );
        assert!(detect_high_memory_usage(&ctx(&results)).is_none());
    }

    #[test]
    fn high_cpu_usage_uses_the_real_load_percentage_field() {
        // Regression: old detector read PercentProcessorTime (never emitted);
        // the real performance output carries cpu_performance.LoadPercentage.
        let results = results_with(
            "performance",
            r#"{"cpu_performance": {"LoadPercentage": 97, "NumberOfCores": 8}}"#,
        );
        assert!(detect_high_cpu_usage(&ctx(&results)).is_some());
        let results = results_with(
            "performance",
            r#"{"cpu_performance": {"LoadPercentage": 40, "NumberOfCores": 8}}"#,
        );
        assert!(detect_high_cpu_usage(&ctx(&results)).is_none());
        // Old wrong field name no longer fires
        let results = results_with(
            "performance",
            r#"{"cpu_performance": {"PercentProcessorTime": 97}}"#,
        );
        assert!(detect_high_cpu_usage(&ctx(&results)).is_none());
    }

    #[test]
    fn firewall_product_state_bitfield() {
        // Second byte 0x10 bit set => firewall ON (e.g. 0x1000 = 4096)
        let results = results_with(
            "firewall_status",
            r#"[{"displayName": "Windows Firewall", "productState": 4096}]"#,
        );
        assert!(detect_firewall_disabled(&ctx(&results)).is_none());
        // 0x0100: second byte 0x01 => OFF
        let results = results_with(
            "firewall_status",
            r#"[{"displayName": "Windows Firewall", "productState": 256}]"#,
        );
        assert!(detect_firewall_disabled(&ctx(&results)).is_some());
        // If Security Center lists an old disabled product next to an enabled
        // active firewall, do not false-alarm.
        let results = results_with(
            "firewall_status",
            r#"[
                {"displayName": "Old Firewall", "productState": 256},
                {"displayName": "Windows Firewall", "productState": 4096}
            ]"#,
        );
        assert!(detect_firewall_disabled(&ctx(&results)).is_none());
    }

    #[test]
    fn temp_files_uses_injected_count() {
        let results = HashMap::new();
        let mut c = ctx(&results);
        c.temp_file_count = Some(500);
        assert!(detect_temp_files(&c).is_some());
        c.temp_file_count = Some(10);
        assert!(detect_temp_files(&c).is_none());
        c.temp_file_count = None; // unknown => never detected
        assert!(detect_temp_files(&c).is_none());
    }

    #[test]
    fn dns_misconfigured_requires_gateway_without_dns() {
        // Gateway + no DNS => detected
        let results = results_with(
            "network_adapter",
            r#"[{"Description": "Ethernet", "DefaultIPGateway": ["192.168.1.1"], "DNSServerSearchOrder": null}]"#,
        );
        assert!(detect_dns_misconfigured(&ctx(&results)).is_some());
        // Gateway + DNS => fine
        let results = results_with(
            "network_adapter",
            r#"[{"Description": "Ethernet", "DefaultIPGateway": ["192.168.1.1"], "DNSServerSearchOrder": ["1.1.1.1"]}]"#,
        );
        assert!(detect_dns_misconfigured(&ctx(&results)).is_none());
        // No gateway (inactive adapter) + no DNS => not our problem
        let results = results_with(
            "network_adapter",
            r#"[{"Description": "Bluetooth", "DefaultIPGateway": [], "DNSServerSearchOrder": null}]"#,
        );
        assert!(detect_dns_misconfigured(&ctx(&results)).is_none());
        // The old "output contains 'error'" nonsense must NOT trigger
        let results = results_with(
            "network_adapter",
            r#"[{"Description": "error in name but adapter is fine", "DefaultIPGateway": ["10.0.0.1"], "DNSServerSearchOrder": ["8.8.8.8"]}]"#,
        );
        assert!(detect_dns_misconfigured(&ctx(&results)).is_none());
    }

    #[test]
    fn pending_updates_uses_injected_clock() {
        // ctx.now is fixed at 2026-06-12; an update from 2026-06-01 is fresh
        let results = results_with(
            "windows_update",
            r#"{"installed_updates": [{"HotFixID": "KB1", "InstalledOn": "6/1/2026"}]}"#,
        );
        assert!(detect_pending_windows_updates(&ctx(&results)).is_none());
        // An update from January is >30 days before the fixed clock
        let results = results_with(
            "windows_update",
            r#"{"installed_updates": [{"HotFixID": "KB1", "InstalledOn": "1/10/2026"}]}"#,
        );
        assert!(detect_pending_windows_updates(&ctx(&results)).is_some());
    }

    // ---------- new detectors (catalog v2) ----------

    #[test]
    fn smart_predictive_failure_and_unhealthy_disks() {
        // MSFT_PhysicalDisk.OperationalStatus is a UInt16[] ARRAY — the real
        // shape. A scalar would never have matched on hardware.
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD A", "OperationalStatus": [5], "HealthStatus": 0}]}"#,
        );
        assert!(detect_smart_failure_predicted(&ctx(&results)).is_some());
        // Multi-element array containing the predictive-failure code still fires
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD A2", "OperationalStatus": [2, 5], "HealthStatus": 0}]}"#,
        );
        assert!(detect_smart_failure_predicted(&ctx(&results)).is_some());
        // HealthStatus 2 => Critical override
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "HDD B", "OperationalStatus": [2], "HealthStatus": 2}]}"#,
        );
        let d = detect_disk_unhealthy(&ctx(&results)).expect("unhealthy");
        assert_eq!(d.severity, Some(IssueSeverity::Critical));
        // HealthStatus 1 => default (Warning)
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "HDD C", "OperationalStatus": [2], "HealthStatus": 1}]}"#,
        );
        assert!(
            detect_disk_unhealthy(&ctx(&results))
                .unwrap()
                .severity
                .is_none()
        );
        // Healthy => nothing
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "NVMe D", "OperationalStatus": [2], "HealthStatus": 0}]}"#,
        );
        assert!(detect_smart_failure_predicted(&ctx(&results)).is_none());
        assert!(detect_disk_unhealthy(&ctx(&results)).is_none());
        // OperationalStatus 3/4/6 are degraded even when HealthStatus remains
        // Healthy on some Storage Management providers.
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD E", "OperationalStatus": [3], "HealthStatus": 0}]}"#,
        );
        assert!(detect_disk_unhealthy(&ctx(&results)).is_some());
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD F", "OperationalStatus": [6], "HealthStatus": 0}]}"#,
        );
        let d = detect_disk_unhealthy(&ctx(&results)).expect("operational error");
        assert_eq!(d.severity, Some(IssueSeverity::Critical));
    }

    #[test]
    fn dism_corruption_levels() {
        let results = results_with(
            "dism_health",
            r#"{"status": "Corrupted", "repairable": false}"#,
        );
        let d = detect_dism_corruption(&ctx(&results)).unwrap();
        assert_eq!(d.severity, Some(IssueSeverity::Critical));
        let results = results_with(
            "dism_health",
            r#"{"status": "Repairable", "repairable": true}"#,
        );
        assert!(
            detect_dism_corruption(&ctx(&results))
                .unwrap()
                .severity
                .is_none()
        );
        let results = results_with(
            "dism_health",
            r#"{"status": "Healthy", "repairable": false}"#,
        );
        assert!(detect_dism_corruption(&ctx(&results)).is_none());
    }

    #[test]
    fn bsod_recent_30_day_boundary() {
        // fixed_now is 2026-06-12T12:00:00Z = 1781265600
        let now = ctx(&HashMap::new()).now.secs;
        let recent = now - 29 * 24 * 3600;
        let old = now - 31 * 24 * 3600;
        let results = results_with(
            "minidump",
            &format!(
                r#"{{"dumps": [{{"filename": "a.dmp", "created": {}}}], "count": 1}}"#,
                recent
            ),
        );
        assert!(detect_bsod_recent(&ctx(&results)).is_some());
        let results = results_with(
            "minidump",
            &format!(
                r#"{{"dumps": [{{"filename": "b.dmp", "created": {}}}], "count": 1}}"#,
                old
            ),
        );
        assert!(detect_bsod_recent(&ctx(&results)).is_none());
    }

    #[test]
    fn event_code_detectors_with_thresholds() {
        let output = r#"{"window_days": 7, "events": [
            {"source": "Microsoft-Windows-Kernel-Power", "code": 41, "count": 2, "sample_message": "rebooted without cleanly shutting down"},
            {"source": "disk", "code": 153, "count": 6, "sample_message": "IO retried"},
            {"source": "Service Control Manager", "code": 7031, "count": 2, "sample_message": "Spooler crashed"},
            {"source": "Service Control Manager", "code": 7034, "count": 2, "sample_message": "Audio crashed"}
        ]}"#;
        let results = results_with("event_codes_critical", output);
        let c = ctx(&results);
        assert!(detect_kernel_power_crashes(&c).is_some());
        assert!(detect_unexpected_shutdowns(&c).is_none()); // no 6008 rows
        // disk count 6 >= 5 => Critical override
        assert_eq!(
            detect_disk_io_errors(&c).unwrap().severity,
            Some(IssueSeverity::Critical)
        );
        assert!(detect_whea_errors(&c).is_none());
        // SCM 2 + 2 = 4 >= 3 => detected with sample
        let scm = detect_service_crash_loops(&c).unwrap();
        assert!(scm.description.contains("4 service crash"));
        // Below the loop threshold: 2 total => not flagged
        let output = r#"{"window_days": 7, "events": [
            {"source": "Service Control Manager", "code": 7031, "count": 2, "sample_message": "x"}
        ]}"#;
        let results = results_with("event_codes_critical", output);
        assert!(detect_service_crash_loops(&ctx(&results)).is_none());
    }

    #[test]
    fn device_errors_code_22_is_informational() {
        let results = results_with(
            "device_errors",
            r#"[{"Name": "Old NIC", "ConfigManagerErrorCode": 22}]"#,
        );
        let d = detect_device_manager_errors(&ctx(&results)).unwrap();
        assert_eq!(d.severity, Some(IssueSeverity::Info));
        // Any non-22 code => default Warning severity
        let results = results_with(
            "device_errors",
            r#"[{"Name": "GPU", "ConfigManagerErrorCode": 43}, {"Name": "Old NIC", "ConfigManagerErrorCode": 22}]"#,
        );
        let d = detect_device_manager_errors(&ctx(&results)).unwrap();
        assert!(d.severity.is_none());
        assert!(d.description.contains("GPU"));
        // Empty array (task ran, no problem devices) => fine
        let results = results_with("device_errors", "[]");
        assert!(detect_device_manager_errors(&ctx(&results)).is_none());
    }

    #[test]
    fn defender_disabled_only_when_all_products_off() {
        // One enabled (0x10 bit in second byte), one disabled => protected
        let results = results_with(
            "defender_status",
            r#"[{"displayName": "Windows Defender", "productState": 397568},
                {"displayName": "ThirdParty AV", "productState": 262144}]"#,
        );
        assert!(detect_defender_disabled(&ctx(&results)).is_none());
        // All disabled => Critical detection
        let results = results_with(
            "defender_status",
            r#"[{"displayName": "Windows Defender", "productState": 262144}]"#,
        );
        assert!(detect_defender_disabled(&ctx(&results)).is_some());
        // Empty / missing namespace => unknown, never "disabled"
        let results = results_with("defender_status", "[]");
        assert!(detect_defender_disabled(&ctx(&results)).is_none());
    }

    #[test]
    fn pending_reboot_pfro_only_is_info() {
        let results = results_with(
            "pending_reboot",
            r#"{"pending": true, "reasons": ["pending_file_rename"]}"#,
        );
        let d = detect_pending_reboot(&ctx(&results)).unwrap();
        assert_eq!(d.severity, Some(IssueSeverity::Info));
        // CBS/WU reasons keep the default Warning severity
        let results = results_with(
            "pending_reboot",
            r#"{"pending": true, "reasons": ["windows_update", "pending_file_rename"]}"#,
        );
        assert!(
            detect_pending_reboot(&ctx(&results))
                .unwrap()
                .severity
                .is_none()
        );
        let results = results_with("pending_reboot", r#"{"pending": false, "reasons": []}"#);
        assert!(detect_pending_reboot(&ctx(&results)).is_none());
    }

    #[test]
    fn page_file_pressure_guards_zero_total() {
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalPageFileMBytes": 16384, "AvailablePageFileMBytes": 800}}"#,
        );
        assert!(detect_page_file_pressure(&ctx(&results)).is_some());
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalPageFileMBytes": 16384, "AvailablePageFileMBytes": 8000}}"#,
        );
        assert!(detect_page_file_pressure(&ctx(&results)).is_none());
        let results = results_with(
            "performance",
            r#"{"memory_performance": {"TotalPageFileMBytes": 0, "AvailablePageFileMBytes": 0}}"#,
        );
        assert!(detect_page_file_pressure(&ctx(&results)).is_none());
    }

    #[test]
    fn battery_degraded_thresholds_and_desktops() {
        let results = results_with(
            "battery_report",
            r#"{"battery_summary": {"battery_health_percentage": 48.5, "battery_health_status": "Poor"}}"#,
        );
        assert!(detect_battery_degraded(&ctx(&results)).is_some());
        let results = results_with(
            "battery_report",
            r#"{"battery_summary": {"battery_health_percentage": 88.0, "battery_health_status": "Good"}}"#,
        );
        assert!(detect_battery_degraded(&ctx(&results)).is_none());
        // Desktop: no battery fields at all => fine
        let results = results_with("battery_report", r#"{"battery_summary": {}}"#);
        assert!(detect_battery_degraded(&ctx(&results)).is_none());
    }

    #[test]
    fn startup_bloat_threshold() {
        let many: Vec<String> = (0..20)
            .map(|i| format!(r#"{{"Name": "app{}"}}"#, i))
            .collect();
        let results = results_with("startup_command", &format!("[{}]", many.join(",")));
        assert!(detect_startup_bloat(&ctx(&results)).is_some());
        let few: Vec<String> = (0..5)
            .map(|i| format!(r#"{{"Name": "app{}"}}"#, i))
            .collect();
        let results = results_with("startup_command", &format!("[{}]", few.join(",")));
        assert!(detect_startup_bloat(&ctx(&results)).is_none());
    }

    #[test]
    fn outdated_drivers_only_display_and_net() {
        // fixed_now 2026: a 2018 display driver is >5y old
        let results = results_with(
            "drivers_list",
            r#"[{"DeviceName": "Old GPU", "DeviceClass": "DISPLAY", "DriverDate": "20180101000000.000000+***"},
                {"DeviceName": "Old Printer", "DeviceClass": "PRINTER", "DriverDate": "20100101000000.000000+***"}]"#,
        );
        let d = detect_outdated_drivers(&ctx(&results)).unwrap();
        assert!(d.description.contains("Old GPU"));
        assert!(!d.description.contains("Printer")); // non-watched class ignored
        // Recent display driver => fine
        let results = results_with(
            "drivers_list",
            r#"[{"DeviceName": "New GPU", "DeviceClass": "DISPLAY", "DriverDate": "20250101000000.000000+***"}]"#,
        );
        assert!(detect_outdated_drivers(&ctx(&results)).is_none());
    }

    #[test]
    fn hosts_hijack_loopback_is_fine() {
        // Ad-blocking style 0.0.0.0 entries must not flag
        let results = results_with(
            "hosts_file",
            r#"{"entries": [{"ip": "0.0.0.0", "hostname": "ads.example.com"},
                            {"ip": "127.0.0.1", "hostname": "telemetry.microsoft.com"}]}"#,
        );
        assert!(detect_hosts_file_hijack(&ctx(&results)).is_none());
        // A watched domain pointed at a real IP is the hijack signature
        let results = results_with(
            "hosts_file",
            r#"{"entries": [{"ip": "203.0.113.7", "hostname": "www.google.com"}]}"#,
        );
        let d = detect_hosts_file_hijack(&ctx(&results)).unwrap();
        assert!(d.description.contains("www.google.com"));
        // Unwatched domains to real IPs are normal intranet usage
        let results = results_with(
            "hosts_file",
            r#"{"entries": [{"ip": "10.0.0.5", "hostname": "nas.local"}]}"#,
        );
        assert!(detect_hosts_file_hijack(&ctx(&results)).is_none());
    }

    #[test]
    fn parse_windows_date_formats() {
        assert!(parse_windows_date("6/12/2026").is_some());
        assert!(parse_windows_date("12/31/2025").is_some());
        assert!(parse_windows_date("2026-06-12").is_none());
        assert!(parse_windows_date("garbage").is_none());
        assert!(parse_windows_date("6/12").is_none());
    }
}
