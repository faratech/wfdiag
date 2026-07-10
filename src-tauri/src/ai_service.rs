//! Unified AI Service Layer
//!
//! This module provides a unified interface for AI analysis that abstracts
//! OpenAI and Phi Silica providers. It handles provider detection, routing,
//! caching, and rate limiting.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ai_cache::AICache;
use crate::ai_prompts;

/// AI Provider enumeration.
///
/// Wire strings are pinned EXPLICITLY per variant (not `rename_all`): serde's
/// snake_case rule turns `OpenAI` into `"open_a_i"`, which the frontend union
/// type never matched — a latent bug fixed in 2.5.0. The alias tolerates any
/// legacy string still in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AIProvider {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "openai", alias = "open_a_i")]
    OpenAI,
    /// On-device Phi Silica via the Windows AI APIs (needs package identity)
    #[serde(rename = "phi_silica")]
    PhiSilica,
    /// Local OpenAI-compatible server (Foundry Local), no identity required
    #[serde(rename = "foundry_local")]
    FoundryLocal,
    /// Local Ollama server (OpenAI-compatible API at /v1)
    #[serde(rename = "ollama")]
    Ollama,
    /// User-configured OpenAI-compatible endpoint (OpenRouter, Groq, …)
    #[serde(rename = "custom_openai")]
    CustomOpenAI,
    /// ChatGPT subscription via the user's installed Codex CLI (the CLI owns
    /// sign-in; no API key — see `ai_providers/cli_bridge.rs`)
    #[serde(rename = "codex_cli")]
    CodexCli,
    /// Claude subscription via the user's installed Claude Code CLI (same
    /// bridge trust model as Codex — the CLI owns sign-in)
    #[serde(rename = "claude_code")]
    ClaudeCode,
    /// Anthropic Claude via the native Messages API
    #[serde(rename = "anthropic")]
    Anthropic,
    /// Google Gemini via the native generateContent API
    #[serde(rename = "gemini")]
    Gemini,
    /// DeepSeek cloud (OpenAI-compatible chat completions)
    #[serde(rename = "deepseek")]
    DeepSeek,
}

impl std::fmt::Display for AIProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIProvider::None => write!(f, "none"),
            AIProvider::OpenAI => write!(f, "openai"),
            AIProvider::PhiSilica => write!(f, "phi_silica"),
            AIProvider::FoundryLocal => write!(f, "foundry_local"),
            AIProvider::Ollama => write!(f, "ollama"),
            AIProvider::CustomOpenAI => write!(f, "custom_openai"),
            AIProvider::CodexCli => write!(f, "codex_cli"),
            AIProvider::ClaudeCode => write!(f, "claude_code"),
            AIProvider::Anthropic => write!(f, "anthropic"),
            AIProvider::Gemini => write!(f, "gemini"),
            AIProvider::DeepSeek => write!(f, "deepseek"),
        }
    }
}

/// User preference for AI provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AIProviderPreference {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "openai", alias = "open_a_i")]
    OpenAI,
    #[serde(rename = "phi_silica")]
    PhiSilica,
    #[serde(rename = "foundry_local")]
    FoundryLocal,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "custom_openai")]
    CustomOpenAI,
    #[serde(rename = "codex_cli")]
    CodexCli,
    #[serde(rename = "claude_code")]
    ClaudeCode,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "deepseek")]
    DeepSeek,
}

/// Context type for different AI analysis scenarios
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextType {
    /// Single diagnostic card interpretation
    DiagnosticInterpretation,
    /// Section summary (Hardware, System, Storage, Network)
    SectionSummary,
    /// Health score explanation
    HealthScoreExplanation,
    /// Issue prioritization and analysis
    IssuePrioritization,
    /// General chat/analysis
    GeneralAnalysis,
}

/// AI analysis request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIRequest {
    pub context_type: ContextType,
    pub context_id: String,
    pub data: String,
    pub task_name: Option<String>,
    pub section_name: Option<String>,
}

/// AI analysis response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIResponse {
    pub interpretation: String,
    pub provider_used: AIProvider,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding: Option<crate::ai_grounding::GroundingTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Per-provider status row (settings badges, provider pickers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: AIProvider,
    /// Usable right now (key stored / endpoint reachable)
    pub available: bool,
    /// User has done the setup (no network validation)
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

/// Full status of AI providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIProviderStatus {
    pub preferred_provider: AIProvider,
    pub openai_available: bool,
    pub openai_api_key_set: bool,
    pub phi_silica_available: bool,
    pub phi_silica_ready: bool,
    pub phi_silica_message: Option<String>,
    #[serde(default)]
    pub foundry_local_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foundry_local_endpoint: Option<String>,
    pub active_provider: AIProvider,
    /// One row per real provider, in Auto routing order
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
}

/// Global AI service state
/// Note: These are initialized lazily via OnceLock, so explicit initialization is optional
#[allow(dead_code)] // Used for tracking - cache/preference are initialized lazily
static AI_SERVICE_INITIALIZED: AtomicBool = AtomicBool::new(false);
static AI_CACHE: OnceLock<std::sync::Mutex<AICache>> = OnceLock::new();
static USER_PREFERENCE: OnceLock<std::sync::Mutex<AIProviderPreference>> = OnceLock::new();
const ANALYSIS_CACHE_VERSION: &str = "rag-v2";

