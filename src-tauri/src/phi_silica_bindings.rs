//! Hand-written WinRT bindings for Microsoft.Windows.AI.Text.LanguageModel
//!
//! These bindings interface with the Windows App SDK AI runtime provided by AI Dev Gallery.
//! The WinRT classes are activated through RoGetActivationFactory when running from an MSIX package.
//!
//! All async operations are wrapped to be Send-safe for Tauri commands.

#![cfg(windows)]
#![allow(dead_code)]
#![allow(non_snake_case)]

use tokio::sync::oneshot;
use windows::core::{Result, HSTRING, IInspectable, GUID, Interface};
use windows::Win32::System::WinRT::{RoGetActivationFactory, RoInitialize, RO_INIT_MULTITHREADED};
use windows_future::{IAsyncOperation, IAsyncOperationWithProgress, IAsyncInfo, AsyncStatus};

// Note: Direct DLL loading and Bootstrap initialization both cause issues.
// See CLAUDE.md "Phi Silica Integration" section for details on what was tried.

/// A Send-safe wrapper for raw COM pointers
/// SAFETY: COM objects initialized with MTA are thread-safe
#[derive(Clone, Copy)]
struct SendPtr(*mut core::ffi::c_void);
unsafe impl Send for SendPtr {}

// Microsoft.Windows.AI.AIFeatureReadyState enum values
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIFeatureReadyState {
    NotReady = 0,
    Ready = 1,
    DisabledByUser = 2,
    NotSupportedOnCurrentSystem = 3,
}

impl AIFeatureReadyState {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::NotReady,
            1 => Self::Ready,
            2 => Self::DisabledByUser,
            3 => Self::NotSupportedOnCurrentSystem,
            _ => Self::NotSupportedOnCurrentSystem,
        }
    }
}

// Microsoft.Windows.AI.AIFeatureReadyResultState enum values
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIFeatureReadyResultState {
    Success = 0,
    Failure = 1,
    Busy = 2,
}

impl AIFeatureReadyResultState {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Success,
            1 => Self::Failure,
            2 => Self::Busy,
            _ => Self::Failure,
        }
    }
}

// Microsoft.Windows.AI.Text.LanguageModelResponseStatus enum values
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageModelResponseStatus {
    Complete = 0,
    InProgress = 1,
    BlockedByPolicy = 2,
    PromptBlockedByContentModeration = 3,
    ResponseBlockedByContentModeration = 4,
    PromptLargerThanContext = 5,
    Error = 6,
}

impl LanguageModelResponseStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Complete,
            1 => Self::InProgress,
            2 => Self::BlockedByPolicy,
            3 => Self::PromptBlockedByContentModeration,
            4 => Self::ResponseBlockedByContentModeration,
            5 => Self::PromptLargerThanContext,
            6 => Self::Error,
            _ => Self::Error,
        }
    }
}

// ILanguageModelStatics GUID (from WinMD)
const IID_ILANGUAGEMODELSTATICS: GUID = GUID::from_u128(0xC9AC7ADD_5BA7_5BD9_942A_0E7DB6DFD25F);

// ILanguageModel GUID (from WinMD)
const IID_ILANGUAGEMODEL: GUID = GUID::from_u128(0x1DD77B9A_CB95_5E1B_A4A2_85C0FF93FF9B);

// ILanguageModelResponse GUID
const IID_ILANGUAGEMODELRESPONSE: GUID = GUID::from_u128(0xA48B48C8_0D5C_5B23_852A_EEB42D3C5E25);

/// Vtable for ILanguageModelStatics
#[repr(C)]
struct ILanguageModelStaticsVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    // IInspectable
    get_iids: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32, *mut *mut GUID) -> windows::core::HRESULT,
    get_runtime_class_name: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    get_trust_level: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> windows::core::HRESULT,
    // ILanguageModelStatics methods
    get_ready_state: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> windows::core::HRESULT,
    ensure_ready_async: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    create_async: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
}

/// Vtable for ILanguageModel instance
#[repr(C)]
struct ILanguageModelVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    // IInspectable
    get_iids: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32, *mut *mut GUID) -> windows::core::HRESULT,
    get_runtime_class_name: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    get_trust_level: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> windows::core::HRESULT,
    // ILanguageModel methods - these need to be in correct order
    generate_response_async: unsafe extern "system" fn(*mut core::ffi::c_void, *const core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
}

