//! UI-framework-neutral settings and provider-credential operations.
//!
#![allow(clippy::missing_errors_doc)]

//! The shipping Tauri shell and the native `WinUI` shell share this typed
//! contract. Platform storage remains injectable: callers retain ownership of
//! the existing settings path, atomic writer, and DPAPI/keyring implementation.
//! Secret values are write-only inputs and are never serialized or returned by
//! [`SettingsService::load`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, mpsc};
use std::thread;

mod persistence;

pub use persistence::{
    DPAPI_ADDITIONAL_ENTROPY, ShippingSettingsStorage, WindowsDpapiCredentialStorage,
    atomic_write_file, credential_path_from_local_data_dir, settings_path_from_config_dir,
    shipping_credential_path, shipping_settings_path,
};

/// Whether automatic local-provider failure may cross the network boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloudFallbackPolicy {
    #[default]
    Ask,
    Allow,
    Never,
}

/// Canonical persisted settings schema shared by every `WFDiag` UI shell.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    #[serde(default = "default_true")]
    pub ai_enabled: bool,
    #[serde(default = "default_ai_provider", rename = "preferredAIProvider")]
    pub preferred_ai_provider: String,
    #[serde(default)]
    pub network_grounding_enabled: bool,
    #[serde(default)]
    pub cloud_fallback_policy: CloudFallbackPolicy,
    #[serde(default, skip_serializing)]
    pub open_ai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_ai_model: Option<String>,
    #[serde(default, skip_serializing)]
    pub anthropic_api_key: Option<String>,
    #[serde(default, skip_serializing)]
    pub gemini_api_key: Option<String>,
    #[serde(default, skip_serializing)]
    pub deepseek_api_key: Option<String>,
    #[serde(default, skip_serializing)]
    pub custom_api_key: Option<String>,
    #[serde(default, skip_deserializing)]
    pub open_ai_api_key_set: bool,
    #[serde(default, skip_deserializing)]
    pub anthropic_api_key_set: bool,
    #[serde(default, skip_deserializing)]
    pub gemini_api_key_set: bool,
    #[serde(default, skip_deserializing)]
    pub deepseek_api_key_set: bool,
    #[serde(default, skip_deserializing)]
    pub custom_api_key_set: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_scan_tasks: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_ai_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_ai_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phi_silica_laf_token: Option<String>,
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
            network_grounding_enabled: false,
            cloud_fallback_policy: CloudFallbackPolicy::Ask,
            open_ai_api_key: None,
            open_ai_model: None,
            anthropic_api_key: None,
            gemini_api_key: None,
            deepseek_api_key: None,
            custom_api_key: None,
            open_ai_api_key_set: false,
            anthropic_api_key_set: false,
            gemini_api_key_set: false,
            deepseek_api_key_set: false,
            custom_api_key_set: false,
            anthropic_model: None,
            gemini_model: None,
            deepseek_model: None,
            custom_endpoint: None,
            custom_model: None,
            ollama_endpoint: None,
            ollama_model: None,
            codex_cli_path: None,
            codex_model: None,
            claude_cli_path: None,
            claude_model: None,
            quick_scan_tasks: None,
            local_ai_endpoint: None,
            local_ai_model: None,
            phi_silica_laf_token: None,
            close_to_tray: false,
        }
    }
}

const fn default_max_concurrent() -> u32 {
    5
}
fn default_export_format() -> String {
    "text".to_string()
}
fn default_theme() -> String {
    "dark".to_string()
}
const fn default_true() -> bool {
    true
}
const fn default_history_limit() -> u32 {
    30
}
fn default_ai_provider() -> String {
    "auto".to_string()
}

impl AppSettings {
    /// Return the only settings shape permitted to reach `settings.json`.
    #[must_use]
    pub fn for_disk(&self) -> Self {
        let mut settings = self.clone();
        settings.open_ai_api_key = None;
        settings.anthropic_api_key = None;
        settings.gemini_api_key = None;
        settings.deepseek_api_key = None;
        settings.custom_api_key = None;
        settings.open_ai_api_key_set = false;
        settings.anthropic_api_key_set = false;
        settings.gemini_api_key_set = false;
        settings.deepseek_api_key_set = false;
        settings.custom_api_key_set = false;
        settings
    }
}

