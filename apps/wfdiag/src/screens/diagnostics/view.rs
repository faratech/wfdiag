//! The Diagnostics page and its result surfaces.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    BOT_AVATAR, DESKTOP_DARK, DESKTOP_LIGHT, STATUS_OK_DARK, STATUS_OK_LIGHT, STATUS_WARN_DARK,
    STATUS_WARN_LIGHT, STETHOSCOPE_DARK, STETHOSCOPE_LIGHT, WAND_DARK, WAND_LIGHT,
};
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::app::state::{DiagnosticAnalysisDisplay, Page};
use crate::screens::ai::state::AiMsg;
use crate::screens::diagnostics::state::{DiagnosticsMsg, DiagnosticsScreen};
use crate::widgets::badges::icon_status_pill;
use crate::widgets::cards::statistic;
use crate::widgets::chrome::{fa_icon_label, page_header, placed};
use crate::widgets::icons::FaIcon;
use crate::widgets::markdown_render::{MarkdownStyle, render_markdown_lite};
use crate::widgets::palette_colors::Palette;
use std::collections::HashMap;
use wfdiag_native_ai_analysis::{GroundingTrace, GroundingTraceSource};
use wfdiag_native_diagnostics::DiagnosticTask;
use wfdiag_native_projection::markdown::safe_markdown_link_target;
use wfdiag_ui_core::{DiagnosticTaskResult, TaskProgressStatus};
use windows_reactor::*;

pub(crate) fn format_diagnostic_duration(duration_ms: u64) -> String {
    if duration_ms == 0 {
        "—".to_string()
    } else if duration_ms >= 1_000 {
        format!("{:.1} s", duration_ms as f64 / 1_000.0)
    } else {
        format!("{duration_ms} ms")
    }
}

