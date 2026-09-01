//! Settings-related Tauri commands.
//!
//! This module handles application settings persistence and API key management.
//!
//! # Commands
//! - `save_settings` - Save application settings to disk
//! - `load_settings` - Load application settings from disk
//! - `store_api_key` - Store OpenAI API key securely (legacy single-key command)
//! - `load_api_key` - Legacy command retained as a write-only-policy error
//! - `clear_api_key` - Remove stored OpenAI API key
//! - `store_provider_api_key` / `clear_provider_api_key` - Per-provider keys
//!   (openai, anthropic, gemini, custom_openai)
//!
//! # Security
//! API keys are stored using:
//! - Windows: DPAPI encryption (one file per provider)
//! - Other platforms: System keyring (one entry per provider)
//!
//! Keys are NEVER returned to the webview or written to settings JSON — the
//! secret fields deserialize write requests but are skipped by Serialize.

use crate::dpapi::ProviderKeyId;
use crate::error::DiagError;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
pub use wfdiag_native_settings::{AppSettings, CloudFallbackPolicy};
use wfdiag_native_settings::{
    CredentialStorage, SettingsError, SettingsService, SettingsStorage, ShippingSettingsStorage,
};

#[cfg(not(windows))]
use keyring_core::Entry;
#[cfg(not(windows))]
use std::sync::OnceLock;

#[cfg(not(windows))]
static KEYRING_STORE_INIT: OnceLock<Result<(), String>> = OnceLock::new();

static NETWORK_GROUNDING_ENABLED: AtomicBool = AtomicBool::new(false);
// 0 = ask, 1 = allow, 2 = never
static CLOUD_FALLBACK_POLICY: AtomicU8 = AtomicU8::new(0);

/// Read settings from disk without hydrating secrets. For backend code that
/// needs provider configuration (endpoints, model names) on a hot path.
pub(crate) fn read_settings_from_disk() -> Option<AppSettings> {
    let path = get_settings_path().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Get the settings file path
pub fn get_settings_path() -> Result<std::path::PathBuf, String> {
    let app_data =
        dirs::config_dir().ok_or_else(|| DiagError::internal("Could not find config directory"))?;
    let path = wfdiag_native_settings::settings_path_from_config_dir(&app_data);
    let settings_dir = path.parent().ok_or_else(|| {
        DiagError::internal("Settings path did not contain an application directory")
    })?;

    // Create directory if it doesn't exist
    if !settings_dir.exists() {
        std::fs::create_dir_all(settings_dir)
            .map_err(|e| DiagError::file(settings_dir.display().to_string(), e.to_string()))?;
    }

    Ok(path)
}

/// Preserve Tauri's structured file-error contract while delegating all path,
/// load, and atomic-write mechanics to the shared shipping store.
struct TauriSettingsStorage;

impl SettingsStorage for TauriSettingsStorage {
    fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
        let path = get_settings_path().map_err(SettingsError::Storage)?;
        ShippingSettingsStorage::at_path(path.clone())
            .load()
            .map_err(|error| {
                let reason = match error {
                    SettingsError::Storage(detail) => detail,
                    other => other.to_string(),
                };
                SettingsError::Storage(DiagError::file(path.display().to_string(), reason).into())
            })
    }

    fn save(&self, serialized: &[u8]) -> Result<(), SettingsError> {
        let path = get_settings_path().map_err(SettingsError::Storage)?;
        ShippingSettingsStorage::at_path(path.clone())
            .save(serialized)
            .map_err(|error| {
                let reason = match error {
                    SettingsError::Storage(detail) => detail,
                    other => other.to_string(),
                };
                SettingsError::Storage(DiagError::file(path.display().to_string(), reason).into())
            })
    }
}

struct TauriCredentialStorage;