/// Closed provider set; caller strings can never become filenames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKeyId {
    OpenAI,
    Anthropic,
    Gemini,
    DeepSeek,
    Custom,
}

impl ProviderKeyId {
    pub const ALL: [Self; 5] = [
        Self::OpenAI,
        Self::Anthropic,
        Self::Gemini,
        Self::DeepSeek,
        Self::Custom,
    ];

    pub fn parse(provider: &str) -> Result<Self, SettingsError> {
        match provider {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "deepseek" => Ok(Self::DeepSeek),
            "custom_openai" | "custom" => Ok(Self::Custom),
            other => Err(SettingsError::Credential(format!(
                "Unknown API key provider '{other}'"
            ))),
        }
    }

    #[must_use]
    pub const fn credential_filename(self) -> &'static str {
        match self {
            Self::OpenAI => "credentials.bin",
            Self::Anthropic => "credentials_anthropic.bin",
            Self::Gemini => "credentials_gemini.bin",
            Self::DeepSeek => "credentials_deepseek.bin",
            Self::Custom => "credentials_custom.bin",
        }
    }

    #[must_use]
    pub const fn keyring_user(self) -> &'static str {
        match self {
            Self::OpenAI => "openai_api_key",
            Self::Anthropic => "anthropic_api_key",
            Self::Gemini => "gemini_api_key",
            Self::DeepSeek => "deepseek_api_key",
            Self::Custom => "custom_api_key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    Storage(String),
    Serialization(String),
    Credential(String),
    Validation(String),
    Runtime(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, detail) = match self {
            Self::Storage(detail) => ("settings storage", detail),
            Self::Serialization(detail) => ("settings serialization", detail),
            Self::Credential(detail) => ("credential storage", detail),
            Self::Validation(detail) => ("settings validation", detail),
            Self::Runtime(detail) => ("settings runtime", detail),
        };
        write!(formatter, "{kind}: {detail}")
    }
}

impl std::error::Error for SettingsError {}

pub trait SettingsStorage: Send + Sync + 'static {
    fn load(&self) -> Result<Option<Vec<u8>>, SettingsError>;
    fn save(&self, serialized: &[u8]) -> Result<(), SettingsError>;
}

pub trait CredentialStorage: Send + Sync + 'static {
    fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError>;
    fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError>;
    fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError>;
}

pub trait SettingsValidator: Send + Sync + 'static {
    fn validate(&self, settings: &AppSettings) -> Result<(), SettingsError>;
}

#[derive(Default)]
pub struct AllowAllSettings;

impl SettingsValidator for AllowAllSettings {
    fn validate(&self, _settings: &AppSettings) -> Result<(), SettingsError> {
        Ok(())
    }
}

/// Typed mutation used by the native command runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsUpdate {
    AutoSave(bool),
    ScanOnStartup(bool),
    MaxConcurrentTasks(u32),
    ExportFormat(String),
    Theme(String),
    ShowNotifications(bool),
    CloseToTray(bool),
    PreferredAiProvider(String),
    NetworkGrounding(bool),
    CloudFallbackPolicy(CloudFallbackPolicy),
}

impl SettingsUpdate {
    fn apply(self, settings: &mut AppSettings) {
        match self {
            Self::AutoSave(value) => settings.auto_save = value,
            Self::ScanOnStartup(value) => settings.scan_on_startup = value,
            Self::MaxConcurrentTasks(value) => settings.max_concurrent_tasks = value,
            Self::ExportFormat(value) => settings.export_format = value,
            Self::Theme(value) => settings.theme = value,
            Self::ShowNotifications(value) => settings.show_notifications = value,
            Self::CloseToTray(value) => settings.close_to_tray = value,
            Self::PreferredAiProvider(value) => settings.preferred_ai_provider = value,
            Self::NetworkGrounding(value) => settings.network_grounding_enabled = value,
            Self::CloudFallbackPolicy(value) => settings.cloud_fallback_policy = value,
        }
    }
}

