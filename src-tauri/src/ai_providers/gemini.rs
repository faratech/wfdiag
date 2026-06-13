//! Native Google Gemini client (generateContent API).
//!
//! API specifics encoded here:
//! - Auth via the `x-goog-api-key` HEADER, never the `?key=` query param —
//!   keys must not appear in URLs (they end up in logs and error messages).
//! - The assistant role is `"model"`.
//! - Tool calls have NO ids; correlation is by function name. Ids are
//!   synthesized (`name#index`) for the unified layer and dropped on the wire.
//! - `functionResponse.response` must be a JSON OBJECT — plain text results
//!   are wrapped as `{"result": <text>}`.

use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ResolvedProviderConfig, ToolCall,
    ToolSpec, sse,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Default model when the `geminiModel` setting is empty.
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-2.5-flash";
const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Build a generateContent request body. Pure and network-free for testability.
pub(crate) fn build_generate_body(
    system: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    max_tokens: Option<u32>,
) -> Value {
    let mut contents: Vec<Value> = Vec::with_capacity(messages.len());
    for message in messages {
        match message.role {
            // No system role in contents; stray system notes travel as user text
            ChatRole::User | ChatRole::System => {
                contents.push(json!({"role": "user", "parts": [{"text": message.content}]}));
            }
            ChatRole::Assistant => {
                let mut parts: Vec<Value> = Vec::new();
                if !message.content.is_empty() {
                    parts.push(json!({"text": message.content}));
                }
                for call in &message.tool_calls {
                    parts.push(json!({
                        "functionCall": {"name": call.name, "args": call.arguments}
                    }));
                }
                contents.push(json!({"role": "model", "parts": parts}));
            }
            ChatRole::Tool => {
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": message.tool_name.clone().unwrap_or_default(),
                            // Must be an object, not a bare string
                            "response": {"result": message.content},
                        }
                    }],
                }));
            }
        }
    }

    let mut body = json!({"contents": contents});
    if let Some(system) = system.filter(|s| !s.is_empty()) {
        body["systemInstruction"] = json!({"parts": [{"text": system}]});
    }
    if !tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect::<Vec<_>>()
        }]);
    }
    if let Some(max_tokens) = max_tokens {
        body["generationConfig"] = json!({"maxOutputTokens": max_tokens});
    }
    body
}

fn map_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match reason {
        Some("MAX_TOKENS") => FinishReason::MaxTokens,
        Some("SAFETY") | Some("PROHIBITED_CONTENT") | Some("BLOCKLIST") => FinishReason::Refusal,
        _ if has_tool_calls => FinishReason::ToolUse,
        _ => FinishReason::Stop,
    }
}

/// Parse a generateContent response. Pure for testability.
pub(crate) fn parse_generate_response(v: &Value) -> Result<ChatTurn, String> {
    if let Some(error) = v.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Gemini API error: {}", message));
    }
    if let Some(reason) = v
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Err(format!("Gemini blocked the request ({})", reason));
    }

    let Some(candidate) = v.pointer("/candidates/0") else {
        return Err("Gemini returned no candidates (the response may have been blocked)".into());
    };

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    if let Some(parts) = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                text.push_str(t);
            }
            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                tool_calls.push(ToolCall {
                    // Gemini has no call ids — synthesize one for correlation
                    id: format!("{}#{}", name, tool_calls.len()),
                    name,
                    arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                });
            }
        }
    }

    let finish = candidate.get("finishReason").and_then(Value::as_str);
    Ok(ChatTurn {
        text,
        tool_calls: tool_calls.clone(),
        finished: map_finish_reason(finish, !tool_calls.is_empty()),
    })
}

fn endpoint_url(model: &str, stream: bool) -> String {
    if stream {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            GEMINI_API_BASE, model
        )
    } else {
        format!("{}/models/{}:generateContent", GEMINI_API_BASE, model)
    }
}

async fn send_request(
    cfg: &ResolvedProviderConfig,
    url: &str,
    body: &Value,
) -> Result<reqwest::Response, String> {
    let response = reqwest::Client::new()
        .post(url)
        .header("x-goog-api-key", cfg.key())
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(120))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Gemini request failed: {}", e))?;

    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body_text = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body_text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or(body_text);
    let hint = match status.as_u16() {
        400 if detail.contains("API key") => " Check your Gemini API key in Settings.",
        401 | 403 => " Check your Gemini API key in Settings.",
        404 => " Check the configured Gemini model name.",
        429 => " Rate limit exceeded — wait a moment and retry.",
        _ => "",
    };
    Err(format!(
        "Gemini API error ({}): {}.{}",
        status, detail, hint
    ))
}

