//! Settings-related Tauri commands.
//!
//! This module handles application settings persistence and API key management.
//!
//! # Commands
//! - `save_settings` - Save application settings to disk
//! - `load_settings` - Load application settings from disk
//! - `store_api_key` - Store OpenAI API key securely (legacy single-key command)
//! - `load_api_key` - Load OpenAI API key from secure storage
//! - `clear_api_key` - Remove stored OpenAI API key
//! - `store_provider_api_key` / `clear_provider_api_key` - Per-provider keys
//!   (openai, anthropic, gemini, custom_openai)
//!
//! # Security
//! API keys are stored using:
//! - Windows: DPAPI encryption (one file per provider)
//! - Other platforms: System keyring (one entry per provider)
//!
//! Keys are NEVER written to the settings JSON file — `settings_for_disk`
//! strips every key field before serialization.

use crate::dpapi::ProviderKeyId;
use crate::error::DiagError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(not(windows))]
use keyring_core::{Entry, Error as KeyringError};
#[cfg(not(windows))]
use std::sync::OnceLock;

#[cfg(not(windows))]
static KEYRING_STORE_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Application settings that persist across sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_true")]
    pub auto_save: bool,
    #[serde(default)]
    pub scan_on_startup: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    #[serde(default = "default_export_format")]
    pub export_format: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub show_notifications: bool,
    #[serde(default)]
    pub custom_export_path: Option<String>,
    #[serde(default = "default_true")]
    pub retain_history: bool,
    #[serde(default = "default_history_limit")]
    pub history_limit: u32,
    // AI settings
    #[serde(default = "default_true")]
    pub ai_enabled: bool,
    #[serde(default = "default_ai_provider", rename = "preferredAIProvider")]
    pub preferred_ai_provider: String,
    // API keys — stored in DPAPI/keyring, included in the frontend response,
    // NEVER written to the settings file (see settings_for_disk)
    #[serde(default)]
    pub open_ai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_api_key: Option<String>,
    // Per-provider model overrides; empty falls back to the provider module's
    // default constant (anthropic.rs / gemini.rs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_model: Option<String>,
    // Generic OpenAI-compatible endpoint (OpenRouter, Groq, …): base URL,
    // model is required for the provider to be usable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_model: Option<String>,
    // Ollama: empty endpoint auto-discovers http://127.0.0.1:11434, empty
    // model resolves to the first entry from /api/tags
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_model: Option<String>,
    // User-customized Quick Scan task IDs. camelCase rename => "quickScanTasks" to match
    // the TS SettingsData field; #[serde(default)] keeps old settings files loadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_scan_tasks: Option<Vec<String>>,
    // Optional base URL of a local OpenAI-compatible endpoint (e.g. Foundry
    // Local). When unset, the endpoint is discovered via the foundry CLI —
    // its port is dynamic by design and must not be hardcoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_ai_endpoint: Option<String>,
    // Microsoft-issued Limited Access Feature token for Phi Silica
    // (systemAIModels). When set, the supported WinRT activation path works
    // without the bundled-DLL bypass. Empty uses the built-in fallback token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phi_silica_laf_token: Option<String>,
    // Closing the main window hides to the system tray instead of exiting
    #[serde(default)]
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_save: true,
            scan_on_startup: false,
            max_concurrent_tasks: default_max_concurrent(),
            export_format: default_export_format(),
            theme: default_theme(),
            show_notifications: true,
            custom_export_path: None,
            retain_history: true,
            history_limit: default_history_limit(),
            ai_enabled: true,
            preferred_ai_provider: default_ai_provider(),
            open_ai_api_key: None,
            anthropic_api_key: None,
            gemini_api_key: None,
            deepseek_api_key: None,
            custom_api_key: None,
            anthropic_model: None,
            gemini_model: None,
            deepseek_model: None,
            custom_endpoint: None,
            custom_model: None,
            ollama_endpoint: None,
            ollama_model: None,
            quick_scan_tasks: None,
            local_ai_endpoint: None,
            phi_silica_laf_token: None,
            close_to_tray: false,
        }
    }
}

fn default_max_concurrent() -> u32 {
    5
}
fn default_export_format() -> String {
    "text".to_string()
}
fn default_theme() -> String {
    "dark".to_string()
}
fn default_true() -> bool {
    true
}
fn default_history_limit() -> u32 {
    30
}
fn default_ai_provider() -> String {
    "auto".to_string()
}

