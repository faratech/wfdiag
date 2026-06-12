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

/// Get the path to the encrypted credentials file
fn get_credentials_path() -> Result<PathBuf, String> {
    let app_data = dirs::data_local_dir()
        .ok_or_else(|| DiagError::internal("Could not find local app data directory"))?;
    let creds_dir = app_data.join("WFDiag");

    // Create directory if it doesn't exist
    if !creds_dir.exists() {
        fs::create_dir_all(&creds_dir)
            .map_err(|e| DiagError::file(creds_dir.display().to_string(), e.to_string()))?;
    }

    Ok(creds_dir.join("credentials.bin"))
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

/// Store the API key securely using DPAPI
pub fn store_api_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return clear_api_key();
    }

    let encrypted = dpapi_encrypt(key)?;
    let path = get_credentials_path()?;

    fs::write(&path, encrypted)
        .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;

    println!("API key stored securely with DPAPI at {:?}", path);
    Ok(())
}

/// Load the API key from secure DPAPI storage
pub fn load_api_key() -> Result<Option<String>, String> {
    let path = get_credentials_path()?;

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

/// Clear the stored API key
pub fn clear_api_key() -> Result<(), String> {
    let path = get_credentials_path()?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;
        println!("API key cleared from DPAPI storage");
    }

    Ok(())
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
}
