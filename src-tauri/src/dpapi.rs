//! Windows DPAPI (Data Protection API) for secure credential storage
//!
//! This module provides secure encryption/decryption of sensitive data
//! using Windows CryptProtectData/CryptUnprotectData APIs.
//! Data is encrypted with the current user's credentials and can only
//! be decrypted by the same user on the same machine.

#[cfg(windows)]
use windows::{
    Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
    core::PCWSTR,
};

use std::fs;
use std::path::PathBuf;

use crate::error::DiagError;

/// Closed set of providers whose API keys we store. Filenames and keyring
/// entry names derive from this enum only — caller strings are never
/// interpolated into paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKeyId {
    OpenAI,
    Anthropic,
    Gemini,
    DeepSeek,
    Custom,
}

impl ProviderKeyId {
    /// Parse a frontend-supplied provider id. Unknown ids are an error, not a
    /// default — this keeps the filename set closed.
    pub fn parse(provider: &str) -> Result<Self, String> {
        match provider {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "deepseek" => Ok(Self::DeepSeek),
            "custom_openai" | "custom" => Ok(Self::Custom),
            other => Err(DiagError::api_key(
                "store",
                format!("Unknown API key provider '{}'", other),
            )
            .into()),
        }
    }

    /// Encrypted credentials filename. OpenAI keeps the legacy name so
    /// existing stored keys work without migration.
    fn filename(self) -> &'static str {
        match self {
            Self::OpenAI => "credentials.bin",
            Self::Anthropic => "credentials_anthropic.bin",
            Self::Gemini => "credentials_gemini.bin",
            Self::DeepSeek => "credentials_deepseek.bin",
            Self::Custom => "credentials_custom.bin",
        }
    }

    /// Keyring entry name (non-Windows dev builds). OpenAI keeps the legacy
    /// entry name for the same no-migration reason.
    #[cfg(not(windows))]
    pub fn keyring_user(self) -> &'static str {
        match self {
            Self::OpenAI => "openai_api_key",
            Self::Anthropic => "anthropic_api_key",
            Self::Gemini => "gemini_api_key",
            Self::DeepSeek => "deepseek_api_key",
            Self::Custom => "custom_api_key",
        }
    }
}

/// Get the path to the encrypted credentials file for a provider
fn get_credentials_path(id: ProviderKeyId) -> Result<PathBuf, String> {
    let app_data = dirs::data_local_dir()
        .ok_or_else(|| DiagError::internal("Could not find local app data directory"))?;
    let creds_dir = app_data.join("WFDiag");

    // Create directory if it doesn't exist
    if !creds_dir.exists() {
        fs::create_dir_all(&creds_dir)
            .map_err(|e| DiagError::file(creds_dir.display().to_string(), e.to_string()))?;
    }

    Ok(creds_dir.join(id.filename()))
}

// FFI for LocalFree - simpler than pulling in another windows crate feature
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

/// Encrypt a string using Windows DPAPI
#[cfg(windows)]
fn dpapi_encrypt(data: &str) -> Result<Vec<u8>, String> {
    use std::ptr::null_mut;

    // Create mutable copy to avoid undefined behavior from const-to-mut cast
    let mut data_bytes = data.as_bytes().to_vec();
    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data_bytes.len() as u32,
        pbData: data_bytes.as_mut_ptr(),
    };

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    // Encrypt with DPAPI - UI_FORBIDDEN prevents prompts in background
    let result = unsafe {
        CryptProtectData(
            &data_in,
            PCWSTR::null(),            // No description
            None,                      // No additional entropy
            None,                      // Reserved
            None,                      // No prompt struct
            CRYPTPROTECT_UI_FORBIDDEN, // Flags
            &mut data_out,
        )
    };

    if result.is_err() {
        return Err(DiagError::api_key(
            "encrypt",
            format!("DPAPI encryption failed: {:?}", result),
        )
        .into());
    }

    // Copy encrypted data to Vec
    let encrypted =
        unsafe { std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize).to_vec() };

    // Free the memory allocated by CryptProtectData
    unsafe {
        LocalFree(data_out.pbData as *mut std::ffi::c_void);
    }

    Ok(encrypted)
}

