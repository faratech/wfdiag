//! Config resolution and key access for the chat-capable OpenAI-compatible
//! providers. Both desktop shells resolve these four providers identically;
//! providers with shell-specific transports (Phi Silica activation, the
//! subscription CLI bridges, and the DeepSeek/Anthropic/Gemini native
//! clients) stay behind their owners.

use crate::composition::{FoundryEndpointSource, OllamaSource};
use crate::network::normalize_base_url;
use crate::provider_config::ResolvedProviderConfig;
use crate::{AIProvider, ProviderCaps, capabilities};
use wfdiag_native_settings::{AppSettings, ProviderKeyId};

/// Synchronous credential lookup. The shipping implementation wraps the
/// DPAPI/keyring-backed settings service.
pub trait ProviderKeySource: Send + Sync + 'static {
    fn load(&self, key: ProviderKeyId) -> Option<String>;
}

/// Everything needed to resolve one chat turn's concrete provider call.
pub struct CompatConfigPorts {
    /// Read-only settings snapshot taken by the shell.
    pub settings: AppSettings,
    pub keys: std::sync::Arc<dyn ProviderKeySource>,
    pub foundry: std::sync::Arc<dyn FoundryEndpointSource>,
    pub ollama: std::sync::Arc<dyn OllamaSource>,
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
                model: Some(configured_model(settings.open_ai_model.as_ref(), "gpt-5-nano")),
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
                    "No local AI endpoint available. Install Foundry Local and run 'foundry service start', or configure an endpoint in Settings.",
                ))?;
            Ok(ResolvedProviderConfig {
                api_key: None,
                endpoint: Some(endpoint),
                model: Some(configured_model(settings.local_ai_model.as_ref(), "phi-4-mini")),
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
                .ok_or_else(|| String::from(
                    "No custom endpoint configured. Set the endpoint URL in Settings.",
                ))?;
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
        other => Err(format!(
            "{other} chat requires a provider transport this shell does not provide yet"
        )),
    }
}

/// Capability projection for a chat-completions turn (streaming, budget).
#[must_use]
pub fn compat_caps(provider: AIProvider) -> ProviderCaps {
    capabilities(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackendFuture;

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
        fn list_models(
            &self,
            _endpoint: String,
        ) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async { Ok(Vec::new()) })
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
        let configured = resolve_compat_config(AIProvider::OpenAI, &ports(Some("sk-test"), settings))
            .await
            .unwrap();
        assert_eq!(configured.model.as_deref(), Some("gpt-5-mini"));

        let defaulted =
            resolve_compat_config(AIProvider::OpenAI, &ports(Some("sk-test"), AppSettings::default()))
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
        let error =
            resolve_compat_config(AIProvider::CustomOpenAI, &ports(None, no_model.clone()))
                .await
                .unwrap_err();
        assert!(error.contains("No model configured"));

        let settings = AppSettings {
            custom_model: Some("qwen3".to_string()),
            ..no_model
        };
        let resolved =
            resolve_compat_config(AIProvider::CustomOpenAI, &ports(None, settings)).await.unwrap();
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
    async fn shell_specific_providers_report_their_transport_gap() {
        for provider in [
            AIProvider::Anthropic,
            AIProvider::Gemini,
            AIProvider::DeepSeek,
            AIProvider::PhiSilica,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
        ] {
            let error = resolve_compat_config(provider, &ports(None, AppSettings::default()))
                .await
                .unwrap_err();
            assert!(error.contains("does not provide yet"), "{provider}: {error}");
        }
    }
}
