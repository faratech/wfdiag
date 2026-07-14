//! Ollama provider: endpoint discovery and model resolution.
//!
//! Ollama serves an OpenAI-compatible API at `{base}/v1` and its native model
//! listing at `{base}/api/tags`. There is deliberately no default model
//! constant — resolution order is the `ollamaModel` setting, then the first
//! locally pulled model, then an instructive error.

use super::discovery::normalize_base_url;

/// Default local Ollama endpoint, used when no `ollamaEndpoint` is configured.
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Resolve a reachable Ollama base URL: the configured endpoint wins, else
/// the default port is probed. Returns None when nothing is listening.
pub async fn discover_endpoint(configured: Option<&str>) -> Option<String> {
    if let Some(endpoint) = configured.and_then(normalize_base_url) {
        if list_models(&endpoint).await.is_ok() {
            return Some(endpoint);
        }
        // An explicitly configured endpoint that is down should not silently
        // fall back to the default port — the user pointed us elsewhere.
        return None;
    }
    if list_models(OLLAMA_DEFAULT_ENDPOINT).await.is_ok() {
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
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|e| format!("Could not read Ollama response: {e}"))?;
    let body: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Unexpected response from Ollama: {e}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown Ollama error");
        return Err(format!("Ollama API error ({status}): {detail}"));
    }
    if !body.get("models").is_some_and(serde_json::Value::is_array) {
        return Err("Ollama /api/tags response omitted the models array".to_string());
    }
    Ok(parse_tags(&body))
}

pub(crate) fn parse_capabilities(body: &serde_json::Value) -> Result<Vec<String>, String> {
    body.get("capabilities")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Ollama /api/show response omitted capabilities".to_string())?
        .iter()
        .map(|capability| {
            capability
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "Ollama returned a non-string model capability".to_string())
        })
        .collect()
}

/// Ask Ollama whether the selected model supports tool calling. The provider
/// capability is model-specific; advertising tools solely because the server
/// supports them makes non-tool models reject otherwise valid chat requests.
pub async fn model_supports_tools(base: &str, model: &str) -> Result<bool, String> {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/show"))
        .timeout(std::time::Duration::from_secs(5))
        .json(&serde_json::json!({"model": model}))
        .send()
        .await
        .map_err(|e| format!("Could not inspect Ollama model '{model}': {e}"))?;
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|e| format!("Could not read Ollama model details: {e}"))?;
    let body: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("Unexpected Ollama model details response: {e}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown Ollama error");
        return Err(format!("Ollama API error ({status}): {detail}"));
    }
    Ok(parse_capabilities(&body)?
        .iter()
        .any(|capability| capability == "tools"))
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

    #[test]
    fn model_capabilities_are_explicit_and_tool_specific() {
        let with_tools = json!({"capabilities": ["completion", "tools"]});
        assert!(
            parse_capabilities(&with_tools)
                .unwrap()
                .iter()
                .any(|capability| capability == "tools")
        );
        let without_tools = json!({"capabilities": ["completion", "vision"]});
        assert!(
            !parse_capabilities(&without_tools)
                .unwrap()
                .contains(&"tools".into())
        );
        assert!(parse_capabilities(&json!({})).is_err());
    }
}
