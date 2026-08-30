//! UI adapters retained while the Tauri and Reactor shells coexist.

use crate::diagnostic_events::{
    DiagnosticEmitFuture, DiagnosticEvent, DiagnosticEventSink, DiagnosticTaskStatus,
};
use crate::native_monitor::{MonitorEmission, MonitorEmitter, SystemStats as NativeSystemStats};
use tauri::{AppHandle, Emitter};
use wfdiag_ui_core::{
    DiagnosticTaskResult, TaskProgress, TaskProgressStatus, UiEvent, UiEventPublisher,
};

/// Preserves the existing `system-stats` Tauri event while the collector lives
/// behind its framework-neutral emitter boundary.
pub struct TauriMonitorEmitter(AppHandle);

impl TauriMonitorEmitter {
    #[must_use]
    pub fn new(app_handle: AppHandle) -> Self {
        Self(app_handle)
    }
}

impl MonitorEmitter for TauriMonitorEmitter {
    fn emit_system_stats(&self, stats: &NativeSystemStats) -> MonitorEmission {
        let _ = self.0.emit("system-stats", stats);
        MonitorEmission::Continue
    }
}

/// Sends diagnostic progress and complete task evidence to a native UI bus.
pub struct UiBusDiagnosticEventSink {
    publisher: UiEventPublisher,
}

impl UiBusDiagnosticEventSink {
    #[must_use]
    pub fn new(publisher: UiEventPublisher) -> Self {
        Self { publisher }
    }
}

impl DiagnosticEventSink for UiBusDiagnosticEventSink {
    fn emit(&self, event: DiagnosticEvent) -> DiagnosticEmitFuture<'_> {
        Box::pin(async move {
            let event = match event {
                DiagnosticEvent::Progress(progress) => UiEvent::TaskProgress(TaskProgress {
                    session_id: progress.session_id.unwrap_or_default(),
                    task_id: progress.task_id,
                    status: progress.status.into(),
                    task_name: progress.task_name,
                    success: progress.success,
                }),
                DiagnosticEvent::Result(result) => {
                    UiEvent::DiagnosticResult(DiagnosticTaskResult {
                        session_id: result.session_id.unwrap_or_default(),
                        task_id: result.task_id,
                        success: result.result.success,
                        output: result.result.output,
                        error: result.result.error,
                        duration_ms: result.result.duration_ms,
                    })
                }
            };

            self.publisher
                .publish(event)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

impl From<DiagnosticTaskStatus> for TaskProgressStatus {
    fn from(value: DiagnosticTaskStatus) -> Self {
        match value {
            DiagnosticTaskStatus::Queued => Self::Queued,
            DiagnosticTaskStatus::Running => Self::Running,
            DiagnosticTaskStatus::Completed => Self::Completed,
            DiagnosticTaskStatus::Failed => Self::Failed,
            DiagnosticTaskStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_events::{
        DiagnosticProgress, DiagnosticResultEvent, DiagnosticTaskStatus,
    };
    use crate::diagnostics::TaskResult;
    use std::num::NonZeroUsize;
    use wfdiag_ui_core::ui_event_bus;

    #[tokio::test]
    async fn diagnostic_sink_projects_progress_and_complete_results_in_order() {
        let (publisher, receiver) = ui_event_bus(NonZeroUsize::new(4).expect("four is non-zero"));
        let sink = UiBusDiagnosticEventSink::new(publisher);

        sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
            session_id: Some("scan-7".into()),
            task_id: "cpu".into(),
            status: DiagnosticTaskStatus::Running,
            task_name: Some("CPU information".into()),
            success: None,
        }))
        .await
        .unwrap();
        sink.emit(DiagnosticEvent::Result(DiagnosticResultEvent {
            session_id: Some("scan-7".into()),
            task_id: "cpu".into(),
            result: TaskResult {
                success: true,
                output: "{\"Name\":\"Example CPU\"}".into(),
                error: None,
                duration_ms: 17,
            },
        }))
        .await
        .unwrap();
        sink.emit(DiagnosticEvent::Progress(DiagnosticProgress {
            session_id: Some("scan-7".into()),
            task_id: "cpu".into(),
            status: DiagnosticTaskStatus::Completed,
            task_name: None,
            success: Some(true),
        }))
        .await
        .unwrap();

        assert_eq!(
            receiver.drain(),
            vec![
                UiEvent::DiagnosticResult(DiagnosticTaskResult {
                    session_id: "scan-7".into(),
                    task_id: "cpu".into(),
                    success: true,
                    output: "{\"Name\":\"Example CPU\"}".into(),
                    error: None,
                    duration_ms: 17,
                }),
                UiEvent::TaskProgress(TaskProgress {
                    session_id: "scan-7".into(),
                    task_id: "cpu".into(),
                    status: TaskProgressStatus::Completed,
                    task_name: None,
                    success: Some(true),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn diagnostic_sink_reports_a_closed_native_receiver() {
        let (publisher, receiver) = ui_event_bus(NonZeroUsize::new(1).expect("one is non-zero"));
        let sink = UiBusDiagnosticEventSink::new(publisher);
        drop(receiver);

        let error = sink
            .emit(DiagnosticEvent::Progress(DiagnosticProgress {
                session_id: Some("scan-7".into()),
                task_id: "cpu".into(),
                status: DiagnosticTaskStatus::Running,
                task_name: Some("CPU information".into()),
                success: None,
            }))
            .await
            .unwrap_err();

        assert_eq!(error, "the UI event receiver is closed");
    }
}
