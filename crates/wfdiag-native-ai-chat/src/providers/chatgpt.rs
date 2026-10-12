//! Direct `ChatGPT` plan inference. Always stateless Responses over HTTP/SSE.
use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ProviderReplay,
    ResolvedProviderConfig, ToolCall, read_text_capped, sse,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use wfdiag_native_chatgpt::{Failure, SessionBinding};

fn input_message(message: &ChatMessage) -> Vec<Value> {
    if let Some(ProviderReplay::ChatGPT { output_items, .. }) = &message.provider_replay {
        return output_items.clone();
    }
    match message.role {
        ChatRole::System => vec![json!({"role":"developer", "content": message.content})],
        ChatRole::User => vec![json!({"role":"user", "content": message.content})],
        ChatRole::Tool => vec![
            json!({"type":"function_call_output", "call_id": message.tool_call_id, "output": message.content}),
        ],
        ChatRole::Assistant => {
            let mut items = Vec::new();
            if !message.content.is_empty() {
                items.push(json!({"role":"assistant", "content": message.content}));
            }
            items.extend(message.tool_calls.iter().map(|call| json!({"type":"function_call", "namespace":"wfdiag", "name":call.name, "call_id":call.id, "arguments":call.arguments.to_string()})));
            items
        }
    }
}
fn request_body(model: &str, request: &ChatRequest) -> Value {
    let mut body = json!({"model":model, "store":false, "stream":true, "include":["reasoning.encrypted_content"],
        "input":request.messages.iter().flat_map(input_message).collect::<Vec<_>>()});
    if let Some(system) = &request.system {
        body["instructions"] = json!(system);
    }
    if !request.tools.is_empty() {
        body["tools"] = json!([{"type":"namespace", "name":"wfdiag", "description":"Windows diagnostics and requests for user review", "tools":request.tools.iter().map(|tool| json!({"type":"function", "name":tool.name, "description":tool.description, "parameters":super::openai_compat::strict_strictify(&tool.parameters), "strict":true})).collect::<Vec<_>>()}]);
    }
    body
}
fn completed_turn(
    response: &Value,
    model: &str,
    request: &ChatRequest,
) -> Result<ChatTurn, String> {
    if response.get("status").and_then(Value::as_str) != Some("completed") {
        return Err("ChatGPT inference did not complete".into());
    }
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or("ChatGPT output was missing")?;
    let mut text = String::new();
    let mut calls = Vec::new();
    let mut refusal = false;
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or("Invalid ChatGPT message")?
                {
                    match content.get("type").and_then(Value::as_str) {
                        Some("output_text") => text.push_str(
                            content
                                .get("text")
                                .and_then(Value::as_str)
                                .ok_or("Invalid ChatGPT output text")?,
                        ),
                        Some("refusal") => {
                            refusal = true;
                            text.push_str(
                                content
                                    .get("refusal")
                                    .and_then(Value::as_str)
                                    .unwrap_or("ChatGPT declined this request."),
                            );
                        }
                        _ => {}
                    }
                }
            }
            Some("function_call") => {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("Invalid ChatGPT function name")?;
                if item.get("namespace").and_then(Value::as_str) != Some("wfdiag")
                    || !request.tools.iter().any(|tool| tool.name == name)
                {
                    return Err(
                        "ChatGPT requested a tool outside the advertised diagnostics namespace"
                            .into(),
                    );
                }
                let arguments: Value = serde_json::from_str(
                    item.get("arguments")
                        .and_then(Value::as_str)
                        .ok_or("Invalid ChatGPT tool arguments")?,
                )
                .map_err(|_| "Invalid ChatGPT tool arguments")?;
                if !arguments.is_object() {
                    return Err("ChatGPT tool arguments must be an object".into());
                }
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or("ChatGPT tool call ID was missing")?;
                if calls.iter().any(|c: &ToolCall| c.id == id) {
                    return Err("ChatGPT returned duplicate tool call IDs".into());
                }
                calls.push(ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            Some("reasoning") => {}
            _ => return Err("ChatGPT returned unsupported output".into()),
        }
    }
    if refusal {
        calls.clear();
    }
    let finished = finish_reason(refusal, calls.is_empty());
    Ok(ChatTurn {
        text,
        tool_calls: calls,
        finished,
        actual_models: response
            .get("model")
            .and_then(Value::as_str)
            .map_or_else(Vec::new, |m| vec![m.into()]),
        provider_replay: Some(ProviderReplay::ChatGPT {
            session_fingerprint: String::new(),
            requested_model: model.into(),
            output_items: output.clone(),
        }),
    })
}
fn finish_reason(refusal: bool, calls_empty: bool) -> FinishReason {
    if refusal {
        FinishReason::Refusal
    } else if calls_empty {
        FinishReason::Stop
    } else {
        FinishReason::ToolUse
    }
}