/// Initialize the AI service (call once at startup)
/// Note: This is optional since OnceLock initializes lazily on first access
#[allow(dead_code)] // Optional - caches initialize lazily
pub fn init_ai_service() {
    if AI_SERVICE_INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }

    // Initialize cache
    AI_CACHE.get_or_init(|| std::sync::Mutex::new(AICache::new(100)));

    // Initialize user preference
    USER_PREFERENCE.get_or_init(|| std::sync::Mutex::new(AIProviderPreference::Auto));
}

/// Get the AI cache
fn get_cache() -> &'static std::sync::Mutex<AICache> {
    AI_CACHE.get_or_init(|| std::sync::Mutex::new(AICache::new(100)))
}

/// Get current user preference
pub fn get_user_preference() -> AIProviderPreference {
    USER_PREFERENCE
        .get_or_init(|| std::sync::Mutex::new(AIProviderPreference::Auto))
        .lock()
        .map(|p| *p)
        .unwrap_or(AIProviderPreference::Auto)
}

/// Set user preference
pub fn set_user_preference(pref: AIProviderPreference) {
    // Use get_or_init to ensure the mutex is initialized before setting
    let mutex = USER_PREFERENCE.get_or_init(|| std::sync::Mutex::new(AIProviderPreference::Auto));
    if let Ok(mut p) = mutex.lock() {
        *p = pref;
    }
}

/// Check if OpenAI is available (API key is set)
pub async fn check_openai_available() -> bool {
    crate::load_api_key_internal().await.is_some()
}

/// Check if Phi Silica is available
pub fn check_phi_silica_available() -> (bool, bool, Option<String>) {
    let status = crate::phi_silica::is_phi_silica_available();
    (
        status.available,
        status.ready_state.as_deref() == Some("Ready"),
        Some(status.message),
    )
}

/// Check if a local OpenAI-compatible endpoint (Foundry Local) is reachable.
/// Returns the base URL when available.
pub async fn check_foundry_local_available() -> Option<String> {
    crate::ai_providers::foundry::local_ai_endpoint().await
}

/// Check if an Ollama server is reachable. Returns the base URL when available.
pub async fn check_ollama_available() -> Option<String> {
    let configured =
        crate::commands::settings::read_settings_from_disk().and_then(|s| s.ollama_endpoint);
    crate::ai_providers::ollama::discover_endpoint(configured.as_deref()).await
}

/// Check if the custom OpenAI-compatible endpoint is usable: endpoint and
/// model configured, and the endpoint reachable. The API key is optional —
/// local proxies often don't need one.
pub async fn check_custom_available() -> Option<String> {
    let settings = crate::commands::settings::read_settings_from_disk()?;
    settings
        .custom_model
        .as_deref()
        .filter(|m| !m.trim().is_empty())?;
    let endpoint = settings
        .custom_endpoint
        .as_deref()
        .and_then(crate::ai_providers::discovery::normalize_base_url)?;
    if crate::ai_providers::discovery::probe_endpoint_async(&endpoint).await {
        Some(endpoint)
    } else {
        None
    }
}

/// Check if the Codex CLI bridge is usable: installed AND signed in with a
/// ChatGPT account (probe result is cached briefly in `cli_bridge`).
pub async fn check_codex_available() -> bool {
    crate::ai_providers::cli_bridge::probe(AIProvider::CodexCli)
        .await
        .usable()
}

/// Check if the Claude Code CLI bridge is usable: installed AND signed in
/// with a Claude account (probe result is cached briefly in `cli_bridge`).
pub async fn check_claude_code_available() -> bool {
    crate::ai_providers::cli_bridge::probe(AIProvider::ClaudeCode)
        .await
        .usable()
}

/// Check if Anthropic is available (API key stored)
pub async fn check_anthropic_available() -> bool {
    crate::commands::settings::load_provider_key_internal(crate::dpapi::ProviderKeyId::Anthropic)
        .await
        .is_some()
}

/// Check if Gemini is available (API key stored)
pub async fn check_gemini_available() -> bool {
    crate::commands::settings::load_provider_key_internal(crate::dpapi::ProviderKeyId::Gemini)
        .await
        .is_some()
}

/// Check if DeepSeek is available (API key stored)
pub async fn check_deepseek_available() -> bool {
    crate::commands::settings::load_provider_key_internal(crate::dpapi::ProviderKeyId::DeepSeek)
        .await
        .is_some()
}

/// Snapshot of which providers could serve a request right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderAvailability {
    pub phi: bool,
    pub foundry: bool,
    pub ollama: bool,
    /// Custom endpoint configured (endpoint + model) AND reachable
    pub custom: bool,
    /// Codex CLI installed AND signed in (ChatGPT subscription bridge)
    pub codex: bool,
    /// Claude Code CLI installed AND signed in (Claude subscription bridge)
    pub claude: bool,
    pub openai: bool,
    pub anthropic: bool,
    pub gemini: bool,
    pub deepseek: bool,
}

