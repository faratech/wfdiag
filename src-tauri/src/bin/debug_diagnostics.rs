//! Standalone debug binary to test NPU detection and utilization
//! Run with: cargo run --bin debug_diagnostics --target aarch64-pc-windows-msvc

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn main() {
    println!("=== NPU Debug Tool ===\n");

    // Step 1: DXCore detection
    println!("--- Step 1: DXCore NPU Detection ---");
    match detect_npu_dxcore() {
        Some((name, count)) => println!("  FOUND: {} (count: {})", name, count),
        None => println!("  NOT FOUND via DXCore"),
    }
    println!();

    // Step 2: WMI GPU Engine enumeration
    println!("--- Step 2: WMI GPU Engine Enumeration ---");
    enumerate_gpu_engines();
    println!();

    // Step 3: LUID Discovery
    println!("--- Step 3: NPU LUID Discovery ---");
    let luid = discover_npu_luid();
    match &luid {
        Some(l) => println!("  NPU LUID: {}", l),
        None => println!("  NO NPU LUID FOUND"),
    }
    println!();

    // Step 4: NPU Utilization Query
    println!("--- Step 4: NPU Utilization Query ---");
    if let Some(luid) = &luid {
        match get_npu_utilization(luid) {
            Some(util) => println!("  NPU Utilization: {}%", util),
            None => println!("  FAILED to get utilization"),
        }
    } else {
        println!("  Skipped - no LUID");
    }
    println!();

    // Step 5: Try all Compute engines
    println!("--- Step 5: All Compute Engine Utilization ---");
    get_all_compute_utilization();
    println!();

    println!("=== Debug Complete ===");
}

fn detect_npu_dxcore() -> Option<(String, u32)> {
    use windows::core::GUID;
    use windows::Win32::Graphics::DXCore::*;

    // DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML
    const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML: GUID = GUID::from_values(
        0xb71b0d41,
        0x1088,
        0x422f,
        [0xa2, 0x7c, 0x02, 0x50, 0xb7, 0xd3, 0xa9, 0x88],
    );

    // DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE
    const DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE: GUID = GUID::from_values(
        0x248e2800,
        0xa793,
        0x4724,
        [0xab, 0xaa, 0x23, 0xa6, 0xde, 0x1b, 0xe0, 0x90],
    );

    // DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS
    const DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS: GUID = GUID::from_values(
        0x0c9ece4d,
        0x2f6e,
        0x4f01,
        [0x8c, 0x96, 0xe8, 0x9e, 0x33, 0x1b, 0x47, 0xb1],
    );

    const NPU_IDENTIFIERS: &[&str] = &[
        "npu", "neural", "hexagon", "ai accelerator", "ai boost", "xdna",
    ];

    unsafe {
        println!("  Creating DXCore factory...");
        let factory: IDXCoreAdapterFactory = match DXCoreCreateAdapterFactory() {
            Ok(f) => f,
            Err(e) => {
                println!("  ERROR: Failed to create factory: {:?}", e);
                return None;
            }
        };

        // Try GENERIC_ML first
        println!("  Querying GENERIC_ML adapters...");
        let adapter_list: IDXCoreAdapterList = match factory
            .CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_GENERIC_ML])
        {
            Ok(list) => list,
            Err(e) => {
                println!("  GENERIC_ML failed: {:?}, trying CORE_COMPUTE...", e);
                match factory.CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE]) {
                    Ok(list) => list,
                    Err(e2) => {
                        println!("  CORE_COMPUTE also failed: {:?}", e2);
                        return None;
                    }
                }
            }
        };

        let adapter_count = adapter_list.GetAdapterCount();
        println!("  Found {} adapters", adapter_count);

        for i in 0..adapter_count {
            let adapter: IDXCoreAdapter = match adapter_list.GetAdapter(i) {
                Ok(a) => a,
                Err(_) => continue,
            };

            // Check capabilities
            let has_compute = adapter.IsAttributeSupported(&DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE);
            let has_graphics = adapter.IsAttributeSupported(&DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS);

            // Get driver description
            let desc_size = match adapter.GetPropertySize(DriverDescription) {
                Ok(s) if s > 0 => s,
                _ => continue,
            };

            let mut desc_buffer: Vec<u8> = vec![0; desc_size];
            if adapter
                .GetProperty(DriverDescription, desc_size, desc_buffer.as_mut_ptr() as *mut _)
                .is_err()
            {
                continue;
            }

            let name = String::from_utf8_lossy(&desc_buffer)
                .trim_end_matches('\0')
                .to_string();

            println!(
                "  Adapter {}: '{}' (compute={}, graphics={})",
                i, name, has_compute, has_graphics
            );

            let name_lower = name.to_lowercase();

            // Check if it's an NPU
            let is_npu_by_caps = has_compute && !has_graphics;
            let is_npu_by_name = NPU_IDENTIFIERS.iter().any(|id| name_lower.contains(id));

            if is_npu_by_caps || is_npu_by_name {
                println!("  -> This is an NPU!");
                return Some((name, 1));
            }
        }

        None
    }
}

