//! How the Diagnostics screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::policy::{
    scan_complete_text, scan_kind_label, scan_progress_text, scan_result_text,
};
use crate::app::screen::{Effect, ScreenCx};
use crate::screens::diagnostics::state::{DiagnosticsMsg, DiagnosticsScreen};
use crate::screens::diagnostics::view::diagnostic_matches_filter;
use wfdiag_app::{AnalysisEvent, AppCommand, AppEvent, DispatchOutcome, ScanEvent};
use wfdiag_native_diagnostics::ScanKind;

impl DiagnosticsScreen {
    pub(crate) fn update(&mut self, message: DiagnosticsMsg, cx: &mut ScreenCx<'_>) {
        match message {
            DiagnosticsMsg::RequestQuickScan => cx.effect(Effect::BeginScan(ScanKind::Quick)),
            DiagnosticsMsg::RequestFullScan => cx.effect(Effect::BeginScan(ScanKind::Full)),
            DiagnosticsMsg::CancelScan => {
                let outcome = cx.dispatch(AppCommand::CancelScan);
                if let DispatchOutcome::Rejected(_) = &outcome {
                    cx.report_rejection(&outcome);
                }
            }
            DiagnosticsMsg::FilterChanged(value) => self.apply_filter(value),
            DiagnosticsMsg::SetRawOutput(raw) => self.raw_output = raw,
            DiagnosticsMsg::SelectResult(task_id) => {
                self.selected_task_id = Some(task_id.clone());
                self.raw_output = false;
                cx.status(format!("Selected diagnostic: {task_id}"));
            }
            DiagnosticsMsg::AnalyzeSelected => self.begin_analysis(false, cx),
            DiagnosticsMsg::RetrySelectedAnalysis => self.begin_analysis(true, cx),
            DiagnosticsMsg::CancelAnalysis => {
                if cx.dispatch(AppCommand::CancelAnalysis).is_accepted() {
                    cx.status("Cancelling the diagnostic interpretation…");
                }
            }
        }
    }

    /// Narrow the result list, keeping the selection only while it stays
    /// visible.
    fn apply_filter(&mut self, value: String) {
        self.filter = value;
        let selected_visible = self.selected_task_id.as_deref().is_some_and(|id| {
            self.results.iter().any(|result| {
                result.task_id == id
                    && diagnostic_matches_filter(result, &self.catalog, &self.filter)
            })
        });
        if !selected_visible {
            self.selected_task_id = self
                .results
                .iter()
                .find(|result| diagnostic_matches_filter(result, &self.catalog, &self.filter))
                .map(|result| result.task_id.clone());
            self.raw_output = false;
        }
    }

