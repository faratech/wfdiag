//! Native Windows system monitoring using Windows APIs instead of sysinfo crate.
//! Optimized with aggressive caching for fluid, low-CPU monitoring.

#![cfg(windows)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::time::interval;
use windows::core::PCWSTR;
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY,
};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};

// Drive type constants
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOVABLE: u32 = 2;

// ============================================================================
// STATIC CACHES - These values never change and are computed once
// ============================================================================

/// CPU count - never changes
static CPU_COUNT: OnceLock<usize> = OnceLock::new();

/// CPU frequency in MHz - rarely changes, cached for session
static CPU_FREQUENCY: OnceLock<u64> = OnceLock::new();

/// Disk types (SSD/HDD) - never change, keyed by drive letter
/// Uses Option to allow non-blocking check
static DISK_TYPES: OnceLock<HashMap<char, String>> = OnceLock::new();

/// NPU info - detected once at startup
/// Uses Option to allow non-blocking check
static NPU_INFO: OnceLock<(bool, Option<String>)> = OnceLock::new();

/// Flag to track if background initialization is complete
static INIT_COMPLETE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// NPU LUID for utilization queries
static NPU_LUID: OnceLock<Option<String>> = OnceLock::new();

/// Total physical memory - never changes
static TOTAL_MEMORY: OnceLock<u64> = OnceLock::new();

// Reusable buffer for NtQuerySystemInformation (per-thread to avoid locks)
thread_local! {
    static PROCESS_BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::with_capacity(2 * 1024 * 1024));
}

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub cpu_utilization: f32,
    pub per_cpu_utilization: Vec<f32>,
    pub cpu_frequency: u64,
    pub memory_total_gb: f64,
    pub memory_used_gb: f64,
    pub memory_available_gb: f64,
    pub memory_utilization: f32,
    pub swap_total_gb: f64,
    pub swap_used_gb: f64,
    pub swap_utilization: f32,
    pub disk_utilization: f32,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disks: Vec<DiskInfo>,
    pub network_upload_kb: f64,
    pub network_download_kb: f64,
    pub npu_available: bool,
    pub npu_name: Option<String>,
    pub npu_utilization: Option<f32>,
    pub top_processes: Vec<ProcessInfo>,
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

/// Raw process information from NtQuerySystemInformation
#[derive(Clone, Debug)]
struct NativeProcessInfo {
    pid: u32,
    name: String,
    working_set: u64,
    private_bytes: u64,
    kernel_time: u64,
    user_time: u64,
    create_time: u64,
    read_bytes: u64,
    write_bytes: u64,
}

// SYSTEM_PROCESS_INFORMATION layout
#[repr(C)]
struct SystemProcessInfo {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    base_priority: i32,
    unique_process_id: HANDLE,
    inherited_from_unique_process_id: HANDLE,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

// ============================================================================
// PDH STATE FOR CPU MONITORING
// ============================================================================

struct SendPtr(*mut std::ffi::c_void);
unsafe impl Send for SendPtr {}
impl SendPtr {
    fn as_query(&self) -> PDH_HQUERY {
        PDH_HQUERY(self.0)
    }
    fn as_counter(&self) -> PDH_HCOUNTER {
        PDH_HCOUNTER(self.0)
    }
}

struct PdhState {
    query: SendPtr,
    counters: Vec<SendPtr>,
    initialized: bool,
    first_sample_done: bool,
}

impl Default for PdhState {
    fn default() -> Self {
        Self {
            query: SendPtr(std::ptr::null_mut()),
            counters: Vec::new(),
            initialized: false,
            first_sample_done: false,
        }
    }
}

impl Drop for PdhState {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                let _ = PdhCloseQuery(self.query.as_query());
            }
        }
    }
}

// ============================================================================
// CACHED STATE FOR SLOW-CHANGING DATA
// ============================================================================

struct CachedDiskInfo {
    disks: Vec<DiskInfo>,
    last_update: Instant,
}

struct CachedNpuUtilization {
    utilization: Option<f32>,
    last_update: Instant,
}

