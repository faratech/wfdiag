use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{System, Disks, Networks, ProcessesToUpdate};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::time::interval;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    // CPU
    pub cpu_utilization: f32,
    pub per_cpu_utilization: Vec<f32>,
    pub cpu_frequency: u64,

    // Memory
    pub memory_total_gb: f64,
    pub memory_used_gb: f64,
    pub memory_available_gb: f64,
    pub memory_utilization: f32,

    // Swap
    pub swap_total_gb: f64,
    pub swap_used_gb: f64,
    pub swap_utilization: f32,

    // Disk
    pub disk_utilization: f32,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disks: Vec<DiskInfo>,

    // Network
    pub network_upload_kb: f64,
    pub network_download_kb: f64,

    // NPU (Neural Processing Unit)
    pub npu_available: bool,
    pub npu_name: Option<String>,
    pub npu_utilization: Option<f32>,

    // Process info
    pub top_processes: Vec<ProcessInfo>,

    // Timestamp
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
    pub utilization: f32,
    pub file_system: String,
    pub disk_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub memory_mb: f64,
    pub virtual_memory_mb: f64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub status: String,
    pub start_time: i64,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub protocol: String,
    pub local_addr: String,
    pub remote_addr: String,
    pub status: String,
}

pub struct SystemMonitor {
    system: Arc<Mutex<System>>,
    disks: Arc<Mutex<Disks>>,
    networks: Arc<Mutex<Networks>>,
    app_handle: AppHandle,
    monitoring: Arc<Mutex<bool>>,
    previous_network: Arc<Mutex<HashMap<String, (u64, u64)>>>, // interface -> (bytes_sent, bytes_recv)
    update_interval: Arc<Mutex<Duration>>,
}

impl SystemMonitor {
    pub fn new(app_handle: AppHandle) -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        
        let disks = Disks::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();
        
        Self {
            system: Arc::new(Mutex::new(system)),
            disks: Arc::new(Mutex::new(disks)),
            networks: Arc::new(Mutex::new(networks)),
            app_handle,
            monitoring: Arc::new(Mutex::new(false)),
            previous_network: Arc::new(Mutex::new(HashMap::new())),
            update_interval: Arc::new(Mutex::new(Duration::from_secs(1))), // 1 second update interval
        }
    }
    
    pub async fn start_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = true;
        drop(monitoring);
        
        let system = Arc::clone(&self.system);
        let disks = Arc::clone(&self.disks);
        let networks = Arc::clone(&self.networks);
        let app_handle = self.app_handle.clone();
        let monitoring_flag = Arc::clone(&self.monitoring);
        let previous_network = Arc::clone(&self.previous_network);
        let update_interval = Arc::clone(&self.update_interval);
        
        tokio::spawn(async move {
            // Use dynamic update interval
            let interval_duration = *update_interval.lock().await;
            let mut interval = interval(interval_duration);
            
            loop {
                interval.tick().await;
                
                let monitoring = monitoring_flag.lock().await;
                if !*monitoring {
                    break;
                }
                drop(monitoring);
                
                let stats = collect_stats(&system, &disks, &networks, &previous_network).await;
                
                // Emit the stats through Tauri's event system
                let _ = app_handle.emit("system-stats", &stats);
            }
        });
    }
    
    pub async fn stop_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = false;
    }
    
    pub async fn get_current_stats(&self) -> SystemStats {
        collect_stats(&self.system, &self.disks, &self.networks, &self.previous_network).await
    }
    
    pub async fn set_update_interval(&self, seconds: u64) {
        let mut interval = self.update_interval.lock().await;
        *interval = Duration::from_secs(seconds.max(1)); // Minimum 1 second
    }
}

