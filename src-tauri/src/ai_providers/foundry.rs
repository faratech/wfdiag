//! Foundry Local client.
//!
//! Both one-shot analysis and chat use Foundry Local's documented
//! `/v1/chat/completions` endpoint. Discovery validates the service through
//! `/openai/status`; a listening TCP port alone is not sufficient evidence.
//!
//! The endpoint is resolved dynamically (settings override, then the foundry
//! CLI) — its port is dynamic by design and must never be hardcoded.

use super::ResolvedProviderConfig;
use super::discovery::{extract_http_base, normalize_base_url};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default model alias requested from Foundry Local. Phi Silica itself is NOT
/// served by Foundry Local (it is only reachable through the Windows AI APIs,
/// which require package identity); phi-4-mini is Microsoft's documented
/// local fallback model.
pub const FOUNDRY_LOCAL_MODEL: &str = "phi-4-mini";

/// Read the user-configured local AI endpoint from settings, normalized to a
/// base URL without a trailing slash or `/v1` suffix.
fn configured_local_endpoint() -> Option<String> {
    crate::commands::settings::read_settings_from_disk()?
        .local_ai_endpoint
        .as_deref()
        .and_then(normalize_base_url)
}

/// Resolve the `foundry` CLI to an absolute path with a short TTL cache.
/// `Command::new("foundry")` cannot spawn npm `.cmd` shims by bare name
/// (same gotcha the Codex/Claude bridges document), so resolution goes
/// through `cli_bridge::resolve_cli`; the cache keeps status refreshes and
/// Auto routing from spawning `where.exe` on every probe.
async fn resolved_foundry_cli() -> Option<PathBuf> {
    static CACHE: Mutex<Option<(Instant, Option<PathBuf>)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(30);
    if let Ok(cache) = CACHE.lock()
        && let Some((at, cached)) = cache.as_ref()
        && at.elapsed() < TTL
    {
        return cached.clone();
    }
    let resolved = match super::cli_bridge::resolve_cli("foundry", None).await {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("Foundry CLI resolution failed: {error}");
            None
        }
    };
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), resolved.clone()));
    }
    resolved
}

/// Ask the Foundry Local CLI where its service is listening. The service port
/// is dynamic by design — Microsoft documents that it must never be hardcoded,
/// so discovery goes through `foundry service status`.
async fn discover_foundry_endpoint() -> Option<String> {
    let path = resolved_foundry_cli().await?;
    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(["service", "status"]);
    // run_headless adds CREATE_NO_WINDOW, a neutral cwd and kill-on-drop.
    let output =
        super::cli_bridge::run_headless(cmd, None, Duration::from_secs(5), "Foundry Local CLI")
            .await
            .ok()?;
    extract_http_base(&String::from_utf8_lossy(&output.stdout))
}

/// Resolve a reachable local OpenAI-compatible endpoint: an explicit setting
/// wins, otherwise Foundry Local is discovered via its CLI. Returns the base
/// URL (append `/v1` for the OpenAI-compatible API root).
pub(crate) async fn local_ai_endpoint() -> Option<String> {
    if let Some(endpoint) = configured_local_endpoint()
        && service_is_healthy(&endpoint).await
    {
        return Some(endpoint);
    }
    let endpoint = discover_foundry_endpoint().await?;
    if service_is_healthy(&endpoint).await {
        Some(endpoint)
    } else {
        None
    }
}

pub(crate) fn valid_status_body(body: &Value) -> bool {
    body.get("Endpoints")
        .or_else(|| body.get("endpoints"))
        .and_then(Value::as_array)
        .is_some_and(|endpoints| endpoints.iter().any(Value::is_string))
}

async fn service_is_healthy(base: &str) -> bool {
    let Ok(response) = reqwest::Client::new()
        .get(format!("{base}/openai/status"))
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<Value>()
        .await
        .is_ok_and(|body| valid_status_body(&body))
}

fn parse_model_names(body: &Value) -> Result<Vec<String>, String> {
    body.as_array()
        .ok_or_else(|| "Foundry Local model response was not an array".to_string())?
        .iter()
        .map(|model| {
            model
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "Foundry Local returned a non-string model name".to_string())
        })
        .collect()
}

pub async fn list_models(base: &str) -> Result<Vec<String>, String> {
    let response = reqwest::Client::new()
        .get(format!("{base}/openai/models"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| format!("Foundry Local model request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Foundry Local model request failed: HTTP {status}"));
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Foundry Local model response was not JSON: {error}"))?;
    parse_model_names(&body)
}

/// One-shot analysis against the documented chat-completions endpoint.
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    super::openai_compat::one_shot(
        crate::ai_service::AIProvider::FoundryLocal,
        cfg,
        system,
        prompt,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_requires_a_real_foundry_endpoint_list() {
        assert!(valid_status_body(&json!({
            "Endpoints": ["http://localhost:5272"],
            "ModelDirPath": "C:/models"
        })));
        assert!(!valid_status_body(&json!({})));
        assert!(!valid_status_body(&json!({"Endpoints": []})));
    }

    #[test]
    fn documented_model_array_is_parsed_strictly() {
        assert_eq!(
            parse_model_names(&json!(["phi-4-mini", "model-2"])).unwrap(),
            vec!["phi-4-mini", "model-2"]
        );
        assert!(parse_model_names(&json!({"data": []})).is_err());
        assert!(parse_model_names(&json!(["valid", 42])).is_err());
    }
}
