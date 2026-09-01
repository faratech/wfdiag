//! Turning user intents into [`AppCommand`]s.
//!
//! Every method here is the same three steps: check the few things that are
//! genuinely presentational (a fixture mode, an open consent dialog, a draft
//! the user has not saved), dispatch one command, and turn a
//! [`DispatchOutcome`] into status text. The engine owns the rest — whether a
//! worker is available, whether a request is already in flight, which request
//! id it gets, and whether its eventual reply is still current.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{rejection_text, scan_kind_label};
use crate::fixtures::visual::LiveTestFixture;
use crate::platform::notifications;
use wfdiag_app::{AppCommand, DispatchOutcome, RejectReason};
use wfdiag_native_diagnostics::ScanKind;

impl WfdiagShell {
    /// Route one command into the engine.
    ///
    /// A fixture build has no engine at all, which is exactly why a screenshot
    /// capture cannot start a scan, write settings, or run a remediation.
    pub(crate) fn dispatch(&mut self, command: AppCommand) -> DispatchOutcome {
        let Some(app) = self.app.as_mut() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "the diagnostic engine is not running".to_string(),
            });
        };
        app.dispatch(command)
    }

    /// Show a refusal in the status line, using the engine's own wording.
    pub(crate) fn report_rejection(&mut self, outcome: &DispatchOutcome) {
        if let Some(reason) = outcome.rejection() {
            self.shell.status = rejection_text(reason);
        }
    }

    // ---- scanning ---------------------------------------------------------

    /// Queue the scan-completion toast, mirroring the shipping plugin's
    /// behavior when notifications are enabled.
    ///
    /// #206: the toast runs on one shared worker thread instead of a detached
    /// thread per scan, and its failure is no longer swallowed — the first one
    /// is reported once in the status line so a silently missing notification
    /// is explainable.
    pub(crate) fn notify_scan_completion(&mut self) {
        if self.shell.deterministic_visual || !self.shell.settings.show_notifications {
            return;
        }
        let collected = self.diagnostics.results.len();
        let errors = self
            .diagnostics
            .results
            .iter()
            .filter(|result| !result.success)
            .count();
        if let Err(error) = notifications::request_scan_complete_toast(collected, errors) {
            self.report_notification_failure(error);
        }
    }

    /// Surface at most one notification failure for the session.
    pub(crate) fn report_notification_failure(&mut self, error: String) {
        if self.shell.notification_failure_reported {
            return;
        }
        self.shell.notification_failure_reported = true;
        self.shell.status = format!("Notification not shown · {error}");
    }

    pub(crate) fn begin_diagnostic_scan(&mut self, scan_kind: ScanKind) {
        if self.shell.deterministic_visual {
            // Screenshot fixtures must never launch WMI, commands, or mutate
            // the captured Store 2.5.8 state.
            self.shell.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::StartScan { kind: scan_kind }) {
            DispatchOutcome::Accepted { .. } => {
                self.shell.status = format!("Starting {}…", scan_kind_label(scan_kind));
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_targeted_diagnostic_scan(&mut self, task_id: &str) {
        if self.shell.deterministic_visual {
            self.shell.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        let Some(task) = self
            .diagnostics
            .catalog
            .iter()
            .find(|task| task.id == task_id)
        else {
            self.shell.status = "That diagnostic task is no longer available".to_string();
            return;
        };
        if task.admin_required && !self.shell.is_admin {
            self.shell.status = format!("{} requires administrator access", task.name);
            return;
        }
        let task_ids = vec![task.id.clone()];
        match self.dispatch(AppCommand::StartTargetedScan { task_ids }) {
            DispatchOutcome::Accepted { .. } => {
                self.shell.status = format!("Starting {}…", scan_kind_label(ScanKind::Targeted));
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn request_diagnostic_cancel(&mut self) {
        let outcome = self.dispatch(AppCommand::CancelScan);
        if let DispatchOutcome::Rejected(_) = &outcome {
            self.report_rejection(&outcome);
        }
    }

    pub(crate) fn request_admin_relaunch(&mut self) {
        if self.shell.deterministic_visual
            && self.shell.live_test_fixture != Some(LiveTestFixture::AdminRelaunch)
        {
            self.shell.status = "Visual fixture mode · elevation is disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::RestartAsAdmin) {
            DispatchOutcome::Accepted { .. } => {
                self.shell.status = "Restarting with administrator rights…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }
}