#[derive(Clone)]
pub struct SettingsService {
    settings: Arc<dyn SettingsStorage>,
    credentials: Arc<dyn CredentialStorage>,
    validator: Arc<dyn SettingsValidator>,
}

impl SettingsService {
    #[must_use]
    pub fn new(
        settings: Arc<dyn SettingsStorage>,
        credentials: Arc<dyn CredentialStorage>,
        validator: Arc<dyn SettingsValidator>,
    ) -> Self {
        Self {
            settings,
            credentials,
            validator,
        }
    }

    pub fn load(&self) -> Result<AppSettings, SettingsError> {
        let mut settings = self.load_nonsecret_settings()?;
        // Availability only: never hydrate plaintext secret fields.
        settings.open_ai_api_key_set = self.provider_key_is_set(ProviderKeyId::OpenAI)?;
        settings.anthropic_api_key_set = self.provider_key_is_set(ProviderKeyId::Anthropic)?;
        settings.gemini_api_key_set = self.provider_key_is_set(ProviderKeyId::Gemini)?;
        settings.deepseek_api_key_set = self.provider_key_is_set(ProviderKeyId::DeepSeek)?;
        settings.custom_api_key_set = self.provider_key_is_set(ProviderKeyId::Custom)?;
        Ok(settings)
    }

    /// Load only the non-secret persisted settings document.
    ///
    /// Provider status uses this separately from credential availability so
    /// a malformed settings file cannot hide otherwise valid DPAPI/keyring
    /// entries. That preserves the shipping 2.5.8 probe behavior.
    pub fn load_nonsecret_settings(&self) -> Result<AppSettings, SettingsError> {
        match self.settings.load()? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| SettingsError::Serialization(error.to_string())),
            None => Ok(AppSettings::default()),
        }
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        // Validation must precede every secret or file write.
        self.validator.validate(settings)?;
        self.persist_optional(ProviderKeyId::OpenAI, settings.open_ai_api_key.as_deref())?;
        self.persist_optional(
            ProviderKeyId::Anthropic,
            settings.anthropic_api_key.as_deref(),
        )?;
        self.persist_optional(ProviderKeyId::Gemini, settings.gemini_api_key.as_deref())?;
        self.persist_optional(
            ProviderKeyId::DeepSeek,
            settings.deepseek_api_key.as_deref(),
        )?;
        self.persist_optional(ProviderKeyId::Custom, settings.custom_api_key.as_deref())?;
        let serialized = serde_json::to_vec_pretty(&settings.for_disk())
            .map_err(|error| SettingsError::Serialization(error.to_string()))?;
        self.settings.save(&serialized)
    }

    pub fn update(&self, update: SettingsUpdate) -> Result<AppSettings, SettingsError> {
        let mut settings = self.load()?;
        update.apply(&mut settings);
        self.save(&settings)?;
        Ok(settings)
    }

    pub fn store_provider_key(
        &self,
        provider: ProviderKeyId,
        key: &str,
    ) -> Result<(), SettingsError> {
        if key.is_empty() {
            self.credentials.clear(provider)
        } else {
            self.credentials.store(provider, key)
        }
    }

    pub fn clear_provider_key(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
        self.credentials.clear(provider)
    }

    pub fn load_provider_key(
        &self,
        provider: ProviderKeyId,
    ) -> Result<Option<String>, SettingsError> {
        self.credentials.load(provider)
    }

    /// Return credential availability without exposing its value.
    pub fn provider_key_is_set(&self, provider: ProviderKeyId) -> Result<bool, SettingsError> {
        Ok(self
            .credentials
            .load(provider)?
            .is_some_and(|value| !value.is_empty()))
    }

    fn persist_optional(
        &self,
        provider: ProviderKeyId,
        value: Option<&str>,
    ) -> Result<(), SettingsError> {
        if let Some(value) = value {
            self.store_provider_key(provider, value)?;
        }
        Ok(())
    }
}

