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
use crate::error::DiagError;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{CreateResponseArgs, InputParam},
};

/// One-shot analysis against the local `/v1/responses` endpoint.
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let endpoint = cfg.endpoint_or_err(crate::ai_service::AIProvider::FoundryLocal)?;
    let model = cfg
        .model
        .as_deref()
        .unwrap_or(crate::openai_integration::FOUNDRY_LOCAL_MODEL);

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
