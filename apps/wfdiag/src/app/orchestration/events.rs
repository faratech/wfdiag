//! Projecting the engine's read model and event stream into view state.
//!
//! # The contract
//!
//! [`wfdiag_app::AppService::drain`] applies every worker reply to
//! [`wfdiag_app::AppSnapshot`] *before* it returns the batch, so by the time
//! this module sees an event the snapshot is already final. That is why
//! [`WfdiagShell::apply_app_events`] copies the snapshot **first** and then
//! walks the batch: the copy is the data, and each event only adds the parts
//! a read model cannot express — the status line, a toast, a focus move, a
//! follow-up command.
//!
//! Nothing here compares a request id, a session id or an epoch. Those guards
//! all live inside the service now; an event that reaches this file is current
//! by construction.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{
    provider_from_wire, scan_complete_text, subscription_auth_state_index,
    subscription_provider_from_wire, worker_timeout_text,
};
use crate::app::state::{
    CloudFallbackConsent, DiagnosticAnalysisDisplay, IssuePrioritizationDisplay, Page,
};
use crate::platform::window;
use wfdiag_app::{AppEvent, ExportEvent, ScanEvent, SystemEvent, UpdateEvent, WorkerKind};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_update::UpdateOutcome;
use wfdiag_native_update::policy::trusted_release_url;
use windows_reactor::*;

impl WfdiagShell {
    /// Copy the engine's read model into view state, then apply the batch.
    pub(crate) fn apply_app_events(
        &mut self,
        events: Vec<AppEvent>,
        context: &ComponentContext<Self>,
    ) {
        self.sync_from_snapshot();
        for event in events {
            // Screens that own their own state see every fact first and take
            // the part that is theirs; what is left below is the shell's own
            // (status text, focus, notifications, navigation).
            self.fan_out_app_event(&event, context);
            match event {
                AppEvent::Started { .. } | AppEvent::Terminated => {}
                AppEvent::Scan(event) => self.apply_scan_event(&event, context),
                AppEvent::Provider(event) => self.apply_provider_event(event),
                AppEvent::Settings(event) => self.apply_settings_event(event),
                AppEvent::Export(event) => self.apply_export_event(event, context),
                AppEvent::Update(event) => self.apply_update_event(event, context),
                AppEvent::System(event) => self.apply_system_event(event, context),
                AppEvent::WorkerStopped { worker, panicked } => {
                    self.apply_worker_stopped(worker, panicked);
                }
                // #195: the three `blocking_recv()` waits are gone with the
                // receivers. A reply that never arrives is now a typed
                // timeout that clears its domain instead of parking a thread.
                AppEvent::ReplyTimedOut { worker, .. } => {
                    self.clear_pending_for(worker);
                    self.shell.status = worker_timeout_text(worker.as_str());
                }
                _ => {}
            }
        }
    }