// Inline WMI helper
mod wmi_helper {
    use anyhow::{anyhow, Result};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use windows::core::{BSTR, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoSetProxyBlanket,
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_DEFAULT,
        RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices,
        WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
        WBEM_INFINITE,
    };
    use windows::Win32::System::Variant::{
        VARIANT, VT_BOOL, VT_BSTR, VT_I4, VT_I8, VT_NULL, VT_EMPTY,
        VT_UI1, VT_UI2, VT_UI4, VT_UI8,
    };

    pub struct WmiConnection {
        services: IWbemServices,
    }

    impl WmiConnection {
        pub fn new() -> Result<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

                let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| anyhow!("Failed to create WbemLocator: {}", e))?;

                let namespace_bstr = BSTR::from("root\\cimv2");
                let services = locator
                    .ConnectServer(
                        &namespace_bstr,
                        &BSTR::new(),
                        &BSTR::new(),
                        &BSTR::new(),
                        0,
                        &BSTR::new(),
                        None,
                    )
                    .map_err(|e| anyhow!("Failed to connect to WMI: {}", e))?;

                CoSetProxyBlanket(
                    &services,
                    10,  // RPC_C_AUTHN_WINNT
                    0,
                    PCWSTR::null(),
                    RPC_C_AUTHN_LEVEL_DEFAULT,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    None,
                    EOAC_NONE,
                ).ok();

