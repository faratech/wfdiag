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
use std::sync::{Arc, Mutex, mpsc};
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
    #[serde(default)]
    pub nav_rail_collapsed: bool,
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
            nav_rail_collapsed: false,
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
    /// Repair legacy or malformed non-secret enum strings at the persistence
    /// boundary. Older builds could write an empty export format while the UI
    /// still displayed its first item (Text); keeping that empty value in the
    /// runtime snapshot made the Export command fail despite the visible
    /// selection.
    pub fn normalize_persisted_values(&mut self) {
        self.export_format = match self.export_format.trim().to_ascii_lowercase().as_str() {
            "json" => "json",
            "html" => "html",
            _ => "text",
        }
        .to_string();
    }

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

    const fn index(self) -> usize {
        match self {
            Self::OpenAI => 0,
            Self::Anthropic => 1,
            Self::Gemini => 2,
            Self::DeepSeek => 3,
            Self::Custom => 4,
        }
    }
}

/// Non-secret projection of a staged provider-credential change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCredentialAction {
    Store,
    Clear,
}

#[derive(Clone)]
enum ProviderCredentialMutation {
    Store(String),
    Clear,
}

impl ProviderCredentialMutation {
    const fn action(&self) -> ProviderCredentialAction {
        match self {
            Self::Store(_) => ProviderCredentialAction::Store,
            Self::Clear => ProviderCredentialAction::Clear,
        }
    }
}

/// In-memory provider-credential edits for one settings-dialog lifetime.
///
/// Staging is side-effect free: credential storage is not read or written
/// until [`SettingsService::commit_provider_credentials`] is called. The
/// transaction deliberately has no serde implementation, and its `Debug`
/// representation exposes actions only, never secret values.
#[derive(Clone, Default)]
pub struct ProviderCredentialTransaction {
    mutations: [Option<ProviderCredentialMutation>; ProviderKeyId::ALL.len()],
}

impl ProviderCredentialTransaction {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a provider key without touching credential storage.
    ///
    /// An empty value preserves the existing immediate API's semantics and
    /// stages a clear operation.
    pub fn stage_store(&mut self, provider: ProviderKeyId, key: impl Into<String>) {
        let key = key.into();
        self.mutations[provider.index()] = Some(if key.is_empty() {
            ProviderCredentialMutation::Clear
        } else {
            ProviderCredentialMutation::Store(key)
        });
    }

    /// Stage removal of a provider key without touching credential storage.
    pub fn stage_clear(&mut self, provider: ProviderKeyId) {
        self.mutations[provider.index()] = Some(ProviderCredentialMutation::Clear);
    }

    /// Remove one pending edit, restoring the dialog's inherited state.
    pub fn unstage(&mut self, provider: ProviderKeyId) {
        self.mutations[provider.index()] = None;
    }

    /// Discard every pending edit. Dropping the transaction has the same
    /// storage semantics.
    pub fn discard(&mut self) {
        self.mutations = Default::default();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mutations.iter().all(Option::is_none)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.mutations.iter().flatten().count()
    }

    /// Inspect only whether a provider will be stored or cleared. Secret
    /// values remain write-only.
    #[must_use]
    pub fn staged_action(&self, provider: ProviderKeyId) -> Option<ProviderCredentialAction> {
        self.mutations[provider.index()]
            .as_ref()
            .map(ProviderCredentialMutation::action)
    }

    fn iter(&self) -> impl Iterator<Item = (ProviderKeyId, &ProviderCredentialMutation)> {
        ProviderKeyId::ALL.into_iter().filter_map(|provider| {
            self.mutations[provider.index()]
                .as_ref()
                .map(|mutation| (provider, mutation))
        })
    }
}

impl fmt::Debug for ProviderCredentialTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut staged = formatter.debug_map();
        for (provider, mutation) in self.iter() {
            staged.entry(&provider, &mutation.action());
        }
        staged.finish()
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

/// One credential that could not be restored after a failed commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredentialRollbackFailure {
    pub provider: ProviderKeyId,
    pub error: SettingsError,
}

/// Structured failure from an atomic provider-credential commit.
///
/// `Apply` means every attempted mutation was restored. `Rollback` means at
/// least one prior value could not be restored and callers should refresh
/// credential-availability state before allowing another edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCredentialTransactionError {
    Coordination {
        error: SettingsError,
    },
    Snapshot {
        provider: ProviderKeyId,
        error: SettingsError,
    },
    Apply {
        provider: ProviderKeyId,
        error: SettingsError,
    },
    Rollback {
        provider: ProviderKeyId,
        error: SettingsError,
        rollback_failures: Vec<ProviderCredentialRollbackFailure>,
    },
}

