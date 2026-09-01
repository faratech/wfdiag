use crate::ProviderUse;
use serde::{Deserialize, Serialize};

// These payloads are the shipping `ai-chat://*` wire contract. Field names
// are pinned by tests and remain camelCase for the Tauri adapter.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaPayload {
    pub session_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPayload {
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonePayload {
    pub session_id: String,
    pub message_id: String,
    pub finish_reason: String,
    pub provider: String,
    pub provider_use: ProviderUse,
    pub tool_call_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub session_id: String,
    pub message_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackRequiredPayload {
    pub session_id: String,
    pub message_id: String,
    pub from: ProviderUse,
    pub to: ProviderUse,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalPayload {
    pub session_id: String,
    pub message_id: String,
    pub proposal: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequestPayload {
    pub session_id: String,
    pub message_id: String,
    pub source_scan_id: String,
    pub kind: String,
    pub reason: String,
    pub question: String,
}

/// Shell-supplied event sink. Native Reactor can deliver these through its
/// command channel while Tauri maps them to the established event names.
pub trait ChatEmitter: Send + Sync {
    fn delta(&self, payload: &DeltaPayload);
    fn tool(&self, payload: &ToolPayload);
    fn done(&self, payload: &DonePayload);
    fn error(&self, payload: &ErrorPayload);
    fn fallback_required(&self, _payload: &FallbackRequiredPayload) {}
    fn proposal(&self, _payload: &ProposalPayload) {}
    fn scan_request(&self, _payload: &ScanRequestPayload) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityView {
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageView {
    pub id: String,
    pub role: String,
    pub text: String,
    pub tools: Vec<ToolActivityView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_use: Option<ProviderUse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFallbackView {
    pub message_id: String,
    pub from: ProviderUse,
    pub to: ProviderUse,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionSnapshot {
    pub session_id: String,
    pub messages: Vec<ChatMessageView>,
    pub busy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_fallback: Option<PendingFallbackView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendAck {
    pub session_id: String,
    pub message_id: String,
    pub provider: String,
    pub provider_use: ProviderUse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatContextRef {
    pub kind: String,
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderExecutionClass;
    use serde_json::json;

    fn provider_use() -> ProviderUse {
        ProviderUse {
            provider_id: "openai".to_string(),
            execution_class: ProviderExecutionClass::ApiCloud,
            fallback_from: Some("ollama".to_string()),
            requested_model: Some("gpt-5-nano".to_string()),
            actual_models: vec!["gpt-5-nano-2026-08-01".to_string()],
        }
    }

    #[test]
    fn delta_and_done_keep_the_shipping_camel_case_contract() {
        let delta = serde_json::to_value(DeltaPayload {
            session_id: "s1".to_string(),
            message_id: "m1".to_string(),
            text: "hello".to_string(),
        })
        .expect("delta should serialize");
        assert_eq!(
            delta,
            json!({"sessionId": "s1", "messageId": "m1", "text": "hello"})
        );

        let done = serde_json::to_value(DonePayload {
            session_id: "s1".to_string(),
            message_id: "m1".to_string(),
            finish_reason: "tool_budget".to_string(),
            provider: "openai".to_string(),
            provider_use: provider_use(),
            tool_call_count: 4,
        })
        .expect("done should serialize");
        assert_eq!(done["finishReason"], "tool_budget");
        assert_eq!(done["providerUse"]["executionClass"], "api_cloud");
        assert_eq!(done["providerUse"]["fallbackFrom"], "ollama");
        assert!(done.get("finish_reason").is_none());
    }

    #[test]
    fn optional_tool_fields_are_omitted_exactly_as_before() {
        let value = serde_json::to_value(ToolPayload {
            session_id: "s1".to_string(),
            message_id: "m1".to_string(),
            call_id: "c1".to_string(),
            tool: "get_scan_summary".to_string(),
            args_summary: String::new(),
            status: "queued".to_string(),
            duration_ms: None,
            result_preview: None,
        })
        .expect("tool should serialize");
        assert_eq!(value["callId"], "c1");
        assert!(value.get("durationMs").is_none());
        assert!(value.get("resultPreview").is_none());
    }
}
