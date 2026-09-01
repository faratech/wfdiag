//! Config resolution and key access for API and OpenAI-compatible providers.
//! Package-bound Phi Silica and subscription CLI bridges remain behind
//! explicit shell adapters, while all API-key providers share this resolver.

use crate::composition::{
    FoundryEndpointSource, OllamaSource, SubscriptionCli, SubscriptionCliStatusSource,
};
use crate::network::normalize_base_url;
use crate::provider_config::ResolvedProviderConfig;
use crate::{AIProvider, ProviderCaps, capabilities};
use wfdiag_native_settings::{AppSettings, ProviderKeyId, SettingsService};

/// Synchronous credential lookup. The shipping implementation wraps the
/// DPAPI/keyring-backed settings service.
pub trait ProviderKeySource: Send + Sync + 'static {
    fn load(&self, key: ProviderKeyId) -> Option<String>;
}

/// The shipping [`ProviderKeySource`]: read one provider's stored key through
/// the settings service, treating any storage error as "no key configured".
///
/// Every shell surface that resolves a provider config needs exactly this, so
/// it lives beside the trait rather than being re-declared per call site.
#[derive(Clone)]
pub struct SettingsProviderKeySource(pub SettingsService);

impl SettingsProviderKeySource {
    #[must_use]
    pub const fn new(settings: SettingsService) -> Self {
        Self(settings)
    }
}

impl ProviderKeySource for SettingsProviderKeySource {
    fn load(&self, key: ProviderKeyId) -> Option<String> {
        self.0.load_provider_key(key).ok().flatten()
    }
}

/// Everything needed to resolve one chat turn's concrete provider call.
pub struct CompatConfigPorts {
    /// Read-only settings snapshot taken by the shell.
    pub settings: AppSettings,
    pub keys: std::sync::Arc<dyn ProviderKeySource>,
    pub foundry: std::sync::Arc<dyn FoundryEndpointSource>,
    pub ollama: std::sync::Arc<dyn OllamaSource>,
}

/// Inputs for resolving a subscription-backed CLI transport. Authentication
/// remains owned by the vendor CLI; this carries only its executable path and
/// optional model selector.
pub struct SubscriptionConfigPorts {
    pub settings: AppSettings,
    pub status: std::sync::Arc<dyn SubscriptionCliStatusSource>,
}

