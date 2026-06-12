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
use crate::error::DiagError;
use crate::openai_integration::OPENAI_MODEL;

/// AI Provider enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AIProvider {
    #[default]
    None,
    OpenAI,
    /// On-device Phi Silica via the Windows AI APIs (needs package identity)
    PhiSilica,
    /// Local OpenAI-compatible server (Foundry Local), no identity required
    FoundryLocal,
}

impl std::fmt::Display for AIProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIProvider::None => write!(f, "none"),
            AIProvider::OpenAI => write!(f, "openai"),
            AIProvider::PhiSilica => write!(f, "phi_silica"),
            AIProvider::FoundryLocal => write!(f, "foundry_local"),
        }
    }
}

/// User preference for AI provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AIProviderPreference {
    #[default]
    Auto,
    OpenAI,
    PhiSilica,
    FoundryLocal,
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
    pub error: Option<String>,
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
}

/// Global AI service state
/// Note: These are initialized lazily via OnceLock, so explicit initialization is optional
#[allow(dead_code)] // Used for tracking - cache/preference are initialized lazily
static AI_SERVICE_INITIALIZED: AtomicBool = AtomicBool::new(false);
static AI_CACHE: OnceLock<std::sync::Mutex<AICache>> = OnceLock::new();
static USER_PREFERENCE: OnceLock<std::sync::Mutex<AIProviderPreference>> = OnceLock::new();

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
    crate::openai_integration::local_ai_endpoint().await
}

/// Pure routing decision: which provider serves a request given the user's
/// preference and what's available. Auto prefers on-device Phi Silica (NPU,
/// no model download), then a local Foundry Local endpoint (still on this
/// machine, no identity needed), then cloud OpenAI. An explicit preference
/// never falls back to a different provider.
pub fn route_provider(
    pref: AIProviderPreference,
    phi_available: bool,
    foundry_available: bool,
    openai_available: bool,
) -> AIProvider {
    match pref {
        AIProviderPreference::Auto => {
            if phi_available {
                AIProvider::PhiSilica
            } else if foundry_available {
                AIProvider::FoundryLocal
            } else if openai_available {
                AIProvider::OpenAI
            } else {
                AIProvider::None
            }
        }
        AIProviderPreference::OpenAI if openai_available => AIProvider::OpenAI,
        AIProviderPreference::PhiSilica if phi_available => AIProvider::PhiSilica,
        AIProviderPreference::FoundryLocal if foundry_available => AIProvider::FoundryLocal,
        _ => AIProvider::None,
    }
}

/// Determine the active provider based on preference and availability.
/// Probes lazily in preference order — the Foundry probe shells out / opens
/// a socket, so it must not run when an earlier provider already won.
pub async fn determine_active_provider(pref: AIProviderPreference) -> AIProvider {
    let (phi, foundry, openai) = match pref {
        AIProviderPreference::OpenAI => (false, false, check_openai_available().await),
        AIProviderPreference::PhiSilica => (check_phi_silica_available().0, false, false),
        AIProviderPreference::FoundryLocal => (
            false,
            check_foundry_local_available().await.is_some(),
            false,
        ),
        AIProviderPreference::Auto => {
            let phi = check_phi_silica_available().0;
            let foundry = !phi && check_foundry_local_available().await.is_some();
            let openai = !phi && !foundry && check_openai_available().await;
            (phi, foundry, openai)
        }
    };
    route_provider(pref, phi, foundry, openai)
}

/// Get current AI provider status
pub async fn get_ai_status() -> AIProviderStatus {
    let pref = get_user_preference();
    let openai_available = check_openai_available().await;
    let (phi_available, phi_ready, phi_message) = check_phi_silica_available();
    let foundry_endpoint = check_foundry_local_available().await;
    // Route from the availability just gathered instead of re-probing every
    // provider a second time via determine_active_provider
    let active = route_provider(
        pref,
        phi_available,
        foundry_endpoint.is_some(),
        openai_available,
    );

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
    }
}