    pub(crate) fn begin_analysis(&mut self, force_refresh: bool, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual {
            cx.status("Visual fixture mode · diagnostic AI is disabled");
            return;
        }
        let Some(task_id) = self
            .selected_task_id
            .clone()
            .or_else(|| self.results.first().map(|result| result.task_id.clone()))
        else {
            cx.status("Run a diagnostic scan before asking for an interpretation");
            return;
        };
        if !force_refresh
            && self
                .analyses
                .get(&task_id)
                .and_then(|display| display.interpretation.as_ref())
                .is_some()
        {
            cx.status("Diagnostic interpretation is already available");
            return;
        }
        match cx.dispatch(AppCommand::AnalyzeDiagnostic {
            task_id,
            force_refresh,
        }) {
            DispatchOutcome::Accepted { .. } => {
                cx.status("Interpreting the selected diagnostic…");
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        match event {
            AppEvent::Scan(event) => self.on_scan_event(event, cx),
            AppEvent::Analysis(event) => Self::on_analysis_event(event, cx),
            _ => {}
        }
    }

    fn on_scan_event(&mut self, event: &ScanEvent, cx: &mut ScreenCx<'_>) {
        match event {
            ScanEvent::Started { kind, .. } => {
                self.task_statuses.clear();
                self.current_task = None;
                cx.status(format!("{} started", scan_kind_label(*kind)));
            }
            ScanEvent::StartFailed { error } => {
                cx.status(format!("Could not start diagnostics · {error}"));
            }
            ScanEvent::Progress {
                task_id,
                status,
                task_name,
                completed,
                total,
            } => {
                let task_name = task_name.clone().unwrap_or_else(|| {
                    self.catalog
                        .iter()
                        .find(|task| &task.id == task_id)
                        .map_or_else(|| task_id.clone(), |task| task.name.clone())
                });
                self.task_statuses.insert(task_id.clone(), *status);
                if *status == wfdiag_ui_core::TaskProgressStatus::Running {
                    self.current_task = Some(task_name.clone());
                }
                cx.status(scan_progress_text(
                    self.scan_label(),
                    self.cancelling(),
                    *completed,
                    *total,
                    &task_name,
                ));
            }
            ScanEvent::TaskResult {
                completed, errors, ..
            } => {
                cx.status(scan_result_text(
                    self.scan_label(),
                    self.busy(),
                    *completed,
                    self.total,
                    *errors,
                ));
            }
            ScanEvent::Committed {
                completed,
                errors,
                auto_save,
                ..
            } => {
                cx.status(if *auto_save {
                    format!("Finalizing {}…", self.scan_label().to_ascii_lowercase())
                } else {
                    scan_complete_text(self.scan_label(), *completed, *errors, false)
                });
            }
            ScanEvent::TargetedCommitted { .. } => {
                cx.status(format!(
                    "Diagnostic rerun complete · {} collected · {} errors",
                    self.completed, self.errors
                ));
            }
            ScanEvent::TargetedFailed { error } => {
                cx.status(format!(
                    "Targeted Scan failed · {error} · previous results restored"
                ));
            }
            ScanEvent::Cancelled => {
                self.current_task = None;
                cx.status(format!(
                    "{} stopped · previous results restored",
                    self.scan_label()
                ));
            }
            ScanEvent::CancelAcknowledged => {
                cx.status(format!(
                    "Stopping {} after in-flight checks finish…",
                    self.scan_kind.map_or("scan", scan_kind_label)
                ));
            }
            ScanEvent::CancelFailed { error } => {
                cx.status(format!("Could not stop the scan · {error}"));
            }
            ScanEvent::Failed { error, stopped } => {
                self.current_task = None;
                let label = self.scan_label();
                cx.status(if *stopped {
                    format!("{label} stopped · previous results restored")
                } else {
                    format!("{label} failed · {error} · previous results restored")
                });
            }
            ScanEvent::Incomplete {
                completed,
                expected,
            } => {
                cx.status(format!(
                    "{} returned an invalid result set ({completed} results for {expected} expected checks) · previous results restored",
                    self.scan_label()
                ));
            }
            // The shell owns finalization: the completion toast, the history
            // reload, and the final status line all outlive this screen.
            ScanEvent::Finalized { .. } => self.current_task = None,
        }
    }

    fn on_analysis_event(event: &AnalysisEvent, cx: &mut ScreenCx<'_>) {
        match event {
            AnalysisEvent::Started { cached, .. } => {
                cx.status(if *cached {
                    "Loading the cached diagnostic interpretation…"
                } else {
                    "The AI provider is interpreting the diagnostic…"
                });
            }
            AnalysisEvent::Completed {
                cached,
                provider_use,
                ..
            } => {
                let provider = provider_use.provider_id.clone();
                cx.status(if *cached {
                    format!("Diagnostic interpretation ready · {provider} · cached")
                } else {
                    format!("Diagnostic interpretation ready · {provider}")
                });
            }
            AnalysisEvent::Failed { message, .. } => {
                cx.status(format!("Diagnostic interpretation failed · {message}"));
            }
            AnalysisEvent::Cancelled { .. } => {
                cx.status("Diagnostic interpretation cancelled");
            }
            _ => {}
        }
    }
}