impl ProviderCredentialTransactionError {
    /// Whether the storage state is known to match its pre-commit snapshot.
    #[must_use]
    pub const fn storage_restored(&self) -> bool {
        !matches!(self, Self::Rollback { .. })
    }
}

impl fmt::Display for ProviderCredentialTransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordination { error } => {
                write!(formatter, "could not coordinate credential commit: {error}")
            }
            Self::Snapshot { provider, error } => {
                write!(
                    formatter,
                    "could not snapshot {provider:?} credential: {error}"
                )
            }
            Self::Apply { provider, error } => {
                write!(
                    formatter,
                    "could not update {provider:?} credential: {error}"
                )
            }
            Self::Rollback {
                provider,
                error,
                rollback_failures,
            } => write!(
                formatter,
                "could not update {provider:?} credential and failed to restore {} credential(s): {error}",
                rollback_failures.len()
            ),
        }
    }
}

impl std::error::Error for ProviderCredentialTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let error = match self {
            Self::Coordination { error }
            | Self::Snapshot { error, .. }
            | Self::Apply { error, .. }
            | Self::Rollback { error, .. } => error,
        };
        Some(error)
    }
}

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
    credential_transaction_lock: Arc<Mutex<()>>,
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
            credential_transaction_lock: Arc::new(Mutex::new(())),
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
        let mut settings = match self.settings.load()? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| SettingsError::Serialization(error.to_string())),
            None => Ok(AppSettings::default()),
        }?;
        settings.normalize_persisted_values();
        Ok(settings)
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        let mut normalized = settings.clone();
        normalized.normalize_persisted_values();
        // Validation must precede every secret or file write.
        self.validator.validate(&normalized)?;
        let mut credentials = ProviderCredentialTransaction::new();
        for (provider, value) in [
            (ProviderKeyId::OpenAI, normalized.open_ai_api_key.as_deref()),
            (
                ProviderKeyId::Anthropic,
                normalized.anthropic_api_key.as_deref(),
            ),
            (ProviderKeyId::Gemini, normalized.gemini_api_key.as_deref()),
            (
                ProviderKeyId::DeepSeek,
                normalized.deepseek_api_key.as_deref(),
            ),
            (ProviderKeyId::Custom, normalized.custom_api_key.as_deref()),
        ] {
            if let Some(value) = value {
                credentials.stage_store(provider, value);
            }
        }
        let snapshots = Self::commit_provider_credentials_snapshotting(self, &credentials)
            .map_err(|error| SettingsError::Credential(error.to_string()))?;
        let serialized = serde_json::to_vec_pretty(&normalized.for_disk())
            .map_err(|error| SettingsError::Serialization(error.to_string()))?;
        if let Err(error) = self.settings.save(&serialized) {
            // Compensate the already-committed credentials so storage stays
            // consistent with the unchanged settings file; without this, keys
            // could exist while their availability flags never persisted. The
            // save can simply be retried.
            let rollback_failures = self.rollback_provider_credentials(&snapshots);
            let _ = rollback_failures;
            return Err(error);
        }
        Ok(())
    }

    pub fn update(&self, update: SettingsUpdate) -> Result<AppSettings, SettingsError> {
        let mut settings = self.load()?;
        update.apply(&mut settings);
        settings.normalize_persisted_values();
        self.save(&settings)?;
        Ok(settings)
    }

    pub fn store_provider_key(
        &self,
        provider: ProviderKeyId,
        key: &str,
    ) -> Result<(), SettingsError> {
        let _guard = self.lock_provider_credentials()?;
        if key.is_empty() {
            self.credentials.clear(provider)
        } else {
            self.credentials.store(provider, key)
        }
    }

    pub fn clear_provider_key(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
        let _guard = self.lock_provider_credentials()?;
        self.credentials.clear(provider)
    }

    pub fn load_provider_key(
        &self,
        provider: ProviderKeyId,
    ) -> Result<Option<String>, SettingsError> {
        let _guard = self.lock_provider_credentials()?;
        self.credentials.load(provider)
    }

    /// Return credential availability without exposing its value.
    pub fn provider_key_is_set(&self, provider: ProviderKeyId) -> Result<bool, SettingsError> {
        Ok(self
            .load_provider_key(provider)?
            .is_some_and(|value| !value.is_empty()))
    }

    /// Apply every staged provider-key edit as one compensating transaction.
    ///
    /// All prior values are loaded before the first write. If an apply step
    /// fails, every attempted provider (including the failing step, which may
    /// have partially mutated an injected store) is restored in reverse order.
    /// A structured `Rollback` error makes the only non-atomic outcome
    /// explicit when the underlying storage also fails during compensation.
    pub fn commit_provider_credentials(
        &self,
        transaction: &ProviderCredentialTransaction,
    ) -> Result<(), ProviderCredentialTransactionError> {
        Self::commit_provider_credentials_snapshotting(self, transaction).map(|_| ())
    }

    /// Like [`Self::commit_provider_credentials`], but returns the pre-mutation
    /// snapshots so a caller with a later failure step can compensate (see
    /// `SettingsService::save`, which must not leave committed keys behind a
    /// settings file that was never updated).
    fn commit_provider_credentials_snapshotting(
        &self,
        transaction: &ProviderCredentialTransaction,
    ) -> Result<Vec<(ProviderKeyId, Option<String>)>, ProviderCredentialTransactionError> {
        if transaction.is_empty() {
            return Ok(Vec::new());
        }

        let _guard = self.credential_transaction_lock.lock().map_err(|_| {
            ProviderCredentialTransactionError::Coordination {
                error: SettingsError::Runtime(
                    "provider credential transaction lock is unavailable".to_string(),
                ),
            }
        })?;
        let mut snapshots = Vec::with_capacity(transaction.len());
        for (provider, _) in transaction.iter() {
            let value = self.credentials.load(provider).map_err(|error| {
                ProviderCredentialTransactionError::Snapshot { provider, error }
            })?;
            snapshots.push((provider, value));
        }

        for (index, (provider, mutation)) in transaction.iter().enumerate() {
            let result = match mutation {
                ProviderCredentialMutation::Store(key) => self.credentials.store(provider, key),
                ProviderCredentialMutation::Clear => self.credentials.clear(provider),
            };
            if let Err(error) = result {
                let rollback_failures = self.rollback_provider_credentials(&snapshots[..=index]);
                return Err(if rollback_failures.is_empty() {
                    ProviderCredentialTransactionError::Apply { provider, error }
                } else {
                    ProviderCredentialTransactionError::Rollback {
                        provider,
                        error,
                        rollback_failures,
                    }
                });
            }
        }

        Ok(snapshots)
    }

    fn lock_provider_credentials(&self) -> Result<std::sync::MutexGuard<'_, ()>, SettingsError> {
        self.credential_transaction_lock.lock().map_err(|_| {
            SettingsError::Runtime(
                "provider credential transaction lock is unavailable".to_string(),
            )
        })
    }

    fn rollback_provider_credentials(
        &self,
        snapshots: &[(ProviderKeyId, Option<String>)],
    ) -> Vec<ProviderCredentialRollbackFailure> {
        snapshots
            .iter()
            .rev()
            .filter_map(|(provider, value)| {
                let result = value.as_ref().map_or_else(
                    || self.credentials.clear(*provider),
                    |value| self.credentials.store(*provider, value),
                );
                result.err().map(|error| ProviderCredentialRollbackFailure {
                    provider: *provider,
                    error,
                })
            })
            .collect()
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
    CommitProviderCredentials {
        request_id: u64,
        transaction: ProviderCredentialTransaction,
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
    ProviderCredentialsCommitted {
        request_id: u64,
        result: Result<(), ProviderCredentialTransactionError>,
    },
    Stopped,
}

/// Optional notification hook for native UIs that drain the event receiver
/// from their own dispatcher. The hook runs only after an event is accepted.
pub type SettingsWakeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

fn send_settings_event(
    events: &mpsc::Sender<SettingsEvent>,
    wake: Option<&SettingsWakeHandler>,
    event: SettingsEvent,
) -> bool {
    if events.send(event).is_err() {
        return false;
    }
    if let Some(wake) = wake {
        wake();
    }
    true
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
        Self::start_inner(service, None)
    }

    /// Start the settings worker with event-driven UI notification.
    ///
    /// Existing shells can keep using [`Self::start`]; native dispatchers use
    /// this form to avoid a permanent receiver-poll task while idle.
    pub fn start_with_wake(
        service: SettingsService,
        wake: SettingsWakeHandler,
    ) -> Result<(Self, mpsc::Receiver<SettingsEvent>), SettingsError> {
        Self::start_inner(service, Some(wake))
    }

    fn start_inner(
        service: SettingsService,
        wake: Option<SettingsWakeHandler>,
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
                        SettingsCommand::CommitProviderCredentials {
                            request_id,
                            transaction,
                        } => SettingsEvent::ProviderCredentialsCommitted {
                            request_id,
                            result: service.commit_provider_credentials(&transaction),
                        },
                        SettingsCommand::Stop => {
                            let _ = send_settings_event(
                                &event_tx,
                                wake.as_ref(),
                                SettingsEvent::Stopped,
                            );
                            break;
                        }
                    };
                    if !send_settings_event(&event_tx, wake.as_ref(), event) {
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

impl SettingsRuntime {
    /// Ask the worker to stop and wait at most `budget` for it to exit.
    ///
    /// Returns `true` when the worker exited within the budget. The worker
    /// loop is strict FIFO, so `Stop` queues behind any in-flight save or
    /// credential commit (DPAPI + write-through file I/O); a worker that is
    /// still busy when the budget elapses is reaped on a detached thread
    /// rather than stalling the caller. Hosts that need ordered teardown call
    /// this explicitly; `Drop` performs the same stop without waiting.
    pub fn stop_and_join(&mut self, budget: std::time::Duration) -> bool {
        let _ = self.commands.send(SettingsCommand::Stop);
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (done, finished) = mpsc::channel();
        reap_worker(worker, Some(done));
        finished.recv_timeout(budget).is_ok()
    }
}

impl Drop for SettingsRuntime {
    fn drop(&mut self) {
        // Never join on the dropping thread: the runtime is owned by the UI
        // shell, and joining behind a queued DPAPI/file save froze the window
        // at close (#184). Mirror the detached reaper the other workers use.
        let _ = self.commands.send(SettingsCommand::Stop);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker, None);
        }
    }
}

