# NPU Implementation Guide

This document describes how NPU (Neural Processing Unit) detection and monitoring is implemented in WFDiag using native Windows APIs via the `windows-rs` crate, without relying on the `wmi` crate.

## Overview

The NPU implementation consists of three main components:

1. **NPU Detection** - Using DXCore APIs to identify NPU hardware
2. **NPU Utilization** - Using WMI performance counters via native COM APIs
3. **Process-level NPU Usage** - Parsing per-process utilization from GPU engine counters

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        NPU Monitoring                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │   DXCore     │    │  WMI Native  │    │  Process List    │  │
│  │  Detection   │    │  Utilization │    │   (Future)       │  │
│  └──────┬───────┘    └──────┬───────┘    └────────┬─────────┘  │
│         │                   │                      │            │
│         ▼                   ▼                      ▼            │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │ IDXCore      │    │ IWbemServices│    │ Parse pid_XXXXX  │  │
│  │ AdapterList  │    │ WQL Queries  │    │ from engine names│  │
│  └──────────────┘    └──────────────┘    └──────────────────┘  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## 1. NPU Detection (DXCore)

### How It Works

We use DXCore to enumerate compute adapters and identify NPUs by their capabilities.

### Key GUIDs

```rust
// For ML-capable devices (NPUs and GPUs)
const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML: GUID = GUID::from_values(
    0xb71b0d41, 0x1088, 0x422f,
    [0xa2, 0x7c, 0x02, 0x50, 0xb7, 0xd3, 0xa9, 0x88],
);

// For compute-only devices (fallback)
const DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE: GUID = GUID::from_values(
    0x248e2800, 0xa793, 0x4724,
    [0xab, 0xaa, 0x23, 0xa6, 0xde, 0x1b, 0xe0, 0x90],
);

// To identify graphics-capable devices (GPUs)
const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS: GUID = GUID::from_values(
    0x0c9ece4d, 0x2f6e, 0x4f01,
    [0x8c, 0x96, 0xe8, 0x9e, 0x33, 0x1b, 0x47, 0xb1],
);
```

### Detection Logic

An adapter is classified as an NPU if:

1. **By Capabilities**: Has `CORE_COMPUTE` but NOT `GRAPHICS`
2. **By Name**: Contains known NPU identifiers: `npu`, `neural`, `hexagon`, `ai accelerator`, `ai boost`, `xdna`

### Implementation

```rust
fn detect_npu_dxcore() -> Option<(String, u32)> {
    unsafe {
        let factory: IDXCoreAdapterFactory = DXCoreCreateAdapterFactory().ok()?;

        // Query for ML-capable adapters
        let adapter_list = factory
            .CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML])
            .ok()?;

        for i in 0..adapter_list.GetAdapterCount() {
            let adapter: IDXCoreAdapter = adapter_list.GetAdapter(i).ok()?;

            // Check capabilities
            let has_compute = adapter.IsAttributeSupported(&CORE_COMPUTE);
            let has_graphics = adapter.IsAttributeSupported(&GRAPHICS);

            // Get driver description
            let name = get_adapter_name(&adapter)?;

            // NPU = compute without graphics, or name contains NPU identifiers
            if (has_compute && !has_graphics) || is_npu_by_name(&name) {
                return Some((name, 1));
            }
        }
        None
    }
}
```

### Filtering Non-NPU Devices

Some devices incorrectly appear in compute adapter lists. We filter out:
- `umbus`, `enumerator` (Windows system devices)
- `virtual`, `microsoft basic`, `remote desktop` (virtual adapters)

## 2. NPU Utilization (WMI Native)

### Why Not the `wmi` Crate?

We removed the `wmi` crate to:
- Reduce binary size
- Have direct control over COM initialization
- Handle edge cases in variant type conversion
- Avoid dependency on external crate maintenance

### Native WMI Implementation

Our `wmi_native.rs` module provides a lightweight WMI wrapper using `windows-rs`:

```rust
pub struct WmiConnection {
    services: IWbemServices,
}

impl WmiConnection {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)?;

            let locator: IWbemLocator = CoCreateInstance(
                &WbemLocator, None, CLSCTX_INPROC_SERVER
            )?;

            let services = locator.ConnectServer(
                &BSTR::from("root\\cimv2"),
                // ... authentication params
            )?;

            CoSetProxyBlanket(&services, /* ... */)?;

            Ok(Self { services })
        }
    }

    pub fn query(&self, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
        // Execute WQL query and return results as JSON
    }
}
```

### LUID Discovery

Each GPU/NPU has a unique LUID (Locally Unique Identifier). We discover the NPU's LUID by querying GPU engine performance counters:

```rust
fn discover_npu_luid(wmi: &WmiConnection) -> Option<String> {
    let results = wmi.query(
        "SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine"
    ).ok()?;

    // Group engines by LUID
    let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();

    for counter in &results {
        // Parse: pid_XXXXX_luid_0x00000000_0x00014B57_phys_0_eng_0_engtype_Compute
        if let Some(name) = counter.get("Name").and_then(|v| v.as_str()) {
            let luid = parse_luid(name)?;
            let engtype = parse_engtype(name)?;
            luid_engines.entry(luid).or_default().insert(engtype);
        }
    }

    // NPU = device with ONLY "Compute" engine (no 3D, VideoEncode, etc.)
    for (luid, engines) in &luid_engines {
        if engines.len() == 1 && engines.contains("Compute") {
            return Some(luid.clone());
        }
    }

    // Fallback: device with Compute but no 3D
    for (luid, engines) in &luid_engines {
        if engines.contains("Compute") && !engines.contains("3D") {
            return Some(luid.clone());
        }
    }

    None
}
```

### Utilization Query