/// Generate cache key for a request
fn generate_cache_key(request: &AIRequest, session_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    request.data.hash(&mut hasher);
    let content_hash = hasher.finish();

    format!(
        "{}:{:?}:{}:{:x}",
        session_id, request.context_type, request.context_id, content_hash
    )
}

/// Analyze with the appropriate provider
/// If api_key is provided, it will be used for OpenAI calls directly
pub async fn analyze(
    request: AIRequest,
    session_id: &str,
    api_key: Option<String>,
) -> Result<AIResponse, String> {
    // Check cache first
    let cache_key = generate_cache_key(&request, session_id);
    if let Ok(cache) = get_cache().lock()
        && let Some(cached) = cache.get(&cache_key)
    {
        return Ok(AIResponse {
            interpretation: cached.clone(),
            provider_used: AIProvider::OpenAI, // Assume OpenAI for cached results
            cached: true,
            error: None,
        });
    }

    // Determine provider - if api_key is provided, we can use OpenAI
    let pref = get_user_preference();
    let provider = if api_key.is_some() {
        // An API key from the frontend makes OpenAI the fallback, but an
        // explicit local preference still wins when that provider is up
        match pref {
            AIProviderPreference::PhiSilica => {
                let (phi_available, _, _) = check_phi_silica_available();
                if phi_available {
                    AIProvider::PhiSilica
                } else {
                    AIProvider::OpenAI
                }
            }
            AIProviderPreference::FoundryLocal => {
                if check_foundry_local_available().await.is_some() {
                    AIProvider::FoundryLocal
                } else {
                    AIProvider::OpenAI
                }
            }
            _ => AIProvider::OpenAI, // Use OpenAI with provided key
        }
    } else {
        determine_active_provider(pref).await
    };

    // Generate prompt based on context type
    let prompt = match request.context_type {
        ContextType::DiagnosticInterpretation => ai_prompts::diagnostic_interpretation_prompt(
            request.task_name.as_deref().unwrap_or("Unknown"),
            &request.data,
        ),
        ContextType::SectionSummary => ai_prompts::section_summary_prompt(
            request.section_name.as_deref().unwrap_or("System"),
            &request.data,
        ),
        ContextType::HealthScoreExplanation => ai_prompts::health_explanation_prompt(&request.data),
        ContextType::IssuePrioritization => ai_prompts::issue_prioritization_prompt(&request.data),
        ContextType::GeneralAnalysis => request.data.clone(),
    };

    // Call the appropriate provider
    let result = match provider {
        AIProvider::OpenAI => analyze_with_openai(&prompt, api_key).await,
        AIProvider::PhiSilica => analyze_with_phi_silica(&prompt).await,
        AIProvider::FoundryLocal => analyze_with_foundry_local(&prompt).await,
        AIProvider::None => Err(DiagError::ai_unavailable(
            "none",
            "No AI provider available. Configure an OpenAI API key in Settings, install \
             Foundry Local (winget install Microsoft.FoundryLocal) for local AI, or use \
             the Microsoft Store version on a Copilot+ PC for on-device Phi Silica.",
        )
        .into()),
    };

    // Cache successful results
    if let Ok(ref interpretation) = result
        && let Ok(mut cache) = get_cache().lock()
    {
        cache.insert(cache_key, interpretation.clone());
    }

    result.map(|interpretation| AIResponse {
        interpretation,
        provider_used: provider,
        cached: false,
        error: None,
    })
}

