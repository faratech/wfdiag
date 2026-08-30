//! Unified AI Service Layer
//!
//! This module provides a unified interface for AI analysis that abstracts
//! OpenAI and Phi Silica providers. It handles provider detection, routing,
//! caching, and rate limiting.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use crate::ai_prompts;
pub use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, AIProviderStatus, ProviderAvailability, route_provider,
};
use wfdiag_native_ai_provider::{
    BackendFuture, CliProbeSnapshot, FoundryEndpointSource, NativeAiProviderRuntime,
    PackageIdentitySource, PhiStatusSnapshot, PhiStatusSource, ProviderManagementService,
    ProviderModelDefaults, ProviderPreferenceSettingsValidator, ProviderProbeBundle,
    ProviderRuntimeError, ProviderSelectionState, SettingsServiceProviderConfigurationSource,
    SharedAiCache, SubscriptionCli, SubscriptionCliStatusSource,
};

fn phi_package_identity_available() -> bool {
    #[cfg(windows)]
    {
        crate::sparse_identity::has_package_identity()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn validate_provider_preference_with_identity(
    preference: AIProviderPreference,
    has_package_identity: bool,
) -> Result<AIProviderPreference, String> {
    wfdiag_native_ai_provider::validate_provider_preference(preference, has_package_identity)
}

/// Reject a provider choice that can never run in the current process. This
/// is deliberately a cheap identity check: readiness remains the normal
/// provider probe, while a loose executable never enters the WinRT path.
pub(crate) fn validate_provider_preference(
    preference: AIProviderPreference,
) -> Result<AIProviderPreference, String> {
    validate_provider_preference_with_identity(preference, phi_package_identity_available())
}

pub(crate) fn parse_and_validate_provider_preference(
    preference: &str,
) -> Result<AIProviderPreference, String> {
    wfdiag_native_ai_provider::parse_and_validate_provider_preference(
        preference,
        phi_package_identity_available(),
    )
}

/// Runtime migration for a stale settings file written by an older loose
/// build. Do not rewrite the user's file; use Auto for this process and return
/// that normalized value to the frontend.
pub(crate) fn provider_preference_for_runtime(preference: &str) -> AIProviderPreference {
    provider_preference_for_runtime_with_identity(preference, phi_package_identity_available())
}

fn provider_preference_for_runtime_with_identity(
    preference: &str,
    has_package_identity: bool,
) -> AIProviderPreference {
    wfdiag_native_ai_provider::provider_preference_for_runtime(preference, has_package_identity)
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
    pub provider_use: crate::state::ProviderUse,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding: Option<crate::ai_grounding::GroundingTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Global AI service state
/// Note: These are initialized lazily via OnceLock, so explicit initialization is optional
#[allow(dead_code)] // Used for tracking - cache/preference are initialized lazily
static AI_SERVICE_INITIALIZED: AtomicBool = AtomicBool::new(false);
static AI_CACHE: OnceLock<SharedAiCache> = OnceLock::new();
static USER_PREFERENCE: OnceLock<ProviderSelectionState> = OnceLock::new();
const ANALYSIS_CACHE_VERSION: &str = "rag-v2";

/// Initialize the AI service (call once at startup)
/// Note: This is optional since OnceLock initializes lazily on first access
#[allow(dead_code)] // Optional - caches initialize lazily
pub fn init_ai_service() {
    if AI_SERVICE_INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }

    AI_CACHE.get_or_init(|| SharedAiCache::new(100));
    USER_PREFERENCE.get_or_init(ProviderSelectionState::default);
}

/// Get the AI cache
pub(crate) fn get_cache() -> &'static SharedAiCache {
    AI_CACHE.get_or_init(|| SharedAiCache::new(100))
}

fn provider_selection() -> &'static ProviderSelectionState {
    USER_PREFERENCE.get_or_init(ProviderSelectionState::default)
}

/// Get current user preference
pub fn get_user_preference() -> AIProviderPreference {
    provider_selection().get()
}

/// Set user preference
pub fn set_user_preference(pref: AIProviderPreference) {
    provider_selection().set(pref);
}

