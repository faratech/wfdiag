//! Minimal WinRT bindings for `Windows.ApplicationModel.LimitedAccessFeatures`
//! using `windows-core 0.100.0`.

#![allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    clippy::all,
    clippy::pedantic
)]

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LimitedAccessFeatureStatus(pub i32);
impl LimitedAccessFeatureStatus {
    pub const Unavailable: Self = Self(0);
    pub const Available: Self = Self(1);
    pub const AvailableWithoutToken: Self = Self(2);
    pub const Unknown: Self = Self(3);
}

windows_core::imp::define_interface!(
    ILimitedAccessFeatureRequestResult,
    ILimitedAccessFeatureRequestResult_Vtbl,
    0xd45156a6_1e24_5ddd_abb4_6188aba4d5bf
);

#[repr(C)]
#[doc(hidden)]
pub struct ILimitedAccessFeatureRequestResult_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub FeatureId: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub Status: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut LimitedAccessFeatureStatus,
    ) -> windows_core::HRESULT,
    pub EstimatedRemovalDate: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}

windows_core::imp::define_interface!(
    ILimitedAccessFeaturesStatics,
    ILimitedAccessFeaturesStatics_Vtbl,
    0x8be612d4_302b_5fbf_a632_1a99e43e8925
);

#[repr(C)]
#[doc(hidden)]
pub struct ILimitedAccessFeaturesStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub TryUnlockFeature: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}

#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LimitedAccessFeatureRequestResult(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    LimitedAccessFeatureRequestResult,
    windows_core::IUnknown,
    windows_core::IInspectable
);

impl LimitedAccessFeatureRequestResult {
    pub fn Status(&self) -> windows_core::Result<LimitedAccessFeatureStatus> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Status)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}

unsafe impl windows_core::Interface for LimitedAccessFeatureRequestResult {
    type Vtable = <ILimitedAccessFeatureRequestResult as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID =
        <ILimitedAccessFeatureRequestResult as windows_core::Interface>::IID;
}

pub struct LimitedAccessFeatures;
impl LimitedAccessFeatures {
    pub fn TryUnlockFeature(
        featureid: &windows_core::HSTRING,
        token: &windows_core::HSTRING,
        attestation: &windows_core::HSTRING,
    ) -> windows_core::Result<LimitedAccessFeatureRequestResult> {
        Self::ILimitedAccessFeaturesStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).TryUnlockFeature)(
                windows_core::Interface::as_raw(this),
                core::mem::transmute_copy(featureid),
                core::mem::transmute_copy(token),
                core::mem::transmute_copy(attestation),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }

    fn ILimitedAccessFeaturesStatics<
        R,
        F: FnOnce(&ILimitedAccessFeaturesStatics) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            LimitedAccessFeatures,
            ILimitedAccessFeaturesStatics,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}

impl windows_core::RuntimeName for LimitedAccessFeatures {
    const NAME: &'static str = "Windows.ApplicationModel.LimitedAccessFeatures";
}
