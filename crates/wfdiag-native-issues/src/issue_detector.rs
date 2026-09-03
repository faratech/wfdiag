//! Pure detection functions for the issue catalog.
//!
//! Each function takes a [`DetectCtx`] and returns `Some(Detection)` when the
//! issue is present. Metadata (ids, titles, recommendations, remediation
//! mapping) lives in [`crate::issue_catalog`] — keep these functions about
//! parsing diagnostic output and thresholds only. Everything here must be
//! deterministic: time comes from `ctx.now`, environment state (temp-file
//! count) is injected by the caller.

use crate::evidence::size::format_bytes;
use crate::issue_catalog::{DetectCtx, Detection, IssueSeverity};
use serde_json::Value;

/// Parse the diagnostic output for `task_id` as a JSON array, if the task
/// succeeded. The common shape for WMI-backed tasks.
fn task_array(ctx: &DetectCtx, task_id: &str) -> Option<Vec<Value>> {
    let result = ctx.results.get_task_result(task_id)?;
    if !result.success {
        return None;
    }
    serde_json::from_str::<Vec<Value>>(&result.output).ok()
}

/// Parse the diagnostic output for `task_id` as a JSON object.
fn task_object(ctx: &DetectCtx, task_id: &str) -> Option<Value> {
    let result = ctx.results.get_task_result(task_id)?;
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
                let mut description = format!(
                    "The disk '{}' is running low on space ({:.2}% free).",
                    disk["Name"].as_str().unwrap_or("Unknown"),
                    free_percent
                );
                // `disk_usage` is not a listed source (old scans lack it), so
                // its breakdown only enriches the wording when present.
                if let Some(largest) = largest_reclaimable_consumers(ctx, 3) {
                    description.push_str(" Largest items: ");
                    description.push_str(&largest);
                    description.push('.');
                }
                let detection = Detection::new(description);
                // The fix that frees the most: the largest consumer's own
                // remediation when the breakdown ran, else the default.
                return Some(match top_space_remediation(ctx) {
                    Some(remediation) => detection.with_remediation(remediation),
                    None => detection,
                });
            }
        }
    }
    None
}

/// The actionable consumers from `disk_usage`, largest first, that are at
/// least 1 GiB or 2 % of the drive.
fn actionable_space_consumers(ctx: &DetectCtx) -> Option<Vec<(String, u64, String)>> {
    const MIN_BYTES: u64 = 1024 * 1024 * 1024;
    let usage = task_object(ctx, "disk_usage")?;
    let total = json_u64(&usage["total_bytes"]).unwrap_or(0);
    let threshold = MIN_BYTES.min((total / 100).saturating_mul(2).max(1));
    let mut consumers: Vec<(String, u64, String)> = usage["consumers"]
        .as_array()?
        .iter()
        .filter_map(|consumer| {
            let bytes = json_u64(&consumer["bytes"])?;
            let remediation = consumer["remediation"].as_str()?;
            let label = consumer["label"]
                .as_str()
                .or_else(|| consumer["id"].as_str())?;
            (bytes >= threshold).then(|| (label.to_string(), bytes, remediation.to_string()))
        })
        .collect();
    consumers.sort_by(|a, b| b.1.cmp(&a.1));
    Some(consumers)
}

/// The vetted remediation of the largest actionable consumer, when `disk_usage`
/// ran and its id is one the rules list.
fn top_space_remediation(ctx: &DetectCtx) -> Option<&'static str> {
    let consumers = actionable_space_consumers(ctx)?;
    let (_, _, remediation) = consumers.first()?;
    ALTERNATE_SPACE_REMEDIATIONS
        .iter()
        .copied()
        .find(|known| known == remediation)
}

