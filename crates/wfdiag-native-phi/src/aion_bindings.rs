windows_core::imp::define_interface!(
    IClosable,
    IClosable_Vtbl,
    0x30d5a829_7fa4_4026_83bb_d75bae4ea99e
);
impl windows_core::RuntimeType for IClosable {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Foundation.IClosable");
}
windows_core::imp::interface_hierarchy!(
    IClosable,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl IClosable {
    pub fn Close(&self) -> windows_core::Result<()> {
        unsafe {
            (windows_core::Interface::vtable(self).Close)(windows_core::Interface::as_raw(self))
                .ok()
        }
    }
}
impl windows_core::RuntimeName for IClosable {
    const NAME: &'static str = "Windows.Foundation.IClosable";
}
pub trait IClosable_Impl: windows_core::IUnknownImpl {
    fn Close(&self) -> windows_core::Result<()>;
}
impl IClosable_Vtbl {
    pub const fn new<Identity: IClosable_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn Close<Identity: IClosable_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IClosable_Impl::Close(this).into()
            }
        }
        Self {
            base__: windows_core::IInspectable_Vtbl::new::<Identity, IClosable, OFFSET>(),
            Close: Close::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IClosable as windows_core::Interface>::IID
    }
}
#[repr(C)]
pub struct IClosable_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub Close: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ILanguageModel,
    ILanguageModel_Vtbl,
    0x01216f3c_4cee_5f00_aedc_c705ed94c10e
);
impl windows_core::RuntimeType for ILanguageModel {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"AionInstructPreview.Text.ILanguageModel");
}
#[repr(C)]
pub struct ILanguageModel_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub GenerateResponseAsync: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub CreateContext: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub GenerateResponseAsync2: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ILanguageModelContext,
    ILanguageModelContext_Vtbl,
    0x9c859b85_61a8_5d80_bddc_32006505f7a9
);
impl windows_core::RuntimeType for ILanguageModelContext {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"AionInstructPreview.Text.ILanguageModelContext",
    );
}
#[repr(C)]
pub struct ILanguageModelContext_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    ILanguageModelResponseResult,
    ILanguageModelResponseResult_Vtbl,
    0x66200df5_2223_51eb_b616_d6279ec0b8c2
);
impl windows_core::RuntimeType for ILanguageModelResponseResult {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"AionInstructPreview.Text.ILanguageModelResponseResult",
    );
}
#[repr(C)]
pub struct ILanguageModelResponseResult_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub Text: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub Status: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut LanguageModelResponseStatus,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ILanguageModelStatics,
    ILanguageModelStatics_Vtbl,
    0xf37c8314_9118_5036_8f18_4a071bf9103d
);
impl windows_core::RuntimeType for ILanguageModelStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"AionInstructPreview.Text.ILanguageModelStatics",
    );
}
#[repr(C)]
pub struct ILanguageModelStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub CreateAsync: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageModel(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    LanguageModel,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(LanguageModel, IClosable);
impl LanguageModel {
    pub fn Close(&self) -> windows_core::Result<()> {
        let this = &windows_core::Interface::cast::<IClosable>(self)?;
        unsafe {
            (windows_core::Interface::vtable(this).Close)(windows_core::Interface::as_raw(this))
                .ok()
        }
    }
    pub fn GenerateResponseAsync(
        &self,
        prompt: &windows_core::HSTRING,
    ) -> windows_core::Result<
        windows_future::IAsyncOperationWithProgress<
            LanguageModelResponseResult,
            windows_core::HSTRING,
        >,
    > {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GenerateResponseAsync)(
                windows_core::Interface::as_raw(self),
                core::mem::transmute_copy(prompt),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn CreateContext(&self) -> windows_core::Result<LanguageModelContext> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).CreateContext)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn GenerateResponseAsync2<P0>(
        &self,
        context: P0,
        prompt: &windows_core::HSTRING,
    ) -> windows_core::Result<
        windows_future::IAsyncOperationWithProgress<
            LanguageModelResponseResult,
            windows_core::HSTRING,
        >,
    >
    where
        P0: windows_core::Param<LanguageModelContext>,
    {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GenerateResponseAsync2)(
                windows_core::Interface::as_raw(self),
                context.param().abi(),
                core::mem::transmute_copy(prompt),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn CreateAsync() -> windows_core::Result<windows_future::IAsyncOperation<Self>> {
        Self::ILanguageModelStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).CreateAsync)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn ILanguageModelStatics<R, F: FnOnce(&ILanguageModelStatics) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<LanguageModel, ILanguageModelStatics> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for LanguageModel {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, ILanguageModel>();
}
unsafe impl windows_core::Interface for LanguageModel {
    type Vtable = <ILanguageModel as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <ILanguageModel as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for LanguageModel {
    const NAME: &'static str = "AionInstructPreview.Text.LanguageModel";
}
unsafe impl Send for LanguageModel {}
unsafe impl Sync for LanguageModel {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageModelContext(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    LanguageModelContext,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(LanguageModelContext, IClosable);
impl LanguageModelContext {
    pub fn Close(&self) -> windows_core::Result<()> {
        let this = &windows_core::Interface::cast::<IClosable>(self)?;
        unsafe {
            (windows_core::Interface::vtable(this).Close)(windows_core::Interface::as_raw(this))
                .ok()
        }
    }
}
impl windows_core::RuntimeType for LanguageModelContext {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, ILanguageModelContext>();
}
unsafe impl windows_core::Interface for LanguageModelContext {
    type Vtable = <ILanguageModelContext as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <ILanguageModelContext as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for LanguageModelContext {
    const NAME: &'static str = "AionInstructPreview.Text.LanguageModelContext";
}
unsafe impl Send for LanguageModelContext {}
unsafe impl Sync for LanguageModelContext {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageModelResponseResult(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    LanguageModelResponseResult,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl LanguageModelResponseResult {
    pub fn Text(&self) -> windows_core::Result<windows_core::HSTRING> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Text)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
    pub fn Status(&self) -> windows_core::Result<LanguageModelResponseStatus> {
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
impl windows_core::RuntimeType for LanguageModelResponseResult {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, ILanguageModelResponseResult>();
}
unsafe impl windows_core::Interface for LanguageModelResponseResult {
    type Vtable = <ILanguageModelResponseResult as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <ILanguageModelResponseResult as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for LanguageModelResponseResult {
    const NAME: &'static str = "AionInstructPreview.Text.LanguageModelResponseResult";
}
unsafe impl Send for LanguageModelResponseResult {}
unsafe impl Sync for LanguageModelResponseResult {}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LanguageModelResponseStatus(pub i32);
impl LanguageModelResponseStatus {
    pub const Complete: Self = Self(0);
    pub const InProgress: Self = Self(1);
    pub const Error: Self = Self(2);
    pub const PromptLargerThanContext: Self = Self(3);
}
impl windows_core::imp::TypeKind for LanguageModelResponseStatus {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for LanguageModelResponseStatus {
    const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"enum(AionInstructPreview.Text.LanguageModelResponseStatus;i4)",
    );
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"AionInstructPreview.Text.LanguageModelResponseStatus",
    );
}
