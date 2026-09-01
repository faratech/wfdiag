//! Provider transports. Both desktop shells compile exactly these clients, so
//! request shape, streaming, tool replay, and provider-specific error handling
//! cannot drift between Tauri and the native shell.
//!
//! Every module here refers to the shared provider-neutral contract, the
//! process bridge, and the prompt-flattening shim through `super::…`; the
//! re-exports below are what make those paths resolve.

pub mod openai_compat;

pub mod anthropic;
pub mod deepseek;
#[allow(dead_code)]
pub mod gemini;
pub(crate) mod sse;

// The subscription transports drive the locally installed Codex / Claude Code
// CLIs. Their process bridge lives in this crate, without any desktop
// framework dependency.
#[allow(dead_code)]
pub(crate) mod acp_bridge;
#[allow(dead_code)]
pub mod claude_cli;
#[allow(dead_code)]
pub mod codex;

pub(crate) use crate::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ProviderReplay,
    ResolvedProviderConfig, ToolCall, ToolSpec, cli_bridge, ollama, phi,
};
