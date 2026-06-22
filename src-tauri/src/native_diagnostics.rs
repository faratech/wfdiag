use crate::wmi_native::WmiConnection;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
    EvtRender, EvtRenderEventXml,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::core::PCWSTR;
// Performance counter imports removed - not used in current implementation
use winreg::RegKey;
use winreg::enums::*;

pub struct NativeDiagnostics;

struct EventLogHandle(EVT_HANDLE);

impl Drop for EventLogHandle {
    fn drop(&mut self) {
        // Best-effort cleanup; closing an event log handle cannot affect diagnostics.
        let _ = unsafe { EvtClose(self.0) };
    }
}

#[derive(Debug)]
struct EventRecord {
    source: String,
    code: u64,
    time_iso: String,
    time_secs: i64,
    level: Option<u64>,
    sample_message: String,
    event_data: serde_json::Map<String, Value>,
}

#[allow(dead_code)]
impl NativeDiagnostics {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn run_wmi_query(&self, class_name: &str, namespace: Option<&str>) -> Result<Value> {
        let wmi_con = if let Some(ns) = namespace {
            WmiConnection::with_namespace(ns)?
        } else {
            WmiConnection::new()?
        };
        let results = wmi_con.query_class(class_name)?;

        let json_results: Vec<Value> = results
            .into_iter()
            .map(|r| {
                let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                Value::Object(obj)
            })
            .collect();

        Ok(Value::Array(json_results))
    }

    pub fn get_operating_system_info(&self) -> Result<Value> {
        let mut value = self.run_wmi_query("Win32_OperatingSystem", None)?;
        let registry = self.windows_release_registry_info();
        if let Value::Array(rows) = &mut value
            && let Some(Value::Object(first)) = rows.first_mut()
        {
            for (key, value) in registry {
                first.insert(key, value);
            }
        }
        Ok(value)
    }

    fn windows_release_registry_info(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
            return out;
        };

