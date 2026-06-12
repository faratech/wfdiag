//! AI provider layer: capabilities and per-provider clients.
//!
//! wfdiag supports seven AI providers behind one enum (`ai_service::AIProvider`):
//!
//! | Provider       | Where it runs        | Auth            | Module        |
//! |----------------|----------------------|-----------------|---------------|
//! | Phi Silica     | on-device NPU (WinRT)| package identity| `phi_silica`  |
//! | Foundry Local  | local server         | none            | (this layer)  |
//! | Ollama         | local server         | none            | `ollama`      |
//! | Custom         | any OpenAI-compatible| optional key    | (this layer)  |
//! | OpenAI         | cloud                | API key         | (this layer)  |
//! | Anthropic      | cloud                | API key         | (this layer)  |
//! | Gemini         | cloud                | API key         | (this layer)  |
//!
//! Dispatch is by exhaustive `match` on the enum (not trait objects) so the
//! compiler forces every dispatch site to handle a newly added provider.
//! `capabilities()` is the single source of truth for what each provider can
//! do and how much context it gets.

pub mod discovery;
pub mod ollama;

use crate::ai_service::AIProvider;

/// What a provider supports and how much context it may consume.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ProviderCaps {
    /// Provider can run the tool-calling (agentic chat) loop
    pub supports_tools: bool,
    /// Provider can stream response deltas
    pub supports_streaming: bool,
    /// Whole-request character budget (~4 chars/token). Deliberately
    /// cost-capped for cloud providers, not the model's context limit.
    pub context_budget_chars: usize,
}

/// Single source of truth for provider capabilities.
///
/// Phi Silica keeps the empirically validated 4k-token budget from
/// `ai_service`/`ai_prompts` — do not raise it. Foundry Local is conservative
/// (no tools) until its `/v1/chat/completions` path is verified on a real
/// install; its one-shot path stays on `/v1/responses`.
pub fn capabilities(provider: AIProvider) -> ProviderCaps {
    match provider {
        AIProvider::None => ProviderCaps {
            supports_tools: false,
            supports_streaming: false,
            context_budget_chars: 0,
        },
        AIProvider::PhiSilica => ProviderCaps {
            supports_tools: false,
            supports_streaming: false,
            context_budget_chars: 2_500,
        },
        AIProvider::FoundryLocal => ProviderCaps {
            supports_tools: false,
            supports_streaming: true,
            context_budget_chars: 12_000,
        },
        AIProvider::Ollama => ProviderCaps {
            supports_tools: true,
            supports_streaming: true,
            context_budget_chars: 12_000,
        },
        AIProvider::CustomOpenAI => ProviderCaps {
            supports_tools: true,
            supports_streaming: true,
            context_budget_chars: 24_000,
        },
        AIProvider::OpenAI | AIProvider::Anthropic | AIProvider::Gemini => ProviderCaps {
            supports_tools: true,
            supports_streaming: true,
            context_budget_chars: 48_000,
        },
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
        assert_eq!(
            capabilities(AIProvider::Anthropic).context_budget_chars,
            cloud
        );
        assert_eq!(capabilities(AIProvider::Gemini).context_budget_chars, cloud);
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
        // chat-completions path is verified on a real install.
        for (provider, tools) in [
            (AIProvider::PhiSilica, false),
            (AIProvider::FoundryLocal, false),
            (AIProvider::Ollama, true),
            (AIProvider::CustomOpenAI, true),
            (AIProvider::OpenAI, true),
            (AIProvider::Anthropic, true),
            (AIProvider::Gemini, true),
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
