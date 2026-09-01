//! The Issues page: detected issues, remediation, and maintenance.

#![deny(unsafe_code)]

use crate::app::consts::{
    ISSUE_INFO_DARK, ISSUE_INFO_LIGHT, ISSUE_SHIELD_DARK, ISSUE_SHIELD_LIGHT,
    ISSUE_STETHOSCOPE_DARK, ISSUE_STETHOSCOPE_LIGHT, ISSUE_USER_SHIELD_DARK,
    ISSUE_USER_SHIELD_LIGHT, ISSUE_WARN_DARK, ISSUE_WARN_LIGHT, STATUS_OK_DARK, STATUS_OK_LIGHT,
};
use crate::app::policy::action_run_status_text;
use crate::app::state::{FixPlanActionSelection, IssuePrioritizationDisplay, Page};
use crate::screens::ai::view::report_provider_attribution;
use crate::widgets::chrome::{fa_icon_label, page_header};
use crate::widgets::icons::FaIcon;
use crate::widgets::markdown_render::{MarkdownStyle, render_markdown_lite};
use crate::widgets::palette_colors::Palette;
use std::collections::HashSet;
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_issues::projection::project_issues;
use wfdiag_native_issues::{Issue, IssueSeverity, RemediationSummary, RemediationTier};
use wfdiag_native_remediation::broker::{ActionRequest, MAX_BATCH_ACTIONS};
use wfdiag_native_remediation::remediation;
use wfdiag_native_remediation::runtime::{
    ActionItemRun, ActionItemStatus, ActionRunStatus, ActionRunSummary,
};
use windows_reactor::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn issues_page(
    palette: Palette,
    theme: WindowTheme,
    issues: &[Issue],
    maintenance: &[RemediationSummary],
    fix_plan: Option<&ValidatedFixPlan>,
    fix_plan_pending: bool,
    fix_plan_error: Option<&str>,
    issue_prioritization: &IssuePrioritizationDisplay,
    active_action_run: Option<&ActionRunSummary>,
    action_run_history: &[ActionRunSummary],
    action_expanded_runs: &HashSet<String>,
    action_busy: bool,
    ai_enabled: bool,
    is_admin: bool,
    detection_pending: bool,
    detection_error: Option<&str>,
    has_committed_evidence: bool,
    projection_current: bool,
    quick_scan: Callback<()>,
    run_remediation: Callback<String>,
    ask_ai: Callback<String>,
    prioritize_issues: Callback<()>,
    cancel_issue_prioritization: Callback<()>,
    propose_fix_plan: Callback<()>,
    cancel_fix_plan: Callback<()>,
    review_fix_plan_actions: Callback<FixPlanActionSelection>,
    cancel_action_run: Callback<()>,
    set_action_run_expanded: Callback<(String, bool)>,
    restart_admin: Callback<()>,
) -> View {
    if !has_committed_evidence || issues.is_empty() {
        return issues_empty_page(
            palette,
            theme,
            maintenance,
            has_committed_evidence,
            detection_pending,
            detection_error,
            quick_scan,
            run_remediation,
            active_action_run,
            action_run_history,
            action_expanded_runs,
            cancel_action_run,
            set_action_run_expanded,
        );
    }

    let projection = project_issues(issues);
    let (admin_icon, category_icon) = if theme == WindowTheme::Light {
        (ISSUE_USER_SHIELD_LIGHT, ISSUE_STETHOSCOPE_LIGHT)
    } else {
        (ISSUE_USER_SHIELD_DARK, ISSUE_STETHOSCOPE_DARK)
    };

    let mut children = vec![
        KeyedView::new("header", page_header(palette, Page::Issues, View::empty())),
        KeyedView::new(
            "summary",
            TextBlock::new()
                .margin(Thickness::new(0.0, 7.0, 0.0, 0.0))
                .font_size(12.0)
                .foreground(palette.muted)
                .text(if projection_current {
                    projection.counts.summary_text()
                } else {
                    format!(
                        "Previous scan issue results · {} detected · {} passed · {} couldn’t verify",
                        projection.counts.detected,
                        projection.counts.passed,
                        projection.counts.unknown,
                    )
                }),
        ),
    ];

    if detection_pending {
        children.push(KeyedView::new(
            "detection-status",
            issue_detection_notice(
                palette,
                if projection_current {
                    "Rechecking issue results for the latest completed scan"
                } else {
                    "Checking the latest completed scan · showing previous scan issue results until detection finishes"
                },
                false,
            ),
        ));
    } else if let Some(error) = detection_error {
        children.push(KeyedView::new(
            "detection-status",
            issue_detection_notice(
                palette,
                &if projection_current {
                    format!("Issue refresh failed · {error} · showing the last successful results for this scan")
                } else {
                    format!("Latest issue detection failed · {error} · showing previous scan issue results")
                },
                true,
            ),
        ));
    }

    if active_action_run.is_some() || !action_run_history.is_empty() {
        children.push(KeyedView::new(
            "action-run",
            action_run_panel(
                palette,
                active_action_run,
                action_run_history,
                action_expanded_runs,
                cancel_action_run,
                set_action_run_expanded.clone(),
            ),
        ));
    }

    if !projection.detected.is_empty() {
        children.push(KeyedView::new(
            "ai-assistance",
            issue_ai_assistance(
                palette,
                issue_prioritization.busy,
                fix_plan_pending,
                ai_enabled,
                prioritize_issues,
                cancel_issue_prioritization,
                propose_fix_plan,
                cancel_fix_plan,
            ),
        ));
        if issue_prioritization.busy
            || issue_prioritization.text.is_some()
            || issue_prioritization.error.is_some()
        {
            children.push(KeyedView::new(
                "ai-prioritization",
                issue_prioritization_panel(palette, issue_prioritization),
            ));
        }
        if fix_plan_pending || fix_plan.is_some() || fix_plan_error.is_some() {
            children.push(KeyedView::new(
                "ai-fix-plan",
                fix_plan_panel(
                    palette,
                    theme,
                    issues,
                    maintenance,
                    fix_plan,
                    fix_plan_pending,
                    fix_plan_error,
                    action_busy,
                    review_fix_plan_actions,
                ),
            ));
        }
    }

    if !is_admin {
        children.push(KeyedView::new(
            "admin-notice",
            issue_card(
                palette,
                palette.active,
                palette.accent,
                admin_icon,
                category_icon,
                "Some checks need administrator access",
                "Crash dumps (BSOD), SMART & disk health, system-file (DISM) and battery checks only run when the app is elevated, so they were skipped. Restart as administrator to include them.",
                None,
                None,
                Some(("Restart as administrator", FaIcon::UserShield, move || {
                    let _ = restart_admin.call(());
                })),
                None::<(&str, bool, fn())>,
                112.0,
                198.0,
            ),
        ));
    }

    for issue in &projection.detected {
        let (tint, accent, icon_data, severity_label) =
            issue_severity_visual(palette, theme, issue.severity);
        let primary_action = issue.remediation.as_ref().map(|remediation| {
            (remediation.label.as_str(), remediation_icon(remediation), {
                let run = run_remediation.clone();
                let remediation_id = remediation.id.clone();
                move || {
                    let _ = run.call(remediation_id.clone());
                }
            })
        });
        let ask_ai_callback = {
            let ask_ai = ask_ai.clone();
            let issue_id = issue.id.clone();
            move || {
                let _ = ask_ai.call(issue_id.clone());
            }
        };
        children.push(KeyedView::new(
            format!("issue:{}", issue.id),
            issue_card(
                palette,
                tint,
                accent,
                icon_data,
                category_icon,
                &issue.title,
                &issue.description,
                (!issue.recommendation.is_empty()).then_some(issue.recommendation.as_str()),
                Some((issue.category.as_str(), severity_label)),
                primary_action,
                Some(("Ask AI", ai_enabled, ask_ai_callback)),
                153.0,
                190.0,
            ),
        ));
    }

    if projection.counts.passed > 0 {
        children.push(KeyedView::new(
            "passed",
            issue_check_group(
                palette,
                &format!("{} checks passed", projection.counts.passed),
                &projection.passed,
                true,
            ),
        ));
    }
    if projection.counts.unknown > 0 {
        children.push(KeyedView::new(
            "unknown",
            issue_check_group(
                palette,
                &format!("Couldn’t verify ({})", projection.counts.unknown),
                &projection.unknown,
                true,
            ),
        ));
    }
    children.push(KeyedView::new(
        "maintenance",
        maintenance_card(palette, maintenance, run_remediation),
    ));

    // The page content overflows the viewport (issue cards + 8 maintenance
    // rows); without the viewer the tail is clipped and unreachable.
    ScrollViewer::new()
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .content(StackPanel::new().spacing(12.0).keyed_children(children))
}