fn largest_reclaimable_consumers(ctx: &DetectCtx, limit: usize) -> Option<String> {
    let consumers = actionable_space_consumers(ctx)?;
    if consumers.is_empty() {
        return None;
    }
    Some(
        consumers
            .iter()
            .take(limit)
            .map(|(label, bytes, _)| format!("{label} {}", format_bytes(*bytes)))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Info-level: the largest reclaimable items, with the top one's remediation.
pub fn detect_space_consumers(ctx: &DetectCtx) -> Option<Detection> {
    let consumers = actionable_space_consumers(ctx)?;
    let (_, _, top_remediation) = consumers.first()?;
    let listed = consumers
        .iter()
        .take(3)
        .map(|(label, bytes, _)| format!("{label} {}", format_bytes(*bytes)))
        .collect::<Vec<_>>()
        .join(", ");
    let remediation: &'static str = ALTERNATE_SPACE_REMEDIATIONS
        .iter()
        .copied()
        .find(|known| *known == top_remediation)?;
    Some(
        Detection::new(format!("Largest reclaimable items: {listed}."))
            .with_remediation(remediation),
    )
}

/// The remediations `space_consumers` may select; mirrors its spec.
const ALTERNATE_SPACE_REMEDIATIONS: [&str; 7] = [
    "open_storage_settings",
    "open_downloads_folder",
    "clear_temp_files",
    "clear_windows_temp",
    "empty_recycle_bin",
    "windows_update_reset",
    "open_disk_cleanup",
];

pub fn detect_disk_fragmentation(ctx: &DetectCtx) -> Option<Detection> {
    for disk in task_array(ctx, "disk_fragmentation")? {
        if let Some(fragmentation) = json_u64(&disk["fragmentation_percent"])
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
            s["Name"].as_str().is_some_and(is_supported_core_service)
                && s["StartMode"].as_str() == Some("Auto")
                && s["State"].as_str().is_some_and(|state| state != "Running")
        })
        .count();
    if stopped_count > 0 {
        return Some(Detection::new(format!(
            "{} supported core automatic service(s) are not running.",
            stopped_count
        )));
    }
    None
}

fn is_supported_core_service(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "wuauserv" | "bits" | "spooler" | "themes" | "audiosrv"
    )
}

/// FIXED in the catalog refactor: the performance task emits
/// `cpu_performance.LoadPercentage` (native_diagnostics get_performance_data),
/// not `PercentProcessorTime` — the old detector never fired on real data.
pub fn detect_high_cpu_usage(ctx: &DetectCtx) -> Option<Detection> {
    let performance = task_object(ctx, "performance")?;
    let cpu_usage = json_u64(&performance["cpu_performance"]["LoadPercentage"])?;
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
    let used_percent = json_u64(&performance["memory_performance"]["UsedPercent"])?;
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
    // Installed-update history cannot prove that updates are pending. Only
    // flag an issue if a source explicitly reports pending updates; the
    // current Win32_QuickFixEngineering source intentionally produces no
    // detection and the UI directs users to Windows Update for live status.
    let pending_count = update_info["pending_update_count"]
        .as_u64()
        .or_else(|| {
            update_info["pending_updates"]
                .as_array()
                .map(|updates| updates.len() as u64)
        })
        .unwrap_or(0);
    if pending_count > 0 {
        return Some(Detection::new(format!(
            "Windows reports {} pending update(s).",
            pending_count
        )));
    }
    None
}

