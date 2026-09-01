//! The History page: scan list, comparison, and per-task diffs.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::{HistoryChangeKind, Message};
use crate::app::policy::{
    history_change_rows, history_display_label, history_scan_matches_filter, history_trend_badge,
};
use crate::app::screen::ShellEnv;
use crate::app::state::{HistoryTaskDiffProjection, HistoryTrendBadge, Page};
use crate::fixtures::visual::VisualState;
use crate::screens::diagnostics::view::format_diagnostic_duration;
use crate::screens::history::state::{HistoryMsg, HistoryScreen};
use crate::widgets::chrome::{fa_icon_label, page_header, placed};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use crate::widgets::table::{table_cell, table_header};
use wfdiag_native_history::{ComparisonSummary, ScanSummary, TaskChangeSummary, TaskTrend};
use wfdiag_native_projection::json_diff::{
    JsonDifference, JsonDifferenceKind, visible_differences,
};
use windows_reactor::*;

impl HistoryScreen {
    /// Paint the page from the screen's own state plus the chrome's env.
    pub(crate) fn view(&self, env: &ShellEnv<'_>, vc: &mut ViewContext<WfdiagShell>) -> View {
        history_page(
            env.palette,
            env.narrow,
            env.deterministic_visual,
            env.visual_state == VisualState::HistoryEmpty,
            &self.summaries,
            &self.filter,
            self.selected_id.as_deref(),
            self.label_draft.as_str(),
            self.label_editing,
            self.tag_draft.as_str(),
            self.comparison.as_ref(),
            self.comparing,
            self.comparison_error.as_deref(),
            self.expanded_task_id.as_deref(),
            self.task_diff.as_ref(),
            self.task_diff_loading,
            self.task_diff_error.as_deref(),
            self.loading,
            self.error.as_deref(),
            self.ack_busy,
            vc.message(Message::History(HistoryMsg::Refresh)),
            vc.callback(|value| Message::History(HistoryMsg::FilterChanged(value))),
            vc.callback(|value| Message::History(HistoryMsg::Select(value))),
            vc.message(Message::History(HistoryMsg::BeginLabelEdit)),
            vc.message(Message::History(HistoryMsg::CancelLabelEdit)),
            vc.callback(|value| Message::History(HistoryMsg::LabelDraftChanged(value))),
            vc.message(Message::History(HistoryMsg::SaveLabel)),
            vc.callback(|value| Message::History(HistoryMsg::TagDraftChanged(value))),
            vc.message(Message::History(HistoryMsg::SaveTags)),
            vc.callback(|value| Message::History(HistoryMsg::ToggleTaskDetail(value))),
            vc.message(Message::History(HistoryMsg::ToggleClearConfirm(true))),
            vc.message(Message::History(HistoryMsg::ClearConfirmed)),
            vc.message(Message::History(HistoryMsg::ToggleClearConfirm(false))),
            self.clear_confirm,
            self.trends.as_deref(),
            self.trends_loading,
            self.trends_error.as_deref(),
            vc.message(Message::History(HistoryMsg::RequestTrends)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_page(
    palette: Palette,
    narrow: bool,
    deterministic_visual: bool,
    fixture_empty: bool,
    summaries: &[ScanSummary],
    filter: &str,
    selected_id: Option<&str>,
    selected_label: &str,
    label_editing: bool,
    selected_tags: &str,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    comparison_error: Option<&str>,
    expanded_task_id: Option<&str>,
    task_diff: Option<&HistoryTaskDiffProjection>,
    task_diff_loading: bool,
    task_diff_error: Option<&str>,
    loading: bool,
    error: Option<&str>,
    ack_busy: bool,
    refresh: Callback<()>,
    filter_changed: Callback<String>,
    select_history: Callback<String>,
    begin_label_edit: Callback<()>,
    cancel_label_edit: Callback<()>,
    label_changed: Callback<String>,
    save_label: Callback<()>,
    tag_changed: Callback<String>,
    save_tags: Callback<()>,
    toggle_task_detail: Callback<String>,
    clear_request: Callback<()>,
    clear_confirmed: Callback<()>,
    clear_cancelled: Callback<()>,
    clear_confirm_open: bool,
    trends: Option<&[TaskTrend]>,
    trends_loading: bool,
    trends_error: Option<&str>,
    load_trends: Callback<()>,
) -> View {
    #[cfg(feature = "validation")]
    if deterministic_visual {
        return history_fixture_page(palette, narrow, fixture_empty);
    }
    // A shipping build's knobs are compile-time `Live`/`None`/`false`, so the
    // fixture page could never be reached.
    #[cfg(not(feature = "validation"))]
    let _ = (deterministic_visual, fixture_empty, palette, narrow);
    history_live_page(
        palette,
        narrow,
        summaries,
        filter,
        selected_id,
        selected_label,
        label_editing,
        selected_tags,
        comparison,
        comparison_loading,
        comparison_error,
        expanded_task_id,
        task_diff,
        task_diff_loading,
        task_diff_error,
        loading,
        error,
        ack_busy,
        refresh,
        filter_changed,
        select_history,
        begin_label_edit,
        cancel_label_edit,
        label_changed,
        save_label,
        tag_changed,
        save_tags,
        toggle_task_detail,
        clear_request,
        clear_confirmed,
        clear_cancelled,
        clear_confirm_open,
        trends,
        trends_loading,
        trends_error,
        load_trends,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_live_page(
    palette: Palette,
    narrow: bool,
    summaries: &[ScanSummary],
    filter: &str,
    selected_id: Option<&str>,
    selected_label: &str,
    label_editing: bool,
    selected_tags: &str,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    comparison_error: Option<&str>,
    expanded_task_id: Option<&str>,
    task_diff: Option<&HistoryTaskDiffProjection>,
    task_diff_loading: bool,
    task_diff_error: Option<&str>,
    loading: bool,
    error: Option<&str>,
    ack_busy: bool,
    refresh: Callback<()>,
    filter_changed: Callback<String>,
    select_history: Callback<String>,
    begin_label_edit: Callback<()>,
    cancel_label_edit: Callback<()>,
    label_changed: Callback<String>,
    save_label: Callback<()>,
    tag_changed: Callback<String>,
    save_tags: Callback<()>,
    toggle_task_detail: Callback<String>,
    clear_request: Callback<()>,
    clear_confirmed: Callback<()>,
    clear_cancelled: Callback<()>,
    clear_confirm_open: bool,
    trends: Option<&[TaskTrend]>,
    trends_loading: bool,
    trends_error: Option<&str>,
    load_trends: Callback<()>,
) -> View {
    let needle = filter.trim().to_ascii_lowercase();
    let filtered = summaries
        .iter()
        .filter(|scan| history_scan_matches_filter(scan, &needle))
        .collect::<Vec<_>>();

    let session_rows = filtered
        .iter()
        .map(|scan| {
            let scan_id = scan.id.clone();
            let select = select_history.clone();
            let failure_count = if scan.failure_count == 0 {
                "—".to_string()
            } else {
                scan.failure_count.to_string()
            };
            KeyedView::new(
                scan.id.clone(),
                Button::new()
                    .height(46.0)
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .horizontal_content_alignment(HorizontalAlignment::Stretch)
                    .resource_overrides(
                        ResourceOverrides::new()
                            .set("ButtonBackground", Color::transparent())
                            .set("ButtonBackgroundPointerOver", palette.active)
                            .set("ButtonBackgroundPressed", palette.active)
                            .set("ButtonForeground", palette.text)
                            .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                            .set("ButtonPadding", Thickness::uniform(0.0))
                            .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
                    )
                    .automation_name(format!(
                        "Compare scan {} from {}",
                        history_display_label(scan),
                        scan.timestamp.to_iso_string()
                    ))
                    .on_click(move || {
                        let _ = select.call(scan_id.clone());
                    })
                    .content(history_row(
                        palette,
                        &history_timestamp(scan),
                        history_display_label(scan),
                        &scan.success_count.to_string(),
                        &failure_count,
                        &format_diagnostic_duration(scan.duration_ms),
                        selected_id == Some(scan.id.as_str()),
                        summaries.first().is_some_and(|latest| latest.id == scan.id),
                    )),
            )
        })
        .collect::<Vec<_>>();

    let list_body: View = if loading && summaries.is_empty() {
        history_list_message(palette, "Loading saved scans…")
    } else if let Some(error) = error
        && summaries.is_empty()
    {
        history_list_message(palette, &format!("Could not load saved scans: {error}"))
    } else if summaries.is_empty() {
        history_list_message(
            palette,
            "No saved scans yet. Run and save a scan to start tracking drift.",
        )
    } else if session_rows.is_empty() {
        history_list_message(
            palette,
            &format!("No saved scans match “{}”.", filter.trim()),
        )
    } else {
        StackPanel::new().keyed_children(session_rows)
    };

    let sessions = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().children((
            history_section_header(palette, "Scan Sessions", Some("Click to compare vs latest")),
            history_header(palette),
            list_body,
        )));

    let latest_id = summaries.first().map(|scan| scan.id.as_str());
    let selected_scan = selected_id.and_then(|id| summaries.iter().find(|scan| scan.id == id));
    let diff = history_live_comparison(
        palette,
        selected_id,
        latest_id,
        selected_scan,
        selected_label,
        label_editing,
        ack_busy,
        comparison,
        comparison_loading,
        comparison_error,
        expanded_task_id,
        task_diff,
        task_diff_loading,
        task_diff_error,
        trends,
        begin_label_edit,
        cancel_label_edit,
        label_changed,
        save_label,
        toggle_task_detail,
    );
    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((sessions, diff))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.4), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((sessions, placed(diff, 1, 0)))
    };
    let visible_count = filtered.len();
    let count = if needle.is_empty() {
        format!("{} scans", summaries.len())
    } else {
        format!("{} of {} scans", visible_count, summaries.len())
    };
    let count_label = if loading && !summaries.is_empty() {
        format!("{count} · refreshing…")
    } else {
        count
    };
    let list_error_notice: View = if let Some(error) = error
        && !summaries.is_empty()
    {
        Border::new()
            .padding(Thickness::new(12.0, 9.0, 12.0, 9.0))
            .background(palette.err_bg)
            .border_brush(palette.err)
            .border_thickness(1.0)
            .corner_radius(7.0)
            .content(
                TextBlock::new()
                    .text(format!(
                        "History refresh failed · {error} · showing the last successful list"
                    ))
                    .font_size(11.5)
                    .foreground(palette.err)
                    .text_wrapping(TextWrapping::Wrap),
            )
    } else {
        View::empty()
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::History, View::empty()),
        Grid::new()
            .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        TextBlock::new()
                            .text(count_label)
                            .font_size(12.0)
                            .vertical_alignment(VerticalAlignment::Center),
                        TextBox::new()
                            .width(260.0)
                            .height(32.0)
                            .text(filter)
                            .placeholder_text("Filter by label, date, machine…")
                            .on_text_changed(filter_changed),
                    )),
                StackPanel::new()
                    .grid_column(1)
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        Button::new()
                            .width(110.0)
                            .is_enabled(!loading)
                            .on_click(refresh)
                            .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
                        Button::new()
                            .width(147.0)
                            .is_enabled(!loading && !ack_busy && !summaries.is_empty())
                            .on_click(clear_request)
                            .automation_name("Clear history")
                            .content(fa_icon_label(FaIcon::Trash, "Clear history")),
                    )),
            )),
        list_error_notice,
        body,
        {
            let load_trends_button = Button::new()
                .height(32.0)
                .is_enabled(!trends_loading)
                .on_click(load_trends)
                .content(if trends_loading {
                    fa_icon_label(FaIcon::Refresh, "Loading…")
                } else {
                    fa_icon_label(FaIcon::ChartLine, "Failure trends")
                });
            let trends_panel: View = if let Some(error) = trends_error {
                TextBlock::new()
                    .text(format!("Failure trends unavailable · {error}"))
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .into()
            } else {
                match trends {
                    None => View::from(
                    TextBlock::new()
                        .text(if trends_loading {
                            "Loading failure trends across recent scans…"
                        } else {
                            "Load failure trends across recent scans."
                        })
                        .font_size(12.0)
                        .foreground(palette.muted),
                    ),
                    Some(items) => {
                        let mut sorted: Vec<&TaskTrend> = items.iter().collect();
                        sorted
                            .sort_by(|a, b| b.failed.cmp(&a.failed).then(a.task_id.cmp(&b.task_id)));
                        let rows: Vec<KeyedView> = sorted
                            .iter()
                            .take(8)
                            .filter(|trend| trend.failed > 0)
                            .enumerate()
                            .map(|(index, trend)| {
                                KeyedView::new(
                                    index,
                                    Grid::new()
                                        .columns([
                                            GridLength::Star(1.0),
                                            GridLength::Pixel(120.0),
                                        ])
                                        .children((
                                            TextBlock::new()
                                                .text(trend.task_id.clone())
                                                .font_size(11.5)
                                                .text_trimming(TextTrimming::CharacterEllipsis),
                                            TextBlock::new()
                                                .text(format!(
                                                    "{} failed / {} scans",
                                                    trend.failed, trend.scans_considered
                                                ))
                                                .grid_column(1)
                                                .font_size(11.5)
                                                .foreground(palette.err),
                                        )),
                                )
                            })
                            .collect();
                        if rows.is_empty() {
                            View::from(
                                TextBlock::new()
                                    .text("No recurring failures in recent scans.")
                                    .font_size(12.0)
                                    .foreground(palette.muted),
                            )
                        } else {
                            StackPanel::new().spacing(3.0).keyed_children(rows)
                        }
                    }
                }
            };
            StackPanel::new()
                .spacing(6.0)
                .children((load_trends_button, trends_panel))
        },
        {
            let tags_editor: View = selected_id
                .and_then(|id| summaries.iter().find(|scan| scan.id == id))
                .map(|_| {
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((
                            TextBlock::new()
                                .text("Tags")
                                .font_size(12.0)
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBox::new()
                                .width(320.0)
                                .height(32.0)
                                .text(selected_tags.to_string())
                                .placeholder_text("comma-separated tags")
                                .is_enabled(!ack_busy)
                                .on_text_changed(tag_changed)
                                .automation_name("Scan tags"),
                            Button::new()
                                .height(32.0)
                                .is_enabled(!ack_busy)
                                .on_click(save_tags)
                                .content("Save tags"),
                        ))
                })
                .unwrap_or_else(View::empty);
            tags_editor
        },
        if clear_confirm_open {
            let confirm = clear_confirmed.clone();
            let cancel = clear_cancelled.clone();
            ContentDialog::new()
                .title("Clear scan history?")
                .is_open(true)
                .primary_button_text("Clear everything")
                .secondary_button_text("Cancel")
                .on_closed(move |result| {
                    if result == ContentDialogResult::Primary {
                        let _ = confirm.call(());
                    } else {
                        let _ = cancel.call(());
                    }
                })
                .content(
                    Border::new()
                        .width(412.0)
                        .background(palette.card_strong)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .content(
                            TextBlock::new()
                                .text("Every stored scan, tag, and comparison baseline will be permanently deleted. This cannot be undone.")
                                .font_size(12.5)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        },
    ))
}

