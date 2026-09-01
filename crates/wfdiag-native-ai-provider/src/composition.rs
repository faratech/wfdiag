use crate::{
    AIProviderPreference, BackendFuture, CliProbeSnapshot, ProviderManagementBackend,
    ProviderModelDefaults, ProviderProbeSnapshot, ProviderSettingsSnapshot, ProviderStatusInput,
    parse_and_validate_provider_preference, provider_preference_for_runtime,
};
use crate::{ProviderCacheControl, ReqwestOllamaSource, TcpCustomEndpointSource};
use std::fmt;
use std::sync::{Arc, Mutex};
use wfdiag_native_settings::{
    AppSettings, ProviderKeyId, SettingsError, SettingsService, SettingsValidator,
};

/// Non-secret provider configuration read from the canonical settings and
/// credential-availability seams.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfigurationSnapshot {
    pub preferred_provider: String,
    pub status: ProviderSettingsSnapshot,
    pub local_ai_endpoint: Option<String>,
    pub ollama_endpoint: Option<String>,
    pub codex_cli_path: Option<String>,
    pub claude_cli_path: Option<String>,
    pub openai_available: bool,
    pub anthropic_available: bool,
    pub gemini_available: bool,
    pub deepseek_available: bool,
}

impl ProviderConfigurationSnapshot {
    #[must_use]
    pub fn from_settings(settings: AppSettings) -> Self {
        Self {
            preferred_provider: settings.preferred_ai_provider,
            status: ProviderSettingsSnapshot {
                local_ai_model: settings.local_ai_model,
                ollama_model: settings.ollama_model,
                custom_endpoint: settings.custom_endpoint,
                custom_model: settings.custom_model,
                codex_model: settings.codex_model,
                claude_model: settings.claude_model,
                open_ai_model: settings.open_ai_model,
                anthropic_model: settings.anthropic_model,
                gemini_model: settings.gemini_model,
                deepseek_model: settings.deepseek_model,
            },
            local_ai_endpoint: settings.local_ai_endpoint,
            ollama_endpoint: settings.ollama_endpoint,
            codex_cli_path: settings.codex_cli_path,
            claude_cli_path: settings.claude_cli_path,
            openai_available: settings.open_ai_api_key_set,
            anthropic_available: settings.anthropic_api_key_set,
            gemini_available: settings.gemini_api_key_set,
            deepseek_available: settings.deepseek_api_key_set,
        }
    }
}

/// Read provider configuration without returning credential values.
pub trait ProviderConfigurationSource: Send + Sync + 'static {
    fn snapshot(&self) -> ProviderConfigurationSnapshot;
}

/// Shared adapter from the canonical settings service to provider status.
/// Settings corruption and individual credential-read errors are isolated in
/// the same way as the shipping 2.5.8 status command.
#[derive(Clone)]
pub struct SettingsServiceProviderConfigurationSource {
    service: SettingsService,
}

impl SettingsServiceProviderConfigurationSource {
    #[must_use]
    pub const fn new(service: SettingsService) -> Self {
        Self { service }
    }
}

impl ProviderConfigurationSource for SettingsServiceProviderConfigurationSource {
    fn snapshot(&self) -> ProviderConfigurationSnapshot {
        let mut settings = self.service.load_nonsecret_settings().unwrap_or_default();
        settings.open_ai_api_key_set = self
            .service
            .provider_key_is_set(ProviderKeyId::OpenAI)
            .unwrap_or(false);
        settings.anthropic_api_key_set = self
            .service
            .provider_key_is_set(ProviderKeyId::Anthropic)
            .unwrap_or(false);
        settings.gemini_api_key_set = self
            .service
            .provider_key_is_set(ProviderKeyId::Gemini)
            .unwrap_or(false);
        settings.deepseek_api_key_set = self
            .service
            .provider_key_is_set(ProviderKeyId::DeepSeek)
            .unwrap_or(false);
        ProviderConfigurationSnapshot::from_settings(settings)
    }
}

/// Package identity is injected because Store identity is process/package
/// composition, not a UI or provider-management concern.
pub trait PackageIdentitySource: Send + Sync + 'static {
    fn has_package_identity(&self) -> bool;
}

/// Settings validator that applies the same provider admission policy as the
/// runtime selection worker.
pub struct ProviderPreferenceSettingsValidator {
    identity: Arc<dyn PackageIdentitySource>,
}

impl ProviderPreferenceSettingsValidator {
    #[must_use]
    pub fn new(identity: Arc<dyn PackageIdentitySource>) -> Self {
        Self { identity }
    }
}

impl SettingsValidator for ProviderPreferenceSettingsValidator {
    fn validate(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        parse_and_validate_provider_preference(
            &settings.preferred_ai_provider,
            self.identity.has_package_identity(),
        )
        .map(|_| ())
        .map_err(SettingsError::Validation)
    }
}