/// Pure routing decision: which provider serves a request given the user's
/// preference and what's available.
///
/// Auto is local-first: on-device Phi Silica (NPU, no server), then Foundry
/// Local (Microsoft's local server — before Ollama so pre-2.5 setups see no
/// behavior change), then Ollama, then a configured custom endpoint
/// (configuring one is a deliberate act), then a signed-in Codex CLI (a
/// ChatGPT subscription has no marginal cost, so it beats metered API keys),
/// then cloud keys in the order OpenAI → Anthropic → Gemini (OpenAI first
/// strictly for backward compatibility; the cloud tiebreak is pinned by test
/// and moot in practice — multiple-key users set an explicit preference).
///
/// An explicit preference never falls back to a different provider.
pub fn route_provider(pref: AIProviderPreference, avail: ProviderAvailability) -> AIProvider {
    match pref {
        AIProviderPreference::Auto => {
            if avail.phi {
                AIProvider::PhiSilica
            } else if avail.foundry {
                AIProvider::FoundryLocal
            } else if avail.ollama {
                AIProvider::Ollama
            } else if avail.custom {
                AIProvider::CustomOpenAI
            } else if avail.codex {
                AIProvider::CodexCli
            } else if avail.claude {
                AIProvider::ClaudeCode
            } else if avail.openai {
                AIProvider::OpenAI
            } else if avail.anthropic {
                AIProvider::Anthropic
            } else if avail.gemini {
                AIProvider::Gemini
            } else if avail.deepseek {
                AIProvider::DeepSeek
            } else {
                AIProvider::None
            }
        }
        AIProviderPreference::OpenAI if avail.openai => AIProvider::OpenAI,
        AIProviderPreference::PhiSilica if avail.phi => AIProvider::PhiSilica,
        AIProviderPreference::FoundryLocal if avail.foundry => AIProvider::FoundryLocal,
        AIProviderPreference::Ollama if avail.ollama => AIProvider::Ollama,
        AIProviderPreference::CustomOpenAI if avail.custom => AIProvider::CustomOpenAI,
        AIProviderPreference::CodexCli if avail.codex => AIProvider::CodexCli,
        AIProviderPreference::ClaudeCode if avail.claude => AIProvider::ClaudeCode,
        AIProviderPreference::Anthropic if avail.anthropic => AIProvider::Anthropic,
        AIProviderPreference::Gemini if avail.gemini => AIProvider::Gemini,
        AIProviderPreference::DeepSeek if avail.deepseek => AIProvider::DeepSeek,
        _ => AIProvider::None,
    }
}

/// Determine the active provider based on preference and availability.
/// Probes lazily in priority order — the Foundry probe shells out and the
/// Ollama/custom probes open sockets, so none of them run once an earlier
/// provider has already won. `frontend_openai_key` marks an OpenAI key passed
/// per-call from the frontend (counts as OpenAI availability, nothing more).
pub async fn determine_active_provider_with_key(
    pref: AIProviderPreference,
    frontend_openai_key: bool,
) -> AIProvider {
    let mut avail = ProviderAvailability::default();
    match pref {
        AIProviderPreference::OpenAI => {
            avail.openai = frontend_openai_key || check_openai_available().await;
        }
        AIProviderPreference::PhiSilica => avail.phi = check_phi_silica_available().1,
        AIProviderPreference::FoundryLocal => {
            avail.foundry = check_foundry_local_available().await.is_some();
        }
        AIProviderPreference::Ollama => avail.ollama = check_ollama_available().await.is_some(),
        AIProviderPreference::CustomOpenAI => {
            avail.custom = check_custom_available().await.is_some();
        }
        AIProviderPreference::CodexCli => avail.codex = check_codex_available().await,
        AIProviderPreference::ClaudeCode => {
            avail.claude = check_claude_code_available().await;
        }
        AIProviderPreference::Anthropic => avail.anthropic = check_anthropic_available().await,
        AIProviderPreference::Gemini => avail.gemini = check_gemini_available().await,
        AIProviderPreference::DeepSeek => avail.deepseek = check_deepseek_available().await,
        AIProviderPreference::Auto => {
            avail.phi = check_phi_silica_available().1;
            avail.foundry = !avail.phi && check_foundry_local_available().await.is_some();
            let local_won = avail.phi || avail.foundry;
            avail.ollama = !local_won && check_ollama_available().await.is_some();
            let won = local_won || avail.ollama;
            avail.custom = !won && check_custom_available().await.is_some();
            let won = won || avail.custom;
            avail.codex = !won && check_codex_available().await;
            let won = won || avail.codex;
            avail.claude = !won && check_claude_code_available().await;
            let won = won || avail.claude;
            avail.openai = !won && (frontend_openai_key || check_openai_available().await);
            let won = won || avail.openai;
            avail.anthropic = !won && check_anthropic_available().await;
            let won = won || avail.anthropic;
            avail.gemini = !won && check_gemini_available().await;
            let won = won || avail.gemini;
            avail.deepseek = !won && check_deepseek_available().await;
        }
    }
    route_provider(pref, avail)
}