/// Analyze using OpenAI Responses API
/// If api_key is provided, uses it directly; otherwise loads from DPAPI storage
async fn analyze_with_openai(prompt: &str, api_key: Option<String>) -> Result<String, String> {
    use async_openai::{
        Client,
        config::OpenAIConfig,
        types::responses::{CreateResponseArgs, InputParam},
    };

    // Use provided key or load from storage
    let api_key = match api_key {
        Some(key) if !key.is_empty() => key,
        _ => crate::load_api_key_internal().await.ok_or_else(|| {
            DiagError::api_key(
                "load",
                "OpenAI API key not configured. Please enter your API key in Settings.",
            )
        })?,
    };

    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    let full_prompt = format!("{}\n\n{}", SYSTEM_PROMPT, prompt);

    let request = CreateResponseArgs::default()
        .model(OPENAI_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| DiagError::AiAnalysisFailed {
            reason: format!("Failed to build request: {}", e),
        })?;

    let response = client.responses().create(request).await.map_err(|e| {
        eprintln!("OpenAI API error in ai_service: {:?}", e);
        DiagError::AiAnalysisFailed {
            reason: format!("OpenAI API error: {}", e),
        }
    })?;

    Ok(response.output_text().unwrap_or_default())
}

/// Maximum prompt size for Phi Silica
/// Phi Silica has 4k token context (~4 chars/token = ~16k chars max)
/// Use 2500 chars for prompt to leave room for output (~1500 chars)
const PHI_SILICA_MAX_PROMPT_CHARS: usize = 2500;

/// Analyze using Phi Silica
/// Note: ai_prompts.rs already converts JSON to readable text and truncates
/// This is a final safety check
async fn analyze_with_phi_silica(prompt: &str) -> Result<String, String> {
    // Final safety truncation if prompt is still too large. Count and slice by CHARACTER,
    // not byte index, so a multi-byte UTF-8 char at the boundary can't panic (and with
    // panic = "abort" in release, abort the whole process).
    let final_prompt = if prompt.chars().count() > PHI_SILICA_MAX_PROMPT_CHARS {
        let head: String = prompt
            .chars()
            .take(PHI_SILICA_MAX_PROMPT_CHARS - 25)
            .collect();
        format!("{}... [input truncated]", head)
    } else {
        prompt.to_string()
    };

    crate::phi_silica::generate_response(&final_prompt).await
}

/// Analyze using a local OpenAI-compatible endpoint (Foundry Local).
/// The endpoint is resolved dynamically — its port must not be hardcoded.
async fn analyze_with_foundry_local(prompt: &str) -> Result<String, String> {
    use async_openai::{
        Client,
        config::OpenAIConfig,
        types::responses::{CreateResponseArgs, InputParam},
    };

    let endpoint = check_foundry_local_available().await.ok_or_else(|| {
        DiagError::ai_unavailable(
            "foundry_local",
            "No local AI endpoint available. Install Foundry Local and run 'foundry service \
             start', or configure an endpoint in Settings.",
        )
    })?;

    let config = OpenAIConfig::new()
        .with_api_base(format!("{}/v1", endpoint))
        .with_api_key("not-needed");
    let client = Client::with_config(config);

    let full_prompt = format!("{}\n\n{}", SYSTEM_PROMPT, prompt);

    let request = CreateResponseArgs::default()
        .model(crate::openai_integration::FOUNDRY_LOCAL_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| DiagError::AiAnalysisFailed {
            reason: format!("Failed to build request: {}", e),
        })?;

    let response = client.responses().create(request).await.map_err(|e| {
        eprintln!("Foundry Local API error in ai_service: {:?}", e);
        DiagError::AiAnalysisFailed {
            reason: format!(
                "Foundry Local error: {}. Ensure the service is running and the model '{}' \
                 is loaded (foundry model run {}).",
                e,
                crate::openai_integration::FOUNDRY_LOCAL_MODEL,
                crate::openai_integration::FOUNDRY_LOCAL_MODEL
            ),
        }
    })?;

    Ok(response.output_text().unwrap_or_default())
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
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::DiagnosticInterpretation,
        context_id: task_id,
        data: diagnostic_output,
        task_name: Some(task_name),
        section_name: None,
    };

    analyze(request, &session_id, api_key).await
}

/// Analyze a section (Hardware, System, Storage, Network)
#[tauri::command]
pub async fn ai_analyze_section(
    section_name: String,
    section_data: String,
    session_id: String,
    api_key: Option<String>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::SectionSummary,
        context_id: section_name.clone(),
        data: section_data,
        task_name: None,
        section_name: Some(section_name),
    };

    analyze(request, &session_id, api_key).await
}

