//! How the History screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::consts::HISTORY_TREND_SCAN_LIMIT;
use crate::app::policy::rejection_text;
use crate::app::screen::ScreenCx;
use crate::app::state::HistoryTaskDiffProjection;
use crate::screens::history::state::{HistoryMsg, HistoryScreen};
use wfdiag_app::{AppCommand, AppEvent, DispatchOutcome, HistoryEvent, HistoryRequest};

impl HistoryScreen {
    pub(crate) fn update(&mut self, message: HistoryMsg, cx: &mut ScreenCx<'_>) {
        match message {
            HistoryMsg::Refresh => self.request_list(cx),
            HistoryMsg::FilterChanged(value) => self.filter = value,
            HistoryMsg::Select(scan_id) => self.select_scan(scan_id, cx),
            HistoryMsg::ToggleTaskDetail(task_id) => self.toggle_task_detail(task_id, cx),
            HistoryMsg::ToggleClearConfirm(open) => self.clear_confirm = open,
            HistoryMsg::ClearConfirmed => self.request_clear(cx),
            HistoryMsg::BeginLabelEdit => self.begin_label_edit(),
            HistoryMsg::CancelLabelEdit => self.cancel_label_edit(),
            HistoryMsg::LabelDraftChanged(value) => self.label_draft = value,
            HistoryMsg::SaveLabel => self.request_label_save(cx),
            HistoryMsg::TagDraftChanged(value) => self.tag_draft = value,
            HistoryMsg::SaveTags => self.request_tags_save(cx),
            HistoryMsg::RequestTrends => self.request_trends(cx),
        }
    }

    pub(crate) fn request_list(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual {
            return;
        }
        let outcome = cx.dispatch(AppCommand::ListHistory);
        if let Some(reason) = outcome.rejection() {
            self.error = Some(rejection_text(reason));
        }
    }

    pub(crate) fn select_scan(&mut self, scan_id: String, cx: &mut ScreenCx<'_>) {
        self.label_draft =
            crate::app::policy::history_label_draft_for_selection(&self.summaries, &scan_id);
        self.label_editing = false;
        self.tag_draft =
            crate::app::policy::history_tag_draft_for_selection(&self.summaries, &scan_id);
        self.selected_id = Some(scan_id.clone());
        self.request_comparison(scan_id, cx);
    }

