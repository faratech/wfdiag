//! Native Anthropic Messages API client (no Rust SDK exists; raw reqwest).
//!
//! API specifics encoded here:
//! - `max_tokens` is REQUIRED on every request.
//! - `temperature` is never sent (rejected by current Opus-tier models).
//! - `stop_reason: "refusal"` must be branched on BEFORE reading content —
//!   refused responses can have an empty content array.
//! - Tool calls round-trip as `tool_use` content blocks on assistant turns
//!   and `tool_result` blocks inside user-role messages.

use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ResolvedProviderConfig, ToolCall,
    ToolSpec, sse,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Default model when the `anthropicModel` setting is empty. Plain model id,
/// no date suffix. Cheaper option: claude-haiku-4-5; stronger: claude-opus-4-8.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-sonnet-4-6";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// max_tokens for one-shot analyses (responses are capped to ~150 words)
const ONE_SHOT_MAX_TOKENS: u32 = 1024;
/// max_tokens for chat turns
const CHAT_MAX_TOKENS: u32 = 4096;

/// Build a Messages API request body. Pure and network-free for testability.
pub(crate) fn build_messages_body(
    model: &str,
    system: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    max_tokens: u32,
    stream: bool,
) -> Value {
    let mut wire_messages: Vec<Value> = Vec::with_capacity(messages.len());
    for message in messages {
        match message.role {
            // The Messages API has no system role inside `messages`; loop
            // nudges and stray system notes travel as user text.
            ChatRole::User | ChatRole::System => {
                wire_messages.push(json!({"role": "user", "content": message.content}));
            }
            ChatRole::Assistant => {
                if message.tool_calls.is_empty() {
                    wire_messages.push(json!({"role": "assistant", "content": message.content}));
                } else {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !message.content.is_empty() {
                        blocks.push(json!({"type": "text", "text": message.content}));
                    }
                    for call in &message.tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": call.arguments,
                        }));
                    }
                    wire_messages.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            ChatRole::Tool => {
                wire_messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                        "content": message.content,
                    }],
                }));
            }
        }
    }

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": wire_messages,
    });
    if let Some(system) = system.filter(|s| !s.is_empty()) {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect(),
        );
    }
    if stream {
        body["stream"] = json!(true);
    }
    body
}

fn map_stop_reason(
    stop_reason: Option<&str>,
    has_tool_calls: bool,
) -> Result<FinishReason, String> {
    match stop_reason {
        Some("refusal") => Ok(FinishReason::Refusal),
        Some("tool_use") => Ok(FinishReason::ToolUse),
        Some("max_tokens" | "model_context_window_exceeded") => Ok(FinishReason::MaxTokens),
        Some("end_turn" | "stop_sequence") if has_tool_calls => Ok(FinishReason::ToolUse),
        Some("end_turn" | "stop_sequence") => Ok(FinishReason::Stop),
        Some("pause_turn") => Err(
            "Anthropic paused the turn before completion; this client does not use server tools"
                .to_string(),
        ),
        Some(other) => Err(format!("Anthropic returned unknown stop reason '{other}'")),
        None => Err("Anthropic response ended without a stop reason".to_string()),
    }
}

/// Parse a non-streaming Messages API response. Pure for testability.
pub(crate) fn parse_message_response(v: &Value) -> Result<ChatTurn, String> {
    if let Some(error) = v.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Anthropic API error: {}", message));
    }

    let stop_reason = v.get("stop_reason").and_then(Value::as_str);
    // Refusals are checked before content: the content array may be empty
    if stop_reason == Some("refusal") {
        return Ok(ChatTurn {
            text: String::new(),
            tool_calls: Vec::new(),
            finished: FinishReason::Refusal,
        });
    }

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    if let Some(blocks) = v.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.trim().is_empty())
                        .ok_or_else(|| {
                            "Anthropic returned a tool_use block without an id".to_string()
                        })?;
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .ok_or_else(|| {
                            "Anthropic returned a tool_use block without a name".to_string()
                        })?;
                    let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    if !arguments.is_object() {
                        return Err(format!(
                            "Anthropic returned non-object arguments for {name}"
                        ));
                    }
                    tool_calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }

    let finished = map_stop_reason(stop_reason, !tool_calls.is_empty())?;
    if text.trim().is_empty() && tool_calls.is_empty() && finished != FinishReason::Refusal {
        return Err("Anthropic completed without text or tool calls".to_string());
    }
    Ok(ChatTurn {
        text,
        tool_calls,
        finished,
    })
}

fn request_builder(cfg: &ResolvedProviderConfig) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", cfg.key())
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(120))
}

fn friendly_transport_error(e: reqwest::Error) -> String {
    format!("Anthropic request failed: {}", e)
}