async fn consume(
    response: reqwest::Response,
    model: &str,
    request: &ChatRequest,
    tx: mpsc::Sender<String>,
    request_id: Option<String>,
) -> Result<ChatTurn, Failure> {
    let mut complete = None;
    let mut failure = None;
    sse::for_each_event(response, |_, data| {
        if data == "[DONE]" {
            return Ok(false);
        }
        let event: Value =
            serde_json::from_str(data).map_err(|_| "ChatGPT returned an invalid stream event")?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or("Invalid ChatGPT text delta")?;
                tx.try_send(delta.into())
                    .map_err(|_| "ChatGPT streaming consumer stopped or exceeded its buffer")?;
            }
            Some("response.completed") => {
                complete = Some(completed_turn(&event["response"], model, request));
                return Ok(false);
            }
            Some("response.failed") => {
                failure = Some(Failure::from_response(
                    200,
                    &event["response"]["error"],
                    request_id.clone(),
                ));
                return Ok(false);
            }
            Some("response.incomplete") => {
                failure = Some(Failure::local(
                    "ChatGPT returned an incomplete response. Try again.",
                ));
                return Ok(false);
            }
            Some("error") => {
                failure = Some(Failure::from_response(200, &event, request_id.clone()));
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    })
    .await
    .map_err(|_| Failure::local("ChatGPT streaming was interrupted or invalid"))?;
    if let Some(failure) = failure {
        return Err(failure);
    }
    complete
        .ok_or_else(|| Failure::local("ChatGPT stream ended without completed inference"))?
        .map_err(|e| Failure::local(&e))
}
async fn call(
    session: &SessionBinding,
    request: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, Failure> {
    let fingerprint = session.lease.fingerprint();
    for message in &request.messages {
        if let Some(ProviderReplay::ChatGPT {
            session_fingerprint,
            ..
        }) = &message.provider_replay
            && session_fingerprint != &fingerprint
        {
            return Err(Failure::local(
                "ChatGPT account or model changed. Start a new conversation.",
            ));
        }
    }
    let token = session
        .service
        .access_token(&session.lease)
        .await
        .map_err(|e| Failure::local(&e))?;
    if session.lease.cancel.is_cancelled() {
        return Err(Failure::local(
            "ChatGPT account changed; start a new request",
        ));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Failure::network())?;
    let mut response = client
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(token.expose())
        .json(&request_body(&session.lease.model, request))
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|_| Failure::network())?;
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = read_text_capped(&mut response, sse::MAX_RESPONSE_BYTES)
            .await
            .map_err(|_| Failure::local("ChatGPT error response was invalid"))?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Err(Failure::from_response(status, &body, request_id));
    }
    let mut turn = consume(response, &session.lease.model, request, tx, request_id).await?;
    if let Some(ProviderReplay::ChatGPT {
        session_fingerprint,
        ..
    }) = &mut turn.provider_replay
    {
        *session_fingerprint = fingerprint;
    }
    Ok(turn)
}
pub async fn chat_stream(
    cfg: &ResolvedProviderConfig,
    request: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let session = cfg
        .chatgpt_session
        .as_ref()
        .ok_or("ChatGPT account session was not resolved")?;
    let result = tokio::select! {
        () = session.lease.cancel.cancelled() => return Err("ChatGPT account changed; start a new request".into()),
        result = call(session, request, tx) => result,
    };
    match result {
        Ok(turn) => {
            session.service.validate_lease(&session.lease).await?;
            Ok(turn)
        }
        Err(failure) => {
            session
                .service
                .record_failure(&session.lease, failure.clone())
                .await;
            Err(failure.to_string())
        }
    }
}
fn one_shot_text(turn: ChatTurn) -> Result<String, String> {
    match turn.finished {
        FinishReason::Stop if !turn.text.trim().is_empty() => Ok(turn.text),
        FinishReason::MaxTokens => {
            Err("ChatGPT response was truncated before completion".to_string())
        }
        FinishReason::Refusal => Err("ChatGPT declined to answer this request.".to_string()),
        _ => Err("ChatGPT returned an unexpected one-shot response".to_string()),
    }
}

pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let request = ChatRequest {
        system: Some(system.into()),
        messages: vec![ChatMessage::user(prompt)],
        ..ChatRequest::default()
    };
    let (tx, mut rx) = mpsc::channel(32);
    let stream = chat_stream(cfg, &request, tx);
    tokio::pin!(stream);
    loop {
        tokio::select! { result = &mut stream => return result.and_then(one_shot_text), delta = rx.recv() => { if delta.is_none() { return stream.await.and_then(one_shot_text); } } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolSpec;
    #[test]
    fn stateless_wire_omits_unsupported_parameters_and_namespaces_tools() {
        let request = ChatRequest {
            system: Some("policy".into()),
            messages: vec![ChatMessage {
                role: ChatRole::System,
                ..ChatMessage::user("loop policy")
            }],
            tools: vec![ToolSpec {
                name: "get_scan_summary".into(),
                description: "summary".into(),
                parameters: json!({"type":"object","properties":{}}),
            }],
            max_tokens: Some(100),
        };
        let body = request_body("account-model", &request);
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["tools"][0]["type"], "namespace");
        assert_eq!(body["input"][0]["role"], "developer");
        for field in [
            "max_output_tokens",
            "max_tokens",
            "temperature",
            "previous_response_id",
            "conversation",
        ] {
            assert!(body.get(field).is_none());
        }
    }
    #[test]
    fn completed_output_is_authoritative_and_replay_preserves_reasoning() {
        let response = json!({"status":"completed", "output":[{"type":"reasoning","encrypted_content":"secret reasoning"},{"type":"message","phase":"final_answer","content":[{"type":"output_text","text":"done"}]}]});
        let turn = completed_turn(&response, "m", &ChatRequest::default()).unwrap();
        assert_eq!(turn.text, "done");
        assert!(!format!("{:?}", turn.provider_replay).contains("secret reasoning"));
        let message =
            ChatMessage::assistant_with_replay(turn.text, turn.tool_calls, turn.provider_replay);
        assert_eq!(
            input_message(&message),
            response["output"].as_array().unwrap().clone()
        );
        assert!(
            completed_turn(
                &json!({"status":"incomplete", "output":[]}),
                "m",
                &ChatRequest::default()
            )
            .is_err()
        );
        assert!(completed_turn(&json!({"status":"completed", "output":[{"type":"function_call","namespace":"other","name":"get_scan_summary","call_id":"a","arguments":"{}"}]}), "m", &ChatRequest::default()).is_err());
    }
    #[test]
    fn one_shot_rejects_refusal_and_empty_stop() {
        let refusal = completed_turn(
            &json!({"status":"completed", "output":[{"type":"message","content":[{"type":"refusal","refusal":"I cannot help with that."}]}]}),
            "m",
            &ChatRequest::default(),
        )
        .unwrap();
        assert_eq!(refusal.finished, FinishReason::Refusal);
        assert_eq!(
            one_shot_text(refusal),
            Err("ChatGPT declined to answer this request.".to_string())
        );
        let empty = completed_turn(
            &json!({"status":"completed", "output":[]}),
            "m",
            &ChatRequest::default(),
        )
        .unwrap();
        assert_eq!(empty.finished, FinishReason::Stop);
        assert_eq!(
            one_shot_text(empty),
            Err("ChatGPT returned an unexpected one-shot response".to_string())
        );
        let ok = completed_turn(
            &json!({"status":"completed", "output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]}),
            "m",
            &ChatRequest::default(),
        )
        .unwrap();
        assert_eq!(one_shot_text(ok), Ok("done".to_string()));
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use std::fmt::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn fixture(events: &[Value], done: bool) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut body = String::new();
        for event in events {
            write!(body, "data: {event}\r\n\r\n").unwrap();
        }
        if done {
            body.push_str("data: [DONE]\r\n\r\n");
        }
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            // Fragment both JSON and UTF-8 across transport chunks.
            for chunk in body.as_bytes().chunks(7) {
                stream.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        reqwest::get(format!("http://{address}/fixture"))
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn fragmented_stream_waits_for_completed_and_keeps_private_output() {
        let response = fixture(&[
            json!({"type":"response.output_text.delta","delta":"café"}),
            json!({"type":"response.completed","response":{"status":"completed","model":"m","output":[{"type":"reasoning","encrypted_content":"private"},{"type":"message","phase":"final_answer","content":[{"type":"output_text","text":"café"}]}]}}),
        ], false).await;
        let (tx, mut rx) = mpsc::channel(8);
        let turn = consume(response, "m", &ChatRequest::default(), tx, None)
            .await
            .unwrap();
        assert_eq!(turn.text, "café");
        assert_eq!(rx.recv().await.unwrap(), "café");
        assert!(turn.provider_replay.is_some());
    }
    #[tokio::test]
    async fn quota_after_visible_text_and_partial_tool_never_returns_success() {
        let response = fixture(&[
            json!({"type":"response.output_text.delta","delta":"partial"}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","namespace":"wfdiag","name":"get_scan_summary","call_id":"call","arguments":"{}"}}),
            json!({"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"raw secret detail"}}}),
        ], false).await;
        let (tx, mut rx) = mpsc::channel(8);
        let failure = consume(
            response,
            "m",
            &ChatRequest::default(),
            tx,
            Some("req_fixture".into()),
        )
        .await
        .unwrap_err();
        assert_eq!(rx.recv().await.unwrap(), "partial");
        assert_eq!(failure.state, wfdiag_native_chatgpt::AccountState::Paused);
        assert_eq!(
            failure.code.as_deref(),
            Some("subscription_sharing_usage_limit_exceeded")
        );
        assert!(!failure.to_string().contains("raw secret"));
    }
    #[tokio::test]
    async fn eof_done_incomplete_and_error_are_terminal_failures() {
        for events in [
            vec![],
            vec![json!({"type":"response.incomplete"})],
            vec![json!({"type":"error","code":"subscription_sharing_user_unavailable"})],
        ] {
            let (tx, _) = mpsc::channel(8);
            assert!(
                consume(
                    fixture(&events, true).await,
                    "m",
                    &ChatRequest::default(),
                    tx,
                    None
                )
                .await
                .is_err()
            );
        }
    }
    #[test]
    fn tool_continuation_preserves_phase_reasoning_and_namespaced_call_outputs() {
        let request = ChatRequest {
            tools: vec![crate::ToolSpec {
                name: "get_scan_summary".into(),
                description: "summary".into(),
                parameters: json!({"type":"object","properties":{}}),
            }],
            ..ChatRequest::default()
        };
        let output = json!({"status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"message","phase":"commentary","content":[{"type":"output_text","text":"Checking"}]},{"type":"function_call","namespace":"wfdiag","name":"get_scan_summary","call_id":"call","arguments":"{}"}]});
        let turn = completed_turn(&output, "m", &request).unwrap();
        assert_eq!(turn.finished, FinishReason::ToolUse);
        let messages = vec![
            ChatMessage::assistant_with_replay(turn.text, turn.tool_calls, turn.provider_replay),
            ChatMessage {
                role: ChatRole::Tool,
                tool_call_id: Some("call".into()),
                ..ChatMessage::user("results")
            },
        ];
        let next = request_body(
            "m",
            &ChatRequest {
                messages,
                ..request
            },
        );
        assert_eq!(next["input"][0]["encrypted_content"], "opaque");
        assert_eq!(next["input"][1]["phase"], "commentary");
        assert_eq!(next["input"][2]["namespace"], "wfdiag");
        assert_eq!(next["input"][3]["call_id"], "call");
        assert_eq!(next["input"][3]["type"], "function_call_output");
    }
}