// ============================================================================
// SYSTEM MONITOR
// ============================================================================

type NetworkBytes = HashMap<String, (u64, u64)>;
type ProcessCpuTimes = HashMap<u32, (u64, u64, Instant)>;

pub struct SystemMonitor {
    app_handle: AppHandle,
    monitoring: Arc<Mutex<bool>>,
    pdh_state: Arc<std::sync::Mutex<PdhState>>,
    previous_network: Arc<Mutex<NetworkBytes>>,
    previous_processes: Arc<Mutex<ProcessCpuTimes>>,
    update_interval: Arc<Mutex<Duration>>,
    // Cached slow-changing data
    cached_disks: Arc<Mutex<Option<CachedDiskInfo>>>,
    cached_npu_util: Arc<Mutex<Option<CachedNpuUtilization>>>,
    // Tick counter for tiered refresh
    tick_count: Arc<AtomicU64>,
}

impl SystemMonitor {
    pub fn new(app_handle: AppHandle) -> Self {
        // Pre-initialize static caches in background
        std::thread::spawn(|| {
            initialize_static_caches();
        });

        Self {
            app_handle,
            monitoring: Arc::new(Mutex::new(false)),
            pdh_state: Arc::new(std::sync::Mutex::new(PdhState::default())),
            previous_network: Arc::new(Mutex::new(HashMap::new())),
            previous_processes: Arc::new(Mutex::new(HashMap::new())),
            update_interval: Arc::new(Mutex::new(Duration::from_secs(1))),
            cached_disks: Arc::new(Mutex::new(None)),
            cached_npu_util: Arc::new(Mutex::new(None)),
            tick_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn start_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = true;
        drop(monitoring);

        let app_handle = self.app_handle.clone();
        let monitoring_flag = Arc::clone(&self.monitoring);
        let pdh_state = Arc::clone(&self.pdh_state);
        let previous_network = Arc::clone(&self.previous_network);
        let previous_processes = Arc::clone(&self.previous_processes);
        let update_interval = Arc::clone(&self.update_interval);
        let cached_disks = Arc::clone(&self.cached_disks);
        let cached_npu_util = Arc::clone(&self.cached_npu_util);
        let tick_count = Arc::clone(&self.tick_count);

        tokio::spawn(async move {
            let interval_duration = *update_interval.lock().await;
            let mut interval = interval(interval_duration);

            loop {
                interval.tick().await;

                let monitoring = monitoring_flag.lock().await;
                if !*monitoring {
                    break;
                }
                drop(monitoring);

                let tick = tick_count.fetch_add(1, Ordering::Relaxed);

                let stats = collect_stats_optimized(
                    &pdh_state,
                    &previous_network,
                    &previous_processes,
                    &cached_disks,
                    &cached_npu_util,
                    tick,
                )
                .await;

                let _ = app_handle.emit("system-stats", &stats);
            }
        });
    }

    pub async fn stop_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = false;
    }

    pub async fn get_current_stats(&self) -> SystemStats {
        let tick = self.tick_count.load(Ordering::Relaxed);
        collect_stats_optimized(
            &self.pdh_state,
            &self.previous_network,
            &self.previous_processes,
            &self.cached_disks,
            &self.cached_npu_util,
            tick,
        )
        .await
    }

    #[allow(dead_code)]
    pub async fn set_update_interval(&self, seconds: u64) {
        let mut interval = self.update_interval.lock().await;
        *interval = Duration::from_secs(seconds.max(1));
    }
}

// ============================================================================
// INITIALIZATION
// ============================================================================

