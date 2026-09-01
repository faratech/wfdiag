//! Native Windows system monitoring using Windows APIs instead of sysinfo crate.
//! Optimized with aggressive caching for fluid, low-CPU monitoring.

#![cfg(windows)]

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::interval;
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetExtendedTcpTable, GetExtendedUdpTable, GetIfTable2, MIB_IF_TABLE2,
    MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};
use windows::Win32::System::Performance::{
    PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW,
};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows::core::PCWSTR;

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

/// Prevents duplicate slow DXCore/WMI NPU detection work.
static NPU_DETECTION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

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

/// Selects the work owned by the one-second system telemetry loop.
///
/// `Legacy` preserves the Tauri monitor contract, including its small
/// `top_processes` projection. Native shells should use `SystemOnly` and let
/// the Processes screen request its independent two-second snapshot instead.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MonitorProfile {
    Legacy { include_process_adapter_stats: bool },
    SystemOnly,
}

impl Default for MonitorProfile {
    fn default() -> Self {
        Self::Legacy {
            include_process_adapter_stats: false,
        }
    }
}

impl MonitorProfile {
    #[inline]
    const fn includes_processes(self) -> bool {
        matches!(self, Self::Legacy { .. })
    }

    #[inline]
    const fn includes_process_adapter_stats(self) -> bool {
        matches!(
            self,
            Self::Legacy {
                include_process_adapter_stats: true
            }
        )
    }
}

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
    /// Percentage of provisioned storage capacity currently occupied. This is
    /// intentionally distinct from disk busy/active time.
    pub storage_used_percent: f32,
    /// Backward-compatible alias for older frontends. New consumers should use
    /// `storage_used_percent`.
    pub disk_utilization: f32,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disks: Vec<DiskInfo>,
    pub network_upload_kb: f64,
    pub network_download_kb: f64,
    pub gpu_available: bool,
    pub gpu_name: Option<String>,
    pub gpu_utilization: Option<f32>,
    pub gpu_memory_used_mb: f64,
    pub gpu_memory_total_mb: f64,
    pub npu_available: bool,
    pub npu_name: Option<String>,
    pub npu_utilization: Option<f32>,
    pub npu_memory_used_mb: f64,
    pub npu_memory_total_mb: f64,
    pub top_processes: Vec<ProcessInfo>,
    /// Deprecated compatibility field. Process snapshots now belong to
    /// `list_processes`, so live telemetry always leaves this empty and avoids
    /// cloning the complete process table for every projected frame.
    #[deprecated(note = "use SystemMonitor::list_processes for full process snapshots")]
    #[serde(skip)]
    pub all_processes: Vec<ProcessInfo>,
    pub timestamp: i64,
}

/// UI-neutral sink for the live monitoring stream.
///
/// The current Tauri shell adapts this to its `system-stats` event while the
/// native Reactor shell can publish the same samples into its UI event bus.
/// Keeping the monitor unaware of either UI runtime also makes its lifetime
/// and backpressure behavior independently testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorEmission {
    Continue,
    Stop,
}

pub trait MonitorEmitter: Send + Sync {
    /// Cheap preflight used before collecting and projecting the next sample.
    fn accepts_system_stats(&self) -> bool {
        true
    }

    fn emit_system_stats(&self, stats: &SystemStats) -> MonitorEmission;
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

/// Process architecture (for mixed x86/x64/ARM64 environments)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProcessArch {
    #[default]
    Native,
    X86,
    X64,
    Arm64,
}

impl ProcessArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessArch::Native => "",
            ProcessArch::X86 => "x86",
            ProcessArch::X64 => "x64",
            ProcessArch::Arm64 => "ARM64",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    // Core identification
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub exe_path: String,
    pub command: String,
    pub user: String,

    // Performance metrics
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub memory_mb: f64,
    pub virtual_memory_mb: f64,
    pub shared_memory_mb: f64,
    pub gpu_percent: f32,
    pub gpu_memory_mb: f64,
    pub npu_percent: f32,
    pub npu_memory_mb: f64,

    // Timing
    pub cpu_time_secs: u64,
    pub start_time: i64,
    pub status: String,

    // Counts
    pub thread_count: u32,
    pub handle_count: u32,
    pub priority: i32,

    // I/O
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,

    // Windows-specific
    pub is_elevated: bool,
    pub efficiency_mode: bool,
    pub arch: ProcessArch,

    // Tree view fields
    pub tree_depth: usize,
    pub tree_prefix: String,
    pub has_children: bool,
    pub is_collapsed: bool,

    // Pre-computed for filtering (lowercase)
    #[serde(skip)]
    pub name_lower: String,
    #[serde(skip)]
    pub command_lower: String,
}