/// The next Auto-order provider not already in `tried`, probed lazily and
/// short-circuited on the first available one. Returns `None` for explicit
/// preferences (they never fall back — the documented rule) and when no
/// untried provider is available. Used to recover in chat when the chosen
/// provider fails before producing any output, so a flaky subscription
/// bridge can't dead-end a request another provider could serve.
pub async fn next_auto_provider(
    pref: AIProviderPreference,
    tried: &[AIProvider],
    frontend_openai_key: bool,
) -> Option<AIProvider> {
    if pref != AIProviderPreference::Auto {
        return None;
    }
    let untried = |p: AIProvider| !tried.contains(&p);
    if untried(AIProvider::PhiSilica) && check_phi_silica_available().1 {
        return Some(AIProvider::PhiSilica);
    }
    if untried(AIProvider::FoundryLocal) && check_foundry_local_available().await.is_some() {
        return Some(AIProvider::FoundryLocal);
    }
    if untried(AIProvider::Ollama) && check_ollama_available().await.is_some() {
        return Some(AIProvider::Ollama);
    }
    if untried(AIProvider::CustomOpenAI) && check_custom_available().await.is_some() {
        return Some(AIProvider::CustomOpenAI);
    }
    if untried(AIProvider::CodexCli) && check_codex_available().await {
        return Some(AIProvider::CodexCli);
    }
    if untried(AIProvider::ClaudeCode) && check_claude_code_available().await {
        return Some(AIProvider::ClaudeCode);
    }
    if untried(AIProvider::OpenAI) && (frontend_openai_key || check_openai_available().await) {
        return Some(AIProvider::OpenAI);
    }
    if untried(AIProvider::Anthropic) && check_anthropic_available().await {
        return Some(AIProvider::Anthropic);
    }
    if untried(AIProvider::Gemini) && check_gemini_available().await {
        return Some(AIProvider::Gemini);
    }
    if untried(AIProvider::DeepSeek) && check_deepseek_available().await {
        return Some(AIProvider::DeepSeek);
    }
    None
}

fn provider_info(
    id: AIProvider,
    available: bool,
    configured: bool,
    model: Option<String>,
    endpoint: Option<String>,
) -> ProviderInfo {
    let caps = crate::ai_providers::capabilities(id);
    ProviderInfo {
        id,
        available,
        configured,
        model: model.filter(|m| !m.trim().is_empty()),
        endpoint,
        supports_tools: caps.supports_tools,
        supports_streaming: caps.supports_streaming,
    }
}

/// Get current AI provider status
pub async fn get_ai_status() -> AIProviderStatus {
    let pref = get_user_preference();
    let settings = crate::commands::settings::read_settings_from_disk().unwrap_or_default();
    let openai_available = check_openai_available().await;
    let (phi_available, phi_ready, phi_message) = check_phi_silica_available();
    let foundry_endpoint = check_foundry_local_available().await;
    let ollama_endpoint = check_ollama_available().await;
    let custom_endpoint = check_custom_available().await;
    let codex_probe = crate::ai_providers::cli_bridge::probe(AIProvider::CodexCli).await;
    let claude_probe = crate::ai_providers::cli_bridge::probe(AIProvider::ClaudeCode).await;
    let anthropic_available = check_anthropic_available().await;
    let gemini_available = check_gemini_available().await;
    let deepseek_available = check_deepseek_available().await;
    // Status reports ALL providers, so every availability is gathered (no
    // lazy short-circuit here) and routing reuses the same snapshot instead
    // of re-probing via determine_active_provider
    let avail = ProviderAvailability {
        phi: phi_ready,
        foundry: foundry_endpoint.is_some(),
        ollama: ollama_endpoint.is_some(),
        custom: custom_endpoint.is_some(),
        codex: codex_probe.usable(),
        claude: claude_probe.usable(),
        openai: openai_available,
        anthropic: anthropic_available,
        gemini: gemini_available,
        deepseek: deepseek_available,
    };
    let active = route_provider(pref, avail);

    let custom_configured = settings
        .custom_endpoint
        .as_deref()
        .is_some_and(|e| !e.trim().is_empty())
        && settings
            .custom_model
            .as_deref()
            .is_some_and(|m| !m.trim().is_empty());

    // In Auto routing order, which is also a sensible display order
    let providers = vec![
        provider_info(AIProvider::PhiSilica, phi_ready, phi_available, None, None),
        provider_info(
            AIProvider::FoundryLocal,
            foundry_endpoint.is_some(),
            foundry_endpoint.is_some(),
            Some(
                settings
                    .local_ai_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::ai_providers::foundry::FOUNDRY_LOCAL_MODEL.to_string()
                    }),
            ),
            foundry_endpoint.clone(),
        ),
        provider_info(
            AIProvider::Ollama,
            ollama_endpoint.is_some(),
            // Works with zero config when the default endpoint answers
            true,
            settings.ollama_model.clone(),
            ollama_endpoint,
        ),
        provider_info(
            AIProvider::CustomOpenAI,
            custom_endpoint.is_some(),
            custom_configured,
            settings.custom_model.clone(),
            custom_endpoint.or_else(|| settings.custom_endpoint.clone()),
        ),
        provider_info(
            AIProvider::CodexCli,
            // Usable = signed in; configured = installed, so the UI can show
            // a Sign in action for the installed-but-signed-out state
            codex_probe.usable(),
            codex_probe.path.is_some(),
            settings.codex_model.clone(),
            codex_probe.path.map(|p| p.display().to_string()),
        ),
        provider_info(
            AIProvider::ClaudeCode,
            claude_probe.usable(),
            claude_probe.path.is_some(),
            settings.claude_model.clone(),
            claude_probe.path.map(|p| p.display().to_string()),
        ),
        provider_info(
            AIProvider::OpenAI,
            openai_available,
            openai_available,
            Some(
                settings
                    .open_ai_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| crate::ai_providers::openai::OPENAI_MODEL.to_string()),
            ),
            None,
        ),
        provider_info(
            AIProvider::Anthropic,
            anthropic_available,
            anthropic_available,
            Some(
                settings
                    .anthropic_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::ai_providers::anthropic::ANTHROPIC_DEFAULT_MODEL.to_string()
                    }),
            ),
            None,
        ),
        provider_info(
            AIProvider::Gemini,
            gemini_available,
            gemini_available,
            Some(
                settings
                    .gemini_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::ai_providers::gemini::GEMINI_DEFAULT_MODEL.to_string()
                    }),
            ),
            None,
        ),
        provider_info(
            AIProvider::DeepSeek,
            deepseek_available,
            deepseek_available,
            Some(
                settings
                    .deepseek_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::ai_providers::deepseek::DEEPSEEK_DEFAULT_MODEL.to_string()
                    }),
            ),
            None,
        ),
    ];

    AIProviderStatus {
        preferred_provider: active,
        openai_available,
        openai_api_key_set: openai_available,
        phi_silica_available: phi_available,
        phi_silica_ready: phi_ready,
        phi_silica_message: phi_message,
        foundry_local_available: foundry_endpoint.is_some(),
        foundry_local_endpoint: foundry_endpoint,
        active_provider: active,
        providers,
    }
}

