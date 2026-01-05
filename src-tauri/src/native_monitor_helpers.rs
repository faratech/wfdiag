//! Standalone system monitoring helpers for the egui binary.
//! Provides stats collection without requiring Tauri's AppHandle.

#![cfg(windows)]

use crate::native_monitor::{DiskInfo, ProcessArch, ProcessInfo, SystemStats};
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Cached NPU info - detected once at first access
static NPU_CACHE: OnceLock<(bool, Option<String>)> = OnceLock::new();

/// Cached NPU utilization - updated periodically
static NPU_UTIL_CACHE: std::sync::Mutex<Option<f32>> = std::sync::Mutex::new(None);
use windows::core::PCWSTR;
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY,
};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
};

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

// Reusable buffer for NtQuerySystemInformation
thread_local! {
    static PROCESS_BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::with_capacity(2 * 1024 * 1024));
}

const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOVABLE: u32 = 2;

// PDH wrapper for Send trait
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

/// PDH state wrapper that can be shared
pub struct PdhStateWrapper {
    query: SendPtr,
    counters: Vec<SendPtr>,
    initialized: bool,
    first_sample_done: bool,
}

impl Default for PdhStateWrapper {
    fn default() -> Self {
        Self {
            query: SendPtr(std::ptr::null_mut()),
            counters: Vec::new(),
            initialized: false,
            first_sample_done: false,
        }
    }
}

impl Drop for PdhStateWrapper {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                let _ = PdhCloseQuery(self.query.as_query());
            }
        }
    }
}

/// Network state wrapper
pub struct NetworkStateWrapper {
    bytes: HashMap<String, (u64, u64)>,
    last_update: Instant,
}

impl Default for NetworkStateWrapper {
    fn default() -> Self {
        Self {
            bytes: HashMap::new(),
            last_update: Instant::now(),
        }
    }
}

/// Cached disk info
pub struct CachedDiskInfo {
    pub disks: Vec<DiskInfo>,
    pub last_update: Instant,
}

/// Cached NPU utilization
pub struct CachedNpuUtilization {
    pub utilization: Option<f32>,
    pub last_update: Instant,
}

/// Fast stats for caching
#[derive(Clone, Default)]
pub struct FastStats {
    pub cpu_utilization: f32,
    pub per_cpu_utilization: Vec<f32>,
    pub cpu_frequency: u64,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub network_upload_kb: f64,
    pub network_download_kb: f64,
    pub top_processes: Vec<ProcessInfo>,
    pub all_processes: Vec<ProcessInfo>,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
}