/// Construct the shipping Windows persistence service without importing a UI
/// framework. The caller supplies provider-policy validation because package
/// identity and AI-provider admission remain application concerns.
#[cfg(windows)]
#[must_use]
pub fn windows_shipping_settings_service(validator: Arc<dyn SettingsValidator>) -> SettingsService {
    SettingsService::new(
        Arc::new(ShippingSettingsStorage::new()),
        Arc::new(WindowsDpapiCredentialStorage::new()),
        validator,
    )
}

#[derive(Debug, Clone)]
pub enum SettingsCommand {
    Load {
        request_id: u64,
    },
    Save {
        request_id: u64,
        settings: Box<AppSettings>,
    },
    Update {
        request_id: u64,
        update: SettingsUpdate,
    },
    StoreProviderKey {
        request_id: u64,
        provider: ProviderKeyId,
        key: String,
    },
    ClearProviderKey {
        request_id: u64,
        provider: ProviderKeyId,
    },
    Stop,
}

#[derive(Debug, Clone)]
pub enum SettingsEvent {
    Loaded {
        request_id: u64,
        result: Result<AppSettings, SettingsError>,
    },
    Saved {
        request_id: u64,
        result: Result<(), SettingsError>,
    },
    Updated {
        request_id: u64,
        result: Result<AppSettings, SettingsError>,
    },
    ProviderKeyStored {
        request_id: u64,
        result: Result<(), SettingsError>,
    },
    ProviderKeyCleared {
        request_id: u64,
        result: Result<(), SettingsError>,
    },
    Stopped,
}

/// Background command worker. `WinUI` can send commands without blocking its
/// single UI thread and poll/bridge the returned event receiver.
pub struct SettingsRuntime {
    commands: mpsc::Sender<SettingsCommand>,
    worker: Option<thread::JoinHandle<()>>,
}

impl SettingsRuntime {
    pub fn start(
        service: SettingsService,
    ) -> Result<(Self, mpsc::Receiver<SettingsEvent>), SettingsError> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("wfdiag-native-settings".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    let event = match command {
                        SettingsCommand::Load { request_id } => SettingsEvent::Loaded {
                            request_id,
                            result: service.load(),
                        },
                        SettingsCommand::Save {
                            request_id,
                            settings,
                        } => SettingsEvent::Saved {
                            request_id,
                            result: service.save(&settings),
                        },
                        SettingsCommand::Update { request_id, update } => SettingsEvent::Updated {
                            request_id,
                            result: service.update(update),
                        },
                        SettingsCommand::StoreProviderKey {
                            request_id,
                            provider,
                            key,
                        } => SettingsEvent::ProviderKeyStored {
                            request_id,
                            result: service.store_provider_key(provider, &key),
                        },
                        SettingsCommand::ClearProviderKey {
                            request_id,
                            provider,
                        } => SettingsEvent::ProviderKeyCleared {
                            request_id,
                            result: service.clear_provider_key(provider),
                        },
                        SettingsCommand::Stop => {
                            let _ = event_tx.send(SettingsEvent::Stopped);
                            break;
                        }
                    };
                    if event_tx.send(event).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| SettingsError::Runtime(error.to_string()))?;
        Ok((
            Self {
                commands: command_tx,
                worker: Some(worker),
            },
            event_rx,
        ))
    }

    pub fn send(&self, command: SettingsCommand) -> Result<(), SettingsError> {
        self.commands
            .send(command)
            .map_err(|_| SettingsError::Runtime("settings worker stopped".to_string()))
    }
}