/// Fires when an update failed in the window and nothing installed after it;
/// the newest failure's error family picks the remediation.
pub fn detect_windows_update_failing(ctx: &DetectCtx) -> Option<Detection> {
    use crate::evidence::windows_update::{decode_hresult, format_code, parse_error_code};

    let events = task_object(ctx, "windows_update_events")?;
    let failures = events["failures"].as_array()?;
    if failures.is_empty() || json_u64(&events["successes_after_last_failure"])? > 0 {
        return None;
    }
    let newest = failures
        .iter()
        .max_by_key(|failure| failure["time_secs"].as_i64().unwrap_or(i64::MIN))?;
    let raw_code = newest["error_code"].as_str().unwrap_or_default();
    let decoded = parse_error_code(raw_code).map(decode_hresult);
    let title = newest["update_title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("an update");
    let window_days = json_u64(&events["window_days"]).unwrap_or(30);
    let description = match decoded {
        Some(info) => format!(
            "Windows Update failed to install {title} with {} ({}): {} Cause: {}. {} failure(s) in the last {window_days} days and nothing installed since.",
            format_code(info.code),
            info.name,
            info.plain,
            info.family.label(),
            failures.len()
        ),
        None => format!(
            "Windows Update failed to install {title} ({}). {} failure(s) in the last {window_days} days and nothing installed since.",
            if raw_code.is_empty() {
                "no error code"
            } else {
                raw_code
            },
            failures.len()
        ),
    };
    let detection = Detection::new(description);
    Some(match decoded {
        Some(info) => detection.with_remediation(info.family.remediation()),
        None => detection,
    })
}

pub fn detect_windows_update_service_disabled(ctx: &DetectCtx) -> Option<Detection> {
    let services = task_array(ctx, "services")?;
    let disabled = services.iter().any(|service| {
        service["Name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("wuauserv"))
            && service["StartMode"]
                .as_str()
                .is_some_and(|mode| mode.eq_ignore_ascii_case("Disabled"))
    });
    disabled.then(|| {
        Detection::new(
            "The Windows Update service (wuauserv) is set to Disabled, so Windows cannot download or install updates.",
        )
    })
}

pub fn detect_firewall_disabled(ctx: &DetectCtx) -> Option<Detection> {
    let mut disabled_products = Vec::new();
    let mut enabled_seen = false;
    for firewall in task_array(ctx, "firewall_status")? {
        // Windows Security Center productState is a packed bitfield, NOT a
        // single enum value. The enabled/disabled state is the second byte
        // (bits 8-15): the 0x10 bit set => ON; 0x00/0x01 => OFF.
        if let Some(product_state) = json_u64(&firewall["productState"]) {
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

fn network_verdict(ctx: &DetectCtx) -> Option<(String, Value)> {
    let report = task_object(ctx, "network_path")?;
    let verdict = report["verdict"].as_str()?.to_string();
    Some((verdict, report))
}

pub fn detect_no_internet(ctx: &DetectCtx) -> Option<Detection> {
    let (verdict, report) = network_verdict(ctx)?;
    match verdict.as_str() {
        "no_internet" => Some(Detection::new(
            "No network adapter is connected with a default gateway, so this PC has no route to the internet.",
        )),
        "wan_down" => Some(Detection::new(format!(
            "The router ({}) answers, but nothing beyond it does: the internet connection itself is down.",
            report["probes"]["gateway"]
                .as_str()
                .unwrap_or("default gateway")
        ))),
        _ => None,
    }
}

pub fn detect_gateway_unreachable(ctx: &DetectCtx) -> Option<Detection> {
    let (verdict, report) = network_verdict(ctx)?;
    (verdict == "gateway_unreachable").then(|| {
        Detection::new(format!(
            "The router ({}) did not answer and nothing beyond it could be reached.",
            report["probes"]["gateway"]
                .as_str()
                .unwrap_or("default gateway")
        ))
    })
}

pub fn detect_dns_resolution_failing(ctx: &DetectCtx) -> Option<Detection> {
    let (verdict, report) = network_verdict(ctx)?;
    (verdict == "dns_resolution_failing").then(|| {
        Detection::new(format!(
            "The internet is reachable but {} did not resolve: DNS is misconfigured or the resolver is down.",
            report["dns_probe_host"].as_str().unwrap_or("a well-known name")
        ))
    })
}

pub fn detect_smart_failure_predicted(ctx: &DetectCtx) -> Option<Detection> {
    let health = task_object(ctx, "chkdsk")?;
    for disk in health["disks"].as_array()? {
        // MSFT_PhysicalDisk.OperationalStatus is a UInt16[] array (CIM), not a
        // scalar — WMI renders it as a JSON array. 5 = Predictive Failure (SMART).
        let predictive = match &disk["OperationalStatus"] {
            Value::Array(states) => states.iter().any(|s| json_u64(s) == Some(5)),
            other => json_u64(other) == Some(5),
        } || disk["OperationalStatusText"]
            .as_str()
            .is_some_and(|status| status.trim().eq_ignore_ascii_case("predictive failure"));
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

        let operational_text = disk["OperationalStatusText"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if operational_text == "error" {
            return Some(Detection::with_severity(
                format!(
                    "Disk '{}' reports an operational error. Back up your data immediately.",
                    disk["Model"].as_str().unwrap_or("Unknown")
                ),
                IssueSeverity::Critical,
            ));
        }
        if matches!(operational_text.as_str(), "degraded" | "stressed") {
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

/// The remediations `bsod_recent` may select; mirrors its spec.
const BSOD_REMEDIATIONS: [&str; 4] = [
    "open_device_manager",
    "open_memory_diagnostic",
    "schedule_memory_diagnostic",
    "sfc_scannow",
];

pub fn detect_bsod_recent(ctx: &DetectCtx) -> Option<Detection> {
    let minidump = task_object(ctx, "minidump")?;
    let dumps = minidump["dumps"].as_array()?;
    let thirty_days_ago = ctx.now.secs - 30 * 24 * 3600;
    let created_secs = |dump: &Value| {
        dump["created"]
            .as_i64()
            .or_else(|| json_u64(&dump["created"]).and_then(|created| i64::try_from(created).ok()))
    };
    let recent: Vec<&Value> = dumps
        .iter()
        .filter(|dump| created_secs(dump).is_some_and(|created| created >= thirty_days_ago))
        .collect();
    if recent.is_empty() {
        return None;
    }
    let count = recent.len();
    let newest = recent
        .iter()
        .copied()
        .max_by_key(|dump| created_secs(dump).unwrap_or(i64::MIN))?;
    let bugcheck = &newest["bugcheck"];
    let Some(name) = bugcheck["name"].as_str() else {
        return Some(Detection::new(format!(
            "{count} blue-screen crash dump(s) from the last 30 days."
        )));
    };
    let code = bugcheck["code"].as_str().unwrap_or("?");
    let module = bugcheck["faulting_module"]
        .as_str()
        .map(|module| format!(" in {module}"))
        .unwrap_or_default();
    let cause_label = bugcheck["cause_label"]
        .as_str()
        .unwrap_or("cause not recognised");
    let plain = bugcheck["plain"].as_str().unwrap_or_default();
    let next_action = bugcheck["next_action"].as_str().unwrap_or_default();
    let description = format!(
        "Latest crash: {name} ({code}){module} — {cause_label}. {plain} {next_action} {count} crash dump(s) in the last 30 days."
    );
    let detection = Detection::new(description);
    let remediation = bugcheck["remediation"].as_str().and_then(|chosen| {
        BSOD_REMEDIATIONS
            .iter()
            .copied()
            .find(|known| *known == chosen)
    });
    Some(match remediation {
        Some(remediation) => detection.with_remediation(remediation),
        None => detection,
    })
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
        let code = json_u64(&event["code"]).unwrap_or(0);
        if !codes.is_empty() && !codes.contains(&code) {
            continue;
        }
        total += json_u64(&event["count"]).unwrap_or(0);
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
        remediation_id: None,
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
        remediation_id: None,
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
        .all(|d| json_u64(&d["ConfigManagerErrorCode"]) == Some(22));
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
    let any_enabled = products
        .iter()
        .any(|p| json_u64(&p["productState"]).is_some_and(|state| ((state >> 8) & 0x10) != 0));
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

/// Defender is the active engine only in "Normal" mode; in Passive or EDR
/// Block mode another product owns real-time protection.
fn defender_is_active(defender: &Value) -> bool {
    defender["AMRunningMode"]
        .as_str()
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("Normal"))
}

fn defender_row(ctx: &DetectCtx) -> Option<Value> {
    let health = task_object(ctx, "defender_health")?;
    health.get("defender").cloned()
}

pub fn detect_realtime_protection_off(ctx: &DetectCtx) -> Option<Detection> {
    let defender = defender_row(ctx)?;
    if !defender_is_active(&defender) {
        return None;
    }
    (defender["RealTimeProtectionEnabled"].as_bool() == Some(false)).then(|| {
        Detection::new(
            "Microsoft Defender is the active antivirus but real-time protection is turned off, so new files and downloads are not being checked.",
        )
    })
}

pub fn detect_defender_definitions_stale(ctx: &DetectCtx) -> Option<Detection> {
    const MAX_AGE_DAYS: u64 = 7;
    let defender = defender_row(ctx)?;
    if !defender_is_active(&defender) {
        return None;
    }
    let age = json_u64(&defender["AntivirusSignatureAge"])?;
    (age > MAX_AGE_DAYS).then(|| {
        Detection::new(format!(
            "Microsoft Defender's security intelligence is {age} days old; new threats from the last week are not recognised."
        ))
    })
}

pub fn detect_defender_quick_scan_overdue(ctx: &DetectCtx) -> Option<Detection> {
    const MAX_AGE_DAYS: u64 = 30;
    let defender = defender_row(ctx)?;
    if !defender_is_active(&defender) {
        return None;
    }
    let quick = json_u64(&defender["QuickScanAge"])?;
    let full = json_u64(&defender["FullScanAge"]).unwrap_or(u64::MAX);
    (quick.min(full) > MAX_AGE_DAYS).then(|| {
        Detection::new(format!(
            "The last Microsoft Defender scan was {} days ago.",
            quick.min(full)
        ))
    })
}

pub fn detect_pending_reboot(ctx: &DetectCtx) -> Option<Detection> {
    let reboot = task_object(ctx, "pending_reboot")?;
    let restart_required = reboot
        .get("restart_required")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| reboot.get("pending").and_then(serde_json::Value::as_bool));
    if restart_required != Some(true) {
        return None;
    }
    let reasons: Vec<&str> = reboot["reasons"]
        .as_array()
        .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let explanations: Vec<&str> = reasons
        .iter()
        .filter_map(|reason| match *reason {
            "windows_update" => {
                Some("Windows Update reports that installed update work still needs a restart")
            }
            "component_based_servicing" => Some(
                "Windows component servicing reports that component changes still need a restart",
            ),
            // Legacy scans may contain this low-confidence signal. Apps use
            // it for deferred cleanup, so it must never create the actionable
            // restart issue by itself.
            "pending_file_rename" | "pending_file_operations" => None,
            _ => None,
        })
        .collect();
    if explanations.is_empty() {
        return None;
    }
    Some(Detection::new(format!("{}.", explanations.join("; "))))
}

pub fn detect_page_file_pressure(ctx: &DetectCtx) -> Option<Detection> {
    let performance = task_object(ctx, "performance")?;
    let total = json_u64(&performance["memory_performance"]["TotalPageFileMBytes"])?;
    let available = json_u64(&performance["memory_performance"]["AvailablePageFileMBytes"])?;
    if total == 0 {
        return None;
    }
    let available_percent = (available as f64 / total as f64) * 100.0;
    if available_percent < 10.0 {
        // ullTotalPageFile/ullAvailPageFile report the commit LIMIT and
        // remaining COMMIT CHARGE (RAM + page file combined), not file
        // occupancy - the text must say what is actually measured
        // (2026-09-03 audit).
        return Some(Detection::new(format!(
            "Commit charge is high: only {:.1}% of the {} MB commit limit (RAM + page file) remains available.",
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
        let hostname_lower = hostname.to_ascii_lowercase();
        let watched = HIJACK_WATCHED_DOMAINS.iter().any(|domain| {
            hostname_lower == *domain || hostname_lower.ends_with(&format!(".{}", domain))
        });
        if watched {
            // Classify by target: loopback entries block a domain (the most
            // common ad/telemetry tweak, and not itself a hijack); an
            // entry pointing at a real remote address is the redirect
            // signature worth a Critical (2026-09-03 audit).
            let loopback = ip == "0.0.0.0"
                || ip == "::"
                || ip == "127.0.0.1"
                || ip.starts_with("127.")
                || ip == "[::1]";
            return if loopback {
                Some(Detection::with_severity(
                    format!(
                        "hosts file blocks '{hostname}' (points to {ip}). Blocking entries are common for ads and telemetry; remove the line if this domain should resolve normally."
                    ),
                    IssueSeverity::Warning,
                ))
            } else {
                Some(Detection::with_severity(
                    format!(
                        "hosts file redirects '{hostname}' to {ip}. Security-sensitive domains should normally resolve through DNS; review this entry before trusting any login page."
                    ),
                    IssueSeverity::Critical,
                ))
            };
        }
    }
    None
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
    fn windows_update_failing_uses_the_newest_failure_family() {
        let results = results_with(
            "windows_update_events",
            r#"{"window_days": 30, "successes_after_last_failure": 0, "failures": [
                {"event_id": 20, "time_secs": 100, "error_code": "0x80070002", "update_title": "Old"},
                {"event_id": 20, "time_secs": 200, "error_code": "0x80070422", "update_title": "2026-08 Cumulative Update"}
            ]}"#,
        );
        let detection = detect_windows_update_failing(&ctx(&results)).expect("should detect");
        assert!(detection.description.contains("0x80070422"));
        assert!(detection.description.contains("2026-08 Cumulative Update"));
        assert_eq!(
            detection.remediation_id,
            Some("enable_windows_update_service")
        );

        let unknown = results_with(
            "windows_update_events",
            r#"{"window_days": 30, "successes_after_last_failure": 0, "failures": [
                {"event_id": 20, "time_secs": 100, "error_code": "0xDEADBEEF", "update_title": ""}
            ]}"#,
        );
        let detection = detect_windows_update_failing(&ctx(&unknown)).expect("should detect");
        assert!(detection.description.contains("an update"));
        assert_eq!(detection.remediation_id, Some("windows_update_reset"));
    }

    #[test]
    fn windows_update_failing_clears_after_a_later_success() {
        let results = results_with(
            "windows_update_events",
            r#"{"window_days": 30, "successes_after_last_failure": 1, "failures": [
                {"event_id": 20, "time_secs": 100, "error_code": "0x80070002", "update_title": "Old"}
            ]}"#,
        );
        assert!(detect_windows_update_failing(&ctx(&results)).is_none());
        let none = results_with(
            "windows_update_events",
            r#"{"window_days": 30, "successes_after_last_failure": 0, "failures": []}"#,
        );
        assert!(detect_windows_update_failing(&ctx(&none)).is_none());
    }

    #[test]
    fn windows_update_service_disabled_reads_the_services_start_mode() {
        let disabled = results_with(
            "services",
            r#"[{"Name": "wuauserv", "StartMode": "Disabled", "State": "Stopped"}]"#,
        );
        assert!(detect_windows_update_service_disabled(&ctx(&disabled)).is_some());
        let manual = results_with(
            "services",
            r#"[{"Name": "wuauserv", "StartMode": "Manual", "State": "Stopped"}]"#,
        );
        assert!(detect_windows_update_service_disabled(&ctx(&manual)).is_none());
    }

    #[test]
    fn space_consumers_rank_actionable_items_and_pick_the_top_remediation() {
        let results = results_with(
            "disk_usage",
            r#"{"drive": "C:", "total_bytes": 500000000000, "free_bytes": 20000000000, "consumers": [
                {"id": "downloads", "label": "Downloads", "bytes": 4400000000, "remediation": "open_downloads_folder"},
                {"id": "recycle_bin", "label": "Recycle Bin", "bytes": 6600000000, "remediation": "empty_recycle_bin"},
                {"id": "user_temp", "label": "Temporary files", "bytes": 1400000000, "remediation": "clear_temp_files"},
                {"id": "page_file", "label": "Page file", "bytes": 9000000000, "remediation": null},
                {"id": "desktop", "label": "Desktop", "bytes": 12000, "remediation": "open_storage_settings"}
            ]}"#,
        );
        let detection = detect_space_consumers(&ctx(&results)).expect("should detect");
        assert_eq!(
            detection.description,
            "Largest reclaimable items: Recycle Bin 6.1 GB, Downloads 4.1 GB, Temporary files 1.3 GB."
        );
        assert_eq!(detection.remediation_id, Some("empty_recycle_bin"));

        let quiet = results_with(
            "disk_usage",
            r#"{"drive": "C:", "total_bytes": 500000000000, "free_bytes": 20000000000, "consumers": [
                {"id": "downloads", "label": "Downloads", "bytes": 1000, "remediation": "open_downloads_folder"}
            ]}"#,
        );
        assert!(detect_space_consumers(&ctx(&quiet)).is_none());
    }

    #[test]
    fn low_disk_space_names_the_largest_items_when_the_breakdown_is_present() {
        let mut results = results_with(
            "logical_disk",
            r#"[{"Name": "C:", "FreeSpace": "5000000000", "Size": "100000000000"}]"#,
        );
        results.insert(
            "disk_usage".to_string(),
            ok_result(
                r#"{"total_bytes": 100000000000, "free_bytes": 5000000000, "consumers": [
                    {"id": "downloads", "label": "Downloads", "bytes": 30000000000, "remediation": "open_downloads_folder"}
                ]}"#,
            ),
        );
        let detection = detect_low_disk_space(&ctx(&results)).expect("should detect");
        assert!(
            detection
                .description
                .contains("Largest items: Downloads 27.9 GB.")
        );
    }

    #[test]
    fn bsod_recent_describes_the_newest_decoded_dump_and_picks_its_remediation() {
        let now = ctx(&HashMap::new()).now.secs;
        let output = format!(
            r#"{{"dumps": [
                {{"filename": "old.dmp", "created": {old}, "bugcheck": {{"name": "MEMORY_MANAGEMENT", "code": "0x0000001A", "cause_label": "usually faulty RAM", "plain": "x", "next_action": "y", "remediation": "open_memory_diagnostic"}}}},
                {{"filename": "new.dmp", "created": {new}, "bugcheck": {{"name": "DRIVER_IRQL_NOT_LESS_OR_EQUAL", "code": "0x000000D1", "faulting_module": "nvlddmkm.sys", "cause_label": "usually a faulty driver", "plain": "A driver accessed pageable memory at too high a priority level.", "next_action": "Update or roll back the driver.", "remediation": "open_device_manager"}}}}
            ]}}"#,
            old = now - 5 * 24 * 3600,
            new = now - 24 * 3600
        );
        let results = results_with("minidump", &output);
        let detection = detect_bsod_recent(&ctx(&results)).expect("should detect");
        assert!(detection.description.starts_with(
            "Latest crash: DRIVER_IRQL_NOT_LESS_OR_EQUAL (0x000000D1) in nvlddmkm.sys — usually a faulty driver."
        ));
        assert!(
            detection
                .description
                .ends_with("2 crash dump(s) in the last 30 days.")
        );
        assert_eq!(detection.remediation_id, Some("open_device_manager"));

        let undecoded = format!(
            r#"{{"dumps": [{{"filename": "a.dmp", "created": {new}, "bugcheck": null, "decode_error": "too short"}}]}}"#,
            new = now - 3600
        );
        let results = results_with("minidump", &undecoded);
        let detection = detect_bsod_recent(&ctx(&results)).expect("should detect");
        assert_eq!(
            detection.description,
            "1 blue-screen crash dump(s) from the last 30 days."
        );
        assert_eq!(detection.remediation_id, None);

        let stale = format!(
            r#"{{"dumps": [{{"filename": "a.dmp", "created": {old}}}]}}"#,
            old = now - 90 * 24 * 3600
        );
        assert!(detect_bsod_recent(&ctx(&results_with("minidump", &stale))).is_none());
    }

    #[test]
    fn defender_health_rules_fire_only_while_defender_is_the_active_engine() {
        let active = results_with(
            "defender_health",
            r#"{"defender": {"AMRunningMode": "Normal", "RealTimeProtectionEnabled": false, "AntivirusSignatureAge": 12, "QuickScanAge": 45, "FullScanAge": 200}}"#,
        );
        assert!(detect_realtime_protection_off(&ctx(&active)).is_some());
        let stale = detect_defender_definitions_stale(&ctx(&active)).expect("stale");
        assert!(stale.description.contains("12 days old"));
        let overdue = detect_defender_quick_scan_overdue(&ctx(&active)).expect("overdue");
        assert!(overdue.description.contains("45 days ago"));

        let passive = results_with(
            "defender_health",
            r#"{"defender": {"AMRunningMode": "Passive Mode", "RealTimeProtectionEnabled": false, "AntivirusSignatureAge": 40, "QuickScanAge": 90}}"#,
        );
        assert!(detect_realtime_protection_off(&ctx(&passive)).is_none());
        assert!(detect_defender_definitions_stale(&ctx(&passive)).is_none());
        assert!(detect_defender_quick_scan_overdue(&ctx(&passive)).is_none());

        let healthy = results_with(
            "defender_health",
            r#"{"defender": {"AMRunningMode": "Normal", "RealTimeProtectionEnabled": true, "AntivirusSignatureAge": "1", "QuickScanAge": "3", "FullScanAge": "60"}}"#,
        );
        assert!(detect_realtime_protection_off(&ctx(&healthy)).is_none());
        assert!(detect_defender_definitions_stale(&ctx(&healthy)).is_none());
        assert!(detect_defender_quick_scan_overdue(&ctx(&healthy)).is_none());
    }

    #[test]
    fn network_path_verdicts_map_to_one_rule_each() {
        let report = |verdict: &str| {
            results_with(
                "network_path",
                &format!(
                    r#"{{"verdict": "{verdict}", "probes": {{"gateway": "192.168.1.1"}}, "dns_probe_host": "www.msftconnecttest.com"}}"#
                ),
            )
        };
        assert!(detect_no_internet(&ctx(&report("no_internet"))).is_some());
        let wan = detect_no_internet(&ctx(&report("wan_down"))).expect("wan down");
        assert!(wan.description.contains("192.168.1.1"));
        assert!(detect_gateway_unreachable(&ctx(&report("gateway_unreachable"))).is_some());
        let dns =
            detect_dns_resolution_failing(&ctx(&report("dns_resolution_failing"))).expect("dns");
        assert!(dns.description.contains("www.msftconnecttest.com"));
        for verdict in ["clear", "unknown"] {
            assert!(detect_no_internet(&ctx(&report(verdict))).is_none());
            assert!(detect_gateway_unreachable(&ctx(&report(verdict))).is_none());
            assert!(detect_dns_resolution_failing(&ctx(&report(verdict))).is_none());
        }
        assert!(detect_gateway_unreachable(&ctx(&report("wan_down"))).is_none());
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
    fn unsupported_stopped_auto_service_not_mapped_to_core_service_fix() {
        let results = results_with(
            "services",
            r#"[{"Name": "SomeVendorSvc", "State": "Stopped", "StartMode": "Auto"}]"#,
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
    fn installed_update_age_is_not_treated_as_pending() {
        let results = results_with(
            "windows_update",
            r#"{"installed_updates": [{"HotFixID": "KB1", "InstalledOn": "1/10/2020"}]}"#,
        );
        assert!(detect_pending_windows_updates(&ctx(&results)).is_none());

        let results = results_with(
            "windows_update",
            r#"{"installed_updates": [], "pending_update_count": 3}"#,
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
        // The native producer also emits OperationalStatusText; text-only
        // payloads must not be treated as healthy.
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD G", "OperationalStatusText": "Degraded", "HealthStatus": 0}]}"#,
        );
        assert!(detect_disk_unhealthy(&ctx(&results)).is_some());
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD H", "OperationalStatusText": "Error", "HealthStatus": 0}]}"#,
        );
        let d = detect_disk_unhealthy(&ctx(&results)).expect("text operational error");
        assert_eq!(d.severity, Some(IssueSeverity::Critical));
        let results = results_with(
            "chkdsk",
            r#"{"disks": [{"Model": "SSD I", "OperationalStatusText": "Predictive Failure", "HealthStatus": 0}]}"#,
        );
        assert!(detect_smart_failure_predicted(&ctx(&results)).is_some());
        let results = results_with("chkdsk", r#"{"disks": [], "errors_found": true}"#);
        assert!(detect_disk_unhealthy(&ctx(&results)).is_some());
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
    fn pending_reboot_requires_a_high_confidence_source_and_explains_it() {
        let results = results_with(
            "pending_reboot",
            r#"{"pending": true, "reasons": ["pending_file_rename"]}"#,
        );
        assert!(detect_pending_reboot(&ctx(&results)).is_none());

        let results = results_with("pending_reboot", r#"{"pending": true, "reasons": []}"#);
        assert!(detect_pending_reboot(&ctx(&results)).is_none());

        let results = results_with(
            "pending_reboot",
            r#"{"pending": true, "reasons": ["windows_update", "pending_file_rename"]}"#,
        );
        let update = detect_pending_reboot(&ctx(&results)).unwrap();
        assert!(update.severity.is_none());
        assert!(update.description.contains("Windows Update"));
        assert!(!update.description.contains("pending_file_rename"));

        let results = results_with(
            "pending_reboot",
            r#"{"pending": true, "reasons": ["component_based_servicing", "windows_update"]}"#,
        );
        let combined = detect_pending_reboot(&ctx(&results)).unwrap();
        assert!(combined.description.contains("Windows component servicing"));
        assert!(combined.description.contains("Windows Update"));
        assert!(!combined.description.contains('_'));

        let results = results_with("pending_reboot", r#"{"pending": false, "reasons": []}"#);
        assert!(detect_pending_reboot(&ctx(&results)).is_none());

        let results = results_with(
            "pending_reboot",
            r#"{"restart_required": false, "pending": true, "reasons": ["windows_update"]}"#,
        );
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
    fn hosts_hijack_includes_loopback_for_watched_domains() {
        // Unwatched ad-block entries are fine, but loopback redirection of a
        // security-sensitive domain is still important evidence.
        let results = results_with(
            "hosts_file",
            r#"{"entries": [{"ip": "0.0.0.0", "hostname": "ads.example.com"}]}"#,
        );
        assert!(detect_hosts_file_hijack(&ctx(&results)).is_none());
        let results = results_with(
            "hosts_file",
            r#"{"entries": [{"ip": "127.0.0.1", "hostname": "microsoft.com"}]}"#,
        );
        assert!(detect_hosts_file_hijack(&ctx(&results)).is_some());
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
}