/// Pre-initialize all static caches. Call this at app startup for faster first display.
pub fn initialize_static_caches() {
    // Fast initializations first (< 1ms each)
    CPU_COUNT.get_or_init(|| unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    });

    CPU_FREQUENCY.get_or_init(get_cpu_frequency_uncached);

    TOTAL_MEMORY.get_or_init(|| {
        let mut mem_status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe {
            if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
                mem_status.ullTotalPhys
            } else {
                0
            }
        }
    });

    // Slow initializations (WMI queries, can take 1-5 seconds)
    // These run in background and UI shows defaults until ready
    DISK_TYPES.get_or_init(detect_all_disk_types);

    NPU_INFO.get_or_init(|| {
        if let Some((name, _)) = detect_npu_dxcore() {
            (true, Some(name))
        } else {
            let (available, name, _) = detect_npu_wmi_fallback();
            (available, name)
        }
    });

    // Signal that initialization is complete
    INIT_COMPLETE.store(true, std::sync::atomic::Ordering::Release);
}

// ============================================================================
// OPTIMIZED STATS COLLECTION
// ============================================================================

async fn collect_stats_optimized(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    previous_network: &Arc<Mutex<NetworkBytes>>,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
    cached_disks: &Arc<Mutex<Option<CachedDiskInfo>>>,
    cached_npu_util: &Arc<Mutex<Option<CachedNpuUtilization>>>,
    tick: u64,
) -> SystemStats {
    // FAST PATH: CPU, memory, network (every tick)
    let cpu_count = *CPU_COUNT.get_or_init(|| unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    });

    let (cpu_utilization, per_cpu_utilization) = get_cpu_utilization_pdh(pdh_state, cpu_count);
    let cpu_frequency = *CPU_FREQUENCY.get_or_init(get_cpu_frequency_uncached);

    // Memory (very fast, no caching needed)
    let (memory_total, memory_used, memory_available, swap_total, swap_used) = get_memory_info();
    let memory_utilization = if memory_total > 0 {
        (memory_used as f32 / memory_total as f32) * 100.0
    } else {
        0.0
    };
    let swap_utilization = if swap_total > 0 {
        (swap_used as f32 / swap_total as f32) * 100.0
    } else {
        0.0
    };

    // MEDIUM PATH: Disk space (every 5 ticks / 5 seconds)
    let disks = get_disks_cached(cached_disks, tick).await;
    let total_disk: u64 = disks
        .iter()
        .map(|d| (d.total_gb * 1024.0 * 1024.0 * 1024.0) as u64)
        .sum();
    let used_disk: u64 = disks
        .iter()
        .map(|d| (d.used_gb * 1024.0 * 1024.0 * 1024.0) as u64)
        .sum();
    let disk_utilization = if total_disk > 0 {
        (used_disk as f32 / total_disk as f32) * 100.0
    } else {
        0.0
    };

    // Network (every tick, uses delta calculation)
    let (network_upload_kb, network_download_kb) = get_network_stats(previous_network).await;

    // Processes (every tick, uses optimized buffer)
    let (top_processes, disk_read_bytes, disk_write_bytes) =
        get_top_processes_optimized(memory_total, previous_processes).await;

    // SLOW PATH: NPU utilization (every 5 ticks)
    // Non-blocking: use cached NPU info if available
    let (npu_available, npu_name) = NPU_INFO
        .get()
        .cloned()
        .unwrap_or((false, None));
    let npu_utilization = if npu_available {
        get_npu_util_cached(cached_npu_util, tick).await
    } else {
        None
    };

    SystemStats {
        cpu_utilization,
        per_cpu_utilization,
        cpu_frequency,
        memory_total_gb: memory_total as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_used_gb: memory_used as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_available_gb: memory_available as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_utilization,
        swap_total_gb: swap_total as f64 / (1024.0 * 1024.0 * 1024.0),
        swap_used_gb: swap_used as f64 / (1024.0 * 1024.0 * 1024.0),
        swap_utilization,
        disk_utilization,
        disk_read_bytes,
        disk_write_bytes,
        disks,
        network_upload_kb,
        network_download_kb,
        npu_available,
        npu_name,
        npu_utilization,
        top_processes,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    }
}

// ============================================================================
// CACHED GETTERS
// ============================================================================