impl CredentialStorage for TauriCredentialStorage {
    fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
        #[cfg(windows)]
        {
            crate::dpapi::store_provider_key(provider, key).map_err(SettingsError::Credential)
        }
        #[cfg(not(windows))]
        {
            let entry = provider_keyring_entry(provider).map_err(SettingsError::Credential)?;
            entry.set_password(key).map_err(|error| {
                SettingsError::Credential(DiagError::api_key("store", error.to_string()).into())
            })
        }
    }

    fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
        #[cfg(windows)]
        {
            // Preserve the shipping read policy: unavailable/corrupt secrets
            // are reported as "not configured" and never block Settings UI.
            Ok(crate::dpapi::load_provider_key(provider).ok().flatten())
        }
        #[cfg(not(windows))]
        {
            let entry = provider_keyring_entry(provider).map_err(SettingsError::Credential)?;
            match entry.get_password() {
                Ok(value) if !value.is_empty() => Ok(Some(value)),
                Ok(_) | Err(keyring_core::Error::NoEntry) => Ok(None),
                Err(_) => Ok(None),
            }
        }
    }

    fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
        #[cfg(windows)]
        {
            crate::dpapi::clear_provider_key(provider).map_err(SettingsError::Credential)
        }
        #[cfg(not(windows))]
        {
            let entry = provider_keyring_entry(provider).map_err(SettingsError::Credential)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(error) => Err(SettingsError::Credential(
                    DiagError::api_key("clear", error.to_string()).into(),
                )),
            }
        }
    }
}

pub(crate) fn native_settings_service() -> SettingsService {
    SettingsService::new(
        Arc::new(TauriSettingsStorage),
        Arc::new(TauriCredentialStorage),
        Arc::new(crate::ai_service::provider_preference_settings_validator()),
    )
}

fn map_native_settings_error(error: SettingsError) -> String {
    match error {
        SettingsError::Storage(detail)
        | SettingsError::Credential(detail)
        | SettingsError::Validation(detail) => detail,
        SettingsError::Serialization(detail) => DiagError::serialization(detail).into(),
        SettingsError::Runtime(detail) => DiagError::internal(detail).into(),
    }
}

fn parse_provider_key_id(provider: &str) -> Result<ProviderKeyId, String> {
    ProviderKeyId::parse(provider).map_err(|error| match error {
        SettingsError::Credential(detail) => DiagError::api_key("store", detail).into(),
        other => DiagError::api_key("store", other.to_string()).into(),
    })
}

#[cfg(not(windows))]
fn ensure_keyring_store() -> Result<(), String> {
    KEYRING_STORE_INIT.get_or_init(init_keyring_store).clone()
}

#[cfg(not(windows))]
fn init_keyring_store() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let store = apple_native_keyring_store::keychain::Store::new()
            .map_err(|e| DiagError::api_key("access", e.to_string()))?;
        keyring_core::set_default_store(store);
        return Ok(());
    }

    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    {
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|e| DiagError::api_key("access", e.to_string()))?;
        keyring_core::set_default_store(store);
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(DiagError::PlatformNotSupported {
        operation: "system keyring".to_string(),
    }
    .into())
}

#[cfg(not(windows))]
fn provider_keyring_entry(id: ProviderKeyId) -> Result<Entry, String> {
    ensure_keyring_store()?;
    Entry::new("wfdiag-tauri", id.keyring_user())
        .map_err(|e| DiagError::api_key("access", e.to_string()).into())
}

/// Store or clear one provider key in the platform secret store.
/// `Some("")` means "clear"; `None` means "not provided, leave untouched".
fn persist_provider_key(id: ProviderKeyId, key: Option<&str>) -> Result<(), String> {
    let Some(key) = key else { return Ok(()) };
    native_settings_service()
        .store_provider_key(id, key)
        .map_err(map_native_settings_error)
}

/// Load one provider key from the platform secret store.
pub async fn load_provider_key_internal(id: ProviderKeyId) -> Option<String> {
    native_settings_service()
        .load_provider_key(id)
        .ok()
        .flatten()
}

