//! Compatibility adapter for the shared shipping DPAPI credential store.
//!
//! The file paths, provider allowlist, no-entropy DPAPI calls, atomic writes,
//! and corrupt-secret policy are owned by `wfdiag-native-settings` so Tauri
//! and native Windows UI shells cannot drift.

use crate::error::DiagError;
use wfdiag_native_settings::{CredentialStorage, WindowsDpapiCredentialStorage};

pub use wfdiag_native_settings::ProviderKeyId;

fn storage() -> WindowsDpapiCredentialStorage {
    WindowsDpapiCredentialStorage::new()
}

/// Store one provider key in its established current-user DPAPI file.
pub fn store_provider_key(id: ProviderKeyId, key: &str) -> Result<(), String> {
    storage()
        .store(id, key)
        .map_err(|error| DiagError::api_key("store", error.to_string()).into())
}

/// Load one provider key. Unavailable, empty, or corrupt files remain `None`.
pub fn load_provider_key(id: ProviderKeyId) -> Result<Option<String>, String> {
    storage()
        .load(id)
        .map_err(|error| DiagError::api_key("load", error.to_string()).into())
}

/// Clear one provider's fixed DPAPI file if present.
pub fn clear_provider_key(id: ProviderKeyId) -> Result<(), String> {
    storage()
        .clear(id)
        .map_err(|error| DiagError::api_key("clear", error.to_string()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_adapter_uses_the_closed_shared_provider_contract() {
        assert_eq!(
            ProviderKeyId::OpenAI.credential_filename(),
            "credentials.bin"
        );
        assert_eq!(
            ProviderKeyId::Anthropic.credential_filename(),
            "credentials_anthropic.bin"
        );
        assert!(ProviderKeyId::parse("../evil").is_err());
        assert!(wfdiag_native_settings::DPAPI_ADDITIONAL_ENTROPY.is_none());
    }
}
