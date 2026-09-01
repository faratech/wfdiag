use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[cfg(windows)]
use windows::Win32::Foundation::{HLOCAL, LocalFree};
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

/// The on-disk envelope version used by every shipping 2.x scan: the payload
/// after the header is current-user DPAPI ciphertext. Windows always reads and
/// writes this version, byte-for-byte as before.
const VERSION_DPAPI: u8 = 2;

/// Envelope version for the non-Windows development fallback, whose payload is
/// **plaintext** JSON because DPAPI does not exist there (#216). It is a
/// distinct version so a plaintext file can never be mistaken for a protected
/// one: Windows refuses to read it, and a non-Windows build refuses to read a
/// real DPAPI envelope instead of returning its ciphertext as "data".
const VERSION_PLAINTEXT_FALLBACK: u8 = 3;

/// The version this build writes.
#[cfg(windows)]
const VERSION: u8 = VERSION_DPAPI;
#[cfg(not(windows))]
const VERSION: u8 = VERSION_PLAINTEXT_FALLBACK;

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedHeader {
    version: u8,
}

/// Secure scan payload storage using current-user Windows DPAPI.
///
/// This is the single implementation used by both the Tauri compatibility
/// layer and the native history runtime. Its header and payload layout are
/// byte-for-byte compatible with existing `.enc` scan files.
pub struct EncryptedStorage {
    storage_path: PathBuf,
}