async fn get_disks_cached(
    cached: &Arc<Mutex<Option<CachedDiskInfo>>>,
    tick: u64,
) -> Vec<DiskInfo> {
    let mut cache = cached.lock().await;

    // Refresh every 5 ticks (5 seconds at 1Hz)
    let should_refresh = match &*cache {
        Some(c) => tick.is_multiple_of(5) || c.last_update.elapsed() > Duration::from_secs(5),
        None => true,
    };

    if should_refresh {
        let disks = get_disk_info_optimized();
        *cache = Some(CachedDiskInfo {
            disks: disks.clone(),
            last_update: Instant::now(),
        });
        disks
    } else {
        cache.as_ref().unwrap().disks.clone()
    }
}

async fn get_npu_util_cached(
    cached: &Arc<Mutex<Option<CachedNpuUtilization>>>,
    tick: u64,
) -> Option<f32> {
    let mut cache = cached.lock().await;

    // Refresh every 5 ticks
    let should_refresh = match &*cache {
        Some(c) => tick.is_multiple_of(5) || c.last_update.elapsed() > Duration::from_secs(5),
        None => true,
    };

    if should_refresh {
        let util = get_npu_utilization();
        *cache = Some(CachedNpuUtilization {
            utilization: util,
            last_update: Instant::now(),
        });
        util
    } else {
        cache.as_ref().unwrap().utilization
    }
}

// ============================================================================
// CPU MONITORING (PDH)
// ============================================================================

fn get_cpu_utilization_pdh(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    cpu_count: usize,
) -> (f32, Vec<f32>) {
    let mut state = pdh_state.lock().unwrap();

    if !state.initialized {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return (0.0, vec![0.0; cpu_count]);
            }
            state.query = SendPtr(query.0);

            state.counters.reserve(cpu_count);
            for i in 0..cpu_count {
                let path = format!("\\Processor({})\\% Processor Time\0", i);
                let path_wide: Vec<u16> = path.encode_utf16().collect();
                let mut counter = PDH_HCOUNTER::default();
                let status =
                    PdhAddEnglishCounterW(query, PCWSTR(path_wide.as_ptr()), 0, &mut counter);
                if status == 0 {
                    state.counters.push(SendPtr(counter.0));
                }
            }

            state.initialized = true;
        }
    }

    unsafe {
        let status = PdhCollectQueryData(state.query.as_query());
        if status != 0 {
            return (0.0, vec![0.0; cpu_count]);
        }
    }

    if !state.first_sample_done {
        state.first_sample_done = true;
        return (0.0, vec![0.0; cpu_count]);
    }

    let mut per_cpu = Vec::with_capacity(cpu_count);
    let mut total = 0.0f32;

    for counter in &state.counters {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        let status = unsafe {
            PdhGetFormattedCounterValue(counter.as_counter(), PDH_FMT_DOUBLE, None, &mut value)
        };
        let cpu_pct = if status == 0 && value.CStatus == PDH_CSTATUS_VALID_DATA {
            unsafe { (value.Anonymous.doubleValue as f32).clamp(0.0, 100.0) }
        } else {
            0.0
        };
        per_cpu.push(cpu_pct);
        total += cpu_pct;
    }

    let avg = if !per_cpu.is_empty() {
        total / per_cpu.len() as f32
    } else {
        0.0
    };

    (avg, per_cpu)
}

fn get_cpu_frequency_uncached() -> u64 {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(cpu_key) = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0")
        && let Ok(mhz) = cpu_key.get_value::<u32, _>("~MHz") {
            return mhz as u64;
        }
    0
}

// ============================================================================
// MEMORY
// ============================================================================

fn get_memory_info() -> (u64, u64, u64, u64, u64) {
    let mut mem_status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };

    let mut perf_info = PERFORMANCE_INFORMATION {
        cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };

    unsafe {
        if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
            let total = mem_status.ullTotalPhys;
            let available = mem_status.ullAvailPhys;
            let used = total.saturating_sub(available);

            let (swap_total, swap_used) = if GetPerformanceInfo(&mut perf_info, perf_info.cb).is_ok()
            {
                let page_size = perf_info.PageSize as u64;
                let commit_limit = perf_info.CommitLimit as u64 * page_size;
                let commit_total = perf_info.CommitTotal as u64 * page_size;
                let pf_total = commit_limit.saturating_sub(total);
                let pf_used = commit_total.saturating_sub(used);
                (pf_total, pf_used.min(pf_total))
            } else {
                (0, 0)
            };

            return (total, used, available, swap_total, swap_used);
        }
    }
    (0, 0, 0, 0, 0)
}