/// Vtable for ILanguageModelResponse
#[repr(C)]
struct ILanguageModelResponseVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    // IInspectable
    get_iids: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32, *mut *mut GUID) -> windows::core::HRESULT,
    get_runtime_class_name: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    get_trust_level: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> windows::core::HRESULT,
    // ILanguageModelResponse properties
    get_text: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> windows::core::HRESULT,
    get_status: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> windows::core::HRESULT,
}

/// LanguageModel static methods wrapper
pub struct LanguageModel;

impl LanguageModel {
    /// Check if Phi Silica is ready (synchronous - safe to call from async context)
    pub fn get_ready_state() -> Result<AIFeatureReadyState> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");
        let factory: IInspectable = unsafe { RoGetActivationFactory(&class_name)? };

        // Query for ILanguageModelStatics
        let statics_ptr = factory.as_raw();

        // Get vtable
        let vtbl = unsafe { *(statics_ptr as *const *const ILanguageModelStaticsVtbl) };

        let mut result: i32 = 0;
        let hr = unsafe { ((*vtbl).get_ready_state)(statics_ptr as *mut _, &mut result) };

        if hr.is_ok() {
            Ok(AIFeatureReadyState::from_i32(result))
        } else {
            Err(windows::core::Error::from(hr))
        }
    }

    /// Ensure the model is ready (may trigger download)
    /// This is Send-safe by running WinRT operations in a blocking thread
    pub async fn ensure_ready_async() -> std::result::Result<AIFeatureReadyResultState, String> {
        let (tx, rx) = oneshot::channel();

        std::thread::spawn(move || {
            let result = Self::ensure_ready_blocking();
            let _ = tx.send(result);
        });

        rx.await.map_err(|_| "Channel closed".to_string())?
    }

    /// Blocking version of ensure_ready
    fn ensure_ready_blocking() -> std::result::Result<AIFeatureReadyResultState, String> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");
        let factory: IInspectable = unsafe {
            RoGetActivationFactory(&class_name).map_err(|e| e.message().to_string())?
        };

        let statics_ptr = factory.as_raw();
        let vtbl = unsafe { *(statics_ptr as *const *const ILanguageModelStaticsVtbl) };

        let mut async_op_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).ensure_ready_async)(statics_ptr as *mut _, &mut async_op_ptr) };

        if hr.is_err() {
            return Err(format!("Failed to start ensure_ready_async: 0x{:08X}", hr.0));
        }

        // Cast to IAsyncOperation
        let async_op: IAsyncOperation<IInspectable> = unsafe {
            IAsyncOperation::from_raw(async_op_ptr)
        };

        // Poll until complete using IAsyncInfo
        let info: IAsyncInfo = async_op.cast().map_err(|e| e.message().to_string())?;
        loop {
            let status = info.Status().map_err(|e| e.message().to_string())?;
            match status {
                AsyncStatus::Completed => break,
                AsyncStatus::Error => {
                    let hr = info.ErrorCode().map_err(|e| e.message().to_string())?;
                    return Err(format!("EnsureReadyAsync failed with error: 0x{:08X}", hr.0));
                }
                AsyncStatus::Canceled => {
                    return Err("EnsureReadyAsync was canceled".to_string());
                }
                AsyncStatus::Started => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                _ => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        // Get results
        let _result = async_op.GetResults().map_err(|e| format!("GetResults failed: {}", e.message()))?;
        Ok(AIFeatureReadyResultState::Success)
    }

    /// Create a new LanguageModel instance
    /// This is Send-safe by running WinRT operations in a blocking thread
    pub async fn create_async() -> std::result::Result<LanguageModelInstance, String> {
        let (tx, rx) = oneshot::channel();

        std::thread::spawn(move || {
            let result = Self::create_blocking();
            let _ = tx.send(result);
        });

        rx.await.map_err(|_| "Channel closed".to_string())?
    }

    /// Blocking version of create
    fn create_blocking() -> std::result::Result<LanguageModelInstance, String> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");
        let factory: IInspectable = unsafe {
            RoGetActivationFactory(&class_name).map_err(|e| e.message().to_string())?
        };

        let statics_ptr = factory.as_raw();
        let vtbl = unsafe { *(statics_ptr as *const *const ILanguageModelStaticsVtbl) };

        let mut async_op_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).create_async)(statics_ptr as *mut _, &mut async_op_ptr) };

        if hr.is_err() {
            return Err(format!("Failed to start create_async: 0x{:08X}", hr.0));
        }

        // Cast to IAsyncOperation
        let async_op: IAsyncOperation<IInspectable> = unsafe {
            IAsyncOperation::from_raw(async_op_ptr)
        };

        // Poll until complete using IAsyncInfo
        let info: IAsyncInfo = async_op.cast().map_err(|e| e.message().to_string())?;
        loop {
            let status = info.Status().map_err(|e| e.message().to_string())?;
            match status {
                AsyncStatus::Completed => break,
                AsyncStatus::Error => {
                    let hr = info.ErrorCode().map_err(|e| e.message().to_string())?;
                    return Err(format!("CreateAsync failed with error: 0x{:08X}", hr.0));
                }
                AsyncStatus::Canceled => {
                    return Err("CreateAsync was canceled".to_string());
                }
                AsyncStatus::Started => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                _ => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        // Get results
        let instance = async_op.GetResults().map_err(|e| format!("GetResults failed: {}", e.message()))?;

        // AddRef to keep the instance alive since we're storing the raw pointer
        let ptr = instance.as_raw();
        unsafe {
            let vtbl = *(ptr as *const *const ILanguageModelVtbl);
            ((*vtbl).add_ref)(ptr as *mut _);
        }

        Ok(LanguageModelInstance { inner: ptr })
    }
}