                Ok(Self { services })
            }
        }

        pub fn query(&self, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
            unsafe {
                let query_lang = BSTR::from("WQL");
                let query_str = BSTR::from(wql);

                let enumerator = self
                    .services
                    .ExecQuery(
                        &query_lang,
                        &query_str,
                        WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                        None,
                    )
                    .map_err(|e| anyhow!("WMI query failed: {}", e))?;

                let mut results = Vec::new();

                loop {
                    let mut objects: [Option<IWbemClassObject>; 1] = [None];
                    let mut returned: u32 = 0;

                    let hr = enumerator.Next(WBEM_INFINITE, &mut objects, &mut returned);

                    if hr.is_err() || returned == 0 {
                        break;
                    }

                    if let Some(obj) = objects[0].take() {
                        let props = self.extract_properties(&obj)?;
                        results.push(props);
                    }
                }

                Ok(results)
            }
        }

        fn extract_properties(&self, obj: &IWbemClassObject) -> Result<HashMap<String, Value>> {
            let mut props = HashMap::new();

            unsafe {
                obj.BeginEnumeration(0).ok();

                loop {
                    let mut name = BSTR::new();
                    let mut value = VARIANT::default();
                    let mut cim_type: i32 = 0;
                    let mut flavor: i32 = 0;

                    let hr = obj.Next(0, &mut name, &mut value, &mut cim_type, &mut flavor);

                    if hr.is_err() {
                        break;
                    }

                    let prop_name = name.to_string();
                    if prop_name.is_empty() {
                        break;
                    }

                    let json_value = self.variant_to_json(&value);
                    props.insert(prop_name, json_value);
                }

                obj.EndEnumeration().ok();
            }

            Ok(props)
        }

        fn variant_to_json(&self, variant: &VARIANT) -> Value {
            unsafe {
                let vt = variant.Anonymous.Anonymous.vt;

                match vt {
                    VT_NULL | VT_EMPTY => Value::Null,
                    VT_BOOL => {
                        let b = variant.Anonymous.Anonymous.Anonymous.boolVal;
                        json!(b.0 != 0)
                    }
                    VT_BSTR => {
                        let bstr = &variant.Anonymous.Anonymous.Anonymous.bstrVal;
                        if bstr.len() == 0 {
                            json!("")
                        } else {
                            json!(bstr.to_string())
                        }
                    }
                    VT_I4 => {
                        let i = variant.Anonymous.Anonymous.Anonymous.lVal;
                        json!(i)
                    }
                    VT_I8 => {
                        let i = variant.Anonymous.Anonymous.Anonymous.llVal;
                        json!(i)
                    }
                    VT_UI1 => {
                        let u = variant.Anonymous.Anonymous.Anonymous.bVal;
                        json!(u)
                    }
                    VT_UI2 => {
                        let u = variant.Anonymous.Anonymous.Anonymous.uiVal;
                        json!(u)
                    }
                    VT_UI4 => {
                        let u = variant.Anonymous.Anonymous.Anonymous.ulVal;
                        json!(u)
                    }
                    VT_UI8 => {
                        let u = variant.Anonymous.Anonymous.Anonymous.ullVal;
                        json!(u)
                    }
                    _ => Value::Null,
                }
            }
        }
    }
}

fn enumerate_gpu_engines() {
    use wmi_helper::WmiConnection;

    println!("  Creating WMI connection...");
    let wmi_con = match WmiConnection::new() {
        Ok(c) => c,
        Err(e) => {
            println!("  ERROR: Failed to create WMI connection: {}", e);
            return;
        }
    };

    println!("  Querying Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine...");
    let results: Vec<HashMap<String, Value>> = match wmi_con.query(
        "SELECT Name, UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine",
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("  ERROR: Query failed: {}", e);
            return;
        }
    };

    println!("  Found {} GPU engine entries", results.len());

    // Group by LUID
    let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();
    let mut luid_utils: HashMap<String, Vec<u64>> = HashMap::new();

    for (idx, counter) in results.iter().enumerate() {
        if let Some(name) = counter.get("Name").and_then(|v| v.as_str()) {
            // Parse LUID and engine type
            if let Some(luid_start) = name.find("luid_") {
                if let Some(phys_start) = name.find("_phys_") {
                    let luid = &name[luid_start..phys_start];

                    if let Some(engtype_start) = name.find("_engtype_") {
                        let engtype = &name[engtype_start + 9..];
                        luid_engines
                            .entry(luid.to_string())
                            .or_default()
                            .insert(engtype.to_string());

                        // Also track utilization
                        if let Some(util) = counter.get("UtilizationPercentage") {
                            let util_val = util.as_u64().or_else(|| util.as_i64().map(|i| i as u64));
                            if let Some(u) = util_val {
                                luid_utils.entry(luid.to_string()).or_default().push(u);
                            }
                        }
                    }
                }
            }

            // Print first 10 entries for visibility
            if idx < 10 {
                let util = counter.get("UtilizationPercentage")
                    .map(|v| format!("{:?}", v))
                    .unwrap_or_else(|| "N/A".to_string());
                println!("    [{}] Name: {}", idx, name);
                println!("         Util: {}", util);
            }
        }
    }

    if results.len() > 10 {
        println!("    ... ({} more entries)", results.len() - 10);
    }

    println!("\n  LUIDs and their engine types:");
    for (luid, engines) in &luid_engines {
        let engines_list: Vec<&str> = engines.iter().map(|s| s.as_str()).collect();
        let utils = luid_utils.get(luid).map(|v| {
            let sum: u64 = v.iter().sum();
            format!("total_util={}", sum)
        }).unwrap_or_default();

        println!("    {} -> {:?} {}", luid, engines_list, utils);

        // Check if this looks like an NPU
        if engines.len() == 1 && engines.contains("Compute") {
            println!("      ^ LIKELY NPU (only Compute engine)");
        } else if !engines.contains("3D") && engines.contains("Compute") {
            println!("      ^ POSSIBLE NPU (Compute without 3D)");
        }
    }
}

