//! On-device Phi Silica client (Microsoft Store build only).
//!
//! Phi Silica has no tool-calling API and no streaming; chat requests are
//! flattened into a single prompt. The 4k-token (~2,500 char) budget is
//! empirical — do not raise it.

use super::{ChatRequest, ChatRole, ChatTurn, FinishReason};
use tokio::sync::mpsc;

/// Maximum prompt size for Phi Silica.
/// Phi Silica has 4k token context (~4 chars/token = ~16k chars max).
/// Use 2500 chars for prompt to leave room for output (~1500 chars).
pub(crate) const PHI_SILICA_MAX_PROMPT_CHARS: usize = 2500;

/// Final safety truncation if a prompt is still too large. Counts and slices
/// by CHARACTER, not byte index, so a multi-byte UTF-8 char at the boundary
/// can't panic (and with panic = "abort" in release, abort the whole process).
fn clamp_prompt(prompt: &str) -> String {
    if prompt.chars().count() > PHI_SILICA_MAX_PROMPT_CHARS {
        let head: String = prompt
            .chars()
            .take(PHI_SILICA_MAX_PROMPT_CHARS - 25)
            .collect();
        format!("{}... [input truncated]", head)
    } else {
        prompt.to_string()
    }
}

/// One-shot analysis. `ai_prompts` templates are already budget-aware; this
/// is the final safety check.
pub async fn one_shot(prompt: &str) -> Result<String, String> {
    crate::phi_silica::generate_response(&clamp_prompt(prompt)).await
}

/// Render a chat request as a single plain-text prompt (Phi Silica has no
/// message API). The chat layer is responsible for trimming the history to
/// the provider budget first; this only applies the hard safety clamp.
fn flatten_request(req: &ChatRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(system) = req.system.as_deref().filter(|s| !s.is_empty()) {
        parts.push(system.to_string());
    }
    for message in &req.messages {
        let speaker = match message.role {
            ChatRole::User => "User",
            ChatRole::Assistant => "Assistant",
            // System notes and tool results are folded in as plain context
            ChatRole::System | ChatRole::Tool => "Context",
        };
        if !message.content.is_empty() {
            parts.push(format!("{}: {}", speaker, message.content));
        }
    }
    parts.push("Assistant:".to_string());
    parts.join("\n\n")
}

/// Chat via a single flattened completion: one full-text delta, then done.
pub async fn chat_single_shot(
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let prompt = clamp_prompt(&flatten_request(req));
    let text = crate::phi_silica::generate_response(&prompt).await?;
    let _ = tx.send(text.clone()).await;
    Ok(ChatTurn {
        text,
        tool_calls: Vec::new(),
        finished: FinishReason::Stop,
    })
}

#[cfg(test)]
mod tests {
    use super::super::ChatMessage;
    use super::*;

    #[test]
    fn clamp_is_char_safe_with_multibyte_input() {
        let multibyte = "é".repeat(3_000);
        let clamped = clamp_prompt(&multibyte);
        assert!(clamped.chars().count() <= PHI_SILICA_MAX_PROMPT_CHARS);
        assert!(clamped.ends_with("[input truncated]"));
        // Short prompts pass through untouched
        assert_eq!(clamp_prompt("hello"), "hello");
    }

    #[test]
    fn flatten_renders_history_and_trailing_assistant_cue() {
        let req = ChatRequest {
            system: Some("Be brief.".into()),
            messages: vec![
                ChatMessage::user("Is my disk ok?"),
                ChatMessage::assistant("Checking."),
                ChatMessage::user("And memory?"),
            ],
            tools: Vec::new(),
            max_tokens: None,
        };
        let flat = flatten_request(&req);
        assert!(flat.starts_with("Be brief."));
        assert!(flat.contains("User: Is my disk ok?"));
        assert!(flat.contains("Assistant: Checking."));
        assert!(flat.ends_with("Assistant:"));
    }
}
