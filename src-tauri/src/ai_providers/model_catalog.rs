//! Model listing for the Settings dropdowns.
//!
//! One command (`ai_list_models`) returns the model ids a provider can serve
//! right now: fetched live where the provider has a list API (OpenAI,
//! Anthropic, Gemini, DeepSeek, custom endpoints, Foundry Local, Ollama),
//! hardcoded where it doesn't (Codex CLI). The dialog passes its DRAFT
//! credentials per call so listing works before the user hits Save; stored
//! settings fill the gaps.

use crate::commands::settings::{load_provider_key_internal, read_settings_from_disk};
use crate::dpapi::ProviderKeyId;
use std::time::Duration;

const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// The Codex CLI has no model-list API; these ids come from OpenAI's Codex
/// model docs (July 2026). An empty selection means the CLI's own default.
pub const CODEX_MODELS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.3-codex-spark",
];

/// Claude Code likewise has no model-list API; `--model` accepts full ids
/// and the family aliases. Empty selection = the CLI's own default.
pub const CLAUDE_CODE_MODELS: &[&str] = &["sonnet", "opus", "haiku"];

/// List model ids for a provider's Settings dropdown. `api_key` and
/// `endpoint` are the dialog's unsaved draft values; empty/missing values
/// fall back to stored configuration.
#[tauri::command]
pub async fn ai_list_models(
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
) -> Result<Vec<String>, String> {
    let api_key = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let endpoint = endpoint
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());

    match provider.to_lowercase().as_str() {
        "openai" => {
            let key = require_key(api_key, ProviderKeyId::OpenAI, "OpenAI").await?;
            let json = get_json(
                "https://api.openai.com/v1/models",
                &[("authorization", format!("Bearer {key}"))],
            )
            .await?;
            Ok(sorted_desc(filter_openai_chat_models(parse_id_list(&json))))
        }
        "anthropic" => {
            let key = require_key(api_key, ProviderKeyId::Anthropic, "Anthropic").await?;
            let json = get_json(
                "https://api.anthropic.com/v1/models?limit=100",
                &[
                    ("x-api-key", key),
                    (
                        "anthropic-version",
                        super::anthropic::ANTHROPIC_VERSION.to_string(),
                    ),
                ],
            )
            .await?;
            // The API already returns newest first — keep its order
            Ok(parse_id_list(&json))
        }
        "gemini" => {
            let key = require_key(api_key, ProviderKeyId::Gemini, "Gemini").await?;
            // Key goes in a HEADER, never the URL (keys must not appear in URLs)
            let json = get_json(
                &format!("{}/models?pageSize=200", super::gemini::GEMINI_API_BASE),
                &[("x-goog-api-key", key)],
            )
            .await?;
            Ok(sorted_desc(parse_gemini_list(&json)))
        }
        "deepseek" => {
            let key = require_key(api_key, ProviderKeyId::DeepSeek, "DeepSeek").await?;
            let json = get_json(
                &format!("{}/v1/models", super::deepseek::DEEPSEEK_API_BASE),
                &[("authorization", format!("Bearer {key}"))],
            )
            .await?;
            Ok(sorted_desc(parse_id_list(&json)))
        }
        "custom_openai" | "custom" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let base = endpoint
                .as_deref()
                .or(settings.custom_endpoint.as_deref())
                .and_then(super::discovery::normalize_base_url)
                .ok_or_else(|| "Set the endpoint URL first to list models.".to_string())?;
            let key = match api_key {
                Some(k) => Some(k),
                None => load_provider_key_internal(ProviderKeyId::Custom).await,
            };
            let headers: Vec<(&str, String)> = key
                .map(|k| vec![("authorization", format!("Bearer {k}"))])
                .unwrap_or_default();
            let json = get_json(&format!("{base}/v1/models"), &headers).await?;
            Ok(sorted_desc(parse_id_list(&json)))
        }
        "foundry_local" | "foundrylocal" => {
            let base = match endpoint
                .as_deref()
                .and_then(super::discovery::normalize_base_url)
            {
                Some(base) => base,
                None => crate::ai_service::check_foundry_local_available()
                    .await
                    .ok_or_else(|| {
                        "Foundry Local is not running (try 'foundry service start').".to_string()
                    })?,
            };
            let json = get_json(&format!("{base}/v1/models"), &[]).await?;
            Ok(sorted_desc(parse_id_list(&json)))
        }
        "ollama" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let configured = endpoint.or(settings.ollama_endpoint);
            let base = super::ollama::discover_endpoint(configured.as_deref())
                .await
                .ok_or_else(|| "No Ollama server reachable.".to_string())?;
            super::ollama::list_models(&base).await
        }
        // No list API through the CLI — hardcoded catalog (empty = CLI default)
        "codex_cli" | "codexcli" | "codex" => {
            Ok(CODEX_MODELS.iter().map(|s| s.to_string()).collect())
        }
        "claude_code" | "claudecode" | "claude" => {
            Ok(CLAUDE_CODE_MODELS.iter().map(|s| s.to_string()).collect())
        }
        // On-device: there is exactly one model, nothing to choose
        "phi_silica" | "phisilica" => Ok(Vec::new()),
        other => Err(format!("Unknown provider: {other}")),
    }
}

