//! Cloud OpenAI client.
//!
//! One-shot analysis stays on the Responses API (the path this app has always
//! shipped). Multi-turn/tool chat goes through the shared chat-completions
//! client in [`super::openai_compat`] with the default OpenAI base URL.

use super::ResolvedProviderConfig;
use crate::error::DiagError;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{CreateResponseArgs, InputParam},
};

/// One-shot analysis using the OpenAI Responses API.
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let api_key = cfg
        .api_key
        .clone()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            String::from(DiagError::api_key(
                "load",
                "OpenAI API key not configured. Please enter your API key in Settings.",
            ))
        })?;

    let client = Client::with_config(OpenAIConfig::new().with_api_key(api_key));
    let model = cfg
        .model
        .as_deref()
        .unwrap_or(crate::openai_integration::OPENAI_MODEL);

    let full_prompt = format!("{}\n\n{}", system, prompt);

    let request = CreateResponseArgs::default()
        .model(model)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| DiagError::AiAnalysisFailed {
            reason: format!("Failed to build request: {}", e),
        })?;

    let response = client.responses().create(request).await.map_err(|e| {
        eprintln!("OpenAI API error in ai_providers: {:?}", e);
        DiagError::AiAnalysisFailed {
            reason: format!("OpenAI API error: {}", e),
        }
    })?;

    Ok(response.output_text().unwrap_or_default())
}
