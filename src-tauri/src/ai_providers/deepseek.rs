//! Native DeepSeek V4 client.
//!
//! DeepSeek V4 enables thinking by default. Thinking-mode tool turns require
//! provider-specific `reasoning_content` to be replayed on every subsequent
//! request. The app's provider-neutral history cannot represent that field,
//! so this adapter explicitly selects documented non-thinking mode instead of
//! silently producing a 400 after the first tool result.

use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ResolvedProviderConfig, ToolCall,
    ToolSpec, sse,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

pub const DEEPSEEK_API_BASE: &str = "https://api.deepseek.com";
const DEEPSEEK_CHAT_URL: &str = "https://api.deepseek.com/chat/completions";

/// Current fast model. The legacy `deepseek-chat` alias is retired on
/// 2026-07-24 and must not be used as an application default.
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";

fn wire_messages(system: Option<&str>, messages: &[ChatMessage]) -> Vec<Value> {
    let mut wire = Vec::with_capacity(messages.len() + usize::from(system.is_some()));
    if let Some(system) = system.filter(|text| !text.is_empty()) {
        wire.push(json!({"role": "system", "content": system}));
    }
    for message in messages {
        match message.role {
            ChatRole::System => {
                wire.push(json!({"role": "system", "content": message.content}));
            }
            ChatRole::User => {
                wire.push(json!({"role": "user", "content": message.content}));
            }
            ChatRole::Assistant => {
                let mut value = json!({
                    "role": "assistant",
                    "content": if message.content.is_empty() {
                        Value::Null
                    } else {
                        Value::String(message.content.clone())
                    },
                });
                if !message.tool_calls.is_empty() {
                    value["tool_calls"] = Value::Array(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.name,
                                        "arguments": call.arguments.to_string(),
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                wire.push(value);
            }
            ChatRole::Tool => wire.push(json!({
                "role": "tool",
                "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                "content": message.content,
            })),
        }
    }
    wire
}

fn wire_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

pub(crate) fn build_chat_body(
    model: &str,
    system: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    max_tokens: Option<u32>,
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": wire_messages(system, messages),
        "stream": stream,
        // Explicit by design: see module documentation.
        "thinking": {"type": "disabled"},
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(wire_tools(tools));
    }
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    body
}

async fn send(cfg: &ResolvedProviderConfig, body: &Value) -> Result<reqwest::Response, String> {
    let response = reqwest::Client::new()
        .post(DEEPSEEK_CHAT_URL)
        .bearer_auth(cfg.key())
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(120))
        .json(body)
        .send()
        .await
        .map_err(|error| format!("DeepSeek request failed: {error}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let raw = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or(raw);
    let hint = match status.as_u16() {
        401 | 403 => " Check the DeepSeek API key in Settings.",
        404 => " Check the configured DeepSeek model name.",
        429 => " Rate limit exceeded — wait a moment and retry.",
        _ => "",
    };
    Err(format!("DeepSeek API error ({status}): {detail}.{hint}"))
}

fn finish_reason(reason: Option<&str>, has_tools: bool) -> Result<FinishReason, String> {
    match reason {
        Some("stop") if has_tools => Ok(FinishReason::ToolUse),
        Some("stop") => Ok(FinishReason::Stop),
        Some("tool_calls") => Ok(FinishReason::ToolUse),
        Some("length") => Ok(FinishReason::MaxTokens),
        Some("content_filter") => Ok(FinishReason::Refusal),
        Some("insufficient_system_resource") => {
            Err("DeepSeek could not complete because provider resources were unavailable".into())
        }
        Some(other) => Err(format!("DeepSeek returned unknown finish reason '{other}'")),
        None => Err("DeepSeek response ended without a finish reason".into()),
    }
}

fn parse_tool_call(value: &Value) -> Result<ToolCall, String> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "DeepSeek returned a tool call without an id".to_string())?;
    let name = value
        .pointer("/function/name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "DeepSeek returned a tool call without a name".to_string())?;
    let arguments = value
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("DeepSeek returned no arguments for {name}"))?;
    let arguments: Value = serde_json::from_str(arguments)
        .map_err(|error| format!("DeepSeek returned invalid JSON arguments for {name}: {error}"))?;
    if !arguments.is_object() {
        return Err(format!("DeepSeek returned non-object arguments for {name}"));
    }
    Ok(ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    })
}