/// Draft key from the dialog wins; else the stored per-provider key.
async fn require_key(
    passed: Option<String>,
    id: ProviderKeyId,
    label: &str,
) -> Result<String, String> {
    if let Some(key) = passed {
        return Ok(key);
    }
    load_provider_key_internal(id)
        .await
        .ok_or_else(|| format!("Add your {label} API key first to list models."))
}

async fn get_json(url: &str, headers: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let mut request = reqwest::Client::new().get(url).timeout(LIST_TIMEOUT);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Model list request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Model list request failed: HTTP {status}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Model list response was not JSON: {e}"))
}

/// `{"data":[{"id":"…"}]}` — the shape OpenAI, Anthropic, DeepSeek and
/// OpenAI-compatible servers all share.
fn parse_id_list(json: &serde_json::Value) -> Vec<String> {
    json.get("data")
        .and_then(|d| d.as_array())
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Gemini: `{"models":[{"name":"models/…","supportedGenerationMethods":[…]}]}`
/// — keep only chat-capable entries and strip the `models/` prefix.
fn parse_gemini_list(json: &serde_json::Value) -> Vec<String> {
    json.get("models")
        .and_then(|m| m.as_array())
        .map(|models| {
            models
                .iter()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(|g| g.as_array())
                        .is_some_and(|g| g.iter().any(|v| v.as_str() == Some("generateContent")))
                })
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .map(|name| name.trim_start_matches("models/").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// OpenAI's /v1/models mixes in embeddings/audio/image/moderation ids that
/// would fail a chat request — keep only the conversational families.
fn filter_openai_chat_models(ids: Vec<String>) -> Vec<String> {
    const EXCLUDE: &[&str] = &[
        "embed",
        "audio",
        "realtime",
        "tts",
        "whisper",
        "moderation",
        "image",
        "dall-e",
        "transcribe",
        "davinci",
        "babbage",
        "instruct",
        "search",
        "computer-use",
    ];
    ids.into_iter()
        .filter(|id| {
            (id.starts_with("gpt-")
                || id.starts_with("o1")
                || id.starts_with("o3")
                || id.starts_with("o4")
                || id.starts_with("chatgpt"))
                && !EXCLUDE.iter().any(|word| id.contains(word))
        })
        .collect()
}

/// Descending sort puts the newest version numbers first; dedup for servers
/// that list aliases twice.
fn sorted_desc(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids.reverse();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openai_style_id_lists() {
        let json = json!({"object":"list","data":[
            {"id":"gpt-5.2","object":"model"},
            {"id":"claude-sonnet-4-6","object":"model"},
        ]});
        assert_eq!(parse_id_list(&json), vec!["gpt-5.2", "claude-sonnet-4-6"]);
        assert!(parse_id_list(&json!({"error":"nope"})).is_empty());
    }

    #[test]
    fn gemini_list_keeps_only_generate_content_models_and_strips_prefix() {
        let json = json!({"models":[
            {"name":"models/gemini-2.5-flash","supportedGenerationMethods":["generateContent","countTokens"]},
            {"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]},
        ]});
        assert_eq!(parse_gemini_list(&json), vec!["gemini-2.5-flash"]);
    }

    #[test]
    fn openai_filter_drops_non_chat_families() {
        let ids = [
            "gpt-5.2",
            "gpt-5-nano",
            "o4-mini",
            "text-embedding-3-small",
            "gpt-4o-audio-preview",
            "whisper-1",
            "dall-e-3",
            "gpt-image-1",
            "omni-moderation-latest",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            filter_openai_chat_models(ids),
            vec!["gpt-5.2", "gpt-5-nano", "o4-mini"]
        );
    }

    #[test]
    fn sorted_desc_puts_newer_versions_first_and_dedups() {
        let ids = ["gpt-5", "gpt-5.2", "gpt-5", "gpt-5-mini"]
            .map(String::from)
            .to_vec();
        assert_eq!(sorted_desc(ids), vec!["gpt-5.2", "gpt-5-mini", "gpt-5"]);
    }

    #[test]
    fn hardcoded_bridge_models_pass_the_bridge_sanitizer() {
        // These go on the CLI command line via -m/--model, so they must
        // satisfy the cli_bridge allowlist
        for id in CODEX_MODELS.iter().chain(CLAUDE_CODE_MODELS) {
            assert_eq!(super::super::cli_bridge::sanitize_model(id).unwrap(), *id);
        }
    }
}