pub(crate) const fn action_item_status_label(status: ActionItemStatus) -> &'static str {
    match status {
        ActionItemStatus::Pending => "Pending",
        ActionItemStatus::Running => "Running",
        ActionItemStatus::Succeeded => "Succeeded",
        ActionItemStatus::Partial => "Partial",
        ActionItemStatus::Failed => "Failed",
        ActionItemStatus::Cancelled => "Cancelled",
        ActionItemStatus::Skipped => "Skipped",
    }
}

pub(crate) fn action_item_status_color(palette: Palette, status: ActionItemStatus) -> Color {
    match status {
        ActionItemStatus::Succeeded => palette.ok,
        ActionItemStatus::Partial | ActionItemStatus::Cancelled | ActionItemStatus::Skipped => {
            palette.warn
        }
        ActionItemStatus::Failed => palette.err,
        ActionItemStatus::Pending | ActionItemStatus::Running => palette.accent,
    }
}

pub(crate) fn remediation_step_label(status: remediation::RemediationStepStatus) -> &'static str {
    match status {
        remediation::RemediationStepStatus::Succeeded => "Succeeded",
        remediation::RemediationStepStatus::AlreadySatisfied => "Already satisfied",
        remediation::RemediationStepStatus::Failed => "Failed",
        remediation::RemediationStepStatus::Cancelled => "Cancelled",
    }
}