        for name in [
            "ProductName",
            "DisplayVersion",
            "ReleaseId",
            "CurrentBuild",
            "CurrentBuildNumber",
            "EditionID",
            "InstallationType",
        ] {
            if let Ok(value) = key.get_value::<String, _>(name) {
                out.insert(name.to_string(), json!(value));
            }
        }
        if let Ok(ubr) = key.get_value::<u32, _>("UBR") {
            out.insert("UBR".to_string(), json!(ubr));
            let build = out
                .get("CurrentBuild")
                .or_else(|| out.get("CurrentBuildNumber"))
                .and_then(Value::as_str);
            if let Some(build) = build {
                out.insert("FullBuild".to_string(), json!(format!("{build}.{ubr}")));
            }
        }
        out
    }

    /// High-signal System-log event codes for issue detection, aggregated by
    /// (source, code) over the last 7 days. Deliberately separate from the
    /// `event_logs` task (Error-type-only, 50-row cap): Kernel-Power 41 is a
    /// Critical-level record, so no Type filter is used here. Time filtering
    /// happens in the Event Log query, which avoids the slow unbounded WMI
    /// `Win32_NTLogEvent` sweep.
    pub fn get_critical_event_codes(&self) -> Result<Value> {
        const WINDOW_DAYS: i64 = 7;
        const ROW_CAP: usize = 100;
        // (source, codes); empty codes = every event from that source
        const TARGETS: &[(&str, &[u32])] = &[
            ("Microsoft-Windows-Kernel-Power", &[41]),
            ("EventLog", &[6008]),
            ("disk", &[7, 51, 153]),
            ("Microsoft-Windows-WHEA-Logger", &[]),
            ("Service Control Manager", &[7031, 7034]),
        ];

        let mut events: Vec<Value> = Vec::new();

        for (source, codes) in TARGETS {
            let records =
                match Self::query_event_records("System", source, codes, WINDOW_DAYS, ROW_CAP) {
                    Ok(records) => records,
                    Err(e) => {
                        eprintln!("Failed to query critical events for {}: {}", source, e);
                        continue;
                    }
                };
            let mut groups: HashMap<u64, (u64, i64, String)> = HashMap::new();
            for record in records {
                let entry = groups.entry(record.code).or_insert((0, 0, String::new()));
                entry.0 += 1;
                if record.time_secs >= entry.1 {
                    entry.1 = record.time_secs;
                    entry.2 = record.sample_message;
                }
            }
            for (code, (count, last_seen, sample_message)) in groups {
                events.push(json!({
                    "source": source,
                    "code": code,
                    "count": count,
                    "last_seen": crate::timestamp::Timestamp::from_secs(last_seen).to_iso_string(),
                    "sample_message": sample_message,
                }));
            }
        }

        Ok(json!({ "window_days": WINDOW_DAYS, "events": events }))
    }

    fn query_event_records(
        channel: &str,
        source: &str,
        codes: &[u32],
        window_days: i64,
        row_cap: usize,
    ) -> Result<Vec<EventRecord>> {
        let mut predicates = vec![format!(
            "Provider[@Name='{}']",
            source.replace('\'', "&apos;")
        )];
        if !codes.is_empty() {
            let code_predicate = codes
                .iter()
                .map(|code| format!("EventID={}", code))
                .collect::<Vec<_>>()
                .join(" or ");
            predicates.push(format!("({})", code_predicate));
        }
        Self::query_channel_events(channel, &predicates, window_days, row_cap)
    }

    fn query_recent_error_events(
        channel: &str,
        window_days: i64,
        row_cap: usize,
    ) -> Result<Vec<EventRecord>> {
        Self::query_channel_events(
            channel,
            &["(Level=1 or Level=2)".to_string()],
            window_days,
            row_cap,
        )
    }

    fn query_channel_events(
        channel: &str,
        system_predicates: &[String],
        window_days: i64,
        row_cap: usize,
    ) -> Result<Vec<EventRecord>> {
        let window_ms = window_days * 24 * 60 * 60 * 1000;
        let mut predicates = Vec::with_capacity(system_predicates.len() + 1);
        predicates.push(format!(
            "TimeCreated[timediff(@SystemTime) <= {}]",
            window_ms
        ));
        predicates.extend(system_predicates.iter().cloned());
        let xpath = format!("*[System[{}]]", predicates.join(" and "));

        let channel_w = wide_null(channel);
        let xpath_w = wide_null(&xpath);
        let query = unsafe {
            EvtQuery(
                None,
                PCWSTR(channel_w.as_ptr()),
                PCWSTR(xpath_w.as_ptr()),
                EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
            )
        }?;
        let query = EventLogHandle(query);

        let mut records = Vec::new();
        let mut handles = [0isize; 16];
        while records.len() < row_cap {
            let mut returned = 0u32;
            if unsafe { EvtNext(query.0, &mut handles, 250, 0, &mut returned) }.is_err()
                || returned == 0
            {
                break;
            }

            for raw_handle in handles.iter().take(returned as usize) {
                let event_handle = EVT_HANDLE(*raw_handle);
                let record = Self::render_event_record(event_handle);
                let _ = unsafe { EvtClose(event_handle) };

                if let Some(record) = record {
                    records.push(record);
                    if records.len() >= row_cap {
                        break;
                    }
                }
            }
        }

        Ok(records)
    }

    fn render_event_record(event_handle: EVT_HANDLE) -> Option<EventRecord> {
        let xml = Self::render_event_xml(event_handle).ok()?;
        let source = xml_attr(&xml, "Provider", "Name")?;
        let code = xml_text(&xml, "EventID")?.parse::<u64>().ok()?;
        let time_iso = xml_attr(&xml, "TimeCreated", "SystemTime")?;
        let time_secs = crate::timestamp::Timestamp::from_iso_string(&time_iso)
            .ok()?
            .secs;
        let event_data = xml_event_data(&xml);
        let sample_message =
            event_data_summary(&event_data).unwrap_or_else(|| format!("{} event {}", source, code));

        Some(EventRecord {
            source,
            code,
            time_iso,
            time_secs,
            level: xml_text(&xml, "Level").and_then(|level| level.parse().ok()),
            sample_message,
            event_data,
        })
    }

    fn render_event_xml(event_handle: EVT_HANDLE) -> Result<String> {
        let mut buffer_used = 0u32;
        let mut property_count = 0u32;
        let _ = unsafe {
            EvtRender(
                None,
                event_handle,
                EvtRenderEventXml.0,
                0,
                None,
                &mut buffer_used,
                &mut property_count,
            )
        };
        if buffer_used == 0 {
            return Err(anyhow::anyhow!("Event XML render returned no buffer"));
        }

        let mut buffer = vec![0u16; (buffer_used as usize).div_ceil(2)];
        unsafe {
            EvtRender(
                None,
                event_handle,
                EvtRenderEventXml.0,
                buffer_used,
                Some(buffer.as_mut_ptr().cast()),
                &mut buffer_used,
                &mut property_count,
            )
        }?;

        let len = buffer
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..len]))
    }

    /// Pending-reboot detection via the canonical registry markers.
    pub fn get_pending_reboot(&self) -> Result<Value> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut reasons: Vec<&str> = Vec::new();
        if hklm
            .open_subkey(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
            )
            .is_ok()
        {
            reasons.push("component_based_servicing");
        }
        if hklm
            .open_subkey(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
            )
            .is_ok()
        {
            reasons.push("windows_update");
        }
        if let Ok(key) = hklm.open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager")
            && key.get_raw_value("PendingFileRenameOperations").is_ok()
        {
            reasons.push("pending_file_rename");
        }
        Ok(json!({ "pending": !reasons.is_empty(), "reasons": reasons }))
    }

    /// Devices reporting a Device Manager problem code (yellow-bang devices).
    /// Code 22 = disabled by the user — callers treat it as lower severity.
    pub fn get_device_errors(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let rows = wmi_con.query(
            "SELECT Name, DeviceID, ConfigManagerErrorCode, Status FROM Win32_PnPEntity \
             WHERE ConfigManagerErrorCode <> 0",
        )?;
        let json_results: Vec<Value> = rows
            .into_iter()
            .map(|r| Value::Object(r.into_iter().collect()))
            .collect();
        Ok(Value::Array(json_results))
    }

    /// Antivirus products from Security Center (same productState bitfield
    /// family as the firewall task). The namespace is absent on Server SKUs —
    /// the resulting task failure reads as "unknown", never as "disabled".
    pub fn get_defender_status(&self) -> Result<Value> {
        self.run_wmi_query("AntiVirusProduct", Some(r"root\SecurityCenter2"))
    }

    pub fn get_native_disk_space(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let results = wmi_con.query("SELECT * FROM Win32_LogicalDisk WHERE DriveType=3")?;

        let drives: Vec<Value> = results
            .into_iter()
            .map(|r| {
                let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                Value::Object(obj)
            })
            .collect();

        Ok(Value::Array(drives))
    }

    pub fn get_native_network_adapters(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let config_results = wmi_con
            .query("SELECT * FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled=TRUE")?;
        let adapter_results = wmi_con.query("SELECT * FROM Win32_NetworkAdapter")?;

        let mut adapters = Vec::new();

        for config in config_results {
            let mut adapter_info: serde_json::Map<String, Value> =
                config.clone().into_iter().collect();

            // Get the index to match with adapter
            let index = config
                .get("Index")
                .and_then(|v| v.as_u64())
                .map(|u| u as u32);

            // Find matching adapter info.
            // Match on Win32_NetworkAdapter.Index (a uint32 that equals the config's
            // Index), NOT DeviceID. DeviceID is a CIM_STRING, so as_u64() always returned
            // None and the enrichment (Name/Speed/MAC/Manufacturer/...) never merged.
            if let Some(idx) = index {
                for adapter in &adapter_results {
                    if let Some(adapter_idx) = adapter
                        .get("Index")
                        .and_then(|v| v.as_u64())
                        .map(|u| u as u32)
                        && adapter_idx == idx
                    {
                        // Add adapter-specific info
                        for (key, value) in adapter {
                            if !adapter_info.contains_key(key) {
                                adapter_info.insert(key.clone(), value.clone());
                            }
                        }
                        break;
                    }
                }
            }

            adapters.push(Value::Object(adapter_info));
        }

        Ok(Value::Array(adapters))
    }

    pub fn get_system_info(&self) -> Result<Value> {
        let mut system_info = SYSTEM_INFO::default();
        unsafe {
            GetSystemInfo(&mut system_info);
        }

        Ok(json!({
            "processor_architecture": unsafe { system_info.Anonymous.Anonymous.wProcessorArchitecture.0 },
            "number_of_processors": system_info.dwNumberOfProcessors,
            "page_size": system_info.dwPageSize,
            "minimum_application_address": format!("{:p}", system_info.lpMinimumApplicationAddress),
            "maximum_application_address": format!("{:p}", system_info.lpMaximumApplicationAddress),
            "active_processor_mask": system_info.dwActiveProcessorMask,
            "processor_type": system_info.dwProcessorType,
            "allocation_granularity": system_info.dwAllocationGranularity,
            "processor_level": system_info.wProcessorLevel,
            "processor_revision": system_info.wProcessorRevision,
        }))
    }

    pub fn get_native_system_info(&self) -> Result<Value> {
        // Get OS info from WMI
        let wmi_con = WmiConnection::new()?;
        let os_results = wmi_con.query("SELECT * FROM Win32_OperatingSystem")?;
        let comp_results = wmi_con.query("SELECT * FROM Win32_ComputerSystem")?;

        let mut info = json!({});

        // Add OS information
        if let Some(os) = os_results.first() {
            let mut os_info = json!({});
            for (key, value) in os {
                os_info[key] = value.clone();
            }

            // Parse Windows version details
            if let Some(caption) = os.get("Caption").and_then(|v| v.as_str()) {
                let windows_version = if caption.contains("Windows 11") {
                    "Windows 11"
                } else if caption.contains("Windows 10") {
                    "Windows 10"
                } else if caption.contains("Windows 8.1") {
                    "Windows 8.1"
                } else if caption.contains("Windows 8") {
                    "Windows 8"
                } else if caption.contains("Windows 7") {
                    "Windows 7"
                } else if caption.contains("Server 2022") {
                    "Windows Server 2022"
                } else if caption.contains("Server 2019") {
                    "Windows Server 2019"
                } else if caption.contains("Server 2016") {
                    "Windows Server 2016"
                } else {
                    "Windows"
                };
                os_info["windows_version"] = json!(windows_version);
            }

            info["os_version"] = os_info;
        }

        // Add Computer System information
        if let Some(comp) = comp_results.first() {
            let mut comp_info = json!({});
            for (key, value) in comp {
                comp_info[key] = value.clone();
            }
            info["computer_system"] = comp_info;
        }

        // Add native system info
        let mut native_info = SYSTEM_INFO::default();
        unsafe {
            GetSystemInfo(&mut native_info);
        }

        info["processor_info"] = json!({
            "architecture": unsafe { native_info.Anonymous.Anonymous.wProcessorArchitecture.0 },
            "processor_count": native_info.dwNumberOfProcessors,
            "processor_type": native_info.dwProcessorType,
            "processor_level": native_info.wProcessorLevel,
            "processor_revision": native_info.wProcessorRevision,
        });

        // Get additional system info
        {
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(cv_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
            {
                let mut version_info = json!({});

                // Read various version fields
                for field in &[
                    "ProductName",
                    "DisplayVersion",
                    "CurrentBuild",
                    "UBR",
                    "EditionID",
                    "CompositionEditionID",
                ] {
                    if let Ok(value) = cv_key.get_value::<String, _>(field) {
                        version_info[field] = json!(value);
                    }
                }

                info["windows_version_details"] = version_info;
            }

            // Get hardware info
            if let Ok(hw_key) =
                hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0")
                && let Ok(cpu_name) = hw_key.get_value::<String, _>("ProcessorNameString")
            {
                info["cpu_name"] = json!(cpu_name.trim());
            }
        }

        Ok(info)
    }

    pub fn get_drivers(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let mut drivers = Vec::new();

        // Get PnP signed drivers (main source on modern Windows)
        match wmi_con.query(
            "SELECT Name, DeviceName, DriverVersion, DriverDate, DriverProviderName, DeviceClass, IsSigned FROM Win32_PnPSignedDriver"
        ) {
            Ok(pnp_results) => {
                for result in pnp_results {
                    let driver_info: serde_json::Map<String, Value> = result.into_iter().collect();
                    drivers.push(Value::Object(driver_info));
                }
            }
            Err(e) => {
                eprintln!("Failed to query Win32_PnPSignedDriver: {}", e);
            }
        }

        // Try to get legacy VxD drivers (might not exist on modern systems)
        match wmi_con
            .query("SELECT Name, DriverVersion, DriverDate, DeviceName FROM Win32_DriverVXD")
        {
            Ok(vxd_results) => {
                for result in vxd_results {
                    let mut driver_info: serde_json::Map<String, Value> =
                        result.into_iter().collect();
                    driver_info.insert("Type".to_string(), json!("VxD"));
                    drivers.push(Value::Object(driver_info));
                }
            }
            Err(_) => {
                // VxD drivers not available on this system - this is normal for modern Windows
            }
        }

        // If no drivers found, return error
        if drivers.is_empty() {
            return Err(anyhow::anyhow!("No drivers found or WMI query failed"));
        }

        Ok(Value::Array(drivers))
    }

    pub fn get_event_logs(&self) -> Result<Value> {
        const WINDOW_DAYS: i64 = 7;
        const ROW_CAP_PER_LOG: usize = 50;
        let mut all_events = Vec::new();

        for log_file in ["System", "Application"] {
            let Ok(records) =
                Self::query_recent_error_events(log_file, WINDOW_DAYS, ROW_CAP_PER_LOG)
            else {
                continue;
            };
            for record in records {
                all_events.push(json!({
                    "LogFile": log_file,
                    "TimeGenerated": record.time_iso,
                    "Type": match record.level {
                        Some(1) => "Critical",
                        Some(2) => "Error",
                        _ => "Unknown",
                    },
                    "SourceName": record.source,
                    "EventCode": record.code,
                    "Message": record.sample_message,
                    "EventData": record.event_data,
                }));
            }
        }

        Ok(Value::Array(all_events))
    }

    pub fn get_installed_programs(&self) -> Result<Value> {
        let mut programs = Vec::new();

        // Check both 32-bit and 64-bit registry locations
        let paths = vec![
            (
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ),
            (
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ),
            (
                HKEY_CURRENT_USER,
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ),
        ];

        for (hkey, path) in paths {
            if let Ok(key) = RegKey::predef(hkey).open_subkey(path) {
                for subkey_name in key.enum_keys().filter_map(Result::ok) {
                    if let Ok(subkey) = key.open_subkey(&subkey_name) {
                        let mut program_info = serde_json::Map::new();

                        // Read common fields
                        for field in &[
                            "DisplayName",
                            "DisplayVersion",
                            "Publisher",
                            "InstallDate",
                            "UninstallString",
                            "InstallLocation",
                        ] {
                            if let Ok(value) = subkey.get_value::<String, _>(field)
                                && !value.is_empty()
                            {
                                program_info.insert(field.to_string(), json!(value));
                            }
                        }

                        // Only add if it has a display name
                        if program_info.contains_key("DisplayName") {
                            programs.push(Value::Object(program_info));
                        }
                    }
                }
            }
        }

        Ok(json!(programs))
    }

    pub fn run_dxdiag(&self) -> Result<Value> {
        eprintln!("[DXDIAG] Starting DirectX diagnostic");

        // Always use WMI as primary method - it's more reliable
        eprintln!("[DXDIAG] Getting DirectX info via WMI");
        self.get_directx_info_via_wmi()
    }

    fn get_directx_info_via_wmi(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let mut info = json!({
            "source": "WMI",
            "description": "DirectX information gathered from Windows Management Instrumentation"
        });

        // Try to determine DirectX version from registry
        {
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(dx_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\DirectX") {
                if let Ok(version) = dx_key.get_value::<String, _>("Version") {
                    info["directx_version"] = json!(version);
                }
                if let Ok(install_version) = dx_key.get_value::<u32, _>("InstalledVersion") {
                    info["directx_installed_version"] = json!(install_version);
                }
            }
        }

        // Get video controller info
        if let Ok(video_results) = wmi_con.query("SELECT * FROM Win32_VideoController") {
            let video_info: Vec<Value> = video_results
                .into_iter()
                .map(|r| {
                    let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                    Value::Object(obj)
                })
                .collect();
            info["video_controllers"] = json!(video_info);
        }

        // Get sound device info
        if let Ok(sound_results) = wmi_con.query("SELECT * FROM Win32_SoundDevice") {
            let sound_info: Vec<Value> = sound_results
                .into_iter()
                .map(|r| {
                    let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                    Value::Object(obj)
                })
                .collect();
            info["sound_devices"] = json!(sound_info);
        }

        Ok(info)
    }

    pub fn run_chkdsk(&self) -> Result<Value> {
        // Use native disk health check via Storage Management WMI - much faster than chkdsk
        self.get_disk_health()
    }

    /// Get disk health using Windows Storage Management API (MSFT_PhysicalDisk)
    /// This is much faster than running chkdsk and provides SMART-like health data
    pub fn get_disk_health(&self) -> Result<Value> {
        let mut health_info = json!({
            "status": "Healthy",
            "message": "All disks operating normally",
            "errors_found": false,
            "disks": []
        });

        let mut all_healthy = true;
        let mut disks_data = Vec::new();

        // Try Storage Management namespace for detailed health (MSFT_PhysicalDisk)
        if let Ok(wmi_storage) = WmiConnection::with_namespace(r"root\Microsoft\Windows\Storage") {
            if let Ok(physical_disks) = wmi_storage.query("SELECT * FROM MSFT_PhysicalDisk") {
                for disk in physical_disks {
                    let mut disk_info: serde_json::Map<String, Value> = disk.into_iter().collect();

                    // Parse health status (0=Healthy, 1=Warning, 2=Unhealthy)
                    // Default to 255 (Unknown) instead of 0 (Healthy) to avoid hiding issues
                    let health_status = disk_info
                        .get("HealthStatus")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(255);

                    // MSFT_PhysicalDisk.OperationalStatus is a UInt16[] array (CIM),
                    // rendered as a JSON array — pick the most severe problem code so
                    // the text/health flag reflect it. Default 0 (Unknown).
                    let operational_status = match disk_info.get("OperationalStatus") {
                        Some(Value::Array(states)) => {
                            let codes: Vec<u64> =
                                states.iter().filter_map(|s| s.as_u64()).collect();
                            // Prefer a problem status (Error > Predictive > Stressed >
                            // Degraded) over OK/Unknown when several are reported.
                            [6, 5, 4, 3]
                                .into_iter()
                                .find(|c| codes.contains(c))
                                .or_else(|| codes.first().copied())
                                .unwrap_or(0)
                        }
                        Some(v) => v.as_u64().unwrap_or(0),
                        None => 0,
                    };

                    // Convert operational status to text (0=Unknown, 1=Other, 2=OK, 3=Degraded, etc.)
                    let operational_str = match operational_status {
                        0 => "Unknown",
                        2 => "OK",
                        3 => {
                            all_healthy = false;
                            "Degraded"
                        }
                        4 => {
                            all_healthy = false;
                            "Stressed"
                        }
                        5 => {
                            all_healthy = false;
                            "Predictive Failure"
                        }
                        6 => {
                            all_healthy = false;
                            "Error"
                        }
                        _ => "Unknown",
                    };
                    disk_info.insert("OperationalStatusText".to_string(), json!(operational_str));

                    let health_str = match health_status {
                        0 => "Healthy",
                        1 => {
                            all_healthy = false;
                            "Warning"
                        }
                        2 => {
                            all_healthy = false;
                            "Unhealthy"
                        }
                        _ => "Unknown", // Includes 255 (unavailable) and any unexpected values
                    };

                    disk_info.insert("HealthStatusText".to_string(), json!(health_str));

                    // Media type (0=Unspecified, 3=HDD, 4=SSD, 5=SCM)
                    let media_type = disk_info
                        .get("MediaType")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let media_str = match media_type {
                        3 => "HDD",
                        4 => "SSD",
                        5 => "SCM",
                        _ => "Unknown",
                    };
                    disk_info.insert("MediaTypeText".to_string(), json!(media_str));

                    // Bus type
                    let bus_type = disk_info
                        .get("BusType")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let bus_str = match bus_type {
                        1 => "SCSI",
                        2 => "ATAPI",
                        3 => "ATA",
                        4 => "1394",
                        5 => "SSA",
                        6 => "Fibre",
                        7 => "USB",
                        8 => "RAID",
                        9 => "iSCSI",
                        10 => "SAS",
                        11 => "SATA",
                        12 => "SD",
                        13 => "MMC",
                        15 => "File Backed Virtual",
                        16 => "Storage Spaces",
                        17 => "NVMe",
                        _ => "Unknown",
                    };
                    disk_info.insert("BusTypeText".to_string(), json!(bus_str));

                    disks_data.push(Value::Object(disk_info));
                }
            }

            // Also get reliability counters if available
            if let Ok(reliability) =
                wmi_storage.query("SELECT * FROM MSFT_StorageReliabilityCounter")
            {
                health_info["reliability_counters"] = json!(
                    reliability
                        .into_iter()
                        .map(|r| {
                            let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                            Value::Object(obj)
                        })
                        .collect::<Vec<_>>()
                );
            }
        }

        // Fallback to Win32_DiskDrive if Storage namespace failed
        if disks_data.is_empty() {
            let wmi_con = WmiConnection::new()?;
            if let Ok(results) = wmi_con.query("SELECT Model, Size, InterfaceType, MediaType, Status, Partitions FROM Win32_DiskDrive") {
                for disk in results {
                    let mut disk_info: serde_json::Map<String, Value> = disk.into_iter().collect();

                    // Check disk status - default to "Unknown" instead of "OK" to avoid hiding issues
                    let status = disk_info
                        .get("Status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");

                    if status != "OK" && status != "Unknown" {
                        all_healthy = false;
                    }

                    disk_info.insert("HealthStatusText".to_string(), json!(status));
                    disks_data.push(Value::Object(disk_info));
                }
            }
        }

        health_info["disks"] = json!(disks_data);

        if !all_healthy {
            health_info["status"] = json!("Warning");
            health_info["message"] = json!("One or more disks may have issues");
            health_info["errors_found"] = json!(true);
        }

        Ok(health_info)
    }

    pub fn run_dism_health(&self) -> Result<Value> {
        // Run DISM health check using secure execution
        let output =
            Self::execute_secure_command("dism", &["/online", "/cleanup-image", "/checkhealth"])?;

        if output.status.success() {
            let output_str = crate::security::decode_windows_output(&output.stdout);

            // Parse DISM output
            let mut health_info = json!({
                "raw_output": output_str.to_string(),
                "status": "Unknown",
                "repairable": false
            });

            // Check for common DISM responses
            if output_str.contains("No component store corruption detected") {
                health_info["status"] = json!("Healthy");
                health_info["message"] = json!("No component store corruption detected");
                health_info["repairable"] = json!(false);
            } else if output_str.contains("The component store is repairable") {
                health_info["status"] = json!("Repairable");
                health_info["message"] = json!("The component store is repairable");
                health_info["repairable"] = json!(true);
            } else if output_str.contains("The component store is corrupted") {
                health_info["status"] = json!("Corrupted");
                health_info["message"] = json!("The component store is corrupted");
                health_info["repairable"] = json!(true);
            }

            // Try to run scanhealth for more detailed info
            let scan_output =
                Self::execute_secure_command("dism", &["/online", "/cleanup-image", "/scanhealth"]);

            if let Ok(scan) = scan_output
                && scan.status.success()
            {
                let scan_str = crate::security::decode_windows_output(&scan.stdout);
                health_info["scan_output"] = json!(scan_str.clone());

                // Extract percentage if available
                if let Some(percent_pos) = scan_str.find("The component store is") {
                    let relevant_text = &scan_str[percent_pos..];
                    if let Some(end) = relevant_text.find('\n') {
                        health_info["scan_result"] = json!(&relevant_text[..end]);
                    }
                }
            }

            Ok(health_info)
        } else {
            let error_str = crate::security::decode_windows_output(&output.stderr);

            // Check if it's an elevation error
            if error_str.contains("Error: 740") || error_str.contains("elevation required") {
                Ok(json!({
                    "error": "DISM requires administrator privileges",
                    "suggestion": "Please run as administrator to check Windows image health",
                    "raw_error": error_str.clone()
                }))
            } else {
                Err(anyhow::anyhow!("DISM health check failed: {}", error_str))
            }
        }
    }

    pub fn run_ipconfig(&self) -> Result<Value> {
        // Query Win32_NetworkAdapterConfiguration for IP settings (different from Network Adapters)
        let wmi_con = WmiConnection::new()?;

        let results = wmi_con.query("SELECT Description, IPAddress, IPSubnet, DefaultIPGateway, DNSServerSearchOrder, DHCPEnabled, DHCPServer, MACAddress, DNSDomain FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled = TRUE")?;

        let configs: Vec<Value> = results
            .into_iter()
            .map(|config| {
                let mut obj = serde_json::Map::new();

                // Copy relevant fields, filtering out nulls
                for (key, value) in config {
                    if !value.is_null() {
                        obj.insert(key, value);
                    }
                }

                Value::Object(obj)
            })
            .collect();

        Ok(json!(configs))
    }

    pub fn read_hosts_file(&self) -> Result<Value> {
        let hosts_path = "C:\\Windows\\System32\\drivers\\etc\\hosts";
        match fs::read_to_string(hosts_path) {
            Ok(content) => Ok(json!({
                "path": hosts_path,
                "content": content,
                "entries": self.parse_hosts_file(&content)
            })),
            Err(e) => Ok(json!({
                "path": hosts_path,
                "error": format!("Failed to read hosts file: {}", e)
            })),
        }
    }

    fn parse_hosts_file(&self, content: &str) -> Vec<Value> {
        let mut entries = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    entries.push(json!({
                        "ip": parts[0],
                        "hostname": parts[1],
                        "aliases": parts[2..].join(" ")
                    }));
                }
            }
        }

        entries
    }

    pub fn run_dsregcmd(&self) -> Result<Value> {
        // Check domain join status via WMI
        let wmi_con = WmiConnection::new()?;

        let cs_results =
            wmi_con.query("SELECT Domain, DomainRole, PartOfDomain FROM Win32_ComputerSystem")?;

        if let Some(result) = cs_results.into_iter().next() {
            let info: serde_json::Map<String, Value> = result.into_iter().collect();
            Ok(Value::Object(info))
        } else {
            Ok(json!({
                "error": "Failed to query domain information"
            }))
        }
    }

    pub fn get_disk_fragmentation(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let disks = wmi_con.query("SELECT Name FROM Win32_LogicalDisk WHERE DriveType=3")?;

        let mut fragmentation_results = Vec::new();

        for disk in disks {
            if let Some(drive_letter) = disk.get("Name").and_then(|v| v.as_str()) {
                let mut result_info = json!({
                    "drive": drive_letter,
                    "fragmentation_percent": null,
                    "status": "Not analyzed",
                    "raw_output": ""
                });

                match Self::execute_secure_command("defrag", &[drive_letter, "/A"]) {
                    Ok(output) => {
                        // Use the OEM-codepage decoder like every other command consumer
                        // in this file; defrag emits OEM text, which from_utf8_lossy would
                        // corrupt to U+FFFD on non-English systems.
                        let output_str = crate::security::decode_windows_output(&output.stdout);
                        result_info["raw_output"] = json!(output_str.to_string());

                        if output.status.success() {
                            if let Some(percent) = self.parse_defrag_output(&output_str) {
                                result_info["fragmentation_percent"] = json!(percent);
                                result_info["status"] = json!("Analyzed");
                            } else {
                                result_info["status"] =
                                    json!("Analysis failed: Could not parse output");
                            }
                        } else {
                            let error_str = crate::security::decode_windows_output(&output.stderr);
                            result_info["status"] =
                                json!(format!("Analysis failed: {}", error_str));
                        }
                    }
                    Err(e) => {
                        result_info["status"] = json!(format!("Execution failed: {}", e));
                    }
                }
                fragmentation_results.push(result_info);
            }
        }

        Ok(json!(fragmentation_results))
    }

    fn parse_defrag_output(&self, output: &str) -> Option<u32> {
        // Look for patterns like:
        // "Total fragmented space = 20%"
        // "Total fragmented space = 20 %"
        // "Current fragmentation = 15%"
        for line in output.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("fragmented space") || line_lower.contains("fragmentation") {
                // Extract number before % sign
                if let Some(percent_pos) = line.find('%') {
                    // Look backwards from % to find the number
                    let before_percent = &line[..percent_pos];
                    // Find the last number in the string
                    let num_str: String = before_percent
                        .chars()
                        .rev()
                        .take_while(|c| c.is_ascii_digit() || c.is_whitespace())
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    if let Ok(num) = num_str.trim().parse::<u32>() {
                        return Some(num);
                    }
                }
            }
        }
        None
    }

    pub fn get_native_services(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let results = wmi_con
            .query("SELECT Name, DisplayName, State, StartMode, PathName FROM Win32_Service")?;

        let services: Vec<Value> = results
            .into_iter()
            .map(|r| {
                let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                Value::Object(obj)
            })
            .collect();

        Ok(Value::Array(services))
    }

    pub fn get_battery_report(&self) -> Result<Value> {
        let temp_file = std::env::temp_dir().join("wfdiag_battery.html");
        let temp_path = temp_file.to_string_lossy();

        let output =
            Self::execute_secure_command("powercfg", &["/batteryreport", "/output", &temp_path])?;

        if output.status.success() && temp_file.exists() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let html_content = fs::read_to_string(&temp_file)?;
            let _ = fs::remove_file(&temp_file);

            // Parse the HTML to extract battery information safely
            let battery_info = self.parse_battery_html(&html_content)?;

            Ok(json!({
                "battery_summary": battery_info,
                "html_content": html_content,
                "parsed_data": true
            }))
        } else {
            Err(anyhow::anyhow!("Failed to generate battery report"))
        }
    }

    /// Parse battery report HTML and extract key information using simple string parsing
    fn parse_battery_html(&self, html_content: &str) -> Result<Value> {
        let mut battery_info = json!({
            "report_generated": true,
            "batteries": []
        });

        // Simple regex-free parsing: extract text between td tags
        fn extract_table_cells(html: &str) -> Vec<Vec<String>> {
            let mut tables = Vec::new();
            let mut current_pos = 0;

            while let Some(table_start) = html[current_pos..].find("<table") {
                let abs_start = current_pos + table_start;
                if let Some(table_end) = html[abs_start..].find("</table>") {
                    let table_html = &html[abs_start..abs_start + table_end];
                    let mut rows = Vec::new();
                    let mut row_pos = 0;

                    while let Some(tr_start) = table_html[row_pos..].find("<tr") {
                        let abs_tr_start = row_pos + tr_start;
                        if let Some(tr_end) = table_html[abs_tr_start..].find("</tr>") {
                            let row_html = &table_html[abs_tr_start..abs_tr_start + tr_end];
                            let mut cells = Vec::new();
                            let mut cell_pos = 0;

                            while let Some(td_start) = row_html[cell_pos..].find("<td") {
                                let abs_td_start = cell_pos + td_start;
                                if let Some(tag_end) = row_html[abs_td_start..].find('>') {
                                    let content_start = abs_td_start + tag_end + 1;
                                    if let Some(td_end) = row_html[content_start..].find("</td>") {
                                        let cell_content =
                                            &row_html[content_start..content_start + td_end];
                                        // Strip HTML tags and decode entities
                                        let text = cell_content
                                            .replace("<br>", " ")
                                            .replace("<br/>", " ")
                                            .replace("&nbsp;", " ")
                                            .replace("&amp;", "&")
                                            .split('<')
                                            .filter_map(|s| s.split('>').next_back())
                                            .collect::<Vec<_>>()
                                            .join("")
                                            .trim()
                                            .to_string();
                                        cells.push(text);
                                        cell_pos = content_start + td_end;
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if !cells.is_empty() {
                                rows.push(cells);
                            }
                            row_pos = abs_tr_start + tr_end;
                        } else {
                            break;
                        }
                    }
                    if !rows.is_empty() {
                        tables.push(rows);
                    }
                    current_pos = abs_start + table_end;
                } else {
                    break;
                }
            }
            tables.into_iter().flatten().collect()
        }

        let all_rows = extract_table_cells(html_content);

        // Extract battery properties (look for key-value pairs)
        let mut battery_data = Vec::new();
        let mut capacity_history = Vec::new();

        for row in &all_rows {
            if row.len() >= 2 {
                let key = row[0].to_lowercase();
                // Battery info patterns
                if key.contains("manufacturer")
                    || key.contains("chemistry")
                    || key.contains("design capacity")
                    || key.contains("full charge")
                    || key.contains("serial")
                    || key.contains("cycle")
                {
                    battery_data.push(json!({
                        "property": row[0].clone(),
                        "value": row[1].clone()
                    }));
                }
                // Capacity history patterns (has mWh values)
                if row.len() >= 3 && (row[1].contains("mWh") || row[2].contains("mWh")) {
                    capacity_history.push(json!({
                        "period": row[0].clone(),
                        "full_charge_capacity": row[1].clone(),
                        "design_capacity": if row.len() > 2 { row[2].clone() } else { String::new() }
                    }));
                }
            }
        }

        if !battery_data.is_empty() {
            battery_info["batteries"] = json!(battery_data);
        }
        if !capacity_history.is_empty() {
            battery_info["battery_capacity_history"] = json!(capacity_history);
        }

        // Calculate battery health
        if let Some(latest) = battery_info["battery_capacity_history"]
            .as_array()
            .and_then(|h| h.first())
            && let (Some(full_charge), Some(design_capacity)) = (
                latest["full_charge_capacity"].as_str(),
                latest["design_capacity"].as_str(),
            )
            && let (Ok(full_mwh), Ok(design_mwh)) = (
                self.extract_mwh_value(full_charge),
                self.extract_mwh_value(design_capacity),
            )
            && design_mwh > 0.0
        {
            let health_percentage = (full_mwh / design_mwh * 100.0).round();
            battery_info["battery_health_percentage"] = json!(health_percentage);
            battery_info["battery_health_status"] = json!(if health_percentage >= 80.0 {
                "Good"
            } else if health_percentage >= 60.0 {
                "Fair"
            } else {
                "Poor"
            });
        }

        Ok(battery_info)
    }

    /// Extract mWh value from capacity string (e.g., "45,000 mWh" -> 45000.0)
    fn extract_mwh_value(&self, capacity_str: &str) -> Result<f64, std::num::ParseFloatError> {
        let cleaned = capacity_str
            .replace(",", "")
            .replace(" mWh", "")
            .replace(" Wh", "")
            .trim()
            .to_string();

        cleaned.parse::<f64>()
    }

    pub fn get_minidumps(&self) -> Result<Value> {
        let minidump_path = Path::new("C:\\Windows\\Minidump");

        if !minidump_path.exists() {
            return Ok(json!({
                "dumps": [],
                "message": "No minidump directory found",
                "can_copy": false
            }));
        }

        let mut dumps = Vec::new();
        let start_time = std::time::Instant::now();
        let search_timeout = std::time::Duration::from_secs(5);

        if let Ok(entries) = fs::read_dir(minidump_path) {
            let mut all_entries = Vec::new();

            // Collect entries first to sort them
            for entry in entries.filter_map(Result::ok) {
                if start_time.elapsed() > search_timeout {
                    break;
                }
                all_entries.push(entry);
            }

            // Sort by modification time (newest first)
            // Files with inaccessible metadata sort to the end (oldest position) - this is intentional
            // as we want to prioritize recent, readable minidumps over potentially corrupted ones
            all_entries.sort_by(|a, b| {
                let a_time = a
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::UNIX_EPOCH);
                let b_time = b
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::UNIX_EPOCH);
                b_time.cmp(&a_time)
            });

            // Filter to .dmp files BEFORE taking the 10 most recent, so stray non-.dmp
            // entries don't consume slots and cause real minidumps to be under-reported.
            for entry in all_entries
                .into_iter()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("dmp"))
                .take(10)
            {
                if let Ok(metadata) = entry.metadata() {
                    dumps.push(json!({
                        "filename": entry.file_name().to_string_lossy(),
                        "size": metadata.len(),
                        "created": metadata.created().ok().map(|t| {
                            match t.duration_since(std::time::UNIX_EPOCH) {
                                Ok(d) => d.as_secs(),
                                Err(_) => 0
                            }
                        }).unwrap_or(0),
                        "path": entry.path().to_string_lossy()
                    }));
                }
            }
        }

        // Check if Desktop\Minidumps exists
        let desktop_minidumps = self.get_desktop_minidumps_path();
        let desktop_minidumps_exists = desktop_minidumps.as_ref().is_some_and(|p| p.exists());

        Ok(json!({
            "dumps": dumps,
            "count": dumps.len(),
            "path": minidump_path.to_string_lossy(),
            "can_copy": !dumps.is_empty(),
            "desktop_path": desktop_minidumps.map(|p| p.to_string_lossy().to_string()),
            "desktop_path_exists": desktop_minidumps_exists
        }))
    }

    /// Get the Desktop\Minidumps path for the current user
    fn get_desktop_minidumps_path(&self) -> Option<PathBuf> {
        dirs::desktop_dir().map(|desktop_path| desktop_path.join("Minidumps"))
    }

    /// Copy minidumps to Desktop\Minidumps for easy sharing on forums
    pub fn copy_minidumps_to_desktop(&self) -> Result<Value> {
        let minidump_path = Path::new("C:\\Windows\\Minidump");

        if !minidump_path.exists() {
            return Ok(json!({
                "success": false,
                "message": "No minidump directory found",
                "copied_files": []
            }));
        }

        // Get Desktop\Minidumps path
        let desktop_minidumps = match self.get_desktop_minidumps_path() {
            Some(path) => path,
            None => {
                return Ok(json!({
                    "success": false,
                    "message": "Could not determine Desktop path",
                    "copied_files": []
                }));
            }
        };

        // Create Desktop\Minidumps directory if it doesn't exist
        if !desktop_minidumps.exists()
            && let Err(e) = fs::create_dir_all(&desktop_minidumps)
        {
            return Ok(json!({
                "success": false,
                "message": format!("Failed to create Desktop\\Minidumps directory: {}", e),
                "copied_files": []
            }));
        }

        let mut copied_files = Vec::new();
        let mut errors = Vec::new();
        let mut total_copied = 0;

        // Copy all .dmp files
        if let Ok(entries) = fs::read_dir(minidump_path) {
            for entry in entries.filter_map(Result::ok) {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("dmp") {
                    let source_file = entry.path();
                    let filename = entry.file_name();
                    let dest_file = desktop_minidumps.join(&filename);

                    match fs::copy(&source_file, &dest_file) {
                        Ok(bytes_copied) => {
                            total_copied += 1;
                            copied_files.push(json!({
                                "filename": filename.to_string_lossy(),
                                "source": source_file.to_string_lossy(),
                                "destination": dest_file.to_string_lossy(),
                                "size": bytes_copied
                            }));
                        }
                        Err(e) => {
                            errors.push(format!(
                                "Failed to copy {}: {}",
                                filename.to_string_lossy(),
                                e
                            ));
                        }
                    }
                }
            }
        }

        Ok(json!({
            "success": total_copied > 0,
            "message": if total_copied > 0 {
                format!("Successfully copied {} minidump file(s) to Desktop\\Minidumps", total_copied)
            } else if !errors.is_empty() {
                format!("Failed to copy minidumps: {}", errors.join(", "))
            } else {
                "No minidump files found to copy".to_string()
            },
            "copied_files": copied_files,
            "destination_path": desktop_minidumps.to_string_lossy(),
            "total_copied": total_copied,
            "errors": errors
        }))
    }

    pub fn get_store_apps(&self) -> Result<Value> {
        // Use Windows PackageManager API to enumerate installed packages
        use windows::Management::Deployment::PackageManager;
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        // Initialize COM and WinRT on this thread (required for Tokio worker threads)
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let mut apps = Vec::new();

        // Create PackageManager and enumerate packages
        let package_manager = PackageManager::new().map_err(|e| {
            anyhow::anyhow!(
                "Failed to create PackageManager: {} (0x{:08X})",
                e,
                e.code().0
            )
        })?;

        // Get packages for current user (empty string = current user)
        let packages = package_manager
            .FindPackages()
            .map_err(|e| anyhow::anyhow!("Failed to enumerate packages: {}", e))?;

        for package in packages {
            // Skip framework packages
            if let Ok(is_framework) = package.IsFramework()
                && is_framework
            {
                continue;
            }

            let mut app_info = serde_json::Map::new();

            // Get package ID info
            if let Ok(id) = package.Id() {
                if let Ok(name) = id.Name() {
                    let name_str = name.to_string();
                    // Skip system framework packages
                    if name_str.contains("Microsoft.NET")
                        || name_str.contains("Microsoft.VCLibs")
                        || name_str.contains("Microsoft.UI.Xaml")
                    {
                        continue;
                    }
                    app_info.insert("Name".to_string(), json!(name_str));
                }
                if let Ok(version) = id.Version() {
                    app_info.insert(
                        "Version".to_string(),
                        json!(format!(
                            "{}.{}.{}.{}",
                            version.Major, version.Minor, version.Build, version.Revision
                        )),
                    );
                }
                if let Ok(publisher) = id.Publisher() {
                    app_info.insert("Publisher".to_string(), json!(publisher.to_string()));
                }
                if let Ok(full_name) = id.FullName() {
                    app_info.insert("PackageFullName".to_string(), json!(full_name.to_string()));
                }
                // Architecture is available via ProcessorArchitecture which requires additional features
                // Skip for now as it's not critical
            }

            // Get display name if available
            if let Ok(display_name) = package.DisplayName() {
                app_info.insert("DisplayName".to_string(), json!(display_name.to_string()));
            }

            if app_info.contains_key("Name") {
                apps.push(Value::Object(app_info));
            }
        }

        if apps.is_empty() {
            return Err(anyhow::anyhow!("No store apps found"));
        }

        Ok(json!(apps))
    }

    pub fn get_performance_data(&self) -> Result<Value> {
        use windows::Win32::System::SystemInformation::{
            GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
        };

        let mut perf_data = json!({});

        // Get memory info using GlobalMemoryStatusEx (native Windows API)
        unsafe {
            let mut mem_status = MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };

            if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
                let total_mb = mem_status.ullTotalPhys / 1024 / 1024;
                let avail_mb = mem_status.ullAvailPhys / 1024 / 1024;
                let used_percent = mem_status.dwMemoryLoad;

                perf_data["memory_performance"] = json!({
                    "TotalMBytes": total_mb,
                    "AvailableMBytes": avail_mb,
                    "UsedPercent": used_percent,
                    "TotalVirtualMBytes": mem_status.ullTotalVirtual / 1024 / 1024,
                    "AvailableVirtualMBytes": mem_status.ullAvailVirtual / 1024 / 1024,
                    "TotalPageFileMBytes": mem_status.ullTotalPageFile / 1024 / 1024,
                    "AvailablePageFileMBytes": mem_status.ullAvailPageFile / 1024 / 1024,
                });
            }

            // Get system info for CPU
            let mut sys_info = SYSTEM_INFO::default();
            GetSystemInfo(&mut sys_info);

            perf_data["cpu_performance"] = json!({
                "NumberOfLogicalProcessors": sys_info.dwNumberOfProcessors,
                "ProcessorArchitecture": match sys_info.Anonymous.Anonymous.wProcessorArchitecture.0 {
                    0 => "x86",
                    5 => "ARM",
                    6 => "IA64",
                    9 => "x64",
                    12 => "ARM64",
                    _ => "Unknown"
                },
                "ProcessorLevel": sys_info.wProcessorLevel,
                "PageSize": sys_info.dwPageSize,
            });
        }

        // Get CPU name and load from WMI (fallback for detailed info)
        if let Ok(wmi_con) = WmiConnection::new() {
            if let Ok(cpu_results) = wmi_con.query(
                "SELECT Name, LoadPercentage, NumberOfCores, MaxClockSpeed FROM Win32_Processor",
            ) && let Some(result) = cpu_results.into_iter().next()
            {
                let cpu_info: serde_json::Map<String, Value> = result.into_iter().collect();
                // Merge with existing cpu_performance
                if let Some(existing) = perf_data.get_mut("cpu_performance")
                    && let Some(obj) = existing.as_object_mut()
                {
                    for (k, v) in cpu_info {
                        obj.insert(k, v);
                    }
                }
            }

            // Get disk info from WMI
            if let Ok(disk_results) = wmi_con
                .query("SELECT DeviceID, Size, FreeSpace FROM Win32_LogicalDisk WHERE DriveType=3")
            {
                let disks: Vec<Value> = disk_results
                    .into_iter()
                    .map(|r| {
                        let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                        Value::Object(obj)
                    })
                    .collect();
                if !disks.is_empty() {
                    perf_data["disk_performance"] = json!(disks);
                }
            }
        }

        Ok(perf_data)
    }

    pub fn get_scheduled_tasks(&self) -> Result<Value> {
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        };
        use windows::Win32::System::TaskScheduler::{
            ITaskFolder, ITaskService, TASK_STATE_DISABLED, TASK_STATE_QUEUED, TASK_STATE_READY,
            TASK_STATE_RUNNING, TaskScheduler,
        };
        use windows::Win32::System::Variant::VARIANT;
        use windows::core::BSTR;

        let mut tasks = Vec::new();

        unsafe {
            // Initialize COM
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            // Create TaskScheduler instance
            let task_service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| anyhow::anyhow!("Failed to create TaskScheduler: {}", e))?;

            // Connect to the task service (local, current user)
            let empty_var = VARIANT::default();
            task_service
                .Connect(&empty_var, &empty_var, &empty_var, &empty_var)
                .map_err(|e| anyhow::anyhow!("Failed to connect to TaskScheduler: {}", e))?;

            // Get root folder
            let root_folder: ITaskFolder = task_service
                .GetFolder(&BSTR::from("\\"))
                .map_err(|e| anyhow::anyhow!("Failed to get root folder: {}", e))?;

            // Recursive function to enumerate tasks
            fn enumerate_folder(folder: &ITaskFolder, tasks: &mut Vec<Value>, depth: u32) {
                if depth > 3 {
                    return;
                } // Limit recursion depth

                unsafe {
                    // Get tasks in this folder
                    if let Ok(task_collection) = folder.GetTasks(0)
                        && let Ok(count) = task_collection.Count()
                    {
                        for i in 1..=count {
                            let idx = VARIANT::from(i);
                            if let Ok(task) = task_collection.get_Item(&idx) {
                                let mut task_info = serde_json::Map::new();

                                if let Ok(name) = task.Name() {
                                    task_info
                                        .insert("TaskName".to_string(), json!(name.to_string()));
                                }
                                if let Ok(path) = task.Path() {
                                    task_info
                                        .insert("TaskPath".to_string(), json!(path.to_string()));
                                }
                                if let Ok(state) = task.State() {
                                    let state_str = match state {
                                        TASK_STATE_DISABLED => "Disabled",
                                        TASK_STATE_QUEUED => "Queued",
                                        TASK_STATE_READY => "Ready",
                                        TASK_STATE_RUNNING => "Running",
                                        _ => "Unknown",
                                    };
                                    task_info.insert("State".to_string(), json!(state_str));
                                }
                                if let Ok(enabled) = task.Enabled() {
                                    task_info
                                        .insert("Enabled".to_string(), json!(enabled.as_bool()));
                                }
                                if let Ok(last_run) = task.LastRunTime() {
                                    task_info.insert("LastRunTime".to_string(), json!(last_run));
                                }
                                if let Ok(next_run) = task.NextRunTime() {
                                    task_info.insert("NextRunTime".to_string(), json!(next_run));
                                }

                                if task_info.contains_key("TaskName") {
                                    tasks.push(Value::Object(task_info));
                                }
                            }
                        }
                    }

                    // Enumerate subfolders
                    if let Ok(folders) = folder.GetFolders(0)
                        && let Ok(count) = folders.Count()
                    {
                        for i in 1..=count {
                            let idx = VARIANT::from(i);
                            if let Ok(subfolder) = folders.get_Item(&idx) {
                                enumerate_folder(&subfolder, tasks, depth + 1);
                            }
                        }
                    }
                }
            }

            enumerate_folder(&root_folder, &mut tasks, 0);
        }

        // Filter and limit
        let filtered: Vec<Value> = tasks
            .into_iter()
            .filter(|t| {
                // Exclude disabled tasks and some noisy system tasks
                if let Some(state) = t.get("State").and_then(|s| s.as_str()) {
                    state != "Disabled"
                } else {
                    true
                }
            })
            .take(200)
            .collect();

        if filtered.is_empty() {
            return Err(anyhow::anyhow!("No scheduled tasks found"));
        }

        Ok(json!(filtered))
    }

    pub fn get_windows_update_history(&self) -> Result<Value> {
        let wmi_con = WmiConnection::new()?;
        let mut update_info = json!({});

        // Get installed hotfixes via WMI (native, no PowerShell)
        if let Ok(hotfix_results) = wmi_con.query("SELECT HotFixID, Description, InstalledOn, InstalledBy, Caption FROM Win32_QuickFixEngineering") {
            let hotfixes: Vec<Value> = hotfix_results
                .into_iter()
                .map(|r| {
                    let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                    Value::Object(obj)
                })
                .collect();
            update_info["summary"] = Self::windows_update_hotfix_summary(&hotfixes);
            update_info["installed_updates"] = json!(hotfixes);
            // Also provide as hotfix_details for frontend compatibility
            update_info["hotfix_details"] = json!(hotfixes);
        }

        // Try to get update history from Windows Update namespace
        if let Ok(wmi_update) =
            WmiConnection::with_namespace(r"root\CCM\SoftwareUpdates\UpdatesStore")
            && let Ok(updates) = wmi_update.query("SELECT * FROM CCM_UpdateStatus")
        {
            let update_history: Vec<Value> = updates
                .into_iter()
                .map(|r| {
                    let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                    Value::Object(obj)
                })
                .collect();
            if !update_history.is_empty() {
                update_info["update_history"] = json!(update_history);
            }
        }

        Ok(update_info)
    }

    fn windows_update_hotfix_summary(hotfixes: &[Value]) -> Value {
        let latest = hotfixes
            .iter()
            .filter_map(|hotfix| {
                let key = hotfix
                    .get("InstalledOn")
                    .and_then(Value::as_str)
                    .and_then(Self::parse_hotfix_date_key)?;
                Some((key, hotfix.clone()))
            })
            .max_by_key(|(key, _)| *key)
            .map(|(_, hotfix)| hotfix);

        json!({
            "installed_update_count": hotfixes.len(),
            "data_source": "Win32_QuickFixEngineering installed hotfix records",
            "latest_installed_update": latest,
        })
    }

    fn parse_hotfix_date_key(value: &str) -> Option<u32> {
        let mut parts = value.split('/');
        let month = parts.next()?.parse::<u32>().ok()?;
        let day = parts.next()?.parse::<u32>().ok()?;
        let year = parts.next()?.parse::<u32>().ok()?;
        (parts.next().is_none() && (1..=12).contains(&month) && (1..=31).contains(&day))
            .then_some(year * 10_000 + month * 100 + day)
    }

    pub fn get_driver_verifier(&self) -> Result<Value> {
        // Run verifier command to get current settings
        let output = Self::execute_secure_command("verifier", &["/querysettings"])?;

        if output.status.success() {
            let output_str = crate::security::decode_windows_output(&output.stdout);

            // Parse verifier flags
            let mut verifier_flags: u32 = 0;
            let mut enabled_flags = Vec::new();
            let mut verified_drivers = Vec::new();
            let mut boot_mode = String::new();

            for line in output_str.lines() {
                let line = line.trim();

                // Parse "Verifier Flags: 0x00000000"
                if line.starts_with("Verifier Flags:")
                    && let Some(hex) = line.split("0x").nth(1)
                    && let Ok(val) = u32::from_str_radix(hex.trim(), 16)
                {
                    verifier_flags = val;
                }

                // Parse enabled flags marked with [X]
                if line.starts_with("[X]")
                    && let Some(flag_desc) = line.strip_prefix("[X]").map(|s| s.trim())
                {
                    enabled_flags.push(flag_desc.to_string());
                }

                // Parse boot mode
                if line.starts_with("Boot Mode:") {
                    boot_mode = line.replace("Boot Mode:", "").trim().to_string();
                }

                // Parse verified drivers section
                if line.starts_with("Verified Drivers:") {
                    let drivers_part = line.replace("Verified Drivers:", "").trim().to_string();
                    if drivers_part != "None" && !drivers_part.is_empty() {
                        verified_drivers.push(drivers_part);
                    }
                }
            }

            let is_enabled = verifier_flags != 0 || !enabled_flags.is_empty();
            let has_drivers = !verified_drivers.is_empty();

            Ok(json!({
                "enabled": is_enabled,
                "status": if is_enabled {
                    if has_drivers { "Active - Monitoring drivers" } else { "Active - No drivers specified" }
                } else {
                    "Inactive"
                },
                "verifier_flags": format!("0x{:08X}", verifier_flags),
                "enabled_flags": enabled_flags,
                "verified_drivers": verified_drivers,
                "boot_mode": boot_mode,
                "raw_output": output_str.to_string()
            }))
        } else {
            // Verifier might require admin privileges
            let error_str = crate::security::decode_windows_output(&output.stderr);
            Ok(json!({
                "enabled": false,
                "status": "Unable to query",
                "error": "Failed to query driver verifier settings. Administrator privileges may be required.",
                "raw_error": error_str
            }))
        }
    }

    /// Secure command execution with validation
    fn execute_secure_command(program: &str, args: &[&str]) -> Result<std::process::Output> {
        let executor = crate::security::SecureCommandExecutor::new();
        executor
            .execute_command(program, args)
            .map_err(|e| anyhow::anyhow!("Security validation failed: {}", e))
    }
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let start = xml.find(&open)?;
    let content_start = xml[start..].find('>')? + start + 1;
    let close = format!("</{}>", tag);
    let content_end = xml[content_start..].find(&close)? + content_start;
    Some(xml_unescape(xml[content_start..content_end].trim()))
}

