//! GPU/NPU adapter monitoring via D3DKMT.
//!
//! This follows the same Windows API path used by htop-win (MIT): D3DKMT
//! node running-time deltas for utilization and segment commitments for memory.

#![cfg(windows)]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERREGISTRYINFO, D3DKMT_ADAPTERTYPE, D3DKMT_CLOSEADAPTER,
    D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE,
    D3DKMT_QUERYSTATISTICS_PROCESS_NODE, D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
    D3DKMT_QUERYSTATISTICS_QUERY_NODE, D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT,
    D3DKMT_QUERYSTATISTICS_SEGMENT, D3DKMTCloseAdapter, D3DKMTEnumAdapters2,
    D3DKMTQueryAdapterInfo, D3DKMTQueryStatistics, KMTQAITYPE_ADAPTERREGISTRYINFO,
    KMTQAITYPE_ADAPTERTYPE,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Clone, Default)]
pub struct AdapterMetrics {
    pub name: String,
    pub utilization: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub dedicated_used: u64,
    pub dedicated_total: u64,
    pub shared_used: u64,
}

impl AdapterMetrics {
    pub fn meter_memory(&self) -> (u64, u64) {
        const DISCRETE_VRAM_MIN: u64 = 1 << 30;
        if self.dedicated_total >= DISCRETE_VRAM_MIN {
            (self.dedicated_used, self.dedicated_total)
        } else {
            (self.mem_used, self.mem_total)
        }
    }
}

#[derive(Clone, Default)]
pub struct AdapterSnapshot {
    pub gpu: Option<AdapterMetrics>,
    pub npu: Option<AdapterMetrics>,
}

#[derive(Clone, Copy, Default)]
pub struct ProcAdapterStats {
    pub gpu_percent: f32,
    pub gpu_memory: u64,
    pub npu_percent: f32,
    pub npu_memory: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterClass {
    Gpu,
    Npu,
}

type LuidKey = (u32, i32);

struct TrackedAdapter {
    class: AdapterClass,
    luid: LUID,
    name: String,
    node_count: u32,
    segment_count: u32,
    aperture_segment: Vec<bool>,
    prev_node_running: Vec<i64>,
}

#[derive(Default)]
struct AdapterState {
    adapters: Vec<TrackedAdapter>,
    detected: bool,
    needs_reenumeration: bool,
    last_sample: Option<Instant>,
    last_proc_sample: Option<Instant>,
    prev_proc_running: HashMap<(u32, u64), Vec<i64>>,
    prev_proc_enabled: bool,
    enumerated_luids: Vec<LuidKey>,
    pending_luids: Vec<LuidKey>,
    last_topology_check: Option<Instant>,
    last_snapshot: AdapterSnapshot,
}

const TOPOLOGY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

static ADAPTER_STATE: Mutex<Option<AdapterState>> = Mutex::new(None);

#[inline]
fn classify_adapter(adapter_type_value: u32) -> Option<AdapterClass> {
    const RENDER_SUPPORTED: u32 = 1 << 0;
    const SOFTWARE_DEVICE: u32 = 1 << 2;
    const COMPUTE_ONLY: u32 = 1 << 11;

    if adapter_type_value & SOFTWARE_DEVICE != 0 {
        return None;
    }
    if adapter_type_value & COMPUTE_ONLY != 0 {
        return Some(AdapterClass::Npu);
    }
    if adapter_type_value & RENDER_SUPPORTED != 0 {
        return Some(AdapterClass::Gpu);
    }
    None
}

#[inline]
fn running_time_to_percent(prev: i64, cur: i64, wall_elapsed_secs: f64) -> f32 {
    if wall_elapsed_secs <= 0.0 {
        return 0.0;
    }
    let delta = (cur - prev).max(0) as f64;
    ((delta / 10_000_000.0) / wall_elapsed_secs * 100.0).clamp(0.0, 100.0) as f32
}

fn utf16_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[inline]
fn query_statistics(query: &mut D3DKMT_QUERYSTATISTICS) -> bool {
    unsafe { D3DKMTQueryStatistics(query as *mut D3DKMT_QUERYSTATISTICS as *const _).is_ok() }
}

fn open_process_query(pid: u32) -> Option<HANDLE> {
    unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok()
    }
}

