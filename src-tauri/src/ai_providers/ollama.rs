//! Ollama provider: endpoint discovery and model resolution.
//!
//! Ollama serves an OpenAI-compatible API at `{base}/v1` and its native model
//! listing at `{base}/api/tags`. There is deliberately no default model
//! constant — resolution order is the `ollamaModel` setting, then the first
//! locally pulled model, then an instructive error.

use super::discovery::{normalize_base_url, probe_endpoint_async};

/// Default local Ollama endpoint, used when no `ollamaEndpoint` is configured.
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Resolve a reachable Ollama base URL: the configured endpoint wins, else
/// the default port is probed. Returns None when nothing is listening.
pub async fn discover_endpoint(configured: Option<&str>) -> Option<String> {
    if let Some(endpoint) = configured.and_then(normalize_base_url) {
        if probe_endpoint_async(&endpoint).await {
            return Some(endpoint);
        }
        // An explicitly configured endpoint that is down should not silently
        // fall back to the default port — the user pointed us elsewhere.
        return None;
    }
    if probe_endpoint_async(OLLAMA_DEFAULT_ENDPOINT).await {
        Some(OLLAMA_DEFAULT_ENDPOINT.to_string())
    } else {
        None
    }
}
