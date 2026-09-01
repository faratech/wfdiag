//! Scan history orchestration: list, comparison, trends, and labels.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::HISTORY_TREND_SCAN_LIMIT;
use crate::app::message::{HistoryAckKind, Message};
use crate::app::policy::history_change_rows;
use crate::app::state::HistoryTaskDiffProjection;
use crate::app::tasks::spawn_history_ack_wait;
use std::sync::Arc;
use std::time::Duration;
use windows_reactor::*;

impl WfdiagShell {
    /// Save or clear the user-facing label without touching metadata tags.
    pub(crate) fn request_history_label_save(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.status = "Native history is unavailable".to_string();
            return;
        };
        let Some(scan_id) = self.selected_history_id.clone() else {
            return;
        };
        let label = self.history_label_draft.trim();
        let label = (!label.is_empty()).then(|| label.to_string());
        match runtime.request_update_label(scan_id, label) {
            Ok(reply) => {
                self.history_ack_busy = true;
                self.status = "Saving label…".to_string();
                self.history_wait = Some(spawn_history_ack_wait(
                    context,
                    HistoryAckKind::Label,
                    reply,
                ));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Save the tag draft for the selected scan through the history worker.
    pub(crate) fn request_history_tags_save(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.status = "Native history is unavailable".to_string();
            return;
        };
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
        match runtime.request_update_tags(scan_id, tags) {
            Ok(reply) => {
                self.history_ack_busy = true;
                self.status = "Saving tags…".to_string();
                self.history_wait =
                    Some(spawn_history_ack_wait(context, HistoryAckKind::Tags, reply));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(crate) fn invalidate_history_trends(&mut self) {
        self.history_trends_request_id = self.history_trends_request_id.wrapping_add(1);
        if self.history_trends_request_id == 0 {
            self.history_trends_request_id = 1;
        }
        self.history_trends = None;
        self.history_trends_loading = false;
        self.history_trends_error = None;
        self.history_trends_baseline_id = None;
    }

    pub(crate) fn request_history_trends(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual || self.history_trends_loading {
            return;
        }
        let baseline_id = self.history_summaries.first().map(|scan| scan.id.clone());
        self.history_trends_request_id = self.history_trends_request_id.wrapping_add(1);
        if self.history_trends_request_id == 0 {
            self.history_trends_request_id = 1;
        }
        let request_id = self.history_trends_request_id;
        self.history_trends_baseline_id = baseline_id;
        self.history_trends_loading = true;
        self.history_trends_error = None;

        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.history_trends_loading = false;
            self.history_trends_error = Some("Native history is unavailable".to_string());
            return;
        };
        match runtime.request_trends(HISTORY_TREND_SCAN_LIMIT) {
            Ok(mut reply) => {
                context.spawn_background_with_rejection(
                    move |cancellation| loop {
                        if cancellation.is_cancelled() {
                            return Message::HistoryTrendsFinished {
                                request_id,
                                result: Box::new(Err(
                                    "The Reactor background queue rejected trends".to_string(),
                                )),
                            };
                        }
                        match reply.try_recv() {
                            Ok(result) => {
                                return Message::HistoryTrendsFinished {
                                    request_id,
                                    result: Box::new(result.map_err(|error| error.to_string())),
                                };
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                                return Message::HistoryTrendsFinished {
                                    request_id,
                                    result: Box::new(Err(
                                        "Native history worker stopped".to_string()
                                    )),
                                };
                            }
                        }
                    },
                    Message::HistoryTrendsFinished {
                        request_id,
                        result: Box::new(Err(
                            "The Reactor background queue rejected trends".to_string()
                        )),
                    },
                );
            }
            Err(error) => {
                self.history_trends_loading = false;
                self.history_trends_error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn clear_history_task_diff(&mut self) {
        self.history_task_diff_request_id = self.history_task_diff_request_id.wrapping_add(1);
        if self.history_task_diff_request_id == 0 {
            self.history_task_diff_request_id = 1;
        }
        if let Some(task) = self.history_task_diff_task.take() {
            task.cancel();
        }
        self.history_expanded_task_id = None;
        self.history_task_diff = None;
        self.history_task_diff_error = None;
    }

    pub(crate) fn toggle_history_task_detail(
        &mut self,
        task_id: String,
        context: &ComponentContext<Self>,
    ) {
        if self.history_expanded_task_id.as_deref() == Some(task_id.as_str()) {
            self.clear_history_task_diff();
            return;
        }

        let Some(comparison) = self.history_comparison.as_ref() else {
            return;
        };
        if !history_change_rows(comparison)
            .iter()
            .any(|(_, change)| change.task_id == task_id)
        {
            return;
        }
        let current_id = comparison.current_scan.id.clone();
        let previous_id = comparison.previous_scan.id.clone();
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.status = "Native history is unavailable".to_string();
            return;
        };

        self.clear_history_task_diff();
        self.history_expanded_task_id = Some(task_id.clone());
        let request_id = self.history_task_diff_request_id;
        match runtime.request_task_diff(current_id, previous_id, task_id.clone()) {
            Ok(mut reply) => {
                self.status = "Loading stored task details…".to_string();
                let rejected_task_id = task_id.clone();
                self.history_task_diff_task = Some(context.spawn_background_with_rejection(
                    move |cancellation| loop {
                        if cancellation.is_cancelled() {
                            return Message::HistoryTaskDiffRejected {
                                request_id,
                                task_id,
                            };
                        }
                        match reply.try_recv() {
                            Ok(result) => {
                                return Message::HistoryTaskDiffFinished {
                                    request_id,
                                    task_id,
                                    result: result
                                        .map(HistoryTaskDiffProjection::from)
                                        .map(Box::new)
                                        .map_err(|error| error.to_string()),
                                };
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                                return Message::HistoryTaskDiffFinished {
                                    request_id,
                                    task_id,
                                    result: Err("Native history worker stopped".to_string()),
                                };
                            }
                        }
                    },
                    Message::HistoryTaskDiffRejected {
                        request_id,
                        task_id: rejected_task_id,
                    },
                ));
            }
            Err(error) => {
                let message = error.to_string();
                self.history_task_diff_error = Some(message.clone());
                self.status = format!("Could not load stored task details · {message}");
            }
        }
    }

    /// Clear all stored scans after the explicit confirmation dialog.
    pub(crate) fn request_history_clear(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual || self.history_ack_busy {
            return;
        }
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.status = "Native history is unavailable".to_string();
            return;
        };
        match runtime.request_clear() {
            Ok(reply) => {
                self.history_ack_busy = true;
                self.history_clear_confirm = false;
                self.status = "Clearing scan history…".to_string();
                self.history_wait = Some(spawn_history_ack_wait(
                    context,
                    HistoryAckKind::Clear,
                    reply,
                ));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(crate) fn request_history_list(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            return;
        }
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.history_loading = false;
            self.history_error
                .get_or_insert_with(|| "Native history is unavailable".to_string());
            return;
        };

        self.history_request_id = self.history_request_id.wrapping_add(1);
        if self.history_request_id == 0 {
            self.history_request_id = 1;
        }
        let request_id = self.history_request_id;
        if let Some(task) = self.history_request_task.take() {
            task.cancel();
        }
        self.history_loading = true;
        self.history_error = None;
        self.history_request_task = Some(context.spawn_background_with_rejection(
            move |_| {
                let result = runtime
                    .request_list()
                    .map_err(|error| error.to_string())
                    .and_then(|receiver| {
                        receiver
                            .blocking_recv()
                            .map_err(|_| "native history worker closed the request".to_string())?
                    });
                Message::HistoryListFinished { request_id, result }
            },
            Message::HistoryQueryRejected {
                request_id,
                comparison: false,
            },
        ));
    }

    pub(crate) fn request_history_comparison(
        &mut self,
        selected_id: String,
        context: &ComponentContext<Self>,
    ) {
        // Invalidate any in-flight result before handling the latest-scan
        // no-op. Otherwise an older request can still win after the user
        // selects the latest row, and the view has no reliable loading state.
        self.history_compare_request_id = self.history_compare_request_id.wrapping_add(1);
        if self.history_compare_request_id == 0 {
            self.history_compare_request_id = 1;
        }
        let request_id = self.history_compare_request_id;
        if let Some(task) = self.history_compare_task.take() {
            task.cancel();
        }
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
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.history_comparison_error = Some("Native history is unavailable".to_string());
            return;
        };

        self.status = "Comparing the selected scan with latest…".to_string();
        self.history_compare_task = Some(context.spawn_background_with_rejection(
            move |_| {
                let result = runtime
                    .request_compare_summary(latest_id, selected_id)
                    .map_err(|error| error.to_string())
                    .and_then(|receiver| {
                        receiver.blocking_recv().map_err(|_| {
                            "native history worker closed the comparison".to_string()
                        })?
                    })
                    .map(Box::new);
                Message::HistoryCompareFinished { request_id, result }
            },
            Message::HistoryQueryRejected {
                request_id,
                comparison: true,
            },
        ));
    }
}
