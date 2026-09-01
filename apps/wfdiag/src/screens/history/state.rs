//! The History screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::app::state::HistoryTaskDiffProjection;
use wfdiag_native_history::{ComparisonSummary, ScanSummary, TaskTrend};

/// Everything the History page renders.
#[derive(Default)]
pub(crate) struct HistoryScreen {
    pub(crate) summaries: Vec<ScanSummary>,
    pub(crate) filter: String,
    pub(crate) selected_id: Option<String>,
    pub(crate) comparison: Option<ComparisonSummary>,
    pub(crate) comparing: bool,
    pub(crate) comparison_error: Option<String>,
    pub(crate) expanded_task_id: Option<String>,
    pub(crate) task_diff: Option<HistoryTaskDiffProjection>,
    pub(crate) task_diff_loading: bool,
    pub(crate) task_diff_error: Option<String>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) clear_confirm: bool,
    pub(crate) label_draft: String,
    pub(crate) label_editing: bool,
    pub(crate) tag_draft: String,
    pub(crate) ack_busy: bool,
    pub(crate) trends: Option<Vec<TaskTrend>>,
    pub(crate) trends_loading: bool,
    pub(crate) trends_error: Option<String>,
    pub(crate) trends_baseline_id: Option<String>,
}

/// Everything the History page can ask for.
#[derive(Clone)]
pub(crate) enum HistoryMsg {
    Refresh,
    FilterChanged(String),
    Select(String),
    ToggleTaskDetail(String),
    ToggleClearConfirm(bool),
    ClearConfirmed,
    BeginLabelEdit,
    CancelLabelEdit,
    LabelDraftChanged(String),
    SaveLabel,
    TagDraftChanged(String),
    SaveTags,
    RequestTrends,
}
