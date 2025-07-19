use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{System, Disks};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::time::interval;

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
    
    // Network
    pub network_upload_kb: f64,
    pub network_download_kb: f64,
    
    // Process info
    pub top_processes: Vec<ProcessInfo>,
    
    // Timestamp
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
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
    app_handle: AppHandle,
    monitoring: Arc<Mutex<bool>>,
    previous_network: Arc<Mutex<(u64, u64)>>, // (bytes_sent, bytes_recv)
}

impl SystemMonitor {
    pub fn new(app_handle: AppHandle) -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        
        let disks = Disks::new_with_refreshed_list();
        
        Self {
            system: Arc::new(Mutex::new(system)),
            disks: Arc::new(Mutex::new(disks)),
            app_handle,
            monitoring: Arc::new(Mutex::new(false)),
            previous_network: Arc::new(Mutex::new((0, 0))),
        }
    }
    
    pub async fn start_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = true;
        drop(monitoring);
        
        let system = Arc::clone(&self.system);
        let disks = Arc::clone(&self.disks);
        let app_handle = self.app_handle.clone();
        let monitoring_flag = Arc::clone(&self.monitoring);
        let previous_network = Arc::clone(&self.previous_network);
        
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(1));
            
            // Initialize network counters
            #[cfg(windows)]
            {
                match get_network_stats() {
                    Ok((sent, recv)) => {
                        let mut prev = previous_network.lock().await;
                        *prev = (sent, recv);
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize network stats: {}", e);
                        let mut prev = previous_network.lock().await;
                        *prev = (0, 0);
                    }
                }
            }
            
            loop {
                interval.tick().await;
                
                let monitoring = monitoring_flag.lock().await;
                if !*monitoring {
                    break;
                }
                drop(monitoring);
                
                let stats = collect_stats(&system, &disks, &previous_network).await;
                
                // Emit the stats through Tauri's event system
                let _ = app_handle.emit_all("system-stats", &stats);
            }
        });
    }
    
    pub async fn stop_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = false;
    }
    
    pub async fn get_current_stats(&self) -> SystemStats {
        collect_stats(&self.system, &self.disks, &self.previous_network).await
    }
}

async fn collect_stats(
    system: &Arc<Mutex<System>>,
    disks: &Arc<Mutex<Disks>>,
    previous_network: &Arc<Mutex<(u64, u64)>>,
) -> SystemStats {
    let mut sys = system.lock().await;
    
    // Refresh system data
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    
    // CPU stats - sysinfo 0.30 API changes
    let cpu_utilization = sys.global_cpu_info().cpu_usage();
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
    disks_guard.refresh();
    let disk_list = disks_guard.list();
    
    let total_disk_space: u64 = disk_list.iter().map(|d| d.total_space()).sum();
    let available_disk_space: u64 = disk_list.iter().map(|d| d.available_space()).sum();
    let used_disk_space = total_disk_space.saturating_sub(available_disk_space);
    
    let disk_utilization = if total_disk_space > 0 {
        (used_disk_space as f32 / total_disk_space as f32) * 100.0
    } else {
        0.0
    };
    drop(disks_guard);
    
    // Network stats - Windows specific
    #[cfg(windows)]
    let (network_upload_kb, network_download_kb) = {
        match get_network_stats() {
            Ok((sent, recv)) => {
                let mut prev = previous_network.lock().await;
                let (prev_sent, prev_recv) = *prev;
                
                // Only calculate if we have previous data
                if prev_sent > 0 || prev_recv > 0 {
                    let upload = ((sent.saturating_sub(prev_sent)) as f64) / 1024.0;
                    let download = ((recv.saturating_sub(prev_recv)) as f64) / 1024.0;
                    *prev = (sent, recv);
                    (upload, download)
                } else {
                    // First run, just store the values
                    *prev = (sent, recv);
                    (0.0, 0.0)
                }
            }
            Err(e) => {
                eprintln!("Failed to get network stats: {}", e);
                (0.0, 0.0)
            }
        }
    };
    
    #[cfg(not(windows))]
    let (network_upload_kb, network_download_kb) = (0.0, 0.0);
    
    // Top processes by CPU usage
    let memory_total_kb = memory_total as f32;
    let mut processes: Vec<_> = sys.processes().iter().map(|(pid, proc)| {
        ProcessInfo {
            pid: pid.as_u32(),
            name: proc.name().to_string(),
            cpu_percent: proc.cpu_usage(),
            memory_percent: (proc.memory() as f32 / memory_total_kb) * 100.0,
        }
    }).collect();
    
    // Sort by CPU usage and take top 10
    processes.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap());
    processes.truncate(10);
    
    // Note: Disk I/O stats would require platform-specific implementation
    // For now, using placeholder values
    let disk_read_bytes = 0;
    let disk_write_bytes = 0;
    
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
        network_upload_kb,
        network_download_kb,
        top_processes: processes,
        timestamp: chrono::Utc::now().timestamp(),
    }
}

// Get network connections (Windows-specific)
#[cfg(windows)]
pub async fn get_network_connections() -> Vec<NetworkConnection> {
    use std::process::Command;
    
    let output = match Command::new("netstat")
        .args(&["-an"])
        .output() {
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

// Get network statistics using Windows API
#[cfg(windows)]
fn get_network_stats() -> Result<(u64, u64), String> {
    use windows::Win32::NetworkManagement::IpHelper::{GetIfTable2, FreeMibTable, MIB_IF_TABLE2};
    use std::ptr;
    
    unsafe {
        let mut table_ptr: *mut MIB_IF_TABLE2 = ptr::null_mut();
        
        match GetIfTable2(&mut table_ptr) {
            Ok(()) => {
                if table_ptr.is_null() {
                    return Err("Failed to get network interface table".to_string());
                }
                
                let table = &*table_ptr;
                let mut total_sent: u64 = 0;
                let mut total_recv: u64 = 0;
                
                // Sum up bytes from all network interfaces
                let num_entries = table.NumEntries as usize;
                if num_entries > 0 && num_entries < 1000 { // Sanity check
                    for i in 0..num_entries.min(table.Table.len()) {
                        let entry = &table.Table[i];
                        total_sent = total_sent.saturating_add(entry.OutOctets);
                        total_recv = total_recv.saturating_add(entry.InOctets);
                    }
                }
                
                let _ = FreeMibTable(table_ptr as *const _);
                Ok((total_sent, total_recv))
            }
            Err(_) => Err("Failed to get network interface table".to_string()),
        }
    }
}

#[cfg(not(windows))]
fn get_network_stats() -> Result<(u64, u64), String> {
    Ok((0, 0))
}