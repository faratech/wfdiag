//! Application state types shared across command modules.
//!
//! This module contains the core state structures used by Tauri commands
//! to manage diagnostic sessions, system monitoring, and scan storage.

use crate::diagnostics::TaskResult;
use crate::native_monitor::SystemMonitor;
use crate::results_storage::ScanStorage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionClass {
    OnDevice,
    LocalServer,
    SubscriptionCloud,
    ApiCloud,
}

impl ProviderExecutionClass {
    pub fn is_cloud(self) -> bool {
        matches!(self, Self::SubscriptionCloud | Self::ApiCloud)
    }
}

/// Trust metadata for the provider that actually handled a response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUse {
    pub provider_id: String,
    pub execution_class: ProviderExecutionClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
}

impl ProviderUse {
    pub fn for_provider(
        provider: crate::ai_service::AIProvider,
        fallback_from: Option<crate::ai_service::AIProvider>,
    ) -> Self {
        use crate::ai_service::AIProvider;
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnRecord {
    pub message_id: String,
    pub user_message_index: usize,
    pub display_text: String,
    /// Clean user query used to resume a paused fallback. Provider-only
    /// context lives in the canonical ChatMessage, never in the projection.
    pub query: String,
    pub provider_use: Option<ProviderUse>,
    pub finish_reason: Option<String>,
    /// Provider-neutral terminal text used only by the render projection when
    /// a failed or cancelled turn did not produce a canonical assistant
    /// message. Keeping it outside provider history prevents synthetic error
    /// text from being sent back to a later model turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_message: Option<String>,
    /// Durable, provider-neutral activity records for truthful history
    /// projection after the frontend remounts.
    #[serde(default)]
    pub tool_activities: Vec<ToolActivityRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityRecord {
    pub call_id: String,
    pub tool: String,
    pub args_summary: String,
    /// "queued" | "running" | "completed" | "failed" | "cancelled"
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
    pub tried: Vec<crate::ai_service::AIProvider>,
    pub failed_message: String,
}

/// Information about the current system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub computer_name: String,
    pub os_version: String,
    pub is_admin: bool,
}

/// A diagnostic session containing task results
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Quick,
    Full,
    #[default]
    Targeted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSession {
    pub session_id: String,
    pub start_time: std::time::SystemTime,
    /// The user-visible scope that created this session. This is explicit
    /// rather than inferred from task counts because a non-admin full scan
    /// legitimately omits admin-only tasks and Quick Scan is customizable.
    #[serde(default)]
    pub scan_kind: ScanKind,
    pub selected_tasks: Vec<String>,
    pub results: HashMap<String, TaskResult>,
}

/// One AI chat conversation. Messages are the canonical provider-neutral
/// history including tool calls/results — in-memory only (kept Serialize so
/// persisting recent conversations later is mechanical).
#[derive(Debug, Clone, Serialize)]
pub struct ChatSession {
    pub id: String,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
    pub messages: Vec<crate::ai_providers::ChatMessage>,
    pub turns: Vec<ChatTurnRecord>,
    /// A turn is in flight; concurrent sends are rejected
    pub busy: bool,
    pub active_message_id: Option<String>,
    pub pending_fallback: Option<PendingChatFallback>,
}

/// Control plane for one streaming scan report. `finished` is signalled only
/// after the report's cache-key lock has been released, so a caller awaiting
/// cancellation can safely start a replacement generation.
#[derive(Clone)]
pub struct ReportControl {
    pub cancel: tokio_util::sync::CancellationToken,
    pub finished: tokio_util::sync::CancellationToken,
}

impl ChatSession {
    pub fn new(id: String) -> Self {
        Self {
            id,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            messages: Vec::new(),
            turns: Vec::new(),
            busy: false,
            active_message_id: None,
            pending_fallback: None,
        }
    }
}

/// Main application state managed by Tauri
pub struct AppState {
    pub current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    /// Last usable scan held transactionally while a replacement scan runs.
    /// Cancellation restores it so a failed Full Scan escalation cannot erase
    /// the Quick Scan that motivated it.
    pub previous_session: Arc<Mutex<Option<DiagnosticSession>>>,
    pub system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
    pub monitoring_lease: Arc<AtomicU64>,
    pub scan_storage: Arc<Mutex<Option<ScanStorage>>>,
    pub scan_storage_error: Arc<Mutex<Option<String>>>,
    /// Session ids cancelled via `cancel_diagnostics`. Queued tasks of a
    /// cancelled session are skipped; in-flight tasks run to completion.
    pub cancelled_sessions: Arc<Mutex<HashSet<String>>>,
    /// Session ids with an active `run_diagnostics_parallel` invocation.
    pub active_scan_runners: Arc<Mutex<HashSet<String>>>,
    /// AI chat conversations, keyed by chat session id
    pub chat_sessions: Arc<Mutex<HashMap<String, ChatSession>>>,
    /// Cancellation tokens for in-flight chat turns, keyed by chat session id
    pub chat_cancels: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    /// Report cache keys with a generation currently streaming. Prevents a
    /// fast double-click on "Generate report" from firing a second,
    /// concurrent paid-provider request for the same scan.
    pub report_in_flight: Arc<Mutex<HashSet<String>>>,
    /// Cancellation and completion signals for streaming scan reports, keyed
    /// by report id.
    pub report_cancels: Arc<Mutex<HashMap<String, ReportControl>>>,
    /// Trusted proposal/authorization/execution state for catalog-backed
    /// remediations. Model output never receives a reference to this store.
    pub action_broker: Arc<Mutex<crate::action_broker::ActionBrokerState>>,
}

impl AppState {
    /// Create a new AppState with optional scan storage
    pub fn new(scan_storage: Option<ScanStorage>, scan_storage_error: Option<String>) -> Self {
        Self {
            current_session: Arc::new(Mutex::new(None)),
            previous_session: Arc::new(Mutex::new(None)),
            system_monitor: Arc::new(Mutex::new(None)),
            monitoring_lease: Arc::new(AtomicU64::new(0)),
            scan_storage: Arc::new(Mutex::new(scan_storage)),
            scan_storage_error: Arc::new(Mutex::new(scan_storage_error)),
            cancelled_sessions: Arc::new(Mutex::new(HashSet::new())),
            active_scan_runners: Arc::new(Mutex::new(HashSet::new())),
            chat_sessions: Arc::new(Mutex::new(HashMap::new())),
            chat_cancels: Arc::new(Mutex::new(HashMap::new())),
            report_in_flight: Arc::new(Mutex::new(HashSet::new())),
            report_cancels: Arc::new(Mutex::new(HashMap::new())),
            action_broker: Arc::new(Mutex::new(
                crate::action_broker::ActionBrokerState::default(),
            )),
        }
    }
}