pub(crate) fn diagnostic_matches_filter(
    result: &DiagnosticTaskResult,
    catalog: &[DiagnosticTask],
    filter: &str,
) -> bool {
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return true;
    }
    let task = catalog.iter().find(|task| task.id == result.task_id);
    result.task_id.to_ascii_lowercase().contains(&filter)
        || task.is_some_and(|task| {
            task.name.to_ascii_lowercase().contains(&filter)
                || task.category.to_ascii_lowercase().contains(&filter)
                || task.description.to_ascii_lowercase().contains(&filter)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagnosticCategoryProgress {
    pub(crate) category: String,
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) running: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
}

impl DiagnosticCategoryProgress {
    pub(crate) fn terminal(&self) -> usize {
        self.completed + self.failed + self.cancelled
    }
}

pub(crate) fn diagnostic_category_progress(
    catalog: &[DiagnosticTask],
    expected_task_ids: &[String],
    task_statuses: &HashMap<String, TaskProgressStatus>,
) -> Vec<DiagnosticCategoryProgress> {
    let mut categories: Vec<DiagnosticCategoryProgress> = Vec::new();
    for task_id in expected_task_ids {
        let category = catalog
            .iter()
            .find(|task| task.id == *task_id)
            .map_or("Other", |task| task.category.as_str());
        let index = categories
            .iter()
            .position(|progress| progress.category == category)
            .unwrap_or_else(|| {
                categories.push(DiagnosticCategoryProgress {
                    category: category.to_string(),
                    total: 0,
                    completed: 0,
                    running: 0,
                    failed: 0,
                    cancelled: 0,
                });
                categories.len() - 1
            });
        let progress = &mut categories[index];
        progress.total += 1;
        match task_statuses
            .get(task_id)
            .copied()
            .unwrap_or(TaskProgressStatus::Queued)
        {
            TaskProgressStatus::Queued => {}
            TaskProgressStatus::Running => progress.running += 1,
            TaskProgressStatus::Completed => progress.completed += 1,
            TaskProgressStatus::Failed => progress.failed += 1,
            TaskProgressStatus::Cancelled => progress.cancelled += 1,
        }
    }
    categories
}

pub(crate) fn diagnostic_category_progress_chip(
    palette: Palette,
    progress: &DiagnosticCategoryProgress,
) -> View {
    let terminal = progress.terminal();
    let (foreground, background, state) = if progress.failed > 0 {
        (palette.err, palette.err_bg, "failed")
    } else if progress.cancelled > 0 {
        (palette.warn, palette.warn_bg, "cancelled")
    } else if terminal == progress.total {
        (palette.ok, palette.ok_bg, "complete")
    } else if progress.running > 0 {
        (palette.accent, palette.active, "running")
    } else {
        (palette.muted, palette.dim, "queued")
    };
    let label = format!("{} {terminal}/{}", progress.category, progress.total);

    Border::new()
        .height(25.0)
        .margin(Thickness::new(0.0, 0.0, 6.0, 6.0))
        .padding(Thickness::xy(10.0, 0.0))
        .background(background)
        .corner_radius(999.0)
        .content(
            TextBlock::new()
                .text(label.clone())
                .font_size(10.5)
                .font_weight(FontWeight::SEMI_BOLD)
                .foreground(foreground)
                .vertical_alignment(VerticalAlignment::Center)
                .automation_name(format!("{label}, {state}")),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn diagnostics_scanning_page(
    palette: Palette,
    catalog: &[DiagnosticTask],
    expected_task_ids: &[String],
    task_statuses: &HashMap<String, TaskProgressStatus>,
    completed: usize,
    total: usize,
    current_task: Option<&str>,
    cancelling: bool,
    cancel_scan: Callback<()>,
) -> View {
    let progress = if total == 0 {
        0.0
    } else {
        (completed as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    };
    let activity = if cancelling {
        "Stopping after in-flight checks finish…".to_string()
    } else {
        current_task.unwrap_or("Starting…").to_string()
    };
    let progress_text = if total == 0 {
        "0%".to_string()
    } else {
        format!("{progress:.0}%")
    };
    let category_chips = diagnostic_category_progress(catalog, expected_task_ids, task_statuses)
        .into_iter()
        .map(|progress| {
            let key = progress.category.clone();
            (key, diagnostic_category_progress_chip(palette, &progress))
        })
        .collect::<Vec<_>>();

    let hero = StackPanel::new()
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .spacing(0.0)
        .children((
            Grid::new().width(124.0).height(124.0).children((
                ProgressRing::new()
                    .width(124.0)
                    .height(124.0)
                    .minimum(0.0)
                    .maximum(100.0)
                    .value(progress)
                    .is_active(true),
                TextBlock::new()
                    .text(progress_text)
                    .font_size(20.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .vertical_alignment(VerticalAlignment::Center),
            )),
            TextBlock::new()
                .text("Scanning this PC…")
                .margin(Thickness::new(0.0, 20.0, 0.0, 0.0))
                .font_size(22.0)
                .font_weight(FontWeight::BOLD)
                .horizontal_alignment(HorizontalAlignment::Center)
                .automation_heading_level(AutomationHeadingLevel::Level2),
            TextBlock::new()
                .text(activity)
                .margin(Thickness::new(0.0, 9.0, 0.0, 0.0))
                .font_size(13.0)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center),
            ProgressBar::new()
                .width(280.0)
                .height(4.0)
                .margin(Thickness::new(0.0, 14.0, 0.0, 0.0))
                .minimum(0.0)
                .maximum(100.0)
                .value(progress),
            TextBlock::new()
                .text(format!("{completed} of {total} diagnostics collected"))
                .margin(Thickness::new(0.0, 9.0, 0.0, 0.0))
                .font_size(11.5)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center),
            VariableSizedWrapGrid::new()
                .margin(Thickness::new(0.0, 14.0, 0.0, 0.0))
                .max_width(640.0)
                .orientation(Orientation::Horizontal)
                .item_height(31.0)
                .horizontal_alignment(HorizontalAlignment::Center)
                .keyed_children(category_chips),
            Button::new()
                .height(33.0)
                .margin(Thickness::new(0.0, 18.0, 0.0, 0.0))
                .is_enabled(!cancelling)
                .on_click(cancel_scan)
                .automation_name("Stop scan")
                .content(fa_icon_label(
                    FaIcon::Xmark,
                    if cancelling {
                        "Stopping…"
                    } else {
                        "Stop scan"
                    },
                )),
        ));

    Grid::new()
        .rows([GridLength::Auto, GridLength::Star(1.0)])
        .children((
            page_header(palette, Page::Diagnostics, View::empty()),
            Border::new()
                .grid_row(1)
                .padding(Thickness::new(0.0, 0.0, 0.0, 18.0))
                .content(hero),
        ))
}

pub(crate) fn live_collected_statistic(
    palette: Palette,
    collected: usize,
    completed: usize,
) -> View {
    StackPanel::new().spacing(1.0).children((
        TextBlock::new()
            .text("COLLECTED")
            .font_size(10.5)
            .font_weight(FontWeight::SEMI_BOLD)
            .foreground(palette.muted),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(3.0)
            .children((
                TextBlock::new()
                    .text(collected.to_string())
                    .font_size(21.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.accent),
                TextBlock::new()
                    .text(format!("/ {completed}"))
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .margin(Thickness::new(0.0, 7.0, 0.0, 0.0)),
            )),
    ))
}

pub(crate) const MAX_DIAGNOSTIC_PREVIEW_CHARS: usize = 48_000;

pub(crate) const MAX_STRUCTURED_OUTPUT_INPUT_BYTES: usize = 128 * 1024;

pub(crate) const MAX_STRUCTURED_OUTPUT_ROWS: usize = 256;

pub(crate) const MAX_STRUCTURED_OUTPUT_BYTES: usize = 48 * 1024;

pub(crate) const MAX_RAW_PRETTY_INPUT_BYTES: usize = 32 * 1024;

pub(crate) const MAX_RAW_FALLBACK_BYTES: usize = 24 * 1024;

pub(crate) const RAW_OUTPUT_TRUNCATION_NOTICE: &str =
    "… Raw output truncated before formatting; the complete result remains available for export.";

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct FormattedOutputRows {
    pub(crate) rows: Vec<(String, String)>,
    pub(crate) byte_len: usize,
    pub(crate) truncated: bool,
}

impl FormattedOutputRows {
    pub(crate) fn new() -> Self {
        Self {
            rows: Vec::new(),
            byte_len: 0,
            truncated: false,
        }
    }

    pub(crate) fn accepts(&mut self, key_bytes: usize, value_bytes: usize) -> bool {
        if self.truncated {
            return false;
        }
        let row_bytes = key_bytes.saturating_add(value_bytes);
        if self.rows.len() >= MAX_STRUCTURED_OUTPUT_ROWS
            || self.byte_len.saturating_add(row_bytes) > MAX_STRUCTURED_OUTPUT_BYTES
        {
            self.truncated = true;
            return false;
        }
        self.byte_len += row_bytes;
        true
    }

    pub(crate) fn push(&mut self, key: String, value: String) {
        if !self.accepts(key.len(), value.len()) {
            return;
        }
        self.rows.push((key, value));
    }

    pub(crate) fn push_str(&mut self, key: String, value: &str) {
        if !self.accepts(key.len(), value.len()) {
            return;
        }
        self.rows.push((key, value.to_string()));
    }
}

pub(crate) fn bounded_utf8_prefix(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes.min(text.len());
    while end != 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

pub(crate) fn oversized_structured_preview(output: &str) -> FormattedOutputRows {
    let key = "Output preview".to_string();
    let value_budget = MAX_STRUCTURED_OUTPUT_BYTES.saturating_sub(key.len());
    let (preview, _) = bounded_utf8_prefix(output, value_budget);
    FormattedOutputRows {
        byte_len: key.len().saturating_add(preview.len()),
        rows: vec![(key, preview.to_string())],
        truncated: true,
    }
}

/// Mirror of the shipping detail view's output conversion: JSON objects
/// flatten into human-facing key/value rows ("group · key"); non-JSON
/// output stays raw text. Parsing and row materialization are bounded before
/// touching the complete collector payload.
pub(crate) fn format_output_key_values(task_id: &str, output: &str) -> Option<FormattedOutputRows> {
    let oversized = output.len() > MAX_STRUCTURED_OUTPUT_INPUT_BYTES;
    let candidate = if oversized {
        bounded_utf8_prefix(output, MAX_STRUCTURED_OUTPUT_INPUT_BYTES).0
    } else {
        output
    };
    // Collector output decoded from PowerShell may carry a leading BOM
    // (U+FEFF), which str::trim does not remove; strip it explicitly.
    let trimmed = candidate.trim().trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    if oversized {
        return Some(oversized_structured_preview(trimmed));
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let mut rows = FormattedOutputRows::new();
    fn flatten(prefix: &str, value: &serde_json::Value, rows: &mut FormattedOutputRows) {
        if rows.truncated {
            return;
        }
        match value {
            serde_json::Value::Object(map) => {
                for (key, entry) in map {
                    if rows.truncated {
                        break;
                    }
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix} · {key}")
                    };
                    flatten(&path, entry, rows);
                }
            }
            // Mirror the shipping detail view: arrays flatten through
            // Object.entries semantics — each item gets an index path
            // ("0 · key"), scalars join inline.
            serde_json::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if rows.truncated {
                        break;
                    }
                    let path = if prefix.is_empty() {
                        index.to_string()
                    } else {
                        format!("{prefix} · {index}")
                    };
                    match item {
                        serde_json::Value::String(text) => {
                            rows.push_str(path, text);
                        }
                        serde_json::Value::Number(number) => {
                            rows.push(path, number.to_string());
                        }
                        serde_json::Value::Bool(flag) => {
                            rows.push(path, flag.to_string());
                        }
                        other => flatten(&path, other, rows),
                    }
                }
            }
            serde_json::Value::Null => {
                rows.push(prefix.to_string(), String::new());
            }
            serde_json::Value::String(text) => {
                rows.push_str(prefix.to_string(), text);
            }
            other => rows.push(prefix.to_string(), other.to_string()),
        }
    }
    // pending_reboot's raw schema needs the same explanation as the
    // shipping detail view: expose restart state instead of raw flags.
    if task_id == "pending_reboot" {
        let serde_json::Value::Object(map) = &value else {
            return Some(FormattedOutputRows::new());
        };
        let reasons: Vec<String> = map
            .get("reasons")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default();
        let high_confidence = reasons
            .iter()
            .any(|reason| reason == "windows_update" || reason == "component_based_servicing");
        let legacy_deferred = reasons
            .iter()
            .any(|reason| reason == "pending_file_rename" || reason == "pending_file_operations");
        let explicit = map
            .get("restart_required")
            .and_then(|value| value.as_bool());
        let legacy = map.get("pending").and_then(|value| value.as_bool());
        let restart_required = explicit.unwrap_or(match legacy {
            Some(legacy_pending) => {
                (legacy_pending && high_confidence)
                    || (legacy_pending && legacy_deferred && !high_confidence)
            }
            None => false,
        });
        rows.push(
            "Restart required".to_string(),
            if restart_required { "Yes" } else { "No" }.to_string(),
        );
        if !reasons.is_empty() {
            let required_by: Vec<&str> = reasons
                .iter()
                .flat_map(|reason| match reason.as_str() {
                    "windows_update" => vec!["Windows Update"],
                    "component_based_servicing" => vec!["Windows component servicing"],
                    _ => Vec::new(),
                })
                .collect();
            if !required_by.is_empty() {
                rows.push("Required by".to_string(), required_by.join(", "));
            }
        }
        for (key, entry) in map {
            if rows.truncated {
                break;
            }
            if key == "restart_required" || key == "pending" || key == "reasons" {
                continue;
            }
            let path = key.clone();
            flatten(&path, entry, &mut rows);
        }
        return Some(rows);
    }
    flatten("", &value, &mut rows);
    (!rows.rows.is_empty() || rows.truncated).then_some(rows)
}

pub(crate) fn diagnostic_output_preview(result: &DiagnosticTaskResult) -> String {
    visible_text_preview(&result.output, "(no output)")
}

pub(crate) fn visible_text_preview(text: &str, empty_label: &str) -> String {
    if text.is_empty() {
        return empty_label.to_string();
    }

    let mut preview: String = text
        .chars()
        .take(MAX_DIAGNOSTIC_PREVIEW_CHARS + 1)
        .collect();
    if preview.chars().count() > MAX_DIAGNOSTIC_PREVIEW_CHARS {
        preview = preview.chars().take(MAX_DIAGNOSTIC_PREVIEW_CHARS).collect();
        preview.push_str("\n\n… Output preview truncated; the complete result remains in memory.");
    }
    preview
}

pub(crate) fn diagnostic_raw_document(
    result: &DiagnosticTaskResult,
    task: Option<&DiagnosticTask>,
) -> String {
    let source_is_bounded = result.output.len() <= MAX_RAW_PRETTY_INPUT_BYTES;
    let output = if source_is_bounded {
        let trimmed = result.output.trim().trim_start_matches('\u{feff}').trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            serde_json::from_str::<serde_json::Value>(trimmed)
                .unwrap_or_else(|_| serde_json::Value::String(result.output.clone()))
        } else {
            serde_json::Value::String(result.output.clone())
        }
    } else {
        let (source_preview, _) = bounded_utf8_prefix(&result.output, MAX_RAW_FALLBACK_BYTES);
        let mut preview = String::with_capacity(
            source_preview
                .len()
                .saturating_add(RAW_OUTPUT_TRUNCATION_NOTICE.len())
                .saturating_add(2),
        );
        preview.push_str(source_preview);
        preview.push_str("\n\n");
        preview.push_str(RAW_OUTPUT_TRUNCATION_NOTICE);
        serde_json::Value::String(preview)
    };
    let mut document = serde_json::json!({
        "task_id": result.task_id,
        "name": task.map_or(result.task_id.as_str(), |task| task.name.as_str()),
        "category": task.map_or("Other", |task| task.category.as_str()),
        "success": result.success,
        "duration_ms": result.duration_ms,
        "admin_required": task.is_some_and(|task| task.admin_required),
        "error": result.error,
        "output": output,
    });
    if !source_is_bounded {
        document
            .as_object_mut()
            .expect("diagnostic raw document is an object")
            .insert(
                "output_truncated".to_string(),
                serde_json::Value::Bool(true),
            );
    }
    serde_json::to_string_pretty(&document)
        .expect("diagnostic raw document contains only serializable values")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticGroundingSourceProjection {
    pub(crate) title: String,
    pub(crate) source: String,
    pub(crate) target: Option<String>,
}

pub(crate) fn safe_diagnostic_grounding_link_target(raw: &str) -> Option<String> {
    let target = safe_markdown_link_target(raw)?;
    let scheme = target.split_once(':')?.0;
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")).then_some(target)
}

pub(crate) fn project_diagnostic_grounding_source(
    source: &GroundingTraceSource,
) -> DiagnosticGroundingSourceProjection {
    DiagnosticGroundingSourceProjection {
        title: source.title.clone(),
        source: source.source.clone(),
        target: source
            .url
            .as_deref()
            .and_then(safe_diagnostic_grounding_link_target),
    }
}

pub(crate) fn diagnostic_grounding_source_row(source: &GroundingTraceSource) -> View {
    let source = project_diagnostic_grounding_source(source);
    let mut inlines = vec![RichTextInline::Run(RichTextRun::plain("• "))];
    if let Some(target) = source.target {
        inlines.push(RichTextInline::Hyperlink(RichTextHyperlink {
            text: source.title,
            uri: target,
        }));
    } else {
        inlines.push(RichTextInline::Run(RichTextRun::plain(source.title)));
    }
    if !source.source.trim().is_empty() {
        inlines.push(RichTextInline::Run(RichTextRun::plain(format!(
            " · {}",
            source.source
        ))));
    }
    RichTextBlock::new()
        .paragraphs(RichText::single_paragraph(inlines))
        .font_size(10.5)
        .is_text_selection_enabled(true)
        .text_wrapping(TextWrapping::Wrap)
        .into()
}

pub(crate) fn diagnostic_grounding_view(palette: Palette, trace: &GroundingTrace) -> View {
    let summary = if trace.source_count == 0 {
        trace.error.as_deref().map_or_else(
            || "Live grounding found no sources".to_string(),
            |error| format!("Live grounding unavailable · {error}"),
        )
    } else {
        let noun = if trace.source_count == 1 {
            "source"
        } else {
            "sources"
        };
        format!("Live grounding · {} {noun}", trace.source_count)
    };
    let error: View = if trace.source_count > 0 {
        trace.error.as_deref().map_or_else(View::empty, |error| {
            TextBlock::new()
                .text(error)
                .font_size(10.5)
                .foreground(palette.err)
                .is_text_selection_enabled(true)
                .text_wrapping(TextWrapping::Wrap)
                .into()
        })
    } else {
        View::empty()
    };
    let sources = trace
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            KeyedView::new(
                format!("diagnostic-grounding-source-{index}"),
                diagnostic_grounding_source_row(source),
            )
        })
        .collect::<Vec<_>>();
    let sources: View = if sources.is_empty() {
        View::empty()
    } else {
        StackPanel::new().spacing(2.0).keyed_children(sources)
    };
    StackPanel::new().spacing(3.0).children((
        TextBlock::new()
            .text(summary)
            .font_size(10.5)
            .foreground(palette.muted)
            .is_text_selection_enabled(true)
            .text_wrapping(TextWrapping::Wrap),
        error,
        sources,
    ))
}

