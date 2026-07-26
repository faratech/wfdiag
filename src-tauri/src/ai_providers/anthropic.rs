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
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ProviderReplay,
    ResolvedProviderConfig, ToolCall, ToolSpec, sse,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;
use tokio::sync::{RwLock, mpsc};

/// Default model when the `anthropicModel` setting is empty. Plain model id,
/// no date suffix. Cheaper option: claude-haiku-4-5; stronger: claude-opus-4-8.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-sonnet-4-6";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models/";

/// Thinking tokens count against `max_tokens`, so the old 1,024/4,096 caps
/// could consume the whole allowance before a visible answer was produced.
const ONE_SHOT_MAX_TOKENS: u32 = 8_192;
const CHAT_MAX_TOKENS: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModelRuntimeCaps {
    adaptive_thinking: bool,
    max_tokens: Option<u32>,
}

static MODEL_CAPS: OnceLock<RwLock<HashMap<String, ModelRuntimeCaps>>> = OnceLock::new();

fn model_caps_cache() -> &'static RwLock<HashMap<String, ModelRuntimeCaps>> {
    MODEL_CAPS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Conservative fallback for the generations Anthropic documents as
/// adaptive-capable. Unknown/future IDs remain off if metadata cannot be
/// fetched, avoiding a 400 from older or custom aliases.
fn fallback_adaptive_thinking(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    [
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-sonnet-4-6",
        "claude-sonnet-5",
        "claude-fable-5",
        "claude-mythos-5",
        "claude-mythos-preview",
    ]
    .iter()
    .any(|prefix| model == *prefix || model.starts_with(&format!("{prefix}-")))
}

fn parse_model_runtime_caps(v: &Value) -> Option<ModelRuntimeCaps> {
    let adaptive_thinking = v
        .pointer("/capabilities/thinking/types/adaptive/supported")
        .and_then(Value::as_bool)?;
    let max_tokens = v
        .get("max_tokens")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok());
    Some(ModelRuntimeCaps {
        adaptive_thinking,
        max_tokens,
    })
}

async fn fetch_model_runtime_caps(
    cfg: &ResolvedProviderConfig,
    model: &str,
) -> Option<ModelRuntimeCaps> {
    let mut url = reqwest::Url::parse(ANTHROPIC_MODELS_URL).ok()?;
    url.path_segments_mut().ok()?.push(model);
    let response = reqwest::Client::new()
        .get(url)
        .header("x-api-key", cfg.key())
        .header("anthropic-version", ANTHROPIC_VERSION)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: Value = response.json().await.ok()?;
    parse_model_runtime_caps(&value)
}

async fn resolve_model_runtime_caps(cfg: &ResolvedProviderConfig, model: &str) -> ModelRuntimeCaps {
    if let Some(cached) = model_caps_cache().read().await.get(model).copied() {
        return cached;
    }
    let caps = fetch_model_runtime_caps(cfg, model)
        .await
        .unwrap_or(ModelRuntimeCaps {
            adaptive_thinking: fallback_adaptive_thinking(model),
            max_tokens: None,
        });
    model_caps_cache()
        .write()
        .await
        .insert(model.to_string(), caps);
    caps
}