fn configured_model(value: Option<&String>, fallback: &str) -> String {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Resolve key/endpoint/model for the chat-completions providers. Returns the
/// same user-facing errors as the shipping Tauri `resolve_config`.
#[allow(clippy::too_many_lines)] // Keep the provider matrix exhaustive in one audited match.
pub async fn resolve_compat_config(
    provider: AIProvider,
    ports: &CompatConfigPorts,
) -> Result<ResolvedProviderConfig, String> {
    let settings = &ports.settings;
    match provider {
        AIProvider::OpenAI => {
            let api_key = ports.keys.load(ProviderKeyId::OpenAI).ok_or_else(|| {
                String::from(
                    "OpenAI API key not configured. Please enter your API key in Settings.",
                )
            })?;
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(configured_model(
                    settings.open_ai_model.as_ref(),
                    crate::OPENAI_DEFAULT_MODEL,
                )),
            })
        }
        AIProvider::FoundryLocal => {
            let configured = settings
                .local_ai_endpoint
                .as_deref()
                .and_then(normalize_base_url);
            let endpoint = ports
                .foundry
                .probe(configured)
                .await
                .ok_or_else(|| String::from(
                    "No local AI endpoint available. Install Foundry Local and run 'foundry server start', or configure an endpoint in Settings.",
                ))?;
            Ok(ResolvedProviderConfig {
                api_key: None,
                endpoint: Some(endpoint),
                model: Some(configured_model(
                    settings.local_ai_model.as_ref(),
                    crate::FOUNDRY_DEFAULT_MODEL,
                )),
            })
        }
        AIProvider::Ollama => {
            let endpoint = ports
                .ollama
                .discover(settings.ollama_endpoint.clone())
                .await
                .ok_or_else(|| String::from(
                    "No Ollama server reachable. Install Ollama (https://ollama.com) and make sure it is running, or configure its endpoint in Settings.",
                ))?;
            let model =
                crate::network::resolve_ollama_model(&endpoint, settings.ollama_model.as_deref())
                    .await?;
            Ok(ResolvedProviderConfig {
                api_key: None,
                endpoint: Some(endpoint),
                model: Some(model),
            })
        }
        AIProvider::CustomOpenAI => {
            let endpoint = settings
                .custom_endpoint
                .as_deref()
                .and_then(normalize_base_url)
                .ok_or_else(|| {
                    String::from("No custom endpoint configured. Set the endpoint URL in Settings.")
                })?;
            let model = settings
                .custom_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .ok_or_else(|| String::from(
                    "No model configured for the custom endpoint. Set a model name in Settings (e.g. the model id your provider documents).",
                ))?;
            Ok(ResolvedProviderConfig {
                api_key: ports.keys.load(ProviderKeyId::Custom),
                endpoint: Some(endpoint),
                model: Some(model.to_string()),
            })
        }
        AIProvider::Anthropic => {
            let api_key = ports.keys.load(ProviderKeyId::Anthropic).ok_or_else(|| {
                String::from(
                    "Anthropic API key not configured. Please enter your API key in Settings.",
                )
            })?;
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(configured_model(
                    settings.anthropic_model.as_ref(),
                    crate::ANTHROPIC_DEFAULT_MODEL,
                )),
            })
        }
        AIProvider::Gemini => {
            let api_key = ports.keys.load(ProviderKeyId::Gemini).ok_or_else(|| {
                String::from(
                    "Gemini API key not configured. Please enter your API key in Settings.",
                )
            })?;
            let model =
                crate::resolve_gemini_model(settings.gemini_model.as_deref(), &api_key).await;
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(model),
            })
        }
        AIProvider::DeepSeek => {
            let api_key = ports.keys.load(ProviderKeyId::DeepSeek).ok_or_else(|| {
                String::from(
                    "DeepSeek API key not configured. Please enter your API key in Settings.",
                )
            })?;
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: Some("https://api.deepseek.com".to_string()),
                model: Some(configured_model(
                    settings.deepseek_model.as_ref(),
                    crate::DEEPSEEK_DEFAULT_MODEL,
                )),
            })
        }
        other => Err(format!(
            "{other} chat requires a provider transport this shell does not provide yet"
        )),
    }
}

/// Resolve a signed-in Codex or Claude Code CLI to the same non-secret
/// provider configuration consumed by the shared chat engine.
pub async fn resolve_subscription_config(
    provider: AIProvider,
    ports: &SubscriptionConfigPorts,
) -> Result<ResolvedProviderConfig, String> {
    let (cli, configured_path, configured_model, label) = match provider {
        AIProvider::CodexCli => (
            SubscriptionCli::Codex,
            ports.settings.codex_cli_path.clone(),
            ports.settings.codex_model.clone(),
            "Codex CLI",
        ),
        AIProvider::ClaudeCode => (
            SubscriptionCli::ClaudeCode,
            ports.settings.claude_cli_path.clone(),
            ports.settings.claude_model.clone(),
            "Claude Code",
        ),
        other => {
            return Err(format!("{other} is not a subscription CLI provider"));
        }
    };
    let probe = ports.status.probe(cli, configured_path).await;
    let path = probe.path.ok_or_else(|| {
        format!("{label} was not found. Install it or configure its executable path in Settings.")
    })?;
    if !probe.usable {
        return Err(format!(
            "{label} is installed but not signed in. Open Settings and sign in with the vendor CLI."
        ));
    }
    Ok(ResolvedProviderConfig {
        api_key: None,
        endpoint: Some(path),
        model: configured_model
            .filter(|model| !model.trim().is_empty())
            .map(|model| sanitize_subscription_model(&model))
            .transpose()?,
    })
}

/// Validate a subscription model selector before it can reach an argument or
/// environment-variable boundary. The accepted alphabet matches the shipping
/// CLI bridge and includes versioned aliases such as `opus[1m]`.
pub fn sanitize_subscription_model(model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Model name is empty".to_string());
    }
    if model.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | ':' | '/' | '-' | '[' | ']')
    }) {
        Ok(model.to_string())
    } else {
        Err(format!(
            "Invalid model name '{model}': only letters, digits and . _ : / - [ ] are allowed"
        ))
    }
}