/// Copy of the settings with every API key stripped — the only shape that may
/// be serialized to the settings file. Keys live in DPAPI/keyring exclusively.
fn settings_for_disk(settings: &AppSettings) -> AppSettings {
    settings.for_disk()
}

#[tauri::command]
pub async fn save_settings(settings: AppSettings) -> Result<(), String> {
    // The native service preserves validation-before-secret-write ordering,
    // write-only secret handling, and the existing atomic settings store.
    native_settings_service()
        .save(&settings)
        .map_err(map_native_settings_error)?;
    let path = get_settings_path()?;

    // Sync settings the backend consults in memory
    sync_in_memory_state(&settings);

    println!("Settings saved to {:?}", path);
    Ok(())
}

/// Mirror settings that backend code reads on hot paths into in-memory state
/// (AI provider routing, the close-to-tray window handler).
fn sync_in_memory_state(settings: &AppSettings) {
    let pref = crate::ai_service::provider_preference_for_runtime(&settings.preferred_ai_provider);
    crate::ai_service::set_user_preference(pref);
    crate::tray::set_close_to_tray(settings.close_to_tray);
    NETWORK_GROUNDING_ENABLED.store(settings.network_grounding_enabled, Ordering::Relaxed);
    CLOUD_FALLBACK_POLICY.store(
        match settings.cloud_fallback_policy {
            CloudFallbackPolicy::Ask => 0,
            CloudFallbackPolicy::Allow => 1,
            CloudFallbackPolicy::Never => 2,
        },
        Ordering::Relaxed,
    );
}

pub(crate) fn sync_in_memory_state_from_disk() {
    let settings = read_settings_from_disk().unwrap_or_default();
    sync_in_memory_state(&settings);
}

fn normalize_unavailable_provider_for_runtime(settings: &mut AppSettings) {
    if crate::ai_service::parse_and_validate_provider_preference(&settings.preferred_ai_provider)
        .is_err()
    {
        settings.preferred_ai_provider = "auto".to_string();
    }
}

#[tauri::command]
pub async fn load_settings() -> Result<AppSettings, String> {
    let path = get_settings_path()?;
    let mut settings = native_settings_service()
        .load()
        .map_err(map_native_settings_error)?;

    // Older loose builds allowed `phi_silica` to be persisted even though
    // Windows AI cannot run without package identity. Normalize only the
    // in-memory/IPC view; leave the file untouched so installing the Store
    // package later does not destroy the user's original preference.
    normalize_unavailable_provider_for_runtime(&mut settings);

    // Sync loaded settings to in-memory state
    sync_in_memory_state(&settings);

    println!("Settings loaded from {:?}", path);
    Ok(settings)
}

/// Store an API key for a specific provider (openai, anthropic, gemini,
/// custom_openai). An empty key clears the stored value.
#[tauri::command]
pub async fn store_provider_api_key(provider: String, key: String) -> Result<(), String> {
    persist_provider_key(parse_provider_key_id(&provider)?, Some(&key))
}

/// Clear the stored API key for a specific provider.
#[tauri::command]
pub async fn clear_provider_api_key(provider: String) -> Result<(), String> {
    persist_provider_key(parse_provider_key_id(&provider)?, Some(""))
}

/// Legacy single-key command — stores the OpenAI key.
#[tauri::command]
pub async fn store_api_key(key: String) -> Result<(), String> {
    persist_provider_key(ProviderKeyId::OpenAI, Some(&key))
}

#[tauri::command]
pub async fn load_api_key() -> Result<String, String> {
    Err(DiagError::api_key(
        "load",
        "API keys are write-only and cannot be returned to the webview",
    )
    .into())
}

/// Network grounding is enabled only by the persisted opt-in. The legacy
/// environment variable remains a kill-switch, but cannot enable grounding
/// when the user setting is off.
pub(crate) fn network_grounding_enabled() -> bool {
    let opted_in = NETWORK_GROUNDING_ENABLED.load(Ordering::Relaxed);
    let env_allows = std::env::var("WFDIAG_AI_GROUNDING")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(true);
    opted_in && env_allows
}