/// Build a Messages API request body. Pure and network-free for testability.
pub(crate) fn build_messages_body(
    model: &str,
    system: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    max_tokens: u32,
    stream: bool,
    adaptive_thinking: bool,
) -> Value {
    let mut wire_messages: Vec<Value> = Vec::with_capacity(messages.len());
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush_tool_results = |wire_messages: &mut Vec<Value>,
                              pending_tool_results: &mut Vec<Value>| {
        if !pending_tool_results.is_empty() {
            wire_messages.push(json!({
                "role": "user",
                "content": std::mem::take(pending_tool_results),
            }));
        }
    };

    for message in messages {
        if matches!(message.role, ChatRole::Tool) {
            let mut result = json!({
                "type": "tool_result",
                "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                "content": message.content,
            });
            if message.tool_result_is_error {
                result["is_error"] = json!(true);
            }
            pending_tool_results.push(result);
            continue;
        }
        flush_tool_results(&mut wire_messages, &mut pending_tool_results);

        match message.role {
            // The Messages API has no system role inside `messages`; loop
            // nudges and stray system notes travel as user text.
            ChatRole::User | ChatRole::System => {
                wire_messages.push(json!({"role": "user", "content": message.content}));
            }
            ChatRole::Assistant => {
                let replay = message
                    .provider_replay
                    .as_ref()
                    .and_then(|replay| match replay {
                        ProviderReplay::Anthropic {
                            requested_model,
                            content_blocks,
                        } if requested_model == model => Some(content_blocks.clone()),
                        _ => None,
                    });
                if let Some(content_blocks) = replay {
                    wire_messages.push(json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                } else if message.tool_calls.is_empty() {
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
            ChatRole::Tool => unreachable!("tool messages are collected before the role match"),
        }
    }
    flush_tool_results(&mut wire_messages, &mut pending_tool_results);

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
    if adaptive_thinking {
        body["thinking"] = json!({
            "type": "adaptive",
            "display": "omitted",
        });
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

fn turn_from_content(
    requested_model: &str,
    blocks: Vec<Value>,
    stop_reason: Option<&str>,
) -> Result<ChatTurn, String> {
    // Refusals are checked before content: the content array may be empty.
    if stop_reason == Some("refusal") {
        return Ok(ChatTurn {
            text: String::new(),
            tool_calls: Vec::new(),
            finished: FinishReason::Refusal,
            actual_models: Vec::new(),
            provider_replay: None,
        });
    }
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut tool_call_ids = BTreeSet::new();
    for block in &blocks {
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
                if !tool_call_ids.insert(id) {
                    return Err(format!("Anthropic returned duplicate tool call id '{id}'"));
                }
                tool_calls.push(ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments,
                });
            }
            // Thinking and redacted-thinking blocks remain only in the
            // provider replay. They are never copied into visible text.
            _ => {}
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
        actual_models: Vec::new(),
        provider_replay: (!requested_model.is_empty()).then(|| ProviderReplay::Anthropic {
            requested_model: requested_model.to_string(),
            content_blocks: blocks,
        }),
    })
}

fn parse_message_response_for_model(v: &Value, requested_model: &str) -> Result<ChatTurn, String> {
    if let Some(error) = v.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Anthropic API error: {}", message));
    }
    let blocks = v
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut turn = turn_from_content(
        requested_model,
        blocks,
        v.get("stop_reason").and_then(Value::as_str),
    )?;
    if let Some(model) = v
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())
    {
        turn.actual_models.push(model.to_string());
    }
    Ok(turn)
}

