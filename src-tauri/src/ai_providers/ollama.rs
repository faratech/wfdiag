//! Ollama provider: endpoint discovery and model resolution.
//!
//! Ollama serves an OpenAI-compatible API at `{base}/v1` and its native model
//! listing at `{base}/api/tags`. There is deliberately no default model
//! constant — resolution order is the `ollamaModel` setting, then the first
//! locally pulled model, then an instructive error.

/// Default local Ollama endpoint, used when no `ollamaEndpoint` is configured.
pub use wfdiag_native_ai_provider::OLLAMA_DEFAULT_ENDPOINT;

/// Resolve a reachable Ollama base URL: the configured endpoint wins, else
/// the default port is probed. Returns None when nothing is listening.
pub async fn discover_endpoint(configured: Option<&str>) -> Option<String> {
    wfdiag_native_ai_provider::discover_ollama_endpoint(configured).await
}

/// List locally pulled models via `GET {base}/api/tags`.
pub async fn list_models(base: &str) -> Result<Vec<String>, String> {
    wfdiag_native_ai_provider::list_ollama_models(base).await
}

/// Ask Ollama whether the selected model supports tool calling. The provider
/// capability is model-specific; advertising tools solely because the server
/// supports them makes non-tool models reject otherwise valid chat requests.
pub async fn model_supports_tools(base: &str, model: &str) -> Result<bool, String> {
    wfdiag_native_ai_provider::ollama_model_supports_tools(base, model).await
}

/// Resolve the model to use: the configured name wins, otherwise the first
/// locally pulled model. No hardcoded default — an empty library is an error
/// with the fix spelled out.
pub async fn resolve_model(base: &str, configured: Option<&str>) -> Result<String, String> {
    wfdiag_native_ai_provider::resolve_ollama_model(base, configured).await
}

/// List models for the Settings dropdown through the native provider worker.
#[tauri::command]
pub async fn ai_list_ollama_models() -> Result<Vec<String>, String> {
    crate::ai_service::managed_ollama_models().await
}
