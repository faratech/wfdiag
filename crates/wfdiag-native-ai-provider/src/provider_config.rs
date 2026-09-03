//! Provider-call configuration resolved once per request.
//!
//! Lives in the lowest provider crate so both the chat engine crate and the
//! shipping backend share one definition.

use crate::AIProvider;

/// Everything a provider call needs, resolved once per request: API key from
/// DPAPI/keyring, endpoint and model from settings (with provider defaults).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ResolvedProviderConfig {
    pub api_key: Option<String>,
    /// Base URL for local/custom providers (no `/v1` suffix)
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

impl std::fmt::Debug for ResolvedProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Presence only: this type reaches request-building code everywhere,
        // so a derived Debug would put the key in any log or panic message
        // that ever formats it (2026-09-03 audit; `ModelCatalogRequest`
        // already redacts the same way).
        f.debug_struct("ResolvedProviderConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .finish()
    }
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