/// Phi readiness projection. Activation and LAF handling remain in the
/// injected shipping backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhiStatusSnapshot {
    pub available: bool,
    pub ready: bool,
    pub message: Option<String>,
}

pub trait PhiStatusSource: Send + Sync + 'static {
    fn probe(&self) -> BackendFuture<'_, PhiStatusSnapshot>;
}

pub trait FoundryEndpointSource: Send + Sync + 'static {
    fn probe(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubscriptionCli {
    Codex,
    ClaudeCode,
}

pub trait SubscriptionCliStatusSource: Send + Sync + 'static {
    fn probe(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> BackendFuture<'_, CliProbeSnapshot>;
}

pub trait OllamaSource: Send + Sync + 'static {
    fn discover(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>>;
    fn list_models(&self, endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>>;
}

pub trait CustomEndpointSource: Send + Sync + 'static {
    fn probe(
        &self,
        endpoint: Option<String>,
        model: Option<String>,
    ) -> BackendFuture<'_, Option<String>>;
}

/// Explicit typed bundle of every provider-specific boundary.
///
/// Reactor can construct this without importing a `src-tauri` module. The
/// Shipping adapters cover Foundry, subscription CLIs, Ollama, and custom
/// endpoints; settings, identity, and Phi activation stay injected.
pub struct ProviderProbeBundle {
    pub configuration: Arc<dyn ProviderConfigurationSource>,
    pub identity: Arc<dyn PackageIdentitySource>,
    pub phi: Arc<dyn PhiStatusSource>,
    pub foundry: Arc<dyn FoundryEndpointSource>,
    pub cli: Arc<dyn SubscriptionCliStatusSource>,
    pub ollama: Arc<dyn OllamaSource>,
    pub custom: Arc<dyn CustomEndpointSource>,
}

impl ProviderProbeBundle {
    #[must_use]
    pub fn shipping_networks(
        configuration: Arc<dyn ProviderConfigurationSource>,
        identity: Arc<dyn PackageIdentitySource>,
        phi: Arc<dyn PhiStatusSource>,
        foundry: Arc<dyn FoundryEndpointSource>,
        cli: Arc<dyn SubscriptionCliStatusSource>,
    ) -> Self {
        Self {
            configuration,
            identity,
            phi,
            foundry,
            cli,
            ollama: Arc::new(ReqwestOllamaSource),
            custom: Arc::new(TcpCustomEndpointSource),
        }
    }
}

/// Cloneable process-level provider selection shared by request routing and
/// provider-management UI.
#[derive(Debug, Clone, Default)]
pub struct ProviderSelectionState {
    preference: Arc<Mutex<AIProviderPreference>>,
}

impl ProviderSelectionState {
    #[must_use]
    pub fn new(preference: AIProviderPreference) -> Self {
        Self {
            preference: Arc::new(Mutex::new(preference)),
        }
    }

    #[must_use]
    pub fn get(&self) -> AIProviderPreference {
        self.preference
            .lock()
            .map_or(AIProviderPreference::Auto, |preference| *preference)
    }

    pub fn set(&self, preference: AIProviderPreference) {
        if let Ok(mut current) = self.preference.lock() {
            *current = preference;
        }
    }

    /// Synchronize the persisted preference using the current package policy.
    pub fn sync_persisted(
        &self,
        preference: &str,
        identity: &dyn PackageIdentitySource,
    ) -> AIProviderPreference {
        let preference =
            provider_preference_for_runtime(preference, identity.has_package_identity());
        self.set(preference);
        preference
    }
}

/// Fully composed provider-management backend shared by Tauri and Reactor.
pub struct ProviderManagementService {
    probes: ProviderProbeBundle,
    selection: ProviderSelectionState,
    cache: Arc<dyn ProviderCacheControl>,
    defaults: ProviderModelDefaults,
}

impl fmt::Debug for ProviderManagementService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderManagementService")
            .field("selection", &self.selection)
            .field("defaults", &self.defaults)
            .finish_non_exhaustive()
    }
}

impl ProviderManagementService {
    #[must_use]
    pub fn new(
        probes: ProviderProbeBundle,
        selection: ProviderSelectionState,
        cache: Arc<dyn ProviderCacheControl>,
        defaults: ProviderModelDefaults,
    ) -> Self {
        Self {
            probes,
            selection,
            cache,
            defaults,
        }
    }

    #[must_use]
    pub const fn selection(&self) -> &ProviderSelectionState {
        &self.selection
    }
}

