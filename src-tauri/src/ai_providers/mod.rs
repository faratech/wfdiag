//! AI provider layer: capabilities and per-provider clients.
//!
//! wfdiag supports the AI providers behind one enum (`ai_service::AIProvider`):
//!
//! | Provider       | Where it runs        | Auth            | Module        |
//! |----------------|----------------------|-----------------|---------------|
//! | Phi Silica     | on-device NPU (WinRT)| package identity| `phi_silica`  |
//! | Foundry Local  | local server         | none            | (this layer)  |
//! | Ollama         | local server         | none            | `ollama`      |
//! | Custom         | any OpenAI-compatible| optional key    | (this layer)  |
//! | Codex CLI      | cloud via local CLI  | ChatGPT sign-in | `codex`       |
//! | Claude Code    | cloud via local CLI  | Claude sign-in  | `claude_cli`  |
//! | OpenAI         | cloud                | API key         | (this layer)  |
//! | Anthropic      | cloud                | API key         | (this layer)  |
//! | Gemini         | cloud                | API key         | (this layer)  |
//!
//! Dispatch is by exhaustive `match` on the enum (not trait objects) so the
//! compiler forces every dispatch site to handle a newly added provider.
//! `capabilities()` is the single source of truth for what each provider can
//! do and how much context it gets.

pub mod acp_bridge;
pub mod anthropic;
pub mod cli_bridge;
pub mod deepseek;
pub mod discovery;
pub mod foundry;
pub mod gemini;
pub mod model_catalog;
pub mod ollama;
pub mod openai;
pub mod phi;
pub mod sse;

use crate::ai_service::AIProvider;
use crate::error::DiagError;
use tokio::sync::mpsc;
pub use wfdiag_native_ai_provider::{ProviderCaps, capabilities};

// ============================================================================
// Chat types shared by every provider client and native UI shell
// ============================================================================

pub use wfdiag_native_ai_chat::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ProviderReplay,
    ResolvedProviderConfig, ToolCall, ToolSpec, claude_cli, codex,
};

// The shared `/v1/chat/completions` client compiles once, in the native chat
// crate, and both shells use it.
pub use wfdiag_native_ai_chat::openai_compat;

