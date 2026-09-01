//! UI-framework-neutral AI chat state, tool loop, and event contracts.
//!
//! The native Reactor shell and the shipping Tauri shell both use this crate
//! for canonical conversation state, provider-neutral request/response types,
//! bounded tool execution, cancellation, and history projection. UI shells
//! supply only provider, tool, and event adapters. This crate has no `Tauri`,
//! `Wry`, `WebView`, or `Reactor` dependency.
//!
//! The crate also owns the shared `/v1/chat/completions` client
//! ([`openai_compat`]) used by cloud `OpenAI` chat, Foundry Local, Ollama, the
//! custom endpoint, and DeepSeek-compatible streaming in both shells.

#![deny(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

mod bounded_tools;
mod cli_bridge;
mod compat_provider;
mod contract;
mod engine;
mod grounding;
mod model;
mod provider_config;
mod subscription_auth;
mod subscription_catalog;
mod subscription_install;

/// Adapter shim: the included shipping client refers to the provider enum by
/// its historical backend path.
pub mod ai_service {
    pub use wfdiag_native_ai_provider::AIProvider;
}

/// Adapter shim: Ollama capability probing lives in the provider crate.
pub mod ollama {
    pub use wfdiag_native_ai_provider::ollama_model_supports_tools as model_supports_tools;
}

// Every provider transport lives in `providers`; both shells use these exact
// clients. The historical flat paths stay valid through the re-exports below.
pub mod providers;

pub use providers::{anthropic, claude_cli, codex, deepseek, gemini, openai_compat};

/// Adapter shim used by the included subscription transports.
mod phi {
    pub(super) fn flatten_request(request: &super::ChatRequest) -> String {
        super::flatten_chat_request(request)
    }
}

pub use bounded_tools::*;
pub use compat_provider::CompatChatProvider;
pub use contract::*;
pub use engine::*;
pub use grounding::search_windows_knowledge;
pub use model::*;
pub use provider_config::*;
pub use subscription_auth::{
    SubscriptionAuthController, SubscriptionAuthError, SubscriptionAuthOperation,
    SubscriptionAuthProvider, SubscriptionAuthState, SubscriptionAuthStatus,
};
pub use subscription_catalog::ProcessSubscriptionModelCatalogSource;
pub use subscription_install::{
    SubscriptionInstallController, SubscriptionInstallError, SubscriptionInstallFallbackReason,
    SubscriptionInstallMethod, SubscriptionInstallProgress, SubscriptionInstallRequest,
    SubscriptionInstallStage, SubscriptionInstallStatus,
};

/// Render the retained provider-neutral conversation for transports whose
/// native interface accepts one prompt rather than structured messages.
#[must_use]
pub fn flatten_chat_request(request: &ChatRequest) -> String {
    let mut parts = Vec::new();
    if let Some(system) = request.system.as_deref().filter(|text| !text.is_empty()) {
        parts.push(system.to_string());
    }
    for message in &request.messages {
        let speaker = match message.role {
            ChatRole::User => "User",
            ChatRole::Assistant => "Assistant",
            ChatRole::System | ChatRole::Tool => "Context",
        };
        if !message.content.is_empty() {
            parts.push(format!("{speaker}: {}", message.content));
        }
    }
    parts.push("Assistant:".to_string());
    parts.join("\n\n")
}