async fn collect_stats(
    system: &Arc<Mutex<System>>,
    disks: &Arc<Mutex<Disks>>,
    networks: &Arc<Mutex<Networks>>,
    previous_network: &Arc<Mutex<HashMap<String, (u64, u64)>>>,
) -> SystemStats {
    let mut sys = system.lock().await;
    
    // Refresh all system data
    sys.refresh_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.refresh_cpu_usage();
    
    // CPU stats - sysinfo 0.36 API changes
    let cpu_utilization = sys.global_cpu_usage();
    let per_cpu_utilization: Vec<f32> = sys.cpus().iter().map(|cpu| cpu.cpu_usage()).collect();
    let cpu_frequency = sys.cpus().first().map(|cpu| cpu.frequency()).unwrap_or(0);
    
    // Memory stats
    let memory_total = sys.total_memory();
    let memory_used = sys.used_memory();
    let memory_available = memory_total - memory_used;
    let memory_utilization = (memory_used as f32 / memory_total as f32) * 100.0;
    
    // Swap stats
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();
    let swap_utilization = if swap_total > 0 {
        (swap_used as f32 / swap_total as f32) * 100.0
    } else {
        0.0
    };
    
    // Disk stats - using Disks API
    let mut disks_guard = disks.lock().await;
    disks_guard.refresh(true);
    let disk_list = disks_guard.list();
    
    // Collect individual disk information
    let mut disk_infos: Vec<DiskInfo> = Vec::new();
    for disk in disk_list {
        let total_space = disk.total_space();
        let available_space = disk.available_space();
        let used_space = total_space.saturating_sub(available_space);
        let utilization = if total_space > 0 {
            (used_space as f32 / total_space as f32) * 100.0
        } else {
            0.0
        };
        
        // Determine disk type
        let disk_type = match disk.kind() {
            sysinfo::DiskKind::HDD => "HDD",
            sysinfo::DiskKind::SSD => "SSD",
            _ => "Unknown",
        }.to_string();
        
        disk_infos.push(DiskInfo {
            name: disk.name().to_string_lossy().to_string(),
            mount_point: disk.mount_point().to_string_lossy().to_string(),
            total_gb: (total_space as f64) / (1024.0 * 1024.0 * 1024.0),
            used_gb: (used_space as f64) / (1024.0 * 1024.0 * 1024.0),
            available_gb: (available_space as f64) / (1024.0 * 1024.0 * 1024.0),
            utilization,
            file_system: format!("{:?}", disk.file_system()).replace("\"", ""),
            disk_type,
        });
    }
    
    // Calculate total disk stats
    let total_disk_space: u64 = disk_list.iter().map(|d| d.total_space()).sum();
    let available_disk_space: u64 = disk_list.iter().map(|d| d.available_space()).sum();
    let used_disk_space = total_disk_space.saturating_sub(available_disk_space);
    
    let disk_utilization = if total_disk_space > 0 {
        (used_disk_space as f32 / total_disk_space as f32) * 100.0
    } else {
        0.0
    };
    drop(disks_guard);
    
    // Network stats with proper monitoring
    let mut networks_guard = networks.lock().await;
    networks_guard.refresh(false);
    
    let mut _total_upload_bytes = 0u64;
    let mut _total_download_bytes = 0u64;
    let mut current_network_stats = HashMap::new();
    
    for (interface_name, network) in networks_guard.iter() {
        let tx_bytes = network.total_transmitted();
        let rx_bytes = network.total_received();
        _total_upload_bytes += tx_bytes;
        _total_download_bytes += rx_bytes;
        current_network_stats.insert(interface_name.to_string(), (tx_bytes, rx_bytes));
    }
    drop(networks_guard);
    
    // Calculate network speed (bytes per second)
    let mut prev_network = previous_network.lock().await;
    let mut network_upload_kb = 0.0;
    let mut network_download_kb = 0.0;
    
    if !prev_network.is_empty() {
        for (interface, (tx, rx)) in &current_network_stats {
            if let Some((prev_tx, prev_rx)) = prev_network.get(interface) {
                let tx_diff = tx.saturating_sub(*prev_tx);
                let rx_diff = rx.saturating_sub(*prev_rx);
                network_upload_kb += (tx_diff as f64) / 1024.0;
                network_download_kb += (rx_diff as f64) / 1024.0;
            }
        }
    }
    
    *prev_network = current_network_stats;
    drop(prev_network);
    
    // Enhanced process information
    let memory_total_kb = memory_total as f64 / 1024.0;
    let mut processes: Vec<_> = sys.processes().iter().map(|(pid, proc)| {
        // Convert Vec<OsString> to String by joining with space
        let cmd_line = proc.cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<String>>()
            .join(" ");
        let status = match proc.status() {
            sysinfo::ProcessStatus::Idle => "Idle",
            sysinfo::ProcessStatus::Run => "Running",
            sysinfo::ProcessStatus::Sleep => "Sleeping",
            sysinfo::ProcessStatus::Stop => "Stopped",
            sysinfo::ProcessStatus::Zombie => "Zombie",
            _ => "Unknown",
        }.to_string();
        
        // Convert &OsStr to String for name
        let process_name = proc.name().to_string_lossy().into_owned();
        
        ProcessInfo {
            pid: pid.as_u32(),
            name: process_name.clone(),
            cpu_percent: proc.cpu_usage(),
            memory_percent: ((proc.memory() as f64 / memory_total_kb / 1024.0) * 100.0) as f32,
            memory_mb: proc.memory() as f64 / 1024.0 / 1024.0,
            virtual_memory_mb: proc.virtual_memory() as f64 / 1024.0 / 1024.0,
            disk_read_bytes: proc.disk_usage().read_bytes,
            disk_write_bytes: proc.disk_usage().written_bytes,
            status,
            start_time: proc.start_time() as i64,
            command: if cmd_line.is_empty() { process_name } else { cmd_line },
        }
    }).collect();
    
    // Sort by CPU usage and take top 20 for better visibility
    processes.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap());
    processes.truncate(20);
    
    // Disk I/O stats (aggregated from all processes)
    let disk_read_bytes = sys.processes().values().map(|p| p.disk_usage().read_bytes).sum();
    let disk_write_bytes = sys.processes().values().map(|p| p.disk_usage().written_bytes).sum();

    // NPU Detection (Windows-specific)
    let (npu_available, npu_name, npu_utilization) = detect_npu().await;

    SystemStats {
        cpu_utilization,
        per_cpu_utilization,
        cpu_frequency,
        memory_total_gb: (memory_total as f64) / (1024.0 * 1024.0 * 1024.0),
        memory_used_gb: (memory_used as f64) / (1024.0 * 1024.0 * 1024.0),
        memory_available_gb: (memory_available as f64) / (1024.0 * 1024.0 * 1024.0),
        memory_utilization,
        swap_total_gb: (swap_total as f64) / (1024.0 * 1024.0 * 1024.0),
        swap_used_gb: (swap_used as f64) / (1024.0 * 1024.0 * 1024.0),
        swap_utilization,
        disk_utilization,
        disk_read_bytes,
        disk_write_bytes,
        disks: disk_infos,
        network_upload_kb,
        network_download_kb,
        npu_available,
        npu_name,
        npu_utilization,
        top_processes: processes,
        timestamp: chrono::Utc::now().timestamp(),
    }
}