pub(crate) fn diagnostic_analysis_panel(
    palette: Palette,
    analysis: Option<&DiagnosticAnalysisDisplay>,
    available: bool,
    analyze: Callback<()>,
    retry: Callback<()>,
    cancel: Callback<()>,
) -> View {
    if analysis.is_none() && !available {
        return View::empty();
    }
    let analysis = analysis.cloned().unwrap_or_default();
    let provider = analysis.provider_use.as_ref().map_or_else(
        || "AI Analysis".to_string(),
        |provider| {
            let model = provider
                .actual_models
                .first()
                .or(provider.requested_model.as_ref())
                .map(|model| format!(" · {model}"))
                .unwrap_or_default();
            let fallback = provider
                .fallback_from
                .as_deref()
                .map(|source| format!(" · fallback from {source}"))
                .unwrap_or_default();
            format!("AI Analysis · {}{model}{fallback}", provider.provider_id)
        },
    );
    let grounding: View = analysis
        .grounding
        .as_ref()
        .map_or_else(View::empty, |trace| {
            diagnostic_grounding_view(palette, trace)
        });
    let body: View = if analysis.busy {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(9.0)
            .children((
                ProgressRing::new().width(18.0).height(18.0).is_active(true),
                TextBlock::new()
                    .text("Interpreting the current diagnostic evidence…")
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .vertical_alignment(VerticalAlignment::Center),
                Button::new()
                    .on_click(cancel)
                    .automation_name("Cancel diagnostic analysis")
                    .content("Cancel"),
            ))
    } else if let Some(error) = analysis.error.as_deref() {
        StackPanel::new().spacing(8.0).children((
            TextBlock::new()
                .text(error)
                .font_size(12.0)
                .foreground(palette.err)
                .text_wrapping(TextWrapping::Wrap),
            Button::new()
                .horizontal_alignment(HorizontalAlignment::Left)
                .on_click(retry)
                .automation_name("Retry diagnostic analysis")
                .content("Retry analysis"),
        ))
    } else if let Some(interpretation) = analysis.interpretation.as_deref() {
        StackPanel::new().spacing(8.0).children((
            grounding,
            render_markdown_lite(
                interpretation,
                MarkdownStyle::with_palette(palette.text, palette.card_strong, palette.border),
            ),
            Button::new()
                .horizontal_alignment(HorizontalAlignment::Left)
                .on_click(retry)
                .automation_name("Retry diagnostic analysis")
                .content(if analysis.cached {
                    "Refresh cached analysis"
                } else {
                    "Retry analysis"
                }),
        ))
    } else {
        Button::new()
            .horizontal_alignment(HorizontalAlignment::Left)
            .on_click(analyze)
            .automation_name("Interpret this diagnostic")
            .content(fa_icon_label(
                FaIcon::WandMagicSparkles,
                "Interpret this diagnostic",
            ))
    };

    Border::new()
        .margin(Thickness::new(0.0, 16.0, 0.0, 0.0))
        .padding(Thickness::uniform(14.0))
        .background(palette.dim)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(8.0)
        .content(
            StackPanel::new().spacing(10.0).children((
                Grid::new()
                    .columns([GridLength::Pixel(24.0), GridLength::Star(1.0)])
                    .children((
                        Image::new()
                            .source_data(EncodedImage::from_static(BOT_AVATAR))
                            .width(20.0)
                            .height(20.0),
                        TextBlock::new()
                            .text(provider)
                            .grid_column(1)
                            .font_size(12.0)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
                body,
            )),
        )
}

pub(crate) fn diagnostic_output_mode_button(
    palette: Palette,
    label: &'static str,
    selected: bool,
    action: Callback<()>,
) -> View {
    Button::new()
        .height(28.0)
        .min_width(if label == "Output" { 70.0 } else { 56.0 })
        .on_click(action)
        .automation_name(format!("Show diagnostic {label}"))
        .resource_overrides(
            ResourceOverrides::new()
                .set(
                    "ButtonBackground",
                    if selected {
                        palette.active
                    } else {
                        Color::transparent()
                    },
                )
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.text)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(5.0)),
        )
        .content(label)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn diagnostics_live_results_page(
    palette: Palette,
    theme: WindowTheme,
    narrow: bool,
    results: &[DiagnosticTaskResult],
    catalog: &[DiagnosticTask],
    duration_ms: u64,
    selected_task_id: Option<&str>,
    select_result: Callback<String>,
    filter: &str,
    filter_changed: Callback<String>,
    raw_output: bool,
    show_output: Callback<()>,
    show_raw: Callback<()>,
    explain_scan: Callback<()>,
    analysis: Option<&DiagnosticAnalysisDisplay>,
    analysis_available: bool,
    analyze: Callback<()>,
    retry_analysis: Callback<()>,
    cancel_analysis: Callback<()>,
) -> View {
    let visible_results = results
        .iter()
        .filter(|result| diagnostic_matches_filter(result, catalog, filter))
        .collect::<Vec<_>>();
    if visible_results.is_empty() {
        return Grid::new()
            .rows([GridLength::Auto, GridLength::Star(1.0)])
            .children((
                page_header(palette, Page::Diagnostics, View::empty()),
                StackPanel::new()
                    .grid_row(1)
                    .spacing(16.0)
                    .margin(Thickness::new(0.0, 24.0, 0.0, 0.0))
                    .children((
                        TextBox::new()
                            .height(32.0)
                            .text(filter)
                            .placeholder_text("Filter diagnostics…")
                            .on_text_changed(filter_changed),
                        Border::new()
                            .padding(Thickness::uniform(28.0))
                            .background(palette.card)
                            .border_brush(palette.border)
                            .border_thickness(1.0)
                            .corner_radius(9.0)
                            .content(
                                TextBlock::new()
                                    .text("No diagnostics match this filter.")
                                    .foreground(palette.muted)
                                    .horizontal_alignment(HorizontalAlignment::Center),
                            ),
                    )),
            ));
    }
    let selected_task_id_effective = selected_task_id
        .filter(|id| visible_results.iter().any(|result| result.task_id == *id))
        .unwrap_or_else(|| visible_results[0].task_id.as_str());
    let collected = results.iter().filter(|result| result.success).count();
    let errors = results.len().saturating_sub(collected);
    let duration = format_diagnostic_duration(duration_ms);
    let wand_icon = if theme == WindowTheme::Light {
        WAND_LIGHT
    } else {
        WAND_DARK
    };
    let status_icon = if errors == 0 {
        if theme == WindowTheme::Light {
            STATUS_OK_LIGHT
        } else {
            STATUS_OK_DARK
        }
    } else if theme == WindowTheme::Light {
        STATUS_WARN_LIGHT
    } else {
        STATUS_WARN_DARK
    };

    let rows = visible_results
        .iter()
        .map(|result| {
            let task = catalog.iter().find(|task| task.id == result.task_id);
            (
                result.task_id.clone(),
                task.map_or_else(|| result.task_id.clone(), |task| task.name.clone()),
                task.map_or_else(|| "Other".to_string(), |task| task.category.clone()),
                result.success,
                result.duration_ms,
            )
        })
        .collect::<Vec<_>>();

    // Precompute grouping before the rows are consumed by the view build.
    let first_in_group_flags: Vec<bool> = rows
        .iter()
        .enumerate()
        .map(|(index, (_, _, category, _, _))| index == 0 || rows[index - 1].2 != *category)
        .collect();
    let group_counts: std::collections::HashMap<String, usize> = rows.iter().fold(
        std::collections::HashMap::new(),
        |mut counts, (_, _, category, _, _)| {
            *counts.entry(category.clone()).or_insert(0) += 1;
            counts
        },
    );

    let task_rows = rows
        .into_iter()
        .enumerate()
        .map(|(index, (task_id, name, category, passed, duration_ms))| {
            let select_result = select_result.clone();
            let first_in_group = first_in_group_flags[index];
            let group_count = group_counts.get(&category).copied().unwrap_or(0);
            let group_header: View = if first_in_group {
                Border::new()
                    .padding(Thickness::new(8.0, 11.0, 8.0, 5.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Star(1.0), GridLength::Auto])
                            .children((
                                TextBlock::new()
                                    .text(category.to_uppercase())
                                    .font_size(10.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(palette.muted),
                                TextBlock::new()
                                    .text(group_count.to_string())
                                    .grid_column(1)
                                    .font_size(10.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(palette.muted),
                            )),
                    )
            } else {
                View::empty()
            };
            let duration = format_diagnostic_duration(duration_ms);
            let is_selected = selected_task_id_effective == task_id;
            let row_background = if is_selected {
                palette.active
            } else {
                Color::transparent()
            };
            let name_weight = if is_selected {
                FontWeight::SEMI_BOLD
            } else {
                FontWeight::NORMAL
            };
            let pointer_task_id = task_id.clone();
            let pointer_select = select_result.clone();
            let task_row = Border::new()
                .height(32.0)
                .background(row_background)
                .corner_radius(6.0)
                // WinUI Button.Click fires on pointer release. Select on
                // pointer press as well so the native row acknowledges the
                // user's action immediately; Click remains for keyboard use.
                .on_pointer_pressed(move |_| {
                    let _ = pointer_select.call(pointer_task_id.clone());
                })
                .content(
                    Button::new()
                        .height(32.0)
                        .resource_overrides(
                            ResourceOverrides::new()
                                .set("ButtonBackground", Color::transparent())
                                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                .set("ButtonBackgroundPointerOver", palette.active)
                                .set("ButtonBackgroundPressed", palette.active),
                        )
                        .on_click({
                            let task_id = task_id.clone();
                            let select_result = select_result.clone();
                            move || {
                                let _ = select_result.call(task_id.clone());
                            }
                        })
                        .automation_name(format!("Diagnostic result: {name}"))
                        .content(
                            Grid::new()
                                .columns([
                                    GridLength::Pixel(7.0),
                                    GridLength::Star(1.0),
                                    GridLength::Auto,
                                ])
                                .column_spacing(9.0)
                                .children((
                                    Border::new()
                                        .width(7.0)
                                        .height(7.0)
                                        .background(if passed {
                                            palette.accent
                                        } else {
                                            palette.err
                                        })
                                        .corner_radius(999.0)
                                        .vertical_alignment(VerticalAlignment::Center),
                                    TextBlock::new()
                                        .text(name.clone())
                                        .grid_column(1)
                                        .font_size(12.5)
                                        .font_weight(name_weight)
                                        .text_trimming(TextTrimming::CharacterEllipsis)
                                        .vertical_alignment(VerticalAlignment::Center),
                                    TextBlock::new()
                                        .text(duration)
                                        .grid_column(2)
                                        .font_size(10.5)
                                        .foreground(palette.muted)
                                        .vertical_alignment(VerticalAlignment::Center),
                                )),
                        ),
                );
            KeyedView::new(
                task_id,
                StackPanel::new().children((group_header, task_row)),
            )
        })
        .collect::<Vec<_>>();

    let task_card = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            Grid::new()
                .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
                .children((
                    Border::new()
                        .padding(Thickness::new(14.0, 12.0, 14.0, 9.0))
                        .content(
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .children((
                                    TextBlock::new()
                                        .text(format!(
                                            "{} of {} diagnostics",
                                            visible_results.len(),
                                            results.len()
                                        ))
                                        .font_size(11.5)
                                        .foreground(palette.muted),
                                    TextBlock::new()
                                        .text(if errors == 0 {
                                            String::new()
                                        } else {
                                            format!("{errors} errors")
                                        })
                                        .grid_column(1)
                                        .font_size(11.5)
                                        .foreground(palette.err)
                                        .font_weight(FontWeight::SEMI_BOLD),
                                )),
                        ),
                    Border::new()
                        .grid_row(1)
                        .padding(Thickness::new(10.0, 0.0, 10.0, 8.0))
                        .content(
                            TextBox::new()
                                .height(30.0)
                                .text(filter)
                                .placeholder_text("Filter diagnostics…")
                                .on_text_changed(filter_changed),
                        ),
                    ScrollViewer::new()
                        .grid_row(2)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
                        .content(
                            Border::new()
                                .padding(Thickness::new(8.0, 0.0, 8.0, 10.0))
                                .content(StackPanel::new().keyed_children(task_rows)),
                        ),
                )),
        );

    let selected = visible_results
        .iter()
        .copied()
        .find(|result| result.task_id == selected_task_id_effective)
        .unwrap_or(visible_results[0]);
    let selected_task = catalog.iter().find(|task| task.id == selected.task_id);
    let selected_name =
        selected_task.map_or_else(|| selected.task_id.clone(), |task| task.name.clone());
    let selected_category = selected_task.map_or("Other", |task| task.category.as_str());
    let selected_admin_required = selected_task.is_some_and(|task| task.admin_required);
    let selected_duration = format_diagnostic_duration(selected.duration_ms);
    let selected_output_rows = (!raw_output)
        .then(|| format_output_key_values(&selected.task_id, &selected.output))
        .flatten();
    let desktop_icon = if theme == WindowTheme::Light {
        DESKTOP_LIGHT
    } else {
        DESKTOP_DARK
    };
    let selected_status = if selected.success {
        "● Collected"
    } else {
        "● Collection error"
    };
    let selected_status_color = if selected.success {
        palette.accent
    } else {
        palette.err
    };
    let selected_status_bg = if selected.success {
        palette.active
    } else {
        palette.err_bg
    };
    let selected_output_view: View = if let Some(rows) = selected_output_rows {
        let truncated = rows.truncated;
        let mut grid_rows: Vec<KeyedView> = rows
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, (key, value))| {
                KeyedView::new(
                    index,
                    Border::new()
                        .padding(Thickness::new(0.0, 4.0, 0.0, 4.0))
                        .border_brush(palette.border)
                        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 0.5))
                        .content(
                            Grid::new()
                                .columns([GridLength::Pixel(240.0), GridLength::Star(1.0)])
                                .children((
                                    TextBlock::new()
                                        .text(key)
                                        .font_size(12.0)
                                        .font_weight(FontWeight::SEMI_BOLD)
                                        .text_wrapping(TextWrapping::Wrap)
                                        .vertical_alignment(VerticalAlignment::Top),
                                    TextBlock::new()
                                        .text(value)
                                        .grid_column(1)
                                        .font_size(12.0)
                                        .foreground(palette.muted)
                                        .is_text_selection_enabled(true)
                                        .text_wrapping(TextWrapping::Wrap),
                                )),
                        ),
                )
            })
            .collect();
        if truncated {
            grid_rows.push(KeyedView::new(
                grid_rows.len(),
                Border::new()
                    .padding(Thickness::new(0.0, 8.0, 0.0, 2.0))
                    .content(
                        TextBlock::new()
                            .text(format!(
                                "… Structured output truncated at {MAX_STRUCTURED_OUTPUT_ROWS} rows or {} KiB; the complete result remains available for export.",
                                MAX_STRUCTURED_OUTPUT_BYTES / 1024
                            ))
                            .font_size(11.5)
                            .foreground(palette.warn)
                            .text_wrapping(TextWrapping::Wrap),
                    ),
            ));
        }
        StackPanel::new().keyed_children(grid_rows)
    } else {
        let selected_output = if raw_output {
            visible_text_preview(
                &diagnostic_raw_document(selected, selected_task),
                "(no raw diagnostic document)",
            )
        } else {
            diagnostic_output_preview(selected)
        };
        TextBlock::new()
            .text(selected_output)
            .font_size(12.0)
            .foreground(palette.text)
            .is_text_selection_enabled(true)
            .text_wrapping(TextWrapping::Wrap)
            .into()
    };
    let failure_callout: View = if selected.success {
        View::empty()
    } else {
        Border::new()
            .padding(Thickness::new(14.0, 12.0, 14.0, 12.0))
            .background(palette.err_bg)
            .border_brush(palette.err)
            .border_thickness(1.0)
            .corner_radius(6.0)
            .content(
                Grid::new()
                    .columns([GridLength::Pixel(24.0), GridLength::Star(1.0)])
                    .column_spacing(10.0)
                    .children((
                        TextBlock::new()
                            .text("!")
                            .font_size(18.0)
                            .font_weight(FontWeight::BOLD)
                            .foreground(palette.err),
                        StackPanel::new()
                            .grid_column(1)
                            .spacing(4.0)
                            .children((
                                TextBlock::new()
                                    .text(
                                        selected
                                            .error
                                            .as_deref()
                                            .filter(|error| !error.trim().is_empty())
                                            .unwrap_or("Diagnostic collection failed"),
                                    )
                                    .font_size(12.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(palette.err)
                                    .is_text_selection_enabled(true)
                                    .text_wrapping(TextWrapping::Wrap),
                                TextBlock::new()
                                    .text("This diagnostic could not complete. Administrator-only diagnostics require relaunching the app elevated.")
                                    .font_size(12.0)
                                    .foreground(palette.muted)
                                    .text_wrapping(TextWrapping::Wrap),
                            )),
                    )),
            )
    };
    let analysis_view = diagnostic_analysis_panel(
        palette,
        analysis,
        analysis_available,
        analyze,
        retry_analysis,
        cancel_analysis,
    );
    let output_tabs = Border::new()
        .horizontal_alignment(HorizontalAlignment::Left)
        .background(palette.card_strong)
        .corner_radius(6.0)
        .padding(Thickness::uniform(2.0))
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(2.0)
                .children((
                    diagnostic_output_mode_button(palette, "Output", !raw_output, show_output),
                    diagnostic_output_mode_button(palette, "Raw", raw_output, show_raw),
                )),
        );
    let admin_badge: View = if selected_admin_required {
        Border::new()
            .height(28.0)
            .padding(Thickness::xy(11.0, 0.0))
            .background(palette.warn_bg)
            .corner_radius(999.0)
            .content(
                TextBlock::new()
                    .text("◆ Admin")
                    .font_size(11.5)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.warn)
                    .vertical_alignment(VerticalAlignment::Center),
            )
    } else {
        View::empty()
    };
    let status_badges = StackPanel::new()
        .grid_column(2)
        .orientation(Orientation::Horizontal)
        .spacing(7.0)
        .vertical_alignment(VerticalAlignment::Center)
        .children((
            Border::new()
                .min_width(if selected.success { 100.0 } else { 132.0 })
                .height(28.0)
                .padding(Thickness::xy(12.0, 0.0))
                .background(selected_status_bg)
                .corner_radius(999.0)
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    TextBlock::new()
                        .text(selected_status)
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .foreground(selected_status_color)
                        .vertical_alignment(VerticalAlignment::Center),
                ),
            admin_badge,
        ));

    let detail_card = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            Grid::new()
                .rows([GridLength::Auto, GridLength::Star(1.0)])
                .children((
                    Border::new()
                        .min_height(65.0)
                        .padding(Thickness::new(20.0, 15.0, 20.0, 13.0))
                        .border_brush(palette.border)
                        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                        .content(
                            Grid::new()
                                .columns([
                                    GridLength::Pixel(36.0),
                                    GridLength::Star(1.0),
                                    GridLength::Auto,
                                ])
                                .children((
                                    Border::new()
                                        .width(36.0)
                                        .height(36.0)
                                        .background(palette.active)
                                        .corner_radius(9.0)
                                        .content(
                                            Image::new()
                                                .source_data(EncodedImage::from_static(
                                                    desktop_icon,
                                                ))
                                                .width(16.0)
                                                .height(16.0),
                                        ),
                                    StackPanel::new()
                                        .grid_column(1)
                                        .margin(Thickness::xy(12.0, 0.0))
                                        .spacing(2.0)
                                        .children((
                                            TextBlock::new()
                                                .text(selected_name)
                                                .font_size(15.0)
                                                .font_weight(FontWeight::BOLD),
                                            TextBlock::new()
                                                .text(format!(
                                                    "{selected_category} · completed in {selected_duration}"
                                                ))
                                                .font_size(11.5)
                                                .foreground(palette.muted),
                                        )),
                                    status_badges,
                                )),
                        ),
                    ScrollViewer::new()
                        .grid_row(1)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                        .content(
                            Border::new()
                                .padding(Thickness::uniform(16.0))
                                .content(
                                    StackPanel::new()
                                        .spacing(12.0)
                                        .children((
                                            failure_callout,
                                            output_tabs,
                                            selected_output_view,
                                            analysis_view,
                                        )),
                                ),
                        ),
                )),
        );

    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((
            Border::new().height(320.0).content(task_card),
            Border::new().height(520.0).content(detail_card),
        ))
    } else {
        Grid::new()
            .columns([GridLength::Pixel(295.0), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((task_card, placed(detail_card, 1, 0)))
    };

    let stats = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(28.0)
        .children((
            live_collected_statistic(palette, collected, results.len()),
            statistic(
                "ERRORS",
                &errors.to_string(),
                if errors == 0 {
                    palette.text
                } else {
                    palette.err
                },
            ),
            statistic("DURATION", &duration, palette.muted),
        ));
    let scan_color = if errors == 0 {
        palette.ok
    } else {
        palette.warn
    };
    let scan_background = if errors == 0 {
        palette.ok_bg
    } else {
        palette.warn_bg
    };

    Grid::new()
        .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
        .children((
            page_header(
                palette,
                Page::Diagnostics,
                Border::new()
                    .width(120.0)
                    .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
                    .content(icon_status_pill(
                        "Scan complete",
                        status_icon,
                        scan_color,
                        scan_background,
                    )),
            ),
            Border::new()
                .grid_row(1)
                .margin(Thickness::new(0.0, 24.0, 0.0, 0.0))
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .children((
                            stats,
                            Button::new()
                                .grid_column(1)
                                .width(154.0)
                                .height(33.0)
                                .on_click(explain_scan)
                                .automation_name("Explain this scan with AI")
                                .content(
                                    StackPanel::new()
                                        .orientation(Orientation::Horizontal)
                                        .spacing(8.0)
                                        .children((
                                            Image::new()
                                                .source_data(EncodedImage::from_static(wand_icon))
                                                .width(17.0)
                                                .height(16.0),
                                            TextBlock::new()
                                                .text("Explain this scan")
                                                .vertical_alignment(VerticalAlignment::Center),
                                        )),
                                ),
                        )),
                ),
            Border::new()
                .grid_row(2)
                .margin(Thickness::new(0.0, 16.0, 0.0, 0.0))
                .content(body),
        ))
}