/// Join a stopped worker on a detached thread so the caller never blocks.
/// `done` (when supplied) receives one message once the join completes.
fn reap_worker(worker: thread::JoinHandle<()>, done: Option<mpsc::Sender<()>>) {
    let spawned = thread::Builder::new()
        .name("wfdiag-settings-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
            if let Some(done) = done {
                let _ = done.send(());
            }
        });
    if spawned.is_err() {
        // Thread creation failed: the worker still exits on its own after
        // `Stop`; leaking the handle is the only non-blocking option left.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
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

    struct TextExportFormatValidator;

    impl SettingsValidator for TextExportFormatValidator {
        fn validate(&self, settings: &AppSettings) -> Result<(), SettingsError> {
            if settings.export_format == "text" {
                Ok(())
            } else {
                Err(SettingsError::Validation(format!(
                    "export format was not normalized: {}",
                    settings.export_format
                )))
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CredentialCall {
        Load(ProviderKeyId),
        Store(ProviderKeyId),
        Clear(ProviderKeyId),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FailureTiming {
        Before,
        After,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct InjectedFailure {
        call: CredentialCall,
        timing: FailureTiming,
    }

    #[derive(Default)]
    struct FaultInjectingCredentials {
        values: Mutex<HashMap<ProviderKeyId, String>>,
        calls: Mutex<Vec<CredentialCall>>,
        failures: Mutex<VecDeque<InjectedFailure>>,
    }

    impl FaultInjectingCredentials {
        fn with_values(values: &[(ProviderKeyId, &str)]) -> Self {
            Self {
                values: Mutex::new(
                    values
                        .iter()
                        .map(|(provider, value)| (*provider, (*value).to_string()))
                        .collect(),
                ),
                ..Self::default()
            }
        }

        fn fail(&self, call: CredentialCall, timing: FailureTiming) {
            self.failures
                .lock()
                .unwrap()
                .push_back(InjectedFailure { call, timing });
        }

        fn should_fail(&self, call: CredentialCall, timing: FailureTiming) -> bool {
            let mut failures = self.failures.lock().unwrap();
            if failures
                .front()
                .is_some_and(|failure| failure.call == call && failure.timing == timing)
            {
                failures.pop_front();
                true
            } else {
                false
            }
        }

        fn record(&self, call: CredentialCall) {
            self.calls.lock().unwrap().push(call);
        }

        fn injected_error() -> SettingsError {
            SettingsError::Credential("injected failure".to_string())
        }

        fn values(&self) -> HashMap<ProviderKeyId, String> {
            self.values.lock().unwrap().clone()
        }

        fn calls(&self) -> Vec<CredentialCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CredentialStorage for FaultInjectingCredentials {
        fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
            let call = CredentialCall::Store(provider);
            self.record(call);
            if self.should_fail(call, FailureTiming::Before) {
                return Err(Self::injected_error());
            }
            self.values
                .lock()
                .unwrap()
                .insert(provider, key.to_string());
            if self.should_fail(call, FailureTiming::After) {
                Err(Self::injected_error())
            } else {
                Ok(())
            }
        }

        fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
            let call = CredentialCall::Load(provider);
            self.record(call);
            if self.should_fail(call, FailureTiming::Before) {
                return Err(Self::injected_error());
            }
            let value = self.values.lock().unwrap().get(&provider).cloned();
            if self.should_fail(call, FailureTiming::After) {
                Err(Self::injected_error())
            } else {
                Ok(value)
            }
        }

        fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
            let call = CredentialCall::Clear(provider);
            self.record(call);
            if self.should_fail(call, FailureTiming::Before) {
                return Err(Self::injected_error());
            }
            self.values.lock().unwrap().remove(&provider);
            if self.should_fail(call, FailureTiming::After) {
                Err(Self::injected_error())
            } else {
                Ok(())
            }
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

    fn fault_service(
        credentials: Arc<FaultInjectingCredentials>,
    ) -> (SettingsService, Arc<MemorySettings>) {
        let settings = Arc::new(MemorySettings::default());
        (
            SettingsService::new(settings.clone(), credentials, Arc::new(AllowAllSettings)),
            settings,
        )
    }

    #[test]
    fn nav_rail_collapsed_defaults_false_for_new_and_legacy_settings() {
        assert!(!AppSettings::default().nav_rail_collapsed);

        let empty_legacy: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!empty_legacy.nav_rail_collapsed);

        let populated_legacy: AppSettings = serde_json::from_str(
            r#"{
                "theme": "light",
                "closeToTray": true,
                "preferredAIProvider": "openai"
            }"#,
        )
        .unwrap();
        assert!(!populated_legacy.nav_rail_collapsed);
        assert_eq!(populated_legacy.theme, "light");
        assert!(populated_legacy.close_to_tray);
        assert_eq!(populated_legacy.preferred_ai_provider, "openai");
    }

    #[test]
    fn nav_rail_collapsed_serializes_with_canonical_camel_case_and_round_trips() {
        let collapsed = AppSettings {
            nav_rail_collapsed: true,
            ..AppSettings::default()
        };

        let serialized = serde_json::to_value(&collapsed).unwrap();
        assert_eq!(serialized["navRailCollapsed"], true);
        assert!(serialized.get("nav_rail_collapsed").is_none());
        assert_eq!(
            serde_json::from_value::<AppSettings>(serialized).unwrap(),
            collapsed
        );

        let expanded = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(expanded["navRailCollapsed"], false);
    }

    #[test]
    fn legacy_empty_or_txt_export_format_loads_as_visible_text_selection() {
        for (persisted, expected) in [
            (r#"{"exportFormat":""}"#, "text"),
            (r#"{"exportFormat":"TXT"}"#, "text"),
            (r#"{"exportFormat":" JSON "}"#, "json"),
            (r#"{"exportFormat":"unsupported"}"#, "text"),
        ] {
            let settings_store = Arc::new(MemorySettings(Mutex::new(Some(
                persisted.as_bytes().to_vec(),
            ))));
            let service = SettingsService::new(
                settings_store,
                Arc::new(MemoryCredentials::default()),
                Arc::new(AllowAllSettings),
            );
            assert_eq!(
                service.load_nonsecret_settings().unwrap().export_format,
                expected,
                "persisted value: {persisted}"
            );
        }
    }

    #[test]
    fn save_normalizes_empty_and_invalid_export_formats_before_validation_and_persistence() {
        for raw in ["", "unsupported"] {
            let settings_store = Arc::new(MemorySettings::default());
            let service = SettingsService::new(
                settings_store.clone(),
                Arc::new(MemoryCredentials::default()),
                Arc::new(TextExportFormatValidator),
            );
            let settings = AppSettings {
                export_format: raw.to_string(),
                ..AppSettings::default()
            };

            service.save(&settings).unwrap();

            let bytes = settings_store.0.lock().unwrap().clone().unwrap();
            let persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                persisted["exportFormat"].as_str(),
                Some("text"),
                "input: {raw:?}"
            );
        }
    }

    #[test]
    fn update_returns_and_persists_text_for_empty_and_invalid_export_formats() {
        for raw in ["", "unsupported"] {
            let (service, settings_store, _) = service();

            let updated = service
                .update(SettingsUpdate::ExportFormat(raw.to_string()))
                .unwrap();

            assert_eq!(updated.export_format, "text", "input: {raw:?}");
            let bytes = settings_store.0.lock().unwrap().clone().unwrap();
            let persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                persisted["exportFormat"].as_str(),
                Some("text"),
                "input: {raw:?}"
            );
        }
    }

    #[test]
    fn nav_rail_collapsed_survives_disk_projection_and_service_persistence() {
        let (service, settings_store, _) = service();
        let settings = AppSettings {
            nav_rail_collapsed: true,
            open_ai_api_key: Some("secret-not-json".to_string()),
            ..AppSettings::default()
        };

        let disk = settings.for_disk();
        assert!(disk.nav_rail_collapsed);
        assert!(disk.open_ai_api_key.is_none());
        service.save(&settings).unwrap();

        let persisted =
            String::from_utf8(settings_store.0.lock().unwrap().clone().unwrap()).unwrap();
        assert!(persisted.contains(r#""navRailCollapsed": true"#));
        assert!(!persisted.contains("secret-not-json"));
        assert!(service.load().unwrap().nav_rail_collapsed);
    }

    #[test]
    fn nav_rail_collapsed_rejects_malformed_values_instead_of_changing_the_default() {
        for malformed in [
            r#"{"navRailCollapsed":"true"}"#,
            r#"{"navRailCollapsed":1}"#,
            r#"{"navRailCollapsed":null}"#,
            r#"{"navRailCollapsed":{}}"#,
        ] {
            assert!(
                serde_json::from_str::<AppSettings>(malformed).is_err(),
                "accepted malformed settings: {malformed}"
            );
        }

        let settings_store = Arc::new(MemorySettings(Mutex::new(Some(
            br#"{"navRailCollapsed":"collapsed"}"#.to_vec(),
        ))));
        let service = SettingsService::new(
            settings_store,
            Arc::new(MemoryCredentials::default()),
            Arc::new(AllowAllSettings),
        );
        assert!(matches!(
            service.load_nonsecret_settings(),
            Err(SettingsError::Serialization(_))
        ));
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
    fn credential_transaction_staging_and_discard_never_touch_storage_or_reveal_secrets() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[(
            ProviderKeyId::OpenAI,
            "old-openai",
        )]));
        let (service, settings) = fault_service(credentials.clone());
        let mut transaction = ProviderCredentialTransaction::new();

        transaction.stage_store(ProviderKeyId::OpenAI, "new-secret-never-log");
        transaction.stage_clear(ProviderKeyId::Anthropic);
        assert_eq!(transaction.len(), 2);
        assert_eq!(
            transaction.staged_action(ProviderKeyId::OpenAI),
            Some(ProviderCredentialAction::Store)
        );
        assert_eq!(
            transaction.staged_action(ProviderKeyId::Anthropic),
            Some(ProviderCredentialAction::Clear)
        );
        assert!(!format!("{transaction:?}").contains("new-secret-never-log"));
        assert!(credentials.calls().is_empty());
        assert!(settings.0.lock().unwrap().is_none());

        transaction.unstage(ProviderKeyId::Anthropic);
        transaction.discard();
        assert!(transaction.is_empty());
        service.commit_provider_credentials(&transaction).unwrap();
        assert!(credentials.calls().is_empty());
        assert_eq!(
            credentials
                .values()
                .get(&ProviderKeyId::OpenAI)
                .map(String::as_str),
            Some("old-openai")
        );
    }

    #[test]
    fn credential_transaction_commits_set_and_clear_in_fixed_order_without_settings_json() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[
            (ProviderKeyId::OpenAI, "old-openai"),
            (ProviderKeyId::Anthropic, "old-anthropic"),
            (ProviderKeyId::Custom, "untouched-custom"),
        ]));
        let (service, settings) = fault_service(credentials.clone());
        let mut transaction = ProviderCredentialTransaction::new();
        transaction.stage_clear(ProviderKeyId::Anthropic);
        transaction.stage_store(ProviderKeyId::OpenAI, "new-openai");

        service.commit_provider_credentials(&transaction).unwrap();

        let values = credentials.values();
        assert_eq!(
            values.get(&ProviderKeyId::OpenAI).map(String::as_str),
            Some("new-openai")
        );
        assert!(!values.contains_key(&ProviderKeyId::Anthropic));
        assert_eq!(
            values.get(&ProviderKeyId::Custom).map(String::as_str),
            Some("untouched-custom")
        );
        assert_eq!(
            credentials.calls(),
            vec![
                CredentialCall::Load(ProviderKeyId::OpenAI),
                CredentialCall::Load(ProviderKeyId::Anthropic),
                CredentialCall::Store(ProviderKeyId::OpenAI),
                CredentialCall::Clear(ProviderKeyId::Anthropic),
            ]
        );
        assert!(settings.0.lock().unwrap().is_none());
    }

    #[test]
    fn credential_transaction_snapshot_failure_performs_no_writes() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[
            (ProviderKeyId::OpenAI, "old-openai"),
            (ProviderKeyId::Anthropic, "old-anthropic"),
        ]));
        credentials.fail(
            CredentialCall::Load(ProviderKeyId::Anthropic),
            FailureTiming::Before,
        );
        let (service, _) = fault_service(credentials.clone());
        let mut transaction = ProviderCredentialTransaction::new();
        transaction.stage_store(ProviderKeyId::OpenAI, "new-openai");
        transaction.stage_store(ProviderKeyId::Anthropic, "new-anthropic");

        let error = service
            .commit_provider_credentials(&transaction)
            .unwrap_err();

        assert!(matches!(
            error,
            ProviderCredentialTransactionError::Snapshot {
                provider: ProviderKeyId::Anthropic,
                ..
            }
        ));
        assert!(error.storage_restored());
        assert_eq!(
            credentials.calls(),
            vec![
                CredentialCall::Load(ProviderKeyId::OpenAI),
                CredentialCall::Load(ProviderKeyId::Anthropic),
            ]
        );
        let values = credentials.values();
        assert_eq!(
            values.get(&ProviderKeyId::OpenAI).map(String::as_str),
            Some("old-openai")
        );
        assert_eq!(
            values.get(&ProviderKeyId::Anthropic).map(String::as_str),
            Some("old-anthropic")
        );
    }

    #[test]
    fn credential_transaction_rolls_back_a_partial_apply_in_reverse_order() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[
            (ProviderKeyId::OpenAI, "old-openai"),
            (ProviderKeyId::Anthropic, "old-anthropic"),
        ]));
        credentials.fail(
            CredentialCall::Store(ProviderKeyId::Anthropic),
            FailureTiming::After,
        );
        let (service, _) = fault_service(credentials.clone());
        let mut transaction = ProviderCredentialTransaction::new();
        transaction.stage_store(ProviderKeyId::OpenAI, "new-openai-never-log");
        transaction.stage_store(ProviderKeyId::Anthropic, "new-anthropic-never-log");

        let error = service
            .commit_provider_credentials(&transaction)
            .unwrap_err();

        assert!(matches!(
            error,
            ProviderCredentialTransactionError::Apply {
                provider: ProviderKeyId::Anthropic,
                ..
            }
        ));
        assert!(error.storage_restored());
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("new-openai-never-log"));
        assert!(!rendered.contains("new-anthropic-never-log"));
        let values = credentials.values();
        assert_eq!(
            values.get(&ProviderKeyId::OpenAI).map(String::as_str),
            Some("old-openai")
        );
        assert_eq!(
            values.get(&ProviderKeyId::Anthropic).map(String::as_str),
            Some("old-anthropic")
        );
        assert_eq!(
            credentials.calls(),
            vec![
                CredentialCall::Load(ProviderKeyId::OpenAI),
                CredentialCall::Load(ProviderKeyId::Anthropic),
                CredentialCall::Store(ProviderKeyId::OpenAI),
                CredentialCall::Store(ProviderKeyId::Anthropic),
                CredentialCall::Store(ProviderKeyId::Anthropic),
                CredentialCall::Store(ProviderKeyId::OpenAI),
            ]
        );
    }

    #[test]
    fn credential_transaction_reports_incomplete_rollback_and_keeps_compensating() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[
            (ProviderKeyId::OpenAI, "old-openai"),
            (ProviderKeyId::Anthropic, "old-anthropic"),
        ]));
        credentials.fail(
            CredentialCall::Store(ProviderKeyId::Anthropic),
            FailureTiming::After,
        );
        credentials.fail(
            CredentialCall::Store(ProviderKeyId::Anthropic),
            FailureTiming::Before,
        );
        let (service, _) = fault_service(credentials.clone());
        let mut transaction = ProviderCredentialTransaction::new();
        transaction.stage_store(ProviderKeyId::OpenAI, "new-openai");
        transaction.stage_store(ProviderKeyId::Anthropic, "new-anthropic");

        let error = service
            .commit_provider_credentials(&transaction)
            .unwrap_err();

        let ProviderCredentialTransactionError::Rollback {
            provider,
            rollback_failures,
            ..
        } = &error
        else {
            panic!("expected rollback error, got {error:?}");
        };
        assert_eq!(*provider, ProviderKeyId::Anthropic);
        assert_eq!(rollback_failures.len(), 1);
        assert_eq!(rollback_failures[0].provider, ProviderKeyId::Anthropic);
        assert!(!error.storage_restored());
        let values = credentials.values();
        assert_eq!(
            values.get(&ProviderKeyId::OpenAI).map(String::as_str),
            Some("old-openai")
        );
        assert_eq!(
            values.get(&ProviderKeyId::Anthropic).map(String::as_str),
            Some("new-anthropic")
        );
        assert_eq!(
            credentials.calls().last(),
            Some(&CredentialCall::Store(ProviderKeyId::OpenAI))
        );
    }

    #[test]
    fn legacy_save_uses_atomic_credential_commit_before_writing_settings() {
        let credentials = Arc::new(FaultInjectingCredentials::with_values(&[
            (ProviderKeyId::OpenAI, "old-openai"),
            (ProviderKeyId::Anthropic, "old-anthropic"),
        ]));
        credentials.fail(
            CredentialCall::Store(ProviderKeyId::Anthropic),
            FailureTiming::After,
        );
        let (service, settings) = fault_service(credentials.clone());
        let candidate = AppSettings {
            open_ai_api_key: Some("new-openai".to_string()),
            anthropic_api_key: Some("new-anthropic".to_string()),
            ..AppSettings::default()
        };

        assert!(service.save(&candidate).is_err());

        assert!(settings.0.lock().unwrap().is_none());
        let values = credentials.values();
        assert_eq!(
            values.get(&ProviderKeyId::OpenAI).map(String::as_str),
            Some("old-openai")
        );
        assert_eq!(
            values.get(&ProviderKeyId::Anthropic).map(String::as_str),
            Some("old-anthropic")
        );
    }

    #[test]
    fn runtime_commits_a_redacted_credential_transaction_off_thread() {
        let (service, _, credentials) = service();
        let (runtime, events) = SettingsRuntime::start(service).unwrap();
        let mut transaction = ProviderCredentialTransaction::new();
        transaction.stage_store(ProviderKeyId::Gemini, "runtime-secret-never-log");
        let command = SettingsCommand::CommitProviderCredentials {
            request_id: 19,
            transaction,
        };
        assert!(!format!("{command:?}").contains("runtime-secret-never-log"));

        runtime.send(command).unwrap();

        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            SettingsEvent::ProviderCredentialsCommitted { request_id, result } => {
                assert_eq!(request_id, 19);
                result.unwrap();
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(
            credentials
                .0
                .lock()
                .unwrap()
                .get(&ProviderKeyId::Gemini)
                .map(String::as_str),
            Some("runtime-secret-never-log")
        );
        drop(runtime);
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

    #[test]
    fn event_driven_runtime_wakes_only_after_delivery() {
        let (service, _, _) = service();
        let (wake_tx, wake_rx) = mpsc::channel();
        let (runtime, events) = SettingsRuntime::start_with_wake(
            service,
            Arc::new(move || {
                let _ = wake_tx.send(());
            }),
        )
        .unwrap();

        runtime
            .send(SettingsCommand::Load { request_id: 23 })
            .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            SettingsEvent::Loaded { request_id: 23, .. }
        ));
        wake_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("accepted settings event wakes the native dispatcher");
        assert!(wake_rx.try_recv().is_err());
        drop(runtime);
    }

    /// Settings storage whose `save` parks until the test releases it, so a
    /// runtime can be torn down while its worker is genuinely busy (#184).
    struct StalledSettings {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl SettingsStorage for StalledSettings {
        fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
            Ok(None)
        }

        fn save(&self, _serialized: &[u8]) -> Result<(), SettingsError> {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
            Ok(())
        }
    }

    /// Starts a runtime whose worker is parked inside a `Save`; the returned
    /// sender releases it.
    fn stalled_runtime() -> (
        SettingsRuntime,
        mpsc::Receiver<SettingsEvent>,
        mpsc::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let storage = Arc::new(StalledSettings {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let service = SettingsService::new(
            storage,
            Arc::new(MemoryCredentials::default()),
            Arc::new(AllowAllSettings),
        );
        let (runtime, events) = SettingsRuntime::start(service).unwrap();
        runtime
            .send(SettingsCommand::Save {
                request_id: 1,
                settings: Box::new(AppSettings::default()),
            })
            .unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker entered the stalled save");
        (runtime, events, release_tx)
    }

    #[test]
    fn runtime_stop_and_join_reports_an_idle_worker_within_budget() {
        let (service, _, _) = service();
        let (mut runtime, _events) = SettingsRuntime::start(service).unwrap();
        assert!(runtime.stop_and_join(Duration::from_secs(2)));
        // Once the worker handle is gone a second call is a no-op.
        assert!(runtime.stop_and_join(Duration::from_millis(1)));
    }

    #[test]
    fn runtime_drop_never_blocks_behind_a_stalled_save() {
        let (runtime, events, release) = stalled_runtime();
        let started = std::time::Instant::now();
        drop(runtime);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop joined the stalled worker inline: {:?}",
            started.elapsed()
        );
        // Releasing the save lets the worker finish its queued work and stop
        // on its own; the completion still reaches the event channel.
        release.send(()).unwrap();
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            format!("{event:?}").contains("request_id: 1"),
            "unexpected event: {event:?}"
        );
    }

    #[test]
    fn runtime_stop_and_join_times_out_on_a_stalled_worker_then_reaps_it() {
        let (mut runtime, _events, release) = stalled_runtime();
        let started = std::time::Instant::now();
        assert!(!runtime.stop_and_join(Duration::from_millis(100)));
        assert!(started.elapsed() < Duration::from_millis(500));
        release.send(()).unwrap();
        // The worker handle has already been handed to the detached reaper.
        assert!(runtime.stop_and_join(Duration::from_millis(1)));
    }
}