Once we have the NPU LUID, we query its utilization:

```rust
fn get_npu_utilization(luid: &str) -> Option<f32> {
    let wmi = WmiConnection::new().ok()?;

    let query = format!(
        "SELECT UtilizationPercentage FROM \
         Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine \
         WHERE Name LIKE '%{}%'",
        luid
    );

    let results = wmi.query(&query).ok()?;

    let mut total_util: u64 = 0;
    let mut count = 0;

    for counter in &results {
        if let Some(util) = counter.get("UtilizationPercentage") {
            // IMPORTANT: WMI returns this as STRING, not integer!
            let value = util
                .as_u64()
                .or_else(|| util.as_i64().map(|i| i as u64))
                .or_else(|| util.as_str().and_then(|s| s.parse().ok()));

            if let Some(u) = value {
                total_util += u;
                count += 1;
            }
        }
    }

    if count > 0 {
        Some((total_util as f32).min(100.0))
    } else {
        None
    }
}
```

### Key Discovery: String Values

A critical finding during development: **WMI performance counters return `UtilizationPercentage` as strings**, not integers:

```
UtilizationPercentage = String("6")  // NOT Number(6)
```

This requires parsing with `.as_str().and_then(|s| s.parse().ok())`.

## 3. Process-Level NPU Usage (Future Enhancement)

### Data Available

The GPU engine counter names contain process IDs:

```
pid_23184_luid_0x00000000_0x00014B57_phys_0_eng_0_engtype_Compute
│       │                                              │
│       │                                              └── Engine type
│       └── NPU LUID
└── Process ID using this engine
```

### Implementation Approach

```rust
struct NpuProcess {
    pid: u32,
    name: String,
    utilization: f32,
}

fn get_npu_processes(npu_luid: &str) -> Vec<NpuProcess> {
    let wmi = WmiConnection::new().ok()?;

    let query = format!(
        "SELECT Name, UtilizationPercentage FROM \
         Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine \
         WHERE Name LIKE '%{}%'",
        npu_luid
    );

    let results = wmi.query(&query).ok()?;

    let mut processes: HashMap<u32, u64> = HashMap::new();

    for counter in &results {
        let name = counter.get("Name")?.as_str()?;
        let util = parse_utilization(counter.get("UtilizationPercentage")?);

        // Extract PID from name: pid_XXXXX_luid_...
        if let Some(pid) = parse_pid(name) {
            *processes.entry(pid).or_default() += util;
        }
    }

    // Convert PIDs to process names
    processes.iter().filter_map(|(pid, util)| {
        let name = get_process_name(*pid)?;
        Some(NpuProcess {
            pid: *pid,
            name,
            utilization: *util as f32,
        })
    }).collect()
}

fn parse_pid(name: &str) -> Option<u32> {
    // Parse "pid_12345_luid_..." -> 12345
    name.strip_prefix("pid_")?
        .split('_')
        .next()?
        .parse()
        .ok()
}
```

### Getting Process Names

```rust
fn get_process_name(pid: u32) -> Option<String> {
    use windows::Win32::System::ProcessStatus::*;
    use windows::Win32::System::Threading::*;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; 260];
        let mut size = buffer.len() as u32;

        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, &mut buffer, &mut size).ok()?;

        CloseHandle(handle);

        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit('\\').next().map(|s| s.to_string())
    }
}
```

## Caching Strategy

To minimize WMI query overhead:

1. **NPU Info** (`OnceLock`): Detected once at startup, never changes
2. **NPU LUID** (`OnceLock`): Discovered once, cached forever
3. **Utilization**: Queried every 5 seconds (configurable)

```rust
static NPU_INFO: OnceLock<(bool, Option<String>)> = OnceLock::new();
static NPU_LUID: OnceLock<Option<String>> = OnceLock::new();

// Background initialization
std::thread::spawn(|| {
    // Detect NPU via DXCore (fast)
    let npu_result = NPU_INFO.get_or_init(|| {
        detect_npu_dxcore()
            .map(|(name, _)| (true, Some(name)))
            .unwrap_or_else(|| detect_npu_wmi_fallback())
    });

    // Discover LUID if NPU found (slower, WMI)
    if npu_result.0 {
        if let Ok(wmi) = WmiConnection::new() {
            NPU_LUID.get_or_init(|| discover_npu_luid(&wmi));
        }
    } else {
        NPU_LUID.get_or_init(|| None);
    }
});
```

## Vendor-Specific Notes

### Qualcomm Hexagon NPU

- Detected via DXCore with name containing "Hexagon" or "NPU"
- Has `compute=false, graphics=false` in DXCore (detected by name)
- Registers GPU performance counters with engine type "Compute"
- LUID example: `luid_0x00000000_0x00014B57`

### Intel NPU (AI Boost)

- Detected via DXCore or by CPU name containing "Core Ultra"
- Should have `compute=true, graphics=false`
- Uses MCDM driver model

### AMD XDNA NPU

- Detected by CPU name containing "Ryzen AI" or by DXCore
- Uses MCDM driver model

## Debugging

A standalone debug binary is available:

```bash
cargo run --bin debug_diagnostics --target aarch64-pc-windows-msvc
```

This outputs:
1. DXCore adapter enumeration
2. All GPU engine entries from WMI
3. LUID grouping and engine types
4. NPU LUID discovery result
5. Utilization query results

## References

- [DXCore Adapter Attribute GUIDs](https://learn.microsoft.com/en-us/windows/win32/dxcore/dxcore-adapter-attribute-guids)
- [Using DXCore to Enumerate Adapters](https://learn.microsoft.com/en-us/windows/win32/dxcore/dxcore-enum-adapters)
- [MCDM Architecture](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/mcdm-architecture)
- [Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-perfformatteddata)