/// Fields supported by the on-demand process explorer. Monitoring continues
/// to emit only `top_processes`; this query is used only while Processes is
/// visible so the full list never rides the one-second stats event.
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSortKey {
    Name,
    Pid,
    #[default]
    CpuPercent,
    MemoryPercent,
    MemoryMb,
    Status,
    ThreadCount,
    GpuPercent,
    NpuPercent,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSortDirection {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessQuery {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub sort_by: ProcessSortKey,
    #[serde(default)]
    pub sort_direction: ProcessSortDirection,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_process_page_size")]
    pub limit: usize,
}

fn default_process_page_size() -> usize {
    100
}

impl Default for ProcessQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            sort_by: ProcessSortKey::default(),
            sort_direction: ProcessSortDirection::default(),
            offset: 0,
            limit: default_process_page_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessPage {
    pub captured_at: i64,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub items: Vec<ProcessRow>,
}

/// Truthful lightweight row for the process explorer. Fields that require an
/// expensive or privileged process-handle query are intentionally omitted
/// instead of being serialized as plausible-looking placeholders.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub memory_mb: f64,
    pub virtual_memory_mb: f64,
    pub gpu_percent: Option<f32>,
    pub gpu_memory_mb: Option<f64>,
    pub npu_percent: Option<f32>,
    pub npu_memory_mb: Option<f64>,
    pub cpu_time_secs: u64,
    pub start_time: i64,
    pub status: String,
    pub thread_count: u32,
    pub handle_count: u32,
    pub priority: i32,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

impl From<ProcessInfo> for ProcessRow {
    fn from(process: ProcessInfo) -> Self {
        Self::from(&process)
    }
}

impl From<&ProcessInfo> for ProcessRow {
    fn from(process: &ProcessInfo) -> Self {
        Self {
            pid: process.pid,
            parent_pid: process.parent_pid,
            name: process.name.clone(),
            cpu_percent: process.cpu_percent,
            memory_percent: process.memory_percent,
            memory_mb: process.memory_mb,
            virtual_memory_mb: process.virtual_memory_mb,
            gpu_percent: (process.gpu_percent > 0.0).then_some(process.gpu_percent),
            gpu_memory_mb: (process.gpu_memory_mb > 0.0).then_some(process.gpu_memory_mb),
            npu_percent: (process.npu_percent > 0.0).then_some(process.npu_percent),
            npu_memory_mb: (process.npu_memory_mb > 0.0).then_some(process.npu_memory_mb),
            cpu_time_secs: process.cpu_time_secs,
            start_time: process.start_time,
            status: process.status.clone(),
            thread_count: process.thread_count,
            handle_count: process.handle_count,
            priority: process.priority,
            io_read_bytes: process.io_read_bytes,
            io_write_bytes: process.io_write_bytes,
        }
    }
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
    parent_pid: u32,
    name: String,
    working_set: u64,
    private_bytes: u64,
    virtual_size: u64,
    kernel_time: u64,
    user_time: u64,
    create_time: u64,
    read_bytes: u64,
    write_bytes: u64,
    thread_count: u32,
    handle_count: u32,
    base_priority: i32,
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

#[derive(Clone)]
struct FastStats {
    profile: MonitorProfile,
    captured_at: Instant,
    cpu_utilization: f32,
    per_cpu_utilization: Vec<f32>,
    cpu_frequency: u64,
    memory_total: u64,
    memory_used: u64,
    memory_available: u64,
    swap_total: u64,
    swap_used: u64,
    network_upload_kb: f64,
    network_download_kb: f64,
    top_processes: Vec<ProcessInfo>,
    disk_read_bytes: u64,
    disk_write_bytes: u64,
    adapter_snapshot: crate::adapter_monitor::AdapterSnapshot,
}

// ============================================================================
// SYSTEM MONITOR
// ============================================================================

type NetworkBytes = HashMap<String, (u64, u64)>;
type ProcessCpuTimes = HashMap<u32, (u64, u64, Instant)>;

const FAST_STATS_MIN_AGE: Duration = Duration::from_millis(750);
pub const PROCESS_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(2);

struct ProcessSnapshot {
    processes: Vec<ProcessInfo>,
    captured_at: i64,
    captured_instant: Instant,
}

impl ProcessSnapshot {
    fn is_fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.captured_instant) < PROCESS_SNAPSHOT_INTERVAL
    }
}

struct NetworkState {
    bytes: NetworkBytes,
    last_update: Instant,
}

pub struct SystemMonitor {
    emitter: Arc<dyn MonitorEmitter>,
    monitoring: Arc<Mutex<bool>>,
    pdh_state: Arc<std::sync::Mutex<PdhState>>,
    previous_network: Arc<Mutex<NetworkState>>,
    // The legacy top-process sampler and the Processes page must never mutate
    // the same CPU-delta baseline. Each cadence owns one history exclusively.
    previous_processes: Arc<Mutex<ProcessCpuTimes>>,
    process_snapshot_history: Arc<Mutex<ProcessCpuTimes>>,
    process_snapshot: Arc<Mutex<Option<ProcessSnapshot>>>,
    process_snapshot_refresh: Arc<Mutex<()>>,
    update_interval: Arc<Mutex<Duration>>,
    // Cached slow-changing data
    cached_disks: Arc<Mutex<Option<CachedDiskInfo>>>,
    cached_npu_util: Arc<Mutex<Option<CachedNpuUtilization>>>,
    // Cached fast-changing data
    cached_fast_stats: Arc<Mutex<Option<FastStats>>>,
    monitor_profile: Arc<RwLock<MonitorProfile>>,
    // Tick counter for tiered refresh
    tick_count: Arc<AtomicU64>,
    // Async update flags
    disk_update_in_progress: Arc<AtomicBool>,
    npu_update_in_progress: Arc<AtomicBool>,
    fast_update_in_progress: Arc<AtomicBool>,
    // Single-owner monitoring loop: handle to the currently-spawned loop (for immediate
    // abort on stop/restart) and a generation token so a superseded loop self-terminates
    // on its next tick even if the shared `monitoring` flag has been re-armed.
    monitor_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    generation: Arc<AtomicU64>,
}

impl SystemMonitor {
    /// Construct a monitor for a non-Tauri shell.
    pub fn with_emitter(emitter: Arc<dyn MonitorEmitter>) -> Self {
        // Initialize fast caches synchronously (< 1ms)
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

        // Initialize slow caches in background (disk types only)
        // NPU detection is done lazily in spawn_blocking context for proper COM support
        std::thread::spawn(|| {
            // These can take 1-5 seconds
            DISK_TYPES.get_or_init(detect_all_disk_types);
            INIT_COMPLETE.store(true, std::sync::atomic::Ordering::Release);
        });

        Self {
            emitter,
            monitoring: Arc::new(Mutex::new(false)),
            pdh_state: Arc::new(std::sync::Mutex::new(PdhState::default())),
            previous_network: Arc::new(Mutex::new(NetworkState {
                bytes: HashMap::new(),
                last_update: Instant::now(),
            })),
            previous_processes: Arc::new(Mutex::new(HashMap::new())),
            process_snapshot_history: Arc::new(Mutex::new(HashMap::new())),
            process_snapshot: Arc::new(Mutex::new(None)),
            process_snapshot_refresh: Arc::new(Mutex::new(())),
            update_interval: Arc::new(Mutex::new(Duration::from_secs(1))),
            cached_disks: Arc::new(Mutex::new(None)),
            cached_npu_util: Arc::new(Mutex::new(None)),
            cached_fast_stats: Arc::new(Mutex::new(None)),
            monitor_profile: Arc::new(RwLock::new(MonitorProfile::default())),
            tick_count: Arc::new(AtomicU64::new(0)),
            disk_update_in_progress: Arc::new(AtomicBool::new(false)),
            npu_update_in_progress: Arc::new(AtomicBool::new(false)),
            fast_update_in_progress: Arc::new(AtomicBool::new(false)),
            monitor_task: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start the legacy Tauri-compatible telemetry profile.
    pub async fn start_monitoring(&self, include_process_adapter_stats: bool) {
        self.start_monitoring_with_profile(MonitorProfile::Legacy {
            include_process_adapter_stats,
        })
        .await;
    }

    /// Start one-second sampling with an explicit work profile.
    ///
    /// `MonitorProfile::SystemOnly` guarantees that the live telemetry path
    /// does not enumerate processes. Full process data remains available from
    /// `list_processes` on its independent two-second snapshot cadence.
    pub async fn start_monitoring_with_profile(&self, profile: MonitorProfile) {
        // Single-owner: supersede any previously-running loop. Bump the generation so a
        // still-running old loop exits on its next tick even if `monitoring` is re-armed
        // below, and abort its task handle for immediate teardown. This makes repeated
        // start_monitoring calls (rapid tab toggles, two monitoring components mounted at
        // once) idempotent instead of accumulating duplicate emission loops.
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut task = self.monitor_task.lock().await;
            if let Some(old) = task.take() {
                old.abort();
            }
        }

        let mut monitoring = self.monitoring.lock().await;
        *monitoring = true;
        drop(monitoring);
        *self
            .monitor_profile
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = profile;

        let emitter = Arc::clone(&self.emitter);
        let monitoring_flag = Arc::clone(&self.monitoring);
        let generation = Arc::clone(&self.generation);
        let pdh_state = Arc::clone(&self.pdh_state);
        let previous_network = Arc::clone(&self.previous_network);
        let previous_processes = Arc::clone(&self.previous_processes);
        let update_interval = Arc::clone(&self.update_interval);
        let cached_disks = Arc::clone(&self.cached_disks);
        let cached_npu_util = Arc::clone(&self.cached_npu_util);
        let cached_fast_stats = Arc::clone(&self.cached_fast_stats);
        let monitor_profile = Arc::clone(&self.monitor_profile);
        let tick_count = Arc::clone(&self.tick_count);
        let disk_update_in_progress = Arc::clone(&self.disk_update_in_progress);
        let npu_update_in_progress = Arc::clone(&self.npu_update_in_progress);
        let fast_update_in_progress = Arc::clone(&self.fast_update_in_progress);

        // Pre-sample PDH to get accurate CPU% on first event
        // PDH requires two samples to calculate delta, so we take the first sample here
        // and wait briefly before starting the monitoring loop
        {
            let pdh_clone = Arc::clone(&self.pdh_state);
            tokio::task::spawn_blocking(move || {
                if let Ok(mut state) = pdh_clone.lock() {
                    // Initialize PDH if needed and collect first sample
                    if !state.initialized {
                        unsafe {
                            let mut query_handle: PDH_HQUERY = PDH_HQUERY::default();
                            // PDH functions return 0 on success
                            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query_handle) == 0 {
                                state.query = SendPtr(query_handle.0);
                                let cpu_count = *CPU_COUNT.get_or_init(|| {
                                    let mut sys_info = SYSTEM_INFO::default();
                                    GetSystemInfo(&mut sys_info);
                                    sys_info.dwNumberOfProcessors as usize
                                });
                                for i in 0..cpu_count {
                                    let counter_path: Vec<u16> =
                                        format!("\\Processor({})\\% Processor Time\0", i)
                                            .encode_utf16()
                                            .collect();
                                    let mut counter_handle: PDH_HCOUNTER = PDH_HCOUNTER::default();
                                    // PDH functions return 0 on success
                                    if PdhAddEnglishCounterW(
                                        query_handle,
                                        PCWSTR::from_raw(counter_path.as_ptr()),
                                        0,
                                        &mut counter_handle,
                                    ) == 0
                                    {
                                        state.counters.push(SendPtr(counter_handle.0));
                                    }
                                }
                                state.initialized = true;
                            }
                        }
                    }
                    // Collect first sample
                    if state.initialized {
                        unsafe {
                            let _ = PdhCollectQueryData(state.query.as_query());
                        }
                        state.first_sample_done = true;
                    }
                }
            })
            .await
            .ok();
        }

        // Wait briefly for meaningful delta between samples
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Seed the fast-stats cache synchronously so the FIRST emitted frame carries real
        // data. Without this, the interval's immediate first tick reads an empty cache and
        // emits an all-zeros sample (0% CPU, 0 GB, no processes).
        {
            let pdh_clone = Arc::clone(&self.pdh_state);
            let net_clone = Arc::clone(&self.previous_network);
            let proc_clone = Arc::clone(&self.previous_processes);
            let initial = tokio::task::spawn_blocking(move || {
                compute_fast_stats(&pdh_clone, &net_clone, &proc_clone, profile)
            })
            .await
            .ok();
            if let Some(initial) = initial {
                *self.cached_fast_stats.lock().await = Some(initial);
            }
        }

        let handle = tokio::spawn(async move {
            let interval_duration = *update_interval.lock().await;
            let mut interval = interval(interval_duration);

            loop {
                interval.tick().await;

                let monitoring = monitoring_flag.lock().await;
                if !*monitoring {
                    break;
                }
                drop(monitoring);

                // Superseded by a newer start_monitoring → this loop is stale, exit.
                if generation.load(Ordering::SeqCst) != my_gen {
                    break;
                }

                // A native UI receiver owns the event bus lifetime. Stop before
                // doing another expensive sample/projection once that receiver
                // has gone away.
                if !emitter.accepts_system_stats() {
                    let mut monitoring = monitoring_flag.lock().await;
                    if generation.load(Ordering::SeqCst) == my_gen {
                        *monitoring = false;
                    }
                    break;
                }

                let tick = tick_count.fetch_add(1, Ordering::Relaxed);

                let stats = collect_stats_optimized(
                    &pdh_state,
                    &previous_network,
                    &previous_processes,
                    &cached_disks,
                    &cached_npu_util,
                    &cached_fast_stats,
                    &monitor_profile,
                    &disk_update_in_progress,
                    &npu_update_in_progress,
                    &fast_update_in_progress,
                    tick,
                )
                .await;

                if emitter.emit_system_stats(&stats) == MonitorEmission::Stop {
                    let mut monitoring = monitoring_flag.lock().await;
                    if generation.load(Ordering::SeqCst) == my_gen {
                        *monitoring = false;
                    }
                    break;
                }
            }
        });

        // Record the loop handle so stop/restart can abort it immediately.
        *self.monitor_task.lock().await = Some(handle);
    }

    pub async fn stop_monitoring(&self) {
        let mut monitoring = self.monitoring.lock().await;
        *monitoring = false;
        drop(monitoring);

        // Bump the generation (so any loop that wakes between now and abort exits) and
        // abort the spawned task for immediate teardown rather than waiting up to one tick.
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut task = self.monitor_task.lock().await;
        if let Some(handle) = task.take() {
            handle.abort();
        }
    }

    pub async fn get_current_stats(&self) -> SystemStats {
        let tick = self.tick_count.load(Ordering::Relaxed);
        collect_stats_optimized(
            &self.pdh_state,
            &self.previous_network,
            &self.previous_processes,
            &self.cached_disks,
            &self.cached_npu_util,
            &self.cached_fast_stats,
            &self.monitor_profile,
            &self.disk_update_in_progress,
            &self.npu_update_in_progress,
            &self.fast_update_in_progress,
            tick,
        )
        .await
    }

    /// Query the complete current process set for the Processes screen. The
    /// high-frequency monitor event deliberately stays small; full enumeration
    /// happens only on demand and is filtered/sorted before crossing IPC. A
    /// capture is reused for two seconds so search, sort, and pagination do not
    /// perturb the per-process CPU delta or trigger redundant enumeration.
    pub async fn list_processes(&self, query: ProcessQuery) -> ProcessPage {
        let _refresh_owner = self.process_snapshot_refresh.lock().await;
        let now = Instant::now();

        {
            let snapshot = self.process_snapshot.lock().await;
            if let Some(snapshot) = snapshot.as_ref()
                && snapshot.is_fresh_at(now)
            {
                return paginate_process_snapshot(&snapshot.processes, snapshot.captured_at, query);
            }
        }

        let processes = get_all_processes(&self.process_snapshot_history).await;
        let captured_at = unix_timestamp_secs();
        let mut snapshot = self.process_snapshot.lock().await;
        *snapshot = Some(ProcessSnapshot {
            processes,
            captured_at,
            captured_instant: now,
        });
        let snapshot = snapshot.as_ref().expect("process snapshot was just stored");
        paginate_process_snapshot(&snapshot.processes, snapshot.captured_at, query)
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

/// Pre-warm NPU cache in background. Called from lib.rs setup hook.
/// This prevents the 200-500ms delay on first monitoring start.
pub fn prewarm_npu_cache() {
    if NPU_INFO.get().is_some() && NPU_LUID.get().is_some() {
        return;
    }

    if NPU_DETECTION_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    struct DetectionGuard;
    impl Drop for DetectionGuard {
        fn drop(&mut self) {
            NPU_DETECTION_IN_PROGRESS.store(false, Ordering::Release);
        }
    }
    let _guard = DetectionGuard;

    // Initialize NPU detection (can take 200-500ms for DXCore/WMI queries)
    let npu_result = NPU_INFO.get_or_init(detect_npu_info_uncached);

    // If NPU was detected, also discover LUID for utilization queries
    if npu_result.0 {
        if let Ok(wmi_con) = crate::wmi_native::WmiConnection::new() {
            NPU_LUID.get_or_init(|| discover_npu_luid_with_wmi(&wmi_con));
        }
    } else {
        NPU_LUID.get_or_init(|| None);
    }
}

fn detect_npu_info_uncached() -> (bool, Option<String>) {
    // Device evidence only (DXCore, then a PnP device match). The CPU-model
    // heuristic deliberately does not count: Live Monitor must not claim an
    // NPU that isn't actually present/enabled.
    if let Some((name, _)) = detect_npu_dxcore() {
        (true, Some(name))
    } else {
        match detect_npu_pnp_device() {
            Some(name) => (true, Some(name)),
            None => (false, None),
        }
    }
}

fn schedule_npu_cache_warmup() {
    if NPU_INFO.get().is_some()
        || NPU_DETECTION_IN_PROGRESS.load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }

    tokio::task::spawn_blocking(prewarm_npu_cache);
}

/// Pre-initialize all static caches. Call this at app startup for faster first display.
/// Note: Caches are also initialized lazily on first access, so calling this is optional.
#[allow(dead_code)] // Optional optimization - caches initialize lazily
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

    let npu_result = NPU_INFO.get_or_init(detect_npu_info_uncached);

    // If NPU was detected, also discover LUID for utilization queries
    if npu_result.0 {
        if let Ok(wmi_con) = crate::wmi_native::WmiConnection::new() {
            NPU_LUID.get_or_init(|| discover_npu_luid_with_wmi(&wmi_con));
        }
    } else {
        NPU_LUID.get_or_init(|| None);
    }

    // Signal that initialization is complete
    INIT_COMPLETE.store(true, std::sync::atomic::Ordering::Release);
}

// ============================================================================
// OPTIMIZED STATS COLLECTION
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn collect_stats_optimized(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    previous_network: &Arc<Mutex<NetworkState>>,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
    cached_disks: &Arc<Mutex<Option<CachedDiskInfo>>>,
    cached_npu_util: &Arc<Mutex<Option<CachedNpuUtilization>>>,
    cached_fast_stats: &Arc<Mutex<Option<FastStats>>>,
    monitor_profile: &Arc<RwLock<MonitorProfile>>,
    disk_update_in_progress: &Arc<AtomicBool>,
    npu_update_in_progress: &Arc<AtomicBool>,
    fast_update_in_progress: &Arc<AtomicBool>,
    tick: u64,
) -> SystemStats {
    // 1. Get Fast Stats (CPU, Mem, Net, Proc) via non-blocking update
    let fast_stats = get_fast_stats_cached(
        pdh_state,
        previous_network,
        previous_processes,
        cached_fast_stats,
        monitor_profile,
        fast_update_in_progress,
    )
    .await;

    // Calculate derived metrics from fast stats (or use defaults if empty)
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

    // 2. Get Disk Stats (Non-blocking)
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

    // 3. GPU/NPU adapter stats. D3DKMT is primary; the older WMI path remains
    // as an NPU fallback on machines where D3DKMT does not expose the adapter.
    let gpu_info = fast_stats.adapter_snapshot.gpu.as_ref();
    let gpu_available = gpu_info.is_some();
    let gpu_name = gpu_info.map(|gpu| gpu.name.clone());
    let gpu_utilization = gpu_info.map(|gpu| gpu.utilization);
    let (gpu_memory_used, gpu_memory_total) =
        gpu_info.map(|gpu| gpu.meter_memory()).unwrap_or_default();

    let d3d_npu = fast_stats.adapter_snapshot.npu.as_ref();
    let (legacy_npu_available, legacy_npu_name) = if d3d_npu.is_some() {
        (false, None)
    } else if let Some(info) = NPU_INFO.get() {
        info.clone()
    } else {
        // Keep DXCore/WMI detection off the stats hot path. The startup prewarm
        // usually fills this before the first tick; if not, schedule it and emit
        // a normal stats frame without legacy NPU fields until the cache is ready.
        schedule_npu_cache_warmup();
        (false, None)
    };
    let legacy_npu_utilization = if d3d_npu.is_none() && legacy_npu_available {
        get_npu_util_cached(cached_npu_util, npu_update_in_progress, tick).await
    } else {
        None
    };
    let npu_available = d3d_npu.is_some() || legacy_npu_available;
    let npu_name = d3d_npu.map(|npu| npu.name.clone()).or(legacy_npu_name);
    let npu_utilization = d3d_npu
        .map(|npu| npu.utilization)
        .or(legacy_npu_utilization);
    let (npu_memory_used, npu_memory_total) =
        d3d_npu.map(|npu| npu.meter_memory()).unwrap_or_default();

    #[allow(deprecated)]
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
        storage_used_percent: disk_utilization,
        disk_utilization,
        disk_read_bytes: fast_stats.disk_read_bytes,
        disk_write_bytes: fast_stats.disk_write_bytes,
        disks,
        network_upload_kb: fast_stats.network_upload_kb,
        network_download_kb: fast_stats.network_download_kb,
        gpu_available,
        gpu_name,
        gpu_utilization,
        gpu_memory_used_mb: gpu_memory_used as f64 / (1024.0 * 1024.0),
        gpu_memory_total_mb: gpu_memory_total as f64 / (1024.0 * 1024.0),
        npu_available,
        npu_name,
        npu_utilization,
        npu_memory_used_mb: npu_memory_used as f64 / (1024.0 * 1024.0),
        npu_memory_total_mb: npu_memory_total as f64 / (1024.0 * 1024.0),
        top_processes: fast_stats.top_processes,
        all_processes: Vec::new(),
        timestamp: unix_timestamp_secs(),
    }
}