// ============================================================================
// DISK INFO (Optimized)
// ============================================================================

/// Detect all disk types once and cache them
fn detect_all_disk_types() -> HashMap<char, String> {
    use std::collections::HashMap;
    use wmi::WMIConnection;

    let mut types = HashMap::new();

    if let Ok(wmi_con) = WMIConnection::new() {
        // Query physical disks to get their media types
        if let Ok(results) =
            wmi_con.raw_query::<HashMap<String, wmi::Variant>>("SELECT * FROM MSFT_PhysicalDisk")
        {
            for disk in &results {
                if let (Some(wmi::Variant::UI2(media_type)), Some(wmi::Variant::String(device_id))) =
                    (disk.get("MediaType"), disk.get("DeviceID"))
                {
                    let disk_type = match media_type {
                        3 => "HDD",
                        4 => "SSD",
                        5 => "SCM",
                        _ => "Unknown",
                    };

                    // Get partitions for this disk
                    let part_query =
                        format!("SELECT DriveLetter FROM MSFT_Partition WHERE DiskNumber = {}", device_id);
                    if let Ok(partitions) =
                        wmi_con.raw_query::<HashMap<String, wmi::Variant>>(&part_query)
                    {
                        for part in &partitions {
                            if let Some(wmi::Variant::String(letter)) = part.get("DriveLetter")
                                && let Some(c) = letter.chars().next() {
                                    types.insert(c, disk_type.to_string());
                                }
                        }
                    }
                }
            }
        }
    }

    // Default C: to SSD if we couldn't detect (common case)
    types.entry('C').or_insert_with(|| "SSD".to_string());

    types
}

fn get_disk_info_optimized() -> Vec<DiskInfo> {
    // Non-blocking: use cached disk types if available, otherwise default to "Unknown"
    let disk_types = DISK_TYPES.get();
    let mut disks = Vec::new();
    let mut buffer = [0u16; 256];

    unsafe {
        let len = GetLogicalDriveStringsW(Some(&mut buffer));
        if len == 0 {
            return disks;
        }

        let mut i = 0;
        while i < len as usize && buffer[i] != 0 {
            let start = i;
            while i < len as usize && buffer[i] != 0 {
                i += 1;
            }

            let drive_str: String = String::from_utf16_lossy(&buffer[start..i]);
            let drive_wide: Vec<u16> = drive_str.encode_utf16().chain(std::iter::once(0)).collect();

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
                let utilization = if total_bytes > 0 {
                    (used_bytes as f32 / total_bytes as f32) * 100.0
                } else {
                    0.0
                };

                // Use cached disk type if available, otherwise "Detecting..."
                let letter = drive_str.chars().next().unwrap_or('C');
                let disk_type = disk_types
                    .and_then(|types| types.get(&letter).cloned())
                    .unwrap_or_else(|| {
                        if INIT_COMPLETE.load(std::sync::atomic::Ordering::Acquire) {
                            "Unknown".to_string()
                        } else {
                            "Detecting...".to_string()
                        }
                    });

                disks.push(DiskInfo {
                    name: drive_str.trim_end_matches('\\').to_string(),
                    mount_point: drive_str.clone(),
                    total_gb: total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    used_gb: used_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    available_gb: total_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    utilization,
                    file_system: "NTFS".to_string(),
                    disk_type,
                });
            }

            i += 1;
        }
    }

    disks
}

// ============================================================================
// NETWORK
// ============================================================================

