//! Application state types shared across command modules.
//!
//! This module contains the core state structures used by Tauri commands
//! to manage diagnostic sessions, system monitoring, and scan storage.

use crate::diagnostics::TaskResult;
use crate::native_monitor::SystemMonitor;
use crate::results_storage::ScanStorage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
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

/// Main application state managed by Tauri
pub struct AppState {
    pub current_session: Arc<Mutex<Option<DiagnosticSession>>>,
    pub system_monitor: Arc<Mutex<Option<SystemMonitor>>>,
    pub scan_storage: Arc<Mutex<Option<ScanStorage>>>,
    pub scan_storage_error: Arc<Mutex<Option<String>>>,
}

impl AppState {
    /// Create a new AppState with optional scan storage
    pub fn new(scan_storage: Option<ScanStorage>, scan_storage_error: Option<String>) -> Self {
        Self {
            current_session: Arc::new(Mutex::new(None)),
            system_monitor: Arc::new(Mutex::new(None)),
            scan_storage: Arc::new(Mutex::new(scan_storage)),
            scan_storage_error: Arc::new(Mutex::new(scan_storage_error)),
        }
    }
}