// Get network connections (Windows-specific)
#[cfg(windows)]
pub async fn get_network_connections() -> Vec<NetworkConnection> {
    use std::process::Command;
    
    let executor = crate::security::SecureCommandExecutor::new();
    let output = match executor.execute_command("netstat", &["-an"]) {
        Ok(output) => output,
        Err(_) => return vec![],
    };
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut connections = Vec::new();
    
    for line in stdout.lines().skip(4) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            let protocol = parts[0];
            let local_addr = parts[1];
            let remote_addr = parts[2];
            let status = if parts.len() > 3 { parts[3] } else { "NONE" };
            
            connections.push(NetworkConnection {
                protocol: protocol.to_string(),
                local_addr: local_addr.to_string(),
                remote_addr: remote_addr.to_string(),
                status: status.to_string(),
            });
        }
    }
    
    connections
}

#[cfg(not(windows))]
pub async fn get_network_connections() -> Vec<NetworkConnection> {
    vec![]
}

// Detect NPU (Neural Processing Unit) on Windows using DXCore API
#[cfg(windows)]
async fn detect_npu() -> (bool, Option<String>, Option<f32>) {
    // Try DXCore first (proper Windows API for NPU enumeration)
    if let Some((name, _)) = detect_npu_dxcore() {
        // Try to get NPU utilization via performance counters
        let utilization = get_npu_utilization();
        return (true, Some(name), utilization);
    }

    // Fallback to WMI-based detection for older systems
    let (available, name, _) = detect_npu_wmi_fallback();
    if available {
        let utilization = get_npu_utilization();
        return (available, name, utilization);
    }

    (false, None, None)
}

