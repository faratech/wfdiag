use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use wmi::WMIConnection;
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
// Performance counter imports removed - not used in current implementation
use winreg::enums::*;
use winreg::RegKey;
use scraper::{Html, Selector};

pub struct NativeDiagnostics;

impl NativeDiagnostics {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn run_wmi_query(&self, class_name: &str, namespace: Option<&str>) -> Result<Value> {
        let wmi_con = if let Some(ns) = namespace {
            WMIConnection::with_namespace_path(ns)?
        } else {
            WMIConnection::new()?
        };
        let results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query(&format!("SELECT * FROM {}", class_name))?;
        
        let mut json_results = Vec::new();
        for result in results {
            let mut json_obj = serde_json::Map::new();
            for (key, value) in result {
                json_obj.insert(key, self.variant_to_json(value));
            }
            json_results.push(Value::Object(json_obj));
        }
        
        Ok(Value::Array(json_results))
    }

    fn variant_to_json(&self, variant: wmi::Variant) -> Value {
        match variant {
            wmi::Variant::String(s) => json!(s),
            wmi::Variant::I4(i) => json!(i),
            wmi::Variant::I8(i) => json!(i),
            wmi::Variant::UI4(u) => json!(u),
            wmi::Variant::UI8(u) => json!(u),
            wmi::Variant::Bool(b) => json!(b),
            wmi::Variant::Array(arr) => {
                let json_arr: Vec<Value> = arr.into_iter().map(|v| self.variant_to_json(v)).collect();
                json!(json_arr)
            }
            _ => json!(null),
        }
    }

    pub fn get_native_disk_space(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT * FROM Win32_LogicalDisk WHERE DriveType=3")?;
        
        let mut drives = Vec::new();
        for result in results {
            let mut drive_info = serde_json::Map::new();
            
            // Include all WMI data
            for (key, value) in result {
                drive_info.insert(key.clone(), self.variant_to_json(value));
            }
            
            drives.push(Value::Object(drive_info));
        }
        
        Ok(Value::Array(drives))
    }

