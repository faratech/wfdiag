//! Native WMI wrapper using windows-rs crate directly
//! Replaces the wmi crate with direct COM calls

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
    VARIANT, VariantClear, VT_BOOL, VT_BSTR, VT_I1, VT_I2, VT_I4, VT_I8, VT_NULL, VT_EMPTY,
    VT_UI1, VT_UI2, VT_UI4, VT_UI8, VT_R4, VT_R8, VT_ARRAY, VT_DATE,
};

// Thread-local COM initialization state
thread_local! {
    static COM_INITIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Initialize COM for the current thread if not already done
fn ensure_com_initialized() -> Result<()> {
    COM_INITIALIZED.with(|initialized| {
        if !initialized.get() {
            unsafe {
                // Initialize COM - use COINIT_APARTMENTTHREADED for better compatibility
                // with GUI apps like Tauri. The result can be S_OK, S_FALSE (already initialized),
                // or RPC_E_CHANGED_MODE (different mode) - all are acceptable.
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);

                // S_OK = success, S_FALSE = already initialized (fine)
                // RPC_E_CHANGED_MODE = already in STA (also fine, we can still use WMI)
                if hr.is_err() {
                    let code = hr.0 as u32;
                    // 0x80010106 = RPC_E_CHANGED_MODE - COM already initialized in different mode
                    // This is OK - we can still use WMI
                    if code != 0x80010106 && code != 1 { // 1 = S_FALSE
                        // Only log, don't fail - COM might still work
                        eprintln!("CoInitializeEx returned: 0x{:08X}", code);
                    }
                }

                // CoInitializeSecurity can only be called once per process.
                // If it fails, that's OK - security was likely already set by Tauri.
                // We don't need to set it ourselves.
            }
            initialized.set(true);
        }
        Ok(())
    })
}

/// Native WMI connection
/// Note: WMI COM objects are thread-local. This connection must be used
/// and dropped on the same thread it was created on.
pub struct WmiConnection {
    services: IWbemServices,
    // PhantomData to prevent Send/Sync auto-implementation
    _marker: std::marker::PhantomData<*const ()>,
}

impl WmiConnection {
    /// Create a new WMI connection to the default namespace (root\cimv2)
    pub fn new() -> Result<Self> {
        Self::with_namespace("root\\cimv2")
    }

    /// Create a WMI connection to a specific namespace
    pub fn with_namespace(namespace: &str) -> Result<Self> {
        ensure_com_initialized()?;

        unsafe {
            // Create WbemLocator instance
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| anyhow!("Failed to create WbemLocator: {}", e))?;

            // Connect to the namespace
            let namespace_bstr = BSTR::from(namespace);
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
                .map_err(|e| anyhow!("Failed to connect to WMI namespace '{}': {}", namespace, e))?;

            // Set security on the proxy
            // CoSetProxyBlanket(proxy, authn_svc, authz_svc, server_princ_name, authn_level, imp_level, auth_info, capabilities)
            CoSetProxyBlanket(
                &services,
                10,  // RPC_C_AUTHN_WINNT
                0,   // RPC_C_AUTHZ_NONE
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .ok();

            Ok(Self {
                services,
                _marker: std::marker::PhantomData,
            })
        }
    }

