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

pub use wfdiag_native_system::SystemInfo;

pub use wfdiag_native_ai_chat::{
    ChatSession, ChatTurnRecord, PendingChatFallback, ProviderExecutionClass, ProviderUse,
    ToolActivityRecord,
};

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

/// Control plane for one streaming scan report. `finished` is signalled only
/// after the report's cache-key lock has been released, so a caller awaiting
/// cancellation can safely start a replacement generation.
#[derive(Clone)]
pub struct ReportControl {
    pub cancel: tokio_util::sync::CancellationToken,
    pub finished: tokio_util::sync::CancellationToken,
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