    pub fn get_native_network_adapters(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let config_results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT * FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled=TRUE")?;
        let adapter_results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT * FROM Win32_NetworkAdapter")?;
        
        let mut adapters = Vec::new();
        
        for config in config_results {
            let mut adapter_info = serde_json::Map::new();
            
            // Get the index to match with adapter
            let index = config.get("Index").and_then(|v| {
                if let wmi::Variant::UI4(i) = v {
                    Some(*i)
                } else {
                    None
                }
            });
            
            // Add all configuration data
            for (key, value) in &config {
                adapter_info.insert(key.clone(), self.variant_to_json(value.clone()));
            }
            
            // Find matching adapter info
            if let Some(idx) = index {
                for adapter in &adapter_results {
                    if let Some(wmi::Variant::UI4(adapter_idx)) = adapter.get("DeviceID") {
                        if *adapter_idx == idx {
                            // Add adapter-specific info
                            for (key, value) in adapter {
                                if !adapter_info.contains_key(key) {
                                    adapter_info.insert(key.clone(), self.variant_to_json(value.clone()));
                                }
                            }
                            break;
                        }
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
        let wmi_con = WMIConnection::new()?;
        let os_results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT * FROM Win32_OperatingSystem")?;
        let comp_results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT * FROM Win32_ComputerSystem")?;
        
        let mut info = json!({});
        
        // Add OS information
        if let Some(os) = os_results.first() {
            let mut os_info = json!({});
            for (key, value) in os {
                os_info[key] = self.variant_to_json(value.clone());
            }
            
            // Parse Windows version details
            if let Some(wmi::Variant::String(caption)) = os.get("Caption") {
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
                comp_info[key] = self.variant_to_json(value.clone());
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
            if let Ok(cv_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion") {
                let mut version_info = json!({});
                
                // Read various version fields
                for field in &["ProductName", "DisplayVersion", "CurrentBuild", "UBR", "EditionID", "CompositionEditionID"] {
                    if let Ok(value) = cv_key.get_value::<String, _>(field) {
                        version_info[field] = json!(value);
                    }
                }
                
                info["windows_version_details"] = version_info;
            }
            
            // Get hardware info
            if let Ok(hw_key) = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0") {
                if let Ok(cpu_name) = hw_key.get_value::<String, _>("ProcessorNameString") {
                    info["cpu_name"] = json!(cpu_name.trim());
                }
            }
        }
        
        Ok(info)
    }

    pub fn get_drivers(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let mut drivers = Vec::new();
        
        // Get PnP signed drivers (main source on modern Windows)
        match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT Name, DeviceName, DriverVersion, DriverDate, DriverProviderName, DeviceClass, IsSigned FROM Win32_PnPSignedDriver"
        ) {
            Ok(pnp_results) => {
                for result in pnp_results {
                    let mut driver_info = serde_json::Map::new();
                    for (key, value) in result {
                        driver_info.insert(key, self.variant_to_json(value));
                    }
                    drivers.push(Value::Object(driver_info));
                }
            }
            Err(e) => {
                eprintln!("Failed to query Win32_PnPSignedDriver: {}", e);
            }
        }
        
        // Try to get legacy VxD drivers (might not exist on modern systems)
        match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT Name, DriverVersion, DriverDate, DeviceName FROM Win32_DriverVXD"
        ) {
            Ok(vxd_results) => {
                for result in vxd_results {
                    let mut driver_info = serde_json::Map::new();
                    driver_info.insert("Type".to_string(), json!("VxD"));
                    for (key, value) in result {
                        driver_info.insert(key, self.variant_to_json(value));
                    }
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
        let wmi_con = WMIConnection::new()?;
        
        let mut all_events = Vec::new();
        
        // Query System events
        match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT TimeGenerated, Type, SourceName, EventCode, Message FROM Win32_NTLogEvent WHERE Logfile='System' AND Type='Error'"
        ) {
            Ok(results) => {
                for result in results.into_iter().take(50) {
                    let mut event_info = serde_json::Map::new();
                    event_info.insert("LogFile".to_string(), json!("System"));
                    for (key, value) in result {
                        event_info.insert(key, self.variant_to_json(value));
                    }
                    all_events.push(Value::Object(event_info));
                }
            }
            Err(e) => {
                eprintln!("Failed to query System events: {}", e);
            }
        }
        
        // Query Application events
        match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT TimeGenerated, Type, SourceName, EventCode, Message FROM Win32_NTLogEvent WHERE Logfile='Application' AND Type='Error'"
        ) {
            Ok(results) => {
                for result in results.into_iter().take(50) {
                    let mut event_info = serde_json::Map::new();
                    event_info.insert("LogFile".to_string(), json!("Application"));
                    for (key, value) in result {
                        event_info.insert(key, self.variant_to_json(value));
                    }
                    all_events.push(Value::Object(event_info));
                }
            }
            Err(e) => {
                eprintln!("Failed to query Application events: {}", e);
            }
        }
        
        Ok(Value::Array(all_events))
    }

    pub fn get_installed_programs(&self) -> Result<Value> {
        let mut programs = Vec::new();
        
        // Check both 32-bit and 64-bit registry locations
        let paths = vec![
            (HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
            (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
            (HKEY_CURRENT_USER, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        ];
        
        for (hkey, path) in paths {
            if let Ok(key) = RegKey::predef(hkey).open_subkey(path) {
                for subkey_name in key.enum_keys().filter_map(Result::ok) {
                    if let Ok(subkey) = key.open_subkey(&subkey_name) {
                        let mut program_info = serde_json::Map::new();
                        
                        // Read common fields
                        for field in &["DisplayName", "DisplayVersion", "Publisher", "InstallDate", "UninstallString", "InstallLocation"] {
                            if let Ok(value) = subkey.get_value::<String, _>(field) {
                                if !value.is_empty() {
                                    program_info.insert(field.to_string(), json!(value));
                                }
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
        let wmi_con = WMIConnection::new()?;
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
        if let Ok(video_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_VideoController"
        ) {
            let mut video_info = Vec::new();
            for result in video_results {
                let mut controller = json!({});
                for (key, value) in result {
                    controller[key] = self.variant_to_json(value);
                }
                video_info.push(controller);
            }
            info["video_controllers"] = json!(video_info);
        }
        
        // Get sound device info
        if let Ok(sound_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_SoundDevice"
        ) {
            let mut sound_info = Vec::new();
            for result in sound_results {
                let mut device = json!({});
                for (key, value) in result {
                    device[key] = self.variant_to_json(value);
                }
                sound_info.push(device);
            }
            info["sound_devices"] = json!(sound_info);
        }
        
        // Try to get DirectX version from registry
        {
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(dx_key) = hklm.open_subkey("SOFTWARE\\Microsoft\\DirectX") {
                if let Ok(version) = dx_key.get_value::<String, _>("Version") {
                    info["directx_version"] = json!(version);
                }
            }
        }
        
        Ok(info)
    }

    pub fn run_chkdsk(&self) -> Result<Value> {
        // First try to run chkdsk in read-only mode using secure execution
        let output = Self::execute_secure_command("chkdsk", &["C:"])?;
        
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            
            // Parse chkdsk output
            let mut check_info = json!({
                "raw_output": output_str.to_string(),
                "status": "Unknown",
                "errors_found": false
            });
            
            // Check for common chkdsk responses
            if output_str.contains("Windows has scanned the file system and found no problems") {
                check_info["status"] = json!("Healthy");
                check_info["message"] = json!("No file system errors found");
                check_info["errors_found"] = json!(false);
            } else if output_str.contains("found problems") || output_str.contains("errors found") {
                check_info["status"] = json!("Errors Found");
                check_info["message"] = json!("File system errors detected");
                check_info["errors_found"] = json!(true);
            } else if output_str.contains("scan completed successfully") {
                check_info["status"] = json!("Scan Completed");
                check_info["message"] = json!("Scan completed successfully");
            }
            
            // Also get disk info from WMI
            let wmi_con = WMIConnection::new()?;
            if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
                "SELECT * FROM Win32_DiskDrive"
            ) {
                let mut disk_info = Vec::new();
                for result in results {
                    let mut info = serde_json::Map::new();
                    for (key, value) in result {
                        info.insert(key, self.variant_to_json(value));
                    }
                    disk_info.push(Value::Object(info));
                }
                check_info["disk_drives"] = json!(disk_info);
            }
            
            Ok(check_info)
        } else {
            let error_str = String::from_utf8_lossy(&output.stderr);
            
            // Check if it's an elevation error
            if error_str.contains("requires elevated") || error_str.contains("Access is denied") {
                // If we can't run chkdsk, at least get disk info
                let wmi_con = WMIConnection::new()?;
                let results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query(
                    "SELECT * FROM Win32_DiskDrive"
                )?;
                
                let mut disk_info = Vec::new();
                for result in results {
                    let mut info = serde_json::Map::new();
                    for (key, value) in result {
                        info.insert(key, self.variant_to_json(value));
                    }
                    disk_info.push(Value::Object(info));
                }
                
                Ok(json!({
                    "note": "Full chkdsk scan requires admin privileges. Showing disk status from WMI.",
                    "suggestion": "Run as administrator for full disk scan",
                    "disk_drives": disk_info,
                    "raw_error": error_str.to_string()
                }))
            } else {
                Err(anyhow::anyhow!("Chkdsk failed: {}", error_str))
            }
        }
    }

    pub fn run_dism_health(&self) -> Result<Value> {
        // Run DISM health check using secure execution
        let output = Self::execute_secure_command("dism", &["/online", "/cleanup-image", "/checkhealth"])?;
        
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            
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
            let scan_output = Self::execute_secure_command("dism", &["/online", "/cleanup-image", "/scanhealth"]);
                
            if let Ok(scan) = scan_output {
                if scan.status.success() {
                    let scan_str = String::from_utf8_lossy(&scan.stdout);
                    health_info["scan_output"] = json!(scan_str.to_string());
                    
                    // Extract percentage if available
                    if let Some(percent_pos) = scan_str.find("The component store is") {
                        let relevant_text = &scan_str[percent_pos..];
                        if let Some(end) = relevant_text.find('\n') {
                            health_info["scan_result"] = json!(&relevant_text[..end]);
                        }
                    }
                }
            }
            
            Ok(health_info)
        } else {
            let error_str = String::from_utf8_lossy(&output.stderr);
            
            // Check if it's an elevation error
            if error_str.contains("Error: 740") || error_str.contains("elevation required") {
                Ok(json!({
                    "error": "DISM requires administrator privileges",
                    "suggestion": "Please run as administrator to check Windows image health",
                    "raw_error": error_str.to_string()
                }))
            } else {
                Err(anyhow::anyhow!("DISM health check failed: {}", error_str))
            }
        }
    }

    pub fn run_ipconfig(&self) -> Result<Value> {
        // Use WMI for network configuration
        self.get_native_network_adapters()
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
            }))
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
        let wmi_con = WMIConnection::new()?;
        
        let cs_results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query(
            "SELECT Domain, DomainRole, PartOfDomain FROM Win32_ComputerSystem"
        )?;
        
        if let Some(result) = cs_results.first() {
            let mut info = serde_json::Map::new();
            for (key, value) in result {
                info.insert(key.clone(), self.variant_to_json(value.clone()));
            }
            Ok(Value::Object(info))
        } else {
            Ok(json!({
                "error": "Failed to query domain information"
            }))
        }
    }

    pub fn get_disk_fragmentation(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let disks: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query("SELECT Name FROM Win32_LogicalDisk WHERE DriveType=3")?;
        
        let mut fragmentation_results = Vec::new();
        
        for disk in disks {
            if let Some(wmi::Variant::String(drive_letter)) = disk.get("Name") {
                let mut result_info = json!({
                    "drive": drive_letter,
                    "fragmentation_percent": null,
                    "status": "Not analyzed",
                    "raw_output": ""
                });

                match Self::execute_secure_command("defrag", &[drive_letter, "/A"]) {
                    Ok(output) => {
                        let output_str = String::from_utf8_lossy(&output.stdout);
                        result_info["raw_output"] = json!(output_str.to_string());

                        if output.status.success() {
                            if let Some(percent) = self.parse_defrag_output(&output_str) {
                                result_info["fragmentation_percent"] = json!(percent);
                                result_info["status"] = json!("Analyzed");
                            } else {
                                result_info["status"] = json!("Analysis failed: Could not parse output");
                            }
                        } else {
                            let error_str = String::from_utf8_lossy(&output.stderr);
                            result_info["status"] = json!(format!("Analysis failed: {}", error_str));
                        }
                    },
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
        // Look for a line like "Total fragmented space = 15 %"
        // Or "Current fragmentation = 15 %"
        output.lines()
            .find(|line| line.contains("fragmented space =") || line.contains("Current fragmentation ="))
            .and_then(|line| {
                line.split('%').next()
                    .and_then(|part| part.split('=').last())
                    .and_then(|num_str| num_str.trim().parse::<u32>().ok())
            })
    }

    pub fn get_native_services(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi_con.raw_query(
            "SELECT Name, DisplayName, State, StartMode, PathName FROM Win32_Service"
        )?;
        
        let mut services = Vec::new();
        for result in results {
            let mut service_info = serde_json::Map::new();
            for (key, value) in result {
                service_info.insert(key, self.variant_to_json(value));
            }
            services.push(Value::Object(service_info));
        }
        
        Ok(Value::Array(services))
    }

    pub fn get_battery_report(&self) -> Result<Value> {
        let temp_file = std::env::temp_dir().join("wfdiag_battery.html");
        let temp_path = temp_file.to_string_lossy();
        
        let output = Self::execute_secure_command("powercfg", &["/batteryreport", "/output", &temp_path])?;
        
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

    /// Parse battery report HTML and extract key information
    fn parse_battery_html(&self, html_content: &str) -> Result<Value> {
        let document = Html::parse_document(html_content);

        // Selectors for different sections of the battery report
        let battery_info_selector = Selector::parse("table")
            .map_err(|e| anyhow::anyhow!("Failed to parse table selector: {:?}", e))?;
        let row_selector = Selector::parse("tr")
            .map_err(|e| anyhow::anyhow!("Failed to parse tr selector: {:?}", e))?;
        let cell_selector = Selector::parse("td")
            .map_err(|e| anyhow::anyhow!("Failed to parse td selector: {:?}", e))?;
        
        let mut battery_info = json!({
            "report_generated": true,
            "batteries": []
        });
        
        // Find all tables
        for (table_index, table) in document.select(&battery_info_selector).enumerate() {
            // Look for battery information table (usually the first few tables)
            if table_index < 5 {
                let rows: Vec<_> = table.select(&row_selector).collect();
                
                // Extract battery basic info
                if rows.len() > 2 && table_index == 1 {
                    let mut battery_data = Vec::new();
                    
                    for row in rows.iter().skip(1) { // Skip header
                        let cells: Vec<_> = row.select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 2 {
                            battery_data.push(json!({
                                "property": cells[0].clone(),
                                "value": cells[1].clone()
                            }));
                        }
                    }
                    
                    if !battery_data.is_empty() {
                        battery_info["batteries"] = json!(battery_data);
                    }
                }
                
                // Look for recent usage
                if table_index == 2 {
                    let mut usage_data = Vec::new();
                    
                    for row in rows.iter().skip(1).take(10) { // Last 10 usage entries
                        let cells: Vec<_> = row.select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 4 {
                            usage_data.push(json!({
                                "start_time": cells[0].clone(),
                                "state": cells[1].clone(),
                                "capacity_remaining": cells[2].clone(),
                                "duration": cells[3].clone()
                            }));
                        }
                    }
                    
                    battery_info["recent_usage"] = json!(usage_data);
                }
                
                // Look for battery capacity history
                if table_index == 3 {
                    let mut capacity_history = Vec::new();
                    
                    for row in rows.iter().skip(1).take(5) { // Last 5 capacity readings
                        let cells: Vec<_> = row.select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 3 {
                            capacity_history.push(json!({
                                "period": cells[0].clone(),
                                "full_charge_capacity": cells[1].clone(),
                                "design_capacity": cells[2].clone()
                            }));
                        }
                    }
                    
                    battery_info["battery_capacity_history"] = json!(capacity_history);
                }
            }
        }
        
        // Try to calculate battery health percentage
        if let Some(_batteries) = battery_info["batteries"].as_array() {
            if let Some(latest) = battery_info["battery_capacity_history"].as_array()
                .and_then(|h| h.first()) {
                
                if let (Some(full_charge), Some(design_capacity)) = (
                    latest["full_charge_capacity"].as_str(),
                    latest["design_capacity"].as_str()
                ) {
                    if let (Ok(full_mwh), Ok(design_mwh)) = (
                        self.extract_mwh_value(full_charge),
                        self.extract_mwh_value(design_capacity)
                    ) {
                        if design_mwh > 0.0 {
                            let health_percentage = (full_mwh / design_mwh * 100.0).round();
                            battery_info["battery_health_percentage"] = json!(health_percentage);
                            battery_info["battery_health_status"] = json!(
                                if health_percentage >= 80.0 { "Good" }
                                else if health_percentage >= 60.0 { "Fair" }
                                else { "Poor" }
                            );
                        }
                    }
                }
            }
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

        if let Ok(entries) = fs::read_dir(minidump_path) {
            for entry in entries.filter_map(Result::ok) {
                if let Ok(metadata) = entry.metadata() {
                    if entry.path().extension().and_then(|s| s.to_str()) == Some("dmp") {
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
        }

        // Check if Desktop\Minidumps exists
        let desktop_minidumps = self.get_desktop_minidumps_path();
        let desktop_minidumps_exists = desktop_minidumps.as_ref().map_or(false, |p| p.exists());

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
        if let Some(desktop_path) = dirs::desktop_dir() {
            Some(desktop_path.join("Minidumps"))
        } else {
            None
        }
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
        if !desktop_minidumps.exists() {
            if let Err(e) = fs::create_dir_all(&desktop_minidumps) {
                return Ok(json!({
                    "success": false,
                    "message": format!("Failed to create Desktop\\Minidumps directory: {}", e),
                    "copied_files": []
                }));
            }
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
                            errors.push(format!("Failed to copy {}: {}", filename.to_string_lossy(), e));
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
        // Use PowerShell to get Windows Store apps
        let executor = crate::security::SecureCommandExecutor::new();
        let output = executor.execute_powershell_script(
            "Get-AppxPackage | Select-Object Name, Version, PackageFullName, InstallLocation, Publisher | ConvertTo-Json"
        )?;
        
        if output.status.success() {
            let json_str = String::from_utf8_lossy(&output.stdout);
            match serde_json::from_str(&json_str) {
                Ok(value) => Ok(value),
                Err(_) => {
                    // If JSON parsing fails, return raw output
                    Ok(json!({
                        "raw_output": json_str.to_string(),
                        "error": "Failed to parse PowerShell output as JSON"
                    }))
                }
            }
        } else {
            Err(anyhow::anyhow!("Failed to get store apps: {}", 
                String::from_utf8_lossy(&output.stderr)))
        }
    }

    pub fn get_performance_data(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let mut perf_data = json!({});
        
        // Get CPU performance data
        if let Ok(cpu_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_PerfFormattedData_PerfOS_Processor WHERE Name='_Total'"
        ) {
            if let Some(result) = cpu_results.first() {
                let mut cpu_info = json!({});
                for (key, value) in result {
                    cpu_info[key] = self.variant_to_json(value.clone());
                }
                perf_data["cpu_performance"] = cpu_info;
            }
        }
        
        // Get memory performance data
        if let Ok(mem_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_PerfFormattedData_PerfOS_Memory"
        ) {
            if let Some(result) = mem_results.first() {
                let mut mem_info = json!({});
                for (key, value) in result {
                    mem_info[key] = self.variant_to_json(value.clone());
                }
                perf_data["memory_performance"] = mem_info;
            }
        }
        
        // Get disk performance data
        if let Ok(disk_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_PerfFormattedData_PerfDisk_PhysicalDisk WHERE Name='_Total'"
        ) {
            if let Some(result) = disk_results.first() {
                let mut disk_info = json!({});
                for (key, value) in result {
                    disk_info[key] = self.variant_to_json(value.clone());
                }
                perf_data["disk_performance"] = disk_info;
            }
        }
        
        Ok(perf_data)
    }

    pub fn get_scheduled_tasks(&self) -> Result<Value> {
        // Use PowerShell to get scheduled tasks
        let executor = crate::security::SecureCommandExecutor::new();
        let output = executor.execute_powershell_script(
            "Get-ScheduledTask | Where-Object {$_.State -ne 'Disabled'} | Select-Object TaskName, State, TaskPath, Description, Author, Date | ConvertTo-Json"
        )?;
        
        if output.status.success() {
            let json_str = String::from_utf8_lossy(&output.stdout);
            match serde_json::from_str(&json_str) {
                Ok(value) => Ok(value),
                Err(_) => {
                    Ok(json!({
                        "raw_output": json_str.to_string(),
                        "error": "Failed to parse PowerShell output as JSON"
                    }))
                }
            }
        } else {
            Err(anyhow::anyhow!("Failed to get scheduled tasks: {}", 
                String::from_utf8_lossy(&output.stderr)))
        }
    }

    pub fn get_windows_update_history(&self) -> Result<Value> {
        let wmi_con = WMIConnection::new()?;
        let mut update_info = json!({});
        
        // Get installed hotfixes
        if let Ok(hotfix_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT * FROM Win32_QuickFixEngineering"
        ) {
            let mut hotfixes = Vec::new();
            for result in hotfix_results {
                let mut hotfix = json!({});
                for (key, value) in result {
                    hotfix[key] = self.variant_to_json(value);
                }
                hotfixes.push(hotfix);
            }
            update_info["installed_updates"] = json!(hotfixes);
        }
        
        // Try to get Windows Update history via PowerShell as fallback
        let executor = crate::security::SecureCommandExecutor::new();
        let ps_output = executor.execute_powershell_script(
            "Get-HotFix | Select-Object Description, HotFixID, InstalledOn, InstalledBy | ConvertTo-Json"
        )?;
        
        if ps_output.status.success() {
            let json_str = String::from_utf8_lossy(&ps_output.stdout);
            if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
                update_info["hotfix_details"] = value;
            }
        }
        
        Ok(update_info)
    }

    pub fn get_driver_verifier(&self) -> Result<Value> {
        // Run verifier command to get current settings
        let output = Self::execute_secure_command("verifier", &["/querysettings"])?;
        
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            
            // Parse verifier output
            let mut verifier_info = json!({
                "raw_output": output_str.to_string(),
                "enabled": false,
                "drivers": []
            });
            
            // Check if verifier is enabled
            if output_str.contains("No drivers are currently verified") {
                verifier_info["enabled"] = json!(false);
                verifier_info["status"] = json!("Driver Verifier is not active");
            } else if output_str.contains("The following drivers are being verified") {
                verifier_info["enabled"] = json!(true);
                verifier_info["status"] = json!("Driver Verifier is active");
                
                // Extract verified drivers if any
                let lines: Vec<&str> = output_str.lines().collect();
                let mut drivers = Vec::new();
                let mut in_driver_list = false;
                
                for line in lines {
                    if line.contains("The following drivers are being verified") {
                        in_driver_list = true;
                        continue;
                    }
                    if in_driver_list && !line.trim().is_empty() && !line.contains(":") {
                        drivers.push(line.trim().to_string());
                    }
                }
                
                verifier_info["verified_drivers"] = json!(drivers);
            }
            
            Ok(verifier_info)
        } else {
            // Verifier might require admin privileges
            Ok(json!({
                "error": "Failed to query driver verifier settings. Administrator privileges may be required.",
                "raw_error": String::from_utf8_lossy(&output.stderr).to_string()
            }))
        }
    }

    fn create_command(program: &str) -> std::process::Command {
        // Create secure command - this just creates the command object
        // Actual execution validation happens in execute_command
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let mut cmd = std::process::Command::new(program);
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd
        }
        #[cfg(not(windows))]
        {
            std::process::Command::new(program)
        }
    }

    /// Secure command execution with validation
    fn execute_secure_command(program: &str, args: &[&str]) -> Result<std::process::Output> {
        let executor = crate::security::SecureCommandExecutor::new();
        executor.execute_command(program, args)
            .map_err(|e| anyhow::anyhow!("Security validation failed: {}", e))
    }
}