pub(crate) fn history_timestamp(scan: &ScanSummary) -> String {
    scan.timestamp.format("%m/%d/%Y\n%H:%M:%S UTC")
}

pub(crate) fn history_list_message(palette: Palette, message: &str) -> View {
    Border::new().height(246.0).content(
        TextBlock::new()
            .text(message)
            .font_size(12.5)
            .foreground(palette.muted)
            .text_wrapping(TextWrapping::Wrap)
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center),
    )
}

pub(crate) fn history_section_header(palette: Palette, title: &str, hint: Option<&str>) -> View {
    let hint: View = hint.map_or_else(View::empty, |hint| {
        TextBlock::new()
            .text(hint.to_string())
            .grid_column(2)
            .font_size(11.0)
            .foreground(palette.muted)
            .vertical_alignment(VerticalAlignment::Center)
            .into()
    });
    Border::new()
        .height(45.0)
        .padding(Thickness::xy(18.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([
                    GridLength::Pixel(3.0),
                    GridLength::Star(1.0),
                    GridLength::Auto,
                ])
                .column_spacing(10.0)
                .children((
                    Border::new()
                        .width(3.0)
                        .height(15.0)
                        .background(palette.accent)
                        .corner_radius(999.0)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(title.to_string())
                        .grid_column(1)
                        .font_size(12.0)
                        .font_weight(FontWeight::BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                    hint,
                )),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_live_comparison(
    palette: Palette,
    selected_id: Option<&str>,
    latest_id: Option<&str>,
    selected_scan: Option<&ScanSummary>,
    selected_label: &str,
    label_editing: bool,
    ack_busy: bool,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    error: Option<&str>,
    expanded_task_id: Option<&str>,
    task_diff: Option<&HistoryTaskDiffProjection>,
    task_diff_loading: bool,
    task_diff_error: Option<&str>,
    trends: Option<&[TaskTrend]>,
    begin_label_edit: Callback<()>,
    cancel_label_edit: Callback<()>,
    label_changed: Callback<String>,
    save_label: Callback<()>,
    toggle_task_detail: Callback<String>,
) -> View {
    let label_editor = history_label_editor(
        palette,
        selected_scan,
        selected_label,
        label_editing,
        ack_busy,
        begin_label_edit,
        cancel_label_edit,
        label_changed,
        save_label,
    );
    let body: View = if let Some(comparison) = comparison {
        let output_changed = comparison
            .status_unchanged
            .iter()
            .filter(|change| change.output_changed)
            .count();
        let mut rows = vec![
            KeyedView::new(
                "metric-failures",
                history_metric(
                    palette,
                    "New collection errors",
                    comparison.new_failures.len().to_string(),
                    palette.err,
                ),
            ),
            KeyedView::new(
                "metric-successes",
                history_metric(
                    palette,
                    "Newly collected",
                    comparison.new_successes.len().to_string(),
                    palette.ok,
                ),
            ),
            KeyedView::new(
                "metric-changed",
                history_metric(
                    palette,
                    "Output changed",
                    output_changed.to_string(),
                    palette.text,
                ),
            ),
        ];
        rows.extend(
            history_change_rows(comparison)
                .into_iter()
                .map(|(kind, change)| {
                    let is_expanded = expanded_task_id == Some(change.task_id.as_str());
                    KeyedView::new(
                        format!("{}-{}", kind.label(), change.task_id),
                        history_change_toggle(
                            palette,
                            kind,
                            change,
                            trends,
                            is_expanded,
                            is_expanded.then_some(task_diff).flatten(),
                            is_expanded && task_diff_loading,
                            is_expanded.then_some(task_diff_error).flatten(),
                            toggle_task_detail.clone(),
                        ),
                    )
                }),
        );
        let no_drift: View = if comparison.total_changes == 0 {
            Border::new()
                .margin(Thickness::new(0.0, 3.0, 0.0, 0.0))
                .padding(Thickness::new(10.0, 8.0, 10.0, 8.0))
                .background(palette.ok_bg)
                .corner_radius(6.0)
                .content(
                    TextBlock::new()
                        .text("No drift — both scans produced identical results.")
                        .font_size(12.0)
                        .foreground(palette.ok)
                        .text_wrapping(TextWrapping::Wrap),
                )
        } else {
            View::empty()
        };
        Border::new()
            .padding(Thickness::new(14.0, 13.0, 14.0, 14.0))
            .content(
                StackPanel::new().spacing(9.0).children((
                    TextBlock::new()
                        .text(format!(
                            "Comparing {} against the latest scan — {} changes.",
                            comparison.previous_scan.timestamp.to_iso_string(),
                            comparison.total_changes
                        ))
                        .text_wrapping(TextWrapping::Wrap)
                        .font_size(12.0),
                    StackPanel::new().keyed_children(rows),
                    no_drift,
                )),
            )
    } else {
        let message =
            history_comparison_placeholder(selected_id, latest_id, comparison_loading, error);
        Border::new()
            .height(72.0)
            .padding(Thickness::xy(14.0, 0.0))
            .content(
                TextBlock::new()
                    .text(message)
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .vertical_alignment(VerticalAlignment::Center),
            )
    };

    Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .vertical_alignment(VerticalAlignment::Top)
        .content(StackPanel::new().children((
            history_section_header(palette, "Diff vs latest", None),
            label_editor,
            body,
        )))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_label_editor(
    palette: Palette,
    selected_scan: Option<&ScanSummary>,
    label_draft: &str,
    editing: bool,
    busy: bool,
    begin_edit: Callback<()>,
    cancel_edit: Callback<()>,
    label_changed: Callback<String>,
    save_label: Callback<()>,
) -> View {
    let Some(scan) = selected_scan else {
        return View::empty();
    };
    let content: View = if editing {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(7.0)
            .children((
                TextBlock::new()
                    .text("Label:")
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .vertical_alignment(VerticalAlignment::Center),
                TextBox::new()
                    .width(190.0)
                    .height(30.0)
                    .text(label_draft)
                    .is_enabled(!busy)
                    .automation_name("Scan label")
                    .on_text_changed(label_changed),
                Button::new()
                    .height(30.0)
                    .is_enabled(!busy)
                    .automation_name("Save scan label")
                    .on_click(save_label)
                    .content("Save"),
                Button::new()
                    .height(30.0)
                    .is_enabled(!busy)
                    .automation_name("Cancel scan label edit")
                    .on_click(cancel_edit)
                    .content("Cancel"),
            ))
    } else {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(7.0)
            .children((
                TextBlock::new()
                    .text("Label:")
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .vertical_alignment(VerticalAlignment::Center),
                TextBlock::new()
                    .text(history_display_label(scan))
                    .font_size(12.0)
                    .font_weight(FontWeight::BOLD)
                    .vertical_alignment(VerticalAlignment::Center),
                Button::new()
                    .width(30.0)
                    .height(30.0)
                    .is_enabled(!busy)
                    .automation_name("Edit scan label")
                    .on_click(begin_edit)
                    .content(icons::path(FaIcon::Pen).width(12.0).height(12.0)),
            ))
    };
    Border::new()
        .padding(Thickness::new(14.0, 10.0, 14.0, 0.0))
        .content(content)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_change_toggle(
    palette: Palette,
    kind: HistoryChangeKind,
    change: &TaskChangeSummary,
    trends: Option<&[TaskTrend]>,
    expanded: bool,
    detail: Option<&HistoryTaskDiffProjection>,
    detail_loading: bool,
    detail_error: Option<&str>,
    toggle: Callback<String>,
) -> View {
    let (color, background) = match kind {
        HistoryChangeKind::Regressed => (palette.err, palette.err_bg),
        HistoryChangeKind::Recovered => (palette.ok, palette.ok_bg),
        HistoryChangeKind::Changed => (palette.accent, palette.active),
    };
    let trend_badge = history_trend_badge(trends, &change.task_id);
    let trend_announcement = trend_badge
        .as_ref()
        .map(|badge| format!(", {}", badge.description))
        .unwrap_or_default();
    let task_id = change.task_id.clone();
    let button = Button::new()
        .height(36.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.text)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::uniform(0.0)),
        )
        .automation_name(format!(
            "{} {} {} details{}",
            if expanded { "Collapse" } else { "Expand" },
            kind.label(),
            change.task_name,
            trend_announcement
        ))
        .on_click(move || {
            let _ = toggle.call(task_id.clone());
        })
        .content(history_change(
            palette,
            kind.label(),
            change.task_name.clone(),
            trend_badge.as_ref(),
            color,
            background,
            expanded,
        ));
    let detail: View = if expanded {
        history_task_diff_panel(palette, detail, detail_loading, detail_error)
    } else {
        View::empty()
    };
    StackPanel::new().children((button, detail))
}

pub(crate) fn history_task_diff_panel(
    palette: Palette,
    detail: Option<&HistoryTaskDiffProjection>,
    loading: bool,
    error: Option<&str>,
) -> View {
    let body: View = if loading {
        TextBlock::new()
            .text("Loading task details…")
            .font_size(11.5)
            .foreground(palette.muted)
            .into()
    } else if let Some(error) = error {
        TextBlock::new()
            .text(format!("Could not load task details: {error}"))
            .font_size(11.5)
            .foreground(palette.err)
            .text_wrapping(TextWrapping::Wrap)
            .into()
    } else if let Some(projection) = detail {
        let raw_outputs: View = Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Star(1.0)])
            .column_spacing(8.0)
            .children((
                history_task_output(palette, "Previous", &projection.detail.previous_output),
                placed(
                    history_task_output(palette, "Current", &projection.detail.current_output),
                    1,
                    0,
                ),
            ));
        if let Some((differences, hidden_count)) =
            history_visible_json_differences(projection.differences.as_deref())
        {
            StackPanel::new().spacing(8.0).children((
                history_json_difference_rows(palette, differences, hidden_count),
                raw_outputs,
            ))
        } else {
            raw_outputs
        }
    } else {
        TextBlock::new()
            .text("Task details are unavailable. Collapse and expand the row to retry.")
            .font_size(11.5)
            .foreground(palette.muted)
            .text_wrapping(TextWrapping::Wrap)
            .into()
    };
    Border::new()
        .margin(Thickness::new(24.0, 2.0, 0.0, 8.0))
        .padding(Thickness::new(8.0, 7.0, 8.0, 8.0))
        .background(palette.card_strong)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(6.0)
        .content(body)
}

pub(crate) fn history_visible_json_differences(
    differences: Option<&[JsonDifference]>,
) -> Option<(&[JsonDifference], usize)> {
    let differences = differences.filter(|differences| !differences.is_empty())?;
    Some(visible_differences(differences))
}

pub(crate) fn history_json_difference_rows(
    palette: Palette,
    differences: &[JsonDifference],
    hidden_count: usize,
) -> View {
    let rows = differences
        .iter()
        .enumerate()
        .map(|(index, difference)| {
            let color = match difference.kind {
                JsonDifferenceKind::Added => palette.ok,
                JsonDifferenceKind::Removed => palette.err,
                JsonDifferenceKind::Modified | JsonDifferenceKind::TypeChanged => palette.text,
            };
            let formatted = difference.formatted();
            KeyedView::new(
                format!("json-difference-{index}"),
                TextBlock::new()
                    .text(formatted.clone())
                    .font_size(11.0)
                    .foreground(color)
                    .text_wrapping(TextWrapping::Wrap)
                    .automation_name(format!("History JSON difference: {formatted}")),
            )
        })
        .collect::<Vec<_>>();
    let overflow: View = if hidden_count == 0 {
        View::empty()
    } else {
        let label = format!("…and {hidden_count} more changes");
        TextBlock::new()
            .text(label.clone())
            .font_size(11.0)
            .foreground(palette.muted)
            .automation_name(format!("History JSON differences: {label}"))
            .into()
    };
    StackPanel::new()
        .spacing(2.0)
        .children((StackPanel::new().keyed_children(rows), overflow))
}

pub(crate) fn history_task_output(palette: Palette, title: &str, output: &str) -> View {
    let output = if output.is_empty() { "(empty)" } else { output };
    StackPanel::new().spacing(4.0).children((
        TextBlock::new()
            .text(title.to_string())
            .font_size(10.0)
            .font_weight(FontWeight::BOLD)
            .foreground(palette.muted),
        Border::new()
            .min_height(70.0)
            .padding(Thickness::new(7.0, 6.0, 7.0, 6.0))
            .background(palette.card)
            .corner_radius(4.0)
            .content(
                ScrollViewer::new()
                    .max_height(180.0)
                    .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .content(
                        TextBlock::new()
                            .text(output.to_string())
                            .font_size(10.5)
                            .text_wrapping(TextWrapping::Wrap),
                    ),
            ),
    ))
}

pub(crate) fn history_comparison_placeholder(
    selected_id: Option<&str>,
    latest_id: Option<&str>,
    comparison_loading: bool,
    error: Option<&str>,
) -> String {
    let Some(selected_id) = selected_id else {
        return "Select a scan to compare it against the latest.".to_string();
    };
    if Some(selected_id) == latest_id {
        return "The latest scan is the comparison baseline. Select an earlier scan to compare."
            .to_string();
    }
    if comparison_loading {
        return "Loading comparison…".to_string();
    }
    if let Some(error) = error {
        return format!("Could not compare the selected scan: {error}");
    }
    "Comparison is unavailable. Select the scan again to retry.".to_string()
}

#[cfg(feature = "validation")]
pub(crate) fn history_fixture_page(palette: Palette, narrow: bool, empty: bool) -> View {
    if empty {
        return history_empty_page(palette, narrow);
    }

    let session_rows = [
        ("7/12/2026, 8:25:35\nPM", "1.9s", false, true),
        ("7/12/2026, 5:18:09\nPM", "2.0s", true, false),
        ("7/12/2026, 5:15:57\nPM", "1.9s", false, false),
        ("7/12/2026, 5:15:47\nPM", "2.0s", false, false),
        ("7/11/2026, 7:20:04\nPM", "1.8s", false, false),
        ("7/11/2026, 7:19:28\nPM", "1.8s", false, false),
        ("7/11/2026, 10:15:58\nAM", "1.8s", false, false),
        ("7/10/2026, 10:35:45\nPM", "1.8s", false, false),
        ("7/10/2026, 10:35:08\nPM", "1.8s", false, false),
        ("7/10/2026, 5:47:09\nPM", "1.7s", false, false),
        ("7/10/2026, 5:45:55\nPM", "1.9s", false, false),
        ("7/10/2026, 3:38:30\nPM", "1.9s", false, false),
        ("7/10/2026, 3:17:32\nPM", "1.8s", false, false),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (timestamp, time, selected, latest))| {
        KeyedView::new(
            index.to_string(),
            history_row(
                palette,
                timestamp,
                "Quick Scan",
                "17",
                "—",
                time,
                selected,
                latest,
            ),
        )
    })
    .collect::<Vec<_>>();

    let sessions = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().children((
                Border::new()
                    .height(45.0)
                    .padding(Thickness::xy(18.0, 0.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([
                                GridLength::Pixel(3.0),
                                GridLength::Star(1.0),
                                GridLength::Auto,
                            ])
                            .column_spacing(10.0)
                            .children((
                                Border::new()
                                    .width(3.0)
                                    .height(15.0)
                                    .background(palette.accent)
                                    .corner_radius(999.0)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Scan Sessions")
                                    .grid_column(1)
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Click to compare vs latest")
                                    .grid_column(2)
                                    .font_size(11.0)
                                    .foreground(palette.muted)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                    ),
                history_header(palette),
                StackPanel::new().keyed_children(session_rows),
            )),
        );
    let diff = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().children((
                Border::new()
                    .height(45.0)
                    .padding(Thickness::xy(18.0, 0.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Pixel(3.0), GridLength::Star(1.0)])
                            .column_spacing(10.0)
                            .children((
                                Border::new()
                                    .width(3.0)
                                    .height(15.0)
                                    .background(palette.accent)
                                    .corner_radius(999.0)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Diff vs latest")
                                    .grid_column(1)
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                    ),
                Border::new()
                    .padding(Thickness::new(14.0, 13.0, 14.0, 14.0))
                    .content(StackPanel::new().spacing(9.0).children((
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(8.0)
                            .children((
                                TextBlock::new()
                                    .text("Label:")
                                    .font_size(12.0)
                                    .foreground(palette.muted),
                                TextBlock::new()
                                    .text("Quick Scan")
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD),
                                icons::path(FaIcon::Pen).width(12.0).height(12.0),
                            )),
                        TextBlock::new()
                            .text("Comparing 7/12/2026, 5:18:09 PM against the latest scan — 6 changes.")
                            .text_wrapping(TextWrapping::Wrap)
                            .font_size(12.0),
                        history_metric(palette, "New collection errors", "0", palette.err),
                        history_metric(palette, "Newly collected", "0", palette.ok),
                        history_metric(palette, "Output changed", "6", palette.text),
                        history_change(palette, "changed", "System Services", None, palette.accent, palette.active, false),
                        history_change(palette, "changed", "Logical Disks", None, palette.accent, palette.active, false),
                        history_change(palette, "changed", "Operating System", None, palette.accent, palette.active, false),
                        history_change(palette, "changed", "Processor", None, palette.accent, palette.active, false),
                        history_change(palette, "changed", "System Information", None, palette.accent, palette.active, false),
                        history_change(palette, "changed", "Startup Commands", None, palette.accent, palette.active, false),
                    ))),
            )),
        );
    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((sessions, diff))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.4), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((
                sessions,
                Border::new()
                    .grid_column(1)
                    .vertical_alignment(VerticalAlignment::Top)
                    .content(diff),
            ))
    };
    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::History, View::empty()),
        Grid::new()
            .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        TextBlock::new()
                            .text("27 scans")
                            .font_size(12.0)
                            .vertical_alignment(VerticalAlignment::Center),
                        TextBox::new()
                            .width(260.0)
                            .height(32.0)
                            .placeholder_text("Filter by label, date, machine…"),
                    )),
                StackPanel::new()
                    .grid_column(1)
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        Button::new()
                            .width(110.0)
                            .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
                        Button::new()
                            .width(147.0)
                            .resource_overrides(
                                ResourceOverrides::new()
                                    .set("ButtonForeground", palette.err)
                                    .set("ButtonBackground", palette.card)
                                    .set("ButtonBackgroundPointerOver", palette.err_bg)
                                    .set("ButtonBackgroundPressed", palette.err_bg),
                            )
                            .content(fa_icon_label(FaIcon::Trash, "Clear history")),
                    )),
            )),
        body,
    ))
}