/// One-shot analysis (system instruction + single user message).
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::Gemini)?;
    let body = build_generate_body(Some(system), &[ChatMessage::user(prompt)], &[], None);
    let response = send_request(cfg, &endpoint_url(model, false), &body).await?;
    let v: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected Gemini response: {}", e))?;
    let turn = parse_generate_response(&v)?;
    if turn.finished == FinishReason::Refusal {
        return Err("Gemini declined to answer this request.".to_string());
    }
    Ok(turn.text)
}

/// Streaming chat with optional tools. Each SSE `data:` line is a complete
/// GenerateContentResponse chunk with incremental parts.
pub async fn chat_stream(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let model = cfg.model_or_err(crate::ai_service::AIProvider::Gemini)?;
    let body = build_generate_body(
        req.system.as_deref(),
        &req.messages,
        &req.tools,
        req.max_tokens,
    );
    let response = send_request(cfg, &endpoint_url(model, true), &body).await?;

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut finish: Option<String> = None;

    sse::for_each_event(response, |_event, data| {
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(true), // tolerate keep-alive noise
        };
        if let Some(error) = v.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown stream error");
            return Err(format!("Gemini stream error: {}", message));
        }
        if let Some(parts) = v
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                    let _ = tx.try_send(t.to_string());
                }
                if let Some(call) = part.get("functionCall") {
                    let name = call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    tool_calls.push(ToolCall {
                        id: format!("{}#{}", name, tool_calls.len()),
                        name,
                        arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    });
                }
            }
        }
        if let Some(reason) = v
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            finish = Some(reason.to_string());
        }
        Ok(true)
    })
    .await?;

    let finished = map_finish_reason(finish.as_deref(), !tool_calls.is_empty());
    Ok(ChatTurn {
        text,
        tool_calls,
        finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_maps_roles_and_system_instruction() {
        let messages = vec![
            ChatMessage::user("check disk"),
            ChatMessage::assistant("On it."),
        ];
        let body = build_generate_body(Some("be brief"), &messages, &[], Some(2048));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
        assert_eq!(body["contents"][0]["role"], "user");
        // Gemini's assistant role is "model"
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 2048);
    }

    #[test]
    fn tools_nest_under_function_declarations() {
        let tool = ToolSpec {
            name: "run_diagnostic".into(),
            description: "Run a diagnostic".into(),
            parameters: json!({"type": "object"}),
        };
        let body = build_generate_body(None, &[ChatMessage::user("hi")], &[tool], None);
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "run_diagnostic"
        );
    }

    #[test]
    fn function_response_is_wrapped_as_object() {
        // functionResponse.response must be a JSON object, never a bare string
        let messages = vec![
            ChatMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "run_diagnostic#0".into(),
                    name: "run_diagnostic".into(),
                    arguments: json!({"task_id": "os_info"}),
                }],
            ),
            ChatMessage::tool_result("run_diagnostic#0", "run_diagnostic", "Windows 11 24H2"),
        ];
        let body = build_generate_body(None, &messages, &[], None);
        let call_part = &body["contents"][0]["parts"][0]["functionCall"];
        assert_eq!(call_part["name"], "run_diagnostic");
        // The synthesized id never reaches the wire
        assert_eq!(body["contents"][0]["parts"][0].get("id"), None);
        let response_part = &body["contents"][1]["parts"][0]["functionResponse"];
        assert_eq!(response_part["name"], "run_diagnostic");
        assert!(response_part["response"].is_object());
        assert_eq!(response_part["response"]["result"], "Windows 11 24H2");
    }

    #[test]
    fn parses_text_and_synthesizes_tool_call_ids() {
        let v = json!({
            "candidates": [{
                "content": {"parts": [
                    {"text": "Checking."},
                    {"functionCall": {"name": "run_diagnostic", "args": {"task_id": "os_info"}}}
                ], "role": "model"},
                "finishReason": "STOP"
            }]
        });
        let turn = parse_generate_response(&v).unwrap();
        assert_eq!(turn.text, "Checking.");
        assert_eq!(turn.tool_calls[0].id, "run_diagnostic#0");
        assert_eq!(turn.tool_calls[0].arguments["task_id"], "os_info");
        // Tool calls present → ToolUse even though Gemini said STOP
        assert_eq!(turn.finished, FinishReason::ToolUse);
    }

    #[test]
    fn empty_candidates_and_blocks_become_readable_errors() {
        assert!(
            parse_generate_response(&json!({"candidates": []}))
                .unwrap_err()
                .contains("no candidates")
        );
        assert!(
            parse_generate_response(&json!({"promptFeedback": {"blockReason": "SAFETY"}}))
                .unwrap_err()
                .contains("SAFETY")
        );
        assert!(
            parse_generate_response(&json!({"error": {"message": "API key not valid"}}))
                .unwrap_err()
                .contains("API key not valid")
        );
    }
}