/// Static storage for discovered NPU LUID (adapter identifier)
#[cfg(windows)]
static NPU_LUID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Try to get NPU utilization from Windows performance counters
/// The NPU is exposed through Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine
/// with a specific LUID that only has "Compute" engine types
#[cfg(windows)]
fn get_npu_utilization() -> Option<f32> {
    use wmi::WMIConnection;
    use std::collections::HashMap;

    let wmi_con = WMIConnection::new().ok()?;

    // Get or discover the NPU LUID
    let npu_luid = NPU_LUID.get_or_init(|| {
        discover_npu_luid(&wmi_con)
    });

    // If we found the NPU LUID, query its utilization
    if let Some(luid) = npu_luid {
        // Query GPU Engine entries for this specific LUID
        let query = format!(
            "SELECT UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine WHERE Name LIKE '%{}%'",
            luid
        );

        if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(&query) {
            // Sum utilization across all NPU engine instances
            let mut total_util: u64 = 0;
            let mut count = 0;

            for counter in &results {
                if let Some(wmi::Variant::UI8(util)) = counter.get("UtilizationPercentage") {
                    total_util += util;
                    count += 1;
                } else if let Some(wmi::Variant::UI4(util)) = counter.get("UtilizationPercentage") {
                    total_util += *util as u64;
                    count += 1;
                }
            }

            if count > 0 {
                // Return total utilization (sum across all processes using NPU)
                // Cap at 100% since multiple processes can use it
                return Some((total_util as f32).min(100.0));
            }
        }
    }

    None
}

/// Discover the NPU LUID by finding the adapter that is a ComputeAccelerator
/// The NPU appears in GPU Engine counters but only has "Compute" engine types
#[cfg(windows)]
fn discover_npu_luid(wmi_con: &wmi::WMIConnection) -> Option<String> {
    use std::collections::HashMap;
    use std::collections::HashSet;

    // Strategy: Find all LUIDs and their engine types
    // The NPU LUID will only have "Compute" engines (no 3D, VideoProcessing, etc.)

    if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
        "SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine"
    ) {
        // Group engine types by LUID
        let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();

        for counter in &results {
            if let Some(wmi::Variant::String(name)) = counter.get("Name") {
                // Parse the Name field: pid_XXXX_luid_0xYYYYYYYY_0xZZZZZZZZ_phys_N_eng_M_engtype_TYPE
                if let Some(luid_start) = name.find("luid_") {
                    if let Some(phys_start) = name.find("_phys_") {
                        let luid = &name[luid_start..phys_start];

                        // Extract engine type
                        if let Some(engtype_start) = name.find("_engtype_") {
                            let engtype = &name[engtype_start + 9..]; // Skip "_engtype_"
                            luid_engines.entry(luid.to_string())
                                .or_insert_with(HashSet::new)
                                .insert(engtype.to_string());
                        }
                    }
                }
            }
        }

        // Find LUID that only has "Compute" engine type (this is likely the NPU)
        for (luid, engine_types) in &luid_engines {
            // NPU typically only has Compute engine, no 3D, VideoProcessing, etc.
            if engine_types.len() == 1 && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }

        // Fallback: If no pure-Compute LUID found, look for one without 3D engine
        // (some NPUs might have multiple engine types but no 3D)
        for (luid, engine_types) in &luid_engines {
            if !engine_types.contains("3D") && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }
    }

    None
}

