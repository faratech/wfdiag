use aes_gcm::{
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Result};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::PathBuf;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secure storage configuration
const PBKDF2_ITERATIONS: u32 = 100_000;
const KEY_LENGTH: usize = 32; // AES-256
const SALT_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12; // AES-GCM standard
const VERSION: u8 = 1;

/// Encrypted file header containing metadata
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedHeader {
    version: u8,
    salt: Vec<u8>,
    nonce: Vec<u8>,
}

/// Secure key derivation and storage
pub struct EncryptedStorage {
    storage_path: PathBuf,
    master_key: [u8; KEY_LENGTH],
}

impl EncryptedStorage {
    /// Create new encrypted storage with machine-specific key derivation
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        let master_key = Self::derive_master_key()?;
        
        Ok(Self {
            storage_path,
            master_key,
        })
    }

    /// Derive master key from machine and user context
    fn derive_master_key() -> Result<[u8; KEY_LENGTH]> {
        // Collect machine-specific entropy sources
        let mut context = Vec::new();
        
        // Machine ID (varies by OS)
        #[cfg(windows)]
        {
            if let Ok(machine_guid) = Self::get_windows_machine_guid() {
                context.extend(machine_guid.as_bytes());
            }
        }
        
        #[cfg(not(windows))]
        {
            // For non-Windows, use hostname and user
            if let Ok(hostname) = std::env::var("HOSTNAME") {
                context.extend(hostname.as_bytes());
            }
        }
        
        // User context
        if let Ok(username) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
            context.extend(username.as_bytes());
        }
        
        // Application-specific identifier
        context.extend(b"WF_DIAGNOSTICS_V2_ENCRYPTION");
        
        // If we can't get enough entropy, use a fallback
        if context.len() < 32 {
            context.extend(b"DEFAULT_FALLBACK_ENTROPY_SOURCE_FOR_ENCRYPTION");
        }
        
        // Derive key using PBKDF2 with the collected context
        let mut master_key = [0u8; KEY_LENGTH];
        pbkdf2_hmac::<Sha256>(&context, b"wfdiag_salt_v1", PBKDF2_ITERATIONS, &mut master_key);
        
        Ok(master_key)
    }

    #[cfg(windows)]
    fn get_windows_machine_guid() -> Result<String> {
        use std::process::Command;
        
        // Get Windows machine GUID from registry
        let output = Command::new("reg")
            .args(&["query", "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography", "/v", "MachineGuid"])
            .output()
            .map_err(|e| anyhow!("Failed to query machine GUID: {}", e))?;
            
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            for line in output_str.lines() {
                if line.contains("MachineGuid") && line.contains("REG_SZ") {
                    if let Some(guid) = line.split_whitespace().last() {
                        return Ok(guid.to_string());
                    }
                }
            }
        }
        
        Err(anyhow!("Could not retrieve machine GUID"))
    }

    /// Encrypt and store data
    pub fn store<T: Serialize>(&self, filename: &str, data: &T) -> Result<()> {
        // Serialize data to JSON
        let json_data = serde_json::to_vec(data)
            .map_err(|e| anyhow!("Failed to serialize data: {}", e))?;

        // Generate random salt and nonce
        let mut salt = vec![0u8; SALT_LENGTH];
        let mut nonce_bytes = vec![0u8; NONCE_LENGTH];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce_bytes);

        // Derive encryption key from master key + salt
        let mut encryption_key = [0u8; KEY_LENGTH];
        pbkdf2_hmac::<Sha256>(&self.master_key, &salt, 10_000, &mut encryption_key);

        // Create cipher and encrypt
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&encryption_key));
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let encrypted_data = cipher.encrypt(nonce, json_data.as_ref())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        // Create header
        let header = EncryptedHeader {
            version: VERSION,
            salt,
            nonce: nonce_bytes,
        };

        // Combine header and encrypted data
        let header_json = serde_json::to_vec(&header)
            .map_err(|e| anyhow!("Failed to serialize header: {}", e))?;
        
        let header_len = header_json.len() as u32;
        let mut file_data = Vec::new();
        file_data.extend(&header_len.to_le_bytes());
        file_data.extend(header_json);
        file_data.extend(encrypted_data);

        // Write to file
        let file_path = self.storage_path.join(format!("{}.enc", filename));
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow!("Failed to create directory: {}", e))?;
        }
        
        fs::write(file_path, file_data)
            .map_err(|e| anyhow!("Failed to write encrypted file: {}", e))?;

        // Zero out sensitive data
        let mut encryption_key = encryption_key;
        encryption_key.zeroize();

        Ok(())
    }

    /// Load and decrypt data
    pub fn load<T: for<'de> Deserialize<'de>>(&self, filename: &str) -> Result<T> {
        let file_path = self.storage_path.join(format!("{}.enc", filename));
        
        let file_data = fs::read(file_path)
            .map_err(|e| anyhow!("Failed to read encrypted file: {}", e))?;

        if file_data.len() < 4 {
            return Err(anyhow!("Invalid encrypted file format"));
        }

        // Read header length
        let header_len = u32::from_le_bytes([
            file_data[0], file_data[1], file_data[2], file_data[3]
        ]) as usize;

        if file_data.len() < 4 + header_len {
            return Err(anyhow!("Invalid encrypted file format"));
        }

        // Parse header
        let header_data = &file_data[4..4 + header_len];
        let header: EncryptedHeader = serde_json::from_slice(header_data)
            .map_err(|e| anyhow!("Failed to parse header: {}", e))?;

        if header.version != VERSION {
            return Err(anyhow!("Unsupported encryption version: {}", header.version));
        }

        // Extract encrypted data
        let encrypted_data = &file_data[4 + header_len..];

        // Derive decryption key
        let mut decryption_key = [0u8; KEY_LENGTH];
        pbkdf2_hmac::<Sha256>(&self.master_key, &header.salt, 10_000, &mut decryption_key);

        // Decrypt
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&decryption_key));
        let nonce = Nonce::from_slice(&header.nonce);
        
        let decrypted_data = cipher.decrypt(nonce, encrypted_data)
            .map_err(|e| anyhow!("Decryption failed: {}", e))?;

        // Deserialize
        let result: T = serde_json::from_slice(&decrypted_data)
            .map_err(|e| anyhow!("Failed to deserialize data: {}", e))?;

        // Zero out sensitive data
        let mut decryption_key = decryption_key;
        decryption_key.zeroize();

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
            
            if let Some(extension) = path.extension() {
                if extension == "enc" {
                    if let Some(stem) = path.file_stem() {
                        if let Some(name) = stem.to_str() {
                            files.push(name.to_string());
                        }
                    }
                }
            }
        }

        files.sort();
        Ok(files)
    }
}

