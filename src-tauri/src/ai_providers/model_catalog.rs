//! Live model discovery for the Settings model picker.
//!
//! `ai_list_models` returns structured, provider-reported choices. The dialog
//! passes its DRAFT credentials, endpoint and CLI path so discovery works
//! before Save; stored settings fill only missing values. Catalogs are never
//! persisted and a failed discovery is returned as an error instead of being
//! hidden behind a stale hardcoded list.

use crate::commands::settings::{load_provider_key_internal, read_settings_from_disk};
use crate::dpapi::ProviderKeyId;
use serde::Serialize;
use std::collections::{HashSet, hash_map::RandomState};
use std::hash::BuildHasher;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_PAGES: usize = 50;
const GEMINI_DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
const GEMINI_FALLBACK_TTL: Duration = Duration::from_secs(60);

/// Optional provider metadata retained from a live model-list response.
///
/// Not every provider supplies these fields. Gemini does, and retaining them
/// lets ranking follow provider lifecycle/version information without a
/// hardcoded model allowlist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_token_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_token_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retirement_time: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_generation_methods: Vec<String>,
}

/// One selectable model exactly as the provider or CLI describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelCatalogMetadata>,
}

impl ModelCatalogEntry {
    fn id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: None,
            description: None,
            metadata: None,
        }
    }
}

/// Structured response consumed by the editable model picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<ModelCatalogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// List models for a provider. All optional values are unsaved Settings
/// drafts; empty/missing values fall back to stored configuration.
#[tauri::command]
pub async fn ai_list_models(
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
    cli_path: Option<String>,
) -> Result<ModelCatalog, String> {
    let api_key = non_empty(api_key);
    let endpoint = non_empty(endpoint);
    let cli_path = non_empty(cli_path);

    tokio::time::timeout(
        CATALOG_TIMEOUT,
        list_models_inner(provider, api_key, endpoint, cli_path),
    )
    .await
    .map_err(|_| {
        format!(
            "Model discovery did not finish within {} seconds.",
            CATALOG_TIMEOUT.as_secs()
        )
    })?
}