/// Compute a fresh FastStats snapshot synchronously. Must be called from a blocking
/// context (it uses block_on for the async network/process helpers). Shared by the
/// async cache-populate path and the synchronous pre-loop seeding in start_monitoring.
fn compute_fast_stats(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    previous_network: &Arc<Mutex<NetworkState>>,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
    profile: MonitorProfile,
) -> FastStats {
    // 1. CPU & Swap (PDH)
    let cpu_count = *CPU_COUNT.get_or_init(|| unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    });
    let (cpu_utilization, per_cpu_utilization, swap_utilization) =
        get_pdh_stats(pdh_state, cpu_count);
    let cpu_frequency = *CPU_FREQUENCY.get_or_init(get_cpu_frequency_uncached);

    // 2. Memory. swap_used comes straight from get_memory_info so it shares the same
    // (commit-charge) denominator as swap_total instead of being recomputed from the
    // page-file-relative PDH "% Usage" counter.
    let (memory_total, memory_used, memory_available, swap_total, swap_used) = get_memory_info();
    let _ = swap_utilization;

    // 3. Network
    // Need to lock async mutex in blocking context - use block_on
    let (network_upload_kb, network_download_kb) =
        futures::executor::block_on(async { get_network_stats(previous_network).await });

    // 4. Adapter stats are part of system telemetry. Process enumeration is
    // profile-gated: native shells use SystemOnly and let the Processes page
    // own its slower, independent snapshot cadence.
    let adapter_snapshot = crate::adapter_monitor::refresh();
    let (top_processes, disk_read_bytes, disk_write_bytes) = if profile.includes_processes() {
        let (processes, disk_read_bytes, disk_write_bytes) = futures::executor::block_on(async {
            get_top_processes_optimized(
                memory_total,
                previous_processes,
                profile.includes_process_adapter_stats(),
            )
            .await
        });
        (
            select_top_processes(&processes, profile.includes_process_adapter_stats()),
            disk_read_bytes,
            disk_write_bytes,
        )
    } else {
        (Vec::new(), 0, 0)
    };

    FastStats {
        profile,
        captured_at: Instant::now(),
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
        disk_read_bytes,
        disk_write_bytes,
        adapter_snapshot,
    }
}