fn for_each_enumerated_adapter(mut f: impl FnMut(&D3DKMT_ADAPTERINFO)) {
    unsafe {
        let mut enum2 = D3DKMT_ENUMADAPTERS2::default();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() || enum2.NumAdapters == 0 {
            return;
        }

        let mut infos = vec![D3DKMT_ADAPTERINFO::default(); enum2.NumAdapters as usize];
        enum2.pAdapters = infos.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() {
            return;
        }

        for info in &infos[..enum2.NumAdapters as usize] {
            if info.hAdapter == 0 {
                continue;
            }
            f(info);
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
                hAdapter: info.hAdapter,
            });
        }
    }
}

enum AdapterProbe {
    Tracked(TrackedAdapter),
    Pending,
    Untrackable,
}

fn detect_adapters() -> (Vec<TrackedAdapter>, Vec<LuidKey>, Vec<LuidKey>) {
    let mut adapters = Vec::new();
    let mut luids = Vec::new();
    let mut pending = Vec::new();

    for_each_enumerated_adapter(|info| {
        let luid = (info.AdapterLuid.LowPart, info.AdapterLuid.HighPart);
        luids.push(luid);
        match build_adapter(info) {
            AdapterProbe::Tracked(adapter) => adapters.push(adapter),
            AdapterProbe::Pending => pending.push(luid),
            AdapterProbe::Untrackable => {}
        }
    });

    luids.sort_unstable();
    pending.sort_unstable();
    (adapters, luids, pending)
}

fn enumerate_luids() -> Vec<LuidKey> {
    let mut luids = Vec::new();
    for_each_enumerated_adapter(|info| {
        luids.push((info.AdapterLuid.LowPart, info.AdapterLuid.HighPart));
    });
    luids.sort_unstable();
    luids
}

fn build_adapter(info: &D3DKMT_ADAPTERINFO) -> AdapterProbe {
    unsafe {
        let mut adapter_type = D3DKMT_ADAPTERTYPE::default();
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: info.hAdapter,
            Type: KMTQAITYPE_ADAPTERTYPE,
            pPrivateDriverData: &mut adapter_type as *mut _ as *mut std::ffi::c_void,
            PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERTYPE>() as u32,
        };
        if D3DKMTQueryAdapterInfo(&mut query).is_err() {
            return AdapterProbe::Untrackable;
        }

        let Some(class) = classify_adapter(adapter_type.Anonymous.Value) else {
            return AdapterProbe::Untrackable;
        };
        let fallback_name = match class {
            AdapterClass::Gpu => "GPU",
            AdapterClass::Npu => "NPU",
        };

        let mut registry_info = D3DKMT_ADAPTERREGISTRYINFO::default();
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: info.hAdapter,
            Type: KMTQAITYPE_ADAPTERREGISTRYINFO,
            pPrivateDriverData: &mut registry_info as *mut _ as *mut std::ffi::c_void,
            PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERREGISTRYINFO>() as u32,
        };
        let name = if D3DKMTQueryAdapterInfo(&mut query).is_ok() {
            let name = utf16_to_string(&registry_info.AdapterString);
            if name.is_empty() {
                fallback_name.to_string()
            } else {
                name
            }
        } else {
            fallback_name.to_string()
        };

        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_ADAPTER,
            AdapterLuid: info.AdapterLuid,
            ..Default::default()
        };
        if !query_statistics(&mut stats) {
            return AdapterProbe::Pending;
        }

        let node_count = stats.QueryResult.AdapterInformation.NodeCount;
        let segment_count = stats.QueryResult.AdapterInformation.NbSegments;
        let mut aperture_segment = Vec::with_capacity(segment_count as usize);

        for segment_id in 0..segment_count {
            let mut stats = D3DKMT_QUERYSTATISTICS {
                Type: D3DKMT_QUERYSTATISTICS_SEGMENT,
                AdapterLuid: info.AdapterLuid,
                ..Default::default()
            };
            stats.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
                SegmentId: segment_id,
            };
            let aperture =
                query_statistics(&mut stats) && stats.QueryResult.SegmentInformation.Aperture != 0;
            aperture_segment.push(aperture);
        }

        AdapterProbe::Tracked(TrackedAdapter {
            class,
            luid: info.AdapterLuid,
            name,
            node_count,
            segment_count,
            aperture_segment,
            prev_node_running: vec![0; node_count as usize],
        })
    }
}

