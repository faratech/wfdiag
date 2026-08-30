//! UI-framework-neutral events emitted by diagnostic orchestration.
//!
//! Tauri remains a compatibility adapter: it forwards the historical
//! `task-progress` payload and continues returning results from the invoke.
//! Native shells can instead consume both progress and complete task results
//! through another [`DiagnosticEventSink`] implementation.

use crate::diagnostics::TaskResult;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticTaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticProgress {
    pub session_id: Option<String>,
    pub task_id: String,
    pub status: DiagnosticTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticResultEvent {
    pub session_id: Option<String>,
    pub task_id: String,
    pub result: TaskResult,
}

#[derive(Debug, Clone)]
pub enum DiagnosticEvent {
    Progress(DiagnosticProgress),
    Result(DiagnosticResultEvent),
}

pub type DiagnosticEmitFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Delivery boundary used by diagnostics without knowing which UI owns it.
pub trait DiagnosticEventSink: Send + Sync {
    /// Whether complete diagnostic output should be cloned for this sink.
    /// Compatibility sinks that already return results out-of-band can opt
    /// out and retain the legacy allocation profile.
    fn accepts_results(&self) -> bool {
        true
    }

    fn emit(&self, event: DiagnosticEvent) -> DiagnosticEmitFuture<'_>;
}

/// Compatibility sink for the shipping React/Tauri shell.
///
/// Result events are deliberately ignored because the existing frontend gets
/// the same results from the resolved command. Only the established
/// `task-progress` event is emitted, with its existing wire shape.
pub struct TauriDiagnosticEventSink {
    window: tauri::Window,
}

impl TauriDiagnosticEventSink {
    #[must_use]
    pub fn new(window: tauri::Window) -> Self {
        Self { window }
    }
}

impl DiagnosticEventSink for TauriDiagnosticEventSink {
    fn accepts_results(&self) -> bool {
        false
    }

    fn emit(&self, event: DiagnosticEvent) -> DiagnosticEmitFuture<'_> {
        Box::pin(async move {
            match event {
                DiagnosticEvent::Progress(progress) => self
                    .window
                    .emit("task-progress", progress)
                    .map_err(|error| format!("Failed to emit event: {error}")),
                DiagnosticEvent::Result(_) => Ok(()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn running_progress_preserves_the_tauri_wire_shape() {
        let progress = DiagnosticProgress {
            session_id: Some("scan-7".into()),
            task_id: "cpu".into(),
            status: DiagnosticTaskStatus::Running,
            task_name: Some("CPU information".into()),
            success: None,
        };

        assert_eq!(
            serde_json::to_value(progress).unwrap(),
            json!({
                "session_id": "scan-7",
                "task_id": "cpu",
                "status": "running",
                "task_name": "CPU information"
            })
        );
    }

    #[test]
    fn completed_progress_preserves_the_tauri_wire_shape() {
        let progress = DiagnosticProgress {
            session_id: Some("scan-7".into()),
            task_id: "cpu".into(),
            status: DiagnosticTaskStatus::Completed,
            task_name: None,
            success: Some(false),
        };

        assert_eq!(
            serde_json::to_value(progress).unwrap(),
            json!({
                "session_id": "scan-7",
                "task_id": "cpu",
                "status": "completed",
                "success": false
            })
        );
    }

    #[test]
    fn absent_session_is_serialized_as_null_like_the_existing_command() {
        let progress = DiagnosticProgress {
            session_id: None,
            task_id: "cpu".into(),
            status: DiagnosticTaskStatus::Running,
            task_name: Some("CPU information".into()),
            success: None,
        };

        assert_eq!(
            serde_json::to_value(progress).unwrap()["session_id"],
            json!(null)
        );
    }
}