/// Explain health scores
#[tauri::command]
pub async fn ai_explain_health(
    metrics_data: String,
    session_id: String,
    api_key: Option<String>,
) -> Result<AIResponse, String> {
    let request = AIRequest {
        context_type: ContextType::HealthScoreExplanation,
        context_id: "health".to_string(),
        data: metrics_data,
        task_name: None,
        section_name: None,
    };

    analyze(request, &session_id, api_key).await
}

/// Set AI provider preference
#[tauri::command]
pub async fn ai_set_preference(preference: String) -> Result<(), String> {
    let pref = match preference.to_lowercase().as_str() {
        "openai" => AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => AIProviderPreference::PhiSilica,
        "foundry_local" | "foundrylocal" => AIProviderPreference::FoundryLocal,
        _ => AIProviderPreference::Auto,
    };
    set_user_preference(pref);
    Ok(())
}

/// Clear AI cache
#[tauri::command]
pub async fn ai_clear_cache(session_id: Option<String>) -> Result<(), String> {
    clear_cache(session_id.as_deref());
    Ok(())
}

/// System prompt for AI analysis
const SYSTEM_PROMPT: &str = r#"You are a Windows system diagnostic expert. Analyze the provided data and give a clear, concise interpretation.

Guidelines:
- Be direct and specific
- Focus on actionable insights
- Mention any anomalies or concerns
- Keep responses under 150 words
- Use technical but accessible language"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_phi_silica_over_everything() {
        assert_eq!(
            route_provider(AIProviderPreference::Auto, true, true, true),
            AIProvider::PhiSilica
        );
    }

    #[test]
    fn auto_falls_back_to_foundry_then_openai() {
        assert_eq!(
            route_provider(AIProviderPreference::Auto, false, true, true),
            AIProvider::FoundryLocal
        );
        assert_eq!(
            route_provider(AIProviderPreference::Auto, false, false, true),
            AIProvider::OpenAI
        );
        assert_eq!(
            route_provider(AIProviderPreference::Auto, false, false, false),
            AIProvider::None
        );
    }

    #[test]
    fn explicit_preference_wins_when_available() {
        assert_eq!(
            route_provider(AIProviderPreference::OpenAI, true, true, true),
            AIProvider::OpenAI
        );
        assert_eq!(
            route_provider(AIProviderPreference::FoundryLocal, true, true, true),
            AIProvider::FoundryLocal
        );
        assert_eq!(
            route_provider(AIProviderPreference::PhiSilica, true, true, true),
            AIProvider::PhiSilica
        );
    }

    #[test]
    fn explicit_preference_never_falls_back() {
        // An unavailable explicit choice yields None rather than silently
        // routing to a provider the user did not pick
        assert_eq!(
            route_provider(AIProviderPreference::PhiSilica, false, true, true),
            AIProvider::None
        );
        assert_eq!(
            route_provider(AIProviderPreference::FoundryLocal, true, false, true),
            AIProvider::None
        );
        assert_eq!(
            route_provider(AIProviderPreference::OpenAI, true, true, false),
            AIProvider::None
        );
    }

    #[test]
    fn provider_serializes_snake_case_for_frontend() {
        // The frontend AIProvider union type depends on these exact strings
        assert_eq!(
            serde_json::to_string(&AIProvider::PhiSilica).unwrap(),
            "\"phi_silica\""
        );
        assert_eq!(
            serde_json::to_string(&AIProvider::FoundryLocal).unwrap(),
            "\"foundry_local\""
        );
        assert_eq!(
            serde_json::to_string(&AIProvider::None).unwrap(),
            "\"none\""
        );
        assert_eq!(
            serde_json::from_str::<AIProviderPreference>("\"foundry_local\"").unwrap(),
            AIProviderPreference::FoundryLocal
        );
    }
}