impl Drop for EncryptedStorage {
    fn drop(&mut self) {
        // Ensure master key is zeroed when dropped
        self.master_key.zeroize();
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
        let temp_dir = std::env::temp_dir().join("wfdiag_test_encryption");
        let storage = EncryptedStorage::new(temp_dir.clone()).expect("Failed to create storage");

        let test_data = TestData {
            id: "test_123".to_string(),
            values: {
                let mut map = HashMap::new();
                map.insert("key1".to_string(), "value1".to_string());
                map.insert("key2".to_string(), "sensitive_value".to_string());
                map
            },
            sensitive_info: "This is sensitive diagnostic data that should be encrypted".to_string(),
        };

        // Store encrypted data
        storage.store("test_data", &test_data).expect("Failed to store data");

        // Load and decrypt data
        let loaded_data: TestData = storage.load("test_data").expect("Failed to load data");

        // Verify data integrity
        assert_eq!(test_data, loaded_data);

        // Cleanup
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_key_derivation_consistency() {
        // Test that the same machine context produces the same key
        let key1 = EncryptedStorage::derive_master_key().expect("Failed to derive key 1");
        let key2 = EncryptedStorage::derive_master_key().expect("Failed to derive key 2");
        
        assert_eq!(key1, key2, "Key derivation should be deterministic for the same machine");
    }
}