/// Parse a non-streaming Messages API response. Pure for testability.
#[cfg(test)]
pub(crate) fn parse_message_response(v: &Value) -> Result<ChatTurn, String> {
    let requested_model = v.get("model").and_then(Value::as_str).unwrap_or_default();
    parse_message_response_for_model(v, requested_model)
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
    let runtime_caps = resolve_model_runtime_caps(cfg, model).await;
    let max_tokens = runtime_caps
        .max_tokens
        .map_or(ONE_SHOT_MAX_TOKENS, |limit| ONE_SHOT_MAX_TOKENS.min(limit));
    let body = build_messages_body(
        model,
        Some(system),
        &[ChatMessage::user(prompt)],
        &[],
        max_tokens,
        false,
        runtime_caps.adaptive_thinking,
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
    let turn = parse_message_response_for_model(&v, model)?;
    match turn.finished {
        FinishReason::Stop if !turn.text.trim().is_empty() => Ok(turn.text),
        FinishReason::MaxTokens => {
            Err("Anthropic response was truncated before completion".to_string())
        }
        FinishReason::Refusal => Err("Anthropic declined to answer this request.".to_string()),
        _ => Err("Anthropic returned an unexpected one-shot response".to_string()),
    }
}

#[derive(Debug)]
struct OpenContentBlock {
    content: Value,
    input_json: String,
}

#[derive(Debug, Default)]
struct StreamAssembler {
    open_blocks: BTreeMap<u64, OpenContentBlock>,
    completed_blocks: BTreeMap<u64, Value>,
    actual_models: Vec<String>,
    stop_reason: Option<String>,
    saw_message_stop: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StreamAction {
    text_delta: Option<String>,
    stop: bool,
}

fn append_string_field(block: &mut Value, field: &str, fragment: &str) -> Result<(), String> {
    let object = block
        .as_object_mut()
        .ok_or_else(|| "Anthropic content block was not an object".to_string())?;
    let value = object
        .entry(field.to_string())
        .or_insert_with(|| Value::String(String::new()));
    let current = value
        .as_str()
        .ok_or_else(|| format!("Anthropic content block field '{field}' was not text"))?;
    *value = Value::String(format!("{current}{fragment}"));
    Ok(())
}

impl StreamAssembler {
    fn apply(&mut self, event: &str, data: &str) -> Result<StreamAction, String> {
        match event {
            "message_start" => {
                let v: Value = serde_json::from_str(data)
                    .map_err(|error| format!("Malformed Anthropic message_start event: {error}"))?;
                if let Some(model) = v
                    .pointer("/message/model")
                    .and_then(Value::as_str)
                    .filter(|model| !model.trim().is_empty())
                    && !self.actual_models.iter().any(|seen| seen == model)
                {
                    self.actual_models.push(model.to_string());
                }
                Ok(StreamAction::default())
            }
            "content_block_start" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_start event: {error}")
                })?;
                let index = v
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Anthropic content block omitted its index".to_string())?;
                let content = v
                    .get("content_block")
                    .cloned()
                    .filter(Value::is_object)
                    .ok_or_else(|| {
                        "Anthropic content_block_start omitted its content block".to_string()
                    })?;
                if self.open_blocks.contains_key(&index)
                    || self.completed_blocks.contains_key(&index)
                {
                    return Err(format!(
                        "Anthropic reused content block index {index} in one response"
                    ));
                }
                let text_delta = (content.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| content.get("text").and_then(Value::as_str))
                    .flatten()
                    .filter(|text| !text.is_empty())
                    .map(str::to_string);
                self.open_blocks.insert(
                    index,
                    OpenContentBlock {
                        content,
                        input_json: String::new(),
                    },
                );
                Ok(StreamAction {
                    text_delta,
                    stop: false,
                })
            }
            "content_block_delta" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_delta event: {error}")
                })?;
                let index = v
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Anthropic content delta omitted its index".to_string())?;
                let delta = v
                    .get("delta")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "Anthropic content delta omitted its payload".to_string())?;
                let open = self.open_blocks.get_mut(&index).ok_or_else(|| {
                    format!("Anthropic sent a delta for unopened content block {index}")
                })?;
                let mut text_delta = None;
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let fragment = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "Anthropic text_delta omitted its text".to_string())?;
                        append_string_field(&mut open.content, "text", fragment)?;
                        if !fragment.is_empty() {
                            text_delta = Some(fragment.to_string());
                        }
                    }
                    Some("thinking_delta") => {
                        let fragment =
                            delta
                                .get("thinking")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    "Anthropic thinking_delta omitted its thinking text".to_string()
                                })?;
                        append_string_field(&mut open.content, "thinking", fragment)?;
                    }
                    Some("signature_delta") => {
                        let fragment =
                            delta
                                .get("signature")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    "Anthropic signature_delta omitted its signature".to_string()
                                })?;
                        append_string_field(&mut open.content, "signature", fragment)?;
                    }
                    Some("input_json_delta") => {
                        let fragment = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                "Anthropic input_json_delta omitted partial_json".to_string()
                            })?;
                        open.input_json.push_str(fragment);
                    }
                    Some("citations_delta") | Some("citation_delta") => {
                        if let Some(citation) = delta.get("citation").cloned() {
                            let object = open.content.as_object_mut().ok_or_else(|| {
                                "Anthropic text content block was not an object".to_string()
                            })?;
                            object
                                .entry("citations")
                                .or_insert_with(|| Value::Array(Vec::new()))
                                .as_array_mut()
                                .ok_or_else(|| {
                                    "Anthropic text block citations was not an array".to_string()
                                })?
                                .push(citation);
                        }
                    }
                    // Preserve the original block from content_block_start.
                    // New delta kinds that do not alter replay-critical
                    // thinking/signature/tool fields are safely ignored.
                    _ => {}
                }
                Ok(StreamAction {
                    text_delta,
                    stop: false,
                })
            }
            "content_block_stop" => {
                let v: Value = serde_json::from_str(data).map_err(|error| {
                    format!("Malformed Anthropic content_block_stop event: {error}")
                })?;
                let index = v
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Anthropic content block stop omitted its index".to_string())?;
                let mut open = self
                    .open_blocks
                    .remove(&index)
                    .ok_or_else(|| format!("Anthropic stopped unopened content block {index}"))?;
                if !open.input_json.is_empty() {
                    let arguments: Value =
                        serde_json::from_str(&open.input_json).map_err(|error| {
                            format!("Anthropic returned invalid tool input JSON: {error}")
                        })?;
                    if !arguments.is_object() {
                        return Err(
                            "Anthropic returned non-object arguments for a tool call".to_string()
                        );
                    }
                    open.content["input"] = arguments;
                }
                self.completed_blocks.insert(index, open.content);
                Ok(StreamAction::default())
            }
            "message_delta" => {
                let v: Value = serde_json::from_str(data)
                    .map_err(|error| format!("Malformed Anthropic message_delta event: {error}"))?;
                if let Some(reason) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_string());
                }
                Ok(StreamAction::default())
            }
            "message_stop" => {
                self.saw_message_stop = true;
                Ok(StreamAction {
                    text_delta: None,
                    stop: true,
                })
            }
            "error" => {
                let v: Value = serde_json::from_str(data)
                    .map_err(|error| format!("Malformed Anthropic error event: {error}"))?;
                let message = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown stream error");
                Err(format!("Anthropic stream error: {message}"))
            }
            // Ping and future envelope events carry no content block state
            // needed for replay.
            _ => Ok(StreamAction::default()),
        }
    }

    fn finish(self, requested_model: &str) -> Result<ChatTurn, String> {
        if !self.saw_message_stop {
            return Err(
                "Anthropic stream ended unexpectedly before completion (no message_stop received)"
                    .to_string(),
            );
        }
        if !self.open_blocks.is_empty() {
            return Err("Anthropic stream ended with an unfinished content block".to_string());
        }
        let blocks = self.completed_blocks.into_values().collect();
        let mut turn = turn_from_content(requested_model, blocks, self.stop_reason.as_deref())?;
        turn.actual_models = self.actual_models;
        Ok(turn)
    }
}

