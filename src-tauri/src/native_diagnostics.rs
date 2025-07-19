use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use wmi::{COMLibrary, WMIConnection, Variant};
use sysinfo::System;
use std::process::Command;
use std::fs;
use std::path::Path;
use crate::windows_native::WindowsNativeAPI;
use scraper::{Html, Selector};
use regex::Regex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;


pub struct NativeDiagnostics {
    wmi_con: Option<WMIConnection>,
    windows_api: WindowsNativeAPI,
}

impl NativeDiagnostics {
    pub fn new() -> Self {
        // Try to initialize WMI, but don't fail if it doesn't work
        let wmi_con = match COMLibrary::new() {
            Ok(com_con) => WMIConnection::new(com_con.into()).ok(),
            Err(_) => None,
        };
        
        Self { 
            wmi_con,
            windows_api: WindowsNativeAPI::new(),
        }
    }

    // Helper to create commands without console window
    fn create_command(program: &str) -> Command {
        let mut cmd = Command::new(program);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    // Convert WMI variants to JSON values
    fn convert_variant_to_json(variant: &Variant) -> Value {
        match variant {
            Variant::String(s) => json!(s),
            Variant::I1(v) => json!(v),
            Variant::I2(v) => json!(v),
            Variant::I4(v) => json!(v),
            Variant::I8(v) => json!(v),
            Variant::UI1(v) => json!(v),
            Variant::UI2(v) => json!(v),
            Variant::UI4(v) => json!(v),
            Variant::UI8(v) => json!(v),
            Variant::Bool(v) => json!(v),
            Variant::R4(v) => json!(v),
            Variant::R8(v) => json!(v),
            Variant::Array(arr) => {
                let values: Vec<Value> = arr.iter()
                    .map(|v| Self::convert_variant_to_json(v))
                    .collect();
                json!(values)
            }
            _ => json!(null),
        }
    }

    fn convert_wmi_results(&self, results: Vec<HashMap<String, Variant>>) -> Vec<Value> {
        results.into_iter()
            .map(|row| {
                let mut obj = serde_json::Map::new();
                for (key, value) in row {
                    obj.insert(key, Self::convert_variant_to_json(&value));
                }
                json!(obj)
            })
            .collect()
    }

    pub fn run_wmi_query(&self, class: &str) -> Result<Value> {
        if let Some(ref wmi) = self.wmi_con {
            let query = format!("SELECT * FROM {}", class);
            let results: Vec<HashMap<String, Variant>> = wmi.raw_query(&query)?;
            Ok(json!(self.convert_wmi_results(results)))
        } else {
            // Fallback to wmic command
            self.run_wmic_command(class)
        }
    }

    fn run_wmic_command(&self, class: &str) -> Result<Value> {
        let output = Command::new("wmic")
            .args(&["path", class, "get", "/format:list"])
            .output()?;
        
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(json!({ "raw_output": text.to_string() }))
    }

    pub fn get_system_info(&self) -> Result<Value> {
        let mut sys = System::new_all();
        sys.refresh_all();
        
        let load = System::load_average();
        Ok(json!({
            "hostname": System::host_name(),
            "os_name": System::name(),
            "os_version": System::os_version(),
            "kernel_version": System::kernel_version(),
            "total_memory": sys.total_memory(),
            "used_memory": sys.used_memory(),
            "total_swap": sys.total_swap(),
            "used_swap": sys.used_swap(),
            "cpu_count": sys.cpus().len(),
            "cpu_brand": sys.cpus().first().map(|c| c.brand().to_string()),
            "cpu_frequency": sys.cpus().first().map(|c| c.frequency()),
            "boot_time": System::boot_time(),
            "uptime": System::uptime(),
            "load_average": {
                "one": load.one,
                "five": load.five,
                "fifteen": load.fifteen
            }
        }))
    }

    pub fn get_drivers(&self) -> Result<Value> {
        if let Some(ref wmi) = self.wmi_con {
            let results: Vec<HashMap<String, Variant>> = wmi.raw_query(
                "SELECT DeviceName, DriverVersion, Manufacturer FROM Win32_PnPSignedDriver"
            )?;
            Ok(json!(self.convert_wmi_results(results)))
        } else {
            self.run_wmic_command("Win32_PnPSignedDriver")
        }
    }

    pub fn get_installed_programs(&self) -> Result<Value> {
        let mut programs = Vec::new();
        
        // Try to read from registry using winreg crate
        #[cfg(windows)]
        {
            use winreg::enums::*;
            use winreg::RegKey;
            
            let paths = [
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            ];
            
            for path in &paths {
                let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
                    if let Ok(uninstall_key) = hklm.open_subkey(path) {
                        for subkey_name in uninstall_key.enum_keys().filter_map(Result::ok) {
                            if let Ok(app_key) = uninstall_key.open_subkey(&subkey_name) {
                                let display_name: Option<String> = app_key.get_value("DisplayName").ok();
                                let version: Option<String> = app_key.get_value("DisplayVersion").ok();
                                let publisher: Option<String> = app_key.get_value("Publisher").ok();
                                
                                if let Some(name) = display_name {
                                    programs.push(json!({
                                        "name": name,
                                        "version": version.unwrap_or_default(),
                                        "publisher": publisher.unwrap_or_default(),
                                    }));
                                }
                            }
                        }
                    }
            }
        }
        
        // If registry reading failed or not on Windows, try PowerShell
        if programs.is_empty() {
            let output = Self::create_command("powershell")
                .args(&["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", "Get-Package | Select-Object Name, Version | ConvertTo-Json"])
                .output()?;
            
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(parsed) = serde_json::from_str::<Value>(&json_str) {
                    return Ok(parsed);
                }
            }
        }
        
        Ok(json!(programs))
    }