/// An instance of LanguageModel
/// We wrap in a raw pointer to make it Send (COM objects are thread-safe when using STA/MTA properly)
pub struct LanguageModelInstance {
    inner: *mut core::ffi::c_void,
}

// SAFETY: COM objects with MTA initialization are thread-safe
unsafe impl Send for LanguageModelInstance {}
unsafe impl Sync for LanguageModelInstance {}

impl Drop for LanguageModelInstance {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe {
                let vtbl = *(self.inner as *const *const ILanguageModelVtbl);
                ((*vtbl).release)(self.inner as *mut _);
            }
        }
    }
}

impl LanguageModelInstance {
    /// Generate a response for the given prompt
    /// This is Send-safe by running WinRT operations in a blocking thread
    pub async fn generate_response_async(&self, prompt: &str) -> std::result::Result<LanguageModelResponse, String> {
        let inner = SendPtr(self.inner);
        let prompt = prompt.to_string();

        let (tx, rx) = oneshot::channel();

        std::thread::spawn(move || {
            let result = Self::generate_response_blocking(inner, &prompt);
            let _ = tx.send(result);
        });

        rx.await.map_err(|_| "Channel closed".to_string())?
    }

    /// Blocking version of generate_response
    fn generate_response_blocking(inner: SendPtr, prompt: &str) -> std::result::Result<LanguageModelResponse, String> {
        let inner = inner.0;

        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let prompt_hstring = HSTRING::from(prompt);

        // Query for ILanguageModel interface from the raw pointer
        let inspectable: IInspectable = unsafe { IInspectable::from_raw(inner) };

        let mut model_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            inspectable.query(&IID_ILANGUAGEMODEL, &mut model_ptr)
        };

        // Don't drop inspectable - we're borrowing the raw pointer
        std::mem::forget(inspectable);

        if hr.is_err() {
            return Err(format!("Failed to query ILanguageModel: 0x{:08X}", hr.0));
        }

        let vtbl = unsafe { *(model_ptr as *const *const ILanguageModelVtbl) };