impl DiagnosticsScreen {
    /// Paint the page from the screen's own state plus the chrome's env.
    ///
    /// `analysis_available` stays an argument because it is an AI-provider
    /// fact, which the AI screen owns; the root passes it in.
    pub(crate) fn view(
        &self,
        env: &ShellEnv<'_>,
        analysis_available: bool,
        vc: &mut ViewContext<WfdiagShell>,
    ) -> View {
        let selected_analysis = self
            .selected_task_id
            .as_deref()
            .or_else(|| self.results.first().map(|result| result.task_id.as_str()))
            .and_then(|task_id| self.analyses.get(task_id));
        diagnostics_page(
            env.palette,
            env.theme,
            env.compact,
            &self.results,
            &self.catalog,
            &self.expected_task_ids,
            &self.task_statuses,
            self.busy(),
            self.cancelling(),
            self.completed,
            self.total,
            self.current_task.as_deref(),
            self.duration_ms,
            vc.message(Message::Diagnostics(DiagnosticsMsg::RequestQuickScan)),
            vc.message(Message::Diagnostics(DiagnosticsMsg::RequestFullScan)),
            vc.message(Message::Diagnostics(DiagnosticsMsg::CancelScan)),
            vc.message(Message::Ai(AiMsg::ExplainLatestScan)),
            self.selected_task_id.clone(),
            vc.callback(|value| Message::Diagnostics(DiagnosticsMsg::SelectResult(value))),
            &self.filter,
            vc.callback(|value| Message::Diagnostics(DiagnosticsMsg::FilterChanged(value))),
            self.raw_output,
            vc.message(Message::Diagnostics(DiagnosticsMsg::SetRawOutput(false))),
            vc.message(Message::Diagnostics(DiagnosticsMsg::SetRawOutput(true))),
            selected_analysis,
            analysis_available,
            vc.message(Message::Diagnostics(DiagnosticsMsg::AnalyzeSelected)),
            vc.message(Message::Diagnostics(DiagnosticsMsg::RetrySelectedAnalysis)),
            vc.message(Message::Diagnostics(DiagnosticsMsg::CancelAnalysis)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn diagnostics_page(
    palette: Palette,
    theme: WindowTheme,
    narrow: bool,
    results: &[DiagnosticTaskResult],
    catalog: &[DiagnosticTask],
    expected_task_ids: &[String],
    task_statuses: &HashMap<String, TaskProgressStatus>,
    scan_active: bool,
    scan_cancelling: bool,
    completed: usize,
    total: usize,
    current_task: Option<&str>,
    duration_ms: u64,
    quick_scan: Callback<()>,
    full_scan: Callback<()>,
    cancel_scan: Callback<()>,
    explain_scan: Callback<()>,
    selected_result_task_id: Option<String>,
    select_result: Callback<String>,
    filter: &str,
    filter_changed: Callback<String>,
    raw_output: bool,
    show_output: Callback<()>,
    show_raw: Callback<()>,
    analysis: Option<&DiagnosticAnalysisDisplay>,
    analysis_available: bool,
    analyze: Callback<()>,
    retry_analysis: Callback<()>,
    cancel_analysis: Callback<()>,
) -> View {
    if scan_active {
        return diagnostics_scanning_page(
            palette,
            catalog,
            expected_task_ids,
            task_statuses,
            completed,
            total,
            current_task,
            scan_cancelling,
            cancel_scan,
        );
    }
    if results.is_empty() {
        return diagnostics_empty_page(palette, theme, quick_scan, full_scan);
    }
    if !results
        .iter()
        .any(|result| result.session_id == "visual-fixture")
    {
        return diagnostics_live_results_page(
            palette,
            theme,
            narrow,
            results,
            catalog,
            duration_ms,
            selected_result_task_id.as_deref(),
            select_result,
            filter,
            filter_changed,
            raw_output,
            show_output,
            show_raw,
            explain_scan,
            analysis,
            analysis_available,
            analyze,
            retry_analysis,
            cancel_analysis,
        );
    }

    let wand_icon = if theme == WindowTheme::Light {
        WAND_LIGHT
    } else {
        WAND_DARK
    };
    let desktop_icon = if theme == WindowTheme::Light {
        DESKTOP_LIGHT
    } else {
        DESKTOP_DARK
    };
    let ok_icon = if theme == WindowTheme::Light {
        STATUS_OK_LIGHT
    } else {
        STATUS_OK_DARK
    };

    let stats = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(28.0)
        .children((
            collected_statistic(palette),
            statistic("ERRORS", "0", palette.text),
            statistic("DURATION", "2.3s", palette.muted),
        ));

    let tasks = [
        ("SYSTEM", "6", "Computer System", "29 ms", true),
        ("", "", "Operating System", "262 ms", true),
        ("", "", "Startup Commands", "138 ms", true),
        ("", "", "System Information", "254 ms", true),
        ("", "", "System Services", "478 ms", true),
        ("", "", "Restart Requirements", "", true),
        ("HARDWARE", "3", "Processor", "1.7 s", true),
        ("", "", "Physical Memory", "15 ms", true),
        ("", "", "Device Errors", "312 ms", true),
        ("STORAGE", "2", "Disk Drives", "16 ms", true),
        ("", "", "Logical Disks", "18 ms", true),
        ("NETWORK", "2", "Network Adapters", "247 ms", true),
        ("", "", "HOSTS File", "8 ms", true),
        ("PERFORMANCE", "1", "Performance Data", "92 ms", true),
        ("SECURITY", "2", "Antivirus Status", "310 ms", true),
        ("", "", "Firewall Status", "36 ms", true),
        ("LOGS", "1", "Critical Event Codes", "418 ms", true),
    ];
    let task_rows = tasks
        .into_iter()
        .enumerate()
        .map(|(index, (group, count, name, time, passed))| {
            let group_header: View = if group.is_empty() {
                View::empty()
            } else {
                Border::new()
                    .padding(Thickness::new(8.0, 11.0, 8.0, 5.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Star(1.0), GridLength::Auto])
                            .children((
                                TextBlock::new()
                                    .text(group)
                                    .font_size(10.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(palette.muted),
                                TextBlock::new()
                                    .text(count)
                                    .grid_column(1)
                                    .font_size(10.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(palette.muted),
                            )),
                    )
            };

            let task = Border::new()
                .height(32.0)
                .background(if index == 0 {
                    palette.active
                } else {
                    Color::transparent()
                })
                .corner_radius(6.0)
                .padding(Thickness::xy(9.0, 0.0))
                .content(
                    Grid::new()
                        .columns([
                            GridLength::Pixel(7.0),
                            GridLength::Star(1.0),
                            GridLength::Auto,
                        ])
                        .column_spacing(9.0)
                        .children((
                            Border::new()
                                .width(7.0)
                                .height(7.0)
                                .background(if passed { palette.accent } else { palette.err })
                                .corner_radius(999.0)
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBlock::new()
                                .text(name)
                                .grid_column(1)
                                .font_size(12.5)
                                .font_weight(if index == 0 {
                                    FontWeight::SEMI_BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBlock::new()
                                .text(time)
                                .grid_column(2)
                                .font_size(10.5)
                                .foreground(palette.muted)
                                .vertical_alignment(VerticalAlignment::Center),
                        )),
                );

            KeyedView::new(
                index.to_string(),
                StackPanel::new().children((group_header, task)),
            )
        })
        .collect::<Vec<_>>();
    let task_card = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            Grid::new()
                .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
                .children((
                    Border::new()
                        .padding(Thickness::new(14.0, 12.0, 14.0, 9.0))
                        .content(
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .children((
                                    TextBlock::new()
                                        .text("17 of 17 diagnostics")
                                        .font_size(11.5)
                                        .foreground(palette.muted),
                                    TextBlock::new()
                                        .text("")
                                        .grid_column(1)
                                        .font_size(11.5)
                                        .foreground(palette.err)
                                        .font_weight(FontWeight::SEMI_BOLD),
                                )),
                        ),
                    Border::new()
                        .grid_row(1)
                        .padding(Thickness::new(10.0, 0.0, 10.0, 8.0))
                        .content(
                            TextBox::new()
                                .height(30.0)
                                .placeholder_text("Filter diagnostics…"),
                        ),
                    ScrollViewer::new()
                        .grid_row(2)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
                        .content(
                            Border::new()
                                .padding(Thickness::new(8.0, 0.0, 8.0, 10.0))
                                .content(StackPanel::new().keyed_children(task_rows)),
                        ),
                )),
        );

    let detail_actions = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(4.0)
        .children((
            small_segment_button(palette, "Output", 70.0),
            small_segment_button(palette, "Raw", 56.0),
            Border::new()
                .width(90.0)
                .height(28.0)
                .padding(Thickness::xy(12.0, 0.0))
                .background(palette.active)
                .corner_radius(999.0)
                .content(
                    TextBlock::new()
                        .text("● Collected")
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .foreground(palette.accent)
                        .vertical_alignment(VerticalAlignment::Center),
                ),
        ));

    let detail_card = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            Grid::new()
                .rows([GridLength::Auto, GridLength::Star(1.0)])
                .children((
                    Border::new()
                        .min_height(65.0)
                        .padding(Thickness::new(20.0, 15.0, 20.0, 13.0))
                        .border_brush(palette.border)
                        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                        .content(
                            Grid::new()
                                .columns([
                                    GridLength::Pixel(36.0),
                                    GridLength::Star(1.0),
                                    GridLength::Auto,
                                ])
                                .children((
                                    Border::new()
                                        .width(36.0)
                                        .height(36.0)
                                        .background(palette.active)
                                        .corner_radius(9.0)
                                        .content(
                                            Image::new()
                                                .source_data(EncodedImage::from_static(
                                                    desktop_icon,
                                                ))
                                                .width(16.0)
                                                .height(16.0),
                                        ),
                                    StackPanel::new()
                                        .grid_column(1)
                                        .margin(Thickness::xy(12.0, 0.0))
                                        .spacing(2.0)
                                        .children((
                                            TextBlock::new()
                                                .text("Computer System")
                                                .font_size(15.0)
                                                .font_weight(FontWeight::BOLD),
                                            TextBlock::new()
                                                .text("System · completed in 29 ms")
                                                .font_size(11.5)
                                                .foreground(palette.muted),
                                        )),
                                    Border::new()
                                        .grid_column(2)
                                        .vertical_alignment(VerticalAlignment::Center)
                                        .content(detail_actions),
                                )),
                        ),
                    ScrollViewer::new()
                        .grid_row(1)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
                        .content(
                            Border::new()
                                .padding(Thickness::new(0.0, 5.0, 0.0, 8.0))
                                .content(StackPanel::new().children((
                                    detail_kv_row(palette, 0, "0 · HypervisorPresent", "true"),
                                    detail_kv_row(
                                        palette,
                                        1,
                                        "0 · SystemSKUNumber",
                                        "Surface_Laptop_7th_Edition_2037",
                                    ),
                                    detail_kv_row(palette, 2, "0 · SystemStartupDelay", "null"),
                                    detail_kv_row(palette, 3, "0 · AdminPasswordStatus", "1"),
                                    detail_kv_row(
                                        palette,
                                        4,
                                        "0 · AutomaticResetBootOption",
                                        "true",
                                    ),
                                    detail_kv_row(
                                        palette,
                                        5,
                                        "0 · NetworkServerModeEnabled",
                                        "true",
                                    ),
                                    detail_kv_row(
                                        palette,
                                        6,
                                        "0 · AutomaticManagedPagefile",
                                        "true",
                                    ),
                                    detail_kv_row(palette, 7, "0 · DaylightInEffect", "true"),
                                    detail_kv_row(palette, 8, "0 · WakeUpType", "2"),
                                    detail_kv_row(palette, 9, "0 · CurrentTimeZone", "-240"),
                                    detail_kv_row(palette, 10, "0 · SystemType", "ARM64-based PC"),
                                    detail_kv_row(palette, 11, "0 · KeyboardPasswordStatus", "2"),
                                    detail_kv_row(
                                        palette,
                                        12,
                                        "0 · Manufacturer",
                                        "Microsoft Corporation",
                                    ),
                                    detail_kv_row(
                                        palette,
                                        13,
                                        "0 · PowerManagementSupported",
                                        "null",
                                    ),
                                    detail_kv_row(
                                        palette,
                                        14,
                                        "0 · __SUPERCLASS",
                                        "CIM_UnitaryComputerSystem",
                                    ),
                                    detail_kv_row(palette, 15, "0 · PartOfDomain", "false"),
                                ))),
                        ),
                )),
        );

    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((
            Border::new().height(320.0).content(task_card),
            Border::new().height(520.0).content(detail_card),
        ))
    } else {
        Grid::new()
            .columns([GridLength::Pixel(295.0), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((task_card, placed(detail_card, 1, 0)))
    };

    Grid::new()
        .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
        .children((
            page_header(
                palette,
                Page::Diagnostics,
                Border::new()
                    .width(120.0)
                    .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
                    .content(icon_status_pill(
                        "Scan complete",
                        ok_icon,
                        palette.ok,
                        palette.ok_bg,
                    )),
            ),
            Border::new()
                .grid_row(1)
                .margin(Thickness::new(0.0, 24.0, 0.0, 0.0))
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .children((
                            stats,
                            Button::new()
                                .grid_column(1)
                                .width(154.0)
                                .height(33.0)
                                .on_click(explain_scan)
                                .automation_name("Explain this scan with AI")
                                .content(
                                    StackPanel::new()
                                        .orientation(Orientation::Horizontal)
                                        .spacing(8.0)
                                        .children((
                                            Image::new()
                                                .source_data(EncodedImage::from_static(wand_icon))
                                                .width(17.0)
                                                .height(16.0),
                                            TextBlock::new()
                                                .text("Explain this scan")
                                                .vertical_alignment(VerticalAlignment::Center),
                                        )),
                                ),
                        )),
                ),
            Border::new()
                .grid_row(2)
                .margin(Thickness::new(0.0, 16.0, 0.0, 0.0))
                .content(body),
        ))
}

pub(crate) fn diagnostics_empty_page(
    palette: Palette,
    theme: WindowTheme,
    quick_scan: Callback<()>,
    full_scan: Callback<()>,
) -> View {
    let primary_button = Button::new()
        .height(36.0)
        .min_width(126.0)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", Color::rgb(15, 108, 189))
                .set("ButtonBackgroundPointerOver", Color::rgb(0, 120, 212))
                .set("ButtonBackgroundPressed", Color::rgb(0, 90, 158))
                .set("ButtonForeground", Color::rgb(255, 255, 255))
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::xy(18.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
        )
        .on_click(quick_scan)
        .automation_name("Quick Scan")
        .content(fa_icon_label(FaIcon::Bolt, "Quick Scan"));

    let secondary_button = Button::new()
        .height(36.0)
        .min_width(108.0)
        .resource_overrides(ResourceOverrides::new().set("ButtonPadding", Thickness::xy(18.0, 0.0)))
        .on_click(full_scan)
        .automation_name("Full Scan")
        .content(fa_icon_label(FaIcon::List, "Full Scan"));

    let hero = StackPanel::new()
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .spacing(0.0)
        .children((
            Border::new()
                .width(68.0)
                .height(68.0)
                .margin(Thickness::new(0.0, 0.0, 0.0, 20.0))
                .background(palette.active)
                .corner_radius(16.0)
                .content(
                    Image::new()
                        .source_data(EncodedImage::from_static(if theme == WindowTheme::Light {
                            STETHOSCOPE_LIGHT
                        } else {
                            STETHOSCOPE_DARK
                        }))
                        .width(30.0)
                        .height(27.0)
                        .stretch(Stretch::Fill),
                ),
            TextBlock::new()
                .text("Ready to diagnose")
                .font_size(22.0)
                .font_weight(FontWeight::BOLD)
                .horizontal_alignment(HorizontalAlignment::Center)
                .automation_heading_level(AutomationHeadingLevel::Level2),
            StackPanel::new()
                .margin(Thickness::new(0.0, 9.0, 0.0, 0.0))
                .horizontal_alignment(HorizontalAlignment::Center)
                .spacing(4.0)
                .children((
                    TextBlock::new()
                        .text("Run a Quick Scan to inventory this PC. Checks are read-only, finish")
                        .font_size(13.5)
                        .foreground(palette.muted)
                        .horizontal_alignment(HorizontalAlignment::Center),
                    TextBlock::new()
                        .text("in seconds, and never leave this machine.")
                        .font_size(13.5)
                        .foreground(palette.muted)
                        .horizontal_alignment(HorizontalAlignment::Center),
                )),
            StackPanel::new()
                .margin(Thickness::new(0.0, 24.0, 0.0, 0.0))
                .orientation(Orientation::Horizontal)
                .horizontal_alignment(HorizontalAlignment::Center)
                .spacing(10.0)
                .children((primary_button, secondary_button)),
        ));

    Grid::new()
        .rows([GridLength::Auto, GridLength::Star(1.0)])
        .children((
            page_header(palette, Page::Diagnostics, View::empty()),
            Border::new()
                .grid_row(1)
                .padding(Thickness::new(0.0, 0.0, 0.0, 18.0))
                .content(hero),
        ))
}

pub(crate) fn collected_statistic(palette: Palette) -> View {
    StackPanel::new().spacing(1.0).children((
        TextBlock::new()
            .text("COLLECTED")
            .font_size(10.5)
            .font_weight(FontWeight::SEMI_BOLD)
            .foreground(palette.muted),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(3.0)
            .children((
                TextBlock::new()
                    .text("17")
                    .font_size(21.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.accent),
                TextBlock::new()
                    .text("/ 17")
                    .font_size(12.0)
                    .foreground(palette.muted)
                    .vertical_alignment(VerticalAlignment::Bottom),
            )),
    ))
}

pub(crate) fn small_segment_button(palette: Palette, label: &'static str, width: f64) -> View {
    Button::new()
        .width(width)
        .height(28.0)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", palette.card_strong)
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::xy(12.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(5.0)),
        )
        .content(TextBlock::new().text(label).font_size(11.5))
}

pub(crate) fn detail_kv_row(
    palette: Palette,
    row: i32,
    label: &'static str,
    value: &'static str,
) -> View {
    Border::new()
        .grid_row(row)
        .height(32.0)
        .padding(Thickness::xy(20.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Pixel(200.0), GridLength::Star(1.0)])
                .column_spacing(16.0)
                .children((
                    TextBlock::new()
                        .text(label)
                        .font_size(12.0)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(value)
                        .grid_column(1)
                        .font_size(12.5)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wfdiag_native_diagnostics::DiagnosticOutput;

    #[test]
    fn diagnostic_grounding_links_allow_only_safe_http_targets() {
        for rejected in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///C:/Windows/System32/calc.exe",
            r"C:\Windows\System32\drivers\etc\hosts",
            "local:diagnostic-output",
            "custom:payload",
            "mailto:support@example.com",
            "/relative/path",
        ] {
            assert_eq!(
                safe_diagnostic_grounding_link_target(rejected),
                None,
                "{rejected}"
            );
        }
        assert_eq!(
            safe_diagnostic_grounding_link_target("http://example.com/kb"),
            Some("http://example.com/kb".to_string())
        );
        assert_eq!(
            safe_diagnostic_grounding_link_target(" HTTPS://example.com/windows?q=1#fix "),
            Some("HTTPS://example.com/windows?q=1#fix".to_string())
        );

        let projected = project_diagnostic_grounding_source(&GroundingTraceSource {
            source: "WindowsForum MCP".to_string(),
            title: "Relevant KB article".to_string(),
            url: Some("javascript:alert(1)".to_string()),
        });
        assert_eq!(projected.title, "Relevant KB article");
        assert_eq!(projected.source, "WindowsForum MCP");
        assert_eq!(projected.target, None);
    }

    fn diagnostic_task(id: &str, category: &str) -> DiagnosticTask {
        DiagnosticTask {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            category: category.to_string(),
            admin_required: false,
        }
    }

    #[test]
    fn scan_category_progress_keeps_selected_task_order_and_counts_states() {
        let catalog = vec![
            diagnostic_task("os", "System"),
            diagnostic_task("disk", "Storage"),
            diagnostic_task("bios", "System"),
        ];
        let expected = ["disk", "os", "bios"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let statuses = HashMap::from([
            ("disk".to_string(), TaskProgressStatus::Completed),
            ("os".to_string(), TaskProgressStatus::Running),
            ("bios".to_string(), TaskProgressStatus::Failed),
        ]);

        let progress = diagnostic_category_progress(&catalog, &expected, &statuses);
        assert_eq!(progress.len(), 2);
        assert_eq!(progress[0].category, "Storage");
        assert_eq!(progress[0].completed, 1);
        assert_eq!(progress[0].terminal(), 1);
        assert_eq!(progress[1].category, "System");
        assert_eq!(progress[1].total, 2);
        assert_eq!(progress[1].running, 1);
        assert_eq!(progress[1].failed, 1);
        assert_eq!(progress[1].terminal(), 1);
    }

    #[test]
    fn scan_category_progress_treats_missing_metadata_and_status_as_queued_other() {
        let expected = vec!["unknown".to_string()];
        let progress = diagnostic_category_progress(&[], &expected, &HashMap::new());

        assert_eq!(
            progress,
            vec![DiagnosticCategoryProgress {
                category: "Other".to_string(),
                total: 1,
                completed: 0,
                running: 0,
                failed: 0,
                cancelled: 0,
            }]
        );
    }

    #[test]
    fn json_output_flattens_to_key_values() {
        let rows = format_output_key_values(
            "processor",
            r#"{"Name":"Snapdragon X","NumberOfCores":12,"_DERIVATION":["A","B"]}"#,
        )
        .expect("JSON object must produce rows");
        assert!(!rows.truncated);
        assert!(
            rows.rows
                .contains(&("Name".to_string(), "Snapdragon X".to_string())),
            "rows were: {rows:?}"
        );
        assert!(
            rows.rows
                .contains(&("NumberOfCores".to_string(), "12".to_string()))
        );
    }

    #[test]
    fn json_output_with_bom_still_parses() {
        let with_bom = "\u{feff}{\"Name\":\"X\"}".to_string();
        let rows = format_output_key_values("os_info", &with_bom).expect("BOM JSON parses");
        assert_eq!(rows.rows, [("Name".to_string(), "X".to_string())]);
        assert!(!rows.truncated);
    }

    #[test]
    fn non_json_output_stays_raw() {
        assert!(format_output_key_values("os_info", "plain text output").is_none());
    }

    #[test]
    fn structured_output_caps_rows_and_bytes() {
        let output = serde_json::to_string(
            &(0..MAX_STRUCTURED_OUTPUT_ROWS + 10)
                .map(|index| format!("value-{index}"))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let rows = format_output_key_values("large_array", &output).unwrap();
        assert_eq!(rows.rows.len(), MAX_STRUCTURED_OUTPUT_ROWS);
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);

        let oversized_value = serde_json::json!({
            "blob": "x".repeat(MAX_STRUCTURED_OUTPUT_BYTES)
        })
        .to_string();
        let rows = format_output_key_values("large_value", &oversized_value).unwrap();
        assert!(rows.rows.is_empty());
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);
    }

    #[test]
    fn oversized_structured_input_uses_a_bounded_preview_without_parsing() {
        let output = format!(
            "{{\"payload\":\"{}\",\"tail\":not-valid-json}}",
            "x".repeat(MAX_STRUCTURED_OUTPUT_INPUT_BYTES)
        );
        let rows = format_output_key_values("oversized", &output).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(rows.byte_len <= MAX_STRUCTURED_OUTPUT_BYTES);
        assert!(rows.truncated);
        assert!(!rows.rows[0].1.contains("not-valid-json"));
    }

    #[test]
    fn diagnostic_raw_document_preserves_structured_output_and_metadata() {
        let mut task = diagnostic_task("os_info", "System");
        task.name = "Operating System".to_string();
        task.admin_required = true;
        let result = DiagnosticTaskResult::new(
            "session",
            "os_info",
            Arc::new(DiagnosticOutput {
                success: true,
                output: r#"{"build":26100,"secure":true}"#.to_string(),
                error: None,
                duration_ms: 42,
            }),
        );

        let value: serde_json::Value =
            serde_json::from_str(&diagnostic_raw_document(&result, Some(&task))).unwrap();
        assert_eq!(value["task_id"], "os_info");
        assert_eq!(value["name"], "Operating System");
        assert_eq!(value["category"], "System");
        assert_eq!(value["success"], true);
        assert_eq!(value["duration_ms"], 42);
        assert_eq!(value["admin_required"], true);
        assert!(value["error"].is_null());
        assert_eq!(value["output"]["build"], 26100);
        assert_eq!(value["output"]["secure"], true);
        assert!(value.get("output_truncated").is_none());
    }

    #[test]
    fn failed_raw_document_keeps_error_and_partial_plain_text_output() {
        let result = DiagnosticTaskResult::new(
            "session",
            "disk",
            Arc::new(DiagnosticOutput {
                success: false,
                output: "partial disk evidence".to_string(),
                error: Some("access denied".to_string()),
                duration_ms: 7,
            }),
        );

        let value: serde_json::Value =
            serde_json::from_str(&diagnostic_raw_document(&result, None)).unwrap();
        assert_eq!(value["name"], "disk");
        assert_eq!(value["category"], "Other");
        assert_eq!(value["admin_required"], false);
        assert_eq!(value["error"], "access denied");
        assert_eq!(value["output"], "partial disk evidence");
        assert_eq!(diagnostic_output_preview(&result), "partial disk evidence");
    }

    #[test]
    fn oversized_previews_are_bounded_without_mutating_the_shared_result() {
        let output = format!(
            "{}RAW_TAIL_SENTINEL",
            "x".repeat(MAX_DIAGNOSTIC_PREVIEW_CHARS + 100)
        );
        let result = DiagnosticTaskResult::new(
            "session",
            "large",
            Arc::new(DiagnosticOutput {
                success: true,
                output: output.clone(),
                error: None,
                duration_ms: 1,
            }),
        );

        let preview = diagnostic_output_preview(&result);
        assert!(preview.contains("Output preview truncated"));
        let value: serde_json::Value =
            serde_json::from_str(&diagnostic_raw_document(&result, None)).unwrap();
        let raw_output = value["output"].as_str().unwrap();
        assert!(value["output_truncated"].as_bool().unwrap());
        assert!(raw_output.len() < output.len());
        assert!(raw_output.contains(RAW_OUTPUT_TRUNCATION_NOTICE));
        assert!(!raw_output.contains("RAW_TAIL_SENTINEL"));
        assert_eq!(result.output, output);
    }
}