/// Read settings from disk without hydrating secrets. For backend code that
/// needs provider configuration (endpoints, model names) on a hot path.
pub(crate) fn read_settings_from_disk() -> Option<AppSettings> {
    let path = get_settings_path().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Get the settings file path
pub fn get_settings_path() -> Result<PathBuf, String> {
    let app_data =
        dirs::config_dir().ok_or_else(|| DiagError::internal("Could not find config directory"))?;
    let settings_dir = app_data.join("com.windowsforum.diagnostics");

    // Create directory if it doesn't exist
    if !settings_dir.exists() {
        std::fs::create_dir_all(&settings_dir)
            .map_err(|e| DiagError::file(settings_dir.display().to_string(), e.to_string()))?;
    }

    Ok(settings_dir.join("settings.json"))
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
    #[cfg(windows)]
    {
        // Propagate (don't swallow) failures: otherwise save_settings returns Ok,
        // the UI believes the key persisted, and it silently vanishes on restart.
        // store_provider_key treats an empty string as "clear".
        crate::dpapi::store_provider_key(id, key)
            .map_err(|e| DiagError::api_key("store", e.to_string()).into())
    }
    #[cfg(not(windows))]
    {
        if !key.is_empty() {
            let entry = provider_keyring_entry(id)?;
            entry
                .set_password(key)
                .map_err(|e| DiagError::api_key("store", e.to_string()).into())
        } else {
            if let Ok(entry) = provider_keyring_entry(id) {
                let _ = entry.delete_credential();
            }
            Ok(())
        }
    }
}

/// Load one provider key from the platform secret store.
pub async fn load_provider_key_internal(id: ProviderKeyId) -> Option<String> {
    #[cfg(windows)]
    {
        crate::dpapi::load_provider_key(id).ok().flatten()
    }
    #[cfg(not(windows))]
    {
        let entry = provider_keyring_entry(id).ok()?;
        match entry.get_password() {
            Ok(pwd) if !pwd.is_empty() => Some(pwd),
            _ => None,
        }
    }
}

/// Copy of the settings with every API key stripped — the only shape that may
/// be serialized to the settings file. Keys live in DPAPI/keyring exclusively.
fn settings_for_disk(settings: &AppSettings) -> AppSettings {
    let mut s = settings.clone();
    s.open_ai_api_key = None;
    s.anthropic_api_key = None;
    s.gemini_api_key = None;
    s.deepseek_api_key = None;
    s.custom_api_key = None;
    s
}

#[tauri::command]
pub async fn save_settings(settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path()?;

    // Route any provided keys to secure storage (separate from settings file)
    persist_provider_key(ProviderKeyId::OpenAI, settings.open_ai_api_key.as_deref())?;
    persist_provider_key(
        ProviderKeyId::Anthropic,
        settings.anthropic_api_key.as_deref(),
    )?;
    persist_provider_key(ProviderKeyId::Gemini, settings.gemini_api_key.as_deref())?;
    persist_provider_key(
        ProviderKeyId::DeepSeek,
        settings.deepseek_api_key.as_deref(),
    )?;
    persist_provider_key(ProviderKeyId::Custom, settings.custom_api_key.as_deref())?;

    let json = serde_json::to_string_pretty(&settings_for_disk(&settings))
        .map_err(|e| DiagError::serialization(e.to_string()))?;
    std::fs::write(&path, &json)
        .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;

    // Sync settings the backend consults in memory
    sync_in_memory_state(&settings);

    println!("Settings saved to {:?}", path);
    Ok(())
}

/// Mirror settings that backend code reads on hot paths into in-memory state
/// (AI provider routing, the close-to-tray window handler).
fn sync_in_memory_state(settings: &AppSettings) {
    let pref = match settings.preferred_ai_provider.to_lowercase().as_str() {
        "openai" => crate::ai_service::AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => crate::ai_service::AIProviderPreference::PhiSilica,
        "foundry_local" | "foundrylocal" => crate::ai_service::AIProviderPreference::FoundryLocal,
        "ollama" => crate::ai_service::AIProviderPreference::Ollama,
        "custom_openai" | "custom" => crate::ai_service::AIProviderPreference::CustomOpenAI,
        "anthropic" => crate::ai_service::AIProviderPreference::Anthropic,
        "gemini" => crate::ai_service::AIProviderPreference::Gemini,
        "deepseek" => crate::ai_service::AIProviderPreference::DeepSeek,
        _ => crate::ai_service::AIProviderPreference::Auto,
    };
    crate::ai_service::set_user_preference(pref);
    crate::tray::set_close_to_tray(settings.close_to_tray);
}

pub(crate) fn sync_in_memory_state_from_disk() {
    let settings = read_settings_from_disk().unwrap_or_default();
    sync_in_memory_state(&settings);
}

#[tauri::command]
pub async fn load_settings() -> Result<AppSettings, String> {
    let path = get_settings_path()?;

    let mut settings = if !path.exists() {
        println!("No settings file found, returning defaults");
        AppSettings::default()
    } else {
        let json = std::fs::read_to_string(&path)
            .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;
        serde_json::from_str(&json).map_err(|e| DiagError::serialization(e.to_string()))?
    };

    // Sync loaded settings to in-memory state
    sync_in_memory_state(&settings);

    // Hydrate API keys from secure storage into the response
    settings.open_ai_api_key = load_provider_key_internal(ProviderKeyId::OpenAI).await;
    settings.anthropic_api_key = load_provider_key_internal(ProviderKeyId::Anthropic).await;
    settings.gemini_api_key = load_provider_key_internal(ProviderKeyId::Gemini).await;
    settings.deepseek_api_key = load_provider_key_internal(ProviderKeyId::DeepSeek).await;
    settings.custom_api_key = load_provider_key_internal(ProviderKeyId::Custom).await;

    println!("Settings loaded from {:?}", path);
    Ok(settings)
}

/// Store an API key for a specific provider (openai, anthropic, gemini,
/// custom_openai). An empty key clears the stored value.
#[tauri::command]
pub async fn store_provider_api_key(provider: String, key: String) -> Result<(), String> {
    persist_provider_key(ProviderKeyId::parse(&provider)?, Some(&key))
}

/// Clear the stored API key for a specific provider.
#[tauri::command]
pub async fn clear_provider_api_key(provider: String) -> Result<(), String> {
    persist_provider_key(ProviderKeyId::parse(&provider)?, Some(""))
}

/// Legacy single-key command — stores the OpenAI key.
#[tauri::command]
pub async fn store_api_key(key: String) -> Result<(), String> {
    persist_provider_key(ProviderKeyId::OpenAI, Some(&key))
}

#[tauri::command]
pub async fn load_api_key() -> Result<String, String> {
    load_api_key_internal()
        .await
        .ok_or_else(|| DiagError::api_key("load", "No API key set").into())
}

/// Internal function for loading the OpenAI API key (used by ai_service)
pub async fn load_api_key_internal() -> Option<String> {
    load_provider_key_internal(ProviderKeyId::OpenAI).await
}

/// Legacy single-key command — clears the OpenAI key.
#[tauri::command]
pub async fn clear_api_key() -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::dpapi::clear_api_key()
    }
    #[cfg(not(windows))]
    {
        let entry = provider_keyring_entry(ProviderKeyId::OpenAI)?;
        match entry.delete_credential() {
            Ok(_) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(DiagError::api_key("clear", e.to_string()).into()),
        }
    }
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
            anthropic_model: Some("claude-sonnet-4-6".into()),
            custom_endpoint: Some("https://openrouter.ai/api".into()),
            custom_model: Some("meta-llama/llama-4".into()),
            ollama_endpoint: Some("http://127.0.0.1:11434".into()),
            ollama_model: Some("llama3.2".into()),
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
        assert_eq!(
            on_disk.anthropic_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            on_disk.custom_endpoint.as_deref(),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(on_disk.ollama_model.as_deref(), Some("llama3.2"));
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
            "\"anthropicApiKey\"",
            "\"geminiApiKey\"",
            "\"customApiKey\"",
            "\"deepseekApiKey\"",
            "\"anthropicModel\"",
            "\"geminiModel\"",
            "\"customEndpoint\"",
            "\"customModel\"",
            "\"ollamaEndpoint\"",
            "\"ollamaModel\"",
            "\"preferredAIProvider\"",
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
    }

    #[test]
    fn old_settings_files_still_load() {
        // A pre-2.5.0 settings file (no provider fields) must deserialize
        let old = r#"{"autoSave":true,"theme":"dark","preferredAIProvider":"auto"}"#;
        let s: AppSettings = serde_json::from_str(old).unwrap();
        assert!(s.auto_save);
        assert!(s.anthropic_api_key.is_none());
        assert!(s.ollama_endpoint.is_none());
    }
}