    /// Execute a WQL query and return results as JSON
    pub fn query(&self, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
        // Wrap entire query in catch_unwind for safety
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.query_internal(wql)
        }));

        match result {
            Ok(r) => r,
            Err(_) => Err(anyhow!("WMI query panicked")),
        }
    }

    fn query_internal(&self, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
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
                .map_err(|e| anyhow!("WMI query failed for '{}': {}", wql, e))?;

            let mut results = Vec::new();

            loop {
                let mut objects: [Option<IWbemClassObject>; 1] = [None];
                let mut returned: u32 = 0;

                let hr = enumerator.Next(
                    WBEM_INFINITE,
                    &mut objects,
                    &mut returned,
                );

                if hr.is_err() || returned == 0 {
                    break;
                }

                if let Some(obj) = objects[0].take() {
                    // Use catch_unwind to prevent panics from crashing the app
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.extract_properties(&obj)
                    })) {
                        Ok(Ok(props)) => results.push(props),
                        Ok(Err(_)) | Err(_) => {
                            // Skip objects that fail to extract
                        }
                    }
                }
            }

            Ok(results)
        }
    }

    /// Execute a simple SELECT * query on a WMI class
    pub fn query_class(&self, class_name: &str) -> Result<Vec<HashMap<String, Value>>> {
        self.query(&format!("SELECT * FROM {}", class_name))
    }

    /// Extract all properties from a WMI object
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

                // Free the VARIANT's heap payload (BSTR / SAFEARRAY / COM ref). windows
                // 0.62's VARIANT has no Drop, so without this every string/array-valued
                // WMI property leaks for the life of the process. `value` is re-zeroed by
                // VARIANT::default() at the top of the next iteration.
                let _ = VariantClear(&mut value);
            }

            obj.EndEnumeration().ok();
        }

        Ok(props)
    }

    /// Convert a VARIANT to JSON Value
    fn variant_to_json(&self, variant: &VARIANT) -> Value {
        unsafe {
            let vt = variant.Anonymous.Anonymous.vt;

            // Handle arrays first (check the VT_ARRAY flag)
            if (vt.0 & VT_ARRAY.0) != 0 {
                return self.safearray_to_json(variant);
            }

            match vt {
                VT_NULL | VT_EMPTY => Value::Null,

                VT_BOOL => {
                    let b = variant.Anonymous.Anonymous.Anonymous.boolVal;
                    json!(b.0 != 0)
                }

                VT_BSTR => {
                    // Use the windows crate's safe BSTR handling
                    let bstr = &variant.Anonymous.Anonymous.Anonymous.bstrVal;
                    // BSTR len() returns 0 for null/empty, and to_string() is safe
                    if bstr.len() == 0 {
                        json!("")
                    } else {
                        json!(bstr.to_string())
                    }
                }

                VT_I1 => {
                    let i = variant.Anonymous.Anonymous.Anonymous.cVal;
                    json!(i as i8)
                }

                VT_I2 => {
                    let i = variant.Anonymous.Anonymous.Anonymous.iVal;
                    json!(i)
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

                VT_R4 => {
                    let f = variant.Anonymous.Anonymous.Anonymous.fltVal;
                    json!(f)
                }

                VT_R8 => {
                    let f = variant.Anonymous.Anonymous.Anonymous.dblVal;
                    json!(f)
                }

                VT_DATE => {
                    let d = variant.Anonymous.Anonymous.Anonymous.date;
                    json!(d)
                }

                _ => {
                    // For other types, return null to avoid crashes
                    Value::Null
                }
            }
        }
    }

    /// Convert a SafeArray VARIANT to JSON array
    fn safearray_to_json(&self, variant: &VARIANT) -> Value {
        unsafe {
            use windows::Win32::System::Ole::{
                SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayGetElement,
            };

            let psa = variant.Anonymous.Anonymous.Anonymous.parray;
            if psa.is_null() {
                return json!([]);
            }

            let lbound = match SafeArrayGetLBound(psa, 1) {
                Ok(lb) => lb,
                Err(_) => return json!([]),
            };
            let ubound = match SafeArrayGetUBound(psa, 1) {
                Ok(ub) => ub,
                Err(_) => return json!([]),
            };

            if ubound < lbound {
                return json!([]);
            }

            let count = (ubound - lbound + 1) as usize;
            if count > 1000 {
                return json!(format!("[Array: {} elements]", count));
            }

            let base_vt = variant.Anonymous.Anonymous.vt.0 & !VT_ARRAY.0;
            let mut arr = Vec::new();

            for i in lbound..=ubound {
                let idx = i as i32;

                if base_vt == VT_BSTR.0 {
                    // SafeArrayGetElement for BSTR returns a copy that we own
                    let mut bstr: BSTR = BSTR::new();
                    if SafeArrayGetElement(psa, &idx, &mut bstr as *mut _ as *mut std::ffi::c_void).is_ok() {
                        // BSTR::to_string() handles null and empty cases
                        let s = bstr.to_string();
                        arr.push(json!(s));
                        // BSTR will be freed when it goes out of scope (Drop impl)
                    }
                } else if base_vt == VT_I4.0 {
                    let mut val: i32 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI1.0 {
                    let mut val: u8 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI2.0 {
                    let mut val: u16 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI4.0 {
                    let mut val: u32 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_I2.0 {
                    let mut val: i16 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_I8.0 {
                    let mut val: i64 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI8.0 {
                    let mut val: u64 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_R4.0 {
                    let mut val: f32 = 0.0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_R8.0 {
                    let mut val: f64 = 0.0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_BOOL.0 {
                    let mut val: i16 = 0;
                    if SafeArrayGetElement(psa, &idx, &mut val as *mut _ as *mut std::ffi::c_void).is_ok() {
                        arr.push(json!(val != 0));
                    }
                }
                // Skip unknown types
            }

            if arr.is_empty() && count > 0 {
                json!(format!("[Array: {} elements, type 0x{:04X}]", count, base_vt))
            } else {
                json!(arr)
            }
        }
    }
}

// Convenience functions (currently unused but available for future use)
#[allow(dead_code)]
pub fn wmi_query(wql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::new()?;
    conn.query(wql)
}

#[allow(dead_code)]
pub fn wmi_query_ns(namespace: &str, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::with_namespace(namespace)?;
    conn.query(wql)
}

#[allow(dead_code)]
pub fn wmi_query_class(class_name: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::new()?;
    conn.query_class(class_name)
}

#[allow(dead_code)]
pub fn get_string(props: &HashMap<String, Value>, key: &str) -> Option<String> {
    props.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

#[allow(dead_code)]
pub fn get_u64(props: &HashMap<String, Value>, key: &str) -> Option<u64> {
    props.get(key).and_then(|v| v.as_u64())
}

#[allow(dead_code)]
pub fn get_i64(props: &HashMap<String, Value>, key: &str) -> Option<i64> {
    props.get(key).and_then(|v| v.as_i64())
}

#[allow(dead_code)]
pub fn get_u32(props: &HashMap<String, Value>, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| v.as_u64()).map(|u| u as u32)
}

#[allow(dead_code)]
pub fn get_bool(props: &HashMap<String, Value>, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| v.as_bool())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wmi_connection() {
        let conn = WmiConnection::new();
        assert!(conn.is_ok());
    }
}
