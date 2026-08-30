use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use wfdiag_native_ai_provider::AIProvider;

/// Role in the provider-neutral canonical conversation history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// One provider-neutral message. Provider-private replay state is kept only
/// in memory and is deliberately excluded from serialization and UI events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip)]
    pub tool_result_is_error: bool,
    #[serde(skip)]
    pub provider_replay: Option<ProviderReplay>,
}

impl ChatMessage {
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(ChatRole::User, content)
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(ChatRole::Assistant, content)
    }

    #[must_use]
    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self::assistant_with_replay(content, tool_calls, None)
    }

    #[must_use]
    pub fn assistant_with_replay(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        provider_replay: Option<ProviderReplay>,
    ) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
            tool_result_is_error: false,
            provider_replay,
        }
    }

    #[must_use]
    pub fn tool_result(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::tool(call_id, tool_name, content, false)
    }

    #[must_use]
    pub fn tool_error(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::tool(call_id, tool_name, content, true)
    }

    fn plain(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            tool_result_is_error: false,
            provider_replay: None,
        }
    }

    fn tool(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
        tool_result_is_error: bool,
    ) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            tool_name: Some(tool_name.into()),
            tool_result_is_error,
            provider_replay: None,
        }
    }
}

/// Opaque provider continuation state. The custom `Debug` implementation
/// redacts provider blocks because they can contain hidden reasoning.
#[derive(Clone)]
pub enum ProviderReplay {
    Anthropic {
        requested_model: String,
        content_blocks: Vec<serde_json::Value>,
    },
}

impl ProviderReplay {
    #[must_use]
    pub fn char_count(&self) -> usize {
        match self {
            Self::Anthropic { content_blocks, .. } => {
                serde_json::to_string(content_blocks).map_or(0, |json| json.chars().count())
            }
        }
    }
}

impl std::fmt::Debug for ProviderReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic {
                requested_model,
                content_blocks,
            } => formatter
                .debug_struct("AnthropicReplay")
                .field("requested_model", requested_model)
                .field(
                    "content_blocks",
                    &format_args!("<redacted:{}>", content_blocks.len()),
                )
                .finish(),
        }
    }
}

/// A tool the model may call (JSON Schema parameters).
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// One tool invocation requested by a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// One provider-neutral chat request.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolUse,
    MaxTokens,
    Refusal,
}

/// A provider's completed response to one model request.
#[derive(Debug, Clone)]
pub struct ChatTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finished: FinishReason,
    pub actual_models: Vec<String>,
    pub provider_replay: Option<ProviderReplay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionClass {
    OnDevice,
    LocalServer,
    SubscriptionCloud,
    ApiCloud,
}

impl ProviderExecutionClass {
    #[must_use]
    pub const fn is_cloud(self) -> bool {
        matches!(self, Self::SubscriptionCloud | Self::ApiCloud)
    }
}

/// Trust and concrete-model metadata for the provider handling a response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUse {
    pub provider_id: String,
    pub execution_class: ProviderExecutionClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actual_models: Vec<String>,
}

impl ProviderUse {
    #[must_use]
    pub fn for_provider(provider: AIProvider, fallback_from: Option<AIProvider>) -> Self {
        let execution_class = match provider {
            AIProvider::PhiSilica => ProviderExecutionClass::OnDevice,
            AIProvider::FoundryLocal | AIProvider::Ollama => ProviderExecutionClass::LocalServer,
            AIProvider::CodexCli | AIProvider::ClaudeCode => {
                ProviderExecutionClass::SubscriptionCloud
            }
            AIProvider::None
            | AIProvider::OpenAI
            | AIProvider::CustomOpenAI
            | AIProvider::Anthropic
            | AIProvider::Gemini
            | AIProvider::DeepSeek => ProviderExecutionClass::ApiCloud,
        };
        Self {
            provider_id: provider.to_string(),
            execution_class,
            fallback_from: fallback_from.map(|from| from.to_string()),
            requested_model: None,
            actual_models: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_requested_model(mut self, model: Option<&str>) -> Self {
        self.requested_model = model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string);
        self
    }

    pub fn set_actual_models(&mut self, models: impl IntoIterator<Item = String>) {
        self.actual_models.clear();
        self.merge_actual_models(models);
    }

    pub fn merge_actual_models(&mut self, models: impl IntoIterator<Item = String>) {
        for model in models {
            let model = model.trim();
            if !model.is_empty() && !self.actual_models.iter().any(|seen| seen == model) {
                self.actual_models.push(model.to_string());
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnRecord {
    pub message_id: String,
    pub user_message_index: usize,
    pub display_text: String,
    pub query: String,
    pub provider_use: Option<ProviderUse>,
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_message: Option<String>,
    #[serde(default)]
    pub tool_activities: Vec<ToolActivityRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityRecord {
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingChatFallback {
    pub message_id: String,
    pub from: ProviderUse,
    pub to: ProviderUse,
    pub tried: Vec<AIProvider>,
    pub failed_message: String,
}

/// Canonical backend conversation state shared by native and Tauri shells.
#[derive(Debug, Clone, Serialize)]
pub struct ChatSession {
    pub id: String,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub messages: Vec<ChatMessage>,
    pub turns: Vec<ChatTurnRecord>,
    pub busy: bool,
    pub active_message_id: Option<String>,
    pub pending_fallback: Option<PendingChatFallback>,
}

impl ChatSession {
    #[must_use]
    pub fn new(id: String) -> Self {
        Self {
            id,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            messages: Vec::new(),
            turns: Vec::new(),
            busy: false,
            active_message_id: None,
            pending_fallback: None,
        }
    }
}