/// Collect system stats standalone (without Tauri)
pub async fn collect_stats_standalone(
    pdh_state: &Arc<std::sync::Mutex<PdhStateWrapper>>,
    previous_network: &Arc<Mutex<NetworkStateWrapper>>,
    previous_processes: &Arc<Mutex<ProcessCpuCache>>,
    cached_disks: &Arc<Mutex<Option<CachedDiskInfo>>>,
    _cached_npu_util: &Arc<Mutex<Option<CachedNpuUtilization>>>,
    cached_fast_stats: &Arc<Mutex<Option<FastStats>>>,
    disk_update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    _npu_update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    fast_update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    tick: u64,
) -> SystemStats {
    // Get CPU count
    let cpu_count = unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    };

    // Collect fast stats (CPU, Memory, Network, Processes)
    let fast_stats = get_fast_stats_cached(
        pdh_state,
        previous_network,
        previous_processes,
        cached_fast_stats,
        fast_update_in_progress,
        cpu_count,
    )
    .await;

    // Calculate derived metrics
    let memory_utilization = if fast_stats.memory_total > 0 {
        (fast_stats.memory_used as f32 / fast_stats.memory_total as f32) * 100.0
    } else {
        0.0
    };
    let swap_utilization = if fast_stats.swap_total > 0 {
        (fast_stats.swap_used as f32 / fast_stats.swap_total as f32) * 100.0
    } else {
        0.0
    };

    // Get disk info
    let disks = get_disks_cached(cached_disks, disk_update_in_progress, tick).await;
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

    // Get NPU info - cache detection, only query utilization each time
    let (npu_available, npu_name) = if let Some(cached) = NPU_CACHE.get() {
        cached.clone()
    } else {
        // First time - run detection in spawn_blocking
        let detected = tokio::task::spawn_blocking(|| {
            let info = crate::native_monitor::get_npu_diagnostic_info();
            let detected = info.get("detected").and_then(|v| v.as_bool()).unwrap_or(false);
            let name = info.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
            (detected, name)
        }).await.unwrap_or((false, None));
        // Cache it (might race but that's ok)
        let _ = NPU_CACHE.set(detected.clone());
        detected
    };

    // Query utilization in background (faster than detection) - but not every frame
    // Only query every few ticks to avoid slowdown, cache the result
    let npu_utilization = if npu_available {
        if tick % 5 == 0 {
            // Query fresh value
            let fresh = tokio::task::spawn_blocking(crate::native_monitor::get_npu_utilization)
                .await
                .unwrap_or(None);
            // Cache it
            if let Ok(mut cache) = NPU_UTIL_CACHE.lock() {
                *cache = fresh;
            }
            fresh
        } else {
            // Return cached value
            NPU_UTIL_CACHE.lock().ok().and_then(|c| *c)
        }
    } else {
        None
    };

    SystemStats {
        cpu_utilization: fast_stats.cpu_utilization,
        per_cpu_utilization: fast_stats.per_cpu_utilization,
        cpu_frequency: fast_stats.cpu_frequency,
        memory_total_gb: fast_stats.memory_total as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_used_gb: fast_stats.memory_used as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_available_gb: fast_stats.memory_available as f64 / (1024.0 * 1024.0 * 1024.0),
        memory_utilization,
        swap_total_gb: fast_stats.swap_total as f64 / (1024.0 * 1024.0 * 1024.0),
        swap_used_gb: fast_stats.swap_used as f64 / (1024.0 * 1024.0 * 1024.0),
        swap_utilization,
        disk_utilization,
        disk_read_bytes: fast_stats.disk_read_bytes,
        disk_write_bytes: fast_stats.disk_write_bytes,
        disks,
        network_upload_kb: fast_stats.network_upload_kb,
        network_download_kb: fast_stats.network_download_kb,
        npu_available,
        npu_name,
        npu_utilization,
        top_processes: fast_stats.top_processes,
        all_processes: fast_stats.all_processes,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    }
}