/// Decrypt data using Windows DPAPI
#[cfg(windows)]
fn dpapi_decrypt(encrypted: &[u8]) -> Result<String, String> {
    use std::ptr::null_mut;

    // Create mutable copy to avoid undefined behavior from const-to-mut cast
    let mut encrypted_copy = encrypted.to_vec();
    let data_in = CRYPT_INTEGER_BLOB {
        cbData: encrypted_copy.len() as u32,
        pbData: encrypted_copy.as_mut_ptr(),
    };

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    let result = unsafe {
        CryptUnprotectData(
            &data_in,
            None, // Don't need description
            None, // No additional entropy
            None, // Reserved
            None, // No prompt struct
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
    };

    if result.is_err() {
        return Err(DiagError::api_key(
            "decrypt",
            format!("DPAPI decryption failed: {:?}", result),
        )
        .into());
    }

    // Convert decrypted bytes to string
    let decrypted = unsafe {
        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        String::from_utf8_lossy(slice).to_string()
    };

    // Free the memory allocated by CryptUnprotectData
    unsafe {
        LocalFree(data_out.pbData as *mut std::ffi::c_void);
    }

    Ok(decrypted)
}

// Non-Windows stubs for cross-compilation
#[cfg(not(windows))]
fn dpapi_encrypt(_data: &str) -> Result<Vec<u8>, String> {
    Err(DiagError::PlatformNotSupported {
        operation: "DPAPI encryption".to_string(),
    }
    .into())
}

#[cfg(not(windows))]
fn dpapi_decrypt(_encrypted: &[u8]) -> Result<String, String> {
    Err(DiagError::PlatformNotSupported {
        operation: "DPAPI decryption".to_string(),
    }
    .into())
}

/// Store a provider's API key securely using DPAPI
pub fn store_provider_key(id: ProviderKeyId, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return clear_provider_key(id);
    }

    let encrypted = dpapi_encrypt(key)?;
    let path = get_credentials_path(id)?;

    fs::write(&path, encrypted)
        .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;

    println!("API key stored securely with DPAPI at {:?}", path);
    Ok(())
}

/// Load a provider's API key from secure DPAPI storage
pub fn load_provider_key(id: ProviderKeyId) -> Result<Option<String>, String> {
    let path = get_credentials_path(id)?;

    if !path.exists() {
        return Ok(None);
    }

    let encrypted =
        fs::read(&path).map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;

    if encrypted.is_empty() {
        return Ok(None);
    }

    match dpapi_decrypt(&encrypted) {
        Ok(key) => {
            if key.is_empty() {
                Ok(None)
            } else {
                Ok(Some(key))
            }
        }
        Err(e) => {
            // If decryption fails, the file might be corrupted or from a different user
            eprintln!("Warning: Failed to decrypt credentials: {}", e);
            Ok(None)
        }
    }
}

/// Clear a provider's stored API key
pub fn clear_provider_key(id: ProviderKeyId) -> Result<(), String> {
    let path = get_credentials_path(id)?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;
        println!("API key cleared from DPAPI storage");
    }

    Ok(())
}

/// Clear the stored OpenAI API key (legacy single-key entry point)
pub fn clear_api_key() -> Result<(), String> {
    clear_provider_key(ProviderKeyId::OpenAI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_dpapi_roundtrip() {
        let original = "sk-test-key-12345";
        let encrypted = dpapi_encrypt(original).expect("Encryption should succeed");
        assert!(!encrypted.is_empty());
        assert_ne!(encrypted, original.as_bytes()); // Should be different

        let decrypted = dpapi_decrypt(&encrypted).expect("Decryption should succeed");
        assert_eq!(decrypted, original);
    }

    #[test]
    fn openai_keeps_legacy_credentials_filename() {
        // Existing installs must keep finding their stored OpenAI key — the
        // legacy filename is load-bearing, the others must not collide.
        assert_eq!(ProviderKeyId::OpenAI.filename(), "credentials.bin");
        assert_eq!(
            ProviderKeyId::Anthropic.filename(),
            "credentials_anthropic.bin"
        );
        assert_eq!(ProviderKeyId::Gemini.filename(), "credentials_gemini.bin");
        assert_eq!(
            ProviderKeyId::DeepSeek.filename(),
            "credentials_deepseek.bin"
        );
        assert_eq!(ProviderKeyId::Custom.filename(), "credentials_custom.bin");
    }

    #[test]
    fn provider_key_id_parse_is_a_closed_set() {
        assert_eq!(ProviderKeyId::parse("openai"), Ok(ProviderKeyId::OpenAI));
        assert_eq!(
            ProviderKeyId::parse("anthropic"),
            Ok(ProviderKeyId::Anthropic)
        );
        assert_eq!(ProviderKeyId::parse("gemini"), Ok(ProviderKeyId::Gemini));
        assert_eq!(
            ProviderKeyId::parse("deepseek"),
            Ok(ProviderKeyId::DeepSeek)
        );
        assert_eq!(
            ProviderKeyId::parse("custom_openai"),
            Ok(ProviderKeyId::Custom)
        );
        assert_eq!(ProviderKeyId::parse("custom"), Ok(ProviderKeyId::Custom));
        // Providers without their own key, and garbage, must be rejected
        assert!(ProviderKeyId::parse("phi_silica").is_err());
        assert!(ProviderKeyId::parse("../evil").is_err());
        assert!(ProviderKeyId::parse("").is_err());
    }
}
