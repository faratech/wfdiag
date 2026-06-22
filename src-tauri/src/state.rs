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

/// Information about the current system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub computer_name: String,
    pub os_version: String,
    pub is_admin: bool,
}

/// A diagnostic session containing task results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSession {
    pub session_id: String,
    pub start_time: std::time::SystemTime,
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
    pub messages: Vec<crate::ai_providers::ChatMessage>,
    /// A turn is in flight; concurrent sends are rejected
    pub busy: bool,
}

impl ChatSession {
    pub fn new(id: String) -> Self {
        Self {
            id,
            created_at: std::time::SystemTime::now(),
            messages: Vec::new(),
            busy: false,
        }
    }
}

/// Main application state managed by Tauri
pub struct AppState {
    pub current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    pub system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
    pub monitoring_lease: Arc<AtomicU64>,
    pub scan_storage: Arc<Mutex<Option<ScanStorage>>>,
    pub scan_storage_error: Arc<Mutex<Option<String>>>,
    /// Session ids cancelled via `cancel_diagnostics`. Queued tasks of a
    /// cancelled session are skipped; in-flight tasks run to completion.
    pub cancelled_sessions: Arc<Mutex<HashSet<String>>>,
    /// AI chat conversations, keyed by chat session id
    pub chat_sessions: Arc<Mutex<HashMap<String, ChatSession>>>,
    /// Cancellation tokens for in-flight chat turns, keyed by chat session id
    pub chat_cancels: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
}

impl AppState {
    /// Create a new AppState with optional scan storage
    pub fn new(scan_storage: Option<ScanStorage>, scan_storage_error: Option<String>) -> Self {
        Self {
            current_session: Arc::new(Mutex::new(None)),
            system_monitor: Arc::new(Mutex::new(None)),
            monitoring_lease: Arc::new(AtomicU64::new(0)),
            scan_storage: Arc::new(Mutex::new(scan_storage)),
            scan_storage_error: Arc::new(Mutex::new(scan_storage_error)),
            cancelled_sessions: Arc::new(Mutex::new(HashSet::new())),
            chat_sessions: Arc::new(Mutex::new(HashMap::new())),
            chat_cancels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
