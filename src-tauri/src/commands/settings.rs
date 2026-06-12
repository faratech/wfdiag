//! Settings-related Tauri commands.
//!
//! This module handles application settings persistence and API key management.
//!
//! # Commands
//! - `save_settings` - Save application settings to disk
//! - `load_settings` - Load application settings from disk
//! - `store_api_key` - Store OpenAI API key securely
//! - `load_api_key` - Load OpenAI API key from secure storage
//! - `clear_api_key` - Remove stored API key
//!
//! # Security
//! API keys are stored using:
//! - Windows: DPAPI encryption
//! - Other platforms: System keyring

use crate::error::DiagError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(not(windows))]
use keyring::{Entry, Error as KeyringError};

/// Application settings that persist across sessions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
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
    // API key - stored in keyring, included in frontend response
    #[serde(default)]
    pub open_ai_api_key: Option<String>,
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

#[tauri::command]
pub async fn save_settings(settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path()?;

    // If API key is provided, store it securely (separate from settings file)
    if let Some(ref api_key) = settings.open_ai_api_key {
        #[cfg(windows)]
        {
            if !api_key.is_empty() {
                // Propagate (don't swallow) failures: otherwise save_settings returns Ok,
                // the UI believes the key persisted, and it silently vanishes on restart.
                crate::dpapi::store_api_key(api_key)
                    .map_err(|e| DiagError::api_key("store", e.to_string()))?;
                println!("API key stored with DPAPI");
            } else {
                // Empty string means clear the key
                let _ = crate::dpapi::clear_api_key();
                println!("API key cleared from DPAPI storage");
            }
        }
        #[cfg(not(windows))]
        {
            if !api_key.is_empty() {
                let entry = Entry::new("wfdiag-tauri", "openai_api_key")
                    .map_err(|e| DiagError::api_key("store", e.to_string()))?;
                entry
                    .set_password(api_key)
                    .map_err(|e| DiagError::api_key("store", e.to_string()))?;
                println!("API key stored in keyring");
            } else if let Ok(entry) = Entry::new("wfdiag-tauri", "openai_api_key") {
                let _ = entry.delete_credential();
                println!("API key cleared from keyring");
            }
        }
    }

    // Create a copy without the API key for file storage (security: don't write key to file)
    let mut settings_for_file = settings.clone();
    settings_for_file.open_ai_api_key = None;

    let json = serde_json::to_string_pretty(&settings_for_file)
        .map_err(|e| DiagError::serialization(e.to_string()))?;
    std::fs::write(&path, &json)
        .map_err(|e| DiagError::file(path.display().to_string(), e.to_string()))?;

    // Sync AI preference to in-memory state
    let pref = match settings.preferred_ai_provider.to_lowercase().as_str() {
        "openai" => crate::ai_service::AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => crate::ai_service::AIProviderPreference::PhiSilica,
        _ => crate::ai_service::AIProviderPreference::Auto,
    };
    crate::ai_service::set_user_preference(pref);

    println!("Settings saved to {:?}", path);
    Ok(())
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

    // Sync loaded AI preference to in-memory state
    let pref = match settings.preferred_ai_provider.to_lowercase().as_str() {
        "openai" => crate::ai_service::AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => crate::ai_service::AIProviderPreference::PhiSilica,
        _ => crate::ai_service::AIProviderPreference::Auto,
    };
    crate::ai_service::set_user_preference(pref);

    // Load API key from keyring and include in response
    if let Some(api_key) = load_api_key_internal().await {
        settings.open_ai_api_key = Some(api_key);
    }

    println!("Settings loaded from {:?}", path);
    Ok(settings)
}

#[tauri::command]
pub async fn store_api_key(key: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::dpapi::store_api_key(&key)
    }
    #[cfg(not(windows))]
    {
        // Fallback to keyring on non-Windows
        let entry = Entry::new("wfdiag-tauri", "openai_api_key")
            .map_err(|e| DiagError::api_key("access", e.to_string()))?;
        entry
            .set_password(&key)
            .map_err(|e| DiagError::api_key("store", e.to_string()))?;
        Ok(())
    }
}

#[tauri::command]
pub async fn load_api_key() -> Result<String, String> {
    load_api_key_internal()
        .await
        .ok_or_else(|| DiagError::api_key("load", "No API key set").into())
}

/// Internal function for loading API key (used by ai_service)
pub async fn load_api_key_internal() -> Option<String> {
    #[cfg(windows)]
    {
        crate::dpapi::load_api_key().ok().flatten()
    }
    #[cfg(not(windows))]
    {
        let entry = Entry::new("wfdiag-tauri", "openai_api_key").ok()?;
        match entry.get_password() {
            Ok(pwd) if !pwd.is_empty() => Some(pwd),
            _ => None,
        }
    }
}

#[tauri::command]
pub async fn clear_api_key() -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::dpapi::clear_api_key()
    }
    #[cfg(not(windows))]
    {
        let entry = Entry::new("wfdiag-tauri", "openai_api_key")
            .map_err(|e| DiagError::api_key("access", e.to_string()))?;
        match entry.delete_credential() {
            Ok(_) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(DiagError::api_key("clear", e.to_string()).into()),
        }
    }
}