async fn get_network_stats(previous_network: &Arc<Mutex<NetworkBytes>>) -> (f64, f64) {
    let mut current_stats: NetworkBytes = HashMap::new();

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();

        if GetIfTable2(&mut table).is_ok() && !table.is_null() {
            let num_entries = (*table).NumEntries as usize;
            let entries = std::slice::from_raw_parts((*table).Table.as_ptr(), num_entries);

            for entry in entries {
                if entry.Type == 24 || entry.OperStatus.0 != 1 {
                    continue;
                }

                let name = String::from_utf16_lossy(&entry.Alias)
                    .trim_end_matches('\0')
                    .to_string();

                current_stats.insert(name, (entry.OutOctets, entry.InOctets));
            }

            FreeMibTable(table as *const _);
        }
    }

    let mut prev = previous_network.lock().await;
    let mut upload_kb = 0.0;
    let mut download_kb = 0.0;

    if !prev.is_empty() {
        for (interface, (tx, rx)) in &current_stats {
            if let Some((prev_tx, prev_rx)) = prev.get(interface) {
                upload_kb += tx.saturating_sub(*prev_tx) as f64 / 1024.0;
                download_kb += rx.saturating_sub(*prev_rx) as f64 / 1024.0;
            }
        }
    }

    *prev = current_stats;
    (upload_kb, download_kb)
}

// ============================================================================
// PROCESS ENUMERATION (Optimized with reusable buffer)
// ============================================================================

fn query_all_processes_optimized() -> Vec<NativeProcessInfo> {
    PROCESS_BUFFER.with(|buf| {
        let mut buffer = buf.borrow_mut();

        // Ensure minimum capacity
        let current_cap = buffer.capacity();
        if current_cap < 2 * 1024 * 1024 {
            buffer.reserve(2 * 1024 * 1024 - current_cap);
        }

        let cap = buffer.capacity();
        buffer.clear();
        buffer.resize(cap, 0);

        let mut return_length: u32 = 0;

        loop {
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    &mut return_length,
                )
            };

            if status.is_ok() {
                break;
            }

            if status.0 as u32 == 0xC0000004 {
                buffer.resize(return_length as usize + 65536, 0);
                continue;
            }

            return Vec::new();
        }

        let mut processes = Vec::with_capacity(300);
        let mut offset: usize = 0;

        loop {
            let proc_info = unsafe { &*(buffer.as_ptr().add(offset) as *const SystemProcessInfo) };

            let name = if proc_info.image_name.Length > 0 && !proc_info.image_name.Buffer.is_null()
            {
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        proc_info.image_name.Buffer.0,
                        (proc_info.image_name.Length / 2) as usize,
                    )
                };
                OsString::from_wide(slice).to_string_lossy().into_owned()
            } else if proc_info.unique_process_id.0 as usize == 0 {
                "System Idle Process".to_string()
            } else {
                "System".to_string()
            };

            processes.push(NativeProcessInfo {
                pid: proc_info.unique_process_id.0 as usize as u32,
                name,
                working_set: proc_info.working_set_size as u64,
                private_bytes: proc_info.private_page_count as u64,
                kernel_time: proc_info.kernel_time as u64,
                user_time: proc_info.user_time as u64,
                create_time: proc_info.create_time as u64,
                read_bytes: proc_info.read_transfer_count as u64,
                write_bytes: proc_info.write_transfer_count as u64,
            });

            if proc_info.next_entry_offset == 0 {
                break;
            }
            offset += proc_info.next_entry_offset as usize;
        }

        processes
    })
}