impl Drop for SettingsRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(SettingsCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct MemorySettings(Mutex<Option<Vec<u8>>>);

    impl SettingsStorage for MemorySettings {
        fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, serialized: &[u8]) -> Result<(), SettingsError> {
            *self.0.lock().unwrap() = Some(serialized.to_vec());
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryCredentials(Mutex<HashMap<ProviderKeyId, String>>);

    impl CredentialStorage for MemoryCredentials {
        fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
            self.0.lock().unwrap().insert(provider, key.to_string());
            Ok(())
        }

        fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
            Ok(self.0.lock().unwrap().get(&provider).cloned())
        }

        fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
            self.0.lock().unwrap().remove(&provider);
            Ok(())
        }
    }

    fn service() -> (SettingsService, Arc<MemorySettings>, Arc<MemoryCredentials>) {
        let settings = Arc::new(MemorySettings::default());
        let credentials = Arc::new(MemoryCredentials::default());
        (
            SettingsService::new(
                settings.clone(),
                credentials.clone(),
                Arc::new(AllowAllSettings),
            ),
            settings,
            credentials,
        )
    }

    #[test]
    fn save_routes_secrets_out_of_json_and_load_returns_flags_only() {
        let (service, settings_store, _) = service();
        let settings = AppSettings {
            open_ai_api_key: Some("sk-secret".to_string()),
            anthropic_api_key: Some("ant-secret".to_string()),
            ..AppSettings::default()
        };
        service.save(&settings).unwrap();
        let bytes = settings_store.0.lock().unwrap().clone().unwrap();
        let json = String::from_utf8(bytes).unwrap();
        assert!(!json.contains("sk-secret"));
        assert!(!json.contains("ant-secret"));
        // Preserve the shipping schema: availability fields remain present but
        // are always persisted as false and re-derived from secure storage.
        assert!(json.contains("\"openAiApiKeySet\": false"));

        let loaded = service.load().unwrap();
        assert!(loaded.open_ai_api_key.is_none());
        assert!(loaded.anthropic_api_key.is_none());
        assert!(loaded.open_ai_api_key_set);
        assert!(loaded.anthropic_api_key_set);
    }

    #[test]
    fn update_is_typed_and_preserves_unrelated_fields() {
        let (service, _, _) = service();
        let initial = AppSettings {
            custom_model: Some("existing-model".to_string()),
            ..AppSettings::default()
        };
        service.save(&initial).unwrap();
        let updated = service
            .update(SettingsUpdate::CloudFallbackPolicy(
                CloudFallbackPolicy::Never,
            ))
            .unwrap();
        assert_eq!(updated.cloud_fallback_policy, CloudFallbackPolicy::Never);
        assert_eq!(updated.custom_model.as_deref(), Some("existing-model"));
    }

    #[test]
    fn provider_ids_are_closed_and_keep_shipping_names() {
        assert_eq!(
            ProviderKeyId::parse("custom_openai"),
            Ok(ProviderKeyId::Custom)
        );
        assert!(ProviderKeyId::parse("../escape").is_err());
        assert_eq!(
            ProviderKeyId::OpenAI.credential_filename(),
            "credentials.bin"
        );
        assert_eq!(ProviderKeyId::OpenAI.keyring_user(), "openai_api_key");
    }

    struct RejectAll;
    impl SettingsValidator for RejectAll {
        fn validate(&self, _settings: &AppSettings) -> Result<(), SettingsError> {
            Err(SettingsError::Validation("rejected".to_string()))
        }
    }

    #[test]
    fn validation_happens_before_any_secret_or_settings_write() {
        let settings = Arc::new(MemorySettings::default());
        let credentials = Arc::new(MemoryCredentials::default());
        let service =
            SettingsService::new(settings.clone(), credentials.clone(), Arc::new(RejectAll));
        let candidate = AppSettings {
            open_ai_api_key: Some("must-not-write".to_string()),
            ..AppSettings::default()
        };
        assert!(service.save(&candidate).is_err());
        assert!(settings.0.lock().unwrap().is_none());
        assert!(credentials.0.lock().unwrap().is_empty());
    }

    #[test]
    fn runtime_executes_commands_off_caller_thread() {
        let (service, _, _) = service();
        let (runtime, events) = SettingsRuntime::start(service).unwrap();
        runtime
            .send(SettingsCommand::Update {
                request_id: 7,
                update: SettingsUpdate::Theme("light".to_string()),
            })
            .unwrap();
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            SettingsEvent::Updated { request_id, result } => {
                assert_eq!(request_id, 7);
                assert_eq!(result.unwrap().theme, "light");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        drop(runtime);
    }
}