fn query_adapter_metrics(
    adapter: &mut TrackedAdapter,
    elapsed: Option<f64>,
) -> Option<AdapterMetrics> {
    let mut metrics = AdapterMetrics {
        name: adapter.name.clone(),
        ..Default::default()
    };

    for node_id in 0..adapter.node_count {
        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_NODE,
            AdapterLuid: adapter.luid,
            ..Default::default()
        };
        stats.Anonymous.QueryNode = D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: node_id };
        if !query_statistics(&mut stats) {
            return None;
        }
        let running = unsafe {
            stats
                .QueryResult
                .NodeInformation
                .GlobalInformation
                .RunningTime
        };
        if let Some(secs) = elapsed {
            metrics.utilization = metrics.utilization.max(running_time_to_percent(
                adapter.prev_node_running[node_id as usize],
                running,
                secs,
            ));
        }
        adapter.prev_node_running[node_id as usize] = running;
    }

    for segment_id in 0..adapter.segment_count {
        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_SEGMENT,
            AdapterLuid: adapter.luid,
            ..Default::default()
        };
        stats.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
            SegmentId: segment_id,
        };
        if !query_statistics(&mut stats) {
            return None;
        }

        let segment = unsafe { stats.QueryResult.SegmentInformation };
        let limit = if segment.CommitLimit == u64::MAX {
            0
        } else {
            segment.CommitLimit
        };
        let used = if segment.BytesResident > 0 {
            segment.BytesResident
        } else {
            segment.BytesCommitted
        };

        if adapter
            .aperture_segment
            .get(segment_id as usize)
            .copied()
            .unwrap_or(false)
        {
            metrics.shared_used = metrics.shared_used.saturating_add(used);
        } else {
            metrics.dedicated_used = metrics.dedicated_used.saturating_add(used);
            metrics.dedicated_total = metrics.dedicated_total.saturating_add(limit);
        }
        metrics.mem_total = metrics.mem_total.saturating_add(limit);
    }

    metrics.mem_used = metrics.dedicated_used.saturating_add(metrics.shared_used);
    Some(metrics)
}

fn aggregate_gpus(per_adapter: &[(AdapterClass, AdapterMetrics)]) -> Option<AdapterMetrics> {
    let mut primary: Option<&AdapterMetrics> = None;
    let mut count = 0usize;

    for (class, metrics) in per_adapter {
        if *class != AdapterClass::Gpu {
            continue;
        }
        count += 1;
        if primary.is_none_or(|p| metrics.dedicated_total > p.dedicated_total) {
            primary = Some(metrics);
        }
    }

    let mut info = primary?.clone();
    if count > 1 {
        info.name.push_str(&format!(" (+{})", count - 1));
    }
    Some(info)
}

fn aggregate_npus(per_adapter: &[(AdapterClass, AdapterMetrics)]) -> Option<AdapterMetrics> {
    let mut npus = per_adapter
        .iter()
        .filter(|(class, _)| *class == AdapterClass::Npu)
        .map(|(_, metrics)| metrics);
    let mut info = npus.next()?.clone();
    let mut extra = 0usize;

    for metrics in npus {
        info.utilization = info.utilization.max(metrics.utilization);
        info.mem_used = info.mem_used.saturating_add(metrics.mem_used);
        info.mem_total = info.mem_total.saturating_add(metrics.mem_total);
        info.dedicated_used = info.dedicated_used.saturating_add(metrics.dedicated_used);
        info.dedicated_total = info.dedicated_total.saturating_add(metrics.dedicated_total);
        info.shared_used = info.shared_used.saturating_add(metrics.shared_used);
        extra += 1;
    }

    if extra > 0 {
        info.name.push_str(&format!(" (+{})", extra));
    }
    Some(info)
}