    pub(crate) fn request_comparison(&mut self, selected_id: String, cx: &mut ScreenCx<'_>) {
        self.clear_task_diff();
        self.comparison = None;
        self.comparison_error = None;
        let Some(latest_id) = self.summaries.first().map(|scan| scan.id.clone()) else {
            self.selected_id = None;
            cx.status("No history comparison is available".to_string());
            return;
        };
        if selected_id == latest_id {
            cx.status("Latest scan is the comparison baseline".to_string());
            return;
        }
        match cx.dispatch(AppCommand::CompareHistory {
            current_id: latest_id,
            previous_id: selected_id,
            summary_only: true,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.comparing = true;
                cx.status("Comparing the selected scan with latest…".to_string());
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.comparison_error = Some(rejection_text(reason));
                }
            }
        }
    }

    pub(crate) fn clear_task_diff(&mut self) {
        self.expanded_task_id = None;
        self.task_diff = None;
        self.task_diff_error = None;
        self.task_diff_loading = false;
    }

    pub(crate) fn toggle_task_detail(&mut self, task_id: String, cx: &mut ScreenCx<'_>) {
        if self.expanded_task_id.as_deref() == Some(task_id.as_str()) {
            self.clear_task_diff();
            return;
        }
        let Some(comparison) = self.comparison.as_ref() else {
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
        self.clear_task_diff();
        self.expanded_task_id = Some(task_id.clone());
        match cx.dispatch(AppCommand::HistoryTaskDiff {
            current_id,
            previous_id,
            task_id,
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.task_diff_loading = true;
                cx.status("Loading stored task details…".to_string());
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.task_diff_error = Some(message.clone());
                    cx.status(format!("Could not load stored task details · {message}"));
                }
            }
        }
    }

    pub(crate) fn invalidate_trends(&mut self) {
        self.trends = None;
        self.trends_loading = false;
        self.trends_error = None;
        self.trends_baseline_id = None;
    }

    pub(crate) fn request_trends(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual || self.trends_loading {
            return;
        }
        self.trends_baseline_id = self.summaries.first().map(|scan| scan.id.clone());
        self.trends_error = None;
        match cx.dispatch(AppCommand::HistoryTrends {
            limit: HISTORY_TREND_SCAN_LIMIT,
        }) {
            DispatchOutcome::Accepted { .. } => self.trends_loading = true,
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.trends_error = Some(rejection_text(reason));
                }
            }
        }
    }

    pub(crate) fn begin_label_edit(&mut self) {
        let Some(scan_id) = self.selected_id.as_deref() else {
            return;
        };
        self.label_draft =
            crate::app::policy::history_label_draft_for_selection(&self.summaries, scan_id);
        self.label_editing = true;
    }

    pub(crate) fn cancel_label_edit(&mut self) {
        if let Some(scan_id) = self.selected_id.as_deref() {
            self.label_draft =
                crate::app::policy::history_label_draft_for_selection(&self.summaries, scan_id);
        }
        self.label_editing = false;
    }

    pub(crate) fn request_label_save(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual || self.ack_busy {
            return;
        }
        let Some(scan_id) = self.selected_id.clone() else {
            return;
        };
        let label = self.label_draft.trim();
        let label = (!label.is_empty()).then(|| label.to_string());
        match cx.dispatch(AppCommand::SaveHistoryLabel { scan_id, label }) {
            DispatchOutcome::Accepted { .. } => {
                self.ack_busy = true;
                cx.status("Saving label…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn request_tags_save(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual || self.ack_busy {
            return;
        }
        let Some(scan_id) = self.selected_id.clone() else {
            return;
        };
        let tags: Vec<String> = self
            .tag_draft
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect();
        match cx.dispatch(AppCommand::SaveHistoryTags { scan_id, tags }) {
            DispatchOutcome::Accepted { .. } => {
                self.ack_busy = true;
                cx.status("Saving tags…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn request_clear(&mut self, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual || self.ack_busy {
            return;
        }
        match cx.dispatch(AppCommand::ClearHistory) {
            DispatchOutcome::Accepted { .. } => {
                self.ack_busy = true;
                self.clear_confirm = false;
                cx.status("Clearing scan history…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        let AppEvent::History(event) = event else {
            return;
        };
        match event.clone() {
            HistoryEvent::Listed { scans } => {
                let latest_id = scans.first().map(|scan| scan.id.clone());
                let baseline_changed = crate::app::policy::history_trends_baseline_changed(
                    self.trends_baseline_id.as_deref(),
                    latest_id.as_deref(),
                );
                // A newer scan at the top changes what "compare with latest"
                // means, so the open comparison is re-requested.
                let comparison_refresh_target =
                    crate::app::policy::history_comparison_refresh_target(
                        self.comparison
                            .as_ref()
                            .map(|comparison| comparison.current_scan.id.as_str()),
                        &scans,
                        self.selected_id.as_deref(),
                    );
                if self
                    .selected_id
                    .as_ref()
                    .is_some_and(|selected| !scans.iter().any(|scan| &scan.id == selected))
                {
                    self.selected_id = None;
                    self.comparison = None;
                    self.clear_task_diff();
                    self.label_draft.clear();
                    self.label_editing = false;
                    self.tag_draft.clear();
                }
                cx.status(format!("History loaded · {} scans", scans.len()));
                if baseline_changed {
                    self.invalidate_trends();
                    if latest_id.is_some() {
                        self.request_trends(cx);
                    }
                }
                if let Some(selected) = comparison_refresh_target {
                    self.request_comparison(selected, cx);
                } else if !self.label_editing
                    && let Some(selected) = self.selected_id.clone()
                {
                    self.label_draft = crate::app::policy::history_label_draft_for_selection(
                        &self.summaries,
                        &selected,
                    );
                }
            }
            HistoryEvent::ComparedSummary { comparison } => {
                self.comparing = false;
                self.comparison_error = None;
                cx.status(format!(
                    "History comparison · {} changes",
                    comparison.total_changes
                ));
            }
            HistoryEvent::TaskDiff { diff } => {
                self.task_diff_loading = false;
                self.task_diff = Some(HistoryTaskDiffProjection::from(*diff));
                self.task_diff_error = None;
                cx.status("Stored task details loaded".to_string());
            }
            HistoryEvent::Trends { trends } => {
                // An empty answer is a real answer: it renders "no trends yet"
                // rather than staying on the spinner.
                self.trends_loading = false;
                self.trends_error = None;
                self.trends = Some(trends);
            }
            HistoryEvent::LabelSaved { .. } => {
                self.ack_busy = false;
                self.label_editing = false;
                cx.status(if self.label_draft.trim().is_empty() {
                    "Label removed".to_string()
                } else {
                    "Label saved".to_string()
                });
                self.request_list(cx);
            }
            HistoryEvent::TagsSaved { .. } => {
                self.ack_busy = false;
                cx.status("Tags saved".to_string());
                self.request_list(cx);
            }
            HistoryEvent::Cleared => {
                self.ack_busy = false;
                self.selected_id = None;
                self.comparison = None;
                self.clear_task_diff();
                self.invalidate_trends();
                self.label_draft.clear();
                self.label_editing = false;
                self.tag_draft.clear();
                cx.status("Scan history cleared".to_string());
            }
            HistoryEvent::Failed { request, error } => match request {
                HistoryRequest::List => {
                    self.error = Some(error.clone());
                    cx.status(format!("Could not load history · {error}"));
                }
                HistoryRequest::Compare | HistoryRequest::CompareToLatest => {
                    self.comparing = false;
                    self.comparison = None;
                    self.comparison_error = Some(error.clone());
                    cx.status(format!("Could not compare history · {error}"));
                }
                HistoryRequest::TaskDiff => {
                    self.task_diff_loading = false;
                    self.task_diff = None;
                    self.task_diff_error = Some(error.clone());
                    cx.status(format!("Could not load stored task details · {error}"));
                }
                HistoryRequest::Trends => {
                    self.trends_loading = false;
                    self.trends_error = Some(error);
                }
                HistoryRequest::Label => {
                    self.ack_busy = false;
                    cx.status(format!("Could not save label · {error}"));
                }
                HistoryRequest::Tags => {
                    self.ack_busy = false;
                    cx.status(format!("Could not save tags · {error}"));
                }
                HistoryRequest::Clear => {
                    self.ack_busy = false;
                    cx.status(format!("Could not clear history · {error}"));
                }
                HistoryRequest::AutoSave | HistoryRequest::Load => {
                    self.error = Some(error);
                }
            },
            _ => {}
        }
    }
}