// Fixture-only: the live page renders its own empty state.
#[cfg(feature = "validation")]
pub(crate) fn history_empty_page(palette: Palette, narrow: bool) -> View {
    let sessions = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().children((
                Border::new()
                    .height(45.0)
                    .padding(Thickness::xy(18.0, 0.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([
                                GridLength::Pixel(3.0),
                                GridLength::Star(1.0),
                                GridLength::Auto,
                            ])
                            .column_spacing(10.0)
                            .children((
                                Border::new()
                                    .width(3.0)
                                    .height(15.0)
                                    .background(palette.accent)
                                    .corner_radius(999.0)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Scan Sessions")
                                    .grid_column(1)
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Click to compare vs latest")
                                    .grid_column(2)
                                    .font_size(11.0)
                                    .foreground(palette.muted)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                    ),
                history_header(palette),
                Border::new().height(246.0).content(
                    StackPanel::new()
                        .spacing(10.0)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Center)
                        .children((
                            Border::new()
                                .width(68.0)
                                .height(68.0)
                                .background(palette.active)
                                .corner_radius(17.0)
                                .content(icons::path(FaIcon::History).width(30.0).height(30.0)),
                            TextBlock::new()
                                .text("No saved scans yet")
                                .font_size(22.0)
                                .font_weight(FontWeight::BOLD)
                                .horizontal_alignment(HorizontalAlignment::Center),
                            TextBlock::new()
                                .text(
                                    "Run and save a scan to start tracking drift between sessions.",
                                )
                                .font_size(12.5)
                                .foreground(palette.muted)
                                .horizontal_alignment(HorizontalAlignment::Center),
                        )),
                ),
            )),
        );

    let diff = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .vertical_alignment(VerticalAlignment::Top)
        .content(
            StackPanel::new().children((
                Border::new()
                    .height(45.0)
                    .padding(Thickness::xy(18.0, 0.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Pixel(3.0), GridLength::Star(1.0)])
                            .column_spacing(10.0)
                            .children((
                                Border::new()
                                    .width(3.0)
                                    .height(15.0)
                                    .background(palette.accent)
                                    .corner_radius(999.0)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Diff vs latest")
                                    .grid_column(1)
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                    ),
                Border::new()
                    .height(47.0)
                    .padding(Thickness::xy(14.0, 0.0))
                    .content(
                        TextBlock::new()
                            .text("Select a scan to compare it against the latest.")
                            .font_size(12.0)
                            .vertical_alignment(VerticalAlignment::Center),
                    ),
            )),
        );

    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((sessions, diff))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.4), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((sessions, placed(diff, 1, 0)))
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::History, View::empty()),
        Grid::new()
            .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        TextBlock::new()
                            .text("0 scans")
                            .font_size(12.0)
                            .vertical_alignment(VerticalAlignment::Center),
                        TextBox::new()
                            .width(260.0)
                            .height(32.0)
                            .placeholder_text("Filter by label, date, machine…"),
                    )),
                StackPanel::new()
                    .grid_column(1)
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        Button::new()
                            .width(110.0)
                            .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
                        Button::new()
                            .width(147.0)
                            .is_enabled(false)
                            .content(fa_icon_label(FaIcon::Trash, "Clear history")),
                    )),
            )),
        body,
    ))
}