    /// Copy [`wfdiag_app::AppSnapshot`] into the fields the views read.
    ///
    /// The service is moved out for the duration so the snapshot can be read
    /// while the rest of `self` is written; nothing else may borrow it here.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn sync_from_snapshot(&mut self) {
        let Some(app) = self.app.take() else {
            return;
        };
        {
            let snapshot = app.snapshot();

            // ---- settings ---------------------------------------------
            self.shell.settings = snapshot.settings.clone();
            self.settings.loading = snapshot.settings_loading;
            self.settings.error = snapshot.settings_error.clone();

            // ---- host identity ----------------------------------------
            if let Some(info) = snapshot.system_info.as_ref() {
                self.shell.is_admin = info.is_admin;
                self.shell.system_info = info.clone();
            }
            self.shell.architecture = snapshot.architecture.clone();
            self.shell.system_error = snapshot.system_error.clone();

            // ---- scan --------------------------------------------------
            self.diagnostics.catalog.clone_from(&snapshot.catalog);
            self.diagnostics.results.clone_from(&snapshot.scan.results);
            self.diagnostics
                .expected_task_ids
                .clone_from(&snapshot.scan.task_ids);
            self.diagnostics.scan_kind = snapshot.scan.scan_kind;
            self.diagnostics.duration_ms = snapshot.scan.duration_ms;
            self.diagnostics.total = snapshot.scan.total;
            self.diagnostics.completed = snapshot.scan.completed;
            self.diagnostics.errors = snapshot.scan.errors;
            self.diagnostics.scan_phase = snapshot.scan_phase;
            // A running scan seeds every requested task as Queued; live
            // transitions arrive as `ScanEvent::Progress`.
            for task_id in &snapshot.scan.task_ids {
                self.diagnostics
                    .task_statuses
                    .entry(task_id.clone())
                    .or_insert(wfdiag_ui_core::TaskProgressStatus::Queued);
            }

            // ---- issues -------------------------------------------------
            self.issues.issues.clone_from(&snapshot.issues);
            self.issues.error = snapshot.issue_error.clone();
            self.issues.maintenance = snapshot.maintenance_remediations();

            // ---- history ------------------------------------------------
            self.history
                .summaries
                .clone_from(&snapshot.history.summaries);
            self.history.loading = snapshot.history.loading;
            self.history.comparison = snapshot.history.comparison_summary.clone();
            self.history.trends =
                (!snapshot.history.trends.is_empty()).then(|| snapshot.history.trends.clone());

            // ---- monitoring ---------------------------------------------
            self.monitor.paused = snapshot.monitor.paused;
            // #194: adopting the page here (not in `view`) is what lets an
            // unchanged row keep its `Arc` and be skipped by reconciliation.
            self.processes
                .set_page(snapshot.monitor.process_page.as_ref());
            self.monitor.network_connections = snapshot.monitor.connections.clone();
            if let Some(error) = snapshot.monitor.error.as_ref() {
                self.monitor.error = Some(error.clone());
            }

            // ---- providers -----------------------------------------------
            self.ai.provider_status = snapshot.provider_status.clone();
            self.ai.status_loading = snapshot.provider_loading;
            for (index, state) in self.settings.provider_catalogs.iter_mut().enumerate() {
                if let Some(provider) = crate::app::policy::provider_setup_provider(index)
                    && let Some(engine) =
                        snapshot.provider_setup.catalogs.get(&provider.to_string())
                {
                    state.clone_from(engine);
                }
            }
            for (wire, account) in &snapshot.provider_setup.accounts {
                if let Some(provider) = subscription_provider_from_wire(wire)
                    && let Some(state) = self
                        .settings
                        .subscription_auth_states
                        .get_mut(subscription_auth_state_index(provider))
                {
                    state.clone_from(account);
                }
            }
            self.settings.subscription_install_prompt = snapshot.provider_setup.install_prompt;
            self.settings.subscription_install_progress = snapshot.provider_setup.install_progress;
            self.settings.subscription_install_error =
                snapshot.provider_setup.install_error.clone();

            // ---- AI -------------------------------------------------------
            self.ai.streaming = snapshot.ai.chat.streaming;
            self.ai.cloud_fallback_consent =
                snapshot
                    .ai
                    .chat
                    .cloud_fallback
                    .as_ref()
                    .map(|prompt| CloudFallbackConsent {
                        candidate: provider_from_wire(&prompt.candidate),
                        local_provider: snapshot
                            .ai
                            .chat
                            .provider
                            .as_deref()
                            .map_or(AIProvider::None, provider_from_wire),
                        reason: prompt.reason.clone(),
                        saving: prompt.saving,
                    });
            self.ai.report_text = snapshot.ai.report.text.clone();
            self.ai.report_provider = snapshot.ai.report.provider.clone();
            self.ai.report_provider_use = snapshot.ai.report.provider_use.clone();
            self.ai.report_generating = snapshot.ai.report.generating;
            self.ai.report_error = snapshot.ai.report.error.clone();
            self.diagnostics.analyses = snapshot
                .ai
                .analyses
                .iter()
                .map(|(task_id, analysis)| {
                    (
                        task_id.clone(),
                        DiagnosticAnalysisDisplay {
                            interpretation: analysis.interpretation.clone(),
                            provider_use: analysis.provider_use.clone(),
                            grounding: analysis.grounding.clone(),
                            cached: analysis.cached,
                            error: analysis.error.clone(),
                            busy: analysis.busy,
                        },
                    )
                })
                .collect();
            self.issues.prioritization = IssuePrioritizationDisplay {
                text: snapshot.ai.prioritization.text.clone(),
                provider_use: snapshot.ai.prioritization.provider_use.clone(),
                cached: snapshot.ai.prioritization.cached,
                error: snapshot.ai.prioritization.error.clone(),
                busy: snapshot.ai.prioritization.busy,
            };
            self.issues.fix_plan = snapshot.ai.fix_plan.plan.clone();
            self.issues.fix_plan_busy = snapshot.ai.fix_plan.busy;
            self.issues.fix_plan_error = snapshot.ai.fix_plan.error.clone();
            self.ai.pending_intent = snapshot.ai.pending_intent.clone();
            self.ai.preparation_error = snapshot.ai.preparation_error.clone();

            // ---- remediation ----------------------------------------------
            self.action_review.review = snapshot.actions.review.clone();
            self.action_review.repair_confirm = snapshot.actions.repair_confirmation.clone();
            self.issues.active_run = snapshot.actions.active_run.clone();
            self.issues
                .run_history
                .clone_from(&snapshot.actions.history);
            self.issues.busy = snapshot.actions.active_run.is_some();

            // ---- updates ---------------------------------------------------
            if let Some(update) = snapshot.update.available.as_ref() {
                self.update_notice.info = Some(update.clone());
            }
        }
        self.app = Some(app);
        self.prune_action_expansion();
    }

    /// Keep the Expander set to runs the user can actually see.
    ///
    /// Runs are *opened* on first observation in `apply_action_event`; this
    /// only drops ids the engine no longer reports, so a run the user
    /// deliberately collapsed stays collapsed.
    fn prune_action_expansion(&mut self) {
        let visible = self
            .issues
            .active_run
            .iter()
            .chain(self.issues.run_history.iter())
            .map(|run| run.run_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let keep = self
            .issues
            .expanded_runs
            .iter()
            .filter(|expanded| visible.contains(expanded.as_str()))
            .cloned()
            .collect();
        self.issues.expanded_runs = keep;
    }

    /// Clear the pending flag a timed-out worker owned.
    fn clear_pending_for(&mut self, worker: WorkerKind) {
        match worker {
            WorkerKind::History => {
                self.history.comparing = false;
                self.history.task_diff_loading = false;
                self.history.trends_loading = false;
                self.history.ack_busy = false;
            }
            WorkerKind::Monitor => {
                self.processes.loading = false;
                self.monitor.network_loading = false;
            }
            WorkerKind::Issues => self.issues.refreshing = false,
            WorkerKind::Settings => self.settings.saving = false,
            _ => {}
        }
    }

    fn apply_worker_stopped(&mut self, worker: WorkerKind, panicked: bool) {
        self.clear_pending_for(worker);
        let detail = if panicked { " unexpectedly" } else { "" };
        self.shell.status = format!("The {} worker stopped{detail}", worker.as_str());
    }

    // ---- scanning --------------------------------------------------------

    /// What is left of the scan stream once the Diagnostics screen has taken
    /// its own progress and the Issues screen its re-detection latch:
    /// finalization, which is genuinely the shell's — the completion toast,
    /// the history reload, and the closing status line.
    fn apply_scan_event(&mut self, event: &ScanEvent, context: &ComponentContext<Self>) {
        let ScanEvent::Finalized { history, .. } = event else {
            return;
        };
        self.notify_scan_completion();
        let label = self.diagnostics.scan_label();
        match history {
            Some(Err(error)) => {
                self.history.error = Some(format!("Scan history was not saved: {error}"));
                self.shell.status = scan_complete_text(
                    label,
                    self.diagnostics.completed,
                    self.diagnostics.errors,
                    true,
                );
            }
            other => {
                if other.is_some() {
                    self.history.error = None;
                    if self.shell.page == Page::History {
                        self.request_history_list(context);
                    }
                }
                self.shell.status = scan_complete_text(
                    label,
                    self.diagnostics.completed,
                    self.diagnostics.errors,
                    false,
                );
            }
        }
    }

    // ---- export --------------------------------------------------------------

    fn apply_export_event(&mut self, event: ExportEvent, context: &ComponentContext<Self>) {
        match event {
            ExportEvent::Completed { payload } => self.deliver_export_payload(*payload, context),
            ExportEvent::Failed { error } => {
                self.export.pending = None;
                self.export.error = Some(error);
                self.shell.status = "Failed to prepare share. Please try again.".to_string();
            }
        }
    }

    // ---- updates --------------------------------------------------------------

    fn apply_update_event(&mut self, event: UpdateEvent, context: &ComponentContext<Self>) {
        // Store builds, debug builds, offline hosts, malformed responses and
        // runtime failures all intentionally remain invisible.
        let UpdateEvent::Checked(outcome) = event else {
            return;
        };
        let UpdateOutcome::Available(update) = *outcome else {
            return;
        };
        if trusted_release_url(&update).is_none() {
            return;
        }
        self.update_notice.info = Some(update);
        self.show_update_notice(context);
    }

    // ---- host identity ---------------------------------------------------------

    fn apply_system_event(&mut self, event: SystemEvent, context: &ComponentContext<Self>) {
        match event {
            SystemEvent::Info(_) | SystemEvent::Architecture(_) => {}
            SystemEvent::ElevationAttempted { restarted } => {
                if restarted {
                    self.shell.status = "Relaunching with administrator rights…".to_string();
                    // The elevated child waits on the process-owned
                    // single-instance mutex. Close this copy for real (not to
                    // tray) so that child can become the new primary.
                    window::request_forced_close();
                    if !context.window().request_close() {
                        window::cancel_forced_close();
                        self.shell.status = "The elevated copy launched, but this window could not close · exit the current copy and try again"
                            .to_string();
                    }
                } else {
                    // Dismissed UAC prompt — keep running, no error.
                    self.shell.status = "Administrator relaunch was cancelled".to_string();
                }
            }
            SystemEvent::Failed { error } => {
                self.shell.status = format!("Could not read native system identity · {error}");
            }
        }
    }
}