fn xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let start = xml.find(&open)?;
    let tag_end = xml[start..].find('>')? + start;
    xml_attr_from_fragment(&xml[start..tag_end], attr)
}

fn xml_attr_from_fragment(fragment: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = fragment.find(&needle)? + needle.len();
    let end = fragment[start..].find('"')? + start;
    Some(xml_unescape(&fragment[start..end]))
}

fn xml_event_data(xml: &str) -> serde_json::Map<String, Value> {
    let mut data = serde_json::Map::new();
    let mut rest = xml;
    let mut fallback_index = 1usize;

    while let Some(data_pos) = rest.find("<Data") {
        rest = &rest[data_pos..];
        let Some(tag_end) = rest.find('>') else {
            break;
        };
        let tag_fragment = &rest[..tag_end];
        let name = xml_attr_from_fragment(tag_fragment, "Name").unwrap_or_else(|| {
            let name = format!("Data{}", fallback_index);
            fallback_index += 1;
            name
        });

        if tag_fragment.ends_with('/') {
            data.insert(name, json!(""));
            rest = &rest[tag_end + 1..];
            continue;
        }

        let value_start = tag_end + 1;
        let Some(value_end) = rest[value_start..].find("</Data>") else {
            break;
        };
        let value = xml_unescape(rest[value_start..value_start + value_end].trim());
        data.insert(name, json!(value));
        rest = &rest[value_start + value_end + "</Data>".len()..];
    }

    data
}

fn event_data_summary(data: &serde_json::Map<String, Value>) -> Option<String> {
    let summary = data
        .iter()
        .filter_map(|(key, value)| {
            let text = value.as_str()?.trim();
            (!text.is_empty()).then(|| format!("{}={}", key, text))
        })
        .take(6)
        .collect::<Vec<_>>()
        .join("; ");
    if summary.is_empty() {
        None
    } else {
        Some(summary.chars().take(300).collect())
    }
}

fn xml_unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