async fn get_fast_stats_cached(
    pdh_state: &Arc<std::sync::Mutex<PdhStateWrapper>>,
    previous_network: &Arc<Mutex<NetworkStateWrapper>>,
    previous_processes: &Arc<Mutex<ProcessCpuCache>>,
    cached: &Arc<Mutex<Option<FastStats>>>,
    update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    cpu_count: usize,
) -> FastStats {
    // If no update is in progress, spawn one
    if !update_in_progress.swap(true, std::sync::atomic::Ordering::Acquire) {
        let pdh_clone = Arc::clone(pdh_state);
        let net_clone = Arc::clone(previous_network);
        let proc_clone = Arc::clone(previous_processes);
        let cached_clone = Arc::clone(cached);
        let flag_clone = Arc::clone(update_in_progress);

        tokio::spawn(async move {
            let stats = tokio::task::spawn_blocking(move || {
                // CPU stats via PDH
                let (cpu_utilization, per_cpu_utilization, swap_utilization) =
                    get_pdh_stats(&pdh_clone, cpu_count);
                let cpu_frequency = get_cpu_frequency();

                // Memory stats
                let (memory_total, memory_used, memory_available, swap_total, _) = get_memory_info();
                let swap_used = if swap_total > 0 {
                    (swap_total as f32 * (swap_utilization / 100.0)) as u64
                } else {
                    0
                };

                // Network stats
                let (network_upload_kb, network_download_kb) =
                    tokio::runtime::Handle::current().block_on(async {
                        get_network_stats(&net_clone).await
                    });

                // Process stats
                let (all_procs, disk_read_bytes, disk_write_bytes) =
                    tokio::runtime::Handle::current().block_on(async {
                        get_all_processes_internal(memory_total, &proc_clone, cpu_count).await
                    });

                // Top 15 processes sorted by CPU
                let top_processes: Vec<ProcessInfo> = all_procs.iter().take(15).cloned().collect();

                FastStats {
                    cpu_utilization,
                    per_cpu_utilization,
                    cpu_frequency,
                    memory_total,
                    memory_used,
                    memory_available,
                    swap_total,
                    swap_used,
                    network_upload_kb,
                    network_download_kb,
                    top_processes,
                    all_processes: all_procs,
                    disk_read_bytes,
                    disk_write_bytes,
                }
            })
            .await
            .ok();

            if let Some(new_stats) = stats {
                let mut cache_lock = cached_clone.lock().await;
                *cache_lock = Some(new_stats);
            }

            flag_clone.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    // Return cached value or default
    let cache = cached.lock().await;
    cache.clone().unwrap_or_default()
}

fn get_pdh_stats(
    pdh_state: &Arc<std::sync::Mutex<PdhStateWrapper>>,
    cpu_count: usize,
) -> (f32, Vec<f32>, f32) {
    let mut state = pdh_state.lock().unwrap_or_else(|e| e.into_inner());

    if !state.initialized {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return (0.0, vec![0.0; cpu_count], 0.0);
            }
            state.query = SendPtr(query.0);

            // CPU Counters
            state.counters.reserve(cpu_count + 1);
            for i in 0..cpu_count {
                let path = format!("\\Processor({})\\% Processor Time\0", i);
                let path_wide: Vec<u16> = path.encode_utf16().collect();
                let mut counter = PDH_HCOUNTER::default();
                let status =
                    PdhAddEnglishCounterW(query, PCWSTR(path_wide.as_ptr()), 0, &mut counter);
                if status == 0 {
                    state.counters.push(SendPtr(counter.0));
                } else {
                    state.counters.push(SendPtr(std::ptr::null_mut()));
                }
            }

            // Swap Counter
            let path_wide: Vec<u16> = "\\Paging File(_Total)\\% Usage\0".encode_utf16().collect();
            let mut counter = PDH_HCOUNTER::default();
            let status =
                PdhAddEnglishCounterW(query, PCWSTR(path_wide.as_ptr()), 0, &mut counter);
            if status == 0 {
                state.counters.push(SendPtr(counter.0));
            } else {
                state.counters.push(SendPtr(std::ptr::null_mut()));
            }

            state.initialized = true;
        }
    }

    unsafe {
        let status = PdhCollectQueryData(state.query.as_query());
        if status != 0 {
            return (0.0, vec![0.0; cpu_count], 0.0);
        }
    }

    if !state.first_sample_done {
        state.first_sample_done = true;
        return (0.0, vec![0.0; cpu_count], 0.0);
    }

    let mut per_cpu = Vec::with_capacity(cpu_count);
    let mut total_cpu = 0.0f32;

    for i in 0..cpu_count {
        if i >= state.counters.len() {
            break;
        }

        let counter_ptr = state.counters[i].0;
        if counter_ptr.is_null() {
            per_cpu.push(0.0);
            continue;
        }

        let mut value = PDH_FMT_COUNTERVALUE::default();
        let status = unsafe {
            PdhGetFormattedCounterValue(PDH_HCOUNTER(counter_ptr), PDH_FMT_DOUBLE, None, &mut value)
        };
        let cpu_pct = if status == 0 && value.CStatus == PDH_CSTATUS_VALID_DATA {
            unsafe { (value.Anonymous.doubleValue as f32).clamp(0.0, 100.0) }
        } else {
            0.0
        };
        per_cpu.push(cpu_pct);
        total_cpu += cpu_pct;
    }

    let avg_cpu = if !per_cpu.is_empty() {
        total_cpu / per_cpu.len() as f32
    } else {
        0.0
    };

    // Swap counter
    let swap_util = if state.counters.len() > cpu_count {
        let counter_ptr = state.counters[cpu_count].0;
        if !counter_ptr.is_null() {
            let mut value = PDH_FMT_COUNTERVALUE::default();
            let status = unsafe {
                PdhGetFormattedCounterValue(PDH_HCOUNTER(counter_ptr), PDH_FMT_DOUBLE, None, &mut value)
            };
            if status == 0 && value.CStatus == PDH_CSTATUS_VALID_DATA {
                unsafe { (value.Anonymous.doubleValue as f32).clamp(0.0, 100.0) }
            } else {
                0.0
            }
        } else {
            0.0
        }
    } else {
        0.0
    };

    (avg_cpu, per_cpu, swap_util)
}

fn get_cpu_frequency() -> u64 {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(cpu_key) = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0") {
        if let Ok(mhz) = cpu_key.get_value::<u32, _>("~MHz") {
            return mhz as u64;
        }
    }
    0
}

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

            let (swap_total, swap_used) = if GetPerformanceInfo(&mut perf_info, perf_info.cb).is_ok() {
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

async fn get_network_stats(previous_network: &Arc<Mutex<NetworkStateWrapper>>) -> (f64, f64) {
    let mut current_stats: HashMap<String, (u64, u64)> = HashMap::new();

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

    let now = Instant::now();
    let mut prev_state = previous_network.lock().await;

    let mut upload_kb_per_sec = 0.0;
    let mut download_kb_per_sec = 0.0;

    let elapsed = now.duration_since(prev_state.last_update).as_secs_f64();
    prev_state.last_update = now;

    if !prev_state.bytes.is_empty() && elapsed > 0.001 {
        for (interface, (tx, rx)) in &current_stats {
            if let Some((prev_tx, prev_rx)) = prev_state.bytes.get(interface) {
                let tx_delta = tx.saturating_sub(*prev_tx) as f64;
                let rx_delta = rx.saturating_sub(*prev_rx) as f64;

                upload_kb_per_sec += (tx_delta / 1024.0) / elapsed;
                download_kb_per_sec += (rx_delta / 1024.0) / elapsed;
            }
        }
    }

    prev_state.bytes = current_stats;
    (upload_kb_per_sec, download_kb_per_sec)
}

async fn get_disks_cached(
    cached: &Arc<Mutex<Option<CachedDiskInfo>>>,
    update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    tick: u64,
) -> Vec<DiskInfo> {
    let cache_guard = cached.lock().await;

    let should_refresh = match &*cache_guard {
        Some(c) => tick % 5 == 0 || c.last_update.elapsed() > Duration::from_secs(5),
        None => true,
    };

    if should_refresh && !update_in_progress.swap(true, std::sync::atomic::Ordering::Acquire) {
        let cached_clone = Arc::clone(cached);
        let update_flag = Arc::clone(update_in_progress);

        tokio::spawn(async move {
            let disks = tokio::task::spawn_blocking(get_disk_info)
                .await
                .unwrap_or_default();

            let mut cache_lock = cached_clone.lock().await;
            *cache_lock = Some(CachedDiskInfo {
                disks,
                last_update: Instant::now(),
            });

            update_flag.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    match &*cache_guard {
        Some(c) => c.disks.clone(),
        None => Vec::new(),
    }
}

fn get_disk_info() -> Vec<DiskInfo> {
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

                disks.push(DiskInfo {
                    name: drive_str.trim_end_matches('\\').to_string(),
                    mount_point: drive_str.clone(),
                    total_gb: total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    used_gb: used_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    available_gb: total_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    utilization,
                    file_system: "NTFS".to_string(),
                    disk_type: "SSD".to_string(), // Default, could enhance later
                });
            }

            i += 1;
        }
    }

    disks
}

// ============================================================================
// PROCESS ENUMERATION
// ============================================================================

/// Cached process CPU times for delta calculation
pub type ProcessCpuCache = HashMap<u32, (u64, u64, Instant)>; // pid -> (cpu_time, create_time, instant)

/// Internal async wrapper for process collection with disk I/O totals
async fn get_all_processes_internal(
    total_memory: u64,
    previous_processes: &Arc<Mutex<ProcessCpuCache>>,
    cpu_count: usize,
) -> (Vec<ProcessInfo>, u64, u64) {
    let mut prev = previous_processes.lock().await;
    let processes = get_all_processes_standalone(total_memory, &mut prev, cpu_count);

    // Calculate disk I/O totals
    let disk_read_bytes: u64 = processes.iter().map(|p| p.io_read_bytes).sum();
    let disk_write_bytes: u64 = processes.iter().map(|p| p.io_write_bytes).sum();

    (processes, disk_read_bytes, disk_write_bytes)
}

/// Get all processes with CPU percentages
pub fn get_all_processes_standalone(
    total_memory: u64,
    previous_processes: &mut ProcessCpuCache,
    cpu_count: usize,
) -> Vec<ProcessInfo> {
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

        let now = Instant::now();
        let mut processes = Vec::with_capacity(300);
        let mut offset: usize = 0;

        loop {
            let proc_info = unsafe { &*(buffer.as_ptr().add(offset) as *const SystemProcessInfo) };

            let pid = proc_info.unique_process_id.0 as usize as u32;
            let parent_pid = proc_info.inherited_from_unique_process_id.0 as usize as u32;

            let name = if proc_info.image_name.Length > 0 && !proc_info.image_name.Buffer.is_null() {
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        proc_info.image_name.Buffer.0,
                        (proc_info.image_name.Length / 2) as usize,
                    )
                };
                OsString::from_wide(slice).to_string_lossy().into_owned()
            } else if pid == 0 {
                "System Idle Process".to_string()
            } else {
                "System".to_string()
            };

            // Calculate CPU percentage
            let total_time = (proc_info.kernel_time + proc_info.user_time) as u64;
            let create_time = proc_info.create_time as u64;

            let cpu_percent = if pid == 0 {
                0.0 // System Idle Process
            } else if let Some((prev_time, prev_create, prev_instant)) = previous_processes.get(&pid) {
                if *prev_create != create_time {
                    0.0 // PID reused
                } else {
                    let time_delta = total_time.saturating_sub(*prev_time);
                    let elapsed = now.duration_since(*prev_instant).as_nanos() as f64;
                    if elapsed > 0.0 {
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
                (proc_info.working_set_size as f64 / total_memory as f64 * 100.0) as f32
            } else {
                0.0
            };

            let cpu_time_secs = total_time / 10_000_000;
            let shared_memory = (proc_info.working_set_size as u64)
                .saturating_sub(proc_info.private_page_count as u64);
            let start_time = filetime_to_unix(create_time);
            let name_lower = name.to_lowercase();

            processes.push(ProcessInfo {
                pid,
                parent_pid,
                name: name.clone(),
                exe_path: String::new(),
                command: name.clone(),
                user: String::new(),
                cpu_percent,
                memory_percent,
                memory_mb: proc_info.working_set_size as f64 / (1024.0 * 1024.0),
                virtual_memory_mb: proc_info.pagefile_usage as f64 / (1024.0 * 1024.0),
                shared_memory_mb: shared_memory as f64 / (1024.0 * 1024.0),
                cpu_time_secs,
                start_time: start_time as i64,
                status: "Running".to_string(),
                thread_count: proc_info.number_of_threads,
                handle_count: proc_info.handle_count,
                priority: proc_info.base_priority,
                io_read_bytes: proc_info.read_transfer_count as u64,
                io_write_bytes: proc_info.write_transfer_count as u64,
                is_elevated: false,
                efficiency_mode: false,
                arch: ProcessArch::Native,
                tree_depth: 0,
                tree_prefix: String::new(),
                has_children: false,
                is_collapsed: false,
                name_lower: name_lower.clone(),
                command_lower: name_lower,
            });

            // Update cache
            previous_processes.insert(pid, (total_time, create_time, now));

            if proc_info.next_entry_offset == 0 {
                break;
            }
            offset += proc_info.next_entry_offset as usize;
        }

        // Sort by CPU descending
        processes.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Clean up stale PIDs from cache (processes that no longer exist)
        let current_pids: std::collections::HashSet<u32> = processes.iter().map(|p| p.pid).collect();
        previous_processes.retain(|pid, _| current_pids.contains(pid));

        processes
    })
}

#[inline]
fn filetime_to_unix(filetime: u64) -> u64 {
    filetime.saturating_sub(116444736000000000) / 10_000_000
}