fn discover_npu_luid() -> Option<String> {
    use wmi_helper::WmiConnection;

    let wmi_con = WmiConnection::new().ok()?;

    let results: Vec<HashMap<String, Value>> = wmi_con.query(
        "SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine",
    ).ok()?;

    let mut luid_engines: HashMap<String, HashSet<String>> = HashMap::new();

    for counter in &results {
        if let Some(name) = counter.get("Name").and_then(|v| v.as_str()) {
            if let Some(luid_start) = name.find("luid_") {
                if let Some(phys_start) = name.find("_phys_") {
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
        }
    }

    // First: find device with ONLY Compute engine
    for (luid, engine_types) in &luid_engines {
        if engine_types.len() == 1 && engine_types.contains("Compute") {
            return Some(luid.clone());
        }
    }

    // Second: find device without 3D but with Compute
    for (luid, engine_types) in &luid_engines {
        if !engine_types.contains("3D") && engine_types.contains("Compute") {
            return Some(luid.clone());
        }
    }

    None
}

fn get_npu_utilization(luid: &str) -> Option<f32> {
    use wmi_helper::WmiConnection;

    let wmi_con = WmiConnection::new().ok()?;

    let query = format!(
        "SELECT UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine WHERE Name LIKE '%{}%'",
        luid
    );

    println!("  Query: {}", query);

    let results: Vec<HashMap<String, Value>> = match wmi_con.query(&query) {
        Ok(r) => r,
        Err(e) => {
            println!("  ERROR: Query failed: {}", e);
            return None;
        }
    };

    println!("  Got {} results", results.len());

    let mut total_util: u64 = 0;
    let mut count = 0;

    for (idx, counter) in results.iter().enumerate() {
        if let Some(util) = counter.get("UtilizationPercentage") {
            println!("    [{}] UtilizationPercentage = {:?}", idx, util);
            let util_val = util.as_u64().or_else(|| util.as_i64().map(|i| i as u64));
            if let Some(u) = util_val {
                total_util += u;
                count += 1;
            }
        }
    }

    println!("  Total: {}, Count: {}", total_util, count);

    if count > 0 {
        Some((total_util as f32).min(100.0))
    } else {
        None
    }
}

fn get_all_compute_utilization() {
    use wmi_helper::WmiConnection;

    let wmi_con = match WmiConnection::new() {
        Ok(c) => c,
        Err(e) => {
            println!("  ERROR: Failed to create WMI connection: {}", e);
            return;
        }
    };

    let query = "SELECT Name, UtilizationPercentage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine WHERE Name LIKE '%Compute%'";
    println!("  Query: {}", query);

    let results: Vec<HashMap<String, Value>> = match wmi_con.query(query) {
        Ok(r) => r,
        Err(e) => {
            println!("  ERROR: Query failed: {}", e);
            return;
        }
    };

    println!("  Found {} Compute engines", results.len());

    for (idx, counter) in results.iter().enumerate() {
        let name = counter.get("Name").and_then(|v| v.as_str()).unwrap_or("?");
        let util = counter.get("UtilizationPercentage")
            .map(|v| format!("{:?}", v))
            .unwrap_or_else(|| "N/A".to_string());

        // Check if it's NOT a GPU (no 3D in the name pattern)
        let is_3d_device = name.contains("_engtype_3D") || name.contains("engtype_3D");
        let marker = if is_3d_device { "(GPU)" } else { "(possible NPU)" };

        println!("    [{}] {} {}", idx, name, marker);
        println!("         Util: {}", util);
    }
}