/// Resolve key/endpoint/model for a provider. Credentials come exclusively
/// from backend secure storage; per-request IPC can never override them.
pub async fn resolve_config(provider: AIProvider) -> Result<ResolvedProviderConfig, String> {
    use crate::commands::settings::{load_provider_key_internal, read_settings_from_disk};
    use crate::dpapi::ProviderKeyId;

    let settings = read_settings_from_disk().unwrap_or_default();

    match provider {
        AIProvider::None => Err(DiagError::ai_unavailable(
            "none",
            "No AI provider available. Configure an API key (OpenAI, Anthropic or Gemini) in \
             Settings, sign in with a ChatGPT or Claude subscription (Codex / Claude Code CLI), \
             install Foundry Local (winget install Microsoft.FoundryLocal) or Ollama for local \
             AI, or use the Microsoft Store version on a Copilot+ PC for on-device Phi Silica.",
        )
        .into()),
        AIProvider::PhiSilica => Ok(ResolvedProviderConfig::default()),
        AIProvider::OpenAI => {
            let api_key = load_provider_key_internal(ProviderKeyId::OpenAI)
                .await
                .ok_or_else(|| {
                    String::from(DiagError::api_key(
                        "load",
                        "OpenAI API key not configured. Please enter your API key in Settings.",
                    ))
                })?;
            let model = settings
                .open_ai_model
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| openai::OPENAI_MODEL.to_string());
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(model),
            })
        }
        AIProvider::FoundryLocal => {
            let endpoint = crate::ai_service::check_foundry_local_available()
                .await
                .ok_or_else(|| {
                    String::from(DiagError::ai_unavailable(
                        "foundry_local",
                        "No local AI endpoint available. Install Foundry Local and run 'foundry \
                         service start', or configure an endpoint in Settings.",
                    ))
                })?;
            let model = settings
                .local_ai_model
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| foundry::FOUNDRY_LOCAL_MODEL.to_string());
            Ok(ResolvedProviderConfig {
                api_key: None,
                endpoint: Some(endpoint),
                model: Some(model),
            })
        }
        AIProvider::Ollama => {
            let endpoint = ollama::discover_endpoint(settings.ollama_endpoint.as_deref())
                .await
                .ok_or_else(|| {
                    String::from(DiagError::ai_unavailable(
                        "ollama",
                        "No Ollama server reachable. Install Ollama (https://ollama.com) and \
                         make sure it is running, or configure its endpoint in Settings.",
                    ))
                })?;
            let model = ollama::resolve_model(&endpoint, settings.ollama_model.as_deref()).await?;
            Ok(ResolvedProviderConfig {
                api_key: None,
                endpoint: Some(endpoint),
                model: Some(model),
            })
        }
        AIProvider::CustomOpenAI => {
            let endpoint = settings
                .custom_endpoint
                .as_deref()
                .and_then(discovery::normalize_base_url)
                .ok_or_else(|| {
                    String::from(DiagError::ai_unavailable(
                        "custom_openai",
                        "No custom endpoint configured. Set the endpoint URL in Settings.",
                    ))
                })?;
            let model = settings
                .custom_model
                .filter(|m| !m.trim().is_empty())
                .ok_or_else(|| {
                    String::from(DiagError::ai_unavailable(
                        "custom_openai",
                        "No model configured for the custom endpoint. Set a model name in \
                         Settings (e.g. the model id your provider documents).",
                    ))
                })?;
            Ok(ResolvedProviderConfig {
                api_key: load_provider_key_internal(ProviderKeyId::Custom).await,
                endpoint: Some(endpoint),
                model: Some(model),
            })
        }
        AIProvider::CodexCli => {
            let probe = cli_bridge::probe(AIProvider::CodexCli).await;
            let path = probe.path.as_ref().ok_or_else(|| {
                String::from(DiagError::ai_unavailable(
                    "codex_cli",
                    "Codex CLI not found. Install it with 'npm install -g @openai/codex' or \
                     set its path in Settings.",
                ))
            })?;
            if !probe.authed {
                return Err(DiagError::ai_unavailable(
                    "codex_cli",
                    "Codex CLI is installed but not signed in. Open Settings → ChatGPT (Codex \
                     CLI) and click Sign in — your browser opens OpenAI's own login (uses your \
                     ChatGPT plan).",
                )
                .into());
            }
            let model = match settings
                .codex_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
            {
                Some(m) => Some(cli_bridge::sanitize_model(m)?),
                None => None,
            };
            Ok(ResolvedProviderConfig {
                api_key: None,
                // For the bridge, "endpoint" is the resolved CLI executable
                endpoint: Some(path.display().to_string()),
                model,
            })
        }
        AIProvider::ClaudeCode => {
            let probe = cli_bridge::probe(AIProvider::ClaudeCode).await;
            let path = probe.path.as_ref().ok_or_else(|| {
                String::from(DiagError::ai_unavailable(
                    "claude_code",
                    "Claude Code CLI not found. Install it with 'npm install -g \
                     @anthropic-ai/claude-code' or set its path in Settings.",
                ))
            })?;
            if !probe.authed {
                return Err(DiagError::ai_unavailable(
                    "claude_code",
                    "Claude Code is installed but not signed in. Open Settings → Claude (Claude \
                     Code CLI) and click Sign in — Anthropic's own login opens in your browser \
                     (uses your Claude plan).",
                )
                .into());
            }
            let model = match settings
                .claude_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
            {
                Some(m) => Some(cli_bridge::sanitize_model(m)?),
                None => None,
            };
            Ok(ResolvedProviderConfig {
                api_key: None,
                // For the bridge, "endpoint" is the resolved CLI executable
                endpoint: Some(path.display().to_string()),
                model,
            })
        }
        AIProvider::Anthropic => {
            let api_key = load_provider_key_internal(ProviderKeyId::Anthropic)
                .await
                .ok_or_else(|| {
                    String::from(DiagError::api_key(
                        "load",
                        "Anthropic API key not configured. Please enter your API key in Settings.",
                    ))
                })?;
            let model = settings
                .anthropic_model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| anthropic::ANTHROPIC_DEFAULT_MODEL.to_string());
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(model),
            })
        }
        AIProvider::Gemini => {
            let api_key = load_provider_key_internal(ProviderKeyId::Gemini)
                .await
                .ok_or_else(|| {
                    String::from(DiagError::api_key(
                        "load",
                        "Gemini API key not configured. Please enter your API key in Settings.",
                    ))
                })?;
            let model = gemini::resolve_model(settings.gemini_model, &api_key).await;
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: None,
                model: Some(model),
            })
        }
        AIProvider::DeepSeek => {
            let api_key = load_provider_key_internal(ProviderKeyId::DeepSeek)
                .await
                .ok_or_else(|| {
                    String::from(DiagError::api_key(
                        "load",
                        "DeepSeek API key not configured. Please enter your API key in Settings.",
                    ))
                })?;
            let model = settings
                .deepseek_model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| deepseek::DEEPSEEK_DEFAULT_MODEL.to_string());
            Ok(ResolvedProviderConfig {
                api_key: Some(api_key),
                endpoint: Some(deepseek::DEEPSEEK_API_BASE.to_string()),
                model: Some(model),
            })
        }
    }
}

// ============================================================================
// Dispatch
// ============================================================================

