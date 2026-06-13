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

/// Parse the `/api/tags` response into model names. Pure for testability.
pub(crate) fn parse_tags(v: &serde_json::Value) -> Vec<String> {
    v.get("models")
        .and_then(|models| models.as_array())
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("name").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// List locally pulled models via `GET {base}/api/tags`.
pub async fn list_models(base: &str) -> Result<Vec<String>, String> {
    let response = reqwest::Client::new()
        .get(format!("{}/api/tags", base))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Could not reach Ollama at {}: {}", base, e))?;
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected response from Ollama: {}", e))?;
    Ok(parse_tags(&body))
}

/// Resolve the model to use: the configured name wins, otherwise the first
/// locally pulled model. No hardcoded default — an empty library is an error
/// with the fix spelled out.
pub async fn resolve_model(base: &str, configured: Option<&str>) -> Result<String, String> {
    if let Some(model) = configured.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(model.to_string());
    }
    list_models(base).await?.into_iter().next().ok_or_else(|| {
        "No Ollama models installed. Pull one first (e.g. `ollama pull llama3.2`), or set a \
         model name in Settings."
            .to_string()
    })
}

/// List models for the Settings dropdown.
#[tauri::command]
pub async fn ai_list_ollama_models() -> Result<Vec<String>, String> {
    let configured =
        crate::commands::settings::read_settings_from_disk().and_then(|s| s.ollama_endpoint);
    let base = discover_endpoint(configured.as_deref())
        .await
        .ok_or_else(|| {
            "No Ollama server reachable. Install Ollama (https://ollama.com) and make sure it \
             is running."
                .to_string()
        })?;
    list_models(&base).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_model_names_from_tags_response() {
        let body = json!({
            "models": [
                {"name": "llama3.2:latest", "size": 2_000_000_000u64},
                {"name": "phi4-mini:latest"},
                {"size": 1} // malformed entry without a name is skipped
            ]
        });
        assert_eq!(
            parse_tags(&body),
            vec![
                "llama3.2:latest".to_string(),
                "phi4-mini:latest".to_string()
            ]
        );
    }

    #[test]
    fn tolerates_empty_and_malformed_responses() {
        assert!(parse_tags(&json!({"models": []})).is_empty());
        assert!(parse_tags(&json!({})).is_empty());
        assert!(parse_tags(&json!("garbage")).is_empty());
    }
}
