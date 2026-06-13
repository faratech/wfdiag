use anyhow::Result;
use serde_json::{Value, json};

#[allow(dead_code)]
pub struct WindowsNativeAPI;

#[allow(dead_code)]
impl WindowsNativeAPI {
    pub fn new() -> Self {
        Self
    }

    /// Get system information using native Windows APIs
    pub fn get_system_info(&self) -> Result<Value> {
        use windows::Win32::System::SystemInformation::{
            GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };

        // Get memory info
        let mut mem_status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        let (total_memory, available_memory, total_swap, free_swap, memory_load) = unsafe {
            if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
                (
                    mem_status.ullTotalPhys,
                    mem_status.ullAvailPhys,
                    mem_status
                        .ullTotalPageFile
                        .saturating_sub(mem_status.ullTotalPhys),
                    mem_status
                        .ullAvailPageFile
                        .saturating_sub(mem_status.ullAvailPhys),
                    mem_status.dwMemoryLoad,
                )
            } else {
                (0, 0, 0, 0, 0)
            }
        };

        // Get Windows version info from registry
        let (os_version, kernel_version) = self.get_windows_version_from_registry();
        let build_number = self.get_windows_build_number();
        let windows_version = self.get_windows_version_name(build_number);

        // Get uptime using GetTickCount64
        let uptime_seconds = unsafe { GetTickCount64() / 1000 };

        // Get architecture information
        let arch_info = crate::architecture::get_architecture_info().unwrap_or_else(|_| {
            crate::architecture::ArchitectureInfo {
                process_arch: crate::architecture::ProcessorArchitecture::Unknown,
                native_arch: crate::architecture::ProcessorArchitecture::Unknown,
                is_emulated: false,
                process_arch_name: "Unknown".to_string(),
                native_arch_name: "Unknown".to_string(),
                page_size: 4096,
                processor_count: std::thread::available_parallelism()
                    .map(|p| p.get() as u32)
                    .unwrap_or(1),
            }
        });