/// Check if OpenAI is available (API key is set)
pub async fn check_openai_available() -> bool {
    crate::load_api_key_internal().await.is_some()
}

/// Check if Phi Silica is available without blocking an async runtime worker.
/// The underlying WinRT probe may create a LanguageModel and wait for a COM
/// operation, so all service callers must use the spawn_blocking-backed command.
pub async fn check_phi_silica_available() -> (bool, bool, Option<String>) {
    match crate::phi_silica::check_phi_silica_available().await {
        Ok(status) => (
            status.available,
            status.ready_state.as_deref() == Some("Ready"),
            Some(status.message),
        ),
        Err(error) => (false, false, Some(error)),
    }
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

/// Determine the active provider based on preference and availability.
/// Probes lazily in priority order — the Foundry probe shells out and the
/// Ollama/custom probes open sockets, so none of them run once an earlier
/// provider has already won. Credentials are detected in backend secure
/// storage only.
pub async fn determine_active_provider(pref: AIProviderPreference) -> AIProvider {
    let mut avail = ProviderAvailability::default();
    match pref {
        AIProviderPreference::OpenAI => {
            avail.openai = check_openai_available().await;
        }
        AIProviderPreference::PhiSilica => avail.phi = check_phi_silica_available().await.1,
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
            avail.phi = check_phi_silica_available().await.1;
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
            avail.openai = !won && check_openai_available().await;
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
) -> Option<AIProvider> {
    if pref != AIProviderPreference::Auto {
        return None;
    }
    if let Some(local) = next_auto_local_provider(pref, tried).await {
        return Some(local);
    }
    let untried = |p: AIProvider| !tried.contains(&p);
    if untried(AIProvider::CustomOpenAI) && check_custom_available().await.is_some() {
        return Some(AIProvider::CustomOpenAI);
    }
    if untried(AIProvider::CodexCli) && check_codex_available().await {
        return Some(AIProvider::CodexCli);
    }
    if untried(AIProvider::ClaudeCode) && check_claude_code_available().await {
        return Some(AIProvider::ClaudeCode);
    }
    if untried(AIProvider::OpenAI) && check_openai_available().await {
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

/// The next private Auto provider only. This deliberately stops before any
/// custom/subscription/API-cloud probes so surfaces without a typed consent
/// flow can retry locally without even touching the cloud fallback path.
pub async fn next_auto_local_provider(
    pref: AIProviderPreference,
    tried: &[AIProvider],
) -> Option<AIProvider> {
    if pref != AIProviderPreference::Auto {
        return None;
    }
    let untried = |provider: AIProvider| !tried.contains(&provider);
    if untried(AIProvider::PhiSilica) && check_phi_silica_available().await.1 {
        return Some(AIProvider::PhiSilica);
    }
    if untried(AIProvider::FoundryLocal) && check_foundry_local_available().await.is_some() {
        return Some(AIProvider::FoundryLocal);
    }
    if untried(AIProvider::Ollama) && check_ollama_available().await.is_some() {
        return Some(AIProvider::Ollama);
    }
    None
}

#[derive(Debug, Default)]
struct TauriPackageIdentitySource;

impl PackageIdentitySource for TauriPackageIdentitySource {
    fn has_package_identity(&self) -> bool {
        phi_package_identity_available()
    }
}

#[derive(Debug, Default)]
struct TauriPhiStatusSource;

impl PhiStatusSource for TauriPhiStatusSource {
    fn probe(&self) -> BackendFuture<'_, PhiStatusSnapshot> {
        Box::pin(async {
            let (available, ready, message) = check_phi_silica_available().await;
            PhiStatusSnapshot {
                available,
                ready,
                message,
            }
        })
    }
}

#[derive(Debug, Default)]
struct TauriFoundryEndpointSource;

impl FoundryEndpointSource for TauriFoundryEndpointSource {
    fn probe(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
        // The existing provider implementation reads the same canonical
        // setting before falling back to CLI discovery.
        Box::pin(check_foundry_local_available())
    }
}

#[derive(Debug, Default)]
struct TauriSubscriptionCliStatusSource;

impl SubscriptionCliStatusSource for TauriSubscriptionCliStatusSource {
    fn probe(
        &self,
        provider: SubscriptionCli,
        _configured_path: Option<String>,
    ) -> BackendFuture<'_, CliProbeSnapshot> {
        // The shipping bridge currently owns its short-lived probe cache and
        // reads the same canonical path setting internally. The explicit path
        // remains part of the shared boundary so a native adapter need not
        // import or reread Tauri settings.
        Box::pin(async move {
            let provider = match provider {
                SubscriptionCli::Codex => AIProvider::CodexCli,
                SubscriptionCli::ClaudeCode => AIProvider::ClaudeCode,
            };
            let probe = crate::ai_providers::cli_bridge::probe(provider).await;
            CliProbeSnapshot {
                usable: probe.usable(),
                installed: probe.path.is_some(),
                path: probe.path.map(|path| path.display().to_string()),
            }
        })
    }
}

pub(crate) fn provider_preference_settings_validator() -> ProviderPreferenceSettingsValidator {
    ProviderPreferenceSettingsValidator::new(Arc::new(TauriPackageIdentitySource))
}

fn native_provider_service() -> ProviderManagementService {
    let identity: Arc<dyn PackageIdentitySource> = Arc::new(TauriPackageIdentitySource);
    let configuration = Arc::new(SettingsServiceProviderConfigurationSource::new(
        crate::commands::settings::native_settings_service(),
    ));
    let probes = ProviderProbeBundle::shipping_networks(
        configuration,
        identity,
        Arc::new(TauriPhiStatusSource),
        Arc::new(TauriFoundryEndpointSource),
        Arc::new(TauriSubscriptionCliStatusSource),
    );
    ProviderManagementService::new(
        probes,
        provider_selection().clone(),
        Arc::new(get_cache().clone()),
        ProviderModelDefaults {
            foundry: crate::ai_providers::foundry::FOUNDRY_LOCAL_MODEL.to_string(),
            openai: crate::ai_providers::openai::OPENAI_MODEL.to_string(),
            anthropic: crate::ai_providers::anthropic::ANTHROPIC_DEFAULT_MODEL.to_string(),
            gemini: crate::ai_providers::gemini::GEMINI_DEFAULT_MODEL.to_string(),
            deepseek: crate::ai_providers::deepseek::DEEPSEEK_DEFAULT_MODEL.to_string(),
        },
    )
}

static NATIVE_PROVIDER_RUNTIME: OnceLock<Result<NativeAiProviderRuntime, ProviderRuntimeError>> =
    OnceLock::new();

fn native_provider_runtime() -> Result<&'static NativeAiProviderRuntime, String> {
    match NATIVE_PROVIDER_RUNTIME
        .get_or_init(|| NativeAiProviderRuntime::start(Arc::new(native_provider_service())))
    {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(error.to_string()),
    }
}

async fn managed_provider_status() -> Result<AIProviderStatus, String> {
    let reply = native_provider_runtime()?
        .request_status()
        .map_err(|error| error.to_string())?;
    reply
        .await
        .map_err(|_| ProviderRuntimeError::WorkerStopped.to_string())
}

pub(crate) async fn managed_ollama_models() -> Result<Vec<String>, String> {
    let reply = native_provider_runtime()?
        .request_ollama_models()
        .map_err(|error| error.to_string())?;
    reply
        .await
        .map_err(|_| ProviderRuntimeError::WorkerStopped.to_string())?
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

/// Analyze with the appropriate provider. Credentials are resolved
/// exclusively from backend secure storage.
pub async fn analyze(
    request: AIRequest,
    session_id: &str,
    force_refresh: bool,
) -> Result<AIResponse, String> {
    let pref = get_user_preference();
    let mut provider = determine_active_provider(pref).await;
    let mut tried = Vec::new();
    let mut failures = Vec::new();
    // Resolve live grounding at most once for the logical request. A provider
    // retry must never turn into a second WindowsForum MCP call.
    let mut grounding_resolved = false;
    let mut grounding: Option<crate::ai_grounding::AnalysisGrounding> = None;

    loop {
        tried.push(provider);
        let attempt = match crate::ai_providers::resolve_config(provider).await {
            Err(error) => Err(error),
            Ok(cfg) => {
                let provider_use = crate::state::ProviderUse::for_provider(
                    provider,
                    tried
                        .first()
                        .copied()
                        .filter(|initial| *initial != provider),
                )
                .with_requested_model(cfg.model.as_deref());
                let config_fingerprint = provider_config_fingerprint(provider, &cfg);
                let base_data_budget = one_shot_data_budget(provider);
                if !grounding_resolved {
                    grounding = crate::ai_grounding::analysis_grounding(
                        request.context_type,
                        request
                            .task_name
                            .as_deref()
                            .or(request.section_name.as_deref()),
                        &request.data,
                        one_shot_grounding_budget(provider, base_data_budget),
                    )
                    .await;
                    grounding_resolved = true;
                }
                let grounding_context = grounding
                    .as_ref()
                    .and_then(|value| value.prompt_context.as_deref());
                let grounding_trace = grounding.as_ref().map(|value| value.trace.clone());
                let cache_key = generate_cache_key(
                    &request,
                    session_id,
                    provider,
                    &config_fingerprint,
                    grounding_context,
                );
                let cached = if force_refresh {
                    None
                } else {
                    get_cache().get(&cache_key)
                };
                if let Some(interpretation) = cached {
                    return Ok(AIResponse {
                        interpretation,
                        provider_used: provider,
                        provider_use,
                        cached: true,
                        grounding: grounding_trace,
                        error: None,
                    });
                }

                let data_budget =
                    one_shot_effective_data_budget(provider, base_data_budget, grounding_context);
                let prompt = match request.context_type {
                    ContextType::DiagnosticInterpretation => {
                        ai_prompts::diagnostic_interpretation_prompt(
                            request.task_name.as_deref().unwrap_or("Unknown"),
                            &request.data,
                            data_budget,
                        )
                    }
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

                match crate::ai_providers::one_shot(provider, &cfg, SYSTEM_PROMPT, &prompt).await {
                    Ok(interpretation) if !interpretation.trim().is_empty() => {
                        get_cache().insert(cache_key, interpretation.clone());
                        Ok(AIResponse {
                            interpretation,
                            provider_used: provider,
                            provider_use,
                            cached: false,
                            grounding: grounding_trace,
                            error: None,
                        })
                    }
                    Ok(_) => Err(format!("{} returned an empty analysis", provider)),
                    Err(error) => Err(error),
                }
            }
        };

        match attempt {
            Ok(response) => return Ok(response),
            Err(error) => {
                // Explicit provider choices never fall back. Auto may recover
                // from Phi/Foundry onto the next private local provider, but
                // one-shot surfaces have no typed consent UI and therefore
                // never cross the local-to-cloud boundary on a retry.
                let can_try_local = pref == AIProviderPreference::Auto
                    && matches!(provider, AIProvider::PhiSilica | AIProvider::FoundryLocal);
                if !can_try_local {
                    if failures.is_empty() {
                        return Err(error);
                    }
                    failures.push(format!("{provider}: {error}"));
                    return Err(format!(
                        "Eligible local AI providers failed: {}",
                        failures.join("; ")
                    ));
                }
                failures.push(format!("{provider}: {error}"));
                match next_auto_local_provider(pref, &tried).await {
                    Some(next) => provider = next,
                    _ => {
                        return Err(format!(
                            "Eligible local AI providers failed: {}",
                            failures.join("; ")
                        ));
                    }
                }
            }
        }
    }
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Get AI provider status
#[tauri::command]
pub async fn ai_get_status() -> Result<AIProviderStatus, String> {
    managed_provider_status().await
}

/// Analyze a single diagnostic
#[tauri::command]
pub async fn ai_analyze_diagnostic(
    task_id: String,
    task_name: String,
    diagnostic_output: String,
    session_id: String,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::DiagnosticInterpretation,
        context_id: task_id,
        data: diagnostic_output,
        task_name: Some(task_name),
        section_name: None,
    };

    analyze(request, &session_id, force_refresh.unwrap_or(false)).await
}

/// Analyze a section (Hardware, System, Storage, Network)
#[tauri::command]
pub async fn ai_analyze_section(
    section_name: String,
    section_data: String,
    session_id: String,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::SectionSummary,
        context_id: section_name.clone(),
        data: section_data,
        task_name: None,
        section_name: Some(section_name),
    };

    analyze(request, &session_id, force_refresh.unwrap_or(false)).await
}

/// Explain health scores
#[tauri::command]
pub async fn ai_explain_health(
    metrics_data: String,
    session_id: String,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::HealthScoreExplanation,
        context_id: "health".to_string(),
        data: metrics_data,
        task_name: None,
        section_name: None,
    };

    analyze(request, &session_id, force_refresh.unwrap_or(false)).await
}

/// Set AI provider preference
#[tauri::command]
pub async fn ai_set_preference(preference: String) -> Result<(), String> {
    let reply = native_provider_runtime()?
        .request_set_preference(preference)
        .map_err(|error| error.to_string())?;
    reply
        .await
        .map_err(|_| ProviderRuntimeError::WorkerStopped.to_string())?
}

/// Prioritize detected issues (the IssuePrioritization prompt finally gets a
/// caller — analyzeGeneric would mis-route through the section template).
#[tauri::command]
pub async fn ai_prioritize_issues(
    issues_data: String,
    session_id: String,
    force_refresh: Option<bool>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::IssuePrioritization,
        context_id: "issues".to_string(),
        data: issues_data,
        task_name: None,
        section_name: None,
    };
    analyze(request, &session_id, force_refresh.unwrap_or(false)).await
}