pub(crate) fn action_item_run_view(palette: Palette, action: &ActionItemRun) -> View {
    let mut details = Vec::new();
    if let Some(result) = action.result.as_ref() {
        details.push(KeyedView::new(
            "message",
            TextBlock::new()
                .text(result.message.clone())
                .font_size(11.5)
                .foreground(if result.success {
                    palette.text
                } else {
                    palette.muted
                })
                .is_text_selection_enabled(true)
                .text_wrapping(TextWrapping::Wrap),
        ));
        for (index, step) in result.steps.iter().enumerate() {
            let detail = step
                .detail
                .as_deref()
                .filter(|detail| !detail.trim().is_empty())
                .map(|detail| format!(" · {detail}"))
                .unwrap_or_default();
            details.push(KeyedView::new(
                format!("step:{index}"),
                TextBlock::new()
                    .text(format!(
                        "{} — {}{}",
                        step.action,
                        remediation_step_label(step.status),
                        detail
                    ))
                    .font_size(10.5)
                    .foreground(match step.status {
                        remediation::RemediationStepStatus::Succeeded
                        | remediation::RemediationStepStatus::AlreadySatisfied => palette.ok,
                        remediation::RemediationStepStatus::Failed => palette.err,
                        remediation::RemediationStepStatus::Cancelled => palette.warn,
                    })
                    .is_text_selection_enabled(true)
                    .text_wrapping(TextWrapping::Wrap),
            ));
        }
        if result.requires_restart {
            details.push(KeyedView::new(
                "restart",
                TextBlock::new()
                    .text("A Windows restart is required for this action to take effect.")
                    .font_size(10.5)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.warn)
                    .text_wrapping(TextWrapping::Wrap),
            ));
        }
    } else if let Some(error) = action.error.as_deref() {
        details.push(KeyedView::new(
            "error",
            TextBlock::new()
                .text(error.to_string())
                .font_size(11.0)
                .foreground(palette.err)
                .is_text_selection_enabled(true)
                .text_wrapping(TextWrapping::Wrap),
        ));
    } else {
        details.push(KeyedView::new(
            "pending",
            TextBlock::new()
                .text(match action.status {
                    ActionItemStatus::Running => "The vetted action is running…",
                    ActionItemStatus::Cancelled => "The action was cancelled before it started.",
                    ActionItemStatus::Skipped => "The action was skipped.",
                    _ => "Waiting for this vetted action to start…",
                })
                .font_size(10.5)
                .foreground(palette.muted),
        ));
    }

    // These per-action details are intentionally static. A status-derived
    // controlled Expander accepted pointer toggles and then snapped back on
    // the next unrelated render. The run-level Expander remains interactive;
    // once open, exact action results are always readable and stable.
    Border::new()
        .padding(Thickness::new(10.0, 8.0, 10.0, 8.0))
        .background(palette.card_strong)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(6.0)
        .content(
            StackPanel::new().spacing(7.0).children((
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .column_spacing(10.0)
                    .children((
                        TextBlock::new()
                            .text(action.label.clone())
                            .font_size(11.5)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .text_trimming(TextTrimming::CharacterEllipsis),
                        TextBlock::new()
                            .grid_column(1)
                            .text(action_item_status_label(action.status))
                            .font_size(10.5)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .foreground(action_item_status_color(palette, action.status)),
                    )),
                StackPanel::new().spacing(6.0).keyed_children(details),
            )),
        )
}

pub(crate) fn action_run_summary_view(
    palette: Palette,
    run: &ActionRunSummary,
    live: bool,
    expanded: bool,
    expanded_changed: Callback<bool>,
    cancel: Callback<()>,
) -> View {
    let can_cancel = run.status == ActionRunStatus::Running
        && run.actions.iter().any(|action| {
            action.cancellable
                && matches!(
                    action.status,
                    ActionItemStatus::Pending | ActionItemStatus::Running
                )
        });
    let cancel_action: View = if live && !run.status.terminal() {
        Button::new()
            .grid_column(1)
            .height(30.0)
            .is_enabled(can_cancel)
            .on_click(cancel)
            .automation_name("Cancel remediation run")
            .content(if run.status == ActionRunStatus::CancelRequested {
                "Stopping…"
            } else if can_cancel {
                "Cancel"
            } else {
                "Cannot cancel"
            })
    } else {
        View::empty()
    };
    let actions = run
        .actions
        .iter()
        .map(|action| {
            KeyedView::new(
                action.remediation_id.clone(),
                action_item_run_view(palette, action),
            )
        })
        .collect::<Vec<_>>();

    Expander::new()
        .is_expanded(expanded)
        .on_is_expanded_changed(expanded_changed)
        .slots([
            SlotView::new(
                ExpanderSlot::Header,
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .column_spacing(10.0)
                    .children((
                        StackPanel::new().spacing(2.0).children((
                            TextBlock::new()
                                .text(if live {
                                    "Active remediation"
                                } else {
                                    "Recent remediation"
                                })
                                .font_size(12.0)
                                .font_weight(FontWeight::SEMI_BOLD),
                            TextBlock::new()
                                .text(action_run_status_text(run))
                                .font_size(10.5)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::Wrap),
                        )),
                        cancel_action,
                    )),
            ),
            SlotView::new(
                ExpanderSlot::Content,
                StackPanel::new().spacing(6.0).keyed_children(actions),
            ),
        ])
}

