//! Native Google Gemini client (generateContent API).
//!
//! API specifics encoded here:
//! - Auth via the `x-goog-api-key` HEADER, never the `?key=` query param —
//!   keys must not appear in URLs (they end up in logs and error messages).
//! - The assistant role is `"model"`.
//! - Gemini 3 tool calls have ids and thought signatures. Both are preserved
//!   across the app's provider-neutral tool loop and returned exactly.
//! - `functionResponse.response` must be a JSON OBJECT — plain text results
//!   are wrapped as `{"result": <text>}`.

use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ResolvedProviderConfig, ToolCall,
    ToolSpec, sse,
};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc;

/// Default model when the `geminiModel` setting is empty.
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3.5-flash";
pub(crate) const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

const MAX_CALL_METADATA: usize = 2_048;
const LEGACY_CALL_PREFIX: &str = "wfdiag-gemini-legacy-";

#[derive(Debug, Clone)]
struct GeminiCallMetadata {
    wire_id: Option<String>,
    thought_signature: Option<String>,
}

#[derive(Default)]
struct MetadataCache {
    entries: HashMap<String, GeminiCallMetadata>,
    order: VecDeque<String>,
}

fn metadata_cache() -> &'static Mutex<MetadataCache> {
    static CACHE: OnceLock<Mutex<MetadataCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MetadataCache::default()))
}

fn remember_call(wire_id: Option<&str>, thought_signature: Option<&str>, name: &str) -> String {
    static NEXT_LEGACY_ID: AtomicU64 = AtomicU64::new(1);
    let public_id = wire_id
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "{LEGACY_CALL_PREFIX}{}-{name}",
                NEXT_LEGACY_ID.fetch_add(1, Ordering::Relaxed)
            )
        });
    if let Ok(mut cache) = metadata_cache().lock() {
        if !cache.entries.contains_key(&public_id) {
            cache.order.push_back(public_id.clone());
        }
        cache.entries.insert(
            public_id.clone(),
            GeminiCallMetadata {
                wire_id: wire_id.map(str::to_string),
                thought_signature: thought_signature.map(str::to_string),
            },
        );
        while cache.entries.len() > MAX_CALL_METADATA {
            if let Some(oldest) = cache.order.pop_front() {
                cache.entries.remove(&oldest);
            }
        }
    }
    public_id
}