pub fn refresh() -> AdapterSnapshot {
    let mut guard = ADAPTER_STATE.lock().unwrap();
    let state = guard.get_or_insert_with(AdapterState::default);
    let now = Instant::now();

    if state.detected
        && !state.needs_reenumeration
        && state
            .last_topology_check
            .is_none_or(|t| now.duration_since(t) >= TOPOLOGY_CHECK_INTERVAL)
    {
        state.last_topology_check = Some(now);
        if enumerate_luids() != state.enumerated_luids || !state.pending_luids.is_empty() {
            state.needs_reenumeration = true;
        }
    }

    if !state.detected || state.needs_reenumeration {
        let old = std::mem::take(&mut state.adapters);
        let (mut adapters, luids, pending) = detect_adapters();

        for adapter in &mut adapters {
            match old.iter().find(|old_adapter| {
                old_adapter.luid.LowPart == adapter.luid.LowPart
                    && old_adapter.luid.HighPart == adapter.luid.HighPart
            }) {
                Some(old_adapter)
                    if old_adapter.prev_node_running.len() == adapter.prev_node_running.len() =>
                {
                    adapter
                        .prev_node_running
                        .clone_from(&old_adapter.prev_node_running);
                }
                _ => {
                    let _ = query_adapter_metrics(adapter, None);
                }
            }
        }

        state.adapters = adapters;
        state.enumerated_luids = luids;
        state.pending_luids = pending;
        state.detected = true;
        state.needs_reenumeration = false;
        state.last_topology_check = Some(now);
        state.last_proc_sample = None;
        state.prev_proc_running.clear();
    }

    if state.adapters.is_empty() {
        state.last_snapshot = AdapterSnapshot::default();
        return AdapterSnapshot::default();
    }

    let elapsed = state
        .last_sample
        .map(|t| now.duration_since(t).as_secs_f64());
    state.last_sample = Some(now);

    let mut per_adapter = Vec::with_capacity(state.adapters.len());
    for adapter in &mut state.adapters {
        let Some(metrics) = query_adapter_metrics(adapter, elapsed) else {
            state.needs_reenumeration = true;
            return state.last_snapshot.clone();
        };
        per_adapter.push((adapter.class, metrics));
    }

    let snapshot = AdapterSnapshot {
        gpu: aggregate_gpus(&per_adapter),
        npu: aggregate_npus(&per_adapter),
    };
    state.last_snapshot = snapshot.clone();
    snapshot
}