        let mut async_op_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            ((*vtbl).generate_response_async)(model_ptr, prompt_hstring.as_ptr() as *const _, &mut async_op_ptr)
        };

        if hr.is_err() {
            return Err(format!("Failed to start generate_response_async: 0x{:08X}", hr.0));
        }

        // Cast to IAsyncOperationWithProgress
        let async_op: IAsyncOperationWithProgress<IInspectable, HSTRING> = unsafe {
            IAsyncOperationWithProgress::from_raw(async_op_ptr)
        };

        // Poll until complete using IAsyncInfo
        let info: IAsyncInfo = async_op.cast().map_err(|e| e.message().to_string())?;
        loop {
            let status = info.Status().map_err(|e| e.message().to_string())?;
            match status {
                AsyncStatus::Completed => break,
                AsyncStatus::Error => {
                    let hr = info.ErrorCode().map_err(|e| e.message().to_string())?;
                    return Err(format!("GenerateResponseAsync failed with error: 0x{:08X}", hr.0));
                }
                AsyncStatus::Canceled => {
                    return Err("GenerateResponseAsync was canceled".to_string());
                }
                AsyncStatus::Started => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                _ => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        // Get results
        let result = async_op.GetResults().map_err(|e| format!("GetResults failed: {}", e.message()))?;

        // AddRef to keep the response alive
        let ptr = result.as_raw();
        unsafe {
            let vtbl = *(ptr as *const *const ILanguageModelResponseVtbl);
            ((*vtbl).add_ref)(ptr as *mut _);
        }

        Ok(LanguageModelResponse { inner: ptr })
    }
}

/// The response from a language model generation
pub struct LanguageModelResponse {
    inner: *mut core::ffi::c_void,
}

// SAFETY: COM objects with MTA initialization are thread-safe
unsafe impl Send for LanguageModelResponse {}
unsafe impl Sync for LanguageModelResponse {}

impl Drop for LanguageModelResponse {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe {
                let vtbl = *(self.inner as *const *const ILanguageModelResponseVtbl);
                ((*vtbl).release)(self.inner as *mut _);
            }
        }
    }
}

impl LanguageModelResponse {
    /// Get the generated text
    pub fn text(&self) -> std::result::Result<String, String> {
        // Query for ILanguageModelResponse interface
        let inspectable: IInspectable = unsafe { IInspectable::from_raw(self.inner) };

        let mut response_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            inspectable.query(&IID_ILANGUAGEMODELRESPONSE, &mut response_ptr)
        };

        // Don't drop inspectable - we're borrowing the raw pointer
        std::mem::forget(inspectable);

        if hr.is_err() {
            return Err(format!("Failed to query ILanguageModelResponse: 0x{:08X}", hr.0));
        }

        let vtbl = unsafe { *(response_ptr as *const *const ILanguageModelResponseVtbl) };

        let mut text_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).get_text)(response_ptr as *mut _, &mut text_ptr) };

        if hr.is_err() {
            return Err(format!("Failed to get text: 0x{:08X}", hr.0));
        }

        // Convert HSTRING raw pointer to String using WindowsGetStringRawBuffer
        if text_ptr.is_null() {
            return Ok(String::new());
        }

        // HSTRING is a handle to a string buffer - use WindowsGetStringRawBuffer to get the data
        use windows::Win32::System::WinRT::{WindowsGetStringRawBuffer, WindowsDeleteString};

        let mut length: u32 = 0;
        let buffer = unsafe {
            WindowsGetStringRawBuffer(std::mem::transmute(text_ptr), Some(&mut length))
        };

        let result = if buffer.is_null() || length == 0 {
            String::new()
        } else {
            let slice = unsafe { std::slice::from_raw_parts(buffer.0, length as usize) };
            String::from_utf16_lossy(slice)
        };

        // Clean up the HSTRING
        unsafe {
            let _ = WindowsDeleteString(std::mem::transmute(text_ptr));
        }

        Ok(result)
    }

    /// Get the response status
    pub fn status(&self) -> std::result::Result<LanguageModelResponseStatus, String> {
        // Query for ILanguageModelResponse interface
        let inspectable: IInspectable = unsafe { IInspectable::from_raw(self.inner) };

        let mut response_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            inspectable.query(&IID_ILANGUAGEMODELRESPONSE, &mut response_ptr)
        };

        // Don't drop inspectable - we're borrowing the raw pointer
        std::mem::forget(inspectable);

        if hr.is_err() {
            return Err(format!("Failed to query ILanguageModelResponse: 0x{:08X}", hr.0));
        }

        let vtbl = unsafe { *(response_ptr as *const *const ILanguageModelResponseVtbl) };

        let mut status: i32 = 0;
        let hr = unsafe { ((*vtbl).get_status)(response_ptr as *mut _, &mut status) };

        if hr.is_err() {
            return Err(format!("Failed to get status: 0x{:08X}", hr.0));
        }

        Ok(LanguageModelResponseStatus::from_i32(status))
    }
}