/// Generate cache key for a request. The provider is part of the key so a
/// response truncated for one provider's budget is never served for another,
/// and a hit always reports the provider that actually produced it.
fn generate_cache_key(
    request: &AIRequest,
    session_id: &str,
    provider: AIProvider,
    config_fingerprint: &str,
    grounding: Option<&str>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    request.data.hash(&mut hasher);
    grounding.unwrap_or_default().hash(&mut hasher);
    let content_hash = hasher.finish();

    format!(
        "{}:{}:{:?}:{}:{}:{}:{:x}",
        session_id,
        ANALYSIS_CACHE_VERSION,
        request.context_type,
        request.context_id,
        provider,
        config_fingerprint,
        content_hash
    )
}

pub(crate) fn provider_config_fingerprint(
    provider: AIProvider,
    cfg: &crate::ai_providers::ResolvedProviderConfig,
) -> String {
    fn key_fingerprint(key: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        "wfdiag-ai-key-v1".hash(&mut hasher);
        key.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    let key = cfg
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .map(key_fingerprint)
        .unwrap_or_else(|| "none".to_string());

    format!(
        "provider={};endpoint={};model={};key={}",
        provider,
        cfg.endpoint.as_deref().unwrap_or_default(),
        cfg.model.as_deref().unwrap_or_default(),
        key
    )
}

/// How many characters of diagnostic DATA a one-shot prompt may embed for a
/// provider — roughly half its whole-request budget (the rest is template,
/// system prompt and output headroom). Phi Silica lands at ~1,250, close to
/// its long-proven 1,200, and its hard 2,500-char prompt clamp still applies
/// as the final guard; cloud providers get the full 20k data window.
fn one_shot_data_budget(provider: AIProvider) -> usize {
    let budget = crate::ai_providers::capabilities(provider).context_budget_chars;
    (budget / 2).clamp(800, 20_000)
}

fn one_shot_grounding_budget(provider: AIProvider, data_budget: usize) -> usize {
    if provider == AIProvider::PhiSilica {
        650
    } else {
        (data_budget / 3).clamp(1_200, 5_000)
    }
}

fn one_shot_effective_data_budget(
    provider: AIProvider,
    data_budget: usize,
    grounding: Option<&str>,
) -> usize {
    if provider == AIProvider::PhiSilica && grounding.is_some() {
        800
    } else {
        data_budget
    }
}

/// Analyze with the appropriate provider
/// If api_key is provided, it counts as OpenAI availability for routing
pub async fn analyze(
    request: AIRequest,
    session_id: &str,
    api_key: Option<String>,
    force_refresh: bool,
) -> Result<AIResponse, String> {
    // Determine the provider first — it is part of the cache key
    let pref = get_user_preference();
    let frontend_key = api_key.as_deref().is_some_and(|k| !k.is_empty());
    let provider = determine_active_provider_with_key(pref, frontend_key).await;
    let cfg = crate::ai_providers::resolve_config(provider, api_key).await?;
    let config_fingerprint = provider_config_fingerprint(provider, &cfg);
    let data_budget = one_shot_data_budget(provider);

    let grounding = if provider == AIProvider::None {
        None
    } else {
        crate::ai_grounding::analysis_grounding(
            request.context_type,
            request
                .task_name
                .as_deref()
                .or(request.section_name.as_deref()),
            &request.data,
            one_shot_grounding_budget(provider, data_budget),
        )
        .await
    };
    let grounding_context = grounding
        .as_ref()
        .and_then(|grounding| grounding.prompt_context.as_deref());
    let grounding_trace = grounding.as_ref().map(|grounding| grounding.trace.clone());

    let cache_key = generate_cache_key(
        &request,
        session_id,
        provider,
        &config_fingerprint,
        grounding_context,
    );
    if !force_refresh
        && let Ok(mut cache) = get_cache().lock()
        && let Some(cached) = cache.get(&cache_key)
    {
        return Ok(AIResponse {
            interpretation: cached.clone(),
            provider_used: provider,
            cached: true,
            grounding: grounding_trace,
            error: None,
        });
    }

    // Generate prompt based on context type, sized to the provider's budget
    let data_budget = one_shot_effective_data_budget(provider, data_budget, grounding_context);
    let prompt = match request.context_type {
        ContextType::DiagnosticInterpretation => ai_prompts::diagnostic_interpretation_prompt(
            request.task_name.as_deref().unwrap_or("Unknown"),
            &request.data,
            data_budget,
        ),
        ContextType::SectionSummary => ai_prompts::section_summary_prompt(
            request.section_name.as_deref().unwrap_or("System"),
            &request.data,
            data_budget,
        ),
        ContextType::HealthScoreExplanation => {
            ai_prompts::health_explanation_prompt(&request.data, data_budget)
        }
        ContextType::IssuePrioritization => {
            ai_prompts::issue_prioritization_prompt(&request.data, data_budget)
        }
        ContextType::GeneralAnalysis => request.data.clone(),
    };
    let prompt = ai_prompts::attach_grounding(prompt, grounding_context);

    // Call the provider client with the config already used for the cache key.
    let result = crate::ai_providers::one_shot(provider, &cfg, SYSTEM_PROMPT, &prompt).await;

    // Cache successful, non-empty results. Some providers return Ok("") on a
    // refusal or empty completion instead of an error — caching that would
    // serve a blank analysis for the cache's full TTL.
    if let Ok(ref interpretation) = result
        && !interpretation.is_empty()
        && let Ok(mut cache) = get_cache().lock()
    {
        cache.insert(cache_key, interpretation.clone());
    }

    result.map(|interpretation| AIResponse {
        interpretation,
        provider_used: provider,
        cached: false,
        grounding: grounding_trace,
        error: None,
    })
}

/// Read the shared AI response cache (used by the scan report).
pub(crate) fn cached_value(key: &str) -> Option<String> {
    get_cache().lock().ok().and_then(|mut cache| cache.get(key))
}

/// Write to the shared AI response cache (used by the scan report).
pub(crate) fn cache_value(key: String, value: String) {
    if let Ok(mut cache) = get_cache().lock() {
        cache.insert(key, value);
    }
}

/// Clear AI cache for a session or all
pub fn clear_cache(session_id: Option<&str>) {
    if let Ok(mut cache) = get_cache().lock() {
        if let Some(sid) = session_id {
            cache.clear_session(sid);
        } else {
            cache.clear_all();
        }
    }
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Get AI provider status
#[tauri::command]
pub async fn ai_get_status() -> Result<AIProviderStatus, String> {
    Ok(get_ai_status().await)
}

/// Analyze a single diagnostic
#[tauri::command]
pub async fn ai_analyze_diagnostic(
    task_id: String,
    task_name: String,
    diagnostic_output: String,
    session_id: String,
    api_key: Option<String>,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::DiagnosticInterpretation,
        context_id: task_id,
        data: diagnostic_output,
        task_name: Some(task_name),
        section_name: None,
    };

    analyze(
        request,
        &session_id,
        api_key,
        force_refresh.unwrap_or(false),
    )
    .await
}

/// Analyze a section (Hardware, System, Storage, Network)
#[tauri::command]
pub async fn ai_analyze_section(
    section_name: String,
    section_data: String,
    session_id: String,
    api_key: Option<String>,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::SectionSummary,
        context_id: section_name.clone(),
        data: section_data,
        task_name: None,
        section_name: Some(section_name),
    };

    analyze(
        request,
        &session_id,
        api_key,
        force_refresh.unwrap_or(false),
    )
    .await
}