async fn list_models_inner(
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
    cli_path: Option<String>,
) -> Result<ModelCatalog, String> {
    match provider.to_lowercase().as_str() {
        "openai" => {
            let key = require_key(api_key, ProviderKeyId::OpenAI, "OpenAI").await?;
            let json = get_json(
                "https://api.openai.com/v1/models",
                &[("authorization", format!("Bearer {key}"))],
                &[],
            )
            .await?;
            Ok(ModelCatalog {
                models: entries_from_ids(sorted_desc(filter_openai_chat_models(parse_id_list(
                    &json,
                )))),
                default_model: Some(super::openai::OPENAI_MODEL.to_string()),
            })
        }
        "anthropic" => {
            let key = require_key(api_key, ProviderKeyId::Anthropic, "Anthropic").await?;
            Ok(ModelCatalog {
                // Anthropic documents newest-first ordering; preserve it
                // across pages instead of applying a lexical sort.
                models: list_anthropic_models(&key).await?,
                default_model: Some(super::anthropic::ANTHROPIC_DEFAULT_MODEL.to_string()),
            })
        }
        "gemini" => {
            let key = require_key(api_key, ProviderKeyId::Gemini, "Gemini").await?;
            let models = list_gemini_models(&key).await?;
            Ok(ModelCatalog {
                default_model: best_gemini_catalog_default(&models),
                models,
            })
        }
        "deepseek" => {
            let key = require_key(api_key, ProviderKeyId::DeepSeek, "DeepSeek").await?;
            let json = get_json(
                &format!("{}/v1/models", super::deepseek::DEEPSEEK_API_BASE),
                &[("authorization", format!("Bearer {key}"))],
                &[],
            )
            .await?;
            Ok(ModelCatalog {
                models: entries_from_ids(sorted_desc(parse_id_list(&json))),
                default_model: Some(super::deepseek::DEEPSEEK_DEFAULT_MODEL.to_string()),
            })
        }
        "custom_openai" | "custom" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let base = endpoint
                .as_deref()
                .or(settings.custom_endpoint.as_deref())
                .and_then(super::discovery::normalize_base_url)
                .ok_or_else(|| "Set the endpoint URL first to list models.".to_string())?;
            let key = match api_key {
                Some(key) => Some(key),
                None => load_provider_key_internal(ProviderKeyId::Custom).await,
            };
            let headers: Vec<(&str, String)> = key
                .map(|key| vec![("authorization", format!("Bearer {key}"))])
                .unwrap_or_default();
            let json = get_json(&format!("{base}/v1/models"), &headers, &[]).await?;
            Ok(ModelCatalog {
                models: entries_from_ids(sorted_desc(parse_id_list(&json))),
                default_model: None,
            })
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
            Ok(ModelCatalog {
                models: entries_from_ids(sorted_desc(super::foundry::list_models(&base).await?)),
                default_model: Some(super::foundry::FOUNDRY_LOCAL_MODEL.to_string()),
            })
        }
        "ollama" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let configured = endpoint.or(settings.ollama_endpoint);
            let base = super::ollama::discover_endpoint(configured.as_deref())
                .await
                .ok_or_else(|| "No Ollama server reachable.".to_string())?;
            let ids = stable_dedupe(super::ollama::list_models(&base).await?);
            Ok(ModelCatalog {
                default_model: ids.first().cloned(),
                models: entries_from_ids(ids),
            })
        }
        "codex_cli" | "codexcli" | "codex" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let configured = cli_path.or(settings.codex_cli_path);
            Ok(super::cli_bridge::list_codex_models(configured.as_deref())
                .await?
                .into())
        }
        "claude_code" | "claudecode" | "claude" => {
            let settings = read_settings_from_disk().unwrap_or_default();
            let configured = cli_path.or(settings.claude_cli_path);
            let claude = super::cli_bridge::resolve_cli("claude", configured.as_deref()).await?;
            Ok(super::acp_bridge::list_claude_models(&claude).await?.into())
        }
        // The on-device model is selected by Windows, not by the app.
        "phi_silica" | "phisilica" => Ok(ModelCatalog::default()),
        other => Err(format!("Unknown provider: {other}")),
    }
}