pub fn process_stats(processes: &[(u32, u64)], enabled: bool) -> HashMap<u32, ProcAdapterStats> {
    let mut guard = ADAPTER_STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return HashMap::new();
    };

    let enabled = enabled
        && state
            .adapters
            .iter()
            .any(|adapter| matches!(adapter.class, AdapterClass::Gpu | AdapterClass::Npu));

    if enabled != state.prev_proc_enabled {
        state.prev_proc_enabled = enabled;
        state.prev_proc_running.clear();
        state.last_proc_sample = None;
    }
    if !enabled {
        return HashMap::new();
    }

    let now = Instant::now();
    let elapsed = state
        .last_proc_sample
        .map(|t| now.duration_since(t).as_secs_f64());
    state.last_proc_sample = Some(now);

    let AdapterState {
        adapters,
        prev_proc_running,
        ..
    } = state;
    let total_nodes: usize = adapters
        .iter()
        .map(|adapter| adapter.node_count as usize)
        .sum();

    let mut result = HashMap::with_capacity(processes.len());
    let mut current_keys = HashSet::with_capacity(processes.len());

    for &(pid, create_time) in processes {
        if pid == 0 || pid == 4 {
            continue;
        }
        let Some(handle) = open_process_query(pid) else {
            continue;
        };

        let key = (pid, create_time);
        let fresh = !prev_proc_running.contains_key(&key);
        let prev = prev_proc_running
            .entry(key)
            .or_insert_with(|| vec![0; total_nodes]);

        let mut entry = ProcAdapterStats::default();
        let mut node_index = 0usize;
        let mut sampled_running_time = false;

        for adapter in adapters.iter() {
            for node_id in 0..adapter.node_count {
                let mut stats = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_NODE,
                    AdapterLuid: adapter.luid,
                    hProcess: handle,
                    ..Default::default()
                };
                stats.Anonymous.QueryProcessNode =
                    D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: node_id };

                if query_statistics(&mut stats) {
                    sampled_running_time = true;
                    let running = unsafe { stats.QueryResult.ProcessNodeInformation.RunningTime };
                    if !fresh && let Some(secs) = elapsed {
                        let pct = running_time_to_percent(prev[node_index], running, secs);
                        match adapter.class {
                            AdapterClass::Gpu => entry.gpu_percent = entry.gpu_percent.max(pct),
                            AdapterClass::Npu => entry.npu_percent = entry.npu_percent.max(pct),
                        }
                    }
                    prev[node_index] = running;
                }
                node_index += 1;
            }

            for segment_id in 0..adapter.segment_count {
                let mut stats = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
                    AdapterLuid: adapter.luid,
                    hProcess: handle,
                    ..Default::default()
                };
                stats.Anonymous.QueryProcessSegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
                    SegmentId: segment_id,
                };

                if query_statistics(&mut stats) {
                    let committed =
                        unsafe { stats.QueryResult.ProcessSegmentInformation.BytesCommitted };
                    let committed = if committed == u64::MAX { 0 } else { committed };
                    match adapter.class {
                        AdapterClass::Gpu => {
                            entry.gpu_memory = entry.gpu_memory.saturating_add(committed)
                        }
                        AdapterClass::Npu => {
                            entry.npu_memory = entry.npu_memory.saturating_add(committed)
                        }
                    }
                }
            }
        }

        unsafe {
            let _ = CloseHandle(handle);
        }

        if sampled_running_time {
            current_keys.insert(key);
        }
        result.insert(pid, entry);
    }

    prev_proc_running.retain(|key, _| current_keys.contains(key));
    result
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterClass, AdapterMetrics, aggregate_gpus, aggregate_npus, classify_adapter,
        running_time_to_percent, utf16_to_string,
    };

    fn metrics(name: &str, utilization: f32, dedicated_total: u64) -> AdapterMetrics {
        AdapterMetrics {
            name: name.to_string(),
            utilization,
            mem_used: 100,
            mem_total: 200,
            dedicated_used: 50,
            dedicated_total,
            shared_used: 50,
        }
    }

    #[test]
    fn classifies_adapter_types() {
        assert!(matches!(classify_adapter(1 << 0), Some(AdapterClass::Gpu)));
        assert!(matches!(classify_adapter(1 << 11), Some(AdapterClass::Npu)));
        assert!(classify_adapter((1 << 0) | (1 << 2)).is_none());
        assert!(classify_adapter((1 << 11) | (1 << 2)).is_none());
        assert!(classify_adapter(0).is_none());
    }

    #[test]
    fn meter_memory_chooses_dedicated_only_for_discrete_gpu() {
        let discrete = AdapterMetrics {
            dedicated_used: 2 << 30,
            dedicated_total: 8 << 30,
            mem_used: 3 << 30,
            mem_total: 24 << 30,
            ..Default::default()
        };
        assert_eq!(discrete.meter_memory(), (2 << 30, 8 << 30));

        let integrated = AdapterMetrics {
            dedicated_used: 0,
            dedicated_total: 128 << 20,
            shared_used: 1_739_767_808,
            mem_used: 1_739_767_808,
            mem_total: 8_722_055_168,
            ..Default::default()
        };
        assert_eq!(integrated.meter_memory(), (1_739_767_808, 8_722_055_168));
    }

    #[test]
    fn aggregates_gpus_by_largest_dedicated_pool() {
        let per_adapter = vec![
            (AdapterClass::Gpu, metrics("iGPU", 80.0, 128 << 20)),
            (AdapterClass::Gpu, metrics("dGPU", 10.0, 8 << 30)),
            (AdapterClass::Npu, metrics("NPU", 99.0, 0)),
        ];
        let gpu = aggregate_gpus(&per_adapter).unwrap();
        assert_eq!(gpu.name, "dGPU (+1)");
        assert_eq!(gpu.utilization, 10.0);
    }

    #[test]
    fn aggregates_npus_by_max_utilization_and_summed_memory() {
        let per_adapter = vec![
            (AdapterClass::Gpu, metrics("GPU", 90.0, 8 << 30)),
            (AdapterClass::Npu, metrics("NPU A", 20.0, 0)),
            (AdapterClass::Npu, metrics("NPU B", 60.0, 0)),
        ];
        let npu = aggregate_npus(&per_adapter).unwrap();
        assert_eq!(npu.name, "NPU A (+1)");
        assert_eq!(npu.utilization, 60.0);
        assert_eq!(npu.mem_used, 200);
        assert_eq!(npu.mem_total, 400);
    }

    #[test]
    fn converts_running_time_delta_to_percent() {
        let half_sec_100ns = 5_000_000;
        assert_eq!(running_time_to_percent(0, half_sec_100ns, 1.0), 50.0);
        assert_eq!(running_time_to_percent(half_sec_100ns, 0, 1.0), 0.0);
        assert_eq!(running_time_to_percent(0, 30_000_000, 1.0), 100.0);
        assert_eq!(running_time_to_percent(0, half_sec_100ns, 0.0), 0.0);
    }

    #[test]
    fn decodes_utf16_until_nul() {
        let buf: Vec<u16> = "Intel(R) AI Boost\0\0extra".encode_utf16().collect();
        assert_eq!(utf16_to_string(&buf), "Intel(R) AI Boost");
        let no_nul: Vec<u16> = "NPU".encode_utf16().collect();
        assert_eq!(utf16_to_string(&no_nul), "NPU");
    }
}