impl EncryptedStorage {
    /// Construct a store rooted at `storage_path`.
    ///
    /// # Errors
    ///
    /// Reserved for future backend initialization failures. The current
    /// implementation performs no I/O until the first operation.
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        Ok(Self { storage_path })
    }

    #[cfg(windows)]
    fn encrypt_data(data: &[u8]) -> Result<Vec<u8>> {
        use std::ptr::null_mut;

        let mut data_copy = data.to_vec();
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data_copy.len())
                .map_err(|_| anyhow!("DPAPI input is too large"))?,
            pbData: data_copy.as_mut_ptr(),
        };
        let mut output_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        unsafe {
            CryptProtectData(
                &raw const input_blob,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output_blob,
            )
        }
        .map_err(|_| anyhow!("DPAPI encryption failed"))?;

        let encrypted = unsafe {
            let slice = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
            let result = slice.to_vec();
            let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
            result
        };
        Ok(encrypted)
    }

    #[cfg(windows)]
    fn decrypt_data(encrypted: &[u8]) -> Result<Vec<u8>> {
        use std::ptr::null_mut;

        let mut encrypted_copy = encrypted.to_vec();
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(encrypted_copy.len())
                .map_err(|_| anyhow!("DPAPI payload is too large"))?,
            pbData: encrypted_copy.as_mut_ptr(),
        };
        let mut output_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        unsafe {
            CryptUnprotectData(
                &raw const input_blob,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output_blob,
            )
        }
        .map_err(|_| anyhow!("DPAPI decryption failed"))?;

        let decrypted = unsafe {
            let slice = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
            let result = slice.to_vec();
            let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
            result
        };
        Ok(decrypted)
    }

    /// Explain why an envelope this build cannot interpret was refused (#216).
    fn wrong_envelope_message(version: u8) -> String {
        match version {
            VERSION_DPAPI => "This scan file is protected with Windows DPAPI (envelope version 2) and can only be read on the Windows build that wrote it.".to_string(),
            VERSION_PLAINTEXT_FALLBACK => "This scan file is an unprotected non-Windows development envelope (version 3) and is refused instead of being treated as DPAPI-protected data.".to_string(),
            other => format!("Unsupported encryption version: {other}"),
        }
    }

    fn validate_filename(filename: &str) -> Result<()> {
        let valid = !filename.is_empty()
            && !filename.contains("..")
            && filename.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            });
        if !valid {
            return Err(anyhow!(
                "Invalid storage id '{filename}': only [A-Za-z0-9._-] are allowed and '..' is forbidden"
            ));
        }
        Ok(())
    }

    /// Serialize, DPAPI-protect, and atomically replace one payload.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid ID, serialization/encryption failure,
    /// or filesystem failure.
    pub fn store<T: Serialize>(&self, filename: &str, data: &T) -> Result<()> {
        Self::validate_filename(filename)?;
        let json_data = serde_json::to_vec(data)
            .map_err(|error| anyhow!("Failed to serialize data: {error}"))?;

        #[cfg(windows)]
        let encrypted_data = Self::encrypt_data(&json_data)?;
        #[cfg(not(windows))]
        let encrypted_data = json_data;

        let header_json = serde_json::to_vec(&EncryptedHeader { version: VERSION })
            .map_err(|error| anyhow!("Failed to serialize header: {error}"))?;
        let mut file_data = Vec::with_capacity(4 + header_json.len() + encrypted_data.len());
        let header_len = u32::try_from(header_json.len())
            .map_err(|_| anyhow!("Encrypted storage header is too large"))?;
        file_data.extend(header_len.to_le_bytes());
        file_data.extend(header_json);
        file_data.extend(encrypted_data);

        let file_path = self.storage_path.join(format!("{filename}.enc"));
        crate::fs_atomic::write_file(&file_path, &file_data)
            .map_err(|error| anyhow!("Failed to write encrypted file: {error}"))
    }

    /// Load, decrypt, and deserialize one payload.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid ID, missing/corrupt/unsupported data,
    /// DPAPI failure, or deserialization failure.
    pub fn load<T: for<'de> Deserialize<'de>>(&self, filename: &str) -> Result<T> {
        Self::validate_filename(filename)?;
        let file_path = self.storage_path.join(format!("{filename}.enc"));
        let file_data = fs::read(file_path)
            .map_err(|error| anyhow!("Failed to read encrypted file: {error}"))?;
        if file_data.len() < 4 {
            return Err(anyhow!("Invalid encrypted file format"));
        }

        let header_len =
            u32::from_le_bytes([file_data[0], file_data[1], file_data[2], file_data[3]]) as usize;
        if file_data.len() < 4 + header_len {
            return Err(anyhow!("Invalid encrypted file format"));
        }
        let header: EncryptedHeader = serde_json::from_slice(&file_data[4..4 + header_len])
            .map_err(|error| anyhow!("Failed to parse header: {error}"))?;
        if header.version == 1 {
            return Err(anyhow!(
                "Legacy v1 encrypted files need re-encryption. Please clear scan history and re-run diagnostics."
            ));
        }
        // #216: the two envelope versions carry different payloads (DPAPI
        // ciphertext vs. plaintext JSON) and are never interchangeable, so a
        // build only ever reads the version it can actually interpret.
        if header.version != VERSION {
            return Err(anyhow!(Self::wrong_envelope_message(header.version)));
        }

        let encrypted_data = &file_data[4 + header_len..];
        #[cfg(windows)]
        let decrypted_data = Self::decrypt_data(encrypted_data)?;
        #[cfg(not(windows))]
        let decrypted_data = encrypted_data.to_vec();

        serde_json::from_slice(&decrypted_data)
            .map_err(|error| anyhow!("Failed to deserialize data: {error}"))
    }

    #[must_use]
    pub fn exists(&self, filename: &str) -> bool {
        Self::validate_filename(filename).is_ok()
            && self.storage_path.join(format!("{filename}.enc")).exists()
    }

    /// Delete one payload when present.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid ID or filesystem failure.
    pub fn delete(&self, filename: &str) -> Result<()> {
        Self::validate_filename(filename)?;
        let file_path = self.storage_path.join(format!("{filename}.enc"));
        if file_path.exists() {
            fs::remove_file(file_path)
                .map_err(|error| anyhow!("Failed to delete encrypted file: {error}"))?;
        }
        Ok(())
    }

    /// Enumerate stored payload IDs in deterministic lexical order.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be enumerated.
    pub fn list_files(&self) -> Result<Vec<String>> {
        if !self.storage_path.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.storage_path)
            .map_err(|error| anyhow!("Failed to read storage directory: {error}"))?;
        let mut files = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| anyhow!("Failed to read directory entry: {error}"))?
                .path();
            if path.extension().is_some_and(|extension| extension == "enc")
                && let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
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
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestData {
        id: String,
        values: HashMap<String, String>,
    }

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "wfdiag_history_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn v2_envelope_roundtrips_and_replaces_atomically() {
        let directory = temp_dir("encrypted");
        let storage = EncryptedStorage::new(directory.clone()).expect("create storage");
        let first = TestData {
            id: "same".into(),
            values: HashMap::from([("value".into(), "first".into())]),
        };
        let second = TestData {
            id: "same".into(),
            values: HashMap::from([("value".into(), "second".into())]),
        };
        storage.store("same", &first).expect("store first");
        storage.store("same", &second).expect("replace first");
        assert_eq!(storage.load::<TestData>("same").expect("load"), second);
        let envelope = fs::read(directory.join("same.enc")).expect("read envelope");
        let header_len =
            u32::from_le_bytes([envelope[0], envelope[1], envelope[2], envelope[3]]) as usize;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&envelope[4..4 + header_len])
                .expect("parse header"),
            serde_json::json!({ "version": VERSION })
        );
        assert!(!directory.join("same.enc.tmp").exists());
        fs::remove_dir_all(directory).ok();
    }

    /// Hand-build an envelope with an arbitrary header version and body so
    /// both directions of the #216 refusal can be tested from either platform.
    fn envelope(version: u8, body: &[u8]) -> Vec<u8> {
        let header = serde_json::to_vec(&EncryptedHeader { version }).expect("serialize header");
        let mut file = u32::try_from(header.len())
            .expect("header length")
            .to_le_bytes()
            .to_vec();
        file.extend(header);
        file.extend(body);
        file
    }

    #[test]
    fn plaintext_fallback_uses_its_own_envelope_version() {
        let directory = temp_dir("envelope_version");
        let storage = EncryptedStorage::new(directory.clone()).expect("create storage");
        storage.store("scan", &42_u8).expect("store");
        let envelope = fs::read(directory.join("scan.enc")).expect("read envelope");
        let header_len =
            u32::from_le_bytes([envelope[0], envelope[1], envelope[2], envelope[3]]) as usize;
        let header: serde_json::Value =
            serde_json::from_slice(&envelope[4..4 + header_len]).expect("parse header");
        // Windows keeps writing the shipping DPAPI envelope byte-for-byte; the
        // plaintext development fallback is a different version entirely.
        if cfg!(windows) {
            assert_eq!(header, serde_json::json!({ "version": 2 }));
            assert_ne!(&envelope[4 + header_len..], b"42");
        } else {
            assert_eq!(header, serde_json::json!({ "version": 3 }));
            assert_eq!(&envelope[4 + header_len..], b"42");
        }
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn an_envelope_from_the_other_build_is_refused_rather_than_misread() {
        let directory = temp_dir("foreign_envelope");
        let storage = EncryptedStorage::new(directory.clone()).expect("create storage");
        fs::create_dir_all(&directory).expect("create directory");

        // A plaintext-fallback file must never be accepted by Windows, and a
        // DPAPI file must never be handed back as plaintext off Windows.
        let foreign = if cfg!(windows) {
            VERSION_PLAINTEXT_FALLBACK
        } else {
            VERSION_DPAPI
        };
        fs::write(directory.join("foreign.enc"), envelope(foreign, b"42")).expect("write envelope");
        let error = storage
            .load::<u8>("foreign")
            .expect_err("a foreign envelope must be refused")
            .to_string();
        assert!(
            error.contains(if cfg!(windows) {
                "unprotected"
            } else {
                "DPAPI"
            }),
            "unexpected refusal message: {error}"
        );

        fs::write(directory.join("future.enc"), envelope(9, b"42")).expect("write envelope");
        assert!(
            storage
                .load::<u8>("future")
                .expect_err("unknown versions stay unsupported")
                .to_string()
                .contains("Unsupported encryption version: 9")
        );
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn unsafe_storage_ids_are_rejected() {
        let storage = EncryptedStorage::new(temp_dir("ids")).expect("create storage");
        assert!(storage.store("../escape", &42).is_err());
        assert!(storage.load::<u8>("C:/escape").is_err());
    }
}
