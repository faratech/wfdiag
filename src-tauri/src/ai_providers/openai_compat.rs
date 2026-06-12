//! Shared `/v1/chat/completions` client for every OpenAI-compatible target:
//! cloud OpenAI chat, the user-configured custom endpoint (OpenRouter, Groq,
//! Gemini's compat layer, …), Ollama, and Foundry Local chat.
//!
//! Generic providers do NOT serve `/v1/responses`, which is why chat goes
//! through chat completions while the OpenAI/Foundry one-shot paths keep the
//! Responses API. No token cap is sent: OpenAI's current models reject the
//! legacy `max_tokens` while several compat servers don't know
//! `max_completion_tokens` — prompts are budgeted on our side instead.

use super::{
    ChatMessage, ChatRequest, ChatRole, ChatTurn, FinishReason, ResolvedProviderConfig, ToolCall,
    ToolSpec,
};
use crate::ai_service::AIProvider;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
        ChatCompletionRequestToolMessageContent, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent, ChatCompletionTool, ChatCompletionTools,
        CreateChatCompletionRequestArgs, FinishReason as OpenAIFinishReason, FunctionCall,
        FunctionObject,
    },
};
use futures::StreamExt;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

/// Map provider-neutral messages onto chat-completions request messages.
pub(crate) fn to_openai_messages(
    system: Option<&str>,
    messages: &[ChatMessage],
) -> Vec<ChatCompletionRequestMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    if let Some(system) = system.filter(|s| !s.is_empty()) {
        out.push(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system.to_string()),
                name: None,
            },
        ));
    }
    for message in messages {
        match message.role {
            ChatRole::System => out.push(ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text(
                        message.content.clone(),
                    ),
                    name: None,
                },
            )),
            ChatRole::User => out.push(ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(message.content.clone()),
                    name: None,
                },
            )),
            ChatRole::Assistant => {
                let tool_calls = if message.tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                ChatCompletionMessageToolCalls::Function(
                                    ChatCompletionMessageToolCall {
                                        id: call.id.clone(),
                                        function: FunctionCall {
                                            name: call.name.clone(),
                                            arguments: call.arguments.to_string(),
                                        },
                                    },
                                )
                            })
                            .collect(),
                    )
                };
                out.push(ChatCompletionRequestMessage::Assistant(
                    ChatCompletionRequestAssistantMessage {
                        content: if message.content.is_empty() {
                            None
                        } else {
                            Some(ChatCompletionRequestAssistantMessageContent::Text(
                                message.content.clone(),
                            ))
                        },
                        tool_calls,
                        ..Default::default()
                    },
                ));
            }
            ChatRole::Tool => out.push(ChatCompletionRequestMessage::Tool(
                ChatCompletionRequestToolMessage {
                    content: ChatCompletionRequestToolMessageContent::Text(message.content.clone()),
                    tool_call_id: message.tool_call_id.clone().unwrap_or_default(),
                },
            )),
        }
    }
    out
}

/// Map provider-neutral tool specs onto chat-completions function tools.
pub(crate) fn to_openai_tools(tools: &[ToolSpec]) -> Vec<ChatCompletionTools> {
    tools
        .iter()
        .map(|tool| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    strict: None,
                },
            })
        })
        .collect()
}

fn client_for(provider: AIProvider, cfg: &ResolvedProviderConfig) -> Client<OpenAIConfig> {
    let mut config = OpenAIConfig::new();
    match cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => config = config.with_api_key(key),
        // Local servers ignore the key but the Authorization header must parse
        None if provider != AIProvider::OpenAI => config = config.with_api_key("not-needed"),
        None => {}
    }
    if let Some(endpoint) = cfg.endpoint.as_deref() {
        config = config.with_api_base(format!("{}/v1", endpoint));
    }
    Client::with_config(config)
}

fn map_finish(reason: Option<OpenAIFinishReason>, has_tool_calls: bool) -> FinishReason {
    match reason {
        Some(OpenAIFinishReason::ToolCalls) => FinishReason::ToolUse,
        Some(OpenAIFinishReason::Length) => FinishReason::MaxTokens,
        Some(OpenAIFinishReason::ContentFilter) => FinishReason::Refusal,
        // Some compat servers report "stop" even when tool calls are present
        _ if has_tool_calls => FinishReason::ToolUse,
        _ => FinishReason::Stop,
    }
}

fn friendly_error(provider: AIProvider, error: impl std::fmt::Display) -> String {
    let detail = error.to_string();
    let hint = if detail.contains("401") || detail.contains("Unauthorized") {
        " Check the API key in Settings."
    } else if detail.contains("404") || detail.contains("model_not_found") {
        " Check that the configured model name exists on this endpoint."
    } else if detail.contains("429") {
        " Rate limit exceeded — wait a moment and retry."
    } else {
        ""
    };
    format!("{} error: {}.{}", provider, detail, hint)
}