/// Streaming chat with optional tools. Visible text deltas go through `tx`;
/// every content block is reconstructed in order for private replay.
pub async fn chat_stream(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::Anthropic)?;
    let runtime_caps = resolve_model_runtime_caps(cfg, model).await;
    let max_tokens = req
        .max_tokens
        .unwrap_or(CHAT_MAX_TOKENS)
        .min(runtime_caps.max_tokens.unwrap_or(u32::MAX));
    let body = build_messages_body(
        model,
        req.system.as_deref(),
        &req.messages,
        &req.tools,
        max_tokens,
        true,
        runtime_caps.adaptive_thinking,
    );
    let response = request_builder(cfg)
        .json(&body)
        .send()
        .await
        .map_err(friendly_transport_error)?;
    let response = check_http_status(response).await?;

    let mut assembler = StreamAssembler::default();

    sse::for_each_event(response, |event, data| {
        let action = assembler.apply(event, data)?;
        if let Some(delta) = action.text_delta {
            // try_send keeps SSE consumption non-blocking; the receiver
            // buffers far more than one turn's visible text.
            let _ = tx.try_send(delta);
        }
        Ok(!action.stop)
    })
    .await?;
    let turn = assembler.finish(model)?;
    if turn.finished == FinishReason::Refusal {
        return Err("Anthropic declined to answer this request".to_string());
    }
    Ok(turn)
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
    fn adaptive_thinking_is_explicitly_omitted_from_display() {
        let body = build_messages_body(
            "claude-opus-5",
            None,
            &[ChatMessage::user("diagnose this")],
            &[],
            CHAT_MAX_TOKENS,
            true,
            true,
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "omitted");
        assert_eq!(body["max_tokens"], CHAT_MAX_TOKENS);
        assert!(body.get("temperature").is_none());
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
        let body = build_messages_body("m", None, &messages, &[], 100, true, false);
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
    fn exact_replay_is_same_model_only_and_parallel_results_are_coalesced() {
        let blocks = vec![
            json!({"type": "thinking", "thinking": "", "signature": "signed-secret"}),
            json!({"type": "redacted_thinking", "data": "encrypted-secret"}),
            json!({"type": "tool_use", "id": "toolu_1", "name": "a", "input": {}}),
            json!({"type": "tool_use", "id": "toolu_2", "name": "b", "input": {}}),
        ];
        let assistant = ChatMessage::assistant_with_replay(
            "",
            vec![
                ToolCall {
                    id: "toolu_1".into(),
                    name: "a".into(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "toolu_2".into(),
                    name: "b".into(),
                    arguments: json!({}),
                },
            ],
            Some(ProviderReplay::Anthropic {
                requested_model: "claude-opus-5".into(),
                content_blocks: blocks.clone(),
            }),
        );
        let serialized = serde_json::to_string(&assistant).unwrap();
        assert!(!serialized.contains("signed-secret"));
        assert!(!serialized.contains("encrypted-secret"));
        assert!(!format!("{assistant:?}").contains("signed-secret"));

        let messages = vec![
            ChatMessage::user("go"),
            assistant.clone(),
            ChatMessage::tool_result("toolu_1", "a", "one"),
            ChatMessage::tool_error("toolu_2", "b", "two"),
        ];
        let same = build_messages_body("claude-opus-5", None, &messages, &[], 100, true, true);
        assert_eq!(same["messages"][1]["content"], Value::Array(blocks));
        assert_eq!(same["messages"][2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(same["messages"][2]["content"][1]["is_error"], true);

        let changed = build_messages_body("claude-opus-6", None, &messages, &[], 100, true, true);
        assert_eq!(changed["messages"][1]["content"][0]["type"], "tool_use");
        assert!(!changed.to_string().contains("signed-secret"));
    }

    #[test]
    fn parses_text_and_tool_use_turn() {
        let v = json!({
            "model": "claude-opus-5",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "sig"},
                {"type": "redacted_thinking", "data": "opaque"},
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
        let ProviderReplay::Anthropic {
            requested_model,
            content_blocks,
        } = turn.provider_replay.unwrap();
        assert_eq!(requested_model, "claude-opus-5");
        assert_eq!(content_blocks[0]["signature"], "sig");
        assert_eq!(content_blocks[1]["data"], "opaque");
    }

    #[test]
    fn stream_reconstructs_thinking_signatures_redactions_text_and_tools() {
        let mut stream = StreamAssembler::default();
        stream
            .apply(
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            )
            .unwrap();
        stream
            .apply(
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
            )
            .unwrap();
        stream
            .apply(
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"signature_delta","signature":"sig-1"}}"#,
            )
            .unwrap();
        stream
            .apply("content_block_stop", r#"{"index":0}"#)
            .unwrap();
        stream
            .apply(
                "content_block_start",
                r#"{"index":1,"content_block":{"type":"redacted_thinking","data":"opaque"}}"#,
            )
            .unwrap();
        stream
            .apply("content_block_stop", r#"{"index":1}"#)
            .unwrap();
        stream
            .apply(
                "content_block_start",
                r#"{"index":2,"content_block":{"type":"text","text":""}}"#,
            )
            .unwrap();
        let delta = stream
            .apply(
                "content_block_delta",
                r#"{"index":2,"delta":{"type":"text_delta","text":"Checking."}}"#,
            )
            .unwrap();
        assert_eq!(delta.text_delta.as_deref(), Some("Checking."));
        stream
            .apply("content_block_stop", r#"{"index":2}"#)
            .unwrap();
        stream
            .apply(
                "content_block_start",
                r#"{"index":3,"content_block":{"type":"tool_use","id":"toolu_1","name":"run_diagnostic","input":{}}}"#,
            )
            .unwrap();
        stream
            .apply(
                "content_block_delta",
                r#"{"index":3,"delta":{"type":"input_json_delta","partial_json":"{\"task_id\":"}}"#,
            )
            .unwrap();
        stream
            .apply(
                "content_block_delta",
                r#"{"index":3,"delta":{"type":"input_json_delta","partial_json":"\"os_info\"}"}}"#,
            )
            .unwrap();
        stream
            .apply("content_block_stop", r#"{"index":3}"#)
            .unwrap();
        stream
            .apply("message_delta", r#"{"delta":{"stop_reason":"tool_use"}}"#)
            .unwrap();
        assert!(stream.apply("message_stop", "{}").unwrap().stop);

        let turn = stream.finish("claude-opus-5").unwrap();
        assert_eq!(turn.text, "Checking.");
        assert_eq!(turn.tool_calls[0].arguments["task_id"], "os_info");
        let ProviderReplay::Anthropic { content_blocks, .. } = turn.provider_replay.unwrap();
        assert_eq!(content_blocks[0]["signature"], "sig-1");
        assert_eq!(content_blocks[1]["data"], "opaque");
        assert_eq!(content_blocks[2]["text"], "Checking.");
        assert_eq!(content_blocks[3]["input"]["task_id"], "os_info");
    }

    #[test]
    fn live_capabilities_control_adaptive_thinking_and_output_limit() {
        let caps = parse_model_runtime_caps(&json!({
            "capabilities": {
                "thinking": {"types": {"adaptive": {"supported": true}}}
            },
            "max_tokens": 12000
        }))
        .unwrap();
        assert!(caps.adaptive_thinking);
        assert_eq!(caps.max_tokens, Some(12_000));
        assert!(fallback_adaptive_thinking("claude-opus-5"));
        assert!(fallback_adaptive_thinking("claude-opus-5-20260701"));
        assert!(!fallback_adaptive_thinking("claude-haiku-4-5"));
        assert!(!fallback_adaptive_thinking("claude-unknown-future"));
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

        let duplicate_id = json!({
            "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "a", "input": {}},
                {"type": "tool_use", "id": "toolu_1", "name": "b", "input": {}}
            ],
            "stop_reason": "tool_use"
        });
        assert!(
            parse_message_response(&duplicate_id)
                .unwrap_err()
                .contains("duplicate")
        );
    }
}
