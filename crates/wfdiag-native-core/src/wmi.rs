//! Native WMI wrapper using windows-rs crate directly
//! Replaces the wmi crate with direct COM calls

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::BuildHasher;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{
    VARIANT, VT_ARRAY, VT_BOOL, VT_BSTR, VT_DATE, VT_EMPTY, VT_I1, VT_I2, VT_I4, VT_I8, VT_NULL,
    VT_R4, VT_R8, VT_UI1, VT_UI2, VT_UI4, VT_UI8, VariantClear,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WbemLocator,
};
use windows::core::{BSTR, PCWSTR};

const WMI_NEXT_TIMEOUT_MS: i32 = 30_000;
const WBEM_S_TIMEDOUT: i32 = 0x0004_0004;

// Thread-local COM initialization state
thread_local! {
    static COM_INITIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Initialize COM for the current thread if not already done.
///
/// # Apartment choice, and why there is no `CoUninitialize` (issue #224)
///
/// The thread joins the **multi-threaded** apartment (`COINIT_MULTITHREADED`).
/// A stale comment here used to claim `COINIT_APARTMENTTHREADED` "for better
/// compatibility with GUI apps like Tauri"; the code has always passed
/// `COINIT_MULTITHREADED` and must keep doing so — WMI queries run on Tokio
/// blocking-pool threads, which have no message pump, and an STA there would
/// deadlock the first cross-apartment call.
///
/// The matching `CoUninitialize` is deliberately omitted: initialization is
/// once per thread and scoped to that thread's whole lifetime. A
/// [`WmiConnection`] holds live `IWbemServices` proxies that may outlast any
/// single call on the thread, so uninitializing at the end of a query would
/// invalidate them. The threads involved live until process exit, at which
/// point the apartment is torn down with the process anyway.
fn ensure_com_initialized() -> Result<()> {
    COM_INITIALIZED.with(|initialized| {
        if !initialized.get() {
            unsafe {
                // The result can be S_OK, S_FALSE (already initialized), or
                // RPC_E_CHANGED_MODE (this thread is already in an STA) - all
                // are acceptable, WMI still works.
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);

                // S_OK = success, S_FALSE = already initialized (fine)
                // RPC_E_CHANGED_MODE = already in STA (also fine, we can still use WMI)
                if hr.is_err() {
                    let code = hr.0.cast_unsigned();
                    // 0x8001_0106 = RPC_E_CHANGED_MODE - COM already initialized in different mode
                    // This is OK - we can still use WMI
                    if code != 0x8001_0106 && code != 1 {
                        // 1 = S_FALSE
                        // Only log, don't fail - COM might still work
                        eprintln!("CoInitializeEx returned: 0x{code:08X}");
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
    ///
    /// # Errors
    /// Returns an error when COM cannot be initialized, `WbemLocator` cannot
    /// be created, or the namespace connection is refused.
    pub fn new() -> Result<Self> {
        Self::with_namespace("root\\cimv2")
    }

    /// Create a WMI connection to a specific namespace
    ///
    /// # Errors
    /// Returns an error when COM cannot be initialized, `WbemLocator` cannot
    /// be created, or `namespace` cannot be connected to.
    pub fn with_namespace(namespace: &str) -> Result<Self> {
        ensure_com_initialized()?;

        unsafe {
            // Create WbemLocator instance
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| anyhow!("Failed to create WbemLocator: {e}"))?;

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
                .map_err(|e| anyhow!("Failed to connect to WMI namespace '{namespace}': {e}"))?;

            // Set security on the proxy
            // CoSetProxyBlanket(proxy, authn_svc, authz_svc, server_princ_name, authn_level, imp_level, auth_info, capabilities)
            CoSetProxyBlanket(
                &services,
                10, // RPC_C_AUTHN_WINNT
                0,  // RPC_C_AUTHZ_NONE
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
    ///
    /// # Errors
    /// Returns an error when the provider rejects the query, enumeration
    /// fails or times out, or the query panics inside the COM layer.
    pub fn query(&self, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
        // Wrap entire query in catch_unwind for safety
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.query_internal(wql)));

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
                .map_err(|e| anyhow!("WMI query failed for '{wql}': {e}"))?;

            let mut results = Vec::new();

            loop {
                let mut objects: [Option<IWbemClassObject>; 1] = [None];
                let mut returned: u32 = 0;

                let hr = enumerator.Next(WMI_NEXT_TIMEOUT_MS, &mut objects, &raw mut returned);

                if hr.0 == WBEM_S_TIMEDOUT {
                    return Err(anyhow!("WMI query timed out for '{wql}'"));
                }
                if hr.is_err() {
                    return Err(anyhow!("WMI enumeration failed for '{wql}': {hr:?}"));
                }
                if returned == 0 {
                    break;
                }

                if let Some(obj) = objects[0].take() {
                    // Use catch_unwind to prevent panics from crashing the app;
                    // an object that panics while extracting is skipped, not fatal.
                    if let Ok(props) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            self.extract_properties(&obj)
                        }))
                    {
                        results.push(props);
                    }
                }
            }

            Ok(results)
        }
    }

    /// Execute a simple SELECT * query on a WMI class
    ///
    /// # Errors
    /// Same failures as [`WmiConnection::query`].
    pub fn query_class(&self, class_name: &str) -> Result<Vec<HashMap<String, Value>>> {
        self.query(&format!("SELECT * FROM {class_name}"))
    }

    /// Extract all properties from a WMI object. Properties that fail to read
    /// are skipped rather than failing the object, so this cannot fail.
    fn extract_properties(&self, obj: &IWbemClassObject) -> HashMap<String, Value> {
        let mut props = HashMap::new();

        unsafe {
            obj.BeginEnumeration(0).ok();

            loop {
                let mut name = BSTR::new();
                let mut value = VARIANT::default();
                let mut cim_type: i32 = 0;
                let mut flavor: i32 = 0;

                let hr = obj.Next(
                    0,
                    &raw mut name,
                    &raw mut value,
                    &raw mut cim_type,
                    &raw mut flavor,
                );

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
                let _ = VariantClear(&raw mut value);
            }

            obj.EndEnumeration().ok();
        }

        props
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
                    if bstr.is_empty() {
                        json!("")
                    } else {
                        json!(bstr.to_string())
                    }
                }

                VT_I1 => {
                    let i = variant.Anonymous.Anonymous.Anonymous.cVal;
                    json!(i)
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

    /// Convert a `SafeArray` VARIANT to JSON array
    // One arm per SAFEARRAY element type, so the length is inherent; the
    // element count is non-negative by the `ubound < lbound` guard above it,
    // and `self` is kept for symmetry with the other converters.
    #[allow(
        clippy::too_many_lines,
        clippy::unused_self,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )]
    fn safearray_to_json(&self, variant: &VARIANT) -> Value {
        unsafe {
            use windows::Win32::System::Ole::{
                SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
            };

            let psa = variant.Anonymous.Anonymous.Anonymous.parray;
            if psa.is_null() {
                return json!([]);
            }

            let Ok(lbound) = SafeArrayGetLBound(psa, 1) else {
                return json!([]);
            };
            let Ok(ubound) = SafeArrayGetUBound(psa, 1) else {
                return json!([]);
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
                let idx = i;

                if base_vt == VT_BSTR.0 {
                    // SafeArrayGetElement for BSTR returns a copy that we own
                    let mut bstr: BSTR = BSTR::new();
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut bstr).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        // BSTR::to_string() handles null and empty cases
                        let s = bstr.to_string();
                        arr.push(json!(s));
                        // BSTR will be freed when it goes out of scope (Drop impl)
                    }
                } else if base_vt == VT_I4.0 {
                    let mut val: i32 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI1.0 {
                    let mut val: u8 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI2.0 {
                    let mut val: u16 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI4.0 {
                    let mut val: u32 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_I2.0 {
                    let mut val: i16 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_I8.0 {
                    let mut val: i64 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_UI8.0 {
                    let mut val: u64 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_R4.0 {
                    let mut val: f32 = 0.0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_R8.0 {
                    let mut val: f64 = 0.0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val));
                    }
                } else if base_vt == VT_BOOL.0 {
                    let mut val: i16 = 0;
                    if SafeArrayGetElement(
                        psa,
                        &raw const idx,
                        (&raw mut val).cast::<std::ffi::c_void>(),
                    )
                    .is_ok()
                    {
                        arr.push(json!(val != 0));
                    }
                }
                // Skip unknown types
            }

            if arr.is_empty() && count > 0 {
                json!(format!(
                    "[Array: {} elements, type 0x{:04X}]",
                    count, base_vt
                ))
            } else {
                json!(arr)
            }
        }
    }
}

// Convenience functions (currently unused but available for future use)
/// Run one WQL query against `root\cimv2` on a fresh connection.
///
/// # Errors
/// Same failures as [`WmiConnection::new`] and [`WmiConnection::query`].
#[allow(dead_code)]
pub fn wmi_query(wql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::new()?;
    conn.query(wql)
}

/// Run one WQL query against `namespace` on a fresh connection.
///
/// # Errors
/// Same failures as [`WmiConnection::with_namespace`] and
/// [`WmiConnection::query`].
#[allow(dead_code)]
pub fn wmi_query_ns(namespace: &str, wql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::with_namespace(namespace)?;
    conn.query(wql)
}

/// `SELECT * FROM <class_name>` against `root\cimv2` on a fresh connection.
///
/// # Errors
/// Same failures as [`WmiConnection::new`] and [`WmiConnection::query_class`].
#[allow(dead_code)]
pub fn wmi_query_class(class_name: &str) -> Result<Vec<HashMap<String, Value>>> {
    let conn = WmiConnection::new()?;
    conn.query_class(class_name)
}

#[allow(dead_code)]
#[must_use]
pub fn get_string<S: BuildHasher>(props: &HashMap<String, Value, S>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

#[allow(dead_code)]
#[must_use]
pub fn get_u64<S: BuildHasher>(props: &HashMap<String, Value, S>, key: &str) -> Option<u64> {
    props.get(key).and_then(serde_json::Value::as_u64)
}

#[allow(dead_code)]
#[must_use]
pub fn get_i64<S: BuildHasher>(props: &HashMap<String, Value, S>, key: &str) -> Option<i64> {
    props.get(key).and_then(serde_json::Value::as_i64)
}

#[allow(dead_code)]
#[must_use]
// Historical behaviour: WMI u32 properties round-trip through JSON as u64,
// and an oversized value truncates rather than disappearing.
#[allow(clippy::cast_possible_truncation)]
pub fn get_u32<S: BuildHasher>(props: &HashMap<String, Value, S>, key: &str) -> Option<u32> {
    props
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|u| u as u32)
}

#[allow(dead_code)]
#[must_use]
pub fn get_bool<S: BuildHasher>(props: &HashMap<String, Value, S>, key: &str) -> Option<bool> {
    props.get(key).and_then(serde_json::Value::as_bool)
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
