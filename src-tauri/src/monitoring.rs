use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{System, Disks, Networks};
use tauri::{AppHandle, Manager};
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
                let _ = app_handle.emit_all("system-stats", &stats);
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
    sys.refresh_processes();
    sys.refresh_cpu_usage();
    
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
    networks_guard.refresh();
    
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
        let cmd_line = proc.cmd().join(" ");
        let status = match proc.status() {
            sysinfo::ProcessStatus::Idle => "Idle",
            sysinfo::ProcessStatus::Run => "Running",
            sysinfo::ProcessStatus::Sleep => "Sleeping",
            sysinfo::ProcessStatus::Stop => "Stopped",
            sysinfo::ProcessStatus::Zombie => "Zombie",
            _ => "Unknown",
        }.to_string();
        
        ProcessInfo {
            pid: pid.as_u32(),
            name: proc.name().to_string(),
            cpu_percent: proc.cpu_usage(),
            memory_percent: ((proc.memory() as f64 / memory_total_kb / 1024.0) * 100.0) as f32,
            memory_mb: proc.memory() as f64 / 1024.0 / 1024.0,
            virtual_memory_mb: proc.virtual_memory() as f64 / 1024.0 / 1024.0,
            disk_read_bytes: proc.disk_usage().read_bytes,
            disk_write_bytes: proc.disk_usage().written_bytes,
            status,
            start_time: proc.start_time() as i64,
            command: if cmd_line.is_empty() { proc.name().to_string() } else { cmd_line },
        }
    }).collect();
    
    // Sort by CPU usage and take top 20 for better visibility
    processes.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap());
    processes.truncate(20);
    
    // Disk I/O stats (aggregated from all processes)
    let disk_read_bytes = sys.processes().values().map(|p| p.disk_usage().read_bytes).sum();
    let disk_write_bytes = sys.processes().values().map(|p| p.disk_usage().written_bytes).sum();
    
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

