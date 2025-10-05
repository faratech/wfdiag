use anyhow::Result;
use serde_json::{json, Value};

pub struct WindowsNativeAPI;

impl WindowsNativeAPI {
    pub fn new() -> Self {
        Self
    }

    /// Get system information using native Windows APIs
    pub fn get_system_info(&self) -> Result<Value> {
        use sysinfo::System;
        let mut system = System::new();
        system.refresh_memory();
        system.refresh_all();

        // Get Windows version info
        let os_version = System::os_version().unwrap_or_else(|| "Unknown".to_string());
        let kernel_version = System::kernel_version().unwrap_or_else(|| "Unknown".to_string());

        // Parse build number from kernel version or use registry
        let build_number = self.get_windows_build_number();
        let windows_version = self.get_windows_version_name(build_number);

        // Get uptime in seconds
        let uptime_seconds = System::uptime();

        // Get architecture information
        let arch_info = crate::architecture::get_architecture_info()
            .unwrap_or_else(|_| crate::architecture::ArchitectureInfo {
                process_arch: crate::architecture::ProcessorArchitecture::Unknown,
                native_arch: crate::architecture::ProcessorArchitecture::Unknown,
                is_emulated: false,
                process_arch_name: "Unknown".to_string(),
                native_arch_name: "Unknown".to_string(),
                page_size: 4096,
                processor_count: std::thread::available_parallelism().map(|p| p.get() as u32).unwrap_or(1),
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
                "total_physical": system.total_memory(),
                "available_physical": system.available_memory(),
                "total_virtual": system.total_swap(),
                "available_virtual": system.free_swap(),
                "memory_load": ((system.total_memory() - system.available_memory()) * 100 / system.total_memory().max(1))
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

    /// Get Windows build number from registry
    fn get_windows_build_number(&self) -> u32 {
        #[cfg(windows)]
        {
            use winreg::RegKey;
            use winreg::enums::*;
            
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") {
                if let Ok(build) = key.get_value::<String, _>("CurrentBuild") {
                    if let Ok(build_num) = build.parse::<u32>() {
                        return build_num;
                    }
                }
            }
        }
        
        // Fallback
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
                _ => "Windows 10 (Unknown Build)".to_string()
            }
        } else {
            match build {
                // Windows 10
                10240 => "Windows 10 1507 (RTM)".to_string(),
                10586 => "Windows 10 1511 (November Update)".to_string(),
                14393 => if self.is_server_edition() { "Windows Server 2016".to_string() } else { "Windows 10 1607 (Anniversary Update)".to_string() },
                15063 => "Windows 10 1703 (Creators Update)".to_string(),
                16299 => "Windows 10 1709 (Fall Creators Update)".to_string(),
                17134 => "Windows 10 1803 (April 2018 Update)".to_string(),
                17763 => if self.is_server_edition() { "Windows Server 2019".to_string() } else { "Windows 10 1809 (October 2018 Update)".to_string() },
                18362 => "Windows 10 1903 (May 2019 Update)".to_string(),
                18363 => "Windows 10 1909 (November 2019 Update)".to_string(),
                19041 => "Windows 10 2004 (May 2020 Update)".to_string(),
                19042 => "Windows 10 20H2 (October 2020 Update)".to_string(),
                19043 => "Windows 10 21H1 (May 2021 Update)".to_string(),
                19044 => "Windows 10 21H2 (November 2021 Update)".to_string(),
                19045 => "Windows 10 22H2 (October 2022 Update)".to_string(),
                
                // Older versions
                9600 => "Windows 8.1 / Server 2012 R2".to_string(),
                9200 => "Windows 8 / Server 2012".to_string(),
                7601 => "Windows 7 SP1 / Server 2008 R2 SP1".to_string(),
                7600 => "Windows 7 RTM / Server 2008 R2 RTM".to_string(),
                6002 => "Windows Vista SP2 / Server 2008 SP2".to_string(),
                6001 => "Windows Vista SP1 / Server 2008 SP1".to_string(),
                6000 => "Windows Vista RTM / Server 2008 RTM".to_string(),
                
                _ if build > 0 => format!("Windows (Build {})", build),
                _ => "Unknown Windows Version".to_string()
            }
        }
    }

    /// Check if this is a server edition
    fn is_server_edition(&self) -> bool {
        #[cfg(windows)]
        {
            use winreg::RegKey;
            use winreg::enums::*;
            
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") {
                if let Ok(product_name) = key.get_value::<String, _>("ProductName") {
                    return product_name.to_lowercase().contains("server");
                }
            }
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
        // TODO: Implement with proper Windows APIs
        // For now, use sysinfo crate as fallback
        use sysinfo::{System, Disks};
        let _system = System::new();
        let disks = Disks::new_with_refreshed_list();
        
        let drives: Vec<Value> = disks
            .iter()
            .map(|disk| {
                let mount_point = disk.mount_point().to_string_lossy().to_string();
                let total_bytes = disk.total_space();
                let free_bytes = disk.available_space();
                let used_bytes = total_bytes - free_bytes;
                let used_percent = if total_bytes > 0 {
                    (used_bytes as f64 / total_bytes as f64 * 100.0).round()
                } else {
                    0.0
                };
                
                json!({
                    "drive_letter": mount_point.trim_end_matches(['\\', '/']),
                    "drive_type": 3, // Fixed disk
                    "drive_type_name": "Fixed",
                    "total_bytes": total_bytes,
                    "free_bytes": free_bytes,
                    "used_bytes": used_bytes,
                    "used_percent": used_percent,
                    "volume_label": disk.name().to_string_lossy().to_string(),
                    "file_system": format!("{:?}", disk.file_system()),
                    "serial_number": "00000000"
                })
            })
            .collect();
            
        Ok(json!(drives))
    }

    /// Get network adapter information using native Windows APIs
    pub fn get_network_adapters(&self) -> Result<Value> {
        // TODO: Implement with proper Windows APIs
        // For now, use sysinfo as fallback
        use sysinfo::Networks;
        let networks = Networks::new_with_refreshed_list();
        
        let adapters: Vec<Value> = networks
            .iter()
            .map(|(name, network)| {
                let name_str = name.to_string();
                let is_physical = !name_str.to_lowercase().contains("virtual") &&
                                !name_str.to_lowercase().contains("loopback") &&
                                !name_str.to_lowercase().contains("teredo") &&
                                !name_str.to_lowercase().contains("isatap");
                
                json!({
                    "adapter_name": name_str,
                    "friendly_name": name_str,
                    "description": name_str,
                    "interface_type": 6, // Ethernet
                    "operational_status": 1, // Up
                    "operational_status_name": "Up",
                    "physical_address": "00:00:00:00:00:00", // Placeholder
                    "physical_address_length": 6,
                    "mtu": 1500,
                    "interface_index": 1,
                    "transmit_link_speed": network.total_transmitted(),
                    "receive_link_speed": network.total_received(),
                    "is_physical": is_physical
                })
            })
            .collect();
            
        Ok(json!(adapters))
    }

    /// Get system services using native Windows APIs
    pub fn get_services(&self) -> Result<Value> {
        // TODO: Implement with proper Windows APIs
        // For now, return placeholder data
        Ok(json!([
            {
                "service_name": "Windows Update",
                "display_name": "Windows Update",
                "service_type": 32,
                "current_state": 4,
                "current_state_name": "Running",
                "controls_accepted": 1,
                "win32_exit_code": 0,
                "service_specific_exit_code": 0,
                "check_point": 0,
                "wait_hint": 0,
                "process_id": 1000,
                "service_flags": 0
            }
        ]))
    }
}