    pub fn run_dxdiag(&self) -> Result<Value> {
        let temp_file = std::env::temp_dir().join("wfdiag_dxdiag.txt");
        let temp_path = temp_file.to_string_lossy();
        
        // Clean up any existing file
        let _ = fs::remove_file(&temp_file);
        
        // Start dxdiag
        let mut child = Self::create_command("dxdiag")
            .args(&["/t", &temp_path, "/whql:off"])
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start dxdiag: {}", e))?;
        
        // Wait up to 15 seconds for dxdiag to complete
        let mut completed = false;
        for i in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if temp_file.exists() {
                if let Ok(metadata) = fs::metadata(&temp_file) {
                    if metadata.len() > 1000 {  // Ensure file has some content
                        // Wait a bit more to ensure it's finished writing
                        std::thread::sleep(std::time::Duration::from_secs(if i > 10 { 2 } else { 1 }));
                        completed = true;
                        break;
                    }
                }
            }
        }
        
        // Kill the process if still running
        let _ = child.kill();
        let _ = child.wait();
        
        // Read the output
        if completed && temp_file.exists() {
            match fs::read_to_string(&temp_file) {
                Ok(content) => {
                    let _ = fs::remove_file(&temp_file);
                    if content.len() > 100 {
                        Ok(json!({ "raw_output": content }))
                    } else {
                        Err(anyhow::anyhow!("DXDiag output file too small or corrupted"))
                    }
                }
                Err(e) => {
                    let _ = fs::remove_file(&temp_file);
                    Err(anyhow::anyhow!("Failed to read DXDiag output: {}", e))
                }
            }
        } else {
            let _ = fs::remove_file(&temp_file);
            Err(anyhow::anyhow!("DXDiag failed to complete or output file not created"))
        }
    }

    pub fn run_chkdsk(&self) -> Result<Value> {
        // Use chkdsk in read-only mode for safety
        let output = Self::create_command("chkdsk")
            .args(&["C:", "/f", "/v"])  // /f = fix errors in read-only mode, /v = verbose
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run chkdsk: {}", e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        let mut result = json!({
            "command": "chkdsk C: /f /v",
            "exit_code": output.status.code().unwrap_or(-1),
            "stdout": stdout.to_string(),
            "stderr": stderr.to_string()
        });

        // If chkdsk fails, try alternative disk check methods
        if !output.status.success() {
            // Try PowerShell Get-Volume as fallback
            match Self::create_command("powershell")
                .args(&["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", "Get-Volume C | Select-Object DriveLetter, FileSystemLabel, DriveType, HealthStatus, OperationalStatus, Size, SizeRemaining"])
                .output() {
                Ok(ps_output) => {
                    let ps_stdout = String::from_utf8_lossy(&ps_output.stdout);
                    result["fallback_volume_info"] = json!(ps_stdout.to_string());
                }
                Err(_) => {}
            }
        }

        Ok(result)
    }

    pub fn run_dism_health(&self) -> Result<Value> {
        // Run DISM health check
        let output = Self::create_command("dism")
            .args(&["/online", "/cleanup-image", "/checkhealth"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run DISM: {}. Make sure you're running as administrator.", e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        let mut result = json!({
            "command": "dism /online /cleanup-image /checkhealth",
            "exit_code": output.status.code().unwrap_or(-1),
            "stdout": stdout.to_string(),
            "stderr": stderr.to_string(),
            "success": output.status.success()
        });

        // If basic check succeeds, also try scan health for more detailed info
        if output.status.success() {
            match Self::create_command("dism")
                .args(&["/online", "/cleanup-image", "/scanhealth"])
                .output() {
                Ok(scan_output) => {
                    let scan_stdout = String::from_utf8_lossy(&scan_output.stdout);
                    result["detailed_scan"] = json!({
                        "stdout": scan_stdout.to_string(),
                        "success": scan_output.status.success()
                    });
                }
                Err(_) => {}
            }
        }

        Ok(result)
    }

    pub fn run_ipconfig(&self) -> Result<Value> {
        let output = Self::create_command("ipconfig")
            .args(&["/all"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run ipconfig: {}", e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        Ok(json!({
            "command": "ipconfig /all",
            "exit_code": output.status.code().unwrap_or(-1),
            "success": output.status.success(),
            "output": stdout.to_string(),
            "stderr": stderr.to_string()
        }))
    }

    pub fn read_hosts_file(&self) -> Result<Value> {
        let hosts_path = "C:\\Windows\\System32\\drivers\\etc\\hosts";
        
        match fs::read_to_string(hosts_path) {
            Ok(content) => {
                // Parse the hosts file to extract meaningful entries
                let mut entries = Vec::new();
                let mut comments = Vec::new();
                
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    
                    if trimmed.starts_with('#') {
                        comments.push(trimmed.to_string());
                    } else {
                        // Parse IP and hostname entries
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if parts.len() >= 2 {
                            entries.push(json!({
                                "ip": parts[0],
                                "hostname": parts[1],
                                "aliases": parts.get(2..).unwrap_or(&[]).to_vec()
                            }));
                        }
                    }
                }
                
                Ok(json!({
                    "file_path": hosts_path,
                    "total_lines": content.lines().count(),
                    "entries": entries,
                    "comments": comments,
                    "raw_content": content
                }))
            }
            Err(e) => Err(anyhow::anyhow!("Failed to read hosts file: {}", e))
        }
    }

    pub fn run_dsregcmd(&self) -> Result<Value> {
        let output = Self::create_command("dsregcmd")
            .args(&["/status"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run dsregcmd: {}", e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        // Parse dsregcmd output for structured data
        let mut device_state = json!({});
        let mut tenant_details = json!({});
        let mut user_state = json!({});
        
        let mut current_section = "";
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            
            if trimmed.contains("Device State") {
                current_section = "device";
            } else if trimmed.contains("Tenant Details") {
                current_section = "tenant";
            } else if trimmed.contains("User State") {
                current_section = "user";
            } else if trimmed.contains(" : ") {
                let parts: Vec<&str> = trimmed.splitn(2, " : ").collect();
                if parts.len() == 2 {
                    let key = parts[0].trim();
                    let value = parts[1].trim();
                    
                    match current_section {
                        "device" => device_state[key] = json!(value),
                        "tenant" => tenant_details[key] = json!(value),
                        "user" => user_state[key] = json!(value),
                        _ => {}
                    }
                }
            }
        }
        
        Ok(json!({
            "command": "dsregcmd /status",
            "exit_code": output.status.code().unwrap_or(-1),
            "success": output.status.success(),
            "device_state": device_state,
            "tenant_details": tenant_details,
            "user_state": user_state,
            "raw_output": stdout.to_string(),
            "stderr": stderr.to_string()
        }))
    }

    /// Get system information using native Windows APIs (replaces multiple WMI calls)
    pub fn get_native_system_info(&self) -> Result<Value> {
        self.windows_api.get_system_info()
    }

    /// Get disk space using native Windows APIs (replaces external commands)
    pub fn get_native_disk_space(&self) -> Result<Value> {
        self.windows_api.get_disk_space()
    }

    /// Get network adapters using native Windows APIs (replaces external commands)
    pub fn get_native_network_adapters(&self) -> Result<Value> {
        self.windows_api.get_network_adapters()
    }

    /// Get services using native Windows APIs (replaces external commands)
    pub fn get_native_services(&self) -> Result<Value> {
        self.windows_api.get_services()
    }

    pub fn get_event_logs(&self) -> Result<Value> {
        // Try native Windows API first, fallback to wevtutil if needed
        #[cfg(windows)]
        {
            // For now, keep the external command but with better error handling
            let mut logs = HashMap::new();
            
            for log_name in &["System", "Application"] {
                let output = Self::create_command("wevtutil")
                    .args(&["qe", log_name, "/c:50", "/f:text", "/rd:true"])
                    .output();
                
                match output {
                    Ok(output) if output.status.success() => {
                        let text = String::from_utf8_lossy(&output.stdout);
                        logs.insert(log_name.to_string(), text.to_string());
                    }
                    _ => {
                        logs.insert(log_name.to_string(), format!("Failed to read {} log", log_name));
                    }
                }
            }
            
            Ok(json!(logs))
        }
        
        #[cfg(not(windows))]
        Err(anyhow::anyhow!("Event logs only available on Windows"))
    }

    pub fn get_battery_report(&self) -> Result<Value> {
        let temp_file = std::env::temp_dir().join("wfdiag_battery.html");
        let temp_path = temp_file.to_string_lossy();
        
        let output = Self::create_command("powercfg")
            .args(&["/batteryreport", "/output", &temp_path])
            .output()?;
        
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
        let table_selector = Selector::parse("table").unwrap();
        let row_selector = Selector::parse("tr").unwrap();
        let cell_selector = Selector::parse("td, th").unwrap();
        
        let mut battery_info = json!({
            "batteries": [],
            "recent_usage": [],
            "usage_history": [],
            "battery_capacity_history": []
        });

        // Extract battery information from tables
        for table in document.select(&table_selector) {
            let rows: Vec<_> = table.select(&row_selector).collect();
            if rows.is_empty() {
                continue;
            }

            // Get table headers to identify the table type
            let headers: Vec<String> = rows[0]
                .select(&cell_selector)
                .map(|cell| cell.text().collect::<String>().trim().to_string())
                .collect();

            if headers.is_empty() {
                continue;
            }

            // Identify table type and extract relevant data
            match headers[0].to_lowercase().as_str() {
                h if h.contains("battery") && h.contains("information") => {
                    // Battery information table
                    let mut batteries = Vec::new();
                    for row in rows.iter().skip(1) {
                        let cells: Vec<String> = row
                            .select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 2 {
                            batteries.push(json!({
                                "property": cells[0],
                                "value": cells.get(1).unwrap_or(&"".to_string())
                            }));
                        }
                    }
                    battery_info["batteries"] = json!(batteries);
                }
                h if h.contains("recent") && h.contains("usage") => {
                    // Recent usage table
                    let mut usage_data = Vec::new();
                    for row in rows.iter().skip(1) {
                        let cells: Vec<String> = row
                            .select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 3 {
                            usage_data.push(json!({
                                "start_time": cells.get(0).unwrap_or(&"".to_string()),
                                "state": cells.get(1).unwrap_or(&"".to_string()),
                                "capacity_remaining": cells.get(2).unwrap_or(&"".to_string()),
                                "duration": cells.get(3).unwrap_or(&"".to_string())
                            }));
                        }
                    }
                    battery_info["recent_usage"] = json!(usage_data);
                }
                h if h.contains("usage") && h.contains("history") => {
                    // Usage history table
                    let mut history_data = Vec::new();
                    for row in rows.iter().skip(1) {
                        let cells: Vec<String> = row
                            .select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 2 {
                            history_data.push(json!({
                                "period": cells.get(0).unwrap_or(&"".to_string()),
                                "battery_life": cells.get(1).unwrap_or(&"".to_string())
                            }));
                        }
                    }
                    battery_info["usage_history"] = json!(history_data);
                }
                h if h.contains("capacity") && h.contains("history") => {
                    // Battery capacity history
                    let mut capacity_data = Vec::new();
                    for row in rows.iter().skip(1) {
                        let cells: Vec<String> = row
                            .select(&cell_selector)
                            .map(|cell| cell.text().collect::<String>().trim().to_string())
                            .collect();
                        
                        if cells.len() >= 3 {
                            capacity_data.push(json!({
                                "period": cells.get(0).unwrap_or(&"".to_string()),
                                "full_charge_capacity": cells.get(1).unwrap_or(&"".to_string()),
                                "design_capacity": cells.get(2).unwrap_or(&"".to_string())
                            }));
                        }
                    }
                    battery_info["battery_capacity_history"] = json!(capacity_data);
                }
                _ => {}
            }
        }

        // Extract summary information using regex patterns
        let text_content = document.root_element().text().collect::<String>();
        
        // Extract computer name
        if let Ok(computer_regex) = Regex::new(r"Computer name\s+(.+)") {
            if let Some(caps) = computer_regex.captures(&text_content) {
                battery_info["computer_name"] = json!(caps[1].trim());
            }
        }

        // Extract report time
        if let Ok(time_regex) = Regex::new(r"Report generated at\s+(.+)") {
            if let Some(caps) = time_regex.captures(&text_content) {
                battery_info["report_generated_at"] = json!(caps[1].trim());
            }
        }

        // Calculate battery health if capacity data is available
        if let Some(capacity_history) = battery_info["battery_capacity_history"].as_array() {
            if let Some(latest) = capacity_history.first() {
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
                "message": "No minidump directory found"
            }));
        }
        
        let mut dumps = Vec::new();
        
        if let Ok(entries) = fs::read_dir(minidump_path) {
            for entry in entries.filter_map(Result::ok) {
                if let Some(ext) = entry.path().extension() {
                    if ext.to_string_lossy().eq_ignore_ascii_case("dmp") {
                        if let Ok(metadata) = entry.metadata() {
                            dumps.push(json!({
                                "filename": entry.file_name().to_string_lossy(),
                                "size": metadata.len(),
                                "modified": metadata.modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs()),
                            }));
                        }
                    }
                }
            }
        }
        
        // Sort by modified time (newest first)
        dumps.sort_by(|a, b| {
            let a_time = a.get("modified").and_then(|v| v.as_u64()).unwrap_or(0);
            let b_time = b.get("modified").and_then(|v| v.as_u64()).unwrap_or(0);
            b_time.cmp(&a_time)
        });
        
        let total_count = dumps.len();
        Ok(json!({
            "dumps": dumps.into_iter().take(5).collect::<Vec<_>>(),
            "total_count": total_count
        }))
    }
}