impl From<super::cli_bridge::BridgeModelCatalog> for ModelCatalog {
    fn from(catalog: super::cli_bridge::BridgeModelCatalog) -> Self {
        Self {
            models: catalog
                .models
                .into_iter()
                .map(|model| ModelCatalogEntry {
                    id: model.id,
                    label: model.label,
                    description: model.description,
                    metadata: None,
                })
                .collect(),
            default_model: catalog.default_model,
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Draft key from the dialog wins; otherwise use the stored per-provider key.
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

async fn get_json(
    url: &str,
    headers: &[(&str, String)],
    query: &[(&str, String)],
) -> Result<serde_json::Value, String> {
    let mut request = reqwest::Client::new()
        .get(url)
        .query(query)
        .timeout(REQUEST_TIMEOUT);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Model list request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Model list request failed: HTTP {status}"));
    }
    response
        .json()
        .await
        .map_err(|error| format!("Model list response was not JSON: {error}"))
}

async fn list_anthropic_models(api_key: &str) -> Result<Vec<ModelCatalogEntry>, String> {
    let headers = [
        ("x-api-key", api_key.to_string()),
        (
            "anthropic-version",
            super::anthropic::ANTHROPIC_VERSION.to_string(),
        ),
    ];
    let mut models = Vec::new();
    let mut after_id: Option<String> = None;
    let mut seen_cursors = HashSet::new();

    for _ in 0..MAX_PAGES {
        let mut query = vec![("limit", "100".to_string())];
        if let Some(cursor) = after_id.as_ref() {
            query.push(("after_id", cursor.clone()));
        }
        let json = get_json("https://api.anthropic.com/v1/models", &headers, &query).await?;
        models.extend(parse_anthropic_list(&json));

        if !json
            .get("has_more")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(stable_dedupe_entries(models));
        }
        let cursor = json
            .get("last_id")
            .and_then(serde_json::Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .ok_or_else(|| {
                "Anthropic model list said another page exists but returned no last_id.".to_string()
            })?
            .to_string();
        if !seen_cursors.insert(cursor.clone()) {
            return Err("Anthropic model list repeated a pagination cursor.".to_string());
        }
        after_id = Some(cursor);
    }

    Err("Anthropic model list exceeded the pagination limit.".to_string())
}

async fn list_gemini_models(api_key: &str) -> Result<Vec<ModelCatalogEntry>, String> {
    let headers = [("x-goog-api-key", api_key.to_string())];
    let mut models = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_cursors = HashSet::new();

    for _ in 0..MAX_PAGES {
        let mut query = vec![("pageSize", "200".to_string())];
        if let Some(cursor) = page_token.as_ref() {
            query.push(("pageToken", cursor.clone()));
        }
        let json = get_json(
            &format!("{}/models", super::gemini::GEMINI_API_BASE),
            &headers,
            &query,
        )
        .await?;
        models.extend(parse_gemini_list(&json));

        let Some(cursor) = json
            .get("nextPageToken")
            .and_then(serde_json::Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string)
        else {
            return Ok(rank_gemini_models(stable_dedupe_entries(models)));
        };
        if !seen_cursors.insert(cursor.clone()) {
            return Err("Gemini model list repeated a pagination token.".to_string());
        }
        page_token = Some(cursor);
    }

    Err("Gemini model list exceeded the pagination limit.".to_string())
}

/// `{"data":[{"id":"…"}]}` — shared by OpenAI, DeepSeek and most
/// OpenAI-compatible servers.
fn parse_id_list(json: &serde_json::Value) -> Vec<String> {
    json.get("data")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_anthropic_list(json: &serde_json::Value) -> Vec<ModelCatalogEntry> {
    json.get("data")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    let id = model.get("id")?.as_str()?.to_string();
                    Some(ModelCatalogEntry {
                        label: optional_distinct_string(
                            model
                                .get("display_name")
                                .and_then(serde_json::Value::as_str),
                            &id,
                        ),
                        id,
                        description: None,
                        metadata: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Gemini: keep only models that advertise `generateContent`, strip the
/// resource prefix and preserve provider labels, descriptions, version,
/// limits, capabilities and any lifecycle metadata the API returns.
///
/// Google's standard Model resource does not guarantee lifecycle fields, so
/// `modelStatus` is parsed opportunistically. Only an explicitly deprecated
/// or retired stage is excluded; unknown/future entries remain selectable.
fn parse_gemini_list(json: &serde_json::Value) -> Vec<ModelCatalogEntry> {
    json.get("models")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter(|model| {
                    model
                        .get("supportedGenerationMethods")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|methods| {
                            methods
                                .iter()
                                .any(|value| value.as_str() == Some("generateContent"))
                        })
                })
                .filter_map(|model| {
                    let id = model
                        .get("name")?
                        .as_str()?
                        .trim_start_matches("models/")
                        .to_string();
                    let model_stage = model
                        .pointer("/modelStatus/modelStage")
                        .or_else(|| model.get("modelStage"))
                        .and_then(serde_json::Value::as_str)
                        .and_then(|stage| non_empty_str(Some(stage)));
                    if model_stage
                        .as_deref()
                        .and_then(GeminiLifecycle::from_provider_value)
                        .is_some_and(GeminiLifecycle::is_unusable)
                    {
                        return None;
                    }
                    let supported_generation_methods = model
                        .get("supportedGenerationMethods")
                        .and_then(serde_json::Value::as_array)
                        .map(|methods| {
                            methods
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(ModelCatalogEntry {
                        label: optional_distinct_string(
                            model.get("displayName").and_then(serde_json::Value::as_str),
                            &id,
                        ),
                        description: non_empty_str(
                            model.get("description").and_then(serde_json::Value::as_str),
                        ),
                        metadata: Some(ModelCatalogMetadata {
                            base_model_id: non_empty_str(
                                model.get("baseModelId").and_then(serde_json::Value::as_str),
                            ),
                            version: non_empty_str(
                                model.get("version").and_then(serde_json::Value::as_str),
                            ),
                            input_token_limit: model
                                .get("inputTokenLimit")
                                .and_then(serde_json::Value::as_u64),
                            output_token_limit: model
                                .get("outputTokenLimit")
                                .and_then(serde_json::Value::as_u64),
                            thinking: model.get("thinking").and_then(serde_json::Value::as_bool),
                            model_stage,
                            retirement_time: model
                                .pointer("/modelStatus/retirementTime")
                                .or_else(|| model.get("retirementTime"))
                                .and_then(serde_json::Value::as_str)
                                .and_then(|value| non_empty_str(Some(value))),
                            supported_generation_methods,
                        }),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeminiLifecycle {
    Stable,
    Unspecified,
    Preview,
    Experimental,
    Legacy,
    Deprecated,
    Retired,
}

impl GeminiLifecycle {
    fn from_provider_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "STABLE" => Some(Self::Stable),
            "MODEL_STAGE_UNSPECIFIED" | "UNSPECIFIED" => Some(Self::Unspecified),
            "PREVIEW" => Some(Self::Preview),
            "EXPERIMENTAL" | "UNSTABLE" => Some(Self::Experimental),
            "LEGACY" => Some(Self::Legacy),
            "DEPRECATED" => Some(Self::Deprecated),
            "RETIRED" => Some(Self::Retired),
            _ => None,
        }
    }

    fn is_unusable(self) -> bool {
        matches!(self, Self::Deprecated | Self::Retired)
    }

    fn rank(self) -> u8 {
        match self {
            Self::Stable => 5,
            Self::Unspecified => 4,
            Self::Preview => 3,
            Self::Experimental => 2,
            Self::Legacy => 1,
            Self::Deprecated | Self::Retired => 0,
        }
    }
}

fn explicit_gemini_lifecycle(model: &ModelCatalogEntry) -> Option<GeminiLifecycle> {
    model
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.model_stage.as_deref())
        .and_then(GeminiLifecycle::from_provider_value)
}

fn gemini_tokens(model: &ModelCatalogEntry) -> Vec<String> {
    let mut text = model.id.to_ascii_lowercase();
    if let Some(base) = model
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.base_model_id.as_deref())
    {
        text.push('-');
        text.push_str(&base.to_ascii_lowercase());
    }
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn inferred_gemini_lifecycle(model: &ModelCatalogEntry) -> GeminiLifecycle {
    if let Some(stage) = explicit_gemini_lifecycle(model) {
        return stage;
    }
    let tokens = gemini_tokens(model);
    if tokens.iter().any(|token| token == "preview") {
        GeminiLifecycle::Preview
    } else if tokens
        .iter()
        .any(|token| token == "experimental" || token == "exp")
    {
        GeminiLifecycle::Experimental
    } else if tokens.iter().any(|token| token == "legacy") {
        GeminiLifecycle::Legacy
    } else if semantic_gemini_version(model).is_some()
        && !tokens.iter().any(|token| token == "latest")
    {
        // Stable Gemini IDs are versioned and do not carry a preview/exp
        // suffix. This inference is needed because /models does not promise a
        // lifecycle field.
        GeminiLifecycle::Stable
    } else {
        GeminiLifecycle::Unspecified
    }
}

fn semantic_gemini_version(model: &ModelCatalogEntry) -> Option<(u32, u32, u32)> {
    let base = model
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.base_model_id.as_deref());
    [base, Some(model.id.as_str())]
        .into_iter()
        .flatten()
        .find_map(|candidate| {
            let tokens = candidate
                .trim_start_matches("models/")
                .split(['-', '_'])
                .collect::<Vec<_>>();
            let version = tokens
                .windows(2)
                .find(|pair| pair[0].eq_ignore_ascii_case("gemini"))
                .map(|pair| pair[1])?;
            let mut parts = version.split('.');
            let major = parts.next()?.parse().ok()?;
            let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
            let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
            Some((major, minor, patch))
        })
}

fn dated_gemini_suffix(model: &ModelCatalogEntry) -> u32 {
    let tokens = gemini_tokens(model);
    for (index, token) in tokens.iter().enumerate() {
        if token.len() == 8
            && let Ok(value) = token.parse::<u32>()
            && (20_000_101..=29_991_231).contains(&value)
        {
            return value;
        }
        if token.len() == 4
            && let Ok(year) = token.parse::<u32>()
            && (2000..=2999).contains(&year)
        {
            let previous = index
                .checked_sub(1)
                .and_then(|position| tokens.get(position))
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|month| (1..=12).contains(month));
            let next = tokens
                .get(index + 1)
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|month| (1..=12).contains(month));
            return year * 10_000 + previous.or(next).unwrap_or(1) * 100 + 1;
        }
    }
    0
}

fn is_gemini_specialized(model: &ModelCatalogEntry) -> bool {
    const SPECIALIZED_TOKENS: &[&str] = &[
        "image",
        "imagen",
        "tts",
        "audio",
        "live",
        "realtime",
        "embedding",
        "embed",
        "aqa",
        "robotics",
        "computer",
        "video",
        "veo",
        "lyria",
        "research",
    ];
    gemini_tokens(model)
        .iter()
        .any(|token| SPECIALIZED_TOKENS.contains(&token.as_str()))
}

fn is_concrete_gemini_model(model: &ModelCatalogEntry) -> bool {
    let tokens = gemini_tokens(model);
    if tokens.iter().any(|token| token == "latest") {
        return false;
    }
    semantic_gemini_version(model).is_some()
        || explicit_gemini_lifecycle(model) == Some(GeminiLifecycle::Stable)
}

fn gemini_rank(model: &ModelCatalogEntry) -> (u8, u8, u8, (u32, u32, u32), u32) {
    (
        u8::from(!is_gemini_specialized(model)),
        inferred_gemini_lifecycle(model).rank(),
        u8::from(is_concrete_gemini_model(model)),
        semantic_gemini_version(model).unwrap_or_default(),
        dated_gemini_suffix(model),
    )
}

fn rank_gemini_models(mut models: Vec<ModelCatalogEntry>) -> Vec<ModelCatalogEntry> {
    // `sort_by` is stable: provider order remains the final tie-breaker.
    models.sort_by_key(|model| std::cmp::Reverse(gemini_rank(model)));
    models
}

fn is_stable_general_gemini(model: &ModelCatalogEntry) -> bool {
    !is_gemini_specialized(model)
        && is_concrete_gemini_model(model)
        && inferred_gemini_lifecycle(model) == GeminiLifecycle::Stable
}

fn best_gemini_catalog_default(models: &[ModelCatalogEntry]) -> Option<String> {
    models
        .iter()
        .find(|model| is_stable_general_gemini(model))
        .or_else(|| {
            models
                .iter()
                .find(|model| !is_gemini_specialized(model) && is_concrete_gemini_model(model))
        })
        .or_else(|| models.iter().find(|model| !is_gemini_specialized(model)))
        .map(|model| model.id.clone())
}

fn best_gemini_runtime_default(models: &[ModelCatalogEntry]) -> Option<String> {
    models
        .iter()
        .find(|model| is_stable_general_gemini(model))
        .map(|model| model.id.clone())
}

#[derive(Debug, Clone)]
struct CachedGeminiDefault {
    key_fingerprint: u64,
    model: String,
    expires_at: Instant,
}

impl CachedGeminiDefault {
    fn is_fresh_for(&self, key_fingerprint: u64, now: Instant) -> bool {
        self.key_fingerprint == key_fingerprint && now < self.expires_at
    }
}

fn gemini_default_cache() -> &'static AsyncMutex<Option<CachedGeminiDefault>> {
    static CACHE: OnceLock<AsyncMutex<Option<CachedGeminiDefault>>> = OnceLock::new();
    CACHE.get_or_init(|| AsyncMutex::new(None))
}

fn key_fingerprint(api_key: &str) -> u64 {
    static HASHER: OnceLock<RandomState> = OnceLock::new();
    HASHER.get_or_init(RandomState::new).hash_one(api_key)
}

/// Resolve the current concrete stable general-chat model for blank Gemini
/// settings. The cache is deliberately capacity one and keyed by a
/// randomized process-local fingerprint, so API keys are never retained
/// in catalog state. Transient failures use a shorter cache window.
pub(crate) async fn resolve_gemini_default_model(api_key: &str) -> String {
    let fingerprint = key_fingerprint(api_key);
    let mut cache = gemini_default_cache().lock().await;
    let now = Instant::now();
    if let Some(cached) = cache.as_ref()
        && cached.is_fresh_for(fingerprint, now)
    {
        return cached.model.clone();
    }

    let discovered = tokio::time::timeout(CATALOG_TIMEOUT, list_gemini_models(api_key)).await;
    let (model, ttl) = match discovered {
        Ok(Ok(models)) => match best_gemini_runtime_default(&models) {
            Some(model) => (model, GEMINI_DEFAULT_TTL),
            None => (
                super::gemini::GEMINI_DEFAULT_MODEL.to_string(),
                GEMINI_FALLBACK_TTL,
            ),
        },
        Ok(Err(_)) | Err(_) => (
            super::gemini::GEMINI_DEFAULT_MODEL.to_string(),
            GEMINI_FALLBACK_TTL,
        ),
    };
    *cache = Some(CachedGeminiDefault {
        key_fingerprint: fingerprint,
        model: model.clone(),
        expires_at: now + ttl,
    });
    model
}

/// `/v1/models` mixes chat models with endpoints this app cannot invoke.
/// Exclude only well-known non-chat families; do not positively allowlist
/// prefixes, because that would hide every future conversational family.
fn filter_openai_chat_models(ids: Vec<String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| {
            let id = id.to_ascii_lowercase();
            const NON_CHAT_MARKERS: &[&str] = &[
                "embedding",
                "moderation",
                "whisper",
                "transcribe",
                "tts",
                "dall-e",
                "gpt-image",
                "audio",
                "realtime",
                "search",
                "computer-use",
                "sora",
                "video",
                "instruct",
            ];
            !NON_CHAT_MARKERS.iter().any(|marker| id.contains(marker))
                && !id.starts_with("text-davinci")
                && !id.starts_with("text-babbage")
                && !id.starts_with("text-curie")
                && !id.starts_with("text-ada")
                && !id.starts_with("davinci")
                && !id.starts_with("babbage")
        })
        .collect()
}

fn optional_distinct_string(value: Option<&str>, id: &str) -> Option<String> {
    non_empty_str(value).filter(|value| value != id)
}

fn non_empty_str(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn entries_from_ids(ids: Vec<String>) -> Vec<ModelCatalogEntry> {
    ids.into_iter().map(ModelCatalogEntry::id).collect()
}

/// Descending lexical order matches the previous picker behavior while
/// remaining deterministic for providers that do not define an order.
fn sorted_desc(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids.reverse();
    ids
}

fn stable_dedupe(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

fn stable_dedupe_entries(models: Vec<ModelCatalogEntry>) -> Vec<ModelCatalogEntry> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_catalog_serializes_in_camel_case() {
        let catalog = ModelCatalog {
            models: vec![ModelCatalogEntry {
                id: "gpt-next".into(),
                label: Some("GPT Next".into()),
                description: None,
                metadata: None,
            }],
            default_model: Some("gpt-next".into()),
        };
        assert_eq!(
            serde_json::to_value(catalog).unwrap(),
            json!({
                "models": [{"id":"gpt-next","label":"GPT Next"}],
                "defaultModel":"gpt-next"
            })
        );
    }

    #[test]
    fn parses_openai_style_id_lists() {
        let json = json!({"object":"list","data":[
            {"id":"gpt-5.6-sol","object":"model"},
            {"id":"future-conversation-1","object":"model"},
        ]});
        assert_eq!(
            parse_id_list(&json),
            vec!["gpt-5.6-sol", "future-conversation-1"]
        );
        assert!(parse_id_list(&json!({"error":"nope"})).is_empty());
    }

    #[test]
    fn anthropic_parser_preserves_labels_and_order() {
        let json = json!({"data":[
            {"id":"claude-opus-5","display_name":"Claude Opus 5"},
            {"id":"claude-sonnet-5","display_name":"Claude Sonnet 5"}
        ],"has_more":true,"last_id":"claude-sonnet-5"});
        assert_eq!(
            parse_anthropic_list(&json),
            vec![
                ModelCatalogEntry {
                    id: "claude-opus-5".into(),
                    label: Some("Claude Opus 5".into()),
                    description: None,
                    metadata: None,
                },
                ModelCatalogEntry {
                    id: "claude-sonnet-5".into(),
                    label: Some("Claude Sonnet 5".into()),
                    description: None,
                    metadata: None,
                },
            ]
        );
    }

    #[test]
    fn gemini_parser_keeps_generation_models_and_metadata() {
        let json = json!({"models":[
            {
                "name":"models/gemini-next-flash",
                "baseModelId":"gemini-next",
                "version":"next-001",
                "displayName":"Gemini Next Flash",
                "description":"Fast generation",
                "inputTokenLimit":1048576,
                "outputTokenLimit":65536,
                "thinking":true,
                "modelStatus":{
                    "modelStage":"STABLE",
                    "retirementTime":"2027-01-01T00:00:00Z"
                },
                "supportedGenerationMethods":["generateContent","countTokens"]
            },
            {
                "name":"models/text-embedding-004",
                "supportedGenerationMethods":["embedContent"]
            }
        ],"nextPageToken":"page-2"});
        assert_eq!(
            parse_gemini_list(&json),
            vec![ModelCatalogEntry {
                id: "gemini-next-flash".into(),
                label: Some("Gemini Next Flash".into()),
                description: Some("Fast generation".into()),
                metadata: Some(ModelCatalogMetadata {
                    base_model_id: Some("gemini-next".into()),
                    version: Some("next-001".into()),
                    input_token_limit: Some(1_048_576),
                    output_token_limit: Some(65_536),
                    thinking: Some(true),
                    model_stage: Some("STABLE".into()),
                    retirement_time: Some("2027-01-01T00:00:00Z".into()),
                    supported_generation_methods: vec![
                        "generateContent".into(),
                        "countTokens".into(),
                    ],
                }),
            }]
        );
        let serialized = serde_json::to_value(&parse_gemini_list(&json)[0]).unwrap();
        assert_eq!(serialized["metadata"]["baseModelId"], "gemini-next");
        assert_eq!(serialized["metadata"]["inputTokenLimit"], 1_048_576);
    }

    #[test]
    fn gemini_ranking_is_semantic_and_lifecycle_aware() {
        let json = json!({"models":[
            {
                "name":"models/gemini-4.0-pro-preview",
                "modelStage":"PREVIEW",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-3.5-flash",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-3.10-flash",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-3.6-flash",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-flash-latest",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-9.0-legacy",
                "modelStage":"LEGACY",
                "supportedGenerationMethods":["generateContent"]
            }
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(
            ranked
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gemini-3.10-flash",
                "gemini-3.6-flash",
                "gemini-3.5-flash",
                "gemini-flash-latest",
                "gemini-4.0-pro-preview",
                "gemini-9.0-legacy",
            ]
        );
        assert_eq!(
            best_gemini_catalog_default(&ranked).as_deref(),
            Some("gemini-3.10-flash")
        );
        assert_eq!(
            best_gemini_runtime_default(&ranked).as_deref(),
            Some("gemini-3.10-flash")
        );
    }

    #[test]
    fn gemini_excludes_only_explicit_deprecated_or_retired_models() {
        let json = json!({"models":[
            {
                "name":"models/gemini-retired-name-but-unflagged",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-future-family",
                "modelStage":"A_FUTURE_STAGE",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-old",
                "modelStatus":{"modelStage":"DEPRECATED"},
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-gone",
                "modelStage":"RETIRED",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-legacy",
                "modelStage":"LEGACY",
                "supportedGenerationMethods":["generateContent"]
            }
        ]});
        let parsed = parse_gemini_list(&json);
        assert_eq!(
            parsed
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gemini-retired-name-but-unflagged",
                "gemini-future-family",
                "gemini-legacy"
            ]
        );
    }

    #[test]
    fn gemini_dated_previews_sort_chronologically() {
        let json = json!({"models":[
            {
                "name":"models/gemini-4.0-flash-preview-12-2025",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-4.0-flash-preview-09-2026",
                "supportedGenerationMethods":["generateContent"]
            }
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(ranked[0].id, "gemini-4.0-flash-preview-09-2026");
    }

    #[test]
    fn specialized_models_remain_visible_but_never_become_the_chat_default() {
        let json = json!({"models":[
            {
                "name":"models/gemini-9.0-live",
                "modelStage":"STABLE",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-8.0-pro-image",
                "modelStage":"STABLE",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-7.0-flash-preview-tts",
                "modelStage":"PREVIEW",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/gemini-3.6-flash",
                "modelStage":"STABLE",
                "supportedGenerationMethods":["generateContent"]
            }
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(ranked.len(), 4, "specialized entries stay selectable");
        assert_eq!(ranked[0].id, "gemini-3.6-flash");
        assert_eq!(
            best_gemini_runtime_default(&ranked).as_deref(),
            Some("gemini-3.6-flash")
        );

        let specialized_only = ranked
            .into_iter()
            .filter(is_gemini_specialized)
            .collect::<Vec<_>>();
        assert_eq!(best_gemini_catalog_default(&specialized_only), None);
        assert_eq!(best_gemini_runtime_default(&specialized_only), None);
    }

    #[test]
    fn unknown_future_models_are_retained_and_provider_order_breaks_ties() {
        let json = json!({"models":[
            {
                "name":"models/gemini-orion",
                "supportedGenerationMethods":["generateContent"]
            },
            {
                "name":"models/future-conversation-family",
                "supportedGenerationMethods":["generateContent"]
            }
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(
            ranked
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gemini-orion", "future-conversation-family"]
        );
    }

    #[test]
    fn gemini_default_cache_is_keyed_and_expires() {
        let now = Instant::now();
        let cached = CachedGeminiDefault {
            key_fingerprint: 42,
            model: "gemini-test".into(),
            expires_at: now + Duration::from_secs(10),
        };
        assert!(cached.is_fresh_for(42, now));
        assert!(!cached.is_fresh_for(7, now));
        assert!(!cached.is_fresh_for(42, now + Duration::from_secs(10)));
    }

    #[test]
    fn openai_filter_accepts_unknown_future_families() {
        let ids = [
            "gpt-5.6-sol",
            "o4-mini",
            "future-conversation-1",
            "text-embedding-3-small",
            "gpt-4o-audio-preview",
            "whisper-1",
            "dall-e-3",
            "gpt-image-1",
            "omni-moderation-latest",
            "sora-2",
            "gpt-3.5-turbo-instruct",
            "davinci-002",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            filter_openai_chat_models(ids),
            vec!["gpt-5.6-sol", "o4-mini", "future-conversation-1"]
        );
    }

    #[test]
    fn deterministic_and_provider_order_dedupe_are_distinct() {
        let ids = ["gpt-5", "gpt-5.6", "gpt-5", "gpt-5-mini"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            sorted_desc(ids.clone()),
            vec!["gpt-5.6", "gpt-5-mini", "gpt-5"]
        );
        assert_eq!(stable_dedupe(ids), vec!["gpt-5", "gpt-5.6", "gpt-5-mini"]);
    }

    #[test]
    fn stable_entry_dedupe_keeps_first_page_metadata() {
        let entries = vec![
            ModelCatalogEntry {
                id: "claude-opus-5".into(),
                label: Some("Newest label".into()),
                description: None,
                metadata: None,
            },
            ModelCatalogEntry {
                id: "claude-opus-5".into(),
                label: Some("Duplicate".into()),
                description: None,
                metadata: None,
            },
        ];
        let deduped = stable_dedupe_entries(entries);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].label.as_deref(), Some("Newest label"));
    }
}
