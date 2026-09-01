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
use crate::app::consts::{HISTORY_TREND_SCAN_LIMIT, PROCESS_PAGE_SIZE};
use crate::app::policy::{
    provider_catalog_draft, provider_setup_provider, rejection_text, scan_kind_label,
    set_provider_key_configured, subscription_auth_state_index,
};
use crate::app::state::{AiMode, FixPlanActionSelection, Page};
use crate::fixtures::visual::LiveTestFixture;
use crate::platform::notifications;
use wfdiag_app::ports::monitor::ProcessQuery;
use wfdiag_app::{
    AppCommand, DispatchOutcome, ProviderCredentialCommand, RejectReason, SubscriptionOperation,
};
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_remediation::broker::ActionRequest;
use wfdiag_native_remediation::remediation;
use wfdiag_native_settings::ProviderKeyId;
use windows_reactor::*;

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
            self.status = rejection_text(reason);
        }
    }

    /// Whether a scan occupies the engine.
    pub(crate) const fn scan_busy(&self) -> bool {
        !matches!(self.scan_phase, wfdiag_app::domain::scan::ScanPhase::Idle)
    }

    /// The name the views use for the same fact.
    pub(crate) const fn diagnostics_busy(&self) -> bool {
        self.scan_busy()
    }

    /// Whether the scan is winding down after a Stop.
    pub(crate) const fn scan_cancelling(&self) -> bool {
        matches!(
            self.scan_phase,
            wfdiag_app::domain::scan::ScanPhase::Cancelling
        )
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
        if self.deterministic_visual || !self.settings_snapshot.show_notifications {
            return;
        }
        let collected = self.diagnostic_results.len();
        let errors = self
            .diagnostic_results
            .iter()
            .filter(|result| !result.success)
            .count();
        if let Err(error) = notifications::request_scan_complete_toast(collected, errors) {
            self.report_notification_failure(error);
        }
    }

    /// Surface at most one notification failure for the session.
    pub(crate) fn report_notification_failure(&mut self, error: String) {
        if self.notification_failure_reported {
            return;
        }
        self.notification_failure_reported = true;
        self.status = format!("Notification not shown · {error}");
    }

    pub(crate) fn begin_diagnostic_scan(&mut self, scan_kind: ScanKind) {
        if self.deterministic_visual {
            // Screenshot fixtures must never launch WMI, commands, or mutate
            // the captured Store 2.5.8 state.
            self.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::StartScan { kind: scan_kind }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = format!("Starting {}…", scan_kind_label(scan_kind));
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_targeted_diagnostic_scan(&mut self, task_id: &str) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        let Some(task) = self
            .diagnostic_catalog
            .iter()
            .find(|task| task.id == task_id)
        else {
            self.status = "That diagnostic task is no longer available".to_string();
            return;
        };
        if task.admin_required && !self.is_admin {
            self.status = format!("{} requires administrator access", task.name);
            return;
        }
        let task_ids = vec![task.id.clone()];
        match self.dispatch(AppCommand::StartTargetedScan { task_ids }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = format!("Starting {}…", scan_kind_label(ScanKind::Targeted));
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

    pub(crate) fn request_issue_refresh(&mut self) -> bool {
        if self.deterministic_visual {
            return false;
        }
        let accepted = self.dispatch(AppCommand::RefreshIssues).is_accepted();
        self.issue_refreshing = accepted;
        accepted
    }

    // ---- history -----------------------------------------------------------

    pub(crate) fn request_history_list(&mut self) {
        if self.deterministic_visual {
            return;
        }
        let outcome = self.dispatch(AppCommand::ListHistory);
        if let Some(reason) = outcome.rejection() {
            self.history_error = Some(rejection_text(reason));
        }
    }

    pub(crate) fn select_history_scan(&mut self, scan_id: String) {
        self.history_label_draft = crate::app::policy::history_label_draft_for_selection(
            &self.history_summaries,
            &scan_id,
        );
        self.history_label_editing = false;
        self.history_tag_draft =
            crate::app::policy::history_tag_draft_for_selection(&self.history_summaries, &scan_id);
        self.selected_history_id = Some(scan_id.clone());
        self.request_history_comparison(scan_id);
    }

    pub(crate) fn request_history_comparison(&mut self, selected_id: String) {
        self.clear_history_task_diff();
        self.history_comparison = None;
        self.history_comparison_error = None;
        let Some(latest_id) = self.history_summaries.first().map(|scan| scan.id.clone()) else {
            self.selected_history_id = None;
            self.status = "No history comparison is available".to_string();
            return;
        };
        if selected_id == latest_id {
            self.status = "Latest scan is the comparison baseline".to_string();
            return;
        }
        match self.dispatch(AppCommand::CompareHistory {
            current_id: latest_id,
            previous_id: selected_id,
            summary_only: true,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.history_comparing = true;
                self.status = "Comparing the selected scan with latest…".to_string();
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.history_comparison_error = Some(rejection_text(reason));
                }
            }
        }
    }

    pub(crate) fn clear_history_task_diff(&mut self) {
        self.history_expanded_task_id = None;
        self.history_task_diff = None;
        self.history_task_diff_error = None;
        self.history_task_diff_loading = false;
    }

    pub(crate) fn toggle_history_task_detail(&mut self, task_id: String) {
        if self.history_expanded_task_id.as_deref() == Some(task_id.as_str()) {
            self.clear_history_task_diff();
            return;
        }
        let Some(comparison) = self.history_comparison.as_ref() else {
            return;
        };
        if !crate::app::policy::history_change_rows(comparison)
            .iter()
            .any(|(_, change)| change.task_id == task_id)
        {
            return;
        }
        let current_id = comparison.current_scan.id.clone();
        let previous_id = comparison.previous_scan.id.clone();
        self.clear_history_task_diff();
        self.history_expanded_task_id = Some(task_id.clone());
        match self.dispatch(AppCommand::HistoryTaskDiff {
            current_id,
            previous_id,
            task_id,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.history_task_diff_loading = true;
                self.status = "Loading stored task details…".to_string();
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.history_task_diff_error = Some(message.clone());
                    self.status = format!("Could not load stored task details · {message}");
                }
            }
        }
    }

    pub(crate) fn invalidate_history_trends(&mut self) {
        self.history_trends = None;
        self.history_trends_loading = false;
        self.history_trends_error = None;
        self.history_trends_baseline_id = None;
    }

    pub(crate) fn request_history_trends(&mut self) {
        if self.deterministic_visual || self.history_trends_loading {
            return;
        }
        self.history_trends_baseline_id =
            self.history_summaries.first().map(|scan| scan.id.clone());
        self.history_trends_error = None;
        match self.dispatch(AppCommand::HistoryTrends {
            limit: HISTORY_TREND_SCAN_LIMIT,
        }) {
            DispatchOutcome::Accepted { .. } => self.history_trends_loading = true,
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.history_trends_error = Some(rejection_text(reason));
                }
            }
        }
    }

    pub(crate) fn begin_history_label_edit(&mut self) {
        let Some(scan_id) = self.selected_history_id.as_deref() else {
            return;
        };
        self.history_label_draft =
            crate::app::policy::history_label_draft_for_selection(&self.history_summaries, scan_id);
        self.history_label_editing = true;
    }

    pub(crate) fn cancel_history_label_edit(&mut self) {
        if let Some(scan_id) = self.selected_history_id.as_deref() {
            self.history_label_draft = crate::app::policy::history_label_draft_for_selection(
                &self.history_summaries,
                scan_id,
            );
        }
        self.history_label_editing = false;
    }

    pub(crate) fn request_history_label_save(&mut self) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        let Some(scan_id) = self.selected_history_id.clone() else {
            return;
        };
        let label = self.history_label_draft.trim();
        let label = (!label.is_empty()).then(|| label.to_string());
        match self.dispatch(AppCommand::SaveHistoryLabel { scan_id, label }) {
            DispatchOutcome::Accepted { .. } => {
                self.history_ack_busy = true;
                self.status = "Saving label…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn request_history_tags_save(&mut self) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        let Some(scan_id) = self.selected_history_id.clone() else {
            return;
        };
        let tags: Vec<String> = self
            .history_tag_draft
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect();
        match self.dispatch(AppCommand::SaveHistoryTags { scan_id, tags }) {
            DispatchOutcome::Accepted { .. } => {
                self.history_ack_busy = true;
                self.status = "Saving tags…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn request_history_clear(&mut self) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        match self.dispatch(AppCommand::ClearHistory) {
            DispatchOutcome::Accepted { .. } => {
                self.history_ack_busy = true;
                self.history_clear_confirm = false;
                self.status = "Clearing scan history…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    // ---- processes -----------------------------------------------------------

    /// Send the process page the current controls describe.
    pub(crate) fn send_process_page_request(&mut self) {
        if self.deterministic_visual || self.page != Page::Processes {
            return;
        }
        let query = ProcessQuery {
            search: self.process_filter.clone(),
            sort_by: self.process_sort_key,
            sort_direction: self.process_sort_direction,
            offset: self.process_offset,
            limit: PROCESS_PAGE_SIZE,
        };
        self.process_last_refresh_started_at = Some(std::time::Instant::now());
        match self.dispatch(AppCommand::RequestProcessPage(query)) {
            DispatchOutcome::Accepted { .. } => {
                self.process_loading = true;
                self.process_error = None;
            }
            outcome => {
                self.process_loading = false;
                if let Some(reason) = outcome.rejection() {
                    self.process_error = Some(rejection_text(reason));
                }
            }
        }
    }

    // ---- AI chat ---------------------------------------------------------------

    /// Whether a consent surface is blocking the composer.
    pub(crate) const fn chat_interaction_blocked(&self) -> bool {
        self.pending_ai_intent.is_some()
            || self.full_scan_consent.is_some()
            || self.cloud_fallback_consent.is_some()
    }

    pub(crate) fn begin_chat_send(&mut self, prompt: String) {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        if self.chat_interaction_blocked() {
            self.status = "Finish or cancel the pending AI request before sending another message"
                .to_string();
            return;
        }
        if self.deterministic_visual {
            self.chat_answer = Some(format!(
                "I checked the fixture scan for “{prompt}”. The urgent finding is low space on C:. Free at least 30 GB, then review the four recent Kernel-Power events."
            ));
            self.chat_last_prompt = Some(prompt);
            self.chat_input.clear();
            self.status = "AI response complete · OpenAI cloud".to_string();
            return;
        }
        self.chat_last_prompt = Some(prompt.clone());
        match self.dispatch(AppCommand::ChatSend { prompt }) {
            DispatchOutcome::Accepted { .. } => {
                self.chat_answer = None;
                self.chat_input.clear();
            }
            DispatchOutcome::Ignored { .. } => {}
            outcome => {
                self.chat_answer = None;
                self.report_rejection(&outcome);
            }
        }
    }

    pub(crate) fn cancel_chat(&mut self) {
        match self.dispatch(AppCommand::ChatCancel) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Cancelling the AI response…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_new_conversation(&mut self) {
        if self.chat_streaming || self.chat_interaction_blocked() {
            self.status =
                "Finish or cancel the current AI request before starting a new conversation"
                    .to_string();
            return;
        }
        self.chat_input.clear();
        self.chat_focus_revision = self.chat_focus_revision.wrapping_add(1);
        self.chat_last_prompt = None;
        self.full_scan_consent = None;
        if self.deterministic_visual {
            self.chat_messages.clear();
            self.chat_answer = None;
            self.status = "New AI conversation started".to_string();
            return;
        }
        let outcome = self.dispatch(AppCommand::ChatReset);
        if outcome.rejection().is_some() {
            self.status = "The native AI conversation could not be reset".to_string();
        }
    }

    pub(crate) fn answer_cloud_fallback(&mut self, allow: bool) {
        match self.dispatch(AppCommand::CloudFallbackDecision { allow }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Saving the cloud fallback preference…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    /// Accept the assistant's Full Scan request.
    ///
    /// Nothing was started by the engine: it reported that the model asked.
    /// The scan is dispatched here and the original prompt is re-sent, which
    /// the engine parks behind the scan and resumes when evidence lands.
    pub(crate) fn approve_full_scan(&mut self) {
        let Some(consent) = self.full_scan_consent.take() else {
            return;
        };
        if self.chat_streaming {
            self.full_scan_consent = Some(consent);
            self.status =
                "Wait for the current AI response to finish before starting the Full Scan"
                    .to_string();
            return;
        }
        if self.scan_busy() {
            self.full_scan_consent = Some(consent);
            self.status =
                "Wait for the active scan to finish before starting a Full Scan".to_string();
            return;
        }
        let current_scan_id = self
            .diagnostic_results
            .first()
            .map(|result| result.session_id.as_str());
        if current_scan_id != Some(consent.source_scan_id.as_str()) {
            self.status = "The scan changed; ask again before running a Full Scan".to_string();
            return;
        }
        self.begin_diagnostic_scan(ScanKind::Full);
        if !consent.original_prompt.is_empty() {
            let _ = self.dispatch(AppCommand::ChatSend {
                prompt: consent.original_prompt,
            });
        }
    }

    // ---- AI report and one-shot analysis -----------------------------------------

    pub(crate) fn begin_report_generation(&mut self, force_refresh: bool) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · report generation is disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::GenerateReport { force_refresh }) {
            DispatchOutcome::Accepted { .. } => {}
            DispatchOutcome::Ignored { .. } => {}
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.report_error = Some(message.clone());
                    self.status = message;
                }
            }
        }
    }

    pub(crate) fn cancel_report(&mut self) {
        // #199: cancellation reaches the generation runtime's own token
        // instead of only clearing the shell's local pending slot.
        match self.dispatch(AppCommand::CancelReport) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Cancelling the AI report…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn cancel_pending_ai_intent(&mut self) {
        if self.pending_ai_intent.is_none() {
            return;
        }
        let busy = self.scan_busy();
        // A parked intent is dropped by cancelling whichever turn owns it.
        let _ = self.dispatch(AppCommand::ChatCancel);
        let _ = self.dispatch(AppCommand::CancelReport);
        self.status = if busy {
            "AI request cancelled · the active diagnostic scan will continue".to_string()
        } else {
            "AI request cancelled".to_string()
        };
    }

    pub(crate) fn retry_pending_ai_intent(&mut self) {
        let Some(intent) = self.pending_ai_intent.clone() else {
            return;
        };
        match intent {
            wfdiag_app::domain::ai_intent::PendingAiIntent::Chat { prompt } => {
                let _ = self.dispatch(AppCommand::ChatSend { prompt });
            }
            wfdiag_app::domain::ai_intent::PendingAiIntent::Report { force_refresh } => {
                let _ = self.dispatch(AppCommand::GenerateReport { force_refresh });
            }
        }
        if self.diagnostic_results.is_empty() && !self.scan_busy() {
            self.status = "Retrying the prerequisite Quick Scan…".to_string();
        } else if self.scan_busy() {
            self.status = "Waiting for the active scan before continuing AI…".to_string();
        }
    }

    pub(crate) fn begin_selected_diagnostic_analysis(&mut self, force_refresh: bool) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · diagnostic AI is disabled".to_string();
            return;
        }
        let Some(task_id) = self.selected_result_task_id.clone().or_else(|| {
            self.diagnostic_results
                .first()
                .map(|result| result.task_id.clone())
        }) else {
            self.status = "Run a diagnostic scan before asking for an interpretation".to_string();
            return;
        };
        if !force_refresh
            && self
                .diagnostic_analyses
                .get(&task_id)
                .and_then(|display| display.interpretation.as_ref())
                .is_some()
        {
            self.status = "Diagnostic interpretation is already available".to_string();
            return;
        }
        match self.dispatch(AppCommand::AnalyzeDiagnostic {
            task_id,
            force_refresh,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Interpreting the selected diagnostic…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_issue_prioritization(&mut self, force_refresh: bool) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · issue prioritization is disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::PrioritizeIssues { force_refresh }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Prioritizing detected issues…".to_string();
            }
            DispatchOutcome::Ignored { detail } => self.status = detail.to_string(),
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_fix_plan(&mut self) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · AI fix planning is disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::GenerateFixPlan) {
            DispatchOutcome::Accepted { .. } => {}
            DispatchOutcome::Ignored { detail } => self.status = detail.to_string(),
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.fix_plan_error = Some(message.clone());
                    self.status = message;
                }
            }
        }
    }

    pub(crate) fn ask_ai_about_issue(&mut self, issue_id: &str) {
        let Some(issue) = self.issues.iter().find(|issue| issue.id == issue_id) else {
            return;
        };
        let remediation = issue.remediation.as_ref().map_or_else(
            || "No vetted remediation is mapped to this issue.".to_string(),
            |remediation| {
                format!(
                    "Vetted remediation: {} (catalog id {}).",
                    remediation.label, remediation.id
                )
            },
        );
        let prompt = format!(
            "Explain this detected Windows issue and give safe next steps using the scan evidence and read-only tools. Issue id: {}. Title: {}. Category: {}. Severity: {:?}. Description: {}. Recommendation: {}. {} Do not claim any repair was run; stage only vetted catalog actions if useful.",
            issue.id,
            issue.title,
            issue.category,
            issue.severity,
            issue.description,
            issue.recommendation,
            remediation,
        );
        self.transition_to_page(Page::Ai);
        self.ai_mode = AiMode::Assistant;
        self.begin_chat_send(prompt);
    }

    // ---- remediation ---------------------------------------------------------------

    pub(crate) fn run_remediation(&mut self, remediation_id: String) {
        if self.deterministic_visual
            && self.live_test_fixture != Some(LiveTestFixture::DeviceManager)
        {
            self.status = "Visual fixture mode · remediation is disabled".to_string();
            return;
        }
        let Some(spec) = remediation::find(&remediation_id) else {
            self.status = format!("Unknown remediation '{remediation_id}'");
            return;
        };
        let issue_id = self
            .issues
            .iter()
            .find(|issue| {
                issue.status == wfdiag_native_issues::IssueStatus::Detected
                    && issue.remediation.as_ref().map(|item| item.id.as_str())
                        == Some(remediation_id.as_str())
            })
            .map(|issue| issue.id.clone());
        if issue_id.is_none() && !spec.maintenance {
            self.status = format!("'{}' is no longer mapped to a detected issue", spec.label);
            return;
        }
        self.prepare_remediation(remediation_id, issue_id);
    }

    pub(crate) fn prepare_remediation(&mut self, remediation_id: String, issue_id: Option<String>) {
        if self.live_test_fixture.is_some_and(|fixture| {
            !fixture.permits_actions(std::slice::from_ref(&ActionRequest {
                remediation_id: remediation_id.clone(),
                issue_id: issue_id.clone(),
            }))
        }) {
            self.status = "The validation fixture rejected an action outside its closed allowlist"
                .to_string();
            return;
        }
        match self.dispatch(AppCommand::PrepareRemediation {
            remediation_id,
            issue_id,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Preparing a review for 1 vetted action…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn prepare_action_selection(&mut self, selection: FixPlanActionSelection) {
        if self
            .live_test_fixture
            .is_some_and(|fixture| !fixture.permits_actions(&selection.actions))
        {
            self.status = "The validation fixture rejected an action outside its closed allowlist"
                .to_string();
            return;
        }
        let action_count = selection.actions.len();
        match self.dispatch(AppCommand::PrepareRemediations {
            actions: selection.actions,
            expected_scan_fingerprint: Some(selection.expected_scan_fingerprint),
            expected_catalog_fingerprint: Some(selection.expected_catalog_fingerprint),
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.status = format!(
                    "Preparing a review for {action_count} vetted action{}…",
                    if action_count == 1 { "" } else { "s" }
                );
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn close_action_review(&mut self, proposal_id: &str, result: ContentDialogResult) {
        let Some(proposal) = self.action_review.clone() else {
            return;
        };
        if proposal.proposal_id != proposal_id {
            return;
        }
        let admin_blocked = !self.is_admin
            && proposal
                .actions
                .iter()
                .any(|action| action.remediation.admin_required);
        if result == ContentDialogResult::Primary && admin_blocked {
            self.request_admin_relaunch();
            return;
        }
        if result == ContentDialogResult::Primary {
            // The Repair gate lives in the broker. Passing the review's
            // confirmation here is not the second approval: a Repair-tier
            // preview still comes back as `RepairConfirmationRequired`.
            let confirm_repair = crate::app::policy::action_proposal_contains_repair(&proposal);
            let outcome = self.dispatch(AppCommand::ApproveAction {
                proposal_id: proposal.proposal_id,
                confirm_repair,
            });
            if outcome.is_accepted() {
                self.status = "Revalidating the reviewed remediation…".to_string();
            } else {
                self.report_rejection(&outcome);
            }
        } else {
            let outcome = self.dispatch(AppCommand::DiscardProposal {
                proposal_id: proposal.proposal_id,
            });
            self.report_rejection(&outcome);
        }
    }

    pub(crate) fn close_repair_confirmation(
        &mut self,
        proposal_id: &str,
        result: ContentDialogResult,
    ) {
        let Some(proposal) = self.repair_confirm.clone() else {
            return;
        };
        if proposal.proposal_id != proposal_id {
            return;
        }
        let outcome = if result == ContentDialogResult::Primary {
            self.dispatch(AppCommand::ApproveAction {
                proposal_id: proposal.proposal_id,
                confirm_repair: true,
            })
        } else {
            self.dispatch(AppCommand::DiscardProposal {
                proposal_id: proposal.proposal_id,
            })
        };
        if outcome.is_accepted() && result == ContentDialogResult::Primary {
            self.status = "Revalidating the reviewed remediation…".to_string();
        } else {
            self.report_rejection(&outcome);
        }
    }

    pub(crate) fn cancel_action_run(&mut self) {
        let Some(run_id) = self
            .action_active_run
            .as_ref()
            .map(|run| run.run_id.clone())
        else {
            self.status = "No remediation run is active".to_string();
            return;
        };
        let outcome = self.dispatch(AppCommand::CancelAction { run_id });
        if let Some(reason) = outcome.rejection() {
            self.status = format!("Could not stop remediation · {}", rejection_text(reason));
        }
    }

    pub(crate) fn request_admin_relaunch(&mut self) {
        if self.deterministic_visual
            && self.live_test_fixture != Some(LiveTestFixture::AdminRelaunch)
        {
            self.status = "Visual fixture mode · elevation is disabled".to_string();
            return;
        }
        match self.dispatch(AppCommand::RestartAsAdmin) {
            DispatchOutcome::Accepted { .. } => {
                self.status = "Restarting with administrator rights…".to_string();
            }
            outcome => self.report_rejection(&outcome),
        }
    }

    // ---- provider setup ---------------------------------------------------------------

    /// Stage a provider credential for the current Settings dialog. Storage is
    /// untouched until the dialog's primary Save action succeeds; Cancel
    /// discards the transaction and every plaintext draft.
    pub(crate) fn submit_provider_key(&mut self, index: usize, store: bool) {
        let Some(provider) = ProviderKeyId::ALL.get(index).copied() else {
            return;
        };
        let Some(key) = self.provider_key_drafts.get(index).cloned() else {
            return;
        };
        if store && key.trim().is_empty() {
            self.status = "Enter an API key first".to_string();
            return;
        }
        if store {
            self.provider_credential_transaction
                .stage_store(provider, key.trim().to_string());
            set_provider_key_configured(&mut self.settings_draft, provider, true);
            self.status = "API key staged · press Save to commit".to_string();
        } else {
            self.provider_credential_transaction.stage_clear(provider);
            set_provider_key_configured(&mut self.settings_draft, provider, false);
            self.status = "API key removal staged · press Save to commit".to_string();
        }
        self.provider_key_busy = false;
        self.settings_save_error = None;
    }

    /// Ask the engine for the selected provider's model list.
    ///
    /// The 400 ms debounce, the cancel-and-retry latch and the "keep the last
    /// catalog visible" rule all live in the engine's own refresh policy, so
    /// this is one dispatch per edit rather than a shell timer.
    pub(crate) fn request_provider_model_refresh(&mut self, forced: bool) {
        if self.deterministic_visual
            || !self.settings_open
            || provider_setup_provider(self.provider_setup_index)
                == Some(wfdiag_native_ai_provider::AIProvider::PhiSilica)
        {
            return;
        }
        let Some(provider) = provider_setup_provider(self.provider_setup_index) else {
            return;
        };
        let draft = match provider_catalog_draft(
            self.provider_setup_index,
            &self.settings_draft,
            &self.provider_key_drafts,
        ) {
            Ok(Some(draft)) => draft,
            // The provider has no catalog at all; show an empty pane.
            Ok(None) => {
                if let Some(state) = self.provider_catalogs.get_mut(self.provider_setup_index) {
                    *state = wfdiag_app::domain::catalog::CatalogState::default();
                }
                return;
            }
            // Discovery cannot even be attempted with the current inputs.
            Err(blocked) => {
                if let Some(state) = self.provider_catalogs.get_mut(self.provider_setup_index) {
                    state.blocked(blocked);
                }
                return;
            }
        };
        let outcome = self.dispatch(AppCommand::RefreshModelCatalog {
            provider: provider.to_string(),
            draft_api_key: draft.api_key,
            draft_endpoint: draft.endpoint,
            draft_cli_path: draft.cli_path,
            forced,
        });
        if let Some(reason) = outcome.rejection()
            && let Some(state) = self.provider_catalogs.get_mut(self.provider_setup_index)
        {
            state.failed(rejection_text(reason));
        }
    }

    pub(crate) fn cancel_provider_model_request(&mut self) {
        if self.dispatch(AppCommand::CancelModelCatalog).is_accepted() && self.settings_open {
            self.status = "Cancelling model discovery…".to_string();
        }
    }

    // ---- subscription CLIs ----------------------------------------------------------------

    pub(crate) fn begin_subscription_auth_operation(
        &mut self,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionOperation,
    ) {
        if self.deterministic_visual || !self.settings_open {
            return;
        }
        let wire = match provider {
            SubscriptionAuthProvider::Codex => "codex_cli",
            SubscriptionAuthProvider::ClaudeCode => "claude_code",
        };
        match self.dispatch(AppCommand::SubscriptionAuth {
            provider: wire.to_string(),
            operation,
        }) {
            DispatchOutcome::Accepted { .. } => {
                if let Some(state) = self
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(provider))
                {
                    state.error = None;
                }
                self.status = match operation {
                    SubscriptionOperation::Status => format!("Checking {provider} account…"),
                    SubscriptionOperation::SignIn => {
                        format!("Waiting for {provider} browser sign-in…")
                    }
                    SubscriptionOperation::SignOut => format!("Signing out of {provider}…"),
                };
            }
            outcome => {
                if let Some(reason) = outcome.rejection()
                    && let Some(state) = self
                        .subscription_auth_states
                        .get_mut(subscription_auth_state_index(provider))
                {
                    state.error = Some(rejection_text(reason));
                    state.operation = None;
                }
            }
        }
    }

    pub(crate) fn cancel_subscription_auth(&mut self) {
        if self
            .dispatch(AppCommand::CancelSubscriptionAuth)
            .is_accepted()
            && self.settings_open
        {
            self.status = "Cancelling the subscription account action…".to_string();
        }
    }

    pub(crate) fn request_subscription_install(&mut self, provider: SubscriptionAuthProvider) {
        if self.deterministic_visual || !self.settings_open {
            return;
        }
        let wire = match provider {
            SubscriptionAuthProvider::Codex => "codex_cli",
            SubscriptionAuthProvider::ClaudeCode => "claude_code",
        };
        // This only raises the confirmation. Nothing is installed until the
        // user answers, and the vendor bootstrap raises a second one of its own.
        let outcome = self.dispatch(AppCommand::InstallSubscriptionCli {
            provider: wire.to_string(),
        });
        if let Some(reason) = outcome.rejection() {
            self.status = rejection_text(reason);
        }
    }

    pub(crate) fn answer_subscription_install_prompt(&mut self, accepted: bool) {
        match self.dispatch(AppCommand::ConfirmSubscriptionInstall { accepted }) {
            DispatchOutcome::Accepted { .. } if accepted => {
                self.subscription_install_busy = true;
                self.status = "Starting the subscription CLI installer…".to_string();
            }
            DispatchOutcome::Accepted { .. } => {}
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn cancel_subscription_install(&mut self) {
        if self
            .dispatch(AppCommand::CancelSubscriptionInstall)
            .is_accepted()
        {
            self.status = "Cancelling the subscription CLI installer…".to_string();
        }
    }

    /// Commit the staged credential transaction alongside a settings save.
    pub(crate) fn commit_provider_credentials(&mut self) {
        if self.provider_credential_transaction.is_empty() {
            return;
        }
        let transaction = self.provider_credential_transaction.clone();
        self.provider_key_busy = true;
        let outcome = self.dispatch(AppCommand::ProviderCredential(
            ProviderCredentialCommand::Commit(Box::new(transaction)),
        ));
        if outcome.rejection().is_some() {
            self.provider_key_busy = false;
            self.report_rejection(&outcome);
        }
    }
}
