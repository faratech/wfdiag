//! The Diagnostics screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::app::policy::scan_kind_label;
use crate::app::state::DiagnosticAnalysisDisplay;
use std::collections::{BTreeMap, HashMap};
use wfdiag_app::domain::scan::ScanPhase;
use wfdiag_native_diagnostics::{DiagnosticTask, ScanKind};
use wfdiag_ui_core::{DiagnosticTaskResult, TaskProgressStatus};

/// Everything the Diagnostics page renders.
#[derive(Default)]
pub(crate) struct DiagnosticsScreen {
    pub(crate) results: Vec<DiagnosticTaskResult>,
    pub(crate) catalog: Vec<DiagnosticTask>,
    pub(crate) expected_task_ids: Vec<String>,
    pub(crate) task_statuses: HashMap<String, TaskProgressStatus>,
    pub(crate) scan_kind: Option<ScanKind>,
    pub(crate) duration_ms: u64,
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) errors: usize,
    pub(crate) current_task: Option<String>,
    pub(crate) scan_phase: ScanPhase,
    pub(crate) selected_task_id: Option<String>,
    pub(crate) filter: String,
    pub(crate) raw_output: bool,
    pub(crate) analyses: BTreeMap<String, DiagnosticAnalysisDisplay>,
}

impl DiagnosticsScreen {
    /// Whether a scan occupies the engine.
    pub(crate) const fn busy(&self) -> bool {
        !matches!(self.scan_phase, ScanPhase::Idle)
    }

    /// Whether the scan is winding down after a Stop.
    pub(crate) const fn cancelling(&self) -> bool {
        matches!(self.scan_phase, ScanPhase::Cancelling)
    }

    /// The name the status line uses for the running or last scan.
    pub(crate) fn scan_label(&self) -> &'static str {
        self.scan_kind.map_or("Scan", scan_kind_label)
    }

    /// The session the visible results came from.
    pub(crate) fn visible_session_id(&self) -> Option<&str> {
        self.results
            .first()
            .map(|result| result.session_id.as_str())
    }
}

/// Everything the Diagnostics page can ask for.
#[derive(Clone)]
pub(crate) enum DiagnosticsMsg {
    RequestQuickScan,
    RequestFullScan,
    CancelScan,
    FilterChanged(String),
    SetRawOutput(bool),
    SelectResult(String),
    AnalyzeSelected,
    RetrySelectedAnalysis,
    CancelAnalysis,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wfdiag_native_diagnostics::DiagnosticOutput;

    fn screen(phase: ScanPhase) -> DiagnosticsScreen {
        DiagnosticsScreen {
            scan_phase: phase,
            ..DiagnosticsScreen::default()
        }
    }

    #[test]
    fn only_a_non_idle_phase_is_busy() {
        assert!(!screen(ScanPhase::Idle).busy());
        assert!(screen(ScanPhase::Cancelling).busy());
    }

    #[test]
    fn only_the_cancelling_phase_is_cancelling() {
        assert!(!screen(ScanPhase::Idle).cancelling());
        assert!(screen(ScanPhase::Cancelling).cancelling());
    }

    #[test]
    fn the_scan_label_falls_back_before_the_first_scan() {
        assert_eq!(DiagnosticsScreen::default().scan_label(), "Scan");
        assert_eq!(
            DiagnosticsScreen {
                scan_kind: Some(ScanKind::Quick),
                ..DiagnosticsScreen::default()
            }
            .scan_label(),
            scan_kind_label(ScanKind::Quick)
        );
    }

    #[test]
    fn the_visible_session_is_the_first_results_session() {
        assert_eq!(DiagnosticsScreen::default().visible_session_id(), None);
        let screen = DiagnosticsScreen {
            results: vec![DiagnosticTaskResult::new(
                "session-7",
                "computer_system",
                Arc::new(DiagnosticOutput {
                    success: true,
                    output: String::new(),
                    error: None,
                    duration_ms: 1,
                }),
            )],
            ..DiagnosticsScreen::default()
        };
        assert_eq!(screen.visible_session_id(), Some("session-7"));
    }
}
