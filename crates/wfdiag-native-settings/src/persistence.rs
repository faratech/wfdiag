use crate::{CredentialStorage, ProviderKeyId, SettingsError, SettingsStorage};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SETTINGS_DIRECTORY: &str = "com.windowsforum.diagnostics";
const SETTINGS_FILENAME: &str = "settings.json";
const CREDENTIALS_DIRECTORY: &str = "WFDiag";

/// Shipping provider credentials have always used current-user DPAPI without
/// optional entropy. Existing files can only be decrypted if this remains
/// `None`.
pub const DPAPI_ADDITIONAL_ENTROPY: Option<&'static [u8]> = None;

/// Build the exact settings path below a Windows roaming-app-data root.
#[must_use]
pub fn settings_path_from_config_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_DIRECTORY).join(SETTINGS_FILENAME)
}

/// Build one closed-set provider path below a Windows local-app-data root.
#[must_use]
pub fn credential_path_from_local_data_dir(
    local_data_dir: &Path,
    provider: ProviderKeyId,
) -> PathBuf {
    local_data_dir
        .join(CREDENTIALS_DIRECTORY)
        .join(provider.credential_filename())
}

fn ensure_parent(path: &Path) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SettingsError::Storage(error.to_string()))?;
    }
    Ok(())
}

/// Resolve and create the shipping `%APPDATA%` settings directory.
///
/// # Errors
///
/// Returns a storage error if the roaming config root is unavailable or the
/// application directory cannot be created.
pub fn shipping_settings_path() -> Result<PathBuf, SettingsError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| SettingsError::Storage("Could not find config directory".to_string()))?;
    let path = settings_path_from_config_dir(&config_dir);
    ensure_parent(&path)?;
    Ok(path)
}

/// Resolve and create the shipping `%LOCALAPPDATA%/WFDiag` credential
/// directory for one provider from the closed identifier set.
///
/// # Errors
///
/// Returns a credential error if local app data is unavailable or the
/// credential directory cannot be created.
pub fn shipping_credential_path(provider: ProviderKeyId) -> Result<PathBuf, SettingsError> {
    let local_data_dir = dirs::data_local_dir().ok_or_else(|| {
        SettingsError::Credential("Could not find local app data directory".to_string())
    })?;
    let path = credential_path_from_local_data_dir(&local_data_dir, provider);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SettingsError::Credential(error.to_string()))?;
    }
    Ok(path)
}

/// Atomically replace `path` through a sibling `.tmp` file, flushing file data
/// before the final same-volume rename.
///
/// # Errors
///
/// Returns a human-readable filesystem error. A failure before replacement
/// leaves the previous destination untouched.
pub fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
    }
    let temporary_path = temporary_sibling(path);
    {
        let mut file = fs::File::create(&temporary_path).map_err(|error| {
            format!(
                "Failed to create temp file {}: {error}",
                temporary_path.display()
            )
        })?;
        file.write_all(contents)
            .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("Failed to flush {}: {error}", path.display()))?;
    }
    replace_file(&temporary_path, path)
}