/// Stable non-secret identity of one resolved provider configuration: the
/// key participates only as a hash, never in the clear. Cache identity in
/// both shells derives from this, so the function body must not drift.
#[must_use]
pub fn provider_config_fingerprint(provider: AIProvider, cfg: &ResolvedProviderConfig) -> String {
    fn key_fingerprint(key: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        "wfdiag-ai-key-v1".hash(&mut hasher);
        key.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    let key = cfg
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .map_or_else(|| "none".to_string(), key_fingerprint);

    format!(
        "provider={};endpoint={};model={};key={}",
        provider,
        cfg.endpoint.as_deref().unwrap_or("none"),
        cfg.model.as_deref().unwrap_or("none"),
        key
    )
}

/// Capability projection for a chat-completions turn (streaming, budget).
#[must_use]
pub fn compat_caps(provider: AIProvider) -> ProviderCaps {
    capabilities(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendFuture, CliProbeSnapshot};

    struct FixedKeys(Option<String>);

    impl ProviderKeySource for FixedKeys {
        fn load(&self, _key: ProviderKeyId) -> Option<String> {
            self.0.clone()
        }
    }

    struct Unreachable;

    impl FoundryEndpointSource for Unreachable {
        fn probe(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
    }

    impl OllamaSource for Unreachable {
        fn discover(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
        fn list_models(&self, _endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    struct FixedSubscriptionStatus(CliProbeSnapshot);

    impl SubscriptionCliStatusSource for FixedSubscriptionStatus {
        fn probe(
            &self,
            _provider: SubscriptionCli,
            _configured_path: Option<String>,
        ) -> crate::BackendFuture<'_, CliProbeSnapshot> {
            let snapshot = self.0.clone();
            Box::pin(async move { snapshot })
        }
    }

    fn ports(key: Option<&str>, settings: AppSettings) -> CompatConfigPorts {
        CompatConfigPorts {
            settings,
            keys: std::sync::Arc::new(FixedKeys(key.map(str::to_string))),
            foundry: std::sync::Arc::new(Unreachable),
            ollama: std::sync::Arc::new(Unreachable),
        }
    }

    #[tokio::test]
    async fn openai_requires_a_configured_key() {
        let error = resolve_compat_config(AIProvider::OpenAI, &ports(None, AppSettings::default()))
            .await
            .unwrap_err();
        assert!(error.contains("OpenAI API key not configured"));
    }

    #[tokio::test]
    async fn openai_uses_configured_model_with_shipping_default() {
        let settings = AppSettings {
            open_ai_model: Some("gpt-5-mini".to_string()),
            ..AppSettings::default()
        };
        let configured =
            resolve_compat_config(AIProvider::OpenAI, &ports(Some("sk-test"), settings))
                .await
                .unwrap();
        assert_eq!(configured.model.as_deref(), Some("gpt-5-mini"));

        let defaulted = resolve_compat_config(
            AIProvider::OpenAI,
            &ports(Some("sk-test"), AppSettings::default()),
        )
        .await
        .unwrap();
        assert_eq!(defaulted.model.as_deref(), Some("gpt-5-nano"));
        assert_eq!(defaulted.endpoint, None);
    }

    #[tokio::test]
    async fn custom_endpoint_requires_a_model() {
        let no_model = AppSettings {
            custom_endpoint: Some("http://127.0.0.1:8080/v1".to_string()),
            ..AppSettings::default()
        };
        let error = resolve_compat_config(AIProvider::CustomOpenAI, &ports(None, no_model.clone()))
            .await
            .unwrap_err();
        assert!(error.contains("No model configured"));

        let settings = AppSettings {
            custom_model: Some("qwen3".to_string()),
            ..no_model
        };
        let resolved = resolve_compat_config(AIProvider::CustomOpenAI, &ports(None, settings))
            .await
            .unwrap();
        // The /v1 suffix is stripped by the shipping normalizer.
        assert_eq!(resolved.endpoint.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(resolved.model.as_deref(), Some("qwen3"));
    }

    #[tokio::test]
    async fn ollama_discovery_failure_names_the_remediation() {
        let error = resolve_compat_config(AIProvider::Ollama, &ports(None, AppSettings::default()))
            .await
            .unwrap_err();
        assert!(error.contains("No Ollama server reachable"));
        assert!(error.contains("https://ollama.com"));
    }

    #[tokio::test]
    async fn native_cloud_providers_require_credentials_and_resolve_shipping_defaults() {
        for (provider, key_name, model, endpoint) in [
            (
                AIProvider::Anthropic,
                "Anthropic API key",
                "claude-sonnet-5",
                None,
            ),
            (
                AIProvider::Gemini,
                "Gemini API key",
                "gemini-3.6-flash",
                None,
            ),
            (
                AIProvider::DeepSeek,
                "DeepSeek API key",
                "deepseek-v4-flash",
                Some("https://api.deepseek.com"),
            ),
        ] {
            let error = resolve_compat_config(provider, &ports(None, AppSettings::default()))
                .await
                .unwrap_err();
            assert!(error.contains(key_name), "{provider}: {error}");

            // Gemini's blank setting intentionally performs live discovery;
            // explicit selection keeps this provider-matrix test hermetic.
            let settings = if provider == AIProvider::Gemini {
                AppSettings {
                    gemini_model: Some(model.to_string()),
                    ..AppSettings::default()
                }
            } else {
                AppSettings::default()
            };
            let resolved =
                resolve_compat_config(provider, &ports(Some("secret-test-key"), settings))
                    .await
                    .unwrap();
            assert_eq!(resolved.model.as_deref(), Some(model), "{provider}");
            assert_eq!(resolved.endpoint.as_deref(), endpoint, "{provider}");
        }
    }

    #[tokio::test]
    async fn host_specific_providers_report_their_transport_gap() {
        for provider in [
            AIProvider::PhiSilica,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
        ] {
            let error = resolve_compat_config(provider, &ports(None, AppSettings::default()))
                .await
                .unwrap_err();
            assert!(
                error.contains("does not provide yet"),
                "{provider}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn subscription_config_requires_install_and_sign_in() {
        let missing = SubscriptionConfigPorts {
            settings: AppSettings::default(),
            status: std::sync::Arc::new(FixedSubscriptionStatus(CliProbeSnapshot::default())),
        };
        assert!(
            resolve_subscription_config(AIProvider::CodexCli, &missing)
                .await
                .unwrap_err()
                .contains("was not found")
        );

        let signed_out = SubscriptionConfigPorts {
            settings: AppSettings::default(),
            status: std::sync::Arc::new(FixedSubscriptionStatus(CliProbeSnapshot {
                usable: false,
                installed: true,
                path: Some("codex".to_string()),
            })),
        };
        assert!(
            resolve_subscription_config(AIProvider::CodexCli, &signed_out)
                .await
                .unwrap_err()
                .contains("not signed in")
        );
    }

    #[tokio::test]
    async fn subscription_config_returns_probed_path_and_model() {
        let ports = SubscriptionConfigPorts {
            settings: AppSettings {
                claude_model: Some("claude-sonnet-5".to_string()),
                ..AppSettings::default()
            },
            status: std::sync::Arc::new(FixedSubscriptionStatus(CliProbeSnapshot {
                usable: true,
                installed: true,
                path: Some("C:/tools/claude.exe".to_string()),
            })),
        };
        let resolved = resolve_subscription_config(AIProvider::ClaudeCode, &ports)
            .await
            .unwrap();
        assert_eq!(resolved.endpoint.as_deref(), Some("C:/tools/claude.exe"));
        assert_eq!(resolved.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(resolved.api_key, None);
    }

    #[tokio::test]
    async fn subscription_config_rejects_model_injection_before_transport() {
        let ports = SubscriptionConfigPorts {
            settings: AppSettings {
                codex_model: Some("gpt-5.6-sol --danger".to_string()),
                ..AppSettings::default()
            },
            status: std::sync::Arc::new(FixedSubscriptionStatus(CliProbeSnapshot {
                usable: true,
                installed: true,
                path: Some("C:/tools/codex.exe".to_string()),
            })),
        };
        let error = resolve_subscription_config(AIProvider::CodexCli, &ports)
            .await
            .unwrap_err();
        assert!(error.contains("Invalid model name"));
        assert_eq!(
            sanitize_subscription_model("opus[1m]").as_deref(),
            Ok("opus[1m]")
        );
    }
}
