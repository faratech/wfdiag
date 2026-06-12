//! Foundry Local client.
//!
//! One-shot analysis keeps the proven `/v1/responses` path (Foundry Local
//! serves it and this is what shipped in 2.3.0). Chat goes through the shared
//! chat-completions client — flagged for verification on a real install; the
//! capability table keeps Foundry tool-less until then.
//!
//! The endpoint is resolved dynamically (settings override, then the foundry
//! CLI) — its port is dynamic by design and must never be hardcoded.

use super::ResolvedProviderConfig;
use super::discovery::{extract_http_base, normalize_base_url, probe_endpoint_async};
use crate::error::DiagError;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{CreateResponseArgs, InputParam},
};

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

/// Ask the Foundry Local CLI where its service is listening. The service port
/// is dynamic by design — Microsoft documents that it must never be hardcoded,
/// so discovery goes through `foundry service status`.
async fn discover_foundry_endpoint() -> Option<String> {
    let mut cmd = tokio::process::Command::new("foundry");
    cmd.args(["service", "status"]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    extract_http_base(&String::from_utf8_lossy(&output.stdout))
}

/// Resolve a reachable local OpenAI-compatible endpoint: an explicit setting
/// wins, otherwise Foundry Local is discovered via its CLI. Returns the base
/// URL (append `/v1` for the OpenAI-compatible API root).
pub(crate) async fn local_ai_endpoint() -> Option<String> {
    if let Some(endpoint) = configured_local_endpoint()
        && probe_endpoint_async(&endpoint).await
    {
        return Some(endpoint);
    }
    let endpoint = discover_foundry_endpoint().await?;
    if probe_endpoint_async(&endpoint).await {
        Some(endpoint)
    } else {
        None
    }
}

/// One-shot analysis against the local `/v1/responses` endpoint.
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let endpoint = cfg.endpoint_or_err(crate::ai_service::AIProvider::FoundryLocal)?;
    let model = cfg.model.as_deref().unwrap_or(FOUNDRY_LOCAL_MODEL);

    let config = OpenAIConfig::new()
        .with_api_base(format!("{}/v1", endpoint))
        .with_api_key("not-needed");
    let client = Client::with_config(config);

    let full_prompt = format!("{}\n\n{}", system, prompt);

    let request = CreateResponseArgs::default()
        .model(model)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| DiagError::AiAnalysisFailed {
            reason: format!("Failed to build request: {}", e),
        })?;

    let response = client.responses().create(request).await.map_err(|e| {
        eprintln!("Foundry Local API error in ai_providers: {:?}", e);
        DiagError::AiAnalysisFailed {
            reason: format!(
                "Foundry Local error: {}. Ensure the service is running and the model '{}' \
                 is loaded (foundry model run {}).",
                e, model, model
            ),
        }
    })?;

    Ok(response.output_text().unwrap_or_default())
}
