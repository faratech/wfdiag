//! Package identity detection.
//!
//! The Windows AI APIs (Phi Silica) gate on registered package identity +
//! the `systemAIModels` capability. Testing proved this is enforced at the
//! API level: an unpackaged process is denied (0x80070005) even when
//! activating the bundled DLLs directly, so there is no identity-free path.
//! Phi Silica is therefore Store-only; loose builds route AI to Foundry
//! Local or OpenAI instead.
//!
//! For development, identity can still be attached to a loose exe by
//! registering the sparse "package with external location" produced by
//! `build-cross.py build-sparse` (run `Install-SparseIdentity.ps1`; the
//! exe's embedded manifest carries the matching `<msix>` element).

#![cfg(windows)]

/// True when the current process runs with package identity.
pub fn has_package_identity() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    let mut length: u32 = 0;
    // With no buffer: ERROR_INSUFFICIENT_BUFFER means "there IS a package
    // name" (we only asked for its length); APPMODEL_ERROR_NO_PACKAGE (15700)
    // means the process is unpackaged.
    let result = unsafe { GetCurrentPackageFullName(&mut length, None) };
    result == ERROR_INSUFFICIENT_BUFFER
}