fn temporary_sibling(path: &Path) -> PathBuf {
    let mut filename = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    filename.push(".tmp");
    path.with_file_name(filename)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn replace_file(temporary_path: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let encode = |candidate: &Path| -> Vec<u16> {
        candidate
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let temporary_wide = encode(temporary_path);
    let destination_wide = encode(path);
    unsafe {
        MoveFileExW(
            PCWSTR(temporary_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| format!("Failed to finalize {}: {error}", path.display()))
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temporary_path, path)
        .map_err(|error| format!("Failed to finalize {}: {error}", path.display()))
}

#[derive(Debug, Clone)]
enum SettingsLocation {
    Shipping,
    Exact(PathBuf),
}

/// Concrete crash-safe settings store used by Tauri and native Windows UI
/// shells.
#[derive(Debug, Clone)]
pub struct ShippingSettingsStorage {
    location: SettingsLocation,
}

impl ShippingSettingsStorage {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            location: SettingsLocation::Shipping,
        }
    }

    /// Construct with an exact file path for deterministic tests or migration
    /// tooling. Normal application code should use [`Self::new`].
    #[must_use]
    pub fn at_path(path: PathBuf) -> Self {
        Self {
            location: SettingsLocation::Exact(path),
        }
    }

    /// Resolve the settings file and ensure its parent exists.
    ///
    /// # Errors
    ///
    /// Returns a storage/path-creation error.
    pub fn path(&self) -> Result<PathBuf, SettingsError> {
        match &self.location {
            SettingsLocation::Shipping => shipping_settings_path(),
            SettingsLocation::Exact(path) => {
                ensure_parent(path)?;
                Ok(path.clone())
            }
        }
    }
}

impl Default for ShippingSettingsStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsStorage for ShippingSettingsStorage {
    fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
        let path = self.path()?;
        if !path.exists() {
            return Ok(None);
        }
        fs::read(path)
            .map(Some)
            .map_err(|error| SettingsError::Storage(error.to_string()))
    }

    fn save(&self, serialized: &[u8]) -> Result<(), SettingsError> {
        let path = self.path()?;
        atomic_write_file(&path, serialized).map_err(SettingsError::Storage)
    }
}

trait SecretProtector: Send + Sync + 'static {
    fn protect(&self, plaintext: &str) -> Result<Vec<u8>, String>;
    fn unprotect(&self, protected: &[u8]) -> Result<String, String>;
}

#[derive(Debug, Clone, Copy, Default)]
struct DpapiProtector;

#[cfg(windows)]
impl SecretProtector for DpapiProtector {
    #[allow(unsafe_code)]
    fn protect(&self, plaintext: &str) -> Result<Vec<u8>, String> {
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
        };
        use windows::core::PCWSTR;

        let mut plaintext_bytes = plaintext.as_bytes().to_vec();
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(plaintext_bytes.len())
                .map_err(|_| "DPAPI input is too large".to_string())?,
            pbData: plaintext_bytes.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        debug_assert!(DPAPI_ADDITIONAL_ENTROPY.is_none());
        unsafe {
            CryptProtectData(
                &raw const input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output,
            )
        }
        .map_err(|error| format!("DPAPI encryption failed: {error:?}"))?;

        let protected = unsafe {
            let length = usize::try_from(output.cbData)
                .map_err(|_| "DPAPI output is too large".to_string())?;
            let bytes = std::slice::from_raw_parts(output.pbData, length).to_vec();
            let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            bytes
        };
        Ok(protected)
    }

    #[allow(unsafe_code)]
    fn unprotect(&self, protected: &[u8]) -> Result<String, String> {
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
        };

        let mut protected_copy = protected.to_vec();
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(protected_copy.len())
                .map_err(|_| "DPAPI payload is too large".to_string())?,
            pbData: protected_copy.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        debug_assert!(DPAPI_ADDITIONAL_ENTROPY.is_none());
        unsafe {
            CryptUnprotectData(
                &raw const input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output,
            )
        }
        .map_err(|error| format!("DPAPI decryption failed: {error:?}"))?;

        let plaintext = unsafe {
            let length = usize::try_from(output.cbData)
                .map_err(|_| "DPAPI output is too large".to_string())?;
            let bytes = std::slice::from_raw_parts(output.pbData, length);
            let value = String::from_utf8_lossy(bytes).into_owned();
            let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            value
        };
        Ok(plaintext)
    }
}

#[cfg(not(windows))]
impl SecretProtector for DpapiProtector {
    fn protect(&self, _plaintext: &str) -> Result<Vec<u8>, String> {
        Err("DPAPI encryption is only available on Windows".to_string())
    }

    fn unprotect(&self, _protected: &[u8]) -> Result<String, String> {
        Err("DPAPI decryption is only available on Windows".to_string())
    }
}

