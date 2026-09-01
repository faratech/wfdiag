//! UI-framework-neutral live model discovery.
//!
//! The service accepts unsaved Settings drafts, resolves missing values from
//! the canonical non-secret settings snapshot and credential store, and
//! returns only provider-reported model metadata. Secrets are request inputs
//! only: they are redacted from `Debug`, never included in events/catalogs,
//! and never placed in request URLs.

use crate::{
    AIProvider, BackendFuture, FoundryEndpointSource, ModelCatalogEntry, OllamaSource,
    ProviderKeySource, SubscriptionCli, best_gemini_catalog_default, list_gemini_models,
    normalize_base_url,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use wfdiag_native_settings::{AppSettings, ProviderKeyId};

pub const OPENAI_DEFAULT_MODEL: &str = "gpt-5-nano";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-sonnet-5";
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";
pub const FOUNDRY_DEFAULT_MODEL: &str = "phi-4-mini";

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEEPSEEK_API_BASE: &str = "https://api.deepseek.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(15);
const BRIDGE_CATALOG_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PAGES: usize = 50;
const OPENAI_NON_CHAT_MARKERS: &[&str] = &[
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

/// Structured response consumed by provider setup model pickers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<ModelCatalogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// Unsaved provider setup values for one discovery request.
///
/// The API key deliberately has a redacted `Debug` representation so a
/// failed worker send or diagnostic log cannot expose the draft secret.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ModelCatalogRequest {
    pub provider: AIProvider,
    pub draft_api_key: Option<String>,
    pub draft_endpoint: Option<String>,
    pub draft_cli_path: Option<String>,
}

impl fmt::Debug for ModelCatalogRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelCatalogRequest")
            .field("provider", &self.provider)
            .field(
                "draft_api_key",
                &self.draft_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("draft_endpoint", &self.draft_endpoint)
            .field("draft_cli_path", &self.draft_cli_path)
            .finish()
    }
}

impl ModelCatalogRequest {
    #[must_use]
    pub fn new(provider: AIProvider) -> Self {
        Self {
            provider,
            ..Self::default()
        }
    }

    fn normalized(mut self) -> Self {
        self.draft_api_key = non_empty(self.draft_api_key);
        self.draft_endpoint = non_empty(self.draft_endpoint);
        self.draft_cli_path = non_empty(self.draft_cli_path);
        self
    }
}

/// Subscription CLI model discovery remains owned by the genuine vendor CLI.
/// Implementations must only inspect account/model metadata; discovery must
/// never start a vendor sign-in flow or install a vendor CLI. A shell must
/// treat discovery itself as explicit when a transport (Claude ACP) may need
/// to materialize its pinned adapter package on first use.
pub trait SubscriptionModelCatalogSource: Send + Sync + 'static {
    fn list_models(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> BackendFuture<'_, Result<ModelCatalog, String>>;
}