pub(crate) fn cloud_fallback_policy() -> CloudFallbackPolicy {
    match CLOUD_FALLBACK_POLICY.load(Ordering::Relaxed) {
        1 => CloudFallbackPolicy::Allow,
        2 => CloudFallbackPolicy::Never,
        _ => CloudFallbackPolicy::Ask,
    }
}

/// Current scan-history retention policy for backend save/cleanup paths.
pub(crate) fn history_retention() -> (bool, u32) {
    let settings = read_settings_from_disk().unwrap_or_default();
    (settings.retain_history, settings.history_limit)
}

/// Persist a fallback choice made in the typed consent flow without ever
/// loading or rewriting secret material.
pub(crate) fn persist_cloud_fallback_policy(policy: CloudFallbackPolicy) -> Result<(), String> {
    let path = get_settings_path()?;
    let mut settings = read_settings_from_disk().unwrap_or_default();
    settings.cloud_fallback_policy = policy;
    let json = serde_json::to_string_pretty(&settings_for_disk(&settings))
        .map_err(|e| DiagError::serialization(e.to_string()))?;
    wfdiag_native_settings::atomic_write_file(&path, json.as_bytes())
        .map_err(|e| DiagError::file(path.display().to_string(), e).to_string())?;
    CLOUD_FALLBACK_POLICY.store(
        match policy {
            CloudFallbackPolicy::Ask => 0,
            CloudFallbackPolicy::Allow => 1,
            CloudFallbackPolicy::Never => 2,
        },
        Ordering::Relaxed,
    );
    Ok(())
}

/// Internal function for loading the OpenAI API key (used by ai_service)
pub async fn load_api_key_internal() -> Option<String> {
    load_provider_key_internal(ProviderKeyId::OpenAI).await
}