pub(crate) fn parse_chat_response(value: &Value) -> Result<ChatTurn, String> {
    if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(format!("DeepSeek API error: {error}"));
    }
    let choice = value
        .pointer("/choices/0")
        .ok_or_else(|| "DeepSeek returned no completion choices".to_string())?;
    let text = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tool_calls: Vec<ToolCall> = choice
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().map(parse_tool_call).collect())
        .transpose()?
        .unwrap_or_default();
    let finished = finish_reason(
        choice.get("finish_reason").and_then(Value::as_str),
        !tool_calls.is_empty(),
    )?;
    if text.trim().is_empty() && tool_calls.is_empty() && finished != FinishReason::Refusal {
        return Err("DeepSeek completed without text or tool calls".into());
    }
    Ok(ChatTurn {
        text,
        tool_calls,
        finished,
        actual_models: value
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty())
            .map(|model| vec![model.to_string()])
            .unwrap_or_default(),
        provider_replay: None,
    })
}

pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::DeepSeek)?;
    let body = build_chat_body(
        model,
        Some(system),
        &[ChatMessage::user(prompt)],
        &[],
        None,
        false,
    );
    let response = send(cfg, &body).await?;
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Unexpected DeepSeek response: {error}"))?;
    let turn = parse_chat_response(&value)?;
    match turn.finished {
        FinishReason::Stop if !turn.text.trim().is_empty() => Ok(turn.text),
        FinishReason::MaxTokens => Err("DeepSeek response was truncated before completion".into()),
        FinishReason::Refusal => Err("DeepSeek declined to answer this request".into()),
        _ => Err("DeepSeek returned an unexpected one-shot response".into()),
    }
}

#[derive(Default)]
struct PendingCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn apply_stream_choice(
    choice: &Value,
    text: &mut String,
    pending: &mut BTreeMap<u64, PendingCall>,
    finish: &mut Option<String>,
) -> Result<Vec<String>, String> {
    let mut deltas = Vec::new();
    if let Some(delta) = choice.pointer("/delta/content").and_then(Value::as_str)
        && !delta.is_empty()
    {
        text.push_str(delta);
        deltas.push(delta.to_string());
    }
    if let Some(calls) = choice
        .pointer("/delta/tool_calls")
        .and_then(Value::as_array)
    {
        for call in calls {
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| "DeepSeek streamed a tool fragment without an index".to_string())?;
            let entry = pending.entry(index).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.id = Some(id.to_string());
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                entry.name.push_str(name);
            }
            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                entry.arguments.push_str(arguments);
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        *finish = Some(reason.to_string());
    }
    Ok(deltas)
}

fn finish_stream(
    text: String,
    pending: BTreeMap<u64, PendingCall>,
    finish: Option<String>,
    saw_done: bool,
    actual_models: Vec<String>,
) -> Result<ChatTurn, String> {
    if !saw_done {
        return Err("DeepSeek stream ended before data: [DONE]".into());
    }
    let mut calls = Vec::with_capacity(pending.len());
    for (_, call) in pending {
        let id = call
            .id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "DeepSeek streamed a tool call without an id".to_string())?;
        if call.name.trim().is_empty() {
            return Err("DeepSeek streamed a tool call without a name".into());
        }
        let arguments: Value = serde_json::from_str(&call.arguments).map_err(|error| {
            format!(
                "DeepSeek returned invalid JSON arguments for {}: {error}",
                call.name
            )
        })?;
        if !arguments.is_object() {
            return Err(format!(
                "DeepSeek returned non-object arguments for {}",
                call.name
            ));
        }
        calls.push(ToolCall {
            id,
            name: call.name,
            arguments,
        });
    }
    let finished = finish_reason(finish.as_deref(), !calls.is_empty())?;
    if finished == FinishReason::Refusal {
        return Err("DeepSeek blocked this response with content filtering".into());
    }
    if text.trim().is_empty() && calls.is_empty() && finished != FinishReason::Refusal {
        return Err("DeepSeek completed without text or tool calls".into());
    }
    Ok(ChatTurn {
        text,
        tool_calls: calls,
        finished,
        actual_models,
        provider_replay: None,
    })
}