fn select_top_processes(
    processes: &[ProcessInfo],
    include_adapter_stats: bool,
) -> Vec<ProcessInfo> {
    const CPU_ROWS: usize = 15;
    const EXTENDED_ROWS: usize = 40;
    const ADAPTER_ROWS: usize = 15;

    let mut selected = Vec::with_capacity(if include_adapter_stats {
        EXTENDED_ROWS.min(processes.len())
    } else {
        CPU_ROWS.min(processes.len())
    });
    let mut seen = HashSet::new();

    for process in processes.iter().take(CPU_ROWS) {
        if seen.insert(process.pid) {
            selected.push(process.clone());
        }
    }

    if !include_adapter_stats {
        return selected;
    }

    let mut add_by_metric = |metric: fn(&ProcessInfo) -> f32| {
        let mut rows: Vec<&ProcessInfo> = processes.iter().collect();
        rows.sort_by(|a, b| {
            metric(b)
                .partial_cmp(&metric(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for process in rows
            .into_iter()
            .filter(|process| metric(process) > 0.0)
            .take(ADAPTER_ROWS)
        {
            if selected.len() >= EXTENDED_ROWS {
                break;
            }
            if seen.insert(process.pid) {
                selected.push(process.clone());
            }
        }
    };

    add_by_metric(|process| process.gpu_percent);
    add_by_metric(|process| process.gpu_memory_mb as f32);
    add_by_metric(|process| process.npu_percent);
    add_by_metric(|process| process.npu_memory_mb as f32);

    selected
}

async fn get_fast_stats_cached(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    previous_network: &Arc<Mutex<NetworkState>>,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
    cached: &Arc<Mutex<Option<FastStats>>>,
    monitor_profile: &Arc<RwLock<MonitorProfile>>,
    update_in_progress: &Arc<AtomicBool>,
) -> FastStats {
    let profile = *monitor_profile
        .read()
        .unwrap_or_else(|poison| poison.into_inner());
    let should_refresh = {
        let cache = cached.lock().await;
        cache.as_ref().is_none_or(|stats| {
            stats.profile != profile || stats.captured_at.elapsed() >= FAST_STATS_MIN_AGE
        })
    };

    // At most one update owns each profile's CPU/network delta at a time. The
    // freshness gate also prevents the immediate post-seed tick from taking a
    // near-zero process delta and replacing the meaningful first sample.
    if should_refresh && !update_in_progress.swap(true, Ordering::Acquire) {
        let pdh_clone = Arc::clone(pdh_state);
        let net_clone = Arc::clone(previous_network);
        let proc_clone = Arc::clone(previous_processes);
        let cached_clone = Arc::clone(cached);
        let flag_clone = Arc::clone(update_in_progress);
        let profile_state = Arc::clone(monitor_profile);

        tokio::spawn(async move {
            let stats = tokio::task::spawn_blocking(move || {
                compute_fast_stats(&pdh_clone, &net_clone, &proc_clone, profile)
            })
            .await
            .ok();

            let current_profile = *profile_state
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(new_stats) = stats
                && current_profile == profile
            {
                let mut cache_lock = cached_clone.lock().await;
                *cache_lock = Some(new_stats);
            }

            flag_clone.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    // Return cached value or default
    let cache = cached.lock().await;
    cache
        .as_ref()
        .filter(|stats| stats.profile == profile)
        .cloned()
        .unwrap_or(FastStats {
            profile,
            captured_at: Instant::now(),
            cpu_utilization: 0.0,
            per_cpu_utilization: Vec::new(),
            cpu_frequency: 0,
            memory_total: 0,
            memory_used: 0,
            memory_available: 0,
            swap_total: 0,
            swap_used: 0,
            network_upload_kb: 0.0,
            network_download_kb: 0.0,
            top_processes: Vec::new(),
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            adapter_snapshot: crate::adapter_monitor::AdapterSnapshot::default(),
        })
}

// ============================================================================
// CACHED GETTERS
// ============================================================================

async fn get_disks_cached(
    cached: &Arc<Mutex<Option<CachedDiskInfo>>>,
    update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    tick: u64,
) -> Vec<DiskInfo> {
    let cache_guard = cached.lock().await;

    // Refresh every 5 ticks (5 seconds at 1Hz)
    let should_refresh = match &*cache_guard {
        Some(c) => tick.is_multiple_of(5) || c.last_update.elapsed() > Duration::from_secs(5),
        None => true,
    };

    if should_refresh {
        // Check if update is already in progress
        if !update_in_progress.swap(true, std::sync::atomic::Ordering::Acquire) {
            // No update in progress, start one in background
            let cached_clone = Arc::clone(cached);
            let update_flag = Arc::clone(update_in_progress);

            tokio::spawn(async move {
                // Run heavy IO in blocking thread
                let disks = tokio::task::spawn_blocking(get_disk_info_optimized)
                    .await
                    .unwrap_or_default();

                // Update cache
                let mut cache_lock = cached_clone.lock().await;
                *cache_lock = Some(CachedDiskInfo {
                    disks,
                    last_update: Instant::now(),
                });

                // Release flag
                update_flag.store(false, std::sync::atomic::Ordering::Release);
            });
        }
    }

    // Return current cache or empty if not yet initialized
    match &*cache_guard {
        Some(c) => c.disks.clone(),
        None => Vec::new(),
    }
}

async fn get_npu_util_cached(
    cached: &Arc<Mutex<Option<CachedNpuUtilization>>>,
    update_in_progress: &Arc<std::sync::atomic::AtomicBool>,
    tick: u64,
) -> Option<f32> {
    let cache_guard = cached.lock().await;

    // Refresh every 5 ticks
    let should_refresh = match &*cache_guard {
        Some(c) => tick.is_multiple_of(5) || c.last_update.elapsed() > Duration::from_secs(5),
        None => true,
    };

    if should_refresh && !update_in_progress.swap(true, std::sync::atomic::Ordering::Acquire) {
        let cached_clone = Arc::clone(cached);
        let update_flag = Arc::clone(update_in_progress);

        tokio::spawn(async move {
            let util = tokio::task::spawn_blocking(get_npu_utilization)
                .await
                .unwrap_or(None);

            let mut cache_lock = cached_clone.lock().await;
            *cache_lock = Some(CachedNpuUtilization {
                utilization: util,
                last_update: Instant::now(),
            });

            update_flag.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    match &*cache_guard {
        Some(c) => c.utilization,
        None => None,
    }
}

// ============================================================================
// CPU & SWAP MONITORING (PDH)
// ============================================================================

fn get_pdh_stats(
    pdh_state: &Arc<std::sync::Mutex<PdhState>>,
    cpu_count: usize,
) -> (f32, Vec<f32>, f32) {
    // Use unwrap_or_else to recover from poisoned mutex instead of panicking
    let mut state = pdh_state.lock().unwrap_or_else(|e| e.into_inner());

    if !state.initialized {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return (0.0, vec![0.0; cpu_count], 0.0);
            }
            state.query = SendPtr(query.0);

            // CPU Counters (Indices 0..cpu_count-1)
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

            // Swap Counter (Index cpu_count)
            // \Paging File(_Total)\% Usage
            let path_wide: Vec<u16> = "\\Paging File(_Total)\\% Usage\0".encode_utf16().collect();
            let mut counter = PDH_HCOUNTER::default();
            let status = PdhAddEnglishCounterW(query, PCWSTR(path_wide.as_ptr()), 0, &mut counter);
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

    // Process CPU counters
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

    // Process Swap counter (last element)
    let swap_util = if state.counters.len() > cpu_count {
        let counter_ptr = state.counters[cpu_count].0;
        if !counter_ptr.is_null() {
            let mut value = PDH_FMT_COUNTERVALUE::default();
            let status = unsafe {
                PdhGetFormattedCounterValue(
                    PDH_HCOUNTER(counter_ptr),
                    PDH_FMT_DOUBLE,
                    None,
                    &mut value,
                )
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

fn get_cpu_frequency_uncached() -> u64 {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(cpu_key) = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0")
        && let Ok(mhz) = cpu_key.get_value::<u32, _>("~MHz")
    {
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

            let (swap_total, swap_used) =
                if GetPerformanceInfo(&mut perf_info, perf_info.cb).is_ok() {
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
    use crate::wmi_native::WmiConnection;

    let mut types = HashMap::new();

    // Use Storage namespace for MSFT_PhysicalDisk
    if let Ok(wmi_con) = WmiConnection::with_namespace("root\\Microsoft\\Windows\\Storage") {
        // Query physical disks to get their media types
        if let Ok(results) = wmi_con.query("SELECT MediaType, DeviceID FROM MSFT_PhysicalDisk") {
            for disk in &results {
                if let (Some(media_type), Some(device_id)) = (
                    disk.get("MediaType").and_then(|v| v.as_u64()),
                    disk.get("DeviceID").and_then(|v| v.as_str()),
                ) {
                    let disk_type = match media_type {
                        3 => "HDD",
                        4 => "SSD",
                        5 => "SCM",
                        _ => "Unknown",
                    };

                    // Get partitions for this disk
                    let part_query = format!(
                        "SELECT DriveLetter FROM MSFT_Partition WHERE DiskNumber = {}",
                        device_id
                    );
                    if let Ok(partitions) = wmi_con.query(&part_query) {
                        for part in &partitions {
                            if let Some(letter) = part.get("DriveLetter").and_then(|v| v.as_str())
                                && let Some(c) = letter.chars().next()
                            {
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

                let mut file_system_buffer = [0u16; 64];
                let file_system = if GetVolumeInformationW(
                    PCWSTR(drive_wide.as_ptr()),
                    None,
                    None,
                    None,
                    None,
                    Some(&mut file_system_buffer),
                )
                .is_ok()
                {
                    let length = file_system_buffer
                        .iter()
                        .position(|value| *value == 0)
                        .unwrap_or(file_system_buffer.len());
                    String::from_utf16_lossy(&file_system_buffer[..length])
                } else {
                    "Unknown".to_string()
                };

                disks.push(DiskInfo {
                    name: drive_str.trim_end_matches('\\').to_string(),
                    mount_point: drive_str.clone(),
                    total_gb: total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    used_gb: used_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    available_gb: total_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    utilization,
                    file_system,
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

async fn get_network_stats(previous_network: &Arc<Mutex<NetworkState>>) -> (f64, f64) {
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

    let now = Instant::now();
    let mut prev_state = previous_network.lock().await;

    let mut upload_kb_per_sec = 0.0;
    let mut download_kb_per_sec = 0.0;

    // Calculate time elapsed since last update (in seconds)
    let elapsed = now.duration_since(prev_state.last_update).as_secs_f64();

    // Update timestamp
    prev_state.last_update = now;

    if !prev_state.bytes.is_empty() && elapsed > 0.001 {
        // Avoid division by zero
        for (interface, (tx, rx)) in &current_stats {
            if let Some((prev_tx, prev_rx)) = prev_state.bytes.get(interface) {
                let tx_delta = tx.saturating_sub(*prev_tx) as f64;
                let rx_delta = rx.saturating_sub(*prev_rx) as f64;

                // Calculate rate: (Total KB delta) / (Seconds elapsed)
                upload_kb_per_sec += (tx_delta / 1024.0) / elapsed;
                download_kb_per_sec += (rx_delta / 1024.0) / elapsed;
            }
        }
    }

    prev_state.bytes = current_stats;
    (upload_kb_per_sec, download_kb_per_sec)
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
                parent_pid: proc_info.inherited_from_unique_process_id.0 as usize as u32,
                name,
                working_set: proc_info.working_set_size as u64,
                private_bytes: proc_info.private_page_count as u64,
                virtual_size: proc_info.virtual_size as u64,
                kernel_time: proc_info.kernel_time as u64,
                user_time: proc_info.user_time as u64,
                create_time: proc_info.create_time as u64,
                read_bytes: proc_info.read_transfer_count as u64,
                write_bytes: proc_info.write_transfer_count as u64,
                thread_count: proc_info.number_of_threads,
                handle_count: proc_info.handle_count,
                base_priority: proc_info.base_priority,
            });

            if proc_info.next_entry_offset == 0 {
                break;
            }
            offset += proc_info.next_entry_offset as usize;
        }

        processes
    })
}

fn prune_process_cpu_times(prev: &mut ProcessCpuTimes, current_pids: &HashSet<u32>) {
    prev.retain(|pid, _| current_pids.contains(pid));
}

async fn get_top_processes_optimized(
    total_memory: u64,
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
    include_process_adapter_stats: bool,
) -> (Vec<ProcessInfo>, u64, u64) {
    // Own the CPU baseline for the entire capture, not just the final update.
    // This prevents two captures from observing the same old baseline and then
    // writing near-simultaneous timestamps that make the next delta jitter.
    let mut prev = previous_processes.lock().await;
    let native_procs = query_all_processes_optimized();
    let current_pids: HashSet<u32> = native_procs.iter().map(|process| process.pid).collect();
    let now = Instant::now();
    let adapter_processes: Vec<(u32, u64)> = if include_process_adapter_stats {
        native_procs
            .iter()
            .map(|process| (process.pid, process.create_time))
            .collect()
    } else {
        Vec::new()
    };
    let adapter_stats =
        crate::adapter_monitor::process_stats(&adapter_processes, include_process_adapter_stats);

    prune_process_cpu_times(&mut prev, &current_pids);
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

        // Calculate CPU time in seconds
        let total_100ns = proc.kernel_time + proc.user_time;
        let cpu_time_secs = total_100ns / 10_000_000;

        // Calculate shared memory (working set - private bytes)
        let shared_memory = proc.working_set.saturating_sub(proc.private_bytes);
        let adapter = adapter_stats.get(&proc.pid).copied().unwrap_or_default();

        let name_lower = proc.name.to_lowercase();

        processes.push(ProcessInfo {
            pid: proc.pid,
            parent_pid: proc.parent_pid,
            name: proc.name.clone(),
            exe_path: String::new(), // Requires opening process handle - expensive
            command: proc.name.clone(), // Command line requires separate query
            user: String::new(),     // Requires token query - expensive
            cpu_percent,
            memory_percent,
            memory_mb: proc.working_set as f64 / (1024.0 * 1024.0),
            virtual_memory_mb: proc.virtual_size as f64 / (1024.0 * 1024.0),
            shared_memory_mb: shared_memory as f64 / (1024.0 * 1024.0),
            gpu_percent: adapter.gpu_percent,
            gpu_memory_mb: adapter.gpu_memory as f64 / (1024.0 * 1024.0),
            npu_percent: adapter.npu_percent,
            npu_memory_mb: adapter.npu_memory as f64 / (1024.0 * 1024.0),
            cpu_time_secs,
            start_time: start_time as i64,
            status: "Running".to_string(),
            thread_count: proc.thread_count,
            handle_count: proc.handle_count,
            priority: proc.base_priority,
            io_read_bytes: proc.read_bytes,
            io_write_bytes: proc.write_bytes,
            is_elevated: false,        // Requires token query - expensive
            efficiency_mode: false,    // Requires PowerThrottling query
            arch: ProcessArch::Native, // Requires IsWow64Process2
            tree_depth: 0,
            tree_prefix: String::new(),
            has_children: false,
            is_collapsed: false,
            name_lower: name_lower.clone(),
            command_lower: name_lower,
        });

        // Update cache with current CPU times
        let total_time = proc.kernel_time + proc.user_time;
        prev.insert(proc.pid, (total_time, proc.create_time, now));
    }

    // Sort by CPU descending
    processes.sort_by(|a, b| {
        b.cpu_percent
            .partial_cmp(&a.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (processes, total_disk_read, total_disk_write)
}

/// Get ALL processes with full information (for process list view)
pub async fn get_all_processes(
    previous_processes: &Arc<Mutex<ProcessCpuTimes>>,
) -> Vec<ProcessInfo> {
    let total_memory = *TOTAL_MEMORY.get_or_init(|| {
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

    let (processes, _, _) =
        get_top_processes_optimized(total_memory, previous_processes, false).await;
    processes
}

#[cfg(test)]
fn paginate_processes(processes: Vec<ProcessInfo>, query: ProcessQuery) -> ProcessPage {
    paginate_process_snapshot(&processes, unix_timestamp_secs(), query)
}

fn paginate_process_snapshot(
    processes: &[ProcessInfo],
    captured_at: i64,
    query: ProcessQuery,
) -> ProcessPage {
    let search = query.search.trim().to_lowercase();
    let mut processes: Vec<&ProcessInfo> = processes
        .iter()
        .filter(|process| {
            search.is_empty()
                || process.name_lower.contains(&search)
                || process.command_lower.contains(&search)
                || process.pid.to_string().contains(&search)
                || process.status.to_ascii_lowercase().contains(&search)
        })
        .collect();

    let compare_float = |left: f32, right: f32| {
        left.partial_cmp(&right)
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    processes.sort_by(|left, right| {
        let ordering = match query.sort_by {
            ProcessSortKey::Name => left.name_lower.cmp(&right.name_lower),
            ProcessSortKey::Pid => left.pid.cmp(&right.pid),
            ProcessSortKey::CpuPercent => compare_float(left.cpu_percent, right.cpu_percent),
            ProcessSortKey::MemoryPercent => {
                compare_float(left.memory_percent, right.memory_percent)
            }
            ProcessSortKey::MemoryMb => left
                .memory_mb
                .partial_cmp(&right.memory_mb)
                .unwrap_or(std::cmp::Ordering::Equal),
            ProcessSortKey::Status => left.status.cmp(&right.status),
            ProcessSortKey::ThreadCount => left.thread_count.cmp(&right.thread_count),
            ProcessSortKey::GpuPercent => compare_float(left.gpu_percent, right.gpu_percent),
            ProcessSortKey::NpuPercent => compare_float(left.npu_percent, right.npu_percent),
        }
        .then_with(|| left.pid.cmp(&right.pid));

        match query.sort_direction {
            ProcessSortDirection::Asc => ordering,
            ProcessSortDirection::Desc => ordering.reverse(),
        }
    });

    let total = processes.len();
    let limit = query.limit.clamp(1, 250);
    // Process counts change continuously. If a client is viewing a page that
    // disappears, return the new last page instead of an empty, impossible
    // range such as "201–150 of 150".
    let offset = if total == 0 {
        0
    } else {
        query.offset.min(((total - 1) / limit) * limit)
    };
    let items = processes
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(ProcessRow::from)
        .collect();

    ProcessPage {
        captured_at,
        total,
        offset,
        limit,
        items,
    }
}

fn unix_timestamp_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

#[inline]
fn filetime_to_unix(filetime: u64) -> u64 {
    filetime.saturating_sub(116444736000000000) / 10_000_000
}

// ============================================================================
// NETWORK CONNECTIONS
// ============================================================================

pub async fn get_network_connections() -> Vec<NetworkConnection> {
    query_native_network_connections()
}

const AF_INET_V4: u32 = 2;
const AF_INET_V6: u32 = 23;
const ERROR_INSUFFICIENT_BUFFER_CODE: u32 = 122;

fn query_native_network_connections() -> Vec<NetworkConnection> {
    let mut connections = Vec::new();

    if let Some(buffer) = query_ip_helper_table(|table, size| unsafe {
        GetExtendedTcpTable(table, size, true, AF_INET_V4, TCP_TABLE_OWNER_PID_ALL, 0)
    }) {
        connections.extend(
            ip_helper_rows::<MIB_TCPROW_OWNER_PID>(&buffer)
                .iter()
                .map(|row| NetworkConnection {
                    protocol: "TCP".to_string(),
                    local_addr: format_ipv4_endpoint(row.dwLocalAddr, row.dwLocalPort),
                    remote_addr: format_ipv4_remote_endpoint(row.dwRemoteAddr, row.dwRemotePort),
                    status: tcp_state_name(row.dwState).to_string(),
                }),
        );
    }

    if let Some(buffer) = query_ip_helper_table(|table, size| unsafe {
        GetExtendedTcpTable(table, size, true, AF_INET_V6, TCP_TABLE_OWNER_PID_ALL, 0)
    }) {
        connections.extend(
            ip_helper_rows::<MIB_TCP6ROW_OWNER_PID>(&buffer)
                .iter()
                .map(|row| NetworkConnection {
                    protocol: "TCP".to_string(),
                    local_addr: format_ipv6_endpoint(
                        row.ucLocalAddr,
                        row.dwLocalScopeId,
                        row.dwLocalPort,
                    ),
                    remote_addr: format_ipv6_remote_endpoint(
                        row.ucRemoteAddr,
                        row.dwRemoteScopeId,
                        row.dwRemotePort,
                    ),
                    status: tcp_state_name(row.dwState).to_string(),
                }),
        );
    }

    if let Some(buffer) = query_ip_helper_table(|table, size| unsafe {
        GetExtendedUdpTable(table, size, true, AF_INET_V4, UDP_TABLE_OWNER_PID, 0)
    }) {
        connections.extend(
            ip_helper_rows::<MIB_UDPROW_OWNER_PID>(&buffer)
                .iter()
                .map(|row| NetworkConnection {
                    protocol: "UDP".to_string(),
                    local_addr: format_ipv4_endpoint(row.dwLocalAddr, row.dwLocalPort),
                    remote_addr: String::from("—"),
                    status: "BOUND".to_string(),
                }),
        );
    }

    if let Some(buffer) = query_ip_helper_table(|table, size| unsafe {
        GetExtendedUdpTable(table, size, true, AF_INET_V6, UDP_TABLE_OWNER_PID, 0)
    }) {
        connections.extend(
            ip_helper_rows::<MIB_UDP6ROW_OWNER_PID>(&buffer)
                .iter()
                .map(|row| NetworkConnection {
                    protocol: "UDP".to_string(),
                    local_addr: format_ipv6_endpoint(
                        row.ucLocalAddr,
                        row.dwLocalScopeId,
                        row.dwLocalPort,
                    ),
                    remote_addr: String::from("—"),
                    status: "BOUND".to_string(),
                }),
        );
    }

    // IP Helper returns listeners before many active sockets on typical
    // systems. Put useful peer-to-peer rows first so a bounded UI preview does
    // not look like ten identical wildcard addresses.
    connections.sort_by(|left, right| {
        network_connection_display_rank(left)
            .cmp(&network_connection_display_rank(right))
            .then_with(|| left.protocol.cmp(&right.protocol))
            .then_with(|| left.local_addr.cmp(&right.local_addr))
            .then_with(|| left.remote_addr.cmp(&right.remote_addr))
    });

    connections
}

fn network_connection_display_rank(connection: &NetworkConnection) -> u8 {
    match connection.status.as_str() {
        "ESTABLISHED" => 0,
        "SYN_SENT" | "SYN_RECEIVED" | "CLOSE_WAIT" | "FIN_WAIT_1" | "FIN_WAIT_2" | "CLOSING"
        | "LAST_ACK" => 1,
        "TIME_WAIT" => 2,
        "LISTENING" => 3,
        "BOUND" => 4,
        _ => 5,
    }
}

fn query_ip_helper_table(
    mut query: impl FnMut(Option<*mut std::ffi::c_void>, *mut u32) -> u32,
) -> Option<Vec<u32>> {
    let mut byte_len = 0u32;
    let initial = query(None, &mut byte_len);
    if initial != 0 && initial != ERROR_INSUFFICIENT_BUFFER_CODE {
        return None;
    }

    // DWORD storage supplies the alignment required by every OWNER_PID row.
    for _ in 0..3 {
        let words = (byte_len as usize).div_ceil(std::mem::size_of::<u32>());
        let mut buffer = vec![0u32; words.max(1)];
        let status = query(Some(buffer.as_mut_ptr().cast()), &mut byte_len as *mut u32);
        if status == 0 {
            return Some(buffer);
        }
        if status != ERROR_INSUFFICIENT_BUFFER_CODE {
            return None;
        }
    }

    None
}

fn ip_helper_rows<T>(buffer: &[u32]) -> &[T] {
    if buffer.is_empty()
        || std::mem::size_of::<T>() == 0
        || std::mem::align_of::<T>() > std::mem::align_of::<u32>()
    {
        return &[];
    }

    let available =
        (std::mem::size_of_val(buffer) - std::mem::size_of::<u32>()) / std::mem::size_of::<T>();
    let count = (buffer[0] as usize).min(available);
    let rows = unsafe { buffer.as_ptr().add(1).cast::<T>() };
    // SAFETY: GetExtended*Table writes a DWORD count followed by `count`
    // OWNER_PID rows. `available` bounds corrupt/racing counts to the allocated
    // buffer, and DWORD storage provides the rows' four-byte alignment.
    unsafe { std::slice::from_raw_parts(rows, count) }
}

fn format_ipv4_endpoint(address: u32, port: u32) -> String {
    let address = Ipv4Addr::from(u32::from_be(address));
    let port = u16::from_be(port as u16);
    if address.is_unspecified() {
        format!("All interfaces:{port}")
    } else {
        format!("{address}:{port}")
    }
}

fn format_ipv4_remote_endpoint(address: u32, port: u32) -> String {
    let address = Ipv4Addr::from(u32::from_be(address));
    let port = u16::from_be(port as u16);
    if address.is_unspecified() && port == 0 {
        "—".to_string()
    } else if address.is_unspecified() {
        format!("*:{port}")
    } else {
        format!("{address}:{port}")
    }
}

fn format_ipv6_endpoint(address: [u8; 16], scope_id: u32, port: u32) -> String {
    let address = Ipv6Addr::from(address);
    let port = u16::from_be(port as u16);
    if address.is_unspecified() {
        format!("All interfaces:{port}")
    } else if scope_id == 0 {
        format!("[{address}]:{port}")
    } else {
        format!("[{address}%{scope_id}]:{port}")
    }
}

fn format_ipv6_remote_endpoint(address: [u8; 16], scope_id: u32, port: u32) -> String {
    let address = Ipv6Addr::from(address);
    let port = u16::from_be(port as u16);
    if address.is_unspecified() && port == 0 {
        "—".to_string()
    } else if address.is_unspecified() {
        format!("*:{port}")
    } else if scope_id == 0 {
        format!("[{address}]:{port}")
    } else {
        format!("[{address}%{scope_id}]:{port}")
    }
}

fn tcp_state_name(state: u32) -> &'static str {
    match state {
        1 => "CLOSED",
        2 => "LISTENING",
        3 => "SYN_SENT",
        4 => "SYN_RECEIVED",
        5 => "ESTABLISHED",
        6 => "FIN_WAIT_1",
        7 => "FIN_WAIT_2",
        8 => "CLOSE_WAIT",
        9 => "CLOSING",
        10 => "LAST_ACK",
        11 => "TIME_WAIT",
        12 => "DELETE_TCB",
        _ => "UNKNOWN",
    }
}

// ============================================================================
// NPU DETECTION
// ============================================================================

pub fn get_npu_utilization() -> Option<f32> {
    use crate::wmi_native::WmiConnection;

    // Check if we have a cached NPU LUID first (fast path)
    // Only create WMI connection if we have a LUID to query
    let npu_luid = match NPU_LUID.get() {
        Some(Some(luid)) => luid.clone(),
        Some(None) => return None, // No NPU LUID was found during init
        None => {
            // LUID not yet initialized - try to discover it
            let wmi_con = WmiConnection::new().ok()?;
            let luid = NPU_LUID.get_or_init(|| discover_npu_luid_with_wmi(&wmi_con));
            luid.clone()?
        }
    };

    // Now query utilization with the known LUID
    let wmi_con = WmiConnection::new().ok()?;

    let query = format!(
        "SELECT UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine WHERE Name LIKE '%{}%'",
        npu_luid
    );

    if let Ok(results) = wmi_con.query(&query) {
        let mut total_util: u64 = 0;
        let mut count = 0;

        for counter in &results {
            if let Some(util) = counter.get("UtilizationPercentage") {
                // WMI can return UtilizationPercentage as string or integer
                let util_val = util
                    .as_u64()
                    .or_else(|| util.as_i64().map(|i| i as u64))
                    .or_else(|| util.as_str().and_then(|s| s.parse::<u64>().ok()));
                if let Some(u) = util_val {
                    total_util += u;
                    count += 1;
                }
            }
        }

        if count > 0 {
            return Some((total_util as f32).min(100.0));
        }
    }

    None
}

fn discover_npu_luid_with_wmi(wmi_con: &crate::wmi_native::WmiConnection) -> Option<String> {
    use std::collections::HashSet;

    if let Ok(results) =
        wmi_con.query("SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine")
    {
        let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();

        for counter in &results {
            if let Some(name) = counter.get("Name").and_then(|v| v.as_str())
                && let Some(luid_start) = name.find("luid_")
                && let Some(phys_start) = name.find("_phys_")
            {
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

        // First: find device with ONLY Compute engine (definitely NPU)
        for (luid, engine_types) in &luid_engines {
            if engine_types.len() == 1 && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }

        // Second: find device without 3D but with Compute (likely NPU)
        for (luid, engine_types) in &luid_engines {
            if !engine_types.contains("3D") && engine_types.contains("Compute") {
                return Some(luid.clone());
            }
        }
    }

    None
}

// DXCore attribute GUIDs that are not (yet) in the windows crate metadata.
// Values are from Microsoft's dxcore_interface.h documentation
// (learn.microsoft.com "DXCore adapter attribute GUIDs"). The D3D12
// CORE_COMPUTE / GRAPHICS attributes ARE bound by the crate and are used
// via their crate constants.

/// ML-capable devices (NPUs and GPUs); reported by drivers that support the
/// DirectX meta-commands required for ML workloads.
const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML: windows::core::GUID =
    windows::core::GUID::from_values(
        0xb71b0d41,
        0x1088,
        0x422f,
        [0xa2, 0x7c, 0x02, 0x50, 0xb7, 0xd3, 0xa9, 0x88],
    );

/// The runtime-agnostic hardware type Microsoft documents for NPUs —
/// "declared by NPUs with or without compute shader support", reported by
/// the driver or inferred by DXCore itself (the same signal Task Manager
/// uses to label adapters). This is the primary, OS-authoritative NPU
/// detection on Windows 11 24H2+.
const DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU: windows::core::GUID = windows::core::GUID::from_values(
    0xd46140c4,
    0xadd7,
    0x451b,
    [0x9e, 0x56, 0x06, 0xfe, 0x8c, 0x3b, 0x58, 0xed],
);

/// Known non-NPU device names, kept as defense-in-depth for the fallback
/// classifier on Windows builds that predate the hardware-type attributes.
const NPU_EXCLUDED_NAMES: &[&str] = &[
    // Virtual/emulated devices
    "umbus",
    "enumerator",
    "virtual",
    "microsoft basic",
    "remote desktop",
    "remote display",
    // USB/HID/Input devices (Issue #10)
    "usb",
    "hid",
    "input device",
    "keyboard",
    "mouse",
    "touchpad",
    "touchscreen",
    "gamepad",
    "joystick",
    "controller",
    // Storage/Network (sometimes have compute caps)
    "storage",
    "ahci",
    "nvme controller",
    "sata",
    "raid",
    "ethernet",
    "wifi",
    "wireless",
    "network adapter",
    "bluetooth",
];

/// Names that positively identify an NPU.
const NPU_IDENTIFIERS: &[&str] = &[
    "npu",
    "neural",
    "hexagon",
    "ai accelerator",
    "ai boost",
    "xdna",
    "myriad",
];

/// PCI/ACPI vendor ids of NPU silicon vendors: Intel 0x8086; AMD 0x1022
/// (NPU/IPU devices — deliberately not 0x1002, which is Radeon graphics);
/// Qualcomm 0x17CB (PCI) and 0x5143 ("QCOM" ACPI). "Microsoft" is NOT
/// vendor evidence — that matched virtual devices.
const NPU_VENDOR_IDS: &[u32] = &[0x8086, 0x1022, 0x17CB, 0x5143];

/// Everything the fallback classifier may consider about one DXCore
/// adapter. Pure data, so classification is unit-testable.
#[derive(Debug, Clone, Default)]
struct NpuAdapterEvidence {
    name: String,
    /// The OS itself reports the NPU hardware type for this adapter.
    os_reports_npu: bool,
    has_compute: bool,
    has_graphics: bool,
    /// DXCoreAdapterProperty::IsHardware; None = property unsupported.
    is_hardware: Option<bool>,
    /// vendorID from DXCoreAdapterProperty::HardwareID; None = unavailable.
    vendor_id: Option<u32>,
}

/// Fallback NPU classification for Windows builds without the hardware-type
/// attribute: positive evidence only — a software adapter or a blocklisted
/// virtual device is never an NPU, a name match is sufficient, and a
/// capability-only match additionally requires known NPU silicon (by
/// hardware id when available, by vendor name otherwise).
fn classify_npu_adapter(e: &NpuAdapterEvidence) -> bool {
    let name = e.name.to_lowercase();
    if name.is_empty() || e.is_hardware == Some(false) {
        return false;
    }
    if NPU_EXCLUDED_NAMES.iter().any(|ex| name.contains(ex)) {
        return false;
    }
    if e.os_reports_npu {
        return true;
    }
    if NPU_IDENTIFIERS.iter().any(|id| name.contains(id)) {
        return true;
    }
    if !e.has_compute || e.has_graphics {
        return false;
    }
    match e.vendor_id {
        Some(vendor) => NPU_VENDOR_IDS.contains(&vendor),
        None => ["qualcomm", "intel", "amd"]
            .iter()
            .any(|vendor| name.contains(vendor)),
    }
}

/// DXCoreAdapterProperty::IsHardware for an adapter, if the OS supports it.
unsafe fn dxcore_adapter_is_hardware(
    adapter: &windows::Win32::Graphics::DXCore::IDXCoreAdapter,
) -> Option<bool> {
    use windows::Win32::Graphics::DXCore::*;
    unsafe {
        if !adapter.IsPropertySupported(IsHardware) {
            return None;
        }
        let mut value = false;
        adapter
            .GetProperty(
                IsHardware,
                std::mem::size_of::<bool>(),
                &mut value as *mut _ as *mut _,
            )
            .ok()?;
        Some(value)
    }
}

/// vendorID from DXCoreAdapterProperty::HardwareID, if available.
unsafe fn dxcore_adapter_vendor_id(
    adapter: &windows::Win32::Graphics::DXCore::IDXCoreAdapter,
) -> Option<u32> {
    use windows::Win32::Graphics::DXCore::*;
    unsafe {
        if !adapter.IsPropertySupported(HardwareID) {
            return None;
        }
        let mut hardware_id = DXCoreHardwareID::default();
        adapter
            .GetProperty(
                HardwareID,
                std::mem::size_of::<DXCoreHardwareID>(),
                &mut hardware_id as *mut _ as *mut _,
            )
            .ok()?;
        Some(hardware_id.vendorID)
    }
}

/// DriverDescription of an adapter, trimmed; None when empty/unreadable.
unsafe fn dxcore_adapter_name(
    adapter: &windows::Win32::Graphics::DXCore::IDXCoreAdapter,
) -> Option<String> {
    use windows::Win32::Graphics::DXCore::*;
    unsafe {
        let size = adapter
            .GetPropertySize(DriverDescription)
            .ok()
            .filter(|size| *size > 0)?;
        let mut buffer: Vec<u8> = vec![0; size];
        adapter
            .GetProperty(DriverDescription, size, buffer.as_mut_ptr() as *mut _)
            .ok()?;
        let name = String::from_utf8_lossy(&buffer)
            .trim_end_matches('\0')
            .to_string();
        (!name.is_empty()).then_some(name)
    }
}

/// Primary detection, exactly the way Microsoft documents it: ask DXCore
/// for the adapters whose hardware type IS "NPU" (driver-declared or
/// OS-inferred; also catches MCDM NPUs without a D3D user-mode driver) and
/// take the first real hardware one.
unsafe fn find_npu_by_hardware_type(
    factory: &windows::Win32::Graphics::DXCore::IDXCoreAdapterFactory,
) -> Option<String> {
    use windows::Win32::Graphics::DXCore::*;
    unsafe {
        let list: IDXCoreAdapterList = factory
            .CreateAdapterList(&[DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU])
            .ok()?;
        for index in 0..list.GetAdapterCount() {
            let Ok(adapter) = list.GetAdapter::<IDXCoreAdapter>(index) else {
                continue;
            };
            if dxcore_adapter_is_hardware(&adapter) == Some(false) {
                continue;
            }
            if let Some(name) = dxcore_adapter_name(&adapter) {
                return Some(name);
            }
        }
        None
    }
}

/// Fallback for Windows builds that predate the hardware-type attributes:
/// enumerate ML/compute adapters and apply `classify_npu_adapter`.
unsafe fn find_npu_by_ml_evidence(
    factory: &windows::Win32::Graphics::DXCore::IDXCoreAdapterFactory,
) -> Option<String> {
    use windows::Win32::Graphics::DXCore::*;
    unsafe {
        let list: IDXCoreAdapterList = factory
            .CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML])
            .or_else(|_| factory.CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE]))
            .ok()?;
        for index in 0..list.GetAdapterCount() {
            let Ok(adapter) = list.GetAdapter::<IDXCoreAdapter>(index) else {
                continue;
            };
            let Some(name) = dxcore_adapter_name(&adapter) else {
                continue;
            };
            let evidence = NpuAdapterEvidence {
                name,
                os_reports_npu: adapter.IsAttributeSupported(&DXCORE_HARDWARE_TYPE_ATTRIBUTE_NPU),
                has_compute: adapter
                    .IsAttributeSupported(&DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE),
                has_graphics: adapter
                    .IsAttributeSupported(&DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS),
                is_hardware: dxcore_adapter_is_hardware(&adapter),
                vendor_id: dxcore_adapter_vendor_id(&adapter),
            };
            if classify_npu_adapter(&evidence) {
                return Some(evidence.name);
            }
        }
        None
    }
}

fn detect_npu_dxcore() -> Option<(String, u32)> {
    use windows::Win32::Graphics::DXCore::*;

    unsafe {
        let factory: IDXCoreAdapterFactory = DXCoreCreateAdapterFactory().ok()?;
        if let Some(name) = find_npu_by_hardware_type(&factory) {
            return Some((name, 1));
        }
        find_npu_by_ml_evidence(&factory).map(|name| (name, 1))
    }
}

/// Positive PnP device evidence of an NPU (an actual device is present).
/// This is the only WMI signal allowed to claim NPU availability. The loose
/// LIKE query alone is NOT trusted: every candidate goes through the same
/// exclusion list / classifier as the DXCore fallback, so a virtual or
/// bus-enumerator device whose name merely contains "NPU" cannot be
/// promoted to a real NPU.
fn detect_npu_pnp_device() -> Option<String> {
    use crate::wmi_native::WmiConnection;

    let wmi_con = WmiConnection::new().ok()?;
    let results = wmi_con
        .query(
            "SELECT Name, DeviceID, Description FROM Win32_PnPEntity WHERE \
             Name LIKE '%NPU%' OR \
             Name LIKE '%Neural%' OR \
             Name LIKE '%AI Accelerator%' OR \
             Name LIKE '%Hexagon%' OR \
             Description LIKE '%Neural Processing%' OR \
             DeviceID LIKE '%NPU%'",
        )
        .ok()?;
    results
        .iter()
        .filter_map(|device| {
            let name = device.get("Name").and_then(|v| v.as_str())?;
            // Classify on name + device id: the id carries vendor evidence
            // (e.g. PCI\\VEN_8086) the plain display name lacks.
            let device_id = device
                .get("DeviceID")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut combined = String::with_capacity(name.len() + device_id.len());
            combined.push_str(name);
            combined.push(' ');
            combined.push_str(device_id);
            let evidence = NpuAdapterEvidence {
                name: combined,
                os_reports_npu: false,
                has_compute: true,
                has_graphics: false,
                is_hardware: None,
                vendor_id: None,
            };
            classify_npu_adapter(&evidence).then(|| name.to_string())
        })
        .next()
}

/// CPU-family heuristic: this processor family normally ships with an NPU.
/// NOT device evidence — the NPU may be disabled or missing a driver — so
/// it never sets `npu_available`; it only enriches the diagnostics text.
fn detect_npu_cpu_hint() -> Option<String> {
    use crate::wmi_native::WmiConnection;

    let wmi_con = WmiConnection::new().ok()?;
    let cpu_results = wmi_con.query("SELECT Name FROM Win32_Processor").ok()?;
    let name = cpu_results
        .first()
        .and_then(|cpu| cpu.get("Name"))
        .and_then(|value| value.as_str())?;
    let name_lower = name.to_lowercase();

    if name_lower.contains("core ultra")
        || name_lower.contains("meteor lake")
        || name_lower.contains("arrow lake")
        || name_lower.contains("lunar lake")
    {
        return Some(format!("{} (Intel NPU)", name));
    }
    if name_lower.contains("ryzen ai")
        || name_lower.contains("ryzen 8000")
        || name_lower.contains("ryzen 9000")
    {
        return Some(format!("{} (AMD XDNA NPU)", name));
    }
    if name_lower.contains("snapdragon")
        || name_lower.contains("x elite")
        || name_lower.contains("x plus")
    {
        return Some(format!("{} (Qualcomm Hexagon NPU)", name));
    }
    None
}

// ============================================================================
// UPTIME
// ============================================================================

pub fn get_uptime_seconds() -> u64 {
    unsafe { GetTickCount64() / 1000 }
}

// ============================================================================
// NPU DIAGNOSTIC INFO
// ============================================================================

/// Get detailed NPU diagnostic information for the diagnostics tab
pub fn get_npu_diagnostic_info() -> serde_json::Value {
    use serde_json::json;

    // Try DXCore first (fast)
    if let Some((name, _)) = detect_npu_dxcore() {
        let tops = estimate_npu_tops(&name);
        return json!({
            "detected": true,
            "name": name,
            "detection_method": "DXCore",
            "tops_estimate": tops,
            "phi_silica_capable": tops.map(|t| t >= 40).unwrap_or(false),
            "status": "Available",
            "description": format_npu_description(&name, tops),
        });
    }

    // Fall back to WMI device evidence (slower but catches more devices)
    if let Some(name) = detect_npu_pnp_device() {
        let tops = estimate_npu_tops(&name);
        return json!({
            "detected": true,
            "name": name,
            "detection_method": "WMI",
            "tops_estimate": tops,
            "phi_silica_capable": tops.map(|t| t >= 40).unwrap_or(false),
            "status": "Available",
            "description": format_npu_description(&name, tops),
        });
    }

    // No NPU device found. The CPU-family heuristic is context, not
    // detection: the silicon normally includes an NPU, but no device is
    // visible (disabled in firmware, missing driver, or cut-down SKU).
    if let Some(hint) = detect_npu_cpu_hint() {
        return json!({
            "detected": false,
            "name": null,
            "detection_method": null,
            "tops_estimate": null,
            "phi_silica_capable": false,
            "status": "Not Detected",
            "cpu_hint": hint,
            "description": format!(
                "No NPU device was detected. This processor ({hint}) normally includes one — \
                 it may be disabled in firmware or missing a driver."
            ),
        });
    }

    // No NPU detected
    json!({
        "detected": false,
        "name": null,
        "detection_method": null,
        "tops_estimate": null,
        "phi_silica_capable": false,
        "status": "Not Detected",
        "description": "No Neural Processing Unit (NPU) detected. NPUs are available on Copilot+ PCs with Snapdragon X, Intel Core Ultra, or AMD Ryzen AI processors.",
    })
}

/// Estimate TOPS (Tera Operations Per Second) based on NPU name
fn estimate_npu_tops(name: &str) -> Option<u32> {
    let name_lower = name.to_lowercase();

    // Qualcomm Snapdragon X
    if name_lower.contains("snapdragon")
        || name_lower.contains("hexagon")
        || name_lower.contains("x elite")
        || name_lower.contains("x plus")
    {
        return Some(45); // Snapdragon X Elite/Plus NPUs are 45 TOPS
    }

    // Intel Core Ultra
    if name_lower.contains("intel")
        || name_lower.contains("core ultra")
        || name_lower.contains("meteor lake")
        || name_lower.contains("lunar lake")
    {
        if name_lower.contains("lunar lake") {
            return Some(48); // Lunar Lake NPU is 48 TOPS
        }
        return Some(10); // Meteor Lake/Core Ultra Gen 1 is ~10 TOPS
    }

    // AMD XDNA
    if name_lower.contains("amd") || name_lower.contains("xdna") || name_lower.contains("ryzen ai")
    {
        if name_lower.contains("xdna 2") || name_lower.contains("strix") {
            return Some(50); // XDNA 2 (Strix Point) is 50 TOPS
        }
        return Some(16); // XDNA 1 (Hawk Point) is ~16 TOPS
    }

    None
}

/// Format a human-readable NPU description
fn format_npu_description(name: &str, tops: Option<u32>) -> String {
    let tops_str = tops
        .map(|t| format!("{} TOPS", t))
        .unwrap_or_else(|| "Unknown performance".to_string());

    let copilot_status = if tops.map(|t| t >= 40).unwrap_or(false) {
        "Meets Copilot+ PC requirements (40+ TOPS). Phi Silica on-device AI available."
    } else if tops.is_some() {
        "Below Copilot+ PC threshold (40 TOPS). Phi Silica not available."
    } else {
        "Performance specifications unknown."
    };

    format!(
        "{}\nEstimated Performance: {}\n{}",
        name, tops_str, copilot_status
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str, cpu_percent: f32, memory_percent: f32) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: 0,
            name: name.to_string(),
            exe_path: String::new(),
            command: name.to_string(),
            user: String::new(),
            cpu_percent,
            memory_percent,
            memory_mb: f64::from(memory_percent) * 10.0,
            virtual_memory_mb: 100.0,
            shared_memory_mb: 0.0,
            gpu_percent: 0.0,
            gpu_memory_mb: 0.0,
            npu_percent: 0.0,
            npu_memory_mb: 0.0,
            cpu_time_secs: 0,
            start_time: 1,
            status: "Running".to_string(),
            thread_count: 1,
            handle_count: 1,
            priority: 0,
            io_read_bytes: 0,
            io_write_bytes: 0,
            is_elevated: false,
            efficiency_mode: false,
            arch: ProcessArch::Native,
            tree_depth: 0,
            tree_prefix: String::new(),
            has_children: false,
            is_collapsed: false,
            name_lower: name.to_lowercase(),
            command_lower: name.to_lowercase(),
        }
    }

    #[test]
    fn process_cpu_history_prunes_exited_pids() {
        let now = Instant::now();
        let mut prev = ProcessCpuTimes::from([
            (100, (10, 1, now)),
            (200, (20, 1, now)),
            (300, (30, 1, now)),
        ]);
        let current_pids = HashSet::from([100, 300]);

        prune_process_cpu_times(&mut prev, &current_pids);

        assert!(prev.contains_key(&100));
        assert!(!prev.contains_key(&200));
        assert!(prev.contains_key(&300));
    }

    #[test]
    fn monitor_profiles_make_process_demand_explicit() {
        let basic = MonitorProfile::Legacy {
            include_process_adapter_stats: false,
        };
        let extended = MonitorProfile::Legacy {
            include_process_adapter_stats: true,
        };

        assert!(basic.includes_processes());
        assert!(!basic.includes_process_adapter_stats());
        assert!(extended.includes_processes());
        assert!(extended.includes_process_adapter_stats());
        assert!(!MonitorProfile::SystemOnly.includes_processes());
        assert!(!MonitorProfile::SystemOnly.includes_process_adapter_stats());
    }

    #[test]
    fn process_snapshot_expires_on_the_two_second_boundary() {
        let captured_instant = Instant::now();
        let snapshot = ProcessSnapshot {
            processes: Vec::new(),
            captured_at: 123,
            captured_instant,
        };

        assert!(snapshot.is_fresh_at(captured_instant));
        assert!(
            snapshot.is_fresh_at(
                captured_instant + PROCESS_SNAPSHOT_INTERVAL - Duration::from_millis(1)
            )
        );
        assert!(!snapshot.is_fresh_at(captured_instant + PROCESS_SNAPSHOT_INTERVAL));
    }

    #[test]
    fn process_snapshot_pagination_preserves_capture_time() {
        let processes = vec![
            process(1, "alpha.exe", 5.0, 10.0),
            process(2, "beta.exe", 20.0, 30.0),
        ];

        let page = paginate_process_snapshot(
            &processes,
            1_234_567,
            ProcessQuery {
                search: "beta".to_string(),
                ..ProcessQuery::default()
            },
        );

        assert_eq!(page.captured_at, 1_234_567);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].pid, 2);
    }

    #[test]
    fn programmatic_process_query_default_uses_the_ui_page_size() {
        let query = ProcessQuery::default();

        assert_eq!(query.offset, 0);
        assert_eq!(query.limit, 100);
        assert!(query.search.is_empty());
    }

    #[test]
    fn process_page_filters_sorts_and_clamps_page_size() {
        let page = paginate_processes(
            vec![
                process(1, "alpha.exe", 5.0, 10.0),
                process(2, "beta.exe", 20.0, 30.0),
                process(3, "beta-helper.exe", 10.0, 20.0),
            ],
            ProcessQuery {
                search: "BETA".to_string(),
                sort_by: ProcessSortKey::CpuPercent,
                sort_direction: ProcessSortDirection::Desc,
                offset: 0,
                limit: 999,
            },
        );

        assert_eq!(page.total, 2);
        assert_eq!(page.limit, 250);
        assert_eq!(page.items[0].pid, 2);
        assert_eq!(page.items[1].pid, 3);
    }

    #[test]
    fn process_page_filter_matches_pid_and_status() {
        let mut suspended = process(42, "alpha.exe", 5.0, 10.0);
        suspended.status = "Suspended".to_string();
        let running = process(9001, "beta.exe", 20.0, 30.0);

        let by_pid = paginate_processes(
            vec![suspended.clone(), running.clone()],
            ProcessQuery {
                search: "9001".to_string(),
                ..ProcessQuery::default()
            },
        );
        assert_eq!(by_pid.items.len(), 1);
        assert_eq!(by_pid.items[0].pid, 9001);

        let by_status = paginate_processes(
            vec![suspended, running],
            ProcessQuery {
                search: "SUSPENDED".to_string(),
                ..ProcessQuery::default()
            },
        );
        assert_eq!(by_status.items.len(), 1);
        assert_eq!(by_status.items[0].pid, 42);
    }

    #[test]
    fn process_page_moves_an_out_of_range_offset_to_the_last_page() {
        let page = paginate_processes(
            vec![
                process(1, "alpha.exe", 5.0, 10.0),
                process(2, "beta.exe", 20.0, 30.0),
                process(3, "gamma.exe", 10.0, 20.0),
            ],
            ProcessQuery {
                search: String::new(),
                sort_by: ProcessSortKey::Name,
                sort_direction: ProcessSortDirection::Asc,
                offset: 100,
                limit: 2,
            },
        );

        assert_eq!(page.offset, 2);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "gamma.exe");
    }

    #[test]
    fn native_network_rows_format_addresses_ports_and_states() {
        assert_eq!(format_ipv4_endpoint(0x0100_007f, 0x5000), "127.0.0.1:80");
        assert_eq!(format_ipv4_endpoint(0, 0x5000), "All interfaces:80");
        assert_eq!(format_ipv4_remote_endpoint(0, 0), "—");
        assert_eq!(
            format_ipv6_endpoint(Ipv6Addr::LOCALHOST.octets(), 0, 0x9001),
            "[::1]:400"
        );
        assert_eq!(
            format_ipv6_endpoint(Ipv6Addr::UNSPECIFIED.octets(), 0, 0x5000),
            "All interfaces:80"
        );
        assert_eq!(
            format_ipv6_remote_endpoint(Ipv6Addr::UNSPECIFIED.octets(), 0, 0),
            "—"
        );
        assert_eq!(
            format_ipv6_endpoint("fe80::1".parse::<Ipv6Addr>().unwrap().octets(), 7, 0x5000,),
            "[fe80::1%7]:80"
        );
        assert_eq!(tcp_state_name(2), "LISTENING");
        assert_eq!(tcp_state_name(5), "ESTABLISHED");
        assert_eq!(tcp_state_name(u32::MAX), "UNKNOWN");
    }

    #[test]
    fn active_network_connections_sort_ahead_of_wildcard_listeners() {
        let connection = |status: &str, local: &str, remote: &str| NetworkConnection {
            protocol: "TCP".to_string(),
            local_addr: local.to_string(),
            remote_addr: remote.to_string(),
            status: status.to_string(),
        };
        let mut connections = [
            connection("BOUND", "All interfaces:5353", "—"),
            connection("LISTENING", "All interfaces:445", "—"),
            connection("TIME_WAIT", "192.0.2.10:51000", "198.51.100.20:443"),
            connection("ESTABLISHED", "192.0.2.10:51001", "198.51.100.20:443"),
        ];
        connections.sort_by_key(network_connection_display_rank);

        assert_eq!(connections[0].status, "ESTABLISHED");
        assert_eq!(connections[1].status, "TIME_WAIT");
        assert_eq!(connections[2].status, "LISTENING");
        assert_eq!(connections[3].status, "BOUND");
    }

    fn npu_evidence(name: &str) -> NpuAdapterEvidence {
        NpuAdapterEvidence {
            name: name.to_string(),
            os_reports_npu: false,
            has_compute: true,
            has_graphics: false,
            is_hardware: None,
            vendor_id: None,
        }
    }

    #[test]
    fn umbus_and_virtual_adapters_are_never_npus() {
        for name in [
            "UMBus Enumerator",
            "UMBus Root Bus Enumerator",
            "Microsoft Basic Render Driver",
            "Microsoft Remote Display Adapter",
        ] {
            assert!(!classify_npu_adapter(&npu_evidence(name)), "{name}");
            // Even with a claimed vendor id, the blocklist wins.
            assert!(
                !classify_npu_adapter(&NpuAdapterEvidence {
                    vendor_id: Some(0x1414),
                    ..npu_evidence(name)
                }),
                "{name} with vendor id"
            );
        }
    }

    #[test]
    fn software_adapters_are_rejected_even_with_npu_names() {
        assert!(!classify_npu_adapter(&NpuAdapterEvidence {
            is_hardware: Some(false),
            ..npu_evidence("Some Emulated NPU Device")
        }));
        assert!(!classify_npu_adapter(&npu_evidence("")));
    }

    #[test]
    fn npu_name_identifiers_are_sufficient_evidence() {
        assert!(classify_npu_adapter(&npu_evidence("Intel(R) AI Boost")));
        assert!(classify_npu_adapter(&npu_evidence(
            "Qualcomm(R) Hexagon(TM) NPU"
        )));
        assert!(classify_npu_adapter(&npu_evidence("AMD XDNA Processor")));
    }

    #[test]
    fn os_reported_npu_hardware_type_is_decisive() {
        assert!(classify_npu_adapter(&NpuAdapterEvidence {
            os_reports_npu: true,
            has_compute: false,
            ..npu_evidence("Vendor ML Coprocessor")
        }));
    }

    #[test]
    fn vendor_hardware_id_beats_name_matching() {
        // Real AMD NPU device id → accepted even without an NPU-ish name
        assert!(classify_npu_adapter(&NpuAdapterEvidence {
            vendor_id: Some(0x1022),
            ..npu_evidence("AMD IPU Device")
        }));
        // A Microsoft vendor id must NOT count as NPU silicon
        assert!(!classify_npu_adapter(&NpuAdapterEvidence {
            vendor_id: Some(0x1414),
            ..npu_evidence("Compute Device")
        }));
        // Without a hardware id, a vendor name is the (weaker) fallback
        assert!(classify_npu_adapter(&npu_evidence("AMD IPU Device")));
        assert!(!classify_npu_adapter(&npu_evidence(
            "Microsoft Compute Device"
        )));
    }

    #[test]
    fn graphics_capable_adapters_are_gpus_not_npus() {
        for name in ["NVIDIA GeForce RTX 4080", "Intel(R) Iris(R) Xe Graphics"] {
            assert!(
                !classify_npu_adapter(&NpuAdapterEvidence {
                    has_graphics: true,
                    vendor_id: Some(0x8086),
                    ..npu_evidence(name)
                }),
                "{name}"
            );
        }
    }
}