async fn check_http_status(response: reqwest::Response) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or(body);
    let hint = match status.as_u16() {
        401 | 403 => " Check your Anthropic API key in Settings.",
        404 => " Check the configured Anthropic model name.",
        429 => " Rate limit exceeded — wait a moment and retry.",
        529 => " Anthropic is temporarily overloaded — retry shortly.",
        _ => "",
    };
    Err(format!(
        "Anthropic API error ({}): {}.{}",
        status, detail, hint
    ))
}

/// One-shot analysis (system + single user message).
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::Anthropic)?;
    let body = build_messages_body(
        model,
        Some(system),
        &[ChatMessage::user(prompt)],
        &[],
        ONE_SHOT_MAX_TOKENS,
        false,
    );
    let response = request_builder(cfg)
        .json(&body)
        .send()
        .await
        .map_err(friendly_transport_error)?;
    let response = check_http_status(response).await?;
    let v: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected Anthropic response: {}", e))?;
    let turn = parse_message_response(&v)?;
    match turn.finished {
        FinishReason::Stop if !turn.text.trim().is_empty() => Ok(turn.text),
        FinishReason::MaxTokens => {
            Err("Anthropic response was truncated before completion".to_string())
        }
        FinishReason::Refusal => Err("Anthropic declined to answer this request.".to_string()),
        _ => Err("Anthropic returned an unexpected one-shot response".to_string()),
    }
}