pub async fn chat_stream(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::DeepSeek)?;
    let body = build_chat_body(
        model,
        req.system.as_deref(),
        &req.messages,
        &req.tools,
        req.max_tokens,
        true,
    );
    let response = send(cfg, &body).await?;
    let mut text = String::new();
    let mut pending = BTreeMap::new();
    let mut finish = None;
    let mut saw_done = false;
    let mut actual_models = Vec::new();
    sse::for_each_event(response, |_event, data| {
        if data == "[DONE]" {
            saw_done = true;
            return Ok(false);
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|error| format!("Malformed DeepSeek stream event: {error}"))?;
        if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
            return Err(format!("DeepSeek stream error: {error}"));
        }
        if let Some(model) = value
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty())
            && !actual_models.iter().any(|seen| seen == model)
        {
            actual_models.push(model.to_string());
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return Err("DeepSeek stream event omitted choices".into());
        };
        for choice in choices {
            for delta in apply_stream_choice(choice, &mut text, &mut pending, &mut finish)? {
                let _ = tx.try_send(delta);
            }
        }
        Ok(true)
    })
    .await?;
    finish_stream(text, pending, finish, saw_done, actual_models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_v4_with_thinking_explicitly_disabled() {
        let body = build_chat_body(
            DEEPSEEK_DEFAULT_MODEL,
            Some("be brief"),
            &[ChatMessage::user("hi")],
            &[],
            Some(500),
            true,
        );
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["max_tokens"], 500);
    }

    #[test]
    fn tool_calls_round_trip_without_reasoning_content() {
        let messages = vec![
            ChatMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "run_diagnostic".into(),
                    arguments: json!({"task_id": "os_info"}),
                }],
            ),
            ChatMessage::tool_result("call_1", "run_diagnostic", "Windows 11"),
        ];
        let body = build_chat_body("deepseek-v4-flash", None, &messages, &[], None, false);
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(body["messages"][1]["tool_call_id"], "call_1");
        assert!(body["messages"][0].get("reasoning_content").is_none());
    }

    #[test]
    fn parses_valid_tool_call_and_rejects_malformed_arguments() {
        let good = json!({
            "choices": [{
                "message": {"content": null, "tool_calls": [{
                    "id": "call_1",
                    "function": {"name": "run_diagnostic", "arguments": "{\"task_id\":\"os_info\"}"}
                }]},
                "finish_reason": "tool_calls"
            }]
        });
        let turn = parse_chat_response(&good).unwrap();
        assert_eq!(turn.finished, FinishReason::ToolUse);
        assert_eq!(turn.tool_calls[0].arguments["task_id"], "os_info");

        let mut bad = good;
        bad["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!("{bad");
        assert!(
            parse_chat_response(&bad)
                .unwrap_err()
                .contains("invalid JSON")
        );

        bad["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!("[]");
        assert!(
            parse_chat_response(&bad)
                .unwrap_err()
                .contains("non-object")
        );
    }

    #[test]
    fn rejects_missing_and_provider_resource_finish_reasons() {
        assert!(finish_reason(None, false).is_err());
        assert!(
            finish_reason(Some("insufficient_system_resource"), false)
                .unwrap_err()
                .contains("resources")
        );
    }
}
