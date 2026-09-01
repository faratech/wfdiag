use crate::{BackendFuture, CustomEndpointSource, OllamaSource};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Normalize a configured endpoint to a base URL. This retains the shipping
/// behavior: trim whitespace/trailing slashes and strip one `/v1` suffix.
#[must_use]
pub fn normalize_base_url(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let endpoint = endpoint.strip_suffix("/v1").unwrap_or(endpoint);
    if endpoint.is_empty() {
        None
    } else {
        Some(endpoint.to_string())
    }
}

/// Blocking TCP reachability probe used only from a background worker.
#[must_use]
pub fn probe_http_endpoint(base: &str) -> bool {
    let Ok(url) = url::Url::parse(base) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    match (host, port).to_socket_addrs() {
        Ok(mut addresses) => addresses
            .any(|address| TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok()),
        Err(_) => false,
    }
}

pub async fn probe_http_endpoint_async(base: &str) -> bool {
    let base = base.to_string();
    tokio::task::spawn_blocking(move || probe_http_endpoint(&base))
        .await
        .unwrap_or(false)
}

#[must_use]
pub fn parse_ollama_tags(value: &Value) -> Vec<String> {
    value
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_ollama_capabilities(body: &Value) -> Result<Vec<String>, String> {
    body.get("capabilities")
        .and_then(Value::as_array)
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

pub async fn list_ollama_models(base: &str) -> Result<Vec<String>, String> {
    let response = reqwest::Client::new()
        .get(format!("{base}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| format!("Could not reach Ollama at {base}: {error}"))?;
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|error| format!("Could not read Ollama response: {error}"))?;
    let body = serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("Unexpected response from Ollama: {error}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown Ollama error");
        return Err(format!("Ollama API error ({status}): {detail}"));
    }
    if !body.get("models").is_some_and(Value::is_array) {
        return Err("Ollama /api/tags response omitted the models array".to_string());
    }
    Ok(parse_ollama_tags(&body))
}

pub async fn discover_ollama_endpoint(configured: Option<&str>) -> Option<String> {
    if let Some(endpoint) = configured.and_then(normalize_base_url) {
        if list_ollama_models(&endpoint).await.is_ok() {
            return Some(endpoint);
        }
        return None;
    }
    if list_ollama_models(OLLAMA_DEFAULT_ENDPOINT).await.is_ok() {
        Some(OLLAMA_DEFAULT_ENDPOINT.to_string())
    } else {
        None
    }
}

/// Model capabilities change only when the user pulls or replaces a model, so
/// a five-minute TTL keeps a multi-round tool conversation from issuing one
/// `POST /api/show` per round inside the shared turn deadline. Only successful
/// probes are cached; errors keep propagating.
const OLLAMA_CAPABILITY_TTL: Duration = Duration::from_mins(5);

type OllamaCapabilityCache = Arc<Mutex<HashMap<(String, String), (Instant, bool)>>>;

fn shared_ollama_capability_cache() -> OllamaCapabilityCache {
    static CACHE: std::sync::OnceLock<OllamaCapabilityCache> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

pub async fn ollama_model_supports_tools(base: &str, model: &str) -> Result<bool, String> {
    let cache_key = (
        base.trim().trim_end_matches('/').to_string(),
        model.to_string(),
    );
    if let Ok(cache) = shared_ollama_capability_cache().lock()
        && let Some((at, supported)) = cache.get(&cache_key)
        && at.elapsed() < OLLAMA_CAPABILITY_TTL
    {
        return Ok(*supported);
    }
    let supported = fetch_ollama_model_supports_tools(base, model).await?;
    if let Ok(mut cache) = shared_ollama_capability_cache().lock() {
        cache.insert(cache_key, (Instant::now(), supported));
    }
    Ok(supported)
}

async fn fetch_ollama_model_supports_tools(base: &str, model: &str) -> Result<bool, String> {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/show"))
        .timeout(Duration::from_secs(5))
        .json(&serde_json::json!({"model": model}))
        .send()
        .await
        .map_err(|error| format!("Could not inspect Ollama model '{model}': {error}"))?;
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|error| format!("Could not read Ollama model details: {error}"))?;
    let body = serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("Unexpected Ollama model details response: {error}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown Ollama error");
        return Err(format!("Ollama API error ({status}): {detail}"));
    }
    Ok(parse_ollama_capabilities(&body)?
        .iter()
        .any(|capability| capability == "tools"))
}

pub async fn resolve_ollama_model(base: &str, configured: Option<&str>) -> Result<String, String> {
    if let Some(model) = configured.map(str::trim).filter(|model| !model.is_empty()) {
        return Ok(model.to_string());
    }
    list_ollama_models(base)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            "No Ollama models installed. Pull one first (e.g. `ollama pull llama3.2`), or set a model name in Settings."
                .to_string()
        })
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReqwestOllamaSource;

impl OllamaSource for ReqwestOllamaSource {
    fn discover(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
        Box::pin(async move { discover_ollama_endpoint(configured.as_deref()).await })
    }

    fn list_models(&self, endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move { list_ollama_models(&endpoint).await })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TcpCustomEndpointSource;

impl CustomEndpointSource for TcpCustomEndpointSource {
    fn probe(
        &self,
        endpoint: Option<String>,
        model: Option<String>,
    ) -> BackendFuture<'_, Option<String>> {
        Box::pin(async move {
            model.as_deref().filter(|model| !model.trim().is_empty())?;
            let endpoint = endpoint.as_deref().and_then(normalize_base_url)?;
            if probe_http_endpoint_async(&endpoint).await {
                Some(endpoint)
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalization_is_shipping_compatible() {
        assert_eq!(
            normalize_base_url(" http://127.0.0.1:11434/ ").as_deref(),
            Some("http://127.0.0.1:11434")
        );
        assert_eq!(
            normalize_base_url("https://openrouter.ai/api/v1").as_deref(),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(normalize_base_url("   "), None);
        assert_eq!(normalize_base_url("/v1"), None);
    }

    #[test]
    fn ollama_responses_are_parsed_without_network() {
        let tags = serde_json::json!({
            "models": [
                {"name": "llama3.2:latest"},
                {"name": "phi4-mini:latest"},
                {"size": 1}
            ]
        });
        assert_eq!(
            parse_ollama_tags(&tags),
            vec!["llama3.2:latest", "phi4-mini:latest"]
        );
        assert!(parse_ollama_tags(&serde_json::json!({})).is_empty());

        let capabilities = serde_json::json!({"capabilities": ["completion", "tools"]});
        assert!(
            parse_ollama_capabilities(&capabilities)
                .unwrap()
                .contains(&"tools".to_string())
        );
        assert!(parse_ollama_capabilities(&serde_json::json!({})).is_err());
    }
}