/// Streaming chat with optional tools. Text deltas go through `tx`; tool
/// calls are accumulated from `input_json_delta` fragments and returned
/// complete in the final turn.
pub async fn chat_stream(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::Anthropic)?;
    let body = build_messages_body(
        model,
        req.system.as_deref(),
        &req.messages,
        &req.tools,
        req.max_tokens.unwrap_or(CHAT_MAX_TOKENS),
        true,
    );
    let response = request_builder(cfg)
        .json(&body)
        .send()
        .await
        .map_err(friendly_transport_error)?;
    let response = check_http_status(response).await?;

    let mut text = String::new();
    let mut stop_reason: Option<String> = None;
    // index -> (id, name, accumulated input JSON)
    let mut open_tools: std::collections::BTreeMap<u64, (String, String, String)> =
        std::collections::BTreeMap::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut saw_message_stop = false;

    sse::for_each_event(response, |event, data| {
        match event {
            "content_block_start" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_start event: {error}")
                })?;
                if v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") {
                    let index = v
                        .get("index")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| "Anthropic tool_use block omitted its index".to_string())?;
                    let id = v
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.trim().is_empty())
                        .ok_or_else(|| "Anthropic tool_use block omitted its id".to_string())?
                        .to_string();
                    let name = v
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .ok_or_else(|| "Anthropic tool_use block omitted its name".to_string())?
                        .to_string();
                    open_tools.insert(index, (id, name, String::new()));
                }
            }
            "content_block_delta" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_delta event: {error}")
                })?;
                let index = v
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Anthropic content delta omitted its index".to_string())?;
                match v.pointer("/delta/type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                            text.push_str(t);
                            // try_send keeps SSE consumption non-blocking; the
                            // receiver buffers far more than one turn's text
                            let _ = tx.try_send(t.to_string());
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(fragment) =
                            v.pointer("/delta/partial_json").and_then(Value::as_str)
                            && let Some(entry) = open_tools.get_mut(&index)
                        {
                            entry.2.push_str(fragment);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_stop event: {error}")
                })?;
                let index = v
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Anthropic content block stop omitted its index".to_string())?;
                if let Some((id, name, input)) = open_tools.remove(&index) {
                    let arguments = if input.is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&input).map_err(|error| {
                            format!("Anthropic returned invalid JSON arguments for {name}: {error}")
                        })?
                    };
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
            }
            "message_delta" => {
                let v: Value = serde_json::from_str(data)
                    .map_err(|error| format!("Malformed Anthropic message_delta event: {error}"))?;
                let reason = v
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Anthropic message_delta omitted stop_reason".to_string())?;
                stop_reason = Some(reason.to_string());
            }
            "message_stop" => {
                saw_message_stop = true;
                return Ok(false);
            }
            "error" => {
                let v: Value = serde_json::from_str(data)
                    .map_err(|error| format!("Malformed Anthropic error event: {error}"))?;
                let message = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown stream error");
                return Err(format!("Anthropic stream error: {}", message));
            }
            // message_start, ping, anything new: ignore
            _ => {}
        }
        Ok(true)
    })
    .await?;

    // A stream that ends without ever sending message_stop was cut short by
    // something other than a normal completion (e.g. a proxy or timeout
    // closing the connection mid-turn) — surface that instead of returning a
    // truncated answer as if it finished normally.
    if !saw_message_stop {
        return Err(
            "Anthropic stream ended unexpectedly before completion (no message_stop received)"
                .to_string(),
        );
    }
    if !open_tools.is_empty() {
        return Err("Anthropic stream ended with an unfinished tool call".to_string());
    }

    let finished = map_stop_reason(stop_reason.as_deref(), !tool_calls.is_empty())?;
    if finished == FinishReason::Refusal {
        return Err("Anthropic declined to answer this request".to_string());
    }
    if text.trim().is_empty() && tool_calls.is_empty() && finished != FinishReason::Refusal {
        return Err("Anthropic completed without text or tool calls".to_string());
    }
    Ok(ChatTurn {
        text,
        tool_calls,
        finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "run_diagnostic".into(),
            description: "Run a diagnostic".into(),
            parameters: json!({"type": "object"}),
        }
    }

    #[test]
    fn body_has_required_fields_and_no_temperature() {
        let body = build_messages_body(
            ANTHROPIC_DEFAULT_MODEL,
            Some("be brief"),
            &[ChatMessage::user("hi")],
            &[tool()],
            1024,
            false,
        );
        assert_eq!(body["model"], ANTHROPIC_DEFAULT_MODEL);
        assert_eq!(body["max_tokens"], 1024); // required by the API
        assert_eq!(body["system"], "be brief"); // top-level param, not a message
        assert!(body.get("temperature").is_none()); // rejected by Opus-tier models
        assert!(body.get("stream").is_none());
        assert_eq!(body["tools"][0]["name"], "run_diagnostic");
        assert!(body["tools"][0]["input_schema"].is_object());
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn tool_round_trip_uses_content_blocks() {
        let messages = vec![
            ChatMessage::user("check disk"),
            ChatMessage::assistant_with_tools(
                "Let me check.",
                vec![ToolCall {
                    id: "toolu_1".into(),
                    name: "run_diagnostic".into(),
                    arguments: json!({"task_id": "logical_disk"}),
                }],
            ),
            ChatMessage::tool_result("toolu_1", "run_diagnostic", "C: 80% free"),
        ];
        let body = build_messages_body("m", None, &messages, &[], 100, true);
        assert_eq!(body["stream"], true);
        let assistant = &body["messages"][1];
        assert_eq!(assistant["content"][0]["type"], "text");
        assert_eq!(assistant["content"][1]["type"], "tool_use");
        assert_eq!(assistant["content"][1]["id"], "toolu_1");
        assert_eq!(assistant["content"][1]["input"]["task_id"], "logical_disk");
        // Tool results are user-role tool_result blocks
        let result = &body["messages"][2];
        assert_eq!(result["role"], "user");
        assert_eq!(result["content"][0]["type"], "tool_result");
        assert_eq!(result["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn parses_text_and_tool_use_turn() {
        let v = json!({
            "content": [
                {"type": "text", "text": "Checking now."},
                {"type": "tool_use", "id": "toolu_9", "name": "run_diagnostic",
                 "input": {"task_id": "os_info"}}
            ],
            "stop_reason": "tool_use"
        });
        let turn = parse_message_response(&v).unwrap();
        assert_eq!(turn.text, "Checking now.");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "run_diagnostic");
        assert_eq!(turn.tool_calls[0].arguments["task_id"], "os_info");
        assert_eq!(turn.finished, FinishReason::ToolUse);
    }

    #[test]
    fn refusal_with_empty_content_does_not_panic() {
        // stop_reason must be branched on before content is read
        let v = json!({"content": [], "stop_reason": "refusal"});
        let turn = parse_message_response(&v).unwrap();
        assert_eq!(turn.finished, FinishReason::Refusal);
        assert!(turn.text.is_empty());
    }

    #[test]
    fn api_error_payload_becomes_err() {
        let v = json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});
        assert!(
            parse_message_response(&v)
                .unwrap_err()
                .contains("Overloaded")
        );
    }

    #[test]
    fn stop_reasons_distinguish_truncation_pause_and_unknown_values() {
        assert_eq!(
            map_stop_reason(Some("model_context_window_exceeded"), false).unwrap(),
            FinishReason::MaxTokens
        );
        assert!(
            map_stop_reason(Some("pause_turn"), false)
                .unwrap_err()
                .contains("paused")
        );
        assert!(map_stop_reason(Some("future_reason"), false).is_err());
        assert!(map_stop_reason(None, false).is_err());
    }

    #[test]
    fn malformed_tool_blocks_are_rejected_instead_of_defaulted() {
        let missing_id = json!({
            "content": [{"type": "tool_use", "name": "run_diagnostic", "input": {}}],
            "stop_reason": "tool_use"
        });
        assert!(parse_message_response(&missing_id).is_err());

        let wrong_input = json!({
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "run_diagnostic",
                "input": "not an object"
            }],
            "stop_reason": "tool_use"
        });
        assert!(
            parse_message_response(&wrong_input)
                .unwrap_err()
                .contains("non-object")
        );
    }
}