impl ProviderManagementBackend for ProviderManagementService {
    fn status_input(&self) -> BackendFuture<'_, ProviderStatusInput> {
        Box::pin(async move {
            // The settings read plus the per-provider DPAPI key checks are
            // blocking file/Win32 work; run them on the blocking pool so they
            // cannot stall the shared current-thread runtime while network
            // probes are in flight. Deliberately uncached: key-set flags must
            // reflect a just-completed credential save on the next probe.
            let configuration_source = std::sync::Arc::clone(&self.probes.configuration);
            let configuration = tokio::task::spawn_blocking(move || {
                configuration_source.snapshot()
            })
            .await
            .unwrap_or_default();
            let phi_probe = self.probes.phi.probe();
            let foundry_probe = self
                .probes
                .foundry
                .probe(configuration.local_ai_endpoint.clone());
            let ollama_probe = self
                .probes
                .ollama
                .discover(configuration.ollama_endpoint.clone());
            let custom_probe = self.probes.custom.probe(
                configuration.status.custom_endpoint.clone(),
                configuration.status.custom_model.clone(),
            );
            let codex_probe = self
                .probes
                .cli
                .probe(SubscriptionCli::Codex, configuration.codex_cli_path);
            let claude_probe = self
                .probes
                .cli
                .probe(SubscriptionCli::ClaudeCode, configuration.claude_cli_path);
            // Every source owns its shipping timeout. Poll them concurrently
            // so an unavailable CLI or local server cannot serialize all
            // provider discovery behind its individual deadline.
            let (phi, foundry_endpoint, ollama_endpoint, custom_endpoint, codex, claude) = tokio::join!(
                phi_probe,
                foundry_probe,
                ollama_probe,
                custom_probe,
                codex_probe,
                claude_probe,
            );
            ProviderStatusInput {
                preference: self.selection.get(),
                settings: configuration.status,
                probes: ProviderProbeSnapshot {
                    openai_available: configuration.openai_available,
                    phi_silica_available: phi.available,
                    phi_silica_ready: phi.ready,
                    phi_silica_message: phi.message,
                    foundry_endpoint,
                    ollama_endpoint,
                    custom_endpoint,
                    codex,
                    claude,
                    anthropic_available: configuration.anthropic_available,
                    gemini_available: configuration.gemini_available,
                    deepseek_available: configuration.deepseek_available,
                },
                defaults: self.defaults.clone(),
            }
        })
    }

    fn has_package_identity(&self) -> bool {
        self.probes.identity.has_package_identity()
    }

    fn set_preference(&self, preference: AIProviderPreference) {
        self.selection.set(preference);
    }

    fn clear_cache(&self, session_id: Option<&str>) {
        self.cache.clear(session_id);
    }

    fn list_ollama_models(&self) -> BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move {
            // The settings read plus the per-provider DPAPI key checks are
            // blocking file/Win32 work; run them on the blocking pool so they
            // cannot stall the shared current-thread runtime while network
            // probes are in flight. Deliberately uncached: key-set flags must
            // reflect a just-completed credential save on the next probe.
            let configuration_source = std::sync::Arc::clone(&self.probes.configuration);
            let configuration = tokio::task::spawn_blocking(move || {
                configuration_source.snapshot()
            })
            .await
            .unwrap_or_default();
            let endpoint = self
                .probes
                .ollama
                .discover(configuration.ollama_endpoint)
                .await
                .ok_or_else(|| {
                    "No Ollama server reachable. Install Ollama (https://ollama.com) and make sure it is running."
                        .to_string()
                })?;
            self.probes.ollama.list_models(endpoint).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AIProvider, SharedAiCache, project_provider_status};
    use std::sync::atomic::{AtomicBool, Ordering};
    use wfdiag_native_settings::{CredentialStorage, SettingsStorage};

    struct StaticConfiguration(ProviderConfigurationSnapshot);

    impl ProviderConfigurationSource for StaticConfiguration {
        fn snapshot(&self) -> ProviderConfigurationSnapshot {
            self.0.clone()
        }
    }

    struct Identity(bool);

    impl PackageIdentitySource for Identity {
        fn has_package_identity(&self) -> bool {
            self.0
        }
    }

    struct Phi;

    impl PhiStatusSource for Phi {
        fn probe(&self) -> BackendFuture<'_, PhiStatusSnapshot> {
            Box::pin(async {
                PhiStatusSnapshot {
                    available: true,
                    ready: false,
                    message: Some("Downloading".to_string()),
                }
            })
        }
    }

    struct Foundry;

    impl FoundryEndpointSource for Foundry {
        fn probe(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { configured })
        }
    }

    struct Cli;

    impl SubscriptionCliStatusSource for Cli {
        fn probe(
            &self,
            provider: SubscriptionCli,
            configured_path: Option<String>,
        ) -> BackendFuture<'_, CliProbeSnapshot> {
            Box::pin(async move {
                match provider {
                    SubscriptionCli::Codex => CliProbeSnapshot {
                        usable: true,
                        installed: true,
                        path: configured_path,
                    },
                    SubscriptionCli::ClaudeCode => CliProbeSnapshot::default(),
                }
            })
        }
    }

    struct Ollama;

    impl OllamaSource for Ollama {
        fn discover(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { configured })
        }

        fn list_models(&self, _endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async { Ok(vec!["llama3.2:latest".to_string()]) })
        }
    }

    struct Custom;

    impl CustomEndpointSource for Custom {
        fn probe(
            &self,
            endpoint: Option<String>,
            model: Option<String>,
        ) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { model.and(endpoint) })
        }
    }

    fn defaults() -> ProviderModelDefaults {
        ProviderModelDefaults {
            foundry: "phi-4-mini".to_string(),
            openai: "gpt-default".to_string(),
            anthropic: "claude-default".to_string(),
            gemini: "gemini-default".to_string(),
            deepseek: "deepseek-default".to_string(),
        }
    }

    fn service(cache: SharedAiCache) -> ProviderManagementService {
        let configuration = ProviderConfigurationSnapshot {
            preferred_provider: "auto".to_string(),
            status: ProviderSettingsSnapshot {
                custom_endpoint: Some("https://example.test/v1".to_string()),
                custom_model: Some("model-a".to_string()),
                ..Default::default()
            },
            local_ai_endpoint: Some("http://127.0.0.1:5273".to_string()),
            ollama_endpoint: Some("http://127.0.0.1:11434".to_string()),
            codex_cli_path: Some("C:\\Tools\\codex.exe".to_string()),
            openai_available: true,
            ..Default::default()
        };
        ProviderManagementService::new(
            ProviderProbeBundle {
                configuration: Arc::new(StaticConfiguration(configuration)),
                identity: Arc::new(Identity(false)),
                phi: Arc::new(Phi),
                foundry: Arc::new(Foundry),
                cli: Arc::new(Cli),
                ollama: Arc::new(Ollama),
                custom: Arc::new(Custom),
            },
            ProviderSelectionState::default(),
            Arc::new(cache),
            defaults(),
        )
    }

    #[tokio::test]
    async fn composed_service_produces_status_models_selection_and_cache_control() {
        let cache = SharedAiCache::new(10);
        cache.insert("session:key".to_string(), "cached".to_string());
        let service = service(cache.clone());
        let status = project_provider_status(service.status_input().await);
        assert_eq!(status.active_provider, AIProvider::FoundryLocal);
        assert!(status.phi_silica_available);
        assert!(!status.phi_silica_ready);
        assert_eq!(status.providers[4].id, AIProvider::CodexCli);
        assert!(status.providers[4].available);
        assert_eq!(
            status.providers[4].endpoint.as_deref(),
            Some("C:\\Tools\\codex.exe")
        );

        service.set_preference(AIProviderPreference::OpenAI);
        let status = project_provider_status(service.status_input().await);
        assert_eq!(status.active_provider, AIProvider::OpenAI);

        assert_eq!(
            service.list_ollama_models().await.unwrap(),
            vec!["llama3.2:latest"]
        );
        service.clear_cache(Some("session"));
        assert_eq!(cache.get("session:key"), None);
    }

    struct RawSettings(Vec<u8>);

    impl SettingsStorage for RawSettings {
        fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
            Ok(Some(self.0.clone()))
        }

        fn save(&self, _serialized: &[u8]) -> Result<(), SettingsError> {
            Ok(())
        }
    }

    struct OpenAiCredential {
        read: AtomicBool,
    }

    impl CredentialStorage for OpenAiCredential {
        fn store(&self, _provider: ProviderKeyId, _key: &str) -> Result<(), SettingsError> {
            Ok(())
        }

        fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
            self.read.store(true, Ordering::Release);
            Ok((provider == ProviderKeyId::OpenAI).then(|| "secret".to_string()))
        }

        fn clear(&self, _provider: ProviderKeyId) -> Result<(), SettingsError> {
            Ok(())
        }
    }

    struct Allow;

    impl SettingsValidator for Allow {
        fn validate(&self, _settings: &AppSettings) -> Result<(), SettingsError> {
            Ok(())
        }
    }

    #[test]
    fn corrupted_settings_do_not_hide_independent_credential_availability() {
        let credentials = Arc::new(OpenAiCredential {
            read: AtomicBool::new(false),
        });
        let source = SettingsServiceProviderConfigurationSource::new(SettingsService::new(
            Arc::new(RawSettings(b"not json".to_vec())),
            credentials.clone(),
            Arc::new(Allow),
        ));
        let snapshot = source.snapshot();
        assert_eq!(snapshot.preferred_provider, "auto");
        assert!(snapshot.openai_available);
        assert!(credentials.read.load(Ordering::Acquire));
    }
}