/// Clear AI cache
#[tauri::command]
pub async fn ai_clear_cache(session_id: Option<String>) -> Result<(), String> {
    let reply = native_provider_runtime()?
        .request_clear_cache(session_id)
        .map_err(|error| error.to_string())?;
    reply
        .await
        .map_err(|_| ProviderRuntimeError::WorkerStopped.to_string())?;
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

    #[test]
    fn provider_preference_parser_is_shared_and_backwards_compatible() {
        assert_eq!(
            wfdiag_native_ai_provider::parse_provider_preference(" PhiSilica "),
            AIProviderPreference::PhiSilica
        );
        assert_eq!(
            wfdiag_native_ai_provider::parse_provider_preference("codex"),
            AIProviderPreference::CodexCli
        );
        assert_eq!(
            wfdiag_native_ai_provider::parse_provider_preference("future_provider"),
            AIProviderPreference::Auto
        );
    }

    #[test]
    fn explicit_phi_without_package_identity_is_rejected_precisely() {
        let error =
            validate_provider_preference_with_identity(AIProviderPreference::PhiSilica, false)
                .unwrap_err();
        assert_eq!(error, wfdiag_native_ai_provider::PHI_SILICA_STORE_REQUIRED);
        assert!(error.contains("Microsoft Store"));
        assert!(error.contains("registered package identity"));
    }

    #[test]
    fn provider_validation_allows_phi_with_identity_and_other_choices_without_it() {
        assert_eq!(
            validate_provider_preference_with_identity(AIProviderPreference::PhiSilica, true),
            Ok(AIProviderPreference::PhiSilica)
        );
        for preference in [
            AIProviderPreference::Auto,
            AIProviderPreference::FoundryLocal,
            AIProviderPreference::CodexCli,
            AIProviderPreference::OpenAI,
        ] {
            assert_eq!(
                validate_provider_preference_with_identity(preference, false),
                Ok(preference)
            );
        }
    }

    #[test]
    fn stale_phi_preference_normalizes_to_auto_only_without_identity() {
        assert_eq!(
            provider_preference_for_runtime_with_identity("phi_silica", false),
            AIProviderPreference::Auto
        );
        assert_eq!(
            provider_preference_for_runtime_with_identity("phi_silica", true),
            AIProviderPreference::PhiSilica
        );
        assert_eq!(
            provider_preference_for_runtime_with_identity("codex_cli", false),
            AIProviderPreference::CodexCli
        );
    }

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