async fn get_top_processes_optimized(
    total_memory: u64,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
) -> (Vec<ProcessInfo>, u64, u64) {
    let native_procs = query_all_processes_optimized();
    let now = Instant::now();

    let mut prev = previous_processes.lock().await;
    let mut processes = Vec::with_capacity(native_procs.len());
    let mut total_disk_read = 0u64;
    let mut total_disk_write = 0u64;

    // Get CPU count for proper percentage capping
    let cpu_count = *CPU_COUNT.get_or_init(|| unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    });

    for proc in &native_procs {
        total_disk_read += proc.read_bytes;
        total_disk_write += proc.write_bytes;

        // System Idle Process (PID 0) represents idle CPU time across all cores,
        // not actual work. Skip it to avoid showing 900%+ on multi-core systems.
        let cpu_percent = if proc.pid == 0 {
            0.0
        } else if let Some((prev_time, prev_create, prev_instant)) = prev.get(&proc.pid) {
            if *prev_create != proc.create_time {
                // PID was reused, reset
                0.0
            } else {
                let total_time = proc.kernel_time + proc.user_time;
                let time_delta = total_time.saturating_sub(*prev_time);
                let elapsed = now.duration_since(*prev_instant).as_nanos() as f64;
                if elapsed > 0.0 {
                    // time_delta is in 100-ns intervals, elapsed is in ns
                    // Convert to percentage of TOTAL system CPU (matches Task Manager)
                    // Formula: (cpu_time_ns / wall_time_ns) / num_cores * 100
                    let cpu_time_ns = time_delta as f64 * 100.0;
                    let single_core_fraction = cpu_time_ns / elapsed;
                    let system_pct = (single_core_fraction / cpu_count as f64) * 100.0;
                    (system_pct as f32).clamp(0.0, 100.0)
                } else {
                    0.0
                }
            }
        } else {
            0.0
        };

        let memory_percent = if total_memory > 0 {
            (proc.working_set as f64 / total_memory as f64 * 100.0) as f32
        } else {
            0.0
        };

        let start_time = filetime_to_unix(proc.create_time);

        processes.push(ProcessInfo {
            pid: proc.pid,
            name: proc.name.clone(),
            cpu_percent,
            memory_percent,
            memory_mb: proc.working_set as f64 / (1024.0 * 1024.0),
            virtual_memory_mb: proc.private_bytes as f64 / (1024.0 * 1024.0),
            disk_read_bytes: proc.read_bytes,
            disk_write_bytes: proc.write_bytes,
            status: "Running".to_string(),
            start_time: start_time as i64,
            command: proc.name.clone(),
        });

        // Update cache with current CPU times
        let total_time = proc.kernel_time + proc.user_time;
        prev.insert(proc.pid, (total_time, proc.create_time, now));
    }

    // Sort by CPU and take top 20
    processes.sort_by(|a, b| {
        b.cpu_percent
            .partial_cmp(&a.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    processes.truncate(20);

    (processes, total_disk_read, total_disk_write)
}

#[inline]
fn filetime_to_unix(filetime: u64) -> u64 {
    filetime.saturating_sub(116444736000000000) / 10_000_000
}

// ============================================================================
// NETWORK CONNECTIONS
// ============================================================================

pub async fn get_network_connections() -> Vec<NetworkConnection> {
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
            connections.push(NetworkConnection {
                protocol: parts[0].to_string(),
                local_addr: parts[1].to_string(),
                remote_addr: parts[2].to_string(),
                status: if parts.len() > 3 {
                    parts[3].to_string()
                } else {
                    "NONE".to_string()
                },
            });
        }
    }

    connections
}

// ============================================================================
// NPU DETECTION
// ============================================================================

fn get_npu_utilization() -> Option<f32> {
    use std::collections::HashMap;
    use wmi::WMIConnection;

    let wmi_con = WMIConnection::new().ok()?;
    let npu_luid = NPU_LUID.get_or_init(|| discover_npu_luid(&wmi_con));

    if let Some(luid) = npu_luid {
        let query = format!(
            "SELECT UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine WHERE Name LIKE '%{}%'",
            luid
        );

        if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(&query) {
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
                return Some((total_util as f32).min(100.0));
            }
        }
    }

    None
}

