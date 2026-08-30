//! Provider-call configuration resolved once per request.
//!
//! Lives beside the chat engine so provider clients in this crate and in the
//! shipping backend share one definition; the backend re-exports it instead
//! of defining its own copy.

use crate::ai_service::AIProvider;

/// Everything a provider call needs, resolved once per request: API key from
/// DPAPI/keyring, endpoint and model from settings (with provider defaults).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedProviderConfig {
    pub api_key: Option<String>,
    /// Base URL for local/custom providers (no `/v1` suffix)
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

impl ResolvedProviderConfig {
    #[must_use]
    pub fn key(&self) -> &str {
        self.api_key.as_deref().unwrap_or_default()
    }

    pub fn endpoint_or_err(&self, provider: AIProvider) -> Result<&str, String> {
        self.endpoint
            .as_deref()
            .ok_or_else(|| format!("No endpoint resolved for {provider}"))
    }

    pub fn model_or_err(&self, provider: AIProvider) -> Result<&str, String> {
        self.model
            .as_deref()
            .ok_or_else(|| format!("No model resolved for {provider}"))
    }
}