/// One-shot analysis: a single prompt in, the full response text out.
/// Phi Silica ignores `system` to preserve its tight on-device budget — the
/// `ai_prompts` templates are self-contained instructions.
pub async fn one_shot(
    provider: AIProvider,
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    match provider {
        AIProvider::None => Err("No AI provider available".to_string()),
        AIProvider::PhiSilica => phi::one_shot(prompt).await,
        AIProvider::OpenAI => openai::one_shot(cfg, system, prompt).await,
        AIProvider::FoundryLocal => foundry::one_shot(cfg, system, prompt).await,
        AIProvider::Ollama | AIProvider::CustomOpenAI => {
            openai_compat::one_shot(provider, cfg, system, prompt).await
        }
        AIProvider::DeepSeek => deepseek::one_shot(cfg, system, prompt).await,
        AIProvider::CodexCli => codex::one_shot(cfg, system, prompt).await,
        AIProvider::ClaudeCode => claude_cli::one_shot(cfg, system, prompt).await,
        AIProvider::Anthropic => anthropic::one_shot(cfg, system, prompt).await,
        AIProvider::Gemini => gemini::one_shot(cfg, system, prompt).await,
    }
}

/// Multi-turn chat with optional tool calling, streaming text deltas through
/// `tx`. Non-streaming providers (Phi Silica) send one delta with the full
/// text. Tool calls are never streamed partially — they arrive complete in
/// the returned [`ChatTurn`].
///
/// `is_cancelled` is only consulted by Phi Silica (see
/// [`crate::ai_chat::RealChatProvider::is_cancelled`]); every other provider
/// already cancels correctly when its future is dropped.
pub async fn chat_stream(
    provider: AIProvider,
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
    is_cancelled: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<ChatTurn, String> {
    match provider {
        AIProvider::None => Err("No AI provider available".to_string()),
        AIProvider::PhiSilica => phi::chat_single_shot(req, tx, is_cancelled).await,
        AIProvider::OpenAI
        | AIProvider::FoundryLocal
        | AIProvider::Ollama
        | AIProvider::CustomOpenAI => openai_compat::chat_stream(provider, cfg, req, tx).await,
        AIProvider::DeepSeek => deepseek::chat_stream(cfg, req, tx).await,
        AIProvider::CodexCli => codex::chat_single_shot(cfg, req, tx).await,
        AIProvider::ClaudeCode => claude_cli::chat_single_shot(cfg, req, tx).await,
        AIProvider::Anthropic => anthropic::chat_stream(cfg, req, tx).await,
        AIProvider::Gemini => gemini::chat_stream(cfg, req, tx).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phi_silica_is_one_shot_only_with_tight_budget() {
        let caps = capabilities(AIProvider::PhiSilica);
        assert!(!caps.supports_tools);
        assert!(!caps.supports_streaming);
        // The 4k-token on-device limit is empirical; raising it breaks generation
        assert_eq!(caps.context_budget_chars, 2_500);
    }

    #[test]
    fn budgets_grow_from_local_to_cloud() {
        let phi = capabilities(AIProvider::PhiSilica).context_budget_chars;
        let local = capabilities(AIProvider::Ollama).context_budget_chars;
        let custom = capabilities(AIProvider::CustomOpenAI).context_budget_chars;
        let cloud = capabilities(AIProvider::OpenAI).context_budget_chars;
        assert!(phi < local && local < custom && custom < cloud);
        assert_eq!(
            capabilities(AIProvider::FoundryLocal).context_budget_chars,
            local
        );
        // Subscription is flat-rate but latency grows with prompt size, so
        // the CLI bridges sit at the custom tier, not the cloud tier
        assert_eq!(
            capabilities(AIProvider::CodexCli).context_budget_chars,
            custom
        );
        assert_eq!(
            capabilities(AIProvider::ClaudeCode).context_budget_chars,
            custom
        );
        assert_eq!(
            capabilities(AIProvider::Anthropic).context_budget_chars,
            cloud
        );
        assert_eq!(capabilities(AIProvider::Gemini).context_budget_chars, cloud);
        assert_eq!(
            capabilities(AIProvider::DeepSeek).context_budget_chars,
            cloud
        );
    }

    #[test]
    fn none_provider_has_no_capabilities() {
        let caps = capabilities(AIProvider::None);
        assert!(!caps.supports_tools);
        assert!(!caps.supports_streaming);
        assert_eq!(caps.context_budget_chars, 0);
    }

    #[test]
    fn tool_capable_set_is_exactly_the_chat_capable_providers() {
        // Phi Silica has no tool API; Foundry stays conservative until its
        // chat-completions path is verified on a real install; the Codex CLI
        // runs its own agent loop, never ours.
        for (provider, tools) in [
            (AIProvider::PhiSilica, false),
            (AIProvider::FoundryLocal, false),
            (AIProvider::CodexCli, false),
            (AIProvider::ClaudeCode, false),
            (AIProvider::Ollama, true),
            (AIProvider::CustomOpenAI, true),
            (AIProvider::OpenAI, true),
            (AIProvider::Anthropic, true),
            (AIProvider::Gemini, true),
            (AIProvider::DeepSeek, true),
        ] {
            assert_eq!(
                capabilities(provider).supports_tools,
                tools,
                "unexpected tool capability for {:?}",
                provider
            );
        }
    }
}