trait ModelCatalogHttpSource: Send + Sync + 'static {
    fn get_json(
        &self,
        url: String,
        headers: Vec<(&'static str, String)>,
        query: Vec<(&'static str, String)>,
    ) -> BackendFuture<'_, Result<Value, String>>;
}

#[derive(Debug, Default)]
struct ReqwestModelCatalogHttpSource;

impl ModelCatalogHttpSource for ReqwestModelCatalogHttpSource {
    fn get_json(
        &self,
        url: String,
        headers: Vec<(&'static str, String)>,
        query: Vec<(&'static str, String)>,
    ) -> BackendFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let mut request = reqwest::Client::new()
                .get(&url)
                .query(&query)
                .timeout(REQUEST_TIMEOUT);
            for (name, value) in headers {
                request = request.header(name, value);
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
        })
    }
}

/// Immutable ports for one provider-setup discovery call.
///
/// Shells rebuild this service from the latest settings before each request,
/// so a just-saved endpoint/path/key is immediately visible.
#[derive(Clone)]
pub struct ModelCatalogService {
    settings: AppSettings,
    keys: Arc<dyn ProviderKeySource>,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    subscriptions: Arc<dyn SubscriptionModelCatalogSource>,
    http: Arc<dyn ModelCatalogHttpSource>,
}

impl ModelCatalogService {
    #[must_use]
    pub fn new(
        settings: AppSettings,
        keys: Arc<dyn ProviderKeySource>,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        subscriptions: Arc<dyn SubscriptionModelCatalogSource>,
    ) -> Self {
        Self {
            settings,
            keys,
            foundry,
            ollama,
            subscriptions,
            http: Arc::new(ReqwestModelCatalogHttpSource),
        }
    }

    #[cfg(test)]
    fn with_http(mut self, http: Arc<dyn ModelCatalogHttpSource>) -> Self {
        self.http = http;
        self
    }

    /// Discover one provider's live model catalog within the shipping budget.
    pub async fn list(&self, request: ModelCatalogRequest) -> Result<ModelCatalog, String> {
        let request = request.normalized();
        let budget = catalog_timeout(request.provider);
        tokio::time::timeout(budget, self.list_inner(request))
            .await
            .map_err(|_| {
                format!(
                    "Model discovery did not finish within {} seconds.",
                    budget.as_secs()
                )
            })?
    }

    #[allow(clippy::too_many_lines)]
    async fn list_inner(&self, request: ModelCatalogRequest) -> Result<ModelCatalog, String> {
        match request.provider {
            AIProvider::OpenAI => {
                let key =
                    self.require_key(request.draft_api_key, ProviderKeyId::OpenAI, "OpenAI")?;
                let json = self
                    .http
                    .get_json(
                        OPENAI_MODELS_URL.to_string(),
                        vec![("authorization", format!("Bearer {key}"))],
                        Vec::new(),
                    )
                    .await?;
                Ok(ModelCatalog {
                    models: entries_from_ids(sorted_desc(filter_openai_chat_models(
                        parse_id_list(&json),
                    ))),
                    default_model: Some(OPENAI_DEFAULT_MODEL.to_string()),
                })
            }
            AIProvider::Anthropic => {
                let key =
                    self.require_key(request.draft_api_key, ProviderKeyId::Anthropic, "Anthropic")?;
                Ok(ModelCatalog {
                    models: self.list_anthropic_models(&key).await?,
                    default_model: Some(ANTHROPIC_DEFAULT_MODEL.to_string()),
                })
            }
            AIProvider::Gemini => {
                let key =
                    self.require_key(request.draft_api_key, ProviderKeyId::Gemini, "Gemini")?;
                let models = list_gemini_models(&key).await?;
                Ok(ModelCatalog {
                    default_model: best_gemini_catalog_default(&models),
                    models,
                })
            }
            AIProvider::DeepSeek => {
                let key =
                    self.require_key(request.draft_api_key, ProviderKeyId::DeepSeek, "DeepSeek")?;
                let json = self
                    .http
                    .get_json(
                        format!("{DEEPSEEK_API_BASE}/v1/models"),
                        vec![("authorization", format!("Bearer {key}"))],
                        Vec::new(),
                    )
                    .await?;
                Ok(ModelCatalog {
                    models: entries_from_ids(sorted_desc(parse_id_list(&json))),
                    default_model: Some(DEEPSEEK_DEFAULT_MODEL.to_string()),
                })
            }
            AIProvider::CustomOpenAI => {
                let base = request
                    .draft_endpoint
                    .as_deref()
                    .or(self.settings.custom_endpoint.as_deref())
                    .and_then(normalize_base_url)
                    .ok_or_else(|| "Set the endpoint URL first to list models.".to_string())?;
                let key = request
                    .draft_api_key
                    .or_else(|| self.keys.load(ProviderKeyId::Custom));
                let headers = key
                    .map(|key| vec![("authorization", format!("Bearer {key}"))])
                    .unwrap_or_default();
                let json = self
                    .http
                    .get_json(format!("{base}/v1/models"), headers, Vec::new())
                    .await?;
                Ok(ModelCatalog {
                    models: entries_from_ids(sorted_desc(parse_id_list(&json))),
                    default_model: None,
                })
            }
            AIProvider::FoundryLocal => {
                let base = match request
                    .draft_endpoint
                    .as_deref()
                    .and_then(normalize_base_url)
                {
                    Some(base) => base,
                    None => self
                        .foundry
                        .probe(self.settings.local_ai_endpoint.clone())
                        .await
                        .ok_or_else(|| {
                            "Foundry Local is not running (try 'foundry server start').".to_string()
                        })?,
                };
                Ok(ModelCatalog {
                    models: entries_from_ids(sorted_desc(self.list_foundry_models(&base).await?)),
                    default_model: Some(FOUNDRY_DEFAULT_MODEL.to_string()),
                })
            }
            AIProvider::Ollama => {
                let configured = request
                    .draft_endpoint
                    .or_else(|| self.settings.ollama_endpoint.clone());
                let base = self
                    .ollama
                    .discover(configured)
                    .await
                    .ok_or_else(|| "No Ollama server reachable.".to_string())?;
                let ids = stable_dedupe(self.ollama.list_models(base).await?);
                Ok(ModelCatalog {
                    default_model: ids.first().cloned(),
                    models: entries_from_ids(ids),
                })
            }
            AIProvider::CodexCli => {
                let configured = request
                    .draft_cli_path
                    .or_else(|| self.settings.codex_cli_path.clone());
                self.subscriptions
                    .list_models(SubscriptionCli::Codex, configured)
                    .await
            }
            AIProvider::ClaudeCode => {
                let configured = request
                    .draft_cli_path
                    .or_else(|| self.settings.claude_cli_path.clone());
                self.subscriptions
                    .list_models(SubscriptionCli::ClaudeCode, configured)
                    .await
            }
            AIProvider::PhiSilica => Ok(ModelCatalog::default()),
            AIProvider::None => Err("Unknown provider: none".to_string()),
        }
    }

    fn require_key(
        &self,
        draft: Option<String>,
        id: ProviderKeyId,
        label: &str,
    ) -> Result<String, String> {
        draft
            .or_else(|| self.keys.load(id))
            .ok_or_else(|| format!("Add your {label} API key first to list models."))
    }

    async fn list_foundry_models(&self, base: &str) -> Result<Vec<String>, String> {
        let current = self
            .http
            .get_json(format!("{base}/v1/models"), Vec::new(), Vec::new())
            .await;
        let current_error = match current {
            Ok(json) => match parse_current_foundry_models(&json) {
                Ok(models) => return Ok(models),
                Err(error) => error,
            },
            Err(error) => error,
        };

        let legacy = self
            .http
            .get_json(format!("{base}/openai/models"), Vec::new(), Vec::new())
            .await
            .map_err(|legacy_error| {
                format!(
                    "Foundry Local model discovery failed on /v1/models ({current_error}) and /openai/models ({legacy_error})"
                )
            })?;
        parse_legacy_foundry_models(&legacy).map_err(|legacy_error| {
            format!(
                "Foundry Local model discovery failed on /v1/models ({current_error}) and /openai/models ({legacy_error})"
            )
        })
    }

    async fn list_anthropic_models(&self, api_key: &str) -> Result<Vec<ModelCatalogEntry>, String> {
        let headers = vec![
            ("x-api-key", api_key.to_string()),
            ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ];
        let mut models = Vec::new();
        let mut after_id: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for _ in 0..MAX_PAGES {
            let mut query = vec![("limit", "100".to_string())];
            if let Some(cursor) = after_id.as_ref() {
                query.push(("after_id", cursor.clone()));
            }
            let json = self
                .http
                .get_json(ANTHROPIC_MODELS_URL.to_string(), headers.clone(), query)
                .await?;
            models.extend(parse_anthropic_list(&json));

            if !json
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(stable_dedupe_entries(models));
            }
            let cursor = json
                .get("last_id")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .ok_or_else(|| {
                    "Anthropic model list said another page exists but returned no last_id."
                        .to_string()
                })?
                .to_string();
            if !seen_cursors.insert(cursor.clone()) {
                return Err("Anthropic model list repeated a pagination cursor.".to_string());
            }
            after_id = Some(cursor);
        }

        Err("Anthropic model list exceeded the pagination limit.".to_string())
    }
}

