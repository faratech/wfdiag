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
    provider_display_name, provider_from_wire, scan_complete_text, scan_kind_label,
    scan_progress_text, scan_result_text, subscription_auth_state_index,
    subscription_provider_from_wire, worker_timeout_text,
};
use crate::app::state::{
    ChatDisplayMessage, ChatDisplayRole, CloudFallbackConsent, DiagnosticAnalysisDisplay,
    FullScanConsent, HistoryTaskDiffProjection, IssuePrioritizationDisplay, Page,
};
use crate::platform::window;
use wfdiag_app::{
    ActionEvent, AnalysisEvent, AppCommand, AppEvent, ChatEvent, ExportEvent, FixPlanEvent,
    HistoryEvent, HistoryRequest, IssuesEvent, ModelCatalogEvent, MonitorEvent,
    PrioritizationEvent, ProviderEvent, ReportEvent, ScanEvent, SettingsEvent, SubscriptionEvent,
    SystemEvent, UpdateEvent, WorkerKind,
};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_issues::projection::project_issues;
use wfdiag_native_projection::process_identity::{ProcessIdentity, reconcile_process_selection_by};
use wfdiag_native_update::UpdateOutcome;
use wfdiag_native_update::policy::trusted_release_url;
use windows_reactor::*;

/// How many rendered chat bubbles are kept.
const MAX_CHAT_DISPLAY_MESSAGES: usize = 200;