/// Explain health scores
#[tauri::command]
pub async fn ai_explain_health(
    metrics_data: String,
    session_id: String,
    api_key: Option<String>,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::HealthScoreExplanation,
        context_id: "health".to_string(),
        data: metrics_data,
        task_name: None,
        section_name: None,
    };

    analyze(
        request,
        &session_id,
        api_key,
        force_refresh.unwrap_or(false),
    )
    .await
}

/// Set AI provider preference
#[tauri::command]
pub async fn ai_set_preference(preference: String) -> Result<(), String> {
    let pref = match preference.to_lowercase().as_str() {
        "openai" => AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => AIProviderPreference::PhiSilica,
        "foundry_local" | "foundrylocal" => AIProviderPreference::FoundryLocal,
        "ollama" => AIProviderPreference::Ollama,
        "custom_openai" | "custom" => AIProviderPreference::CustomOpenAI,
        "codex_cli" | "codexcli" | "codex" => AIProviderPreference::CodexCli,
        "claude_code" | "claudecode" | "claude" => AIProviderPreference::ClaudeCode,
        "anthropic" => AIProviderPreference::Anthropic,
        "gemini" => AIProviderPreference::Gemini,
        "deepseek" => AIProviderPreference::DeepSeek,
        _ => AIProviderPreference::Auto,
    };
    set_user_preference(pref);
    Ok(())
}

