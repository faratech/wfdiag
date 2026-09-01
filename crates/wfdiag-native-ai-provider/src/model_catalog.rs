//! Framework-neutral live model discovery used by every desktop shell.
//!
//! Gemini intentionally resolves a blank model setting against Google's live
//! catalog. The catalog parser, lifecycle-aware ranking, credential-scoped
//! cache, and outage fallback live here so Tauri and Windows Reactor cannot
//! silently choose different models.

use serde::{Deserialize, Serialize};
use std::collections::{HashSet, hash_map::RandomState};
use std::hash::BuildHasher;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

/// Last-known GA Gemini model used only when live discovery is unavailable.
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3.6-flash";
/// Google Generative Language API root used by Gemini generation and catalog calls.
pub const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-catalog budget. MUST stay below the callers' 10s model-resolve
/// deadline (and below `REQUEST_TIMEOUT` per page) so a slow catalog surfaces
/// its own timeout and falls back to the default model instead of being cut
/// off by the outer resolve limit.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_PAGES: usize = 50;
const GEMINI_DEFAULT_TTL: Duration = Duration::from_mins(15);
const GEMINI_FALLBACK_TTL: Duration = Duration::from_secs(60);

/// Optional provider metadata retained from a live model-list response.
///
/// Not every provider supplies these fields. Gemini does, and retaining them
/// lets ranking follow provider lifecycle/version information without a
/// hardcoded model allowlist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// One selectable model exactly as its provider describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Construct an entry for providers whose catalog supplies only an ID.
    pub fn from_id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: None,
            description: None,
            metadata: None,
        }
    }
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

    const fn is_unusable(self) -> bool {
        matches!(self, Self::Deprecated | Self::Retired)
    }

    const fn rank(self) -> u8 {
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

fn non_empty_str(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_distinct_string(value: Option<&str>, id: &str) -> Option<String> {
    non_empty_str(value).filter(|value| value != id)
}

/// Parse Gemini's Model resources, retaining only `generateContent` models.
///
/// Google's standard resource does not guarantee lifecycle fields, so
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

/// Pick the best general model for the editable Settings catalog.
#[must_use]
pub fn best_gemini_catalog_default(models: &[ModelCatalogEntry]) -> Option<String> {
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

fn stable_dedupe_entries(models: Vec<ModelCatalogEntry>) -> Vec<ModelCatalogEntry> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
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

fn key_fingerprint(api_key: &str) -> u64 {
    static HASHER: OnceLock<RandomState> = OnceLock::new();
    HASHER.get_or_init(RandomState::new).hash_one(api_key)
}

struct GeminiModelResolver {
    api_base: String,
    request_timeout: Duration,
    catalog_timeout: Duration,
    success_ttl: Duration,
    fallback_ttl: Duration,
    cache: AsyncMutex<Option<CachedGeminiDefault>>,
}

impl GeminiModelResolver {
    fn production() -> Self {
        Self {
            api_base: GEMINI_API_BASE.to_string(),
            request_timeout: REQUEST_TIMEOUT,
            catalog_timeout: CATALOG_TIMEOUT,
            success_ttl: GEMINI_DEFAULT_TTL,
            fallback_ttl: GEMINI_FALLBACK_TTL,
            cache: AsyncMutex::new(None),
        }
    }

    #[cfg(test)]
    fn hermetic(api_base: String) -> Self {
        Self {
            api_base,
            request_timeout: Duration::from_secs(2),
            catalog_timeout: Duration::from_secs(3),
            success_ttl: Duration::from_secs(60),
            fallback_ttl: Duration::from_secs(60),
            cache: AsyncMutex::new(None),
        }
    }

    async fn get_json(
        &self,
        url: &str,
        api_key: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, String> {
        let response = reqwest::Client::new()
            .get(url)
            .query(query)
            // Authentication stays in a header so secrets never enter URLs,
            // access logs, or request-error strings.
            .header("x-goog-api-key", api_key)
            .timeout(self.request_timeout)
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

    async fn list_models(&self, api_key: &str) -> Result<Vec<ModelCatalogEntry>, String> {
        let mut models = Vec::new();
        let mut page_token: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for _ in 0..MAX_PAGES {
            let mut query = vec![("pageSize", "200".to_string())];
            if let Some(cursor) = page_token.as_ref() {
                query.push(("pageToken", cursor.clone()));
            }
            let json = self
                .get_json(&format!("{}/models", self.api_base), api_key, &query)
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

    async fn resolve_default_model(&self, api_key: &str) -> String {
        let fingerprint = key_fingerprint(api_key);
        let mut cache = self.cache.lock().await;
        let now = Instant::now();
        if let Some(cached) = cache.as_ref()
            && cached.is_fresh_for(fingerprint, now)
        {
            return cached.model.clone();
        }

        let discovered =
            tokio::time::timeout(self.catalog_timeout, self.list_models(api_key)).await;
        let (model, ttl) = match discovered {
            Ok(Ok(models)) => match best_gemini_runtime_default(&models) {
                Some(model) => (model, self.success_ttl),
                None => (GEMINI_DEFAULT_MODEL.to_string(), self.fallback_ttl),
            },
            Ok(Err(_)) | Err(_) => (GEMINI_DEFAULT_MODEL.to_string(), self.fallback_ttl),
        };
        // Anchor freshness AFTER the fetch: a multi-second catalog round trip
        // must not shorten the entry's effective TTL.
        *cache = Some(CachedGeminiDefault {
            key_fingerprint: fingerprint,
            model: model.clone(),
            expires_at: Instant::now() + ttl,
        });
        model
    }
}

fn gemini_resolver() -> &'static GeminiModelResolver {
    static RESOLVER: OnceLock<GeminiModelResolver> = OnceLock::new();
    RESOLVER.get_or_init(GeminiModelResolver::production)
}

/// Fetch and rank every selectable Gemini generation model.
pub async fn list_gemini_models(api_key: &str) -> Result<Vec<ModelCatalogEntry>, String> {
    gemini_resolver().list_models(api_key).await
}

/// Resolve the current concrete stable general-chat model for a blank Gemini
/// setting. The cache is capacity one and keyed by a randomized process-local
/// fingerprint, so credentials are never retained in catalog state.
pub async fn resolve_gemini_default_model(api_key: &str) -> String {
    gemini_resolver().resolve_default_model(api_key).await
}

/// Preserve an explicit Gemini model choice, otherwise perform shared live
/// discovery with the last-known GA outage fallback.
pub async fn resolve_gemini_model(configured: Option<&str>, api_key: &str) -> String {
    if let Some(model) = configured.map(str::trim).filter(|model| !model.is_empty()) {
        return model.to_string();
    }
    resolve_gemini_default_model(api_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct MockCatalog {
        api_base: String,
        requests: Arc<Mutex<Vec<String>>>,
        worker: thread::JoinHandle<()>,
    }

    fn mock_catalog(responses: Vec<(u16, serde_json::Value)>) -> MockCatalog {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            for (status, body) in responses {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "mock catalog request timed out");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("mock catalog accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 2048];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).unwrap();
                    assert_ne!(read, 0, "request closed before its headers");
                    bytes.extend_from_slice(&buffer[..read]);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Internal Server Error"
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });
        MockCatalog {
            api_base: format!("http://{address}/v1beta"),
            requests,
            worker,
        }
    }

    fn generation_model(id: &str) -> serde_json::Value {
        json!({
            "name": format!("models/{id}"),
            "supportedGenerationMethods": ["generateContent"]
        })
    }

    #[test]
    fn parser_keeps_generation_models_and_metadata() {
        let json = json!({"models":[
            {
                "name":"models/gemini-next-flash",
                "baseModelId":"gemini-next",
                "version":"next-001",
                "displayName":"Gemini Next Flash",
                "description":"Fast generation",
                "inputTokenLimit":1_048_576,
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
        ]});
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
    }

    #[test]
    fn ranking_is_semantic_and_lifecycle_aware() {
        let json = json!({"models":[
            {"name":"models/gemini-4.0-pro-preview","modelStage":"PREVIEW","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-3.5-flash","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-3.10-flash","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-3.6-flash","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-flash-latest","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-9.0-legacy","modelStage":"LEGACY","supportedGenerationMethods":["generateContent"]}
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
    fn explicit_deprecated_models_are_excluded_but_unknown_future_models_remain() {
        let json = json!({"models":[
            {"name":"models/gemini-retired-name-but-unflagged","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-future-family","modelStage":"A_FUTURE_STAGE","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-old","modelStatus":{"modelStage":"DEPRECATED"},"supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-gone","modelStage":"RETIRED","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-legacy","modelStage":"LEGACY","supportedGenerationMethods":["generateContent"]}
        ]});
        assert_eq!(
            parse_gemini_list(&json)
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
    fn dated_previews_sort_chronologically() {
        let json = json!({"models":[
            {"name":"models/gemini-4.0-flash-preview-12-2025","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-4.0-flash-preview-09-2026","supportedGenerationMethods":["generateContent"]}
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(ranked[0].id, "gemini-4.0-flash-preview-09-2026");
    }

    #[test]
    fn unknown_future_models_retain_provider_order_when_ranked_equally() {
        let json = json!({"models":[
            {"name":"models/gemini-orion","supportedGenerationMethods":["generateContent"]},
            {"name":"models/future-conversation-family","supportedGenerationMethods":["generateContent"]}
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
    fn specialized_models_never_become_the_runtime_default() {
        let json = json!({"models":[
            {"name":"models/gemini-9.0-live","modelStage":"STABLE","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-8.0-pro-image","modelStage":"STABLE","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-7.0-flash-preview-tts","modelStage":"PREVIEW","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-3.6-flash","modelStage":"STABLE","supportedGenerationMethods":["generateContent"]}
        ]});
        let ranked = rank_gemini_models(parse_gemini_list(&json));
        assert_eq!(ranked[0].id, "gemini-3.6-flash");
        assert_eq!(
            best_gemini_runtime_default(&ranked).as_deref(),
            Some("gemini-3.6-flash")
        );
        let specialized = ranked
            .into_iter()
            .filter(is_gemini_specialized)
            .collect::<Vec<_>>();
        assert_eq!(best_gemini_catalog_default(&specialized), None);
        assert_eq!(best_gemini_runtime_default(&specialized), None);
    }

    #[test]
    fn credential_scoped_cache_entry_expires_at_its_deadline() {
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

    #[tokio::test]
    async fn live_catalog_paginates_and_never_places_the_key_in_the_url() {
        let mock = mock_catalog(vec![
            (
                200,
                json!({
                    "models": [generation_model("gemini-3.6-flash")],
                    "nextPageToken": "page-2"
                }),
            ),
            (
                200,
                json!({"models": [generation_model("gemini-3.10-flash")]}),
            ),
        ]);
        let resolver = GeminiModelResolver::hermetic(mock.api_base.clone());
        let models = resolver.list_models("secret-test-key").await.unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gemini-3.10-flash", "gemini-3.6-flash"]
        );
        mock.worker.join().unwrap();
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("GET /v1beta/models?pageSize=200 HTTP/1.1"));
        assert!(requests[1].contains("pageToken=page-2"));
        for request in requests.iter() {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("x-goog-api-key: secret-test-key")
            );
            let request_target = request.lines().next().unwrap();
            assert!(!request_target.contains("secret-test-key"));
            assert!(!request_target.contains("key="));
        }
    }

    #[tokio::test]
    async fn blank_model_uses_live_default_and_cache_is_key_scoped() {
        let response = json!({"models": [generation_model("gemini-4.2-flash")]});
        let mock = mock_catalog(vec![(200, response.clone()), (200, response)]);
        let resolver = GeminiModelResolver::hermetic(mock.api_base.clone());
        assert_eq!(
            resolver.resolve_default_model("key-a").await,
            "gemini-4.2-flash"
        );
        assert_eq!(
            resolver.resolve_default_model("key-a").await,
            "gemini-4.2-flash"
        );
        assert_eq!(
            resolver.resolve_default_model("key-b").await,
            "gemini-4.2-flash"
        );
        mock.worker.join().unwrap();
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "same-key lookup should use the cache");
        assert!(requests[0].contains("x-goog-api-key: key-a"));
        assert!(requests[1].contains("x-goog-api-key: key-b"));
    }

    #[tokio::test]
    async fn catalog_failure_uses_and_caches_the_last_known_ga_fallback() {
        let mock = mock_catalog(vec![(500, json!({"error": "temporary"}))]);
        let resolver = GeminiModelResolver::hermetic(mock.api_base.clone());
        assert_eq!(
            resolver.resolve_default_model("key-a").await,
            GEMINI_DEFAULT_MODEL
        );
        assert_eq!(
            resolver.resolve_default_model("key-a").await,
            GEMINI_DEFAULT_MODEL
        );
        mock.worker.join().unwrap();
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn explicit_model_bypasses_discovery() {
        assert_eq!(
            resolve_gemini_model(Some("  gemini-user-choice  "), "not-a-real-key").await,
            "gemini-user-choice"
        );
    }
}