pub(crate) fn history_columns() -> [GridLength; 6] {
    [
        GridLength::Pixel(22.0),
        GridLength::Pixel(132.0),
        GridLength::Star(1.0),
        GridLength::Pixel(72.0),
        GridLength::Pixel(58.0),
        GridLength::Pixel(60.0),
    ]
}

pub(crate) fn history_header(palette: Palette) -> View {
    Border::new()
        .height(43.0)
        .padding(Thickness::xy(14.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(Grid::new().columns(history_columns()).children((
            table_header("TIMESTAMP", 1),
            table_header("LABEL", 2),
            table_header("COLLECTED", 3),
            table_header("ERRORS", 4),
            table_header("TIME", 5),
        )))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_row(
    palette: Palette,
    timestamp: &str,
    label: &str,
    pass: &str,
    fail: &str,
    time: &str,
    selected: bool,
    latest: bool,
) -> View {
    let label_content: View = if latest {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(6.0)
            .vertical_alignment(VerticalAlignment::Center)
            .children((
                TextBlock::new()
                    .text(label)
                    .font_size(11.5)
                    .font_weight(FontWeight::BOLD),
                Border::new()
                    .height(18.0)
                    .padding(Thickness::xy(6.0, 0.0))
                    .background(palette.active)
                    .corner_radius(999.0)
                    .content(
                        TextBlock::new()
                            .text("Latest")
                            .font_size(10.5)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .foreground(palette.accent)
                            .vertical_alignment(VerticalAlignment::Center),
                    ),
            ))
    } else {
        TextBlock::new()
            .text(label)
            .font_size(11.5)
            .font_weight(FontWeight::BOLD)
            .into()
    };

    Border::new()
        .height(46.0)
        .padding(Thickness::xy(14.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .background(if selected {
            palette.active
        } else {
            Color::transparent()
        })
        .content(
            Grid::new().columns(history_columns()).children((
                Border::new()
                    .width(7.0)
                    .height(7.0)
                    .background(if fail == "—" {
                        palette.ok
                    } else {
                        palette.warn
                    })
                    .corner_radius(999.0)
                    .vertical_alignment(VerticalAlignment::Center),
                table_cell(timestamp, 1).foreground(palette.muted),
                Border::new()
                    .grid_column(2)
                    .margin(Thickness::xy(8.0, 0.0))
                    .vertical_alignment(VerticalAlignment::Center)
                    .content(label_content),
                table_cell(pass, 3).foreground(palette.accent),
                table_cell(fail, 4).foreground(palette.err),
                table_cell(time, 5),
            )),
        )
}

pub(crate) fn history_metric(
    palette: Palette,
    label: impl Into<String>,
    value: impl Into<String>,
    color: Color,
) -> View {
    let label = label.into();
    let value = value.into();
    Border::new()
        .height(31.0)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .children((
                    TextBlock::new()
                        .text(label)
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(value)
                        .grid_column(1)
                        .font_size(11.5)
                        .foreground(color)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

pub(crate) fn history_change(
    palette: Palette,
    kind: impl Into<String>,
    label: impl Into<String>,
    trend_badge: Option<&HistoryTrendBadge>,
    color: Color,
    background: Color,
    expanded: bool,
) -> View {
    let kind = kind.into();
    let label = label.into();
    let trend_badge = trend_badge.map_or_else(View::empty, |badge| {
        Border::new()
            .height(21.0)
            .padding(Thickness::xy(7.0, 0.0))
            .background(palette.warn_bg)
            .corner_radius(999.0)
            .vertical_alignment(VerticalAlignment::Center)
            .automation_name(badge.description.clone())
            .content(
                TextBlock::new()
                    .text(badge.label.clone())
                    .font_size(10.0)
                    .font_weight(FontWeight::BOLD)
                    .foreground(palette.warn)
                    .vertical_alignment(VerticalAlignment::Center),
            )
            .tooltip(badge.description.clone())
    });
    Grid::new()
        .height(36.0)
        .columns([
            GridLength::Pixel(16.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ])
        .column_spacing(8.0)
        .children((
            icons::path(if expanded {
                FaIcon::ChevronDown
            } else {
                FaIcon::ChevronRight
            })
            .width(12.0)
            .height(12.0)
            .vertical_alignment(VerticalAlignment::Center),
            Border::new()
                .grid_column(1)
                .height(21.0)
                .padding(Thickness::xy(10.0, 0.0))
                .background(background)
                .corner_radius(999.0)
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    TextBlock::new()
                        .text(kind)
                        .font_size(10.0)
                        .font_weight(FontWeight::BOLD)
                        .foreground(color)
                        .vertical_alignment(VerticalAlignment::Center),
                ),
            TextBlock::new()
                .text(label)
                .grid_column(2)
                .font_size(11.5)
                .font_weight(FontWeight::SEMI_BOLD)
                .vertical_alignment(VerticalAlignment::Center),
            placed(trend_badge, 3, 0),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_history::TaskDiffDetail;

    #[test]
    fn history_json_projection_uses_raw_fallback_and_caps_structured_rows() {
        let malformed = HistoryTaskDiffProjection::from(TaskDiffDetail {
            task_id: "malformed".to_string(),
            previous_output: "not json".to_string(),
            current_output: "still not json".to_string(),
        });
        assert!(malformed.differences.is_none());
        assert!(history_visible_json_differences(malformed.differences.as_deref()).is_none());

        let identical = HistoryTaskDiffProjection::from(TaskDiffDetail {
            task_id: "identical".to_string(),
            previous_output: r#"{"value":1}"#.to_string(),
            current_output: r#"{"value":1}"#.to_string(),
        });
        assert_eq!(identical.differences.as_deref(), Some([].as_slice()));
        assert!(history_visible_json_differences(identical.differences.as_deref()).is_none());

        let previous = format!(
            "[{}]",
            std::iter::repeat_n("0", 13).collect::<Vec<_>>().join(",")
        );
        let current = format!(
            "[{}]",
            std::iter::repeat_n("1", 13).collect::<Vec<_>>().join(",")
        );
        let changed = HistoryTaskDiffProjection::from(TaskDiffDetail {
            task_id: "changed".to_string(),
            previous_output: previous,
            current_output: current.clone(),
        });
        let (visible, hidden) = history_visible_json_differences(changed.differences.as_deref())
            .expect("valid changed JSON has a structured projection");
        assert_eq!(visible.len(), 12);
        assert_eq!(hidden, 1);
        assert_eq!(changed.detail.current_output, current);
    }
}