/// Parse the historical provider spellings accepted by Tauri Settings while
/// keeping the service itself strongly typed.
pub fn parse_model_catalog_provider(provider: &str) -> Result<AIProvider, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" => Ok(AIProvider::OpenAI),
        "anthropic" => Ok(AIProvider::Anthropic),
        "gemini" => Ok(AIProvider::Gemini),
        "deepseek" => Ok(AIProvider::DeepSeek),
        "custom_openai" | "custom" => Ok(AIProvider::CustomOpenAI),
        "foundry_local" | "foundrylocal" => Ok(AIProvider::FoundryLocal),
        "ollama" => Ok(AIProvider::Ollama),
        "codex_cli" | "codexcli" | "codex" => Ok(AIProvider::CodexCli),
        "claude_code" | "claudecode" | "claude" => Ok(AIProvider::ClaudeCode),
        "phi_silica" | "phisilica" => Ok(AIProvider::PhiSilica),
        other => Err(format!("Unknown provider: {other}")),
    }
}

const fn catalog_timeout(provider: AIProvider) -> Duration {
    match provider {
        AIProvider::CodexCli | AIProvider::ClaudeCode => BRIDGE_CATALOG_TIMEOUT,
        _ => CATALOG_TIMEOUT,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_id_list(json: &Value) -> Vec<String> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_anthropic_list(json: &Value) -> Vec<ModelCatalogEntry> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    let id = model.get("id")?.as_str()?.to_string();
                    Some(ModelCatalogEntry {
                        label: optional_distinct_string(
                            model.get("display_name").and_then(Value::as_str),
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

fn parse_current_foundry_models(json: &Value) -> Result<Vec<String>, String> {
    json.get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Foundry Local /v1/models response omitted the data array".to_string())?;
    Ok(parse_id_list(json))
}

fn parse_legacy_foundry_models(json: &Value) -> Result<Vec<String>, String> {
    json.as_array()
        .ok_or_else(|| "Foundry Local /openai/models response was not an array".to_string())?
        .iter()
        .map(|model| {
            model.as_str().map(str::to_string).ok_or_else(|| {
                "Foundry Local /openai/models returned a non-string model name".to_string()
            })
        })
        .collect()
}

fn filter_openai_chat_models(ids: Vec<String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| {
            let id = id.to_ascii_lowercase();
            !OPENAI_NON_CHAT_MARKERS
                .iter()
                .any(|marker| id.contains(marker))
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
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != id)
        .map(str::to_string)
}

fn entries_from_ids(ids: Vec<String>) -> Vec<ModelCatalogEntry> {
    ids.into_iter().map(ModelCatalogEntry::from_id).collect()
}

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
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeKeys(Mutex<HashMap<ProviderKeyId, String>>);

    impl ProviderKeySource for FakeKeys {
        fn load(&self, key: ProviderKeyId) -> Option<String> {
            self.0.lock().unwrap().get(&key).cloned()
        }
    }

    #[derive(Default)]
    struct FakeFoundry(Option<String>);

    impl FoundryEndpointSource for FakeFoundry {
        fn probe(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { self.0.clone() })
        }
    }

    #[derive(Default)]
    struct FakeOllama;

    impl OllamaSource for FakeOllama {
        fn discover(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async move { configured.or_else(|| Some("http://ollama.test".to_string())) })
        }

        fn list_models(&self, _endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async {
                Ok(vec![
                    "llama3.2:latest".to_string(),
                    "llama3.2:latest".to_string(),
                    "phi4:latest".to_string(),
                ])
            })
        }
    }

    #[derive(Default)]
    struct FakeSubscriptions;

    impl SubscriptionModelCatalogSource for FakeSubscriptions {
        fn list_models(
            &self,
            provider: SubscriptionCli,
            configured_path: Option<String>,
        ) -> BackendFuture<'_, Result<ModelCatalog, String>> {
            Box::pin(async move {
                Ok(ModelCatalog {
                    models: vec![ModelCatalogEntry::from_id(format!(
                        "{provider:?}:{}",
                        configured_path.unwrap_or_default()
                    ))],
                    default_model: None,
                })
            })
        }
    }

    #[derive(Default)]
    struct FakeHttp {
        responses: Mutex<VecDeque<Result<Value, String>>>,
        requests: Mutex<Vec<CapturedHttpRequest>>,
    }

    type CapturedHttpRequest = (String, Vec<(&'static str, String)>);

    impl ModelCatalogHttpSource for FakeHttp {
        fn get_json(
            &self,
            url: String,
            headers: Vec<(&'static str, String)>,
            _query: Vec<(&'static str, String)>,
        ) -> BackendFuture<'_, Result<Value, String>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push((url, headers));
                self.responses.lock().unwrap().pop_front().unwrap()
            })
        }
    }

    fn service(keys: Arc<dyn ProviderKeySource>) -> ModelCatalogService {
        ModelCatalogService::new(
            AppSettings::default(),
            keys,
            Arc::new(FakeFoundry::default()),
            Arc::new(FakeOllama),
            Arc::new(FakeSubscriptions),
        )
    }

    #[test]
    fn request_debug_redacts_the_only_secret_field() {
        let request = ModelCatalogRequest {
            provider: AIProvider::Anthropic,
            draft_api_key: Some("sk-ant-secret-value".to_string()),
            draft_endpoint: Some("https://example.test".to_string()),
            draft_cli_path: None,
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-ant-secret-value"));
    }

    #[tokio::test]
    async fn draft_key_wins_and_catalog_never_returns_it() {
        let keys = Arc::new(FakeKeys::default());
        keys.0
            .lock()
            .unwrap()
            .insert(ProviderKeyId::OpenAI, "stored-secret".to_string());
        let http = Arc::new(FakeHttp::default());
        http.responses
            .lock()
            .unwrap()
            .push_back(Ok(serde_json::json!({
                "data": [
                    {"id":"text-embedding-3-small"},
                    {"id":"future-conversation-1"},
                    {"id":"gpt-5.6-sol"}
                ]
            })));
        let catalog = service(keys)
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::OpenAI,
                draft_api_key: Some("draft-secret".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.6-sol", "future-conversation-1"]
        );
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests[0].1[0].1, "Bearer draft-secret");
        let public = serde_json::to_string(&catalog).unwrap();
        assert!(!public.contains("draft-secret"));
        assert!(!public.contains("stored-secret"));
    }

    #[tokio::test]
    async fn blank_draft_falls_back_to_the_stored_provider_key() {
        let keys = Arc::new(FakeKeys::default());
        keys.0.lock().unwrap().insert(
            ProviderKeyId::DeepSeek,
            "stored-deepseek-secret".to_string(),
        );
        let http = Arc::new(FakeHttp::default());
        http.responses
            .lock()
            .unwrap()
            .push_back(Ok(serde_json::json!({"data":[{"id":"deepseek-v4-flash"}]})));

        let catalog = service(keys)
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::DeepSeek,
                draft_api_key: Some("  ".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(catalog.default_model.as_deref(), Some("deepseek-v4-flash"));
        let requests = http.requests.lock().unwrap();
        assert_eq!(
            requests[0].1,
            [("authorization", "Bearer stored-deepseek-secret".to_string())]
        );
        assert!(
            !serde_json::to_string(&catalog)
                .unwrap()
                .contains("stored-deepseek-secret")
        );
    }

    #[tokio::test]
    async fn ollama_and_subscription_drafts_are_routed_without_ui_state() {
        let ollama = service(Arc::new(FakeKeys::default()))
            .list(ModelCatalogRequest {
                provider: AIProvider::Ollama,
                draft_endpoint: Some(" http://draft-ollama.test/v1 ".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(ollama.default_model.as_deref(), Some("llama3.2:latest"));
        assert_eq!(ollama.models.len(), 2);

        let codex = service(Arc::new(FakeKeys::default()))
            .list(ModelCatalogRequest {
                provider: AIProvider::CodexCli,
                draft_cli_path: Some(" C:\\Tools\\codex.exe ".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(codex.models[0].id, "Codex:C:\\Tools\\codex.exe");
    }

    #[tokio::test]
    async fn foundry_uses_current_v1_catalog_and_normalizes_the_configured_base() {
        let http = Arc::new(FakeHttp::default());
        http.responses
            .lock()
            .unwrap()
            .push_back(Ok(serde_json::json!({
                "object": "list",
                "data": [
                    {"id":"phi-4-mini"},
                    {"id":"zeta-local"},
                    {"id":"phi-4-mini"},
                    {"missing":"ignored"}
                ]
            })));

        let catalog = service(Arc::new(FakeKeys::default()))
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::FoundryLocal,
                draft_endpoint: Some(" http://foundry.test/v1/ ".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["zeta-local", "phi-4-mini"]
        );
        assert_eq!(catalog.default_model.as_deref(), Some("phi-4-mini"));
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "http://foundry.test/v1/models");
        assert!(requests[0].1.is_empty());
    }

    #[tokio::test]
    async fn empty_current_foundry_catalog_is_valid_and_does_not_try_legacy() {
        let http = Arc::new(FakeHttp::default());
        http.responses
            .lock()
            .unwrap()
            .push_back(Ok(serde_json::json!({"object":"list","data":[]})));

        let catalog = service(Arc::new(FakeKeys::default()))
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::FoundryLocal,
                draft_endpoint: Some("http://foundry.test".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert!(catalog.models.is_empty());
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "http://foundry.test/v1/models");
    }

    #[tokio::test]
    async fn foundry_falls_back_to_legacy_catalog_after_current_http_failure() {
        let http = Arc::new(FakeHttp::default());
        http.responses.lock().unwrap().extend([
            Err("Model list request failed: HTTP 404".to_string()),
            Ok(serde_json::json!(["phi-4-mini", "legacy-local"])),
        ]);

        let catalog = service(Arc::new(FakeKeys::default()))
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::FoundryLocal,
                draft_endpoint: Some("http://foundry.test".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["phi-4-mini", "legacy-local"]
        );
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "http://foundry.test/v1/models");
        assert_eq!(requests[1].0, "http://foundry.test/openai/models");
    }

    #[tokio::test]
    async fn foundry_falls_back_to_legacy_catalog_after_current_schema_failure() {
        let http = Arc::new(FakeHttp::default());
        http.responses.lock().unwrap().extend([
            Ok(serde_json::json!({"models":[]})),
            Ok(serde_json::json!(["legacy-only"])),
        ]);

        let catalog = service(Arc::new(FakeKeys::default()))
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::FoundryLocal,
                draft_endpoint: Some("http://foundry.test".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(catalog.models[0].id, "legacy-only");
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "http://foundry.test/v1/models");
        assert_eq!(requests[1].0, "http://foundry.test/openai/models");
    }

    #[tokio::test]
    async fn foundry_reports_both_current_and_legacy_catalog_failures() {
        let http = Arc::new(FakeHttp::default());
        http.responses
            .lock()
            .unwrap()
            .extend([Err("HTTP 404".to_string()), Err("HTTP 410".to_string())]);

        let error = service(Arc::new(FakeKeys::default()))
            .with_http(http.clone())
            .list(ModelCatalogRequest {
                provider: AIProvider::FoundryLocal,
                draft_endpoint: Some("http://foundry.test".to_string()),
                ..ModelCatalogRequest::default()
            })
            .await
            .unwrap_err();

        assert!(error.contains("/v1/models (HTTP 404)"));
        assert!(error.contains("/openai/models (HTTP 410)"));
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "http://foundry.test/v1/models");
        assert_eq!(requests[1].0, "http://foundry.test/openai/models");
    }

    #[test]
    fn aliases_are_accepted_but_unknown_providers_fail_closed() {
        assert_eq!(
            parse_model_catalog_provider("claudecode"),
            Ok(AIProvider::ClaudeCode)
        );
        assert_eq!(
            parse_model_catalog_provider("custom"),
            Ok(AIProvider::CustomOpenAI)
        );
        assert!(parse_model_catalog_provider("future-provider").is_err());
    }

    #[test]
    fn sonnet_five_remains_the_anthropic_default() {
        assert_eq!(ANTHROPIC_DEFAULT_MODEL, "claude-sonnet-5");
    }

    #[test]
    fn foundry_parsers_accept_current_openai_and_legacy_string_schemas() {
        assert_eq!(
            parse_current_foundry_models(&serde_json::json!({
                "data":[{"id":"phi-4-mini"},{"missing":"ignored"},{"id":"model-2"}]
            }))
            .unwrap(),
            vec!["phi-4-mini", "model-2"]
        );
        assert_eq!(
            parse_current_foundry_models(&serde_json::json!({"data":[]})).unwrap(),
            Vec::<String>::new()
        );
        assert!(parse_current_foundry_models(&serde_json::json!([])).is_err());

        assert_eq!(
            parse_legacy_foundry_models(&serde_json::json!(["phi-4-mini", "model-2"])).unwrap(),
            vec!["phi-4-mini", "model-2"]
        );
        assert!(parse_legacy_foundry_models(&serde_json::json!({"data":[]})).is_err());
        assert!(parse_legacy_foundry_models(&serde_json::json!(["valid", 42])).is_err());
    }
}
