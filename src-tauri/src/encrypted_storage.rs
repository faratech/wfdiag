use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

#[cfg(windows)]
use windows::Win32::Foundation::{HLOCAL, LocalFree};

/// Encrypted file header containing metadata
const VERSION: u8 = 2; // Bumped version for DPAPI format

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedHeader {
    version: u8,
}

/// Secure storage using Windows DPAPI
pub struct EncryptedStorage {
    storage_path: PathBuf,
}

impl EncryptedStorage {
    /// Create new encrypted storage with DPAPI
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        Ok(Self { storage_path })
    }

    /// Encrypt data using Windows DPAPI
    #[cfg(windows)]
    fn encrypt_data(data: &[u8]) -> Result<Vec<u8>> {
        use std::ptr::null_mut;

        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };

        let mut output_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        // Use DPAPI to encrypt - data is protected for current user
        let result = unsafe {
            CryptProtectData(
                &input_blob,
                None,                      // Description (optional)
                None,                      // Optional entropy (we don't need it)
                None,                      // Reserved
                None,                      // Prompt struct
                CRYPTPROTECT_UI_FORBIDDEN, // No UI
                &mut output_blob,
            )
        };

        if result.is_err() {
            return Err(anyhow!("DPAPI encryption failed"));
        }

        // Copy encrypted data and free the Windows-allocated buffer
        let encrypted = unsafe {
            let slice = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
            let result = slice.to_vec();
            let _ = LocalFree(Some(HLOCAL(output_blob.pbData as *mut _)));
            result
        };

        Ok(encrypted)
    }

    /// Decrypt data using Windows DPAPI
    #[cfg(windows)]
    fn decrypt_data(encrypted: &[u8]) -> Result<Vec<u8>> {
        use std::ptr::null_mut;

        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };

        let mut output_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        let result = unsafe {
            CryptUnprotectData(
                &input_blob,
                None, // Description output
                None, // Optional entropy
                None, // Reserved
                None, // Prompt struct
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output_blob,
            )
        };

        if result.is_err() {
            return Err(anyhow!("DPAPI decryption failed"));
        }

        // Copy decrypted data and free the Windows-allocated buffer
        let decrypted = unsafe {
            let slice = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
            let result = slice.to_vec();
            let _ = LocalFree(Some(HLOCAL(output_blob.pbData as *mut _)));
            result
        };

        Ok(decrypted)
    }

    /// Encrypt and store data
    pub fn store<T: Serialize>(&self, filename: &str, data: &T) -> Result<()> {
        // Serialize data to JSON
        let json_data =
            serde_json::to_vec(data).map_err(|e| anyhow!("Failed to serialize data: {}", e))?;

        // Encrypt using DPAPI
        #[cfg(windows)]
        let encrypted_data = Self::encrypt_data(&json_data)?;

        #[cfg(not(windows))]
        let encrypted_data = json_data; // No encryption on non-Windows (for testing)

        // Create header
        let header = EncryptedHeader { version: VERSION };
        let header_json = serde_json::to_vec(&header)
            .map_err(|e| anyhow!("Failed to serialize header: {}", e))?;

        // Combine header and encrypted data
        let header_len = header_json.len() as u32;
        let mut file_data = Vec::new();
        file_data.extend(&header_len.to_le_bytes());
        file_data.extend(header_json);
        file_data.extend(encrypted_data);

        // Write to file atomically to avoid corruption on crashes
        let file_path = self.storage_path.join(format!("{}.enc", filename));
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(|e| anyhow!("Failed to create directory: {}", e))?;
        }

        let temp_path = file_path.with_extension("enc.tmp");
        {
            let mut temp_file = fs::File::create(&temp_path)
                .map_err(|e| anyhow!("Failed to create temp file: {}", e))?;
            temp_file
                .write_all(&file_data)
                .map_err(|e| anyhow!("Failed to write encrypted file: {}", e))?;
            temp_file
                .sync_all()
                .map_err(|e| anyhow!("Failed to flush encrypted file: {}", e))?;
        }

        fs::rename(&temp_path, &file_path)
            .map_err(|e| anyhow!("Failed to finalize encrypted file: {}", e))?;

        Ok(())
    }

    /// Load and decrypt data
    pub fn load<T: for<'de> Deserialize<'de>>(&self, filename: &str) -> Result<T> {
        let file_path = self.storage_path.join(format!("{}.enc", filename));

        let file_data =
            fs::read(&file_path).map_err(|e| anyhow!("Failed to read encrypted file: {}", e))?;

        if file_data.len() < 4 {
            return Err(anyhow!("Invalid encrypted file format"));
        }

        // Read header length
        let header_len =
            u32::from_le_bytes([file_data[0], file_data[1], file_data[2], file_data[3]]) as usize;

        if file_data.len() < 4 + header_len {
            return Err(anyhow!("Invalid encrypted file format"));
        }

        // Parse header
        let header_data = &file_data[4..4 + header_len];
        let header: EncryptedHeader = serde_json::from_slice(header_data)
            .map_err(|e| anyhow!("Failed to parse header: {}", e))?;

        // Support both old (v1) and new (v2) formats during migration
        if header.version != VERSION && header.version != 1 {
            return Err(anyhow!(
                "Unsupported encryption version: {}",
                header.version
            ));
        }

        // Extract encrypted data
        let encrypted_data = &file_data[4 + header_len..];

        // Handle version migration
        let decrypted_data = if header.version == 1 {
            // Old format - try to decrypt with legacy method or return error
            return Err(anyhow!(
                "Legacy v1 encrypted files need re-encryption. Please clear scan history and re-run diagnostics."
            ));
        } else {
            // New DPAPI format
            #[cfg(windows)]
            {
                Self::decrypt_data(encrypted_data)?
            }
            #[cfg(not(windows))]
            {
                encrypted_data.to_vec()
            }
        };

        // Deserialize
        let result: T = serde_json::from_slice(&decrypted_data)
            .map_err(|e| anyhow!("Failed to deserialize data: {}", e))?;

        Ok(result)
    }

    /// Check if encrypted file exists
    pub fn exists(&self, filename: &str) -> bool {
        let file_path = self.storage_path.join(format!("{}.enc", filename));
        file_path.exists()
    }

    /// Delete encrypted file
    pub fn delete(&self, filename: &str) -> Result<()> {
        let file_path = self.storage_path.join(format!("{}.enc", filename));
        if file_path.exists() {
            fs::remove_file(file_path)
                .map_err(|e| anyhow!("Failed to delete encrypted file: {}", e))?;
        }
        Ok(())
    }

    /// List all encrypted files (without .enc extension)
    pub fn list_files(&self) -> Result<Vec<String>> {
        if !self.storage_path.exists() {
            return Ok(Vec::new());
        }

        let entries = fs::read_dir(&self.storage_path)
            .map_err(|e| anyhow!("Failed to read storage directory: {}", e))?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| anyhow!("Failed to read directory entry: {}", e))?;
            let path = entry.path();

            if let Some(extension) = path.extension()
                && extension == "enc"
                && let Some(stem) = path.file_stem()
                && let Some(name) = stem.to_str()
            {
                files.push(name.to_string());
            }
        }

        files.sort();
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestData {
        id: String,
        values: HashMap<String, String>,
        sensitive_info: String,
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let temp_dir = std::env::temp_dir().join("wfdiag_test_dpapi");
        let storage = EncryptedStorage::new(temp_dir.clone()).expect("Failed to create storage");

        let test_data = TestData {
            id: "test_123".to_string(),
            values: {
                let mut map = HashMap::new();
                map.insert("key1".to_string(), "value1".to_string());
                map.insert("key2".to_string(), "sensitive_value".to_string());
                map
            },
            sensitive_info: "This is sensitive diagnostic data".to_string(),
        };

        // Store encrypted data
        storage
            .store("test_data", &test_data)
            .expect("Failed to store data");

        // Load and decrypt data
        let loaded_data: TestData = storage.load("test_data").expect("Failed to load data");

        // Verify data integrity
        assert_eq!(test_data, loaded_data);

        // Cleanup
        let _ = fs::remove_dir_all(temp_dir);
    }
}
