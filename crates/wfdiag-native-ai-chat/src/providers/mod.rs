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
pub mod gemini;
pub(crate) mod sse;

// The subscription transports drive the locally installed Codex / Claude Code
// CLIs. Their process bridge lives in this crate, without any desktop
// framework dependency.
pub(crate) mod acp_bridge;
pub mod claude_cli;
pub mod codex;

pub(crate) use crate::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ProviderReplay,
    ResolvedProviderConfig, ToolCall, ToolSpec, cli_bridge, ollama, phi,
};

/// Read a non-streaming response body under the same hard byte cap the SSE
/// transports enforce (`sse::MAX_RESPONSE_BYTES`): a broken or hostile
/// endpoint must not turn a duration-bounded request into an unbounded
/// allocation.
pub(crate) async fn read_text_capped(
    response: &mut reqwest::Response,
    cap: usize,
) -> Result<String, String> {
    if let Some(declared) = response.content_length()
        && declared > cap as u64
    {
        return Err(format!(
            "response body of {declared} bytes exceeds the {cap}-byte limit"
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("response body read failed: {error}"))?
    {
        if body.len() + chunk.len() > cap {
            return Err(format!(
                "response body exceeded the {cap}-byte limit mid-read"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "response body was not UTF-8".to_string())
}