/// One-shot analysis through chat completions (system + single user message).
pub async fn one_shot(
    provider: AIProvider,
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let request = ChatRequest {
        system: Some(system.to_string()),
        messages: vec![ChatMessage::user(prompt)],
        tools: Vec::new(),
        max_tokens: None,
    };
    let client = client_for(provider, cfg);
    let chat_request = CreateChatCompletionRequestArgs::default()
        .model(cfg.model_or_err(provider)?)
        .messages(to_openai_messages(
            request.system.as_deref(),
            &request.messages,
        ))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client
        .chat()
        .create(chat_request)
        .await
        .map_err(|e| friendly_error(provider, e))?;

    Ok(response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .unwrap_or_default())
}

/// Streaming chat with optional tools. Text deltas go through `tx`; tool-call
/// fragments are accumulated by index and returned complete in the final turn.
pub async fn chat_stream(
    provider: AIProvider,
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let client = client_for(provider, cfg);
    let mut args = CreateChatCompletionRequestArgs::default();
    args.model(cfg.model_or_err(provider)?)
        .messages(to_openai_messages(req.system.as_deref(), &req.messages));
    if !req.tools.is_empty() {
        args.tools(to_openai_tools(&req.tools));
    }
    let chat_request = args
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let mut stream = client
        .chat()
        .create_stream(chat_request)
        .await
        .map_err(|e| friendly_error(provider, e))?;

    let mut text = String::new();
    let mut finish: Option<OpenAIFinishReason> = None;
    // index -> (id, name, accumulated argument JSON fragments)
    let mut pending: BTreeMap<u32, (Option<String>, String, String)> = BTreeMap::new();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| friendly_error(provider, e))?;
        let Some(choice) = chunk.choices.into_iter().next() else {
            continue;
        };
        if let Some(content) = choice.delta.content
            && !content.is_empty()
        {
            text.push_str(&content);
            let _ = tx.send(content).await;
        }
        if let Some(fragments) = choice.delta.tool_calls {
            for fragment in fragments {
                let entry = pending.entry(fragment.index).or_default();
                if let Some(id) = fragment.id {
                    entry.0 = Some(id);
                }
                if let Some(function) = fragment.function {
                    if let Some(name) = function.name {
                        entry.1.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        entry.2.push_str(&arguments);
                    }
                }
            }
        }
        if let Some(reason) = choice.finish_reason {
            finish = Some(reason);
        }
    }

    let tool_calls: Vec<ToolCall> = pending
        .into_values()
        .enumerate()
        .map(|(index, (id, name, arguments))| ToolCall {
            id: id.unwrap_or_else(|| format!("{}#{}", name, index)),
            name,
            arguments: serde_json::from_str(&arguments).unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect();

    let finished = map_finish(finish, !tool_calls.is_empty());
    Ok(ChatTurn {
        text,
        tool_calls,
        finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "run_diagnostic".into(),
            description: "Run a diagnostic".into(),
            parameters: json!({"type": "object", "properties": {"task_id": {"type": "string"}}}),
        }
    }

    #[test]
    fn messages_map_to_wire_roles_with_tool_round_trip() {
        let messages = vec![
            ChatMessage::user("check my disk"),
            ChatMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "run_diagnostic".into(),
                    arguments: json!({"task_id": "logical_disk"}),
                }],
            ),
            ChatMessage::tool_result("call_1", "run_diagnostic", "C: 80% free"),
            ChatMessage::assistant("Your disk is fine."),
        ];
        let wire = serde_json::to_value(to_openai_messages(Some("be brief"), &messages)).unwrap();
        let arr = wire.as_array().unwrap();

        assert_eq!(arr[0]["role"], "system");
        assert_eq!(arr[0]["content"], "be brief");
        assert_eq!(arr[1]["role"], "user");
        // Assistant tool-call turn: no content key, function call serialized
        assert_eq!(arr[2]["role"], "assistant");
        assert_eq!(arr[2]["tool_calls"][0]["type"], "function");
        assert_eq!(arr[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            arr[2]["tool_calls"][0]["function"]["name"],
            "run_diagnostic"
        );
        // Tool result correlates by tool_call_id
        assert_eq!(arr[3]["role"], "tool");
        assert_eq!(arr[3]["tool_call_id"], "call_1");
        assert_eq!(arr[3]["content"], "C: 80% free");
        assert_eq!(arr[4]["role"], "assistant");
        assert_eq!(arr[4]["content"], "Your disk is fine.");
    }

    #[test]
    fn tools_serialize_as_function_declarations() {
        let wire = serde_json::to_value(to_openai_tools(&[spec()])).unwrap();
        assert_eq!(wire[0]["type"], "function");
        assert_eq!(wire[0]["function"]["name"], "run_diagnostic");
        assert_eq!(wire[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn finish_reason_maps_tool_calls_and_server_quirks() {
        assert_eq!(
            map_finish(Some(OpenAIFinishReason::ToolCalls), true),
            FinishReason::ToolUse
        );
        assert_eq!(
            map_finish(Some(OpenAIFinishReason::Length), false),
            FinishReason::MaxTokens
        );
        // Compat servers sometimes say "stop" while still returning tool calls
        assert_eq!(
            map_finish(Some(OpenAIFinishReason::Stop), true),
            FinishReason::ToolUse
        );
        assert_eq!(map_finish(None, false), FinishReason::Stop);
    }
}