impl WfdiagShell {
    /// Copy the engine's read model into view state, then apply the batch.
    pub(crate) fn apply_app_events(
        &mut self,
        events: Vec<AppEvent>,
        context: &ComponentContext<Self>,
    ) {
        self.sync_from_snapshot();
        for event in events {
            match event {
                AppEvent::Started { .. } | AppEvent::Terminated => {}
                AppEvent::Scan(event) => self.apply_scan_event(event),
                AppEvent::Issues(event) => self.apply_issues_event(event),
                AppEvent::History(event) => self.apply_history_event(event),
                AppEvent::Monitor(event) => self.apply_monitor_event(event, context),
                AppEvent::Provider(event) => self.apply_provider_event(event),
                AppEvent::Settings(event) => self.apply_settings_event(event),
                AppEvent::Export(event) => self.apply_export_event(event, context),
                AppEvent::Update(event) => self.apply_update_event(event, context),
                AppEvent::System(event) => self.apply_system_event(event, context),
                AppEvent::Chat(event) => self.apply_chat_event(event),
                AppEvent::Report(event) => self.apply_report_event(event),
                AppEvent::Analysis(event) => self.apply_analysis_event(event),
                AppEvent::Prioritization(event) => self.apply_prioritization_event(&event),
                AppEvent::FixPlan(event) => self.apply_fix_plan_event(&event),
                AppEvent::Action(event) => self.apply_action_event(event),
                AppEvent::WorkerStopped { worker, panicked } => {
                    self.apply_worker_stopped(worker, panicked);
                }
                // #195: the three `blocking_recv()` waits are gone with the
                // receivers. A reply that never arrives is now a typed
                // timeout that clears its domain instead of parking a thread.
                AppEvent::ReplyTimedOut { worker, .. } => {
                    self.clear_pending_for(worker);
                    self.status = worker_timeout_text(worker.as_str());
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
            self.settings_snapshot = snapshot.settings.clone();
            self.settings_loading = snapshot.settings_loading;
            self.settings_error = snapshot.settings_error.clone();

            // ---- host identity ----------------------------------------
            if let Some(info) = snapshot.system_info.as_ref() {
                self.is_admin = info.is_admin;
                self.system_info = info.clone();
            }
            self.architecture = snapshot.architecture.clone();
            self.system_error = snapshot.system_error.clone();

            // ---- scan --------------------------------------------------
            self.diagnostic_catalog.clone_from(&snapshot.catalog);
            self.diagnostic_results.clone_from(&snapshot.scan.results);
            self.diagnostic_expected_task_ids
                .clone_from(&snapshot.scan.task_ids);
            self.diagnostic_scan_kind = snapshot.scan.scan_kind;
            self.diagnostic_session_id = snapshot.scan.session_id.clone();
            self.diagnostic_duration_ms = snapshot.scan.duration_ms;
            self.diagnostic_total = snapshot.scan.total;
            self.diagnostic_completed = snapshot.scan.completed;
            self.diagnostic_errors = snapshot.scan.errors;
            self.scan_phase = snapshot.scan_phase;
            // A running scan seeds every requested task as Queued; live
            // transitions arrive as `ScanEvent::Progress`.
            for task_id in &snapshot.scan.task_ids {
                self.diagnostic_task_statuses
                    .entry(task_id.clone())
                    .or_insert(wfdiag_ui_core::TaskProgressStatus::Queued);
            }

            // ---- issues -------------------------------------------------
            self.issues.clone_from(&snapshot.issues);
            self.issue_error = snapshot.issue_error.clone();
            self.issue_maintenance = snapshot.maintenance_remediations();

            // ---- history ------------------------------------------------
            self.history_summaries
                .clone_from(&snapshot.history.summaries);
            self.history_loading = snapshot.history.loading;
            self.history_comparison = snapshot.history.comparison_summary.clone();
            self.history_trends =
                (!snapshot.history.trends.is_empty()).then(|| snapshot.history.trends.clone());

            // ---- monitoring ---------------------------------------------
            self.monitoring_paused = snapshot.monitor.paused;
            self.process_page = snapshot.monitor.process_page.clone();
            self.network_connections = snapshot.monitor.connections.clone();
            if let Some(error) = snapshot.monitor.error.as_ref() {
                self.monitor_error = Some(error.clone());
            }

            // ---- providers -----------------------------------------------
            self.ai_provider_status = snapshot.provider_status.clone();
            self.ai_status_loading = snapshot.provider_loading;
            for (index, state) in self.provider_catalogs.iter_mut().enumerate() {
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
                        .subscription_auth_states
                        .get_mut(subscription_auth_state_index(provider))
                {
                    state.clone_from(account);
                }
            }
            self.subscription_install_prompt = snapshot.provider_setup.install_prompt;
            self.subscription_install_progress = snapshot.provider_setup.install_progress;
            self.subscription_install_error = snapshot.provider_setup.install_error.clone();

            // ---- AI -------------------------------------------------------
            self.chat_streaming = snapshot.ai.chat.streaming;
            self.cloud_fallback_consent =
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
            self.report_text = snapshot.ai.report.text.clone();
            self.report_provider = snapshot.ai.report.provider.clone();
            self.report_provider_use = snapshot.ai.report.provider_use.clone();
            self.report_generating = snapshot.ai.report.generating;
            self.report_error = snapshot.ai.report.error.clone();
            self.diagnostic_analyses = snapshot
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
            self.issue_prioritization = IssuePrioritizationDisplay {
                text: snapshot.ai.prioritization.text.clone(),
                provider_use: snapshot.ai.prioritization.provider_use.clone(),
                cached: snapshot.ai.prioritization.cached,
                error: snapshot.ai.prioritization.error.clone(),
                busy: snapshot.ai.prioritization.busy,
            };
            self.fix_plan = snapshot.ai.fix_plan.plan.clone();
            self.fix_plan_busy = snapshot.ai.fix_plan.busy;
            self.fix_plan_error = snapshot.ai.fix_plan.error.clone();
            self.pending_ai_intent = snapshot.ai.pending_intent.clone();
            self.pending_ai_preparation_error = snapshot.ai.preparation_error.clone();

            // ---- remediation ----------------------------------------------
            self.action_review = snapshot.actions.review.clone();
            self.repair_confirm = snapshot.actions.repair_confirmation.clone();
            self.action_active_run = snapshot.actions.active_run.clone();
            self.action_run_history
                .clone_from(&snapshot.actions.history);
            self.action_busy = snapshot.actions.active_run.is_some();

            // ---- updates ---------------------------------------------------
            if let Some(update) = snapshot.update.available.as_ref() {
                self.update_info = Some(update.clone());
            }

            // Runner failures the engine recorded at start, surfaced where the
            // corresponding surface looks for them.
            self.provider_setup_error = snapshot
                .worker_error(WorkerKind::Provider)
                .map(str::to_string);
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
            .action_active_run
            .iter()
            .chain(self.action_run_history.iter())
            .map(|run| run.run_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let keep = self
            .action_expanded_runs
            .iter()
            .filter(|expanded| visible.contains(expanded.as_str()))
            .cloned()
            .collect();
        self.action_expanded_runs = keep;
    }

    /// Clear the pending flag a timed-out worker owned.
    fn clear_pending_for(&mut self, worker: WorkerKind) {
        match worker {
            WorkerKind::History => {
                self.history_comparing = false;
                self.history_task_diff_loading = false;
                self.history_trends_loading = false;
                self.history_ack_busy = false;
            }
            WorkerKind::Monitor => {
                self.process_loading = false;
                self.network_loading = false;
            }
            WorkerKind::Issues => self.issue_refreshing = false,
            WorkerKind::Settings => self.settings_saving = false,
            _ => {}
        }
    }

    fn apply_worker_stopped(&mut self, worker: WorkerKind, panicked: bool) {
        self.clear_pending_for(worker);
        let detail = if panicked { " unexpectedly" } else { "" };
        self.status = format!("The {} worker stopped{detail}", worker.as_str());
    }

    // ---- scanning --------------------------------------------------------

    fn scan_label(&self) -> &'static str {
        self.diagnostic_scan_kind.map_or("Scan", scan_kind_label)
    }

    fn apply_scan_event(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Started { kind, .. } => {
                self.diagnostic_task_statuses.clear();
                self.diagnostic_current_task = None;
                self.status = format!("{} started", scan_kind_label(kind));
            }
            ScanEvent::StartFailed { error } => {
                self.status = format!("Could not start diagnostics · {error}");
            }
            ScanEvent::Progress {
                task_id,
                status,
                task_name,
                completed,
                total,
            } => {
                let task_name = task_name.unwrap_or_else(|| {
                    self.diagnostic_catalog
                        .iter()
                        .find(|task| task.id == task_id)
                        .map_or_else(|| task_id.clone(), |task| task.name.clone())
                });
                self.diagnostic_task_statuses.insert(task_id, status);
                if status == wfdiag_ui_core::TaskProgressStatus::Running {
                    self.diagnostic_current_task = Some(task_name.clone());
                }
                self.status = scan_progress_text(
                    self.scan_label(),
                    self.scan_cancelling(),
                    completed,
                    total,
                    &task_name,
                );
            }
            ScanEvent::TaskResult {
                completed, errors, ..
            } => {
                self.status = scan_result_text(
                    self.scan_label(),
                    self.scan_busy(),
                    completed,
                    self.diagnostic_total,
                    errors,
                );
            }
            ScanEvent::Committed {
                completed,
                errors,
                auto_save,
                ..
            } => {
                // Committing evidence re-runs detection inside the engine.
                self.issue_refreshing = true;
                self.status = if auto_save {
                    format!("Finalizing {}…", self.scan_label().to_ascii_lowercase())
                } else {
                    scan_complete_text(self.scan_label(), completed, errors, false)
                };
            }
            ScanEvent::TargetedCommitted { .. } => {
                self.issue_refreshing = true;
                self.status = format!(
                    "Diagnostic rerun complete · {} collected · {} errors",
                    self.diagnostic_completed, self.diagnostic_errors
                );
            }
            ScanEvent::TargetedFailed { error } => {
                self.status = format!("Targeted Scan failed · {error} · previous results restored");
            }
            ScanEvent::Cancelled => {
                self.diagnostic_current_task = None;
                self.status = format!("{} stopped · previous results restored", self.scan_label());
            }
            ScanEvent::CancelAcknowledged => {
                self.status = format!(
                    "Stopping {} after in-flight checks finish…",
                    self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
                );
            }
            ScanEvent::CancelFailed { error } => {
                self.status = format!("Could not stop the scan · {error}");
            }
            ScanEvent::Failed { error, stopped } => {
                self.diagnostic_current_task = None;
                let label = self.scan_label();
                self.status = if stopped {
                    format!("{label} stopped · previous results restored")
                } else {
                    format!("{label} failed · {error} · previous results restored")
                };
            }
            ScanEvent::Incomplete {
                completed,
                expected,
            } => {
                self.status = format!(
                    "{} returned an invalid result set ({completed} results for {expected} expected checks) · previous results restored",
                    self.scan_label()
                );
            }
            ScanEvent::Finalized { history, .. } => {
                self.diagnostic_current_task = None;
                self.notify_scan_completion();
                let label = self.scan_label();
                match history {
                    Some(Err(error)) => {
                        self.history_error = Some(format!("Scan history was not saved: {error}"));
                        self.status = scan_complete_text(
                            label,
                            self.diagnostic_completed,
                            self.diagnostic_errors,
                            true,
                        );
                    }
                    other => {
                        if other.is_some() {
                            self.history_error = None;
                            if self.page == Page::History {
                                self.request_history_list();
                            }
                        }
                        self.status = scan_complete_text(
                            label,
                            self.diagnostic_completed,
                            self.diagnostic_errors,
                            false,
                        );
                    }
                }
            }
        }
    }

    // ---- issues -----------------------------------------------------------

    fn apply_issues_event(&mut self, event: IssuesEvent) {
        match event {
            IssuesEvent::Updated { session_id, .. } => {
                self.issue_refreshing = false;
                self.issue_projected_session_id = Some(session_id);
                if self.page == Page::Issues && !self.scan_busy() {
                    self.status = project_issues(&self.issues).counts.summary_text();
                }
            }
            IssuesEvent::Failed { error } => {
                self.issue_refreshing = false;
                if self.page == Page::Issues {
                    self.status = error;
                }
            }
        }
    }

    // ---- history ----------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn apply_history_event(&mut self, event: HistoryEvent) {
        match event {
            HistoryEvent::Listed { scans } => {
                let latest_id = scans.first().map(|scan| scan.id.clone());
                let baseline_changed = crate::app::policy::history_trends_baseline_changed(
                    self.history_trends_baseline_id.as_deref(),
                    latest_id.as_deref(),
                );
                // A newer scan at the top changes what "compare with latest"
                // means, so the open comparison is re-requested.
                let comparison_refresh_target =
                    crate::app::policy::history_comparison_refresh_target(
                        self.history_comparison
                            .as_ref()
                            .map(|comparison| comparison.current_scan.id.as_str()),
                        &scans,
                        self.selected_history_id.as_deref(),
                    );
                if self
                    .selected_history_id
                    .as_ref()
                    .is_some_and(|selected| !scans.iter().any(|scan| &scan.id == selected))
                {
                    self.selected_history_id = None;
                    self.history_comparison = None;
                    self.clear_history_task_diff();
                    self.history_label_draft.clear();
                    self.history_label_editing = false;
                    self.history_tag_draft.clear();
                }
                self.status = format!("History loaded · {} scans", scans.len());
                if baseline_changed {
                    self.invalidate_history_trends();
                    if latest_id.is_some() {
                        self.request_history_trends();
                    }
                }
                if let Some(selected) = comparison_refresh_target {
                    self.request_history_comparison(selected);
                } else if !self.history_label_editing
                    && let Some(selected) = self.selected_history_id.clone()
                {
                    self.history_label_draft =
                        crate::app::policy::history_label_draft_for_selection(
                            &self.history_summaries,
                            &selected,
                        );
                }
            }
            HistoryEvent::ComparedSummary { comparison } => {
                self.history_comparing = false;
                self.history_comparison_error = None;
                self.status = format!("History comparison · {} changes", comparison.total_changes);
            }
            HistoryEvent::TaskDiff { diff } => {
                self.history_task_diff_loading = false;
                self.history_task_diff = Some(HistoryTaskDiffProjection::from(*diff));
                self.history_task_diff_error = None;
                self.status = "Stored task details loaded".to_string();
            }
            HistoryEvent::Trends { trends } => {
                // An empty answer is a real answer: it renders "no trends yet"
                // rather than staying on the spinner.
                self.history_trends_loading = false;
                self.history_trends_error = None;
                self.history_trends = Some(trends);
            }
            HistoryEvent::LabelSaved { .. } => {
                self.history_ack_busy = false;
                self.history_label_editing = false;
                self.status = if self.history_label_draft.trim().is_empty() {
                    "Label removed".to_string()
                } else {
                    "Label saved".to_string()
                };
                self.request_history_list();
            }
            HistoryEvent::TagsSaved { .. } => {
                self.history_ack_busy = false;
                self.status = "Tags saved".to_string();
                self.request_history_list();
            }
            HistoryEvent::Cleared => {
                self.history_ack_busy = false;
                self.selected_history_id = None;
                self.history_comparison = None;
                self.clear_history_task_diff();
                self.invalidate_history_trends();
                self.history_label_draft.clear();
                self.history_label_editing = false;
                self.history_tag_draft.clear();
                self.status = "Scan history cleared".to_string();
            }
            HistoryEvent::Failed { request, error } => match request {
                HistoryRequest::List => {
                    self.history_error = Some(error.clone());
                    self.status = format!("Could not load history · {error}");
                }
                HistoryRequest::Compare | HistoryRequest::CompareToLatest => {
                    self.history_comparing = false;
                    self.history_comparison = None;
                    self.history_comparison_error = Some(error.clone());
                    self.status = format!("Could not compare history · {error}");
                }
                HistoryRequest::TaskDiff => {
                    self.history_task_diff_loading = false;
                    self.history_task_diff = None;
                    self.history_task_diff_error = Some(error.clone());
                    self.status = format!("Could not load stored task details · {error}");
                }
                HistoryRequest::Trends => {
                    self.history_trends_loading = false;
                    self.history_trends_error = Some(error);
                }
                HistoryRequest::Label => {
                    self.history_ack_busy = false;
                    self.status = format!("Could not save label · {error}");
                }
                HistoryRequest::Tags => {
                    self.history_ack_busy = false;
                    self.status = format!("Could not save tags · {error}");
                }
                HistoryRequest::Clear => {
                    self.history_ack_busy = false;
                    self.status = format!("Could not clear history · {error}");
                }
                HistoryRequest::AutoSave | HistoryRequest::Load => {
                    self.history_error = Some(error);
                }
            },
            _ => {}
        }
    }

    // ---- monitoring --------------------------------------------------------

    fn apply_monitor_event(&mut self, event: MonitorEvent, context: &ComponentContext<Self>) {
        match event {
            MonitorEvent::Stats(stats) => {
                self.monitor_error = None;
                if !self.monitoring_paused && self.page.consumes_live_telemetry() {
                    self.status = format!(
                        "Live sample · CPU {:.0}% · memory {:.0}%",
                        stats.cpu_utilization, stats.memory_utilization
                    );
                }
                self.monitor_history.push_stats(&stats);
                self.latest_system_stats = Some(*stats);
                // The process table rides the telemetry tick rather than
                // running a timer of its own, at half the sample rate.
                let due = self.process_last_refresh_started_at.is_none_or(|last| {
                    std::time::Instant::now().duration_since(last)
                        >= crate::app::consts::PROCESS_LIVE_REFRESH_INTERVAL
                });
                if due
                    && !self.monitoring_paused
                    && !self.process_loading
                    && self.page == Page::Processes
                {
                    self.request_process_page(context, false);
                }
            }
            MonitorEvent::ProcessPage(page) => {
                self.process_loading = false;
                self.process_error = None;
                self.process_offset = page.offset;
                self.selected_process =
                    reconcile_process_selection_by(self.selected_process, &page.items, |row| {
                        ProcessIdentity::new(row.pid, row.start_time)
                    });
                self.status = format!(
                    "Process inventory · {} of {} shown",
                    page.items.len(),
                    page.total
                );
            }
            MonitorEvent::ProcessPageSuperseded => self.process_loading = false,
            MonitorEvent::NetworkConnections(_) => self.network_loading = false,
            MonitorEvent::PausedChanged { .. } => {}
            MonitorEvent::Unavailable { reason } => {
                self.process_loading = false;
                self.network_loading = false;
                self.process_error = Some(reason.clone());
                self.monitor_error = Some(reason.clone());
                self.status = reason;
            }
        }
    }

    // ---- providers -----------------------------------------------------------

    fn apply_provider_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Status(status) | ProviderEvent::PreferenceApplied { status, .. } => {
                self.ai_status_error = None;
                if self.page == Page::Ai {
                    self.status = if status.active_provider == AIProvider::None {
                        "AI provider check complete · no provider is ready".to_string()
                    } else {
                        format!("AI provider ready · {}", status.active_provider)
                    };
                }
            }
            ProviderEvent::PreferenceRejected { reason } => {
                self.settings_save_error = Some(reason);
                self.status = "Phi Silica preference was not changed".to_string();
            }
            ProviderEvent::CacheCleared | ProviderEvent::OllamaModels(_) => {}
            ProviderEvent::ModelCatalog(event) => self.apply_model_catalog_event(event),
            ProviderEvent::Subscription(event) => self.apply_subscription_event(*event),
            ProviderEvent::Failed { error } => {
                self.ai_status_error = Some(error.clone());
                if self.page == Page::Ai {
                    self.status = format!("AI provider check failed · {error}");
                }
            }
        }
    }

    fn apply_model_catalog_event(&mut self, event: ModelCatalogEvent) {
        match event {
            ModelCatalogEvent::Started { provider } => {
                self.status = format!(
                    "Loading {} models…",
                    provider_display_name(provider_from_wire(&provider))
                );
            }
            ModelCatalogEvent::Loaded { provider, catalog } => {
                let count = catalog.models.len();
                self.status = format!(
                    "Loaded {count} {} model{}",
                    provider_display_name(provider_from_wire(&provider)),
                    if count == 1 { "" } else { "s" }
                );
            }
            ModelCatalogEvent::Failed { error, .. } => {
                self.status = format!("Model discovery failed · {error}");
            }
            ModelCatalogEvent::Throttled { .. } | ModelCatalogEvent::Cancelled { .. } => {}
            _ => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_subscription_event(&mut self, event: SubscriptionEvent) {
        use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthState;
        match event {
            SubscriptionEvent::Started { .. } | SubscriptionEvent::InstallStarted { .. } => {}
            SubscriptionEvent::Status { status } | SubscriptionEvent::Completed { status, .. } => {
                self.status = match status.state {
                    SubscriptionAuthState::NotInstalled => {
                        format!("{} CLI was not detected", status.provider)
                    }
                    SubscriptionAuthState::SignedOut => {
                        format!("{} is installed · sign-in required", status.provider)
                    }
                    SubscriptionAuthState::SignedIn => {
                        format!("{} account is signed in", status.provider)
                    }
                    SubscriptionAuthState::Unknown => {
                        format!("{} account status could not be confirmed", status.provider)
                    }
                };
            }
            SubscriptionEvent::Failed {
                provider, error, ..
            } => {
                self.subscription_auth_error = Some(error.clone());
                self.status = format!("{provider} account action failed · {error}");
            }
            SubscriptionEvent::Cancelled { provider, .. } => {
                if self.settings_open {
                    self.status = format!("{provider} account action cancelled");
                }
            }
            SubscriptionEvent::InstallProgress { progress } => {
                self.subscription_install_busy = true;
                self.status =
                    crate::app::policy::subscription_install_progress_label(*progress).to_string();
            }
            SubscriptionEvent::InstallFallbackRequired { .. } => {
                self.subscription_install_busy = false;
                self.status =
                    "winget could not finish · vendor installer approval required".to_string();
            }
            SubscriptionEvent::Installed { status } => {
                self.subscription_install_busy = false;
                if self.settings_open {
                    let path = Some(status.path.to_string_lossy().into_owned());
                    match status.provider {
                        wfdiag_native_ai_chat::SubscriptionAuthProvider::Codex => {
                            self.settings_draft.codex_cli_path = path;
                        }
                        wfdiag_native_ai_chat::SubscriptionAuthProvider::ClaudeCode => {
                            self.settings_draft.claude_cli_path = path;
                        }
                    }
                    self.request_provider_model_refresh(false);
                }
                self.status = format!(
                    "{} CLI installed · account sign-in was not started",
                    status.provider
                );
            }
            SubscriptionEvent::InstallFailed {
                provider, error, ..
            } => {
                self.subscription_install_busy = false;
                self.status = format!("{provider} CLI installation failed · {error}");
            }
            SubscriptionEvent::InstallCancelled { provider, .. } => {
                self.subscription_install_busy = false;
                self.status = format!("{provider} CLI installation cancelled");
            }
            _ => {}
        }
    }

    // ---- settings -----------------------------------------------------------

    fn apply_settings_event(&mut self, event: SettingsEvent) {
        match event {
            SettingsEvent::Loaded { settings } => {
                self.adopt_persisted_settings(&settings, true);
            }
            SettingsEvent::Saved { settings } => {
                self.settings_saving = false;
                let closes_dialog = self
                    .settings_save_epoch
                    .take()
                    .is_some_and(|epoch| self.settings_dialog_is_current(epoch));
                self.adopt_persisted_settings(&settings, closes_dialog || !self.settings_open);
                self.settings_save_error = None;
                if closes_dialog {
                    self.settings_open = false;
                    self.cancel_provider_model_request();
                    self.cancel_subscription_auth();
                }
                self.status = "Settings saved".to_string();
            }
            SettingsEvent::Updated { settings } => {
                self.settings_snapshot.cloud_fallback_policy = settings.cloud_fallback_policy;
                self.settings_draft.cloud_fallback_policy = settings.cloud_fallback_policy;
            }
            SettingsEvent::CredentialsCommitted => {}
            SettingsEvent::Failed { error } => {
                self.settings_saving = false;
                self.settings_save_epoch = None;
                window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                self.settings_save_error = Some(error);
                self.status = "Settings were not saved".to_string();
            }
        }
    }

    // ---- export --------------------------------------------------------------

    fn apply_export_event(&mut self, event: ExportEvent, context: &ComponentContext<Self>) {
        match event {
            ExportEvent::Completed { payload } => self.deliver_export_payload(*payload, context),
            ExportEvent::Failed { error } => {
                self.export_pending = None;
                self.export_error = Some(error);
                self.status = "Failed to prepare share. Please try again.".to_string();
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
        self.update_info = Some(update);
        self.show_update_notice(context);
    }

    // ---- host identity ---------------------------------------------------------

    fn apply_system_event(&mut self, event: SystemEvent, context: &ComponentContext<Self>) {
        match event {
            SystemEvent::Info(_) | SystemEvent::Architecture(_) => {}
            SystemEvent::ElevationAttempted { restarted } => {
                if restarted {
                    self.status = "Relaunching with administrator rights…".to_string();
                    // The elevated child waits on the process-owned
                    // single-instance mutex. Close this copy for real (not to
                    // tray) so that child can become the new primary.
                    window::request_forced_close();
                    if !context.window().request_close() {
                        window::cancel_forced_close();
                        self.status = "The elevated copy launched, but this window could not close · exit the current copy and try again"
                            .to_string();
                    }
                } else {
                    // Dismissed UAC prompt — keep running, no error.
                    self.status = "Administrator relaunch was cancelled".to_string();
                }
            }
            SystemEvent::Failed { error } => {
                self.status = format!("Could not read native system identity · {error}");
            }
        }
    }

    // ---- chat --------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn apply_chat_event(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Started { provider } => {
                if !self.chat_turn_open {
                    self.chat_turn_open = true;
                    let prompt = self.chat_last_prompt.clone().unwrap_or_default();
                    self.push_chat_turn(prompt);
                }
                self.status = format!(
                    "Asking the AI assistant · {}…",
                    provider_display_name(provider_from_wire(&provider))
                );
            }
            ChatEvent::Deferred { reason } => self.status = reason,
            ChatEvent::Delta { text } => {
                self.chat_answer
                    .get_or_insert_with(String::new)
                    .push_str(&text);
                let turn = self.chat_turn;
                if let Some(message) = self.chat_assistant_message_mut(turn) {
                    message.text.push_str(&text);
                }
            }
            ChatEvent::ToolActivity { activity, history } => {
                let summary = activity.compatibility_summary();
                let turn = self.chat_turn;
                if let Some(message) = self.chat_assistant_message_mut(turn) {
                    message.tools = *history;
                }
                self.status = summary;
            }
            ChatEvent::ProposalStaged {
                remediation_id,
                issue_id,
            } => {
                let turn = self.chat_turn;
                let label = format!(
                    "{remediation_id}{}",
                    issue_id
                        .as_deref()
                        .map_or_else(String::new, |issue| format!(" for {issue}"))
                );
                if let Some(message) = self.chat_assistant_message_mut(turn) {
                    message.proposals.push(label);
                }
                // The engine staged nothing: it reported that the model asked.
                // The normal prepare/approve flow still runs from here.
                self.prepare_remediation(remediation_id, issue_id);
            }
            ChatEvent::FullScanRequested {
                source_scan_id,
                reason,
            } => {
                self.full_scan_consent = Some(FullScanConsent {
                    source_scan_id,
                    reason,
                    original_prompt: self.chat_last_prompt.clone().unwrap_or_default(),
                });
                self.status =
                    "The AI assistant requested a Full Scan for more evidence".to_string();
            }
            ChatEvent::CloudFallbackRequired { .. } => {
                self.status = "Local AI was unavailable · cloud permission required".to_string();
            }
            ChatEvent::Done {
                provider,
                provider_use,
                finish_reason,
                tool_history,
            } => {
                let notice = crate::app::policy::chat_completion_notice(&finish_reason);
                let turn = self.chat_turn;
                if let Some(message) = self.chat_assistant_message_mut(turn) {
                    message.provider_use = Some(*provider_use);
                    message.finish_reason = Some(finish_reason);
                    message.terminal_message = notice.map(str::to_string);
                    message.tools = *tool_history;
                }
                self.chat_turn_open = false;
                self.status = notice.map_or_else(
                    || format!("AI response complete · {provider}"),
                    |notice| format!("AI response complete · {provider} · {notice}"),
                );
            }
            ChatEvent::Failed { message } => {
                self.chat_turn_open = false;
                let turn = self.chat_turn;
                if let Some(display) = self.chat_assistant_message_mut(turn) {
                    display.finish_reason = Some("error".to_string());
                    display.terminal_message = Some(message.clone());
                }
                self.status = message;
            }
            ChatEvent::Cancelled => {
                self.chat_turn_open = false;
                let turn = self.chat_turn;
                if let Some(display) = self.chat_assistant_message_mut(turn) {
                    display.finish_reason = Some("cancelled".to_string());
                    display.terminal_message = Some("Response cancelled".to_string());
                }
                self.status = "AI response cancelled".to_string();
            }
            ChatEvent::SessionReset => {
                self.chat_turn_open = false;
                self.chat_messages.clear();
                self.chat_answer = None;
                self.status = "New AI conversation started".to_string();
            }
            _ => {}
        }
    }

    /// Append the user and assistant bubbles for one submitted turn.
    pub(crate) fn push_chat_turn(&mut self, prompt: String) {
        self.chat_turn = self.chat_turn.wrapping_add(1);
        let turn = self.chat_turn;
        self.chat_messages.push(ChatDisplayMessage {
            turn,
            role: ChatDisplayRole::User,
            text: prompt,
            provider_use: None,
            finish_reason: Some("submitted".to_string()),
            terminal_message: None,
            tools: wfdiag_native_ai_chat::ChatToolHistory::default(),
            proposals: Vec::new(),
        });
        self.chat_messages.push(ChatDisplayMessage {
            turn,
            role: ChatDisplayRole::Assistant,
            text: String::new(),
            provider_use: None,
            finish_reason: None,
            terminal_message: None,
            tools: wfdiag_native_ai_chat::ChatToolHistory::default(),
            proposals: Vec::new(),
        });
        let excess = self
            .chat_messages
            .len()
            .saturating_sub(MAX_CHAT_DISPLAY_MESSAGES);
        if excess > 0 {
            self.chat_messages.drain(0..excess);
        }
    }

    fn chat_assistant_message_mut(&mut self, turn: u64) -> Option<&mut ChatDisplayMessage> {
        self.chat_messages
            .iter_mut()
            .rev()
            .find(|message| message.turn == turn && message.role == ChatDisplayRole::Assistant)
    }

    // ---- report ---------------------------------------------------------------------

    fn apply_report_event(&mut self, event: ReportEvent) {
        match event {
            ReportEvent::Started { .. } => self.status = "Preparing AI report…".to_string(),
            ReportEvent::Deferred { reason } => self.status = reason,
            ReportEvent::Delta { .. } => {}
            ReportEvent::Cached { provider, .. } => {
                self.status = format!("AI report ready · {provider} · cached");
            }
            ReportEvent::Done { provider, .. } => {
                self.status = format!("AI report ready · {provider}");
            }
            ReportEvent::Failed { message } => self.status = message,
            ReportEvent::Cancelled => self.status = "AI report cancelled".to_string(),
            ReportEvent::Invalidated => {}
            _ => {}
        }
    }

    // ---- one-shot analysis ------------------------------------------------------------

    fn apply_analysis_event(&mut self, event: AnalysisEvent) {
        match event {
            AnalysisEvent::Started { cached, .. } => {
                self.status = if cached {
                    "Loading the cached diagnostic interpretation…".to_string()
                } else {
                    "The AI provider is interpreting the diagnostic…".to_string()
                };
            }
            AnalysisEvent::Completed {
                cached,
                provider_use,
                ..
            } => {
                let provider = provider_use.provider_id.clone();
                self.status = if cached {
                    format!("Diagnostic interpretation ready · {provider} · cached")
                } else {
                    format!("Diagnostic interpretation ready · {provider}")
                };
            }
            AnalysisEvent::Failed { message, .. } => {
                self.status = format!("Diagnostic interpretation failed · {message}");
            }
            AnalysisEvent::Cancelled { .. } => {
                self.status = "Diagnostic interpretation cancelled".to_string();
            }
            AnalysisEvent::Invalidated => {}
            _ => {}
        }
    }

    fn apply_prioritization_event(&mut self, event: &PrioritizationEvent) {
        match event {
            PrioritizationEvent::Started { .. } => {
                self.status = "The AI provider is prioritizing detected issues…".to_string();
            }
            PrioritizationEvent::Completed { cached, .. } => {
                let provider = self
                    .issue_prioritization
                    .provider_use
                    .as_ref()
                    .map_or_else(|| "AI".to_string(), |use_| use_.provider_id.clone());
                self.status = if *cached {
                    format!("Issue prioritization ready · {provider} · cached")
                } else {
                    format!("Issue prioritization ready · {provider}")
                };
            }
            PrioritizationEvent::Failed { message, .. } => {
                self.status = format!("Issue prioritization failed · {message}");
            }
            PrioritizationEvent::Cancelled => {
                self.status = "Issue prioritization cancelled".to_string();
            }
            PrioritizationEvent::Invalidated => {
                self.status =
                    "Ignored stale issue prioritization after evidence changed".to_string();
            }
            _ => {}
        }
    }

    fn apply_fix_plan_event(&mut self, event: &FixPlanEvent) {
        match event {
            FixPlanEvent::Started { provider } => {
                self.status = format!("Preparing a vetted fix plan with {provider}…");
            }
            FixPlanEvent::Completed { plan } => {
                let entry_count = plan.entries.len();
                self.status = format!(
                    "Fix plan ready · {entry_count} vetted action{} · {}",
                    if entry_count == 1 { "" } else { "s" },
                    plan.provider_use.provider_id
                );
            }
            FixPlanEvent::Failed { message, .. } => self.status = message.clone(),
            FixPlanEvent::Cancelled => self.status = "AI fix plan cancelled".to_string(),
            FixPlanEvent::Invalidated => {}
            _ => {}
        }
    }

    // ---- remediation ---------------------------------------------------------------------

    fn apply_action_event(&mut self, event: ActionEvent) {
        match event {
            ActionEvent::Proposal { proposal } => {
                let action_count = proposal.actions.len();
                self.status = format!(
                    "Review {action_count} vetted remediation action{}",
                    if action_count == 1 { "" } else { "s" }
                );
            }
            ActionEvent::RepairConfirmationRequired { .. } => {
                self.status = "Explicit repair confirmation is required".to_string();
            }
            ActionEvent::Approved { summary, .. } | ActionEvent::Run { summary } => {
                self.action_expanded_runs.insert(summary.run_id.clone());
                self.status = crate::app::policy::action_run_status_text(&summary);
            }
            ActionEvent::Summary { summary } => {
                let succeeded = summary
                    .actions
                    .iter()
                    .any(|action| action.result.as_ref().is_some_and(|result| result.success));
                self.status = crate::app::policy::action_run_status_text(&summary);
                if succeeded && !self.deterministic_visual {
                    // The fix may have changed what detection sees. Refresh
                    // the authoritative projection even when the user left the
                    // Issues page while a long repair ran.
                    self.dispatch(AppCommand::RefreshIssues);
                    self.issue_refreshing = true;
                }
            }
            ActionEvent::Rejected { message } => self.status = message,
            ActionEvent::Discarded { .. } => {}
            _ => {}
        }
    }
}
