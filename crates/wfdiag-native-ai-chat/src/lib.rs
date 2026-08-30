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

mod compat_provider;
mod contract;
mod engine;
mod model;
mod provider_config;

/// Adapter shim: the included shipping client refers to the provider enum by
/// its historical backend path.
pub mod ai_service {
    pub use wfdiag_native_ai_provider::AIProvider;
}

/// Adapter shim: Ollama capability probing lives in the provider crate.
pub mod ollama {
    pub use wfdiag_native_ai_provider::ollama_model_supports_tools as model_supports_tools;
}

// Included verbatim from the shipping backend so both shells compile the
// exact same client; edit it there, never here.
#[path = "../../../src-tauri/src/ai_providers/openai_compat.rs"]
pub mod openai_compat;

pub use compat_provider::CompatChatProvider;
pub use contract::*;
pub use engine::*;
pub use model::*;
pub use provider_config::*;