/// Detect NPU using DXCore API (Windows 10 1903+)
/// This is the proper Windows API for enumerating compute adapters including NPUs
#[cfg(windows)]
fn detect_npu_dxcore() -> Option<(String, u32)> {
    use windows::Win32::Graphics::DXCore::*;
    use windows::core::GUID;

    // NPU hardware type GUID - from Windows SDK dxcore_interface.h
    // DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU = {b69eb219-3ded-4464-979f-a0b033988718}
    const DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU: GUID = GUID::from_values(
        0xb69eb219,
        0x3ded,
        0x4464,
        [0x97, 0x9f, 0xa0, 0xb0, 0x33, 0x98, 0x87, 0x18],
    );

    unsafe {
        // Create DXCore adapter factory
        let factory: IDXCoreAdapterFactory = match DXCoreCreateAdapterFactory() {
            Ok(f) => f,
            Err(_) => return None,
        };

        // Create adapter list filtering for NPU hardware type
        let attributes = [DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU];
        let adapter_list: IDXCoreAdapterList = match factory.CreateAdapterList(&attributes) {
            Ok(list) => list,
            Err(_) => return None,
        };

        let adapter_count = adapter_list.GetAdapterCount();
        if adapter_count == 0 {
            return None;
        }

        // Get the first NPU adapter
        let adapter: IDXCoreAdapter = match adapter_list.GetAdapter(0) {
            Ok(a) => a,
            Err(_) => return None,
        };

        // Get adapter description size using the proper constant name
        let desc_size = match adapter.GetPropertySize(DriverDescription) {
            Ok(size) => size,
            Err(_) => return Some(("NPU Detected".to_string(), adapter_count)),
        };

        if desc_size == 0 {
            return Some(("NPU Detected".to_string(), adapter_count));
        }

        // Get the property value
        let mut desc_buffer: Vec<u8> = vec![0; desc_size];
        if adapter.GetProperty(
            DriverDescription,
            desc_size,
            desc_buffer.as_mut_ptr() as *mut _,
        ).is_err() {
            return Some(("NPU Detected".to_string(), adapter_count));
        }

        // Convert to string (null-terminated wide string or UTF-8)
        let name = String::from_utf8_lossy(&desc_buffer)
            .trim_end_matches('\0')
            .to_string();

        if name.is_empty() {
            Some(("NPU Detected".to_string(), adapter_count))
        } else {
            Some((name, adapter_count))
        }
    }
}

/// Fallback WMI-based NPU detection for systems without DXCore NPU support
#[cfg(windows)]
fn detect_npu_wmi_fallback() -> (bool, Option<String>, Option<f32>) {
    use wmi::WMIConnection;
    use std::collections::HashMap;

    if let Ok(wmi_con) = WMIConnection::new() {
        // Check for NPU in PnP devices
        if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT Name, DeviceID, Description FROM Win32_PnPEntity WHERE \
             Name LIKE '%NPU%' OR \
             Name LIKE '%Neural%' OR \
             Name LIKE '%AI Accelerator%' OR \
             Name LIKE '%Hexagon%' OR \
             Description LIKE '%Neural Processing%' OR \
             DeviceID LIKE '%NPU%'"
        ) {
            if let Some(device) = results.first() {
                if let Some(wmi::Variant::String(name)) = device.get("Name") {
                    return (true, Some(name.clone()), None);
                }
            }
        }

        // Check CPU name for integrated NPU
        if let Ok(cpu_results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT Name FROM Win32_Processor"
        ) {
            if let Some(cpu) = cpu_results.first() {
                if let Some(wmi::Variant::String(name)) = cpu.get("Name") {
                    let name_lower = name.to_lowercase();

                    // Intel Core Ultra / Meteor Lake / Arrow Lake / Lunar Lake
                    if name_lower.contains("core ultra") ||
                       name_lower.contains("meteor lake") ||
                       name_lower.contains("arrow lake") ||
                       name_lower.contains("lunar lake") {
                        return (true, Some(format!("{} (Intel NPU)", name)), None);
                    }

                    // AMD Ryzen AI / XDNA
                    if name_lower.contains("ryzen ai") ||
                       name_lower.contains("ryzen 8000") ||
                       name_lower.contains("ryzen 9000") {
                        return (true, Some(format!("{} (AMD XDNA NPU)", name)), None);
                    }

                    // Qualcomm Snapdragon X Elite/Plus with Hexagon NPU
                    if name_lower.contains("snapdragon") ||
                       name_lower.contains("x elite") ||
                       name_lower.contains("x plus") {
                        return (true, Some(format!("{} (Qualcomm Hexagon NPU)", name)), None);
                    }
                }
            }
        }
    }

    (false, None, None)
}

#[cfg(not(windows))]
async fn detect_npu() -> (bool, Option<String>, Option<f32>) {
    (false, None, None)
}