/// Prioritize detected issues (the IssuePrioritization prompt finally gets a
/// caller — analyzeGeneric would mis-route through the section template).
#[tauri::command]
pub async fn ai_prioritize_issues(
    issues_data: String,
    session_id: String,
    api_key: Option<String>,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::IssuePrioritization,
        context_id: "issues".to_string(),
        data: issues_data,
        task_name: None,
        section_name: None,
    };
    analyze(
        request,
        &session_id,
        api_key,
        force_refresh.unwrap_or(false),
    )
    .await
}

/// Clear AI cache
#[tauri::command]
pub async fn ai_clear_cache(session_id: Option<String>) -> Result<(), String> {
    clear_cache(session_id.as_deref());
    Ok(())
}

/// System prompt for AI analysis
// Phi Silica never sees this (its one-shot path skips the system prompt to
// preserve the on-device budget); per-prompt length hints in ai_prompts.rs
// handle the compact case.
const SYSTEM_PROMPT: &str = r#"You are a Windows system diagnostic expert. Analyze the provided data and give a clear, concise interpretation.

Guidelines:
- Be direct and specific
- Focus on actionable insights
- Mention any anomalies or concerns, with the values that matter
- Use live WindowsForum MCP grounding when present for current Windows release, KB, support, driver, and known-issue facts; cite source title/URL when using it
- Do not infer release channel, Insider/pre-release status, support status, or update availability from a build number alone
- Treat the grounding block as newer than model memory. If grounding lists Microsoft Support update-history pages for a build, do not call that build Insider/Preview unless those sources explicitly say the installed build is Insider/Preview
- A base BuildNumber such as 26200 is not enough to decide patch compliance. Only compare updates when the diagnostic includes UBR or FullBuild
- If live grounding is unavailable or inconclusive, say that instead of guessing
- Be as brief as the data allows; never pad
- Use technical but accessible language"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything available
    fn all() -> ProviderAvailability {
        ProviderAvailability {
            phi: true,
            foundry: true,
            ollama: true,
            custom: true,
            codex: true,
            claude: true,
            openai: true,
            anthropic: true,
            gemini: true,
            deepseek: true,
        }
    }

    #[test]
    fn auto_walks_the_priority_chain_link_by_link() {
        // Local-first chain: phi → foundry → ollama → custom → codex →
        // claude → openai → anthropic → gemini → none. Each step turns off
        // the previous winner.
        let mut avail = all();
        let chain = [
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal,
            AIProvider::Ollama,
            AIProvider::CustomOpenAI,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
            AIProvider::OpenAI,
            AIProvider::Anthropic,
            AIProvider::Gemini,
            AIProvider::DeepSeek,
        ];
        for expected in chain {
            assert_eq!(
                route_provider(AIProviderPreference::Auto, avail),
                expected,
                "wrong Auto winner with availability {:?}",
                avail
            );
            match expected {
                AIProvider::PhiSilica => avail.phi = false,
                AIProvider::FoundryLocal => avail.foundry = false,
                AIProvider::Ollama => avail.ollama = false,
                AIProvider::CustomOpenAI => avail.custom = false,
                AIProvider::CodexCli => avail.codex = false,
                AIProvider::ClaudeCode => avail.claude = false,
                AIProvider::OpenAI => avail.openai = false,
                AIProvider::Anthropic => avail.anthropic = false,
                AIProvider::Gemini => avail.gemini = false,
                AIProvider::DeepSeek => avail.deepseek = false,
                _ => unreachable!(),
            }
        }
        // Nothing left
        assert_eq!(
            route_provider(AIProviderPreference::Auto, avail),
            AIProvider::None
        );
    }

    #[test]
    fn auto_with_only_an_openai_key_picks_openai() {
        // Backward compatibility: the pre-2.5.0 single-key setup must keep
        // resolving exactly as it did.
        let avail = ProviderAvailability {
            openai: true,
            ..Default::default()
        };
        assert_eq!(
            route_provider(AIProviderPreference::Auto, avail),
            AIProvider::OpenAI
        );
    }

    #[test]
    fn explicit_preference_wins_when_available() {
        let cases = [
            (AIProviderPreference::OpenAI, AIProvider::OpenAI),
            (AIProviderPreference::PhiSilica, AIProvider::PhiSilica),
            (AIProviderPreference::FoundryLocal, AIProvider::FoundryLocal),
            (AIProviderPreference::Ollama, AIProvider::Ollama),
            (AIProviderPreference::CustomOpenAI, AIProvider::CustomOpenAI),
            (AIProviderPreference::CodexCli, AIProvider::CodexCli),
            (AIProviderPreference::ClaudeCode, AIProvider::ClaudeCode),
            (AIProviderPreference::Anthropic, AIProvider::Anthropic),
            (AIProviderPreference::Gemini, AIProvider::Gemini),
            (AIProviderPreference::DeepSeek, AIProvider::DeepSeek),
        ];
        for (pref, expected) in cases {
            assert_eq!(route_provider(pref, all()), expected);
        }
    }

    #[test]
    fn explicit_preference_never_falls_back() {
        // An unavailable explicit choice yields None rather than silently
        // routing to a provider the user did not pick — for every provider.
        let unavailable = |pref| {
            let mut avail = all();
            match pref {
                AIProviderPreference::PhiSilica => avail.phi = false,
                AIProviderPreference::FoundryLocal => avail.foundry = false,
                AIProviderPreference::Ollama => avail.ollama = false,
                AIProviderPreference::CustomOpenAI => avail.custom = false,
                AIProviderPreference::CodexCli => avail.codex = false,
                AIProviderPreference::ClaudeCode => avail.claude = false,
                AIProviderPreference::OpenAI => avail.openai = false,
                AIProviderPreference::Anthropic => avail.anthropic = false,
                AIProviderPreference::Gemini => avail.gemini = false,
                AIProviderPreference::DeepSeek => avail.deepseek = false,
                AIProviderPreference::Auto => unreachable!(),
            }
            avail
        };
        for pref in [
            AIProviderPreference::OpenAI,
            AIProviderPreference::PhiSilica,
            AIProviderPreference::FoundryLocal,
            AIProviderPreference::Ollama,
            AIProviderPreference::CustomOpenAI,
            AIProviderPreference::CodexCli,
            AIProviderPreference::ClaudeCode,
            AIProviderPreference::Anthropic,
            AIProviderPreference::Gemini,
            AIProviderPreference::DeepSeek,
        ] {
            assert_eq!(
                route_provider(pref, unavailable(pref)),
                AIProvider::None,
                "preference {:?} fell back instead of yielding None",
                pref
            );
        }
    }

    #[test]
    fn provider_wire_strings_are_pinned_for_frontend() {
        // The frontend AIProvider union type depends on these exact strings.
        // "openai" regression-pins the rename_all bug that emitted "open_a_i".
        let cases = [
            (AIProvider::None, "\"none\""),
            (AIProvider::OpenAI, "\"openai\""),
            (AIProvider::PhiSilica, "\"phi_silica\""),
            (AIProvider::FoundryLocal, "\"foundry_local\""),
            (AIProvider::Ollama, "\"ollama\""),
            (AIProvider::CustomOpenAI, "\"custom_openai\""),
            (AIProvider::CodexCli, "\"codex_cli\""),
            (AIProvider::ClaudeCode, "\"claude_code\""),
            (AIProvider::Anthropic, "\"anthropic\""),
            (AIProvider::Gemini, "\"gemini\""),
            (AIProvider::DeepSeek, "\"deepseek\""),
        ];
        for (provider, wire) in cases {
            assert_eq!(serde_json::to_string(&provider).unwrap(), wire);
            // Round-trip
            assert_eq!(serde_json::from_str::<AIProvider>(wire).unwrap(), provider);
        }
        // Legacy "open_a_i" strings still deserialize
        assert_eq!(
            serde_json::from_str::<AIProvider>("\"open_a_i\"").unwrap(),
            AIProvider::OpenAI
        );
    }

    #[test]
    fn provider_config_fingerprint_changes_on_key_rotation_without_leaking_key() {
        let first = crate::ai_providers::ResolvedProviderConfig {
            api_key: Some("sk-old-secret".to_string()),
            endpoint: Some("https://api.openai.com".to_string()),
            model: Some("gpt-test".to_string()),
        };
        let second = crate::ai_providers::ResolvedProviderConfig {
            api_key: Some("sk-new-secret".to_string()),
            ..first.clone()
        };

        let first_fp = provider_config_fingerprint(AIProvider::OpenAI, &first);
        let second_fp = provider_config_fingerprint(AIProvider::OpenAI, &second);

        assert_ne!(first_fp, second_fp);
        assert!(!first_fp.contains("sk-old-secret"));
        assert!(!second_fp.contains("sk-new-secret"));
        assert!(first_fp.contains("endpoint=https://api.openai.com"));
        assert!(first_fp.contains("model=gpt-test"));
    }

    #[test]
    fn preference_wire_strings_round_trip() {
        for (pref, wire) in [
            (AIProviderPreference::Auto, "\"auto\""),
            (AIProviderPreference::OpenAI, "\"openai\""),
            (AIProviderPreference::PhiSilica, "\"phi_silica\""),
            (AIProviderPreference::FoundryLocal, "\"foundry_local\""),
            (AIProviderPreference::Ollama, "\"ollama\""),
            (AIProviderPreference::CustomOpenAI, "\"custom_openai\""),
            (AIProviderPreference::CodexCli, "\"codex_cli\""),
            (AIProviderPreference::ClaudeCode, "\"claude_code\""),
            (AIProviderPreference::Anthropic, "\"anthropic\""),
            (AIProviderPreference::Gemini, "\"gemini\""),
            (AIProviderPreference::DeepSeek, "\"deepseek\""),
        ] {
            assert_eq!(serde_json::to_string(&pref).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<AIProviderPreference>(wire).unwrap(),
                pref
            );
        }
    }
}