        Ok(json!({
            "computer_name": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string()),
            "processor_architecture": arch_info.process_arch.to_u16(),
            "processor_architecture_name": arch_info.process_arch_name,
            "native_architecture": arch_info.native_arch.to_u16(),
            "native_architecture_name": arch_info.native_arch_name,
            "is_emulated": arch_info.is_emulated,
            "processor_count": arch_info.processor_count,
            "page_size": arch_info.page_size,
            "memory": {
                "total_physical": total_memory,
                "available_physical": available_memory,
                "total_virtual": total_swap,
                "available_virtual": free_swap,
                "memory_load": memory_load
            },
            "os_version": {
                "version_string": os_version,
                "kernel_version": kernel_version,
                "build_number": build_number,
                "windows_version": windows_version,
                "major": 10,
                "minor": 0,
                "platform_id": 2,
                "service_pack_major": 0,
                "service_pack_minor": 0,
                "product_type": 1
            },
            "uptime": {
                "seconds": uptime_seconds,
                "formatted": self.format_uptime(uptime_seconds)
            }
        }))
    }

    /// Get Windows version info from registry
    fn get_windows_version_from_registry(&self) -> (String, String) {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") {
            let product_name = key
                .get_value::<String, _>("ProductName")
                .unwrap_or_else(|_| "Windows".to_string());
            let build = key
                .get_value::<String, _>("CurrentBuild")
                .unwrap_or_else(|_| "0".to_string());
            let ubr = key
                .get_value::<u32, _>("UBR")
                .map(|u| format!(".{}", u))
                .unwrap_or_default();

            let os_version = format!("{} (Build {}{})", product_name, build, ubr);
            let kernel_version = format!("{}{}", build, ubr);
            return (os_version, kernel_version);
        }

        ("Windows".to_string(), "Unknown".to_string())
    }

    /// Get Windows build number from registry
    fn get_windows_build_number(&self) -> u32 {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            && let Ok(build) = key.get_value::<String, _>("CurrentBuild")
            && let Ok(build_num) = build.parse::<u32>()
        {
            return build_num;
        }
        0
    }

    /// Convert build number to Windows version name
    fn get_windows_version_name(&self, build: u32) -> String {
        if build >= 26000 {
            "Windows 11 (Insider)".to_string()
        } else if build >= 25000 {
            "Windows 11 24H2".to_string()
        } else if build >= 24000 {
            "Windows 11 23H2".to_string()
        } else if build >= 23000 {
            "Windows 11 22H2".to_string()
        } else if build >= 22000 {
            "Windows 11 21H2".to_string()
        } else if build >= 20000 {
            match build {
                20348 => "Windows Server 2022".to_string(),
                _ => "Windows 10 (Unknown Build)".to_string(),
            }
        } else {
            match build {
                10240 => "Windows 10 1507 (RTM)".to_string(),
                10586 => "Windows 10 1511 (November Update)".to_string(),
                14393 => {
                    if self.is_server_edition() {
                        "Windows Server 2016".to_string()
                    } else {
                        "Windows 10 1607 (Anniversary Update)".to_string()
                    }
                }
                15063 => "Windows 10 1703 (Creators Update)".to_string(),
                16299 => "Windows 10 1709 (Fall Creators Update)".to_string(),
                17134 => "Windows 10 1803 (April 2018 Update)".to_string(),
                17763 => {
                    if self.is_server_edition() {
                        "Windows Server 2019".to_string()
                    } else {
                        "Windows 10 1809 (October 2018 Update)".to_string()
                    }
                }
                18362 => "Windows 10 1903 (May 2019 Update)".to_string(),
                18363 => "Windows 10 1909 (November 2019 Update)".to_string(),
                19041 => "Windows 10 2004 (May 2020 Update)".to_string(),
                19042 => "Windows 10 20H2 (October 2020 Update)".to_string(),
                19043 => "Windows 10 21H1 (May 2021 Update)".to_string(),
                19044 => "Windows 10 21H2 (November 2021 Update)".to_string(),
                19045 => "Windows 10 22H2 (October 2022 Update)".to_string(),
                9600 => "Windows 8.1 / Server 2012 R2".to_string(),
                9200 => "Windows 8 / Server 2012".to_string(),
                7601 => "Windows 7 SP1 / Server 2008 R2 SP1".to_string(),
                7600 => "Windows 7 RTM / Server 2008 R2 RTM".to_string(),
                6002 => "Windows Vista SP2 / Server 2008 SP2".to_string(),
                6001 => "Windows Vista SP1 / Server 2008 SP1".to_string(),
                6000 => "Windows Vista RTM / Server 2008 RTM".to_string(),
                _ if build > 0 => format!("Windows (Build {})", build),
                _ => "Unknown Windows Version".to_string(),
            }
        }
    }

    /// Check if this is a server edition
    fn is_server_edition(&self) -> bool {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            && let Ok(product_name) = key.get_value::<String, _>("ProductName")
        {
            return product_name.to_lowercase().contains("server");
        }
        false
    }

    /// Format uptime into human readable string
    fn format_uptime(&self, uptime_seconds: u64) -> String {
        let days = uptime_seconds / 86400;
        let hours = (uptime_seconds % 86400) / 3600;
        let minutes = (uptime_seconds % 3600) / 60;
        let seconds = uptime_seconds % 60;

        if days > 0 {
            format!("{} days, {}:{:02}:{:02}", days, hours, minutes, seconds)
        } else if hours > 0 {
            format!("{}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{}:{:02}", minutes, seconds)
        }
    }

    /// Get disk space information using native Windows APIs
    pub fn get_disk_space(&self) -> Result<Value> {
        use windows::Win32::Storage::FileSystem::{
            GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
        };
        use windows::core::PCWSTR;
        // Drive type constants (not exported as named constants in windows crate 0.62)
        const DRIVE_FIXED: u32 = 3;
        const DRIVE_REMOVABLE: u32 = 2;

        let mut drives = Vec::new();
        let mut buffer = [0u16; 256];

        unsafe {
            let len = GetLogicalDriveStringsW(Some(&mut buffer));
            if len == 0 {
                return Ok(json!([]));
            }

            let mut i = 0;
            while i < len as usize && buffer[i] != 0 {
                let start = i;
                while i < len as usize && buffer[i] != 0 {
                    i += 1;
                }

                let drive_str: String = String::from_utf16_lossy(&buffer[start..i]);
                let drive_wide: Vec<u16> =
                    drive_str.encode_utf16().chain(std::iter::once(0)).collect();

                let drive_type_raw = GetDriveTypeW(PCWSTR(drive_wide.as_ptr()));
                if drive_type_raw != DRIVE_FIXED && drive_type_raw != DRIVE_REMOVABLE {
                    i += 1;
                    continue;
                }

                let mut free_bytes_available: u64 = 0;
                let mut total_bytes: u64 = 0;
                let mut total_free_bytes: u64 = 0;

                if GetDiskFreeSpaceExW(
                    PCWSTR(drive_wide.as_ptr()),
                    Some(&mut free_bytes_available),
                    Some(&mut total_bytes),
                    Some(&mut total_free_bytes),
                )
                .is_ok()
                {
                    let used_bytes = total_bytes.saturating_sub(total_free_bytes);
                    let used_percent = if total_bytes > 0 {
                        (used_bytes as f64 / total_bytes as f64 * 100.0).round()
                    } else {
                        0.0
                    };

                    drives.push(json!({
                        "drive_letter": drive_str.trim_end_matches(['\\', '/']),
                        "drive_type": drive_type_raw,
                        "drive_type_name": if drive_type_raw == DRIVE_FIXED { "Fixed" } else { "Removable" },
                        "total_bytes": total_bytes,
                        "free_bytes": total_free_bytes,
                        "used_bytes": used_bytes,
                        "used_percent": used_percent,
                        "volume_label": "",
                        "file_system": "NTFS",
                        "serial_number": "00000000"
                    }));
                }

                i += 1;
            }
        }

        Ok(json!(drives))
    }

    /// Get network adapter information using native Windows APIs
    pub fn get_network_adapters(&self) -> Result<Value> {
        use windows::Win32::NetworkManagement::IpHelper::{
            FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
        };

        let mut adapters = Vec::new();

        unsafe {
            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();

            if GetIfTable2(&mut table).is_ok() && !table.is_null() {
                let num_entries = (*table).NumEntries as usize;
                let entries = std::slice::from_raw_parts((*table).Table.as_ptr(), num_entries);

                for entry in entries {
                    // Skip loopback interfaces
                    if entry.Type == 24 {
                        continue;
                    }

                    let name = String::from_utf16_lossy(&entry.Alias)
                        .trim_end_matches('\0')
                        .to_string();

                    let description = String::from_utf16_lossy(&entry.Description)
                        .trim_end_matches('\0')
                        .to_string();

                    let is_physical = !name.to_lowercase().contains("virtual")
                        && !name.to_lowercase().contains("loopback")
                        && !name.to_lowercase().contains("teredo")
                        && !name.to_lowercase().contains("isatap");

                    // Format MAC address
                    let mac = if entry.PhysicalAddressLength > 0 {
                        entry.PhysicalAddress[..entry.PhysicalAddressLength as usize]
                            .iter()
                            .map(|b| format!("{:02X}", b))
                            .collect::<Vec<_>>()
                            .join(":")
                    } else {
                        "00:00:00:00:00:00".to_string()
                    };

                    // Use .0 to get raw u32 value from IF_OPER_STATUS
                    let oper_status = entry.OperStatus.0;
                    adapters.push(json!({
                        "adapter_name": name,
                        "friendly_name": name,
                        "description": description,
                        "interface_type": entry.Type,
                        "operational_status": oper_status,
                        "operational_status_name": if oper_status == 1 { "Up" } else { "Down" },
                        "physical_address": mac,
                        "physical_address_length": entry.PhysicalAddressLength,
                        "mtu": entry.Mtu,
                        "interface_index": entry.InterfaceIndex,
                        "transmit_link_speed": entry.TransmitLinkSpeed,
                        "receive_link_speed": entry.ReceiveLinkSpeed,
                        "is_physical": is_physical
                    }));
                }

                FreeMibTable(table as *const _);
            }
        }

        Ok(json!(adapters))
    }

    /// Get system services using WMI for reliable enumeration
    pub fn get_services(&self) -> Result<Value> {
        // Use WMI for reliable service enumeration (consistent with native_diagnostics.rs)
        let wmi_con = crate::wmi_native::WmiConnection::new()?;
        let results = wmi_con.query(
            "SELECT Name, DisplayName, State, StartMode, PathName, ProcessId FROM Win32_Service",
        )?;

        let services: Vec<Value> = results
            .into_iter()
            .map(|r| {
                let obj: serde_json::Map<String, Value> = r.into_iter().collect();
                Value::Object(obj)
            })
            .collect();

        Ok(Value::Array(services))
    }
}