fn call_metadata(public_id: &str) -> Option<GeminiCallMetadata> {
    metadata_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.entries.get(public_id).cloned())
}

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
                    let metadata = call_metadata(&call.id);
                    let mut function_call = json!({
                        "name": call.name,
                        "args": call.arguments,
                    });
                    if let Some(wire_id) = metadata
                        .as_ref()
                        .and_then(|metadata| metadata.wire_id.as_deref())
                    {
                        function_call["id"] = json!(wire_id);
                    }
                    let mut part = json!({"functionCall": function_call});
                    if let Some(signature) = metadata
                        .as_ref()
                        .and_then(|metadata| metadata.thought_signature.as_deref())
                    {
                        part["thoughtSignature"] = json!(signature);
                    }
                    parts.push(part);
                }
                contents.push(json!({"role": "model", "parts": parts}));
            }
            ChatRole::Tool => {
                let mut function_response = json!({
                    "name": message.tool_name.clone().unwrap_or_default(),
                    // Must be an object, not a bare string
                    "response": {"result": message.content},
                });
                if let Some(wire_id) = message
                    .tool_call_id
                    .as_deref()
                    .and_then(call_metadata)
                    .and_then(|metadata| metadata.wire_id)
                {
                    function_response["id"] = json!(wire_id);
                }
                let response_part = json!({"functionResponse": function_response});
                // Gemini requires every function response from one round of
                // parallel tool calls to be a part of a SINGLE content entry
                // — sending each as its own "user" turn causes a 400. Append
                // to the previous entry when it's already a tool-result
                // batch instead of starting a new turn per result.
                let appended = contents.last_mut().is_some_and(|last| {
                    let is_tool_batch = last.get("role").and_then(Value::as_str) == Some("user")
                        && last["parts"].as_array().is_some_and(|parts| {
                            !parts.is_empty()
                                && parts.iter().all(|p| p.get("functionResponse").is_some())
                        });
                    if is_tool_batch {
                        last["parts"]
                            .as_array_mut()
                            .expect("checked above")
                            .push(response_part.clone());
                    }
                    is_tool_batch
                });
                if !appended {
                    contents.push(json!({"role": "user", "parts": [response_part]}));
                }
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

fn map_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> Result<FinishReason, String> {
    match reason {
        Some("STOP") if has_tool_calls => Ok(FinishReason::ToolUse),
        Some("STOP") => Ok(FinishReason::Stop),
        Some("MAX_TOKENS") => Ok(FinishReason::MaxTokens),
        // All of these mean the model did NOT produce a normal completion —
        // treating them as Stop would make a blocked/malformed generation
        // look like a successful (if empty) answer.
        Some("SAFETY")
        | Some("PROHIBITED_CONTENT")
        | Some("BLOCKLIST")
        | Some("RECITATION")
        | Some("SPII")
        | Some("OTHER")
        | Some("MALFORMED_FUNCTION_CALL")
        | Some("LANGUAGE")
        | Some("IMAGE_SAFETY")
        | Some("IMAGE_PROHIBITED_CONTENT")
        | Some("IMAGE_OTHER")
        | Some("NO_IMAGE")
        | Some("IMAGE_RECITATION")
        | Some("UNEXPECTED_TOOL_CALL")
        | Some("TOO_MANY_TOOL_CALLS")
        | Some("MISSING_THOUGHT_SIGNATURE")
        | Some("MALFORMED_RESPONSE") => Ok(FinishReason::Refusal),
        Some(other) => Err(format!("Gemini returned unknown finish reason '{other}'")),
        None => Err("Gemini response ended without a finish reason".to_string()),
    }
}

fn collect_parts(
    parts: &[Value],
    text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) -> Result<Vec<String>, String> {
    let mut deltas = Vec::new();
    for part in parts {
        if let Some(delta) = part.get("text").and_then(Value::as_str)
            && !delta.is_empty()
        {
            text.push_str(delta);
            deltas.push(delta.to_string());
        }
        let Some(call) = part.get("functionCall") else {
            continue;
        };
        let name = call
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "Gemini returned a function call without a name".to_string())?;
        let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return Err(format!("Gemini returned non-object arguments for {name}"));
        }
        let id = remember_call(
            call.get("id").and_then(Value::as_str),
            part.get("thoughtSignature").and_then(Value::as_str),
            name,
        );
        tool_calls.push(ToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
    }
    Ok(deltas)
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
        collect_parts(parts, &mut text, &mut tool_calls)?;
    }

    let finish = candidate.get("finishReason").and_then(Value::as_str);
    Ok(ChatTurn {
        text,
        tool_calls: tool_calls.clone(),
        finished: map_finish_reason(finish, !tool_calls.is_empty())?,
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
    match turn.finished {
        FinishReason::Stop if !turn.text.trim().is_empty() => Ok(turn.text),
        FinishReason::MaxTokens => {
            Err("Gemini response was truncated before completion".to_string())
        }
        FinishReason::Refusal => Err("Gemini declined to answer this request.".to_string()),
        _ => Err("Gemini returned an unexpected one-shot response".to_string()),
    }
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
        let v: Value = serde_json::from_str(data)
            .map_err(|error| format!("Malformed Gemini stream event: {error}"))?;
        if let Some(error) = v.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown stream error");
            return Err(format!("Gemini stream error: {}", message));
        }
        let candidate = v
            .pointer("/candidates/0")
            .ok_or_else(|| "Gemini stream event omitted its candidate".to_string())?;
        if let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        {
            for delta in collect_parts(parts, &mut text, &mut tool_calls)? {
                let _ = tx.try_send(delta);
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

    let finished = map_finish_reason(finish.as_deref(), !tool_calls.is_empty())?;
    if finished == FinishReason::Refusal {
        return Err("Gemini blocked or could not complete this response".to_string());
    }
    if text.trim().is_empty() && tool_calls.is_empty() && finished != FinishReason::Refusal {
        return Err("Gemini completed without text or tool calls".to_string());
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

    #[test]
    fn default_tracks_the_current_flash_generation() {
        assert_eq!(GEMINI_DEFAULT_MODEL, "gemini-3.5-flash");
    }

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
    fn parallel_tool_results_batch_into_one_turn() {
        // Gemini requires all function responses from one round of parallel
        // tool calls to live in a single content entry, not one turn each.
        let messages = vec![
            ChatMessage::assistant_with_tools(
                "",
                vec![
                    ToolCall {
                        id: "get_scan_summary#0".into(),
                        name: "get_scan_summary".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "get_detected_issues#1".into(),
                        name: "get_detected_issues".into(),
                        arguments: json!({}),
                    },
                ],
            ),
            ChatMessage::tool_result("get_scan_summary#0", "get_scan_summary", "3 passed"),
            ChatMessage::tool_result("get_detected_issues#1", "get_detected_issues", "none"),
        ];
        let body = build_generate_body(None, &messages, &[], None);
        // Exactly two contents: the model's tool-call turn, then ONE user
        // turn carrying both function responses as separate parts.
        assert_eq!(body["contents"].as_array().unwrap().len(), 2);
        let parts = body["contents"][1]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "get_scan_summary");
        assert_eq!(parts[1]["functionResponse"]["name"], "get_detected_issues");
    }

    #[test]
    fn parses_text_and_synthesizes_legacy_tool_call_ids() {
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
        assert!(turn.tool_calls[0].id.starts_with(LEGACY_CALL_PREFIX));
        assert_eq!(turn.tool_calls[0].arguments["task_id"], "os_info");
        // Tool calls present → ToolUse even though Gemini said STOP
        assert_eq!(turn.finished, FinishReason::ToolUse);
    }

    #[test]
    fn blocked_and_malformed_finish_reasons_are_not_reported_as_stop() {
        for reason in [
            "SAFETY",
            "PROHIBITED_CONTENT",
            "BLOCKLIST",
            "RECITATION",
            "SPII",
            "OTHER",
            "MALFORMED_FUNCTION_CALL",
        ] {
            assert_eq!(
                map_finish_reason(Some(reason), false).unwrap(),
                FinishReason::Refusal,
                "{reason} should not map to Stop"
            );
        }
        for reason in [
            "LANGUAGE",
            "IMAGE_SAFETY",
            "UNEXPECTED_TOOL_CALL",
            "TOO_MANY_TOOL_CALLS",
            "MISSING_THOUGHT_SIGNATURE",
            "MALFORMED_RESPONSE",
        ] {
            assert_eq!(
                map_finish_reason(Some(reason), false).unwrap(),
                FinishReason::Refusal
            );
        }
        assert_eq!(
            map_finish_reason(Some("STOP"), false).unwrap(),
            FinishReason::Stop
        );
        assert!(map_finish_reason(None, false).is_err());
        assert!(map_finish_reason(Some("NEW_REASON"), false).is_err());
    }

    #[test]
    fn gemini_three_id_and_thought_signature_round_trip_exactly() {
        let response = json!({
            "candidates": [{
                "content": {"parts": [{
                    "functionCall": {
                        "id": "call_abc123",
                        "name": "run_diagnostic",
                        "args": {"task_id": "os_info"}
                    },
                    "thoughtSignature": "encrypted-signature"
                }]},
                "finishReason": "STOP"
            }]
        });
        let turn = parse_generate_response(&response).unwrap();
        assert_eq!(turn.tool_calls[0].id, "call_abc123");

        let messages = vec![
            ChatMessage::assistant_with_tools("", turn.tool_calls),
            ChatMessage::tool_result("call_abc123", "run_diagnostic", "Windows 11 24H2"),
        ];
        let body = build_generate_body(None, &messages, &[], None);
        let call = &body["contents"][0]["parts"][0];
        assert_eq!(call["functionCall"]["id"], "call_abc123");
        assert_eq!(call["thoughtSignature"], "encrypted-signature");
        assert_eq!(
            body["contents"][1]["parts"][0]["functionResponse"]["id"],
            "call_abc123"
        );
    }

    #[test]
    fn rejects_non_object_function_arguments() {
        let response = json!({
            "candidates": [{
                "content": {"parts": [{
                    "functionCall": {"id": "call_1", "name": "bad", "args": "not an object"}
                }]},
                "finishReason": "STOP"
            }]
        });
        assert!(
            parse_generate_response(&response)
                .unwrap_err()
                .contains("non-object")
        );
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