#[derive(Debug, Clone)]
enum CredentialLocation {
    Shipping,
    #[cfg(test)]
    Directory(PathBuf),
}

/// Exact current-user DPAPI file store used by shipping Windows builds.
#[derive(Clone)]
pub struct WindowsDpapiCredentialStorage {
    location: CredentialLocation,
    protector: Arc<dyn SecretProtector>,
}

impl std::fmt::Debug for WindowsDpapiCredentialStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsDpapiCredentialStorage")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl WindowsDpapiCredentialStorage {
    #[must_use]
    pub fn new() -> Self {
        Self {
            location: CredentialLocation::Shipping,
            protector: Arc::new(DpapiProtector),
        }
    }

    #[cfg(test)]
    fn at_directory_with_protector(
        directory: PathBuf,
        protector: Arc<dyn SecretProtector>,
    ) -> Self {
        Self {
            location: CredentialLocation::Directory(directory),
            protector,
        }
    }

    /// Resolve one provider's fixed credential path.
    ///
    /// # Errors
    ///
    /// Returns a credential/path-creation error.
    pub fn path(&self, provider: ProviderKeyId) -> Result<PathBuf, SettingsError> {
        match &self.location {
            CredentialLocation::Shipping => shipping_credential_path(provider),
            #[cfg(test)]
            CredentialLocation::Directory(directory) => {
                fs::create_dir_all(directory)
                    .map_err(|error| SettingsError::Credential(error.to_string()))?;
                Ok(directory.join(provider.credential_filename()))
            }
        }
    }
}

impl Default for WindowsDpapiCredentialStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStorage for WindowsDpapiCredentialStorage {
    fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
        if key.is_empty() {
            return self.clear(provider);
        }
        let protected = self
            .protector
            .protect(key)
            .map_err(SettingsError::Credential)?;
        let path = self.path(provider)?;
        atomic_write_file(&path, &protected).map_err(SettingsError::Credential)
    }

    fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
        // Shipping Settings treats unavailable/corrupt credentials as not set;
        // one bad secret must never block the whole Settings UI.
        let Ok(path) = self.path(provider) else {
            return Ok(None);
        };
        let Ok(protected) = fs::read(&path) else {
            return Ok(None);
        };
        if protected.is_empty() {
            return Ok(None);
        }
        match self.protector.unprotect(&protected) {
            Ok(value) if !value.is_empty() => Ok(Some(value)),
            Ok(_) => Ok(None),
            Err(error) => {
                eprintln!(
                    "WARNING: Stored credentials at {} could not be decrypted ({error}). The saved API key was discarded — please re-enter it in Settings.",
                    path.display()
                );
                Ok(None)
            }
        }
    }

    fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
        let path = self.path(provider)?;
        if path.exists() {
            fs::remove_file(path).map_err(|error| SettingsError::Credential(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    fn test_directory(name: &str) -> PathBuf {
        let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "wfdiag_native_settings_{name}_{}_{id}",
            std::process::id()
        ))
    }

    fn remove_test_directory(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[derive(Debug)]
    struct TestProtector;

    impl SecretProtector for TestProtector {
        fn protect(&self, plaintext: &str) -> Result<Vec<u8>, String> {
            let mut protected = b"test-v1:".to_vec();
            protected.extend(plaintext.bytes().map(|byte| byte ^ 0xA5));
            Ok(protected)
        }

        fn unprotect(&self, protected: &[u8]) -> Result<String, String> {
            let payload = protected
                .strip_prefix(b"test-v1:")
                .ok_or_else(|| "corrupt test secret".to_string())?;
            String::from_utf8(payload.iter().map(|byte| byte ^ 0xA5).collect())
                .map_err(|error| error.to_string())
        }
    }

    #[test]
    fn shipping_paths_and_closed_provider_files_are_exact() {
        let roaming = Path::new("/profile/AppData/Roaming");
        assert_eq!(
            settings_path_from_config_dir(roaming),
            roaming
                .join("com.windowsforum.diagnostics")
                .join("settings.json")
        );
        let local = Path::new("/profile/AppData/Local");
        let expected = [
            (ProviderKeyId::OpenAI, "credentials.bin"),
            (ProviderKeyId::Anthropic, "credentials_anthropic.bin"),
            (ProviderKeyId::Gemini, "credentials_gemini.bin"),
            (ProviderKeyId::DeepSeek, "credentials_deepseek.bin"),
            (ProviderKeyId::Custom, "credentials_custom.bin"),
        ];
        for (provider, filename) in expected {
            assert_eq!(
                credential_path_from_local_data_dir(local, provider),
                local.join("WFDiag").join(filename)
            );
        }
        assert!(DPAPI_ADDITIONAL_ENTROPY.is_none());
        assert!(ProviderKeyId::parse("../credentials.bin").is_err());
        assert!(ProviderKeyId::parse("phi_silica").is_err());
    }

    #[test]
    fn settings_store_atomically_overwrites_and_leaves_no_temp_file() {
        let directory = test_directory("settings");
        let path = directory.join("nested").join("settings.json");
        let storage = ShippingSettingsStorage::at_path(path.clone());
        storage.save(br#"{"version":1}"#).unwrap();
        storage.save(br#"{"version":2}"#).unwrap();
        assert_eq!(storage.load().unwrap().unwrap(), br#"{"version":2}"#);
        assert!(!temporary_sibling(&path).exists());
        remove_test_directory(&directory);
    }

    #[test]
    fn failed_temp_creation_preserves_the_previous_settings_file() {
        let directory = test_directory("atomic_failure");
        let path = directory.join("settings.json");
        atomic_write_file(&path, b"previous").unwrap();
        fs::create_dir(temporary_sibling(&path)).unwrap();
        assert!(atomic_write_file(&path, b"replacement").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        remove_test_directory(&directory);
    }

    #[test]
    fn provider_secret_files_are_protected_atomic_and_clearable() {
        let directory = test_directory("credentials");
        let storage = WindowsDpapiCredentialStorage::at_directory_with_protector(
            directory.clone(),
            Arc::new(TestProtector),
        );
        storage
            .store(ProviderKeyId::OpenAI, "sk-never-plaintext")
            .unwrap();
        let path = directory.join("credentials.bin");
        let bytes = fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-never-plaintext"));
        assert!(!temporary_sibling(&path).exists());
        assert_eq!(
            storage.load(ProviderKeyId::OpenAI).unwrap().as_deref(),
            Some("sk-never-plaintext")
        );
        storage.clear(ProviderKeyId::OpenAI).unwrap();
        assert!(!path.exists());
        remove_test_directory(&directory);
    }

    #[test]
    fn corrupt_or_empty_provider_secret_is_reported_as_not_configured() {
        let directory = test_directory("corrupt_credentials");
        let storage = WindowsDpapiCredentialStorage::at_directory_with_protector(
            directory.clone(),
            Arc::new(TestProtector),
        );
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("credentials_gemini.bin");
        fs::write(&path, b"not-a-protected-secret").unwrap();
        assert_eq!(storage.load(ProviderKeyId::Gemini).unwrap(), None);
        fs::write(&path, []).unwrap();
        assert_eq!(storage.load(ProviderKeyId::Gemini).unwrap(), None);
        remove_test_directory(&directory);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_uses_current_user_and_no_entropy() {
        let protector = DpapiProtector;
        let protected = protector.protect("sk-test-key-12345").unwrap();
        assert!(!protected.is_empty());
        assert_ne!(protected, b"sk-test-key-12345");
        assert_eq!(
            protector.unprotect(&protected).unwrap(),
            "sk-test-key-12345"
        );
        assert!(DPAPI_ADDITIONAL_ENTROPY.is_none());
    }
}