fn discover_npu_luid(wmi_con: &wmi::WMIConnection) -> Option<String> {
    use std::collections::HashMap;
    use std::collections::HashSet;

    if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
        "SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine",
    ) {
        let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();

        for counter in &results {
            if let Some(wmi::Variant::String(name)) = counter.get("Name")
                && let Some(luid_start) = name.find("luid_")
                    && let Some(phys_start) = name.find("_phys_") {
                        let luid = &name[luid_start..phys_start];
                        if let Some(engtype_start) = name.find("_engtype_") {
                            let engtype = &name[engtype_start + 9..];
                            luid_engines
                                .entry(luid.to_string())
                                .or_default()
                                .insert(engtype.to_string());
                        }
                    }
        }

        for (luid, engine_types) in &luid_engines {
            if engine_types.len() == 1 && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }

        for (luid, engine_types) in &luid_engines {
            if !engine_types.contains("3D") && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }
    }

    None
}

fn detect_npu_dxcore() -> Option<(String, u32)> {
    use windows::core::GUID;
    use windows::Win32::Graphics::DXCore::*;

    const DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU: GUID = GUID::from_values(
        0xb69eb219,
        0x3ded,
        0x4464,
        [0x97, 0x9f, 0xa0, 0xb0, 0x33, 0x98, 0x87, 0x18],
    );

    unsafe {
        let factory: IDXCoreAdapterFactory = DXCoreCreateAdapterFactory().ok()?;
        let attributes = [DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU];
        let adapter_list: IDXCoreAdapterList = factory.CreateAdapterList(&attributes).ok()?;

        let adapter_count = adapter_list.GetAdapterCount();
        if adapter_count == 0 {
            return None;
        }

        let adapter: IDXCoreAdapter = adapter_list.GetAdapter(0).ok()?;

        let desc_size = adapter.GetPropertySize(DriverDescription).ok()?;
        if desc_size == 0 {
            return Some(("NPU Detected".to_string(), adapter_count));
        }

        let mut desc_buffer: Vec<u8> = vec![0; desc_size];
        if adapter
            .GetProperty(DriverDescription, desc_size, desc_buffer.as_mut_ptr() as *mut _)
            .is_err()
        {
            return Some(("NPU Detected".to_string(), adapter_count));
        }

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

fn detect_npu_wmi_fallback() -> (bool, Option<String>, Option<f32>) {
    use std::collections::HashMap;
    use wmi::WMIConnection;

    if let Ok(wmi_con) = WMIConnection::new() {
        if let Ok(results) = wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT Name, DeviceID, Description FROM Win32_PnPEntity WHERE \
             Name LIKE '%NPU%' OR \
             Name LIKE '%Neural%' OR \
             Name LIKE '%AI Accelerator%' OR \
             Name LIKE '%Hexagon%' OR \
             Description LIKE '%Neural Processing%' OR \
             DeviceID LIKE '%NPU%'",
        )
            && let Some(device) = results.first()
                && let Some(wmi::Variant::String(name)) = device.get("Name") {
                    return (true, Some(name.clone()), None);
                }

        if let Ok(cpu_results) =
            wmi_con.raw_query::<HashMap<String, wmi::Variant>>("SELECT Name FROM Win32_Processor")
            && let Some(cpu) = cpu_results.first()
                && let Some(wmi::Variant::String(name)) = cpu.get("Name") {
                    let name_lower = name.to_lowercase();

                    if name_lower.contains("core ultra")
                        || name_lower.contains("meteor lake")
                        || name_lower.contains("arrow lake")
                        || name_lower.contains("lunar lake")
                    {
                        return (true, Some(format!("{} (Intel NPU)", name)), None);
                    }

                    if name_lower.contains("ryzen ai")
                        || name_lower.contains("ryzen 8000")
                        || name_lower.contains("ryzen 9000")
                    {
                        return (true, Some(format!("{} (AMD XDNA NPU)", name)), None);
                    }

                    if name_lower.contains("snapdragon")
                        || name_lower.contains("x elite")
                        || name_lower.contains("x plus")
                    {
                        return (true, Some(format!("{} (Qualcomm Hexagon NPU)", name)), None);
                    }
                }
    }

    (false, None, None)
}

// ============================================================================
// UPTIME
// ============================================================================

pub fn get_uptime_seconds() -> u64 {
    unsafe { GetTickCount64() / 1000 }
}