/// Legacy single-key command — clears the OpenAI key.
#[tauri::command]
pub async fn clear_api_key() -> Result<(), String> {
    persist_provider_key(ProviderKeyId::OpenAI, Some(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_keys() -> AppSettings {
        AppSettings {
            open_ai_api_key: Some("sk-openai".into()),
            anthropic_api_key: Some("sk-ant".into()),
            gemini_api_key: Some("AIza-gem".into()),
            deepseek_api_key: Some("sk-deep".into()),
            custom_api_key: Some("sk-or-custom".into()),
            anthropic_model: Some("claude-sonnet-5".into()),
            custom_endpoint: Some("https://openrouter.ai/api".into()),
            custom_model: Some("meta-llama/llama-4".into()),
            ollama_endpoint: Some("http://127.0.0.1:11434".into()),
            ollama_model: Some("llama3.2".into()),
            codex_cli_path: Some(r"C:\Tools\codex.exe".into()),
            codex_model: Some("gpt-5-codex".into()),
            claude_cli_path: Some(r"C:\Tools\claude.exe".into()),
            claude_model: Some("claude-sonnet-5".into()),
            open_ai_model: Some("gpt-5-nano".into()),
            local_ai_model: Some("phi-4-mini".into()),
            ..AppSettings::default()
        }
    }

    #[test]
    fn app_settings_default_matches_frontend_defaults() {
        let settings = AppSettings::default();
        assert!(settings.auto_save);
        assert!(!settings.scan_on_startup);
        assert_eq!(settings.max_concurrent_tasks, 5);
        assert_eq!(settings.export_format, "text");
        assert_eq!(settings.theme, "dark");
        assert!(settings.show_notifications);
        assert!(settings.retain_history);
        assert_eq!(settings.history_limit, 30);
        assert!(settings.ai_enabled);
        assert_eq!(settings.preferred_ai_provider, "auto");
        assert!(!settings.network_grounding_enabled);
        assert_eq!(settings.cloud_fallback_policy, CloudFallbackPolicy::Ask);
    }

    #[test]
    #[cfg(not(windows))]
    fn stale_phi_setting_is_normalized_in_memory_without_touching_other_fields() {
        let mut settings = AppSettings {
            preferred_ai_provider: "phi_silica".to_string(),
            theme: "light".to_string(),
            ..AppSettings::default()
        };
        normalize_unavailable_provider_for_runtime(&mut settings);
        assert_eq!(settings.preferred_ai_provider, "auto");
        assert_eq!(settings.theme, "light");
    }

    #[test]
    fn settings_for_disk_strips_every_api_key() {
        let on_disk = settings_for_disk(&settings_with_keys());
        assert!(on_disk.open_ai_api_key.is_none());
        assert!(on_disk.anthropic_api_key.is_none());
        assert!(on_disk.gemini_api_key.is_none());
        assert!(on_disk.deepseek_api_key.is_none());
        assert!(on_disk.custom_api_key.is_none());
        // Non-secret provider config must survive
        assert_eq!(on_disk.anthropic_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(
            on_disk.custom_endpoint.as_deref(),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(on_disk.ollama_model.as_deref(), Some("llama3.2"));
        assert_eq!(
            on_disk.codex_cli_path.as_deref(),
            Some(r"C:\Tools\codex.exe")
        );
        assert_eq!(on_disk.codex_model.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn no_key_material_in_serialized_settings() {
        // Belt-and-braces: the serialized JSON must not contain the secrets,
        // under any field name.
        let json = serde_json::to_string(&settings_for_disk(&settings_with_keys())).unwrap();
        for secret in ["sk-openai", "sk-ant", "AIza-gem", "sk-deep", "sk-or-custom"] {
            assert!(!json.contains(secret), "secret {} leaked to disk", secret);
        }
    }

    #[test]
    fn settings_serde_uses_camel_case_field_names() {
        // The TS SettingsData interface depends on these exact JSON keys
        let json = serde_json::to_string(&settings_with_keys()).unwrap();
        for field in [
            "\"openAiApiKeySet\"",
            "\"anthropicApiKeySet\"",
            "\"geminiApiKeySet\"",
            "\"customApiKeySet\"",
            "\"deepseekApiKeySet\"",
            "\"anthropicModel\"",
            "\"geminiModel\"",
            "\"customEndpoint\"",
            "\"customModel\"",
            "\"ollamaEndpoint\"",
            "\"ollamaModel\"",
            "\"codexCliPath\"",
            "\"codexModel\"",
            "\"claudeCliPath\"",
            "\"claudeModel\"",
            "\"openAiModel\"",
            "\"localAiModel\"",
            "\"preferredAIProvider\"",
            "\"networkGroundingEnabled\"",
            "\"cloudFallbackPolicy\"",
        ] {
            // geminiModel is None in the fixture and skipped — check it on a
            // populated copy instead
            if field == "\"geminiModel\"" {
                let mut s = settings_with_keys();
                s.gemini_model = Some("gemini-2.5-flash".into());
                assert!(serde_json::to_string(&s).unwrap().contains(field));
            } else {
                assert!(json.contains(field), "missing JSON key {}", field);
            }
        }
        for secret_field in [
            "\"openAiApiKey\"",
            "\"anthropicApiKey\"",
            "\"geminiApiKey\"",
            "\"customApiKey\"",
            "\"deepseekApiKey\"",
        ] {
            assert!(
                !json.contains(secret_field),
                "secret field {} was serialized",
                secret_field
            );
        }
    }

    #[test]
    fn old_settings_files_still_load() {
        // A pre-2.5.0 settings file (no provider fields) must deserialize
        let old = r#"{"autoSave":true,"theme":"dark","preferredAIProvider":"auto"}"#;
        let s: AppSettings = serde_json::from_str(old).unwrap();
        assert!(s.auto_save);
        assert!(s.anthropic_api_key.is_none());
        assert!(s.ollama_endpoint.is_none());
        assert!(!s.network_grounding_enabled);
        assert_eq!(s.cloud_fallback_policy, CloudFallbackPolicy::Ask);
    }
}