pub(crate) fn action_run_panel(
    palette: Palette,
    active: Option<&ActionRunSummary>,
    history: &[ActionRunSummary],
    expanded_runs: &HashSet<String>,
    cancel: Callback<()>,
    set_expanded: Callback<(String, bool)>,
) -> View {
    let mut runs = Vec::new();
    if let Some(active) = active {
        let run_id = active.run_id.clone();
        let forward = set_expanded.clone();
        runs.push(KeyedView::new(
            format!("run:{}", active.run_id),
            action_run_summary_view(
                palette,
                active,
                true,
                expanded_runs.contains(&active.run_id),
                Callback::new(move |expanded| {
                    let _ = forward.call((run_id.clone(), expanded));
                }),
                cancel.clone(),
            ),
        ));
    }
    for run in history
        .iter()
        .filter(|run| active.is_none_or(|active| active.run_id != run.run_id))
        .take(3)
    {
        let run_id = run.run_id.clone();
        let forward = set_expanded.clone();
        runs.push(KeyedView::new(
            format!("run:{}", run.run_id),
            action_run_summary_view(
                palette,
                run,
                false,
                expanded_runs.contains(&run.run_id),
                Callback::new(move |expanded| {
                    let _ = forward.call((run_id.clone(), expanded));
                }),
                cancel.clone(),
            ),
        ));
    }

    Border::new()
        .padding(Thickness::new(14.0, 12.0, 14.0, 12.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().spacing(8.0).children((
                StackPanel::new().spacing(2.0).children((
                    TextBlock::new()
                        .text("Remediation activity")
                        .font_size(12.5)
                        .font_weight(FontWeight::BOLD),
                    TextBlock::new()
                        .text("Live status and exact per-step results from vetted catalog actions")
                        .font_size(10.5)
                        .foreground(palette.muted),
                )),
                StackPanel::new().spacing(6.0).keyed_children(runs),
            )),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn issues_empty_page(
    palette: Palette,
    theme: WindowTheme,
    maintenance: &[RemediationSummary],
    has_committed_evidence: bool,
    detection_pending: bool,
    detection_error: Option<&str>,
    quick_scan: Callback<()>,
    run_remediation: Callback<String>,
    active_action_run: Option<&ActionRunSummary>,
    action_run_history: &[ActionRunSummary],
    action_expanded_runs: &HashSet<String>,
    cancel_action_run: Callback<()>,
    set_action_run_expanded: Callback<(String, bool)>,
) -> View {
    let (title, description) = if detection_pending {
        (
            "Checking the latest scan…",
            "Native issue detection is preparing the completed diagnostic evidence.".to_string(),
        )
    } else if let Some(error) = detection_error {
        ("Issue detection unavailable", error.to_string())
    } else if has_committed_evidence {
        (
            "No issue results available",
            "The completed scan is retained. Press Ctrl+R to try native issue detection again."
                .to_string(),
        )
    } else {
        (
            "No scan data yet",
            "Run a Quick Scan and any detected problems will appear here with recommended next steps."
                .to_string(),
        )
    };
    let quick_scan_action: View = if has_committed_evidence {
        View::empty()
    } else {
        Button::new()
            .margin(Thickness::new(0.0, 22.0, 0.0, 0.0))
            .height(34.0)
            .resource_overrides(issue_primary_button_resources())
            .on_click(quick_scan)
            .content(fa_icon_label(FaIcon::Bolt, "Quick Scan"))
    };

    let hero = Border::new().height(359.0).content(
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(0.0)
            .children((
                Border::new()
                    .width(56.0)
                    .height(56.0)
                    .margin(Thickness::new(0.0, 0.0, 0.0, 18.0))
                    .background(palette.active)
                    .corner_radius(12.0)
                    .content(
                        Image::new()
                            .source_data(EncodedImage::from_static(
                                if theme == WindowTheme::Light {
                                    ISSUE_SHIELD_LIGHT
                                } else {
                                    ISSUE_SHIELD_DARK
                                },
                            ))
                            .width(27.0)
                            .height(27.0),
                    ),
                TextBlock::new()
                    .text(title)
                    .font_size(22.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .margin(Thickness::new(0.0, 10.0, 0.0, 0.0))
                    .text(description)
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(590.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                quick_scan_action,
            )),
    );

    let action_panel: View = if active_action_run.is_some() || !action_run_history.is_empty() {
        Border::new()
            .margin(Thickness::new(0.0, 0.0, 0.0, 12.0))
            .content(action_run_panel(
                palette,
                active_action_run,
                action_run_history,
                action_expanded_runs,
                cancel_action_run,
                set_action_run_expanded,
            ))
    } else {
        View::empty()
    };

    // A clean scan still shows all eight always-available maintenance rows.
    // The hero plus that catalog is taller than the normal workspace, so the
    // empty branch needs the same reachable overflow behavior as the detected
    // issue branch above. Without this viewer only the first few rows are
    // realized and neither keyboard nor UI Automation can reach the rest.
    ScrollViewer::new()
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .content(StackPanel::new().spacing(0.0).children((
            page_header(palette, Page::Issues, View::empty()),
            hero,
            action_panel,
            maintenance_card(palette, maintenance, run_remediation),
        )))
}

pub(crate) const fn issue_ai_action_enabled(ai_enabled: bool, competing_action_busy: bool) -> bool {
    ai_enabled && !competing_action_busy
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn issue_ai_assistance(
    palette: Palette,
    prioritization_busy: bool,
    fix_plan_busy: bool,
    ai_enabled: bool,
    prioritize: Callback<()>,
    cancel_prioritization: Callback<()>,
    propose_fix_plan: Callback<()>,
    cancel_fix_plan: Callback<()>,
) -> View {
    let prioritization_action: View = if prioritization_busy {
        Button::new()
            .height(32.0)
            .on_click(cancel_prioritization)
            .automation_name("Cancel issue prioritization")
            .content(fa_icon_label(FaIcon::Xmark, "Cancel prioritization"))
    } else {
        Button::new()
            .height(32.0)
            .is_enabled(issue_ai_action_enabled(ai_enabled, fix_plan_busy))
            .on_click(prioritize)
            .resource_overrides(
                ResourceOverrides::new()
                    .set("ButtonBackground", Color::transparent())
                    .set("ButtonBackgroundPointerOver", palette.active)
                    .set("ButtonBackgroundPressed", palette.active)
                    .set("ButtonBackgroundDisabled", Color::transparent())
                    .set("ButtonForeground", palette.text)
                    .set("ButtonForegroundDisabled", palette.muted)
                    .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                    .set("ButtonPadding", Thickness::xy(10.0, 0.0)),
            )
            .content(fa_icon_label(FaIcon::RankingStar, "Prioritize"))
    };
    let plan_action: View = if fix_plan_busy {
        Button::new()
            .height(32.0)
            .on_click(cancel_fix_plan)
            .automation_name("Cancel fix plan")
            .content(fa_icon_label(FaIcon::Xmark, "Cancel"))
    } else {
        Button::new()
            .height(32.0)
            .is_enabled(issue_ai_action_enabled(ai_enabled, prioritization_busy))
            .on_click(propose_fix_plan)
            .resource_overrides(
                ResourceOverrides::new()
                    .set("ButtonBackground", Color::transparent())
                    .set("ButtonBackgroundPointerOver", palette.active)
                    .set("ButtonBackgroundPressed", palette.active)
                    .set("ButtonBackgroundDisabled", Color::transparent())
                    .set("ButtonForeground", palette.text)
                    .set("ButtonForegroundDisabled", palette.muted)
                    .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                    .set("ButtonPadding", Thickness::xy(10.0, 0.0)),
            )
            .content(fa_icon_label(FaIcon::ListCheck, "Propose fix plan"))
    };
    Border::new()
        .height(61.0)
        .padding(Thickness::xy(18.0, 0.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
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
                        .text("AI Assistance")
                        .grid_column(1)
                        .font_size(12.0)
                        .font_weight(FontWeight::BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                    StackPanel::new()
                        .grid_column(2)
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .vertical_alignment(VerticalAlignment::Center)
                        .children((prioritization_action, plan_action)),
                )),
        )
}

pub(crate) fn issue_prioritization_panel(
    palette: Palette,
    prioritization: &IssuePrioritizationDisplay,
) -> View {
    let body: View = if prioritization.busy && prioritization.text.is_none() {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(10.0)
            .children((
                ProgressRing::new().width(20.0).height(20.0).is_active(true),
                TextBlock::new()
                    .text("Prioritizing issues…")
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .vertical_alignment(VerticalAlignment::Center),
            ))
    } else if let Some(error) = prioritization.error.as_deref() {
        TextBlock::new()
            .text(format!("AI triage failed · {error}"))
            .font_size(12.5)
            .foreground(palette.err)
            .text_wrapping(TextWrapping::Wrap)
            .into()
    } else if let Some(text) = prioritization.text.as_deref() {
        render_markdown_lite(
            text,
            MarkdownStyle::with_palette(palette.text, palette.card_strong, palette.border),
        )
    } else {
        View::empty()
    };
    let attribution =
        prioritization
            .provider_use
            .as_ref()
            .map_or_else(String::new, |provider_use| {
                let mut text = format!("Provider: {}", provider_use.provider_id);
                if prioritization.cached {
                    text.push_str(" · cached");
                }
                text
            });
    let attribution: View = if attribution.is_empty() {
        View::empty()
    } else {
        TextBlock::new()
            .text(attribution)
            .font_size(10.5)
            .foreground(palette.muted)
            .into()
    };

    Border::new()
        .padding(Thickness::uniform(16.0))
        .background(palette.dim)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().spacing(8.0).children((body, attribution)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fix_plan_panel(
    palette: Palette,
    theme: WindowTheme,
    issues: &[Issue],
    maintenance: &[RemediationSummary],
    plan: Option<&ValidatedFixPlan>,
    pending: bool,
    error: Option<&str>,
    action_busy: bool,
    review_actions: Callback<FixPlanActionSelection>,
) -> View {
    if pending {
        return Border::new()
            .padding(Thickness::uniform(18.0))
            .background(palette.card)
            .border_brush(palette.border)
            .border_thickness(1.0)
            .corner_radius(9.0)
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(10.0)
                    .children((
                        ProgressRing::new().width(20.0).height(20.0).is_active(true),
                        TextBlock::new()
                            .text("Generating and validating an ordered fix plan…")
                            .font_size(12.5)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
            );
    }

    if let Some(error) = error {
        return issue_detection_notice(palette, error, true);
    }

    let Some(plan) = plan else {
        return View::empty();
    };
    let find_remediation = |remediation_id: &str| {
        issues
            .iter()
            .filter_map(|issue| issue.remediation.as_ref())
            .chain(maintenance.iter())
            .find(|remediation| remediation.id == remediation_id)
    };

    let mut batch_seen = HashSet::new();
    let batch_actions = plan
        .entries
        .iter()
        .filter(|entry| {
            issues
                .iter()
                .any(|issue| issue.detected && issue.id == entry.issue_id)
        })
        .filter_map(|entry| {
            let remediation = find_remediation(&entry.remediation_id)?;
            (remediation.batch_eligible && batch_seen.insert(remediation.id.clone())).then(|| {
                ActionRequest {
                    remediation_id: remediation.id.clone(),
                    issue_id: Some(entry.issue_id.clone()),
                }
            })
        })
        .take(MAX_BATCH_ACTIONS)
        .collect::<Vec<_>>();
    let batch_button: View = if batch_actions.len() > 1 {
        let review = review_actions.clone();
        let selection = FixPlanActionSelection {
            actions: batch_actions.clone(),
            expected_scan_fingerprint: plan.scan_fingerprint.clone(),
            expected_catalog_fingerprint: plan.catalog_fingerprint.clone(),
        };
        Button::new()
            .height(34.0)
            .is_enabled(!action_busy)
            .automation_name(format!(
                "Review {} low-impact fix-plan actions together",
                batch_actions.len()
            ))
            .on_click(move || {
                let _ = review.call(selection.clone());
            })
            .resource_overrides(issue_primary_button_resources())
            .content(fa_icon_label(
                FaIcon::ListCheck,
                format!("Review {} low-impact actions together", batch_actions.len()),
            ))
    } else {
        View::empty()
    };

    let mut rows = Vec::new();
    for entry in &plan.entries {
        let Some(issue) = issues
            .iter()
            .find(|issue| issue.detected && issue.id == entry.issue_id)
        else {
            continue;
        };
        let Some(remediation) = find_remediation(&entry.remediation_id) else {
            continue;
        };
        let (tint, accent, icon_data, _) = issue_severity_visual(palette, theme, issue.severity);
        let tier = match entry.tier {
            RemediationTier::OpenTool => "Open tool",
            RemediationTier::AutoSafe => "Auto-safe",
            RemediationTier::Repair => "Repair",
        };
        let review = review_actions.clone();
        let selection = FixPlanActionSelection {
            actions: vec![ActionRequest {
                remediation_id: remediation.id.clone(),
                issue_id: Some(entry.issue_id.clone()),
            }],
            expected_scan_fingerprint: plan.scan_fingerprint.clone(),
            expected_catalog_fingerprint: plan.catalog_fingerprint.clone(),
        };
        let remediation_label = remediation.label.clone();
        let automation_name = format!("Review fix-plan action {remediation_label}");
        rows.push(KeyedView::new(
            format!("{}:{}", entry.issue_id, entry.remediation_id),
            Border::new()
                .min_height(98.0)
                .padding(Thickness::uniform(14.0))
                .background(palette.card)
                .border_brush(palette.border)
                .border_thickness(1.0)
                .corner_radius(8.0)
                .content(
                    Grid::new()
                        .columns([
                            GridLength::Pixel(36.0),
                            GridLength::Star(1.0),
                            GridLength::Auto,
                        ])
                        .column_spacing(12.0)
                        .children((
                            Border::new()
                                .width(36.0)
                                .height(36.0)
                                .background(tint)
                                .corner_radius(9.0)
                                .content(
                                    Image::new()
                                        .source_data(EncodedImage::from_static(icon_data))
                                        .width(17.0)
                                        .height(17.0),
                                ),
                            StackPanel::new().grid_column(1).spacing(5.0).children((
                                TextBlock::new()
                                    .text(issue.title.clone())
                                    .font_weight(FontWeight::BOLD),
                                render_markdown_lite(
                                    &entry.rationale,
                                    MarkdownStyle::with_palette(
                                        palette.text,
                                        palette.card_strong,
                                        palette.border,
                                    ),
                                ),
                                TextBlock::new()
                                    .text(tier)
                                    .font_size(10.5)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .foreground(accent),
                            )),
                            Button::new()
                                .grid_column(2)
                                .height(32.0)
                                .is_enabled(!action_busy)
                                .automation_name(automation_name)
                                .on_click(move || {
                                    let _ = review.call(selection.clone());
                                })
                                .resource_overrides(issue_primary_button_resources())
                                .content(fa_icon_label(
                                    remediation_icon(remediation),
                                    &remediation_label,
                                )),
                        )),
                ),
        ));
    }

    let entries: View = if rows.is_empty() {
        render_markdown_lite(
            if plan.entries.is_empty() {
                &plan.notes
            } else {
                "This plan no longer matches the current detected issues."
            },
            MarkdownStyle::with_palette(palette.text, palette.card_strong, palette.border),
        )
    } else {
        StackPanel::new().spacing(8.0).keyed_children(rows)
    };
    let notes: View = if !plan.notes.trim().is_empty() && !plan.entries.is_empty() {
        render_markdown_lite(
            &plan.notes,
            MarkdownStyle::with_palette(palette.text, palette.card_strong, palette.border),
        )
    } else {
        View::empty()
    };
    let provider = report_provider_attribution(
        Some(plan.provider_use.provider_id.as_str()),
        Some(&plan.provider_use),
    );

    Border::new()
        .padding(Thickness::uniform(16.0))
        .background(palette.dim)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().spacing(9.0).children((
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        TextBlock::new()
                            .text("Vetted fix plan")
                            .font_size(14.0)
                            .font_weight(FontWeight::BOLD),
                        TextBlock::new()
                            .grid_column(1)
                            .text(provider)
                            .font_size(10.5)
                            .foreground(palette.muted)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
                batch_button,
                entries,
                notes,
            )),
        )
}

pub(crate) fn issue_detection_notice(palette: Palette, text: &str, is_error: bool) -> View {
    Border::new()
        .min_height(40.0)
        .padding(Thickness::xy(14.0, 8.0))
        .background(if is_error {
            palette.err_bg
        } else {
            palette.active
        })
        .border_brush(if is_error {
            palette.err
        } else {
            palette.accent
        })
        .border_thickness(Thickness::new(3.0, 0.0, 0.0, 0.0))
        .corner_radius(8.0)
        .content(
            TextBlock::new()
                .text(text)
                .font_size(11.5)
                .foreground(if is_error { palette.err } else { palette.muted })
                .text_wrapping(TextWrapping::Wrap),
        )
}

pub(crate) fn maintenance_card(
    palette: Palette,
    maintenance: &[RemediationSummary],
    run_remediation: Callback<String>,
) -> View {
    let rows = maintenance
        .iter()
        .map(|remediation| {
            let run = run_remediation.clone();
            let remediation_id = remediation.id.clone();
            KeyedView::new(
                remediation.id.clone(),
                maintenance_row(palette, remediation, move || {
                    let _ = run.call(remediation_id.clone());
                }),
            )
        })
        .collect::<Vec<_>>();

    Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            StackPanel::new().children((
                Border::new()
                    .height(46.0)
                    .padding(Thickness::xy(18.0, 0.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Pixel(3.0), GridLength::Star(1.0)])
                            .column_spacing(11.0)
                            .children((
                                Border::new()
                                    .width(3.0)
                                    .height(18.0)
                                    .background(palette.accent)
                                    .corner_radius(999.0)
                                    .vertical_alignment(VerticalAlignment::Center),
                                TextBlock::new()
                                    .text("Maintenance")
                                    .grid_column(1)
                                    .font_size(13.0)
                                    .font_weight(FontWeight::BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                    ),
                StackPanel::new().keyed_children(rows),
            )),
        )
}

pub(crate) fn maintenance_row(
    palette: Palette,
    remediation: &RemediationSummary,
    run: impl Fn() + 'static,
) -> View {
    let mut title = remediation.label.clone();
    if remediation.tier == RemediationTier::Repair {
        title.push_str(" repair");
    }
    if remediation.admin_required {
        title.push_str(" admin");
    }
    let run_automation = format!("Run {}", remediation.label);

    Border::new()
        .min_height(54.0)
        .padding(Thickness::xy(14.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .children((
                    StackPanel::new()
                        .vertical_alignment(VerticalAlignment::Center)
                        .spacing(2.0)
                        .children((
                            TextBlock::new()
                                .text(title)
                                .font_size(12.0)
                                .font_weight(FontWeight::SEMI_BOLD),
                            TextBlock::new()
                                .text(&remediation.description)
                                .font_size(10.5)
                                .foreground(palette.muted)
                                .text_trimming(TextTrimming::CharacterEllipsis),
                        )),
                    Button::new()
                        .grid_column(1)
                        .width(58.0)
                        .height(31.0)
                        .on_click(run)
                        .automation_name(run_automation)
                        .resource_overrides(
                            ResourceOverrides::new().set("ButtonForegroundDisabled", palette.text),
                        )
                        .vertical_alignment(VerticalAlignment::Center)
                        .content("Run"),
                )),
        )
}

pub(crate) fn issue_primary_button_resources() -> ResourceOverrides {
    ResourceOverrides::new()
        .set("ButtonBackground", Color::rgb(15, 108, 189))
        .set("ButtonBackgroundPointerOver", Color::rgb(0, 120, 212))
        .set("ButtonBackgroundPressed", Color::rgb(0, 90, 158))
        .set("ButtonBackgroundDisabled", Color::rgb(15, 108, 189))
        .set("ButtonForeground", Color::rgb(255, 255, 255))
        .set("ButtonForegroundDisabled", Color::rgb(255, 255, 255))
        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
        .set("ButtonPadding", Thickness::xy(15.0, 0.0))
        .set("ControlCornerRadius", CornerRadius::uniform(7.0))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn issue_card(
    palette: Palette,
    tint: Color,
    accent: Color,
    icon_data: &'static [u8],
    category_icon_data: &'static [u8],
    title: &str,
    description: &str,
    recommendation: Option<&str>,
    chips: Option<(&str, &str)>,
    primary_action: Option<(&str, FaIcon, impl Fn() + 'static)>,
    secondary_action: Option<(&str, bool, impl Fn() + 'static)>,
    min_height: f64,
    action_width: f64,
) -> View {
    let recommendation: View = if let Some(text) = recommendation {
        Border::new()
            .background(palette.card)
            .border_brush(accent)
            .border_thickness(Thickness::new(3.0, 0.0, 0.0, 0.0))
            .padding(Thickness::new(10.0, 7.0, 10.0, 7.0))
            .content(
                TextBlock::new()
                    .text(format!("Recommended: {text}"))
                    .font_size(12.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .text_wrapping(TextWrapping::Wrap),
            )
    } else {
        View::empty()
    };

    let chips: View = if let Some((category, severity)) = chips {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(7.0)
            .children((
                issue_chip(
                    palette,
                    category,
                    palette.muted,
                    palette.card,
                    Some(category_icon_data),
                ),
                issue_chip(palette, severity, accent, tint, None),
            ))
    } else {
        View::empty()
    };

    let secondary_action: View = if let Some((label, enabled, on_click)) = secondary_action {
        Button::new()
            .width(action_width)
            .style(ButtonStyle::Subtle)
            .is_enabled(enabled)
            .on_click(on_click)
            .resource_overrides(
                ResourceOverrides::new().set("ButtonForegroundDisabled", palette.muted),
            )
            .content(fa_icon_label(FaIcon::CommentDots, label))
    } else {
        View::empty()
    };
    let primary_action: View = if let Some((label, icon, on_click)) = primary_action {
        let automation = format!("Run {label}");
        Button::new()
            .width(action_width)
            .height(32.0)
            .on_click(on_click)
            .automation_name(automation)
            .resource_overrides(issue_primary_button_resources())
            .content(fa_icon_label(icon, label))
    } else {
        View::empty()
    };

    Border::new()
        .min_height(min_height)
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .padding(Thickness::new(18.0, 16.0, 18.0, 16.0))
        .content(
            Grid::new()
                .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
                .children((
                    Border::new()
                        .width(36.0)
                        .height(36.0)
                        .background(tint)
                        .corner_radius(9.0)
                        .vertical_alignment(VerticalAlignment::Top)
                        .content(
                            Image::new()
                                .source_data(EncodedImage::from_static(icon_data))
                                .width(17.0)
                                .height(17.0),
                        ),
                    StackPanel::new()
                        .grid_column(1)
                        .margin(Thickness::xy(15.0, 0.0))
                        .spacing(7.0)
                        .children((
                            TextBlock::new().text(title).font_weight(FontWeight::BOLD),
                            TextBlock::new()
                                .text(description)
                                .text_wrapping(TextWrapping::Wrap)
                                .font_size(12.5)
                                .foreground(palette.muted)
                                .max_width(560.0)
                                .horizontal_alignment(HorizontalAlignment::Left),
                            recommendation,
                            chips,
                        )),
                    StackPanel::new()
                        .grid_column(2)
                        .spacing(6.0)
                        .children((primary_action, secondary_action)),
                )),
        )
}

pub(crate) fn issue_chip(
    palette: Palette,
    label: &str,
    foreground: Color,
    background: Color,
    icon_data: Option<&'static [u8]>,
) -> View {
    let content: View = if let Some(icon_data) = icon_data {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(5.0)
            .vertical_alignment(VerticalAlignment::Center)
            .children((
                Image::new()
                    .source_data(EncodedImage::from_static(icon_data))
                    .width(10.0)
                    .height(10.0),
                TextBlock::new()
                    .text(label)
                    .font_size(9.5)
                    .font_weight(FontWeight::BOLD)
                    .foreground(foreground),
            ))
    } else {
        TextBlock::new()
            .text(label)
            .font_size(9.5)
            .font_weight(FontWeight::BOLD)
            .foreground(foreground)
            .into()
    };

    Border::new()
        .height(22.0)
        .padding(Thickness::xy(9.0, 0.0))
        .background(background)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(999.0)
        .content(content)
}

pub(crate) fn issue_check_group(
    palette: Palette,
    label: &str,
    issues: &[&Issue],
    passed: bool,
) -> View {
    let rows = issues
        .iter()
        .map(|issue| {
            let indicator: View = if passed {
                TextBlock::new()
                    .text("✓")
                    .font_size(11.0)
                    .font_weight(FontWeight::BOLD)
                    .foreground(palette.ok)
                    .into()
            } else {
                TextBlock::new()
                    .text("—")
                    .font_size(12.0)
                    .font_weight(FontWeight::BOLD)
                    .foreground(palette.muted)
                    .into()
            };
            KeyedView::new(
                issue.id.clone(),
                Border::new()
                    .min_height(34.0)
                    .padding(Thickness::new(2.0, 5.0, 2.0, 5.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([
                                GridLength::Pixel(16.0),
                                GridLength::Auto,
                                GridLength::Star(1.0),
                            ])
                            .column_spacing(8.0)
                            .children((
                                indicator,
                                TextBlock::new()
                                    .grid_column(1)
                                    .text(issue.title.clone())
                                    .font_size(12.0)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .vertical_alignment(VerticalAlignment::Top),
                                TextBlock::new()
                                    .grid_column(2)
                                    .text(issue.description.clone())
                                    .font_size(12.0)
                                    .foreground(palette.muted)
                                    .text_wrapping(TextWrapping::Wrap),
                            )),
                    ),
            )
        })
        .collect::<Vec<_>>();

    Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(8.0)
        .content(
            Expander::new().is_expanded(false).slots([
                SlotView::new(
                    ExpanderSlot::Header,
                    TextBlock::new()
                        .text(label)
                        .font_size(12.0)
                        .font_weight(FontWeight::BOLD),
                ),
                SlotView::new(
                    ExpanderSlot::Content,
                    Border::new()
                        .padding(Thickness::new(14.0, 0.0, 14.0, 10.0))
                        .content(StackPanel::new().keyed_children(rows)),
                ),
            ]),
        )
}

pub(crate) fn issue_severity_visual(
    palette: Palette,
    theme: WindowTheme,
    severity: IssueSeverity,
) -> (Color, Color, &'static [u8], &'static str) {
    let (info_icon, warning_icon, ok_icon) = if theme == WindowTheme::Light {
        (ISSUE_INFO_LIGHT, ISSUE_WARN_LIGHT, STATUS_OK_LIGHT)
    } else {
        (ISSUE_INFO_DARK, ISSUE_WARN_DARK, STATUS_OK_DARK)
    };
    match severity {
        IssueSeverity::Critical => (palette.err_bg, palette.err, warning_icon, "CRITICAL"),
        IssueSeverity::Warning => (palette.warn_bg, palette.warn, warning_icon, "WARNING"),
        IssueSeverity::Info => (palette.active, palette.accent, info_icon, "INFO"),
        IssueSeverity::Ok => (palette.ok_bg, palette.ok, ok_icon, "OK"),
    }
}

pub(crate) fn remediation_icon(remediation: &RemediationSummary) -> FaIcon {
    if remediation.id.contains("temp") || remediation.id.contains("recycle") {
        return FaIcon::Broom;
    }
    match remediation.tier {
        RemediationTier::OpenTool => FaIcon::ArrowUpRightFromSquare,
        RemediationTier::AutoSafe | RemediationTier::Repair => FaIcon::WandMagicSparkles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_ai_controls_follow_the_committed_enablement_and_mutual_busy_gate() {
        assert!(issue_ai_action_enabled(true, false));
        assert!(!issue_ai_action_enabled(false, false));
        assert!(!issue_ai_action_enabled(true, true));
        assert!(!issue_ai_action_enabled(false, true));
    }
}
