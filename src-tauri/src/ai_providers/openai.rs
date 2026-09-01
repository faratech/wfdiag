//! Cloud OpenAI client.
//!
//! One-shot analysis stays on the Responses API (the path this app has always
//! shipped). Multi-turn/tool chat goes through the shared chat-completions
//! client in [`super::openai_compat`] with the default OpenAI base URL.

use super::ResolvedProviderConfig;
use crate::error::DiagError;

/// OpenAI model used for all cloud-OpenAI calls.
/// Change this constant to switch models globally.
pub const OPENAI_MODEL: &str = wfdiag_native_ai_provider::OPENAI_DEFAULT_MODEL;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{CreateResponseArgs, ErrorObject, IncompleteDetails, InputParam, Status},
};

fn completed_output(
    status: &Status,
    error: Option<&ErrorObject>,
    incomplete: Option<&IncompleteDetails>,
    output: Option<String>,
) -> Result<String, String> {
    if status != &Status::Completed {
        let detail = error
            .map(|error| format!("{}: {}", error.code, error.message))
            .or_else(|| incomplete.map(|details| details.reason.clone()))
            .unwrap_or_else(|| format!("response status was {status:?}"));
        return Err(format!("OpenAI response did not complete: {detail}"));
    }
    output
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "OpenAI completed without returning any text".to_string())
}

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
    let model = cfg.model.as_deref().unwrap_or(OPENAI_MODEL);

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

    completed_output(
        &response.status,
        response.error.as_ref(),
        response.incomplete_details.as_ref(),
        response.output_text(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_non_empty_completed_responses() {
        assert_eq!(
            completed_output(&Status::Completed, None, None, Some("answer".into())).unwrap(),
            "answer"
        );
        assert!(completed_output(&Status::Completed, None, None, Some("  ".into())).is_err());
    }

    #[test]
    fn surfaces_incomplete_and_failed_response_details() {
        let incomplete = IncompleteDetails {
            reason: "max_output_tokens".into(),
        };
        assert!(
            completed_output(
                &Status::Incomplete,
                None,
                Some(&incomplete),
                Some("partial".into())
            )
            .unwrap_err()
            .contains("max_output_tokens")
        );
        let error = ErrorObject {
            code: "server_error".into(),
            message: "generation failed".into(),
        };
        assert!(
            completed_output(&Status::Failed, Some(&error), None, Some("partial".into()))
                .unwrap_err()
                .contains("generation failed")
        );
    }
}
