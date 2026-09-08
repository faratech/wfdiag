//! The AI page: assistant workspace and scan report.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{BOT_AVATAR, ISSUE_WARN_DARK};
use crate::app::message::Message;
use crate::app::policy::{
    OnboardingAction, OnboardingCandidate, SignInBanner, SignInBannerAction,
    ai_runtime_pill_compact, ai_workspace_height, onboarding_candidates, provider_display_name,
    shell_uses_short_layout, sign_in_banner,
};
use crate::app::screen::ShellEnv;
use crate::app::state::{
    AiMode, AiPreparationUi, ChatDisplayMessage, ChatDisplayRole, CloudFallbackConsent,
    FullScanConsent, Page,
};
use crate::dialogs::settings::msg::SettingsMsg;
use crate::fixtures::visual::VisualState;
use crate::screens::ai::state::{AiMsg, AiScreen};
use crate::widgets::chrome::{fa_icon_label, page_header, placed};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::markdown_render::{MarkdownStyle, render_markdown_lite};
use crate::widgets::palette_colors::Palette;
use wfdiag_native_ai_chat::{
    ChatToolActivity, ChatToolActivityState, ChatToolHistory, ProviderUse,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderStatus, parse_provider_preference, route_provider,
};
use windows_reactor::*;

pub(crate) fn ai_provider_pill_content(
    preferred_provider: &str,
    deterministic_visual: bool,
    ai_enabled: bool,
    provider_status: Option<&AIProviderStatus>,
    provider_loading: bool,
    error: Option<&str>,
) -> (String, String, Option<String>, bool, bool) {
    if deterministic_visual {
        return (
            "Phi Silica".to_string(),
            "·  On device".to_string(),
            None,
            true,
            false,
        );
    }
    if !ai_enabled {
        return (
            "AI disabled".to_string(),
            "·  Settings".to_string(),
            None,
            false,
            false,
        );
    }
    if provider_loading && provider_status.is_none() {
        return (
            "Checking AI provider".to_string(),
            "·  Please wait".to_string(),
            None,
            false,
            false,
        );
    }
    if error.is_some() {
        return (
            "AI unavailable".to_string(),
            "·  Check Settings".to_string(),
            None,
            false,
            false,
        );
    }
    // The same pure decision the engine takes (`route_provider`): an explicit
    // preference is honoured only while that provider is available and never
    // falls back; Auto walks the local-first chain. An explicit but
    // unavailable provider therefore reads "No provider · Not connected",
    // exactly as the Tauri shell's AIContext mirror does.
    let preferred = parse_provider_preference(preferred_provider);
    let active = provider_status.map_or(AIProvider::None, |status| {
        route_provider(preferred, status.availability())
    });
    let (provider, execution, cloud) = match active {
        AIProvider::PhiSilica => ("Phi Silica", "·  On device", false),
        AIProvider::FoundryLocal => ("Foundry Local", "·  Local server", false),
        AIProvider::Ollama => ("Ollama", "·  Local server", false),
        AIProvider::CustomOpenAI => ("Custom endpoint", "·  API cloud", true),
        AIProvider::CodexCli => ("ChatGPT via Codex", "·  Subscription cloud", true),
        AIProvider::ClaudeCode => ("Claude Code", "·  Subscription cloud", true),
        AIProvider::OpenAI => ("OpenAI", "·  API cloud", true),
        AIProvider::Anthropic => ("Anthropic Claude", "·  API cloud", true),
        AIProvider::Gemini => ("Google Gemini", "·  API cloud", true),
        AIProvider::DeepSeek => ("DeepSeek", "·  API cloud", true),
        AIProvider::None => ("No provider", "·  Not connected", false),
    };
    let model = provider_status.and_then(|status| {
        status
            .providers
            .iter()
            .find(|p| p.id == active)
            .and_then(|p| p.model.clone())
    });
    (
        provider.to_string(),
        execution.to_string(),
        model,
        active != AIProvider::None,
        cloud,
    )
}

impl AiScreen {
    /// Paint the page from the screen's own state plus the chrome's env.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn view(&self, env: &ShellEnv<'_>, vc: &mut ViewContext<WfdiagShell>) -> View {
        ai_page(
            &env.settings.preferred_ai_provider,
            env.palette,
            env.narrow,
            ai_runtime_pill_compact(env.window_size.width),
            env.window_size.height,
            env.visual_state,
            env.deterministic_visual,
            env.settings.ai_enabled,
            self.mode,
            &self.chat_input,
            &self.composer_reference,
            self.answer.as_deref(),
            &self.messages,
            self.full_scan_consent.as_ref(),
            self.cloud_fallback_consent.as_ref(),
            self.provider_status.as_ref(),
            self.status_loading,
            self.status_error.as_deref(),
            AiPreparationUi {
                intent: self.pending_intent.as_ref(),
                error: self.preparation_error.as_deref(),
                scan_busy: env.scan.busy,
                scan_cancelling: env.scan.cancelling,
                completed: env.scan.completed,
                total: env.scan.total,
                current_task: env.scan.current_task,
            },
            vc.message(Message::Ai(AiMsg::SetMode(AiMode::Assistant))),
            vc.message(Message::Ai(AiMsg::SetMode(AiMode::ScanReport))),
            vc.callback(|value| Message::Ai(AiMsg::ChatInputChanged(value))),
            vc.callback(|value| Message::Ai(AiMsg::UsePrompt(value))),
            vc.message(Message::Ai(AiMsg::SendChat)),
            vc.message(Message::Ai(AiMsg::NewConversation)),
            vc.message(Message::Ai(AiMsg::AllowCloudFallback)),
            vc.message(Message::Ai(AiMsg::NeverCloudFallback)),
            vc.message(Message::Ai(AiMsg::ApproveFullScan)),
            vc.message(Message::Ai(AiMsg::DismissFullScan)),
            vc.message(Message::Settings(SettingsMsg::Open)),
            vc.message(Message::Ai(AiMsg::CancelPendingIntent)),
            vc.message(Message::Ai(AiMsg::RetryPendingIntent)),
            self.report_text.as_deref(),
            self.report_provider.as_deref(),
            self.report_provider_use.as_ref(),
            self.report_generating,
            self.report_error.as_deref(),
            env.scan.has_results,
            vc.message(Message::Ai(AiMsg::GenerateReport)),
            vc.message(Message::Ai(AiMsg::RegenerateReport)),
            vc.message(Message::Ai(AiMsg::CancelReport)),
            vc.message(Message::Ai(AiMsg::CopyReport)),
            self.streaming,
            vc.message(Message::Ai(AiMsg::CancelChat)),
            env.settings.ai_onboarding_seen,
            vc.callback(|action| Message::Ai(AiMsg::OnboardingAction(action))),
            vc.message(Message::Ai(AiMsg::DismissOnboarding)),
            sign_in_banner(
                self.sign_in_required.as_ref(),
                self.sign_in_banner_dismissed.as_ref(),
            ),
            vc.callback(|action| Message::Ai(AiMsg::SignInBannerAction(action))),
            vc.message(Message::Ai(AiMsg::DismissSignInBanner)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_page(
    preferred_provider: &str,
    palette: Palette,
    narrow: bool,
    pill_compact: bool,
    window_height: f64,
    visual_state: VisualState,
    deterministic_visual: bool,
    ai_enabled: bool,
    mode: AiMode,
    input: &str,
    composer_reference: &ElementRef<TextBox>,
    answer: Option<&str>,
    chat_messages: &[ChatDisplayMessage],
    full_scan_consent: Option<&FullScanConsent>,
    cloud_fallback_consent: Option<&CloudFallbackConsent>,
    provider_status: Option<&AIProviderStatus>,
    provider_loading: bool,
    provider_error: Option<&str>,
    preparation: AiPreparationUi<'_>,
    assistant_mode: Callback<()>,
    report_mode: Callback<()>,
    input_changed: Callback<String>,
    use_prompt: Callback<String>,
    send: Callback<()>,
    new_conversation: Callback<()>,
    allow_cloud_fallback: Callback<()>,
    never_cloud_fallback: Callback<()>,
    approve_full_scan: Callback<()>,
    dismiss_full_scan: Callback<()>,
    open_settings: Callback<()>,
    cancel_preparation: Callback<()>,
    retry_preparation: Callback<()>,
    report_text: Option<&str>,
    report_provider: Option<&str>,
    report_provider_use: Option<&ProviderUse>,
    report_generating: bool,
    report_error: Option<&str>,
    report_has_scan: bool,
    generate_report: Callback<()>,
    regenerate_report: Callback<()>,
    cancel_report: Callback<()>,
    copy_report: Callback<()>,
    chat_pending: bool,
    cancel_chat: Callback<()>,
    ai_onboarding_seen: bool,
    onboarding_action: Callback<OnboardingAction>,
    dismiss_onboarding: Callback<()>,
    sign_in_banner: Option<SignInBanner>,
    sign_in_banner_action: Callback<SignInBannerAction>,
    dismiss_sign_in_banner: Callback<()>,
) -> View {
    // #32: the one-time connect offer, derived from the probed rows. It only
    // applies to an enabled profile that has not dismissed it before.
    let onboarding: Vec<OnboardingCandidate> = if ai_enabled && !ai_onboarding_seen {
        onboarding_candidates(provider_status)
    } else {
        Vec::new()
    };
    let chat_interaction_blocked = preparation.intent.is_some()
        || full_scan_consent.is_some()
        || cloud_fallback_consent.is_some();
    let prompts = [
        "Summarize my latest scan",
        "What failed and why?",
        "Any security concerns?",
        "How do I free up disk space?",
    ];
    // Suggestion chips are optional chrome: the Store shell drops them under
    // 650 px of height so the composer keeps its room.
    let prompt_buttons = if shell_uses_short_layout(window_height) {
        Vec::new()
    } else {
        prompts
            .into_iter()
            .enumerate()
            .map(|(index, prompt)| {
                let callback = use_prompt.clone();
                KeyedView::new(
                    index.to_string(),
                    Border::new()
                        .height(27.0)
                        .background(palette.card)
                        .border_brush(palette.border)
                        .border_thickness(1.0)
                        .corner_radius(999.0)
                        .content(
                            Button::new()
                                .height(27.0)
                                .is_enabled(
                                    !provider_loading && !chat_pending && !chat_interaction_blocked,
                                )
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", Color::transparent())
                                        .set("ButtonBackgroundPressed", Color::transparent())
                                        .set("ButtonForeground", palette.text)
                                        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                        .set("ButtonPadding", Thickness::xy(10.0, 0.0)),
                                )
                                .on_click(move || {
                                    let _ = callback.call(prompt.to_string());
                                })
                                .content(TextBlock::new().text(prompt).font_size(12.0)),
                        ),
                )
            })
            .collect::<Vec<_>>()
    };

    let mode_switch = Border::new()
        .width(217.0)
        .height(38.0)
        .horizontal_alignment(HorizontalAlignment::Left)
        .padding(Thickness::uniform(3.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(8.0)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(3.0)
                .children((
                    ai_mode_button(
                        palette,
                        FaIcon::CommentDots,
                        "Assistant",
                        mode == AiMode::Assistant,
                        assistant_mode,
                    ),
                    ai_mode_button(
                        palette,
                        FaIcon::FileExport,
                        "Scan Report",
                        mode == AiMode::ScanReport,
                        report_mode,
                    ),
                )),
        );

    let (provider_label, execution_label, model_label, provider_ready, cloud_execution) =
        ai_provider_pill_content(
            preferred_provider,
            deterministic_visual,
            ai_enabled,
            provider_status,
            provider_loading,
            provider_error,
        );
    let model_view: View = model_label.map_or_else(View::empty, |model| {
        TextBlock::new()
            .text(format!("·  {model}"))
            .max_width(190.0)
            .font_size(12.0)
            .foreground(palette.muted)
            .text_trimming(TextTrimming::CharacterEllipsis)
            .into()
    });
    let configure_ai = open_settings.clone();
    // Compact: only the status dot, the provider name and the gear remain,
    // as the Store shell hides `.ai-runtime-class` / `.ai-runtime-model`
    // below 680 px of content width.
    let execution_view: View = if pill_compact {
        View::empty()
    } else {
        TextBlock::new()
            .text(execution_label)
            .font_size(12.0)
            .foreground(palette.muted)
            .into()
    };
    let model_view: View = if pill_compact {
        View::empty()
    } else {
        model_view
    };
    let provider_view = TextBlock::new()
        .text(provider_label)
        .font_size(12.0)
        .font_weight(FontWeight::SEMI_BOLD)
        .text_trimming(TextTrimming::CharacterEllipsis);
    let provider_view = if pill_compact {
        provider_view.max_width(150.0)
    } else {
        provider_view
    };
    let runtime_pill = Border::new()
        .min_width(if pill_compact { 0.0 } else { 179.0 })
        .height(38.0)
        .padding(Thickness::new(10.0, 0.0, 5.0, 0.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(999.0)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    TextBlock::new()
                        .text("●")
                        .font_size(13.0)
                        .foreground(if cloud_execution {
                            palette.warn
                        } else if provider_ready {
                            palette.ok
                        } else {
                            palette.muted
                        }),
                    provider_view,
                    execution_view,
                    model_view,
                    Button::new()
                        .width(27.0)
                        .height(27.0)
                        .resource_overrides(
                            ResourceOverrides::new()
                                .set("ButtonBackground", Color::transparent())
                                .set("ButtonBackgroundPointerOver", palette.active)
                                .set("ButtonBackgroundPressed", palette.active)
                                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                .set("ButtonPadding", Thickness::uniform(6.0))
                                .set("ControlCornerRadius", CornerRadius::uniform(5.0)),
                        )
                        .automation_name("Open AI settings")
                        .on_click(open_settings.clone())
                        .content(icons::path(FaIcon::Settings)),
                )),
        );

    // The Store UI keeps the mode tabs and provider status on one row even
    // in the 900 px compact state. Stacking them steals 46 px from the chat
    // surface and is the source of the compact composer clipping.
    let mode_bar = Border::new()
        .margin(Thickness::new(0.0, 6.0, 0.0, -6.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .children((mode_switch, placed(runtime_pill, 1, 0))),
        );

    // The composer is part of the workspace's fixed bottom row. Matching the
    // workspace to the actual client height keeps it pinned in view instead
    // of forcing users to scroll past an artificial 550/650 px minimum.
    let workspace_height = ai_workspace_height(window_height);

    let workspace = if mode == AiMode::Assistant {
        ai_assistant_workspace(
            palette,
            narrow,
            workspace_height,
            visual_state,
            input,
            composer_reference,
            answer,
            chat_messages,
            full_scan_consent,
            cloud_fallback_consent,
            prompt_buttons,
            input_changed,
            deterministic_visual,
            ai_enabled,
            provider_loading,
            provider_ready || deterministic_visual,
            configure_ai,
            send,
            new_conversation,
            allow_cloud_fallback,
            never_cloud_fallback,
            approve_full_scan,
            dismiss_full_scan,
            preparation,
            cancel_preparation.clone(),
            retry_preparation.clone(),
            chat_pending,
            cancel_chat,
            &onboarding,
            onboarding_action.clone(),
            dismiss_onboarding.clone(),
        )
    } else {
        ai_scan_report_workspace(
            palette,
            workspace_height,
            report_text,
            report_provider,
            report_provider_use,
            report_generating,
            report_error,
            deterministic_visual,
            ai_enabled,
            provider_loading,
            provider_ready || deterministic_visual,
            open_settings,
            report_has_scan,
            generate_report,
            regenerate_report,
            cancel_report,
            copy_report,
            preparation,
            cancel_preparation,
            retry_preparation,
        )
    };

    // The sign-in banner: the one thing between the user and a working
    // assistant, with its one action, above the mode bar in both modes.
    let banner: View = match sign_in_banner {
        Some(banner) if ai_enabled && !deterministic_visual => sign_in_banner_card(
            palette,
            &banner,
            sign_in_banner_action,
            dismiss_sign_in_banner,
        ),
        _ => View::empty(),
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Ai, View::empty()),
        banner,
        mode_bar,
        workspace,
    ))
}

fn sign_in_banner_card(
    palette: Palette,
    banner: &SignInBanner,
    action: Callback<SignInBannerAction>,
    dismiss: Callback<()>,
) -> View {
    let act = {
        let action = action.clone();
        let banner_action = banner.action;
        move || {
            let _ = action.call(banner_action);
        }
    };
    Border::new()
        .background(palette.card_strong)
        .border_brush(palette.accent)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .padding(Thickness::new(16.0, 12.0, 16.0, 12.0))
        .automation_name(banner.title.clone())
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .column_spacing(12.0)
                .children((
                    StackPanel::new().spacing(3.0).children((
                        TextBlock::new()
                            .text(banner.title.clone())
                            .font_size(13.5)
                            .font_weight(FontWeight::SEMI_BOLD),
                        TextBlock::new()
                            .text(banner.body.clone())
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::Wrap),
                    )),
                    StackPanel::new()
                        .grid_column(1)
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .vertical_alignment(VerticalAlignment::Center)
                        .children((
                            Button::new()
                                .height(30.0)
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", palette.accent)
                                        .set("ButtonBackgroundPointerOver", palette.accent)
                                        .set("ButtonBackgroundPressed", palette.accent)
                                        .set("ButtonForeground", Color::rgb(255, 255, 255))
                                        .set(
                                            "ButtonForegroundPointerOver",
                                            Color::rgb(255, 255, 255),
                                        )
                                        .set("ButtonForegroundPressed", Color::rgb(255, 255, 255))
                                        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0)),
                                )
                                .automation_name(format!(
                                    "{} · {}",
                                    banner.action_label, banner.title
                                ))
                                .on_click(act)
                                .content(banner.action_label),
                            Button::new()
                                .height(30.0)
                                .automation_name("Not now")
                                .on_click(dismiss)
                                .content("Not now"),
                        )),
                )),
        )
}

pub(crate) fn ai_mode_button(
    palette: Palette,
    icon: FaIcon,
    label: &'static str,
    selected: bool,
    action: Callback<()>,
) -> View {
    Button::new()
        .width(if label == "Assistant" { 96.0 } else { 112.0 })
        .height(30.0)
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
                .set(
                    "ButtonForeground",
                    if selected {
                        palette.text
                    } else {
                        palette.muted
                    },
                )
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::xy(4.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
        )
        .on_click(action)
        .automation_name(label)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    icons::path(icon).width(13.0).height(13.0),
                    TextBlock::new()
                        .text(label)
                        .font_size(12.0)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

pub(crate) fn chat_tool_activity_view(palette: Palette, activity: &ChatToolActivity) -> View {
    let state_color = match activity.state {
        ChatToolActivityState::Done => palette.ok,
        ChatToolActivityState::Failed | ChatToolActivityState::TimedOut => palette.err,
        ChatToolActivityState::Cancelled | ChatToolActivityState::CancelRequested => palette.warn,
        ChatToolActivityState::Queued | ChatToolActivityState::Running => palette.accent,
    };
    let duration = activity
        .duration_ms
        .map(|duration| format!(" · {duration} ms"))
        .unwrap_or_default();
    let details = activity
        .model_error
        .as_deref()
        .or(activity.model_output.as_deref())
        .or(activity.result_preview.as_deref())
        .unwrap_or("No tool output is available yet.");
    Expander::new().is_expanded(false).slots([
        SlotView::new(
            ExpanderSlot::Header,
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .column_spacing(10.0)
                .children((
                    StackPanel::new().spacing(2.0).children((
                        TextBlock::new()
                            .text(activity.tool.clone())
                            .font_size(11.5)
                            .font_weight(FontWeight::SEMI_BOLD),
                        TextBlock::new()
                            .text(activity.args_summary.clone())
                            .font_size(10.0)
                            .foreground(palette.muted)
                            .text_trimming(TextTrimming::CharacterEllipsis),
                    )),
                    TextBlock::new()
                        .grid_column(1)
                        .text(format!("{}{}", activity.state.as_str(), duration))
                        .font_size(10.0)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .foreground(state_color)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        ),
        SlotView::new(
            ExpanderSlot::Content,
            TextBlock::new()
                .text(details)
                .font_size(10.5)
                .foreground(if activity.model_error.is_some() {
                    palette.err
                } else {
                    palette.muted
                })
                .is_text_selection_enabled(true)
                .text_wrapping(TextWrapping::Wrap),
        ),
    ])
}

pub(crate) fn chat_tool_history_view(palette: Palette, history: &ChatToolHistory) -> View {
    if history.activities().is_empty() {
        return View::empty();
    }
    let rows = history
        .activities()
        .iter()
        .map(|activity| {
            KeyedView::new(
                activity.call_id.clone(),
                chat_tool_activity_view(palette, activity),
            )
        })
        .collect::<Vec<_>>();
    StackPanel::new()
        .max_width(760.0)
        .spacing(4.0)
        .keyed_children(rows)
}

pub(crate) fn ai_preparation_panel(
    palette: Palette,
    title: &'static str,
    description: &'static str,
    preparation: AiPreparationUi<'_>,
    cancel: Callback<()>,
    retry: Callback<()>,
) -> View {
    let progress = if preparation.total == 0 {
        0.0
    } else {
        (preparation.completed as f64 / preparation.total as f64 * 100.0).clamp(0.0, 100.0)
    };
    let activity: View = if let Some(error) = preparation.error {
        StackPanel::new().spacing(10.0).children((
            TextBlock::new()
                .text(error.to_string())
                .font_size(12.0)
                .foreground(palette.err)
                .text_wrapping(TextWrapping::Wrap)
                .max_width(560.0)
                .horizontal_alignment(HorizontalAlignment::Center),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .horizontal_alignment(HorizontalAlignment::Center)
                .children((
                    Button::new()
                        .height(32.0)
                        .resource_overrides(primary_button_resources())
                        .on_click(retry)
                        .automation_name("Retry pending AI request")
                        .content("Retry"),
                    Button::new()
                        .height(32.0)
                        .on_click(cancel)
                        .automation_name("Cancel pending AI request")
                        .content("Cancel AI request"),
                )),
        ))
    } else {
        let activity_text = if preparation.scan_cancelling {
            "Stopping the prerequisite scan…".to_string()
        } else if preparation.scan_busy {
            preparation.current_task.map_or_else(
                || "Starting the prerequisite scan…".to_string(),
                |task| format!("Scanning · {task}"),
            )
        } else {
            "Waiting to start the prerequisite scan…".to_string()
        };
        let retry_action: View = if preparation.scan_busy {
            View::empty()
        } else {
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(retry)
                .automation_name("Start prerequisite scan")
                .content("Start scan")
        };
        StackPanel::new().spacing(10.0).children((
            TextBlock::new()
                .text(activity_text)
                .font_size(12.0)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center),
            ProgressBar::new()
                .width(310.0)
                .height(4.0)
                .minimum(0.0)
                .maximum(100.0)
                .value(progress)
                .is_indeterminate(preparation.scan_busy && preparation.total == 0),
            TextBlock::new()
                .text(if preparation.total == 0 {
                    "Preparing scan plan".to_string()
                } else {
                    format!(
                        "{} of {} diagnostics collected",
                        preparation.completed, preparation.total
                    )
                })
                .font_size(10.5)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .horizontal_alignment(HorizontalAlignment::Center)
                .children((
                    retry_action,
                    Button::new()
                        .height(32.0)
                        .on_click(cancel)
                        .automation_name("Cancel pending AI request")
                        .content("Cancel AI request"),
                )),
        ))
    };

    StackPanel::new()
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .spacing(10.0)
        .children((
            Border::new()
                .width(48.0)
                .height(48.0)
                .background(palette.active)
                .corner_radius(10.0)
                .content(
                    icons::path(FaIcon::MagnifyingGlass)
                        .width(22.0)
                        .height(22.0),
                ),
            TextBlock::new()
                .text(title)
                .font_size(18.0)
                .font_weight(FontWeight::BOLD),
            TextBlock::new()
                .text(description)
                .font_size(12.0)
                .foreground(palette.muted)
                .text_wrapping(TextWrapping::Wrap)
                .max_width(540.0)
                .horizontal_alignment(HorizontalAlignment::Center),
            activity,
        ))
}

/// The one-time connect offer's rows: one proposal per detected provider,
/// then "Not now" (dismiss) and the plain Settings route. With no
/// proposals the layout is exactly what shipped before #32.
#[allow(clippy::too_many_lines)]
fn onboarding_rows(
    onboarding: &[OnboardingCandidate],
    onboarding_action: Callback<OnboardingAction>,
    dismiss_onboarding: Callback<()>,
    open_settings: Callback<()>,
    configure_label: &'static str,
    palette: Palette,
) -> Vec<View> {
    if onboarding.is_empty() {
        return vec![
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(open_settings)
                .content(configure_label),
        ];
    }
    let mut rows = Vec::new();
    for candidate in onboarding {
        let button = match candidate.action {
            OnboardingAction::OpenSettings => Button::new()
                .height(30.0)
                .on_click(open_settings.clone())
                .content(candidate.label),
            _ => {
                let callback = onboarding_action.clone();
                let action = candidate.action;
                Button::new()
                    .height(30.0)
                    .resource_overrides(primary_button_resources())
                    .on_click(move || {
                        let _ = callback.call(action);
                    })
                    .content(candidate.label)
            }
        };
        rows.push(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(10.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    TextBlock::new()
                        .text(candidate.detail)
                        .font_size(12.0)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    button,
                )),
        );
    }
    rows.push(
        Button::new()
            .height(26.0)
            .on_click(dismiss_onboarding)
            .content("Not now"),
    );
    rows
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_assistant_workspace(
    palette: Palette,
    narrow: bool,
    workspace_height: f64,
    visual_state: VisualState,
    input: &str,
    composer_reference: &ElementRef<TextBox>,
    answer: Option<&str>,
    chat_messages: &[ChatDisplayMessage],
    full_scan_consent: Option<&FullScanConsent>,
    cloud_fallback_consent: Option<&CloudFallbackConsent>,
    prompt_buttons: Vec<KeyedView>,
    input_changed: Callback<String>,
    deterministic_visual: bool,
    ai_enabled: bool,
    provider_loading: bool,
    provider_ready: bool,
    open_settings: Callback<()>,
    send: Callback<()>,
    new_conversation: Callback<()>,
    allow_cloud_fallback: Callback<()>,
    never_cloud_fallback: Callback<()>,
    approve_full_scan: Callback<()>,
    dismiss_full_scan: Callback<()>,
    preparation: AiPreparationUi<'_>,
    cancel_preparation: Callback<()>,
    retry_preparation: Callback<()>,
    chat_pending: bool,
    cancel_chat: Callback<()>,
    onboarding: &[OnboardingCandidate],
    onboarding_action: Callback<OnboardingAction>,
    dismiss_onboarding: Callback<()>,
) -> View {
    let conversation_active = visual_state.is_conversation() || !chat_messages.is_empty();
    let strip_open_settings = open_settings.clone();
    let interaction_blocked = preparation.intent.is_some()
        || full_scan_consent.is_some()
        || cloud_fallback_consent.is_some();
    let body: View = if let Some(consent) = cloud_fallback_consent {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(11.0)
            .children((
                icons::path(FaIcon::Globe).width(30.0).height(30.0),
                TextBlock::new()
                    .text(format!(
                        "Continue with {}?",
                        provider_display_name(consent.candidate)
                    ))
                    .font_size(17.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .text(format!(
                        "{} could not answer before producing any output. {}",
                        provider_display_name(consent.local_provider),
                        consent.reason
                    ))
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(540.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                TextBlock::new()
                    .text("Continuing may send this question and its bounded diagnostic evidence to the configured cloud provider. Your choice is saved in Settings.")
                    .font_size(11.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(540.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .children((
                        Button::new()
                            .on_click(allow_cloud_fallback)
                            .automation_name("Allow cloud fallback")
                            .content("Allow cloud fallback"),
                        Button::new()
                            .on_click(never_cloud_fallback)
                            .automation_name("Never use cloud fallback")
                            .content("Never use cloud"),
                    )),
            ))
    } else if let Some(consent) = full_scan_consent {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(11.0)
            .children((
                icons::path(FaIcon::MagnifyingGlass).width(30.0).height(30.0),
                TextBlock::new()
                    .text("Run a Full Scan for more evidence?")
                    .font_size(17.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .text(consent.reason.clone())
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(520.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                TextBlock::new()
                    .text("The scan will start only after you approve. Your original question will be asked again with the completed Full Scan evidence.")
                    .font_size(11.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(520.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .children((
                        Button::new()
                            .is_enabled(!chat_pending && !preparation.scan_busy)
                            .on_click(approve_full_scan)
                            .automation_name("Approve Full Scan")
                            .content("Run Full Scan"),
                        Button::new()
                            .on_click(dismiss_full_scan)
                            .automation_name("Dismiss Full Scan request")
                            .content("Not now"),
                    )),
            ))
    } else if preparation.is_chat() {
        ai_preparation_panel(
            palette,
            "Preparing diagnostic context",
            "Your question will be sent automatically after the prerequisite scan completes.",
            preparation,
            cancel_preparation,
            retry_preparation,
        )
    } else if visual_state == VisualState::IssueToChat {
        ai_issue_to_chat_body(palette)
    } else if matches!(
        visual_state,
        VisualState::AiConversationDesktop
            | VisualState::AiConversationTopCompact
            | VisualState::AiConversationBottomCompact
    ) {
        ai_conversation_body(palette, visual_state)
    } else if !chat_messages.is_empty() {
        let rows = chat_messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let is_user = message.role == ChatDisplayRole::User;
                let speaker = if is_user {
                    "You".to_string()
                } else {
                    message.provider_use.as_ref().map_or_else(
                        || "WindowsForum Assistant".to_string(),
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
                            format!("{}{}{}", provider.provider_id, model, fallback)
                        },
                    )
                };
                // The last assistant turn streams while a request is pending:
                // a spinner before the first token, then a caret after the
                // text, as the Store shell's blinking caret does.
                let awaiting_first_token =
                    !is_user && message.text.is_empty() && message.finish_reason.is_none();
                let streaming = !is_user
                    && chat_pending
                    && message.finish_reason.is_none()
                    && index + 1 == chat_messages.len();
                let text = if streaming && !message.text.is_empty() {
                    format!("{}▍", message.text)
                } else {
                    message.text.clone()
                };
                let message_content: View = if is_user {
                    TextBlock::new()
                        .text(text)
                        .font_size(12.5)
                        .is_text_selection_enabled(true)
                        .text_wrapping(TextWrapping::Wrap)
                        .into()
                } else if awaiting_first_token {
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((
                            ProgressRing::new().is_active(true).width(14.0).height(14.0),
                            TextBlock::new()
                                .text("Thinking…")
                                .font_size(12.5)
                                .foreground(palette.muted)
                                .vertical_alignment(VerticalAlignment::Center),
                        ))
                } else {
                    render_markdown_lite(
                        &text,
                        MarkdownStyle::with_palette(
                            palette.text,
                            palette.card_strong,
                            palette.border,
                        ),
                    )
                };
                let tools = chat_tool_history_view(palette, &message.tools);
                let proposals: View = if message.proposals.is_empty() {
                    View::empty()
                } else {
                    TextBlock::new()
                        .text(format!(
                            "Review requested: {}",
                            message.proposals.join(", ")
                        ))
                        .font_size(10.5)
                        .foreground(palette.warn)
                        .text_wrapping(TextWrapping::Wrap)
                        .into()
                };
                let terminal: View =
                    message
                        .terminal_message
                        .as_ref()
                        .map_or_else(View::empty, |terminal| {
                            TextBlock::new()
                                .text(terminal.clone())
                                .font_size(10.5)
                                .foreground(palette.muted)
                                .into()
                        });
                KeyedView::new(
                    format!("{}-{:?}", message.turn, message.role),
                    StackPanel::new()
                        .max_width(if narrow { 620.0 } else { 760.0 })
                        .horizontal_alignment(if is_user {
                            HorizontalAlignment::Right
                        } else {
                            HorizontalAlignment::Left
                        })
                        .spacing(4.0)
                        .children((
                            TextBlock::new()
                                .text(speaker)
                                .font_size(10.5)
                                .font_weight(FontWeight::SEMI_BOLD)
                                .foreground(palette.muted),
                            Border::new()
                                .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                                .background(if is_user {
                                    palette.card_strong
                                } else {
                                    palette.active
                                })
                                .corner_radius(10.0)
                                .content(message_content),
                            tools,
                            proposals,
                            terminal,
                        )),
                )
            })
            .collect::<Vec<_>>();
        ScrollViewer::new()
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .content(
                Border::new()
                    .padding(Thickness::new(24.0, 18.0, 24.0, 18.0))
                    .content(StackPanel::new().spacing(13.0).keyed_children(rows)),
            )
    } else if let Some(answer) = answer {
        ScrollViewer::new()
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .content(
                Border::new()
                    .padding(Thickness::new(24.0, 22.0, 24.0, 22.0))
                    .content(
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(11.0)
                            .children((
                                bot_avatar(30.0),
                                StackPanel::new().spacing(4.0).children((
                                    TextBlock::new()
                                        .text("WindowsForum Assistant")
                                        .font_size(11.0)
                                        .font_weight(FontWeight::SEMI_BOLD)
                                        .foreground(palette.muted),
                                    Border::new()
                                        .max_width(if narrow { 620.0 } else { 760.0 })
                                        .padding(Thickness::new(14.0, 11.0, 14.0, 11.0))
                                        .background(palette.active)
                                        .corner_radius(10.0)
                                        .horizontal_alignment(HorizontalAlignment::Left)
                                        .content(render_markdown_lite(
                                            answer,
                                            MarkdownStyle::with_palette(
                                                palette.text,
                                                palette.card_strong,
                                                palette.border,
                                            ),
                                        )),
                                )),
                            )),
                    ),
            )
    } else if !deterministic_visual && (provider_loading && !provider_ready) {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(10.0)
            .children((
                icons::path(FaIcon::Refresh).width(28.0).height(28.0),
                TextBlock::new()
                    .text("Checking AI availability…")
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD),
            ))
    } else if !deterministic_visual && (!ai_enabled || !provider_ready) {
        let configure_label = if ai_enabled {
            "Configure AI"
        } else {
            "Open Settings"
        };
        let mut rows: Vec<View> = vec![
            icons::path(if ai_enabled {
                FaIcon::CircleInfo
            } else {
                FaIcon::Gear
            })
            .width(30.0)
            .height(30.0)
            .into(),
            TextBlock::new()
                .text(if ai_enabled {
                    "Connect an AI provider"
                } else {
                    "AI insights are turned off"
                })
                .font_size(18.0)
                .font_weight(FontWeight::BOLD)
                .into(),
            TextBlock::new()
                .text(if ai_enabled {
                    "Choose a local, subscription, or API provider in Settings. Diagnostics remain on this PC until a cloud provider is used."
                } else {
                    "Enable them in Settings to use the assistant or create scan reports."
                })
                .font_size(12.5)
                .foreground(palette.muted)
                .text_wrapping(TextWrapping::Wrap)
                .horizontal_alignment(HorizontalAlignment::Center)
                .max_width(520.0)
                .into(),
        ];
        rows.extend(onboarding_rows(
            onboarding,
            onboarding_action,
            dismiss_onboarding,
            open_settings,
            configure_label,
            palette,
        ));
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(9.0)
            .keyed_children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, view)| KeyedView::new(index, view)),
            )
    } else {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(0.0)
            .children((
                Border::new()
                    .margin(Thickness::new(0.0, 0.0, 0.0, 10.0))
                    .content(bot_avatar(46.0)),
                TextBlock::new()
                    .text("What would you like to understand?")
                    .font_size(16.0)
                    .font_weight(FontWeight::BOLD)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .automation_heading_level(AutomationHeadingLevel::Level2),
                TextBlock::new()
                    .text("Ask about the latest diagnostics, failures, risks, or next steps.")
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .margin(Thickness::new(0.0, 5.0, 0.0, 0.0))
                    .horizontal_alignment(HorizontalAlignment::Center),
            ))
    };

    let header_trailing: View = if conversation_active {
        Button::new()
            .grid_column(1)
            .is_enabled(!chat_pending && !interaction_blocked)
            .on_click(new_conversation)
            .automation_name("New conversation")
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(7.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .children((
                        SymbolIcon::new()
                            .symbol(Symbol::Add)
                            .width(11.0)
                            .height(11.0),
                        TextBlock::new()
                            .text("New conversation")
                            .font_size(11.5)
                            .foreground(palette.muted),
                    )),
            )
    } else {
        View::empty()
    };
    let prompts: View = if conversation_active
        || (!deterministic_visual && (provider_loading || !ai_enabled || !provider_ready))
    {
        View::empty()
    } else {
        VariableSizedWrapGrid::new()
            .grid_row(2)
            .margin(Thickness::new(13.0, 0.0, 13.0, 6.0))
            .orientation(Orientation::Horizontal)
            .item_height(27.0)
            .keyed_children(prompt_buttons)
    };

    // A conversation whose provider has since dropped keeps its transcript
    // but says why the composer is disabled, as the Store shell's
    // `.chat-unavailable` strip does.
    let unavailable_strip: View = if conversation_active
        && !deterministic_visual
        && !provider_loading
        && (!ai_enabled || !provider_ready)
    {
        Border::new()
            .grid_row(2)
            .padding(Thickness::new(14.0, 7.0, 14.0, 7.0))
            .border_brush(palette.border)
            .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .column_spacing(12.0)
                    .children((
                        TextBlock::new()
                            .text(if ai_enabled {
                                "The AI provider is unavailable · connect a provider to continue this conversation"
                            } else {
                                "AI insights are turned off · enable them to continue this conversation"
                            })
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::Wrap)
                            .vertical_alignment(VerticalAlignment::Center),
                        Button::new()
                            .grid_column(1)
                            .height(27.0)
                            .on_click(strip_open_settings)
                            .automation_name("Open Settings")
                            .content("Open Settings"),
                    )),
            )
    } else {
        View::empty()
    };

    let composer_placeholder = if preparation.intent.is_some() {
        "Preparing scan evidence…"
    } else if !deterministic_visual && (provider_loading && !provider_ready) {
        "Checking AI provider…"
    } else if !deterministic_visual && (!ai_enabled || !provider_ready) {
        "Configure an AI provider to start…"
    } else {
        "Ask about a diagnostic, error, or trend…"
    };

    Border::new()
        .height(workspace_height)
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(
            Grid::new()
                .rows([
                    GridLength::Pixel(62.0),
                    GridLength::Star(1.0),
                    GridLength::Auto,
                    GridLength::Auto,
                ])
                .children((
                    Border::new()
                        .padding(Thickness::xy(18.0, 0.0))
                        .border_brush(palette.border)
                        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                        .content(
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .children((
                                    StackPanel::new()
                                        .orientation(Orientation::Horizontal)
                                        .spacing(10.0)
                                        .vertical_alignment(VerticalAlignment::Center)
                                        .children((
                                            bot_avatar(25.0),
                                            StackPanel::new().spacing(2.0).children((
                                                TextBlock::new()
                                                    .text("WindowsForum Assistant")
                                                    .font_size(13.0)
                                                    .font_weight(FontWeight::SEMI_BOLD),
                                                TextBlock::new()
                                                    .text("Explains the current diagnostic results")
                                                    .font_size(10.5)
                                                    .foreground(palette.muted),
                                            )),
                                        )),
                                    header_trailing,
                                )),
                        ),
                    Border::new().grid_row(1).content(body),
                    prompts,
                    unavailable_strip,
                    Border::new()
                        .grid_row(3)
                        .padding(Thickness::new(13.0, 10.0, 13.0, 10.0))
                        .border_brush(palette.border)
                        .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                        .content(
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .column_spacing(8.0)
                                .children((
                                    TextBox::new()
                                        // Grows with the draft like the Store
                                        // shell's textarea (36 → 92 px); Enter
                                        // sends, Shift+Enter adds a line.
                                        .min_height(36.0)
                                        .max_height(92.0)
                                        .accepts_return(true)
                                        .text_wrapping(TextWrapping::Wrap)
                                        .text(input)
                                        .placeholder_text(composer_placeholder)
                                        .is_enabled(
                                            deterministic_visual
                                                || (ai_enabled
                                                    && provider_ready
                                                    && !provider_loading
                                                    && !interaction_blocked),
                                        )
                                        .on_text_changed(input_changed)
                                        .element_ref(composer_reference)
                                        .automation_name("Chat message"),
                                    if chat_pending {
                                        Button::new()
                                            .grid_column(1)
                                            .width(83.0)
                                            .height(32.0)
                                            .resource_overrides(primary_button_resources())
                                            .on_click(cancel_chat)
                                            .automation_name("Stop generating")
                                            .content(fa_icon_label(FaIcon::Xmark, "Stop"))
                                    } else {
                                        Button::new()
                                            .grid_column(1)
                                            .width(83.0)
                                            .height(32.0)
                                            .resource_overrides(primary_button_resources())
                                            .is_enabled(
                                                provider_ready
                                                    && !interaction_blocked
                                                    && !input.trim().is_empty(),
                                            )
                                            .on_click(send)
                                            .automation_name("Send chat message")
                                            .content(fa_icon_label(FaIcon::PaperPlane, "Send"))
                                    },
                                )),
                        ),
                )),
        )
}

pub(crate) fn ai_user_message(_palette: Palette, text: &'static str) -> View {
    Grid::new()
        .columns([
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Pixel(42.0),
        ])
        .column_spacing(10.0)
        .children((
            StackPanel::new()
                .grid_column(1)
                .spacing(4.0)
                .horizontal_alignment(HorizontalAlignment::Right)
                .children((
                    TextBlock::new()
                        .text("You")
                        .font_size(10.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .horizontal_alignment(HorizontalAlignment::Left),
                    Border::new()
                        .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                        .background(Color::rgb(77, 166, 229))
                        .corner_radius(10.0)
                        .content(
                            TextBlock::new()
                                .text(text)
                                .font_size(12.5)
                                .foreground(Color::rgb(255, 255, 255)),
                        ),
                )),
            Border::new()
                .grid_column(2)
                .width(29.0)
                .height(29.0)
                .background(Color::rgb(77, 166, 229))
                .corner_radius(999.0)
                .vertical_alignment(VerticalAlignment::Top)
                .content(
                    TextBlock::new()
                        .text("ME")
                        .font_size(10.0)
                        .font_weight(FontWeight::BOLD)
                        .foreground(Color::rgb(255, 255, 255))
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Center),
                ),
        ))
}

pub(crate) fn ai_assistant_message(palette: Palette, content: impl Into<View>) -> View {
    Grid::new()
        .columns([GridLength::Pixel(38.0), GridLength::Star(1.0)])
        .column_spacing(2.0)
        .children((
            bot_avatar(29.0),
            StackPanel::new().grid_column(1).spacing(5.0).children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(7.0)
                    .children((
                        TextBlock::new()
                            .text("WF Assistant")
                            .font_size(10.5)
                            .font_weight(FontWeight::SEMI_BOLD),
                        TextBlock::new()
                            .text("·  Phi Silica · On Device")
                            .font_size(10.5)
                            .foreground(palette.muted),
                    )),
                content.into(),
            )),
        ))
}

pub(crate) fn ai_error_message(palette: Palette) -> View {
    ai_assistant_message(
        palette,
        Border::new()
            .padding(Thickness::new(12.0, 9.0, 12.0, 9.0))
            .background(palette.active)
            .corner_radius(9.0)
            .horizontal_alignment(HorizontalAlignment::Left)
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(7.0)
                    .children((
                        icons::path(FaIcon::TriangleExclamation)
                            .width(12.0)
                            .height(12.0),
                        TextBlock::new()
                            .text("The local provider failed, and cloud fallback was declined.")
                            .font_size(12.5)
                            .foreground(palette.err),
                    )),
            ),
    )
}

pub(crate) fn ai_response_message(palette: Palette) -> View {
    let response_line = |text: &'static str| {
        TextBlock::new()
            .text(text)
            .font_size(14.0)
            .vertical_alignment(VerticalAlignment::Top)
    };

    ai_assistant_message(
        palette,
        Border::new()
            .width(522.0)
            .height(209.0)
            .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
            .padding(Thickness::new(13.0, 27.0, 13.0, 14.0))
            .background(Color::argb(16, 255, 255, 255))
            .corner_radius(9.0)
            .horizontal_alignment(HorizontalAlignment::Left)
            .content(
                StackPanel::new().spacing(6.0).children((
                    response_line(
                        "CPU usage refers to the amount of processing power being used by the",
                    ),
                    response_line(
                        "computer's processor at any given time. It's a measure of how much work",
                    ),
                    response_line(
                        "the CPU is doing, which can be influenced by the number of tasks it's",
                    ),
                    response_line(
                        "handling, the type of tasks, and the efficiency of the system's software and",
                    ),
                    response_line("hardware."),
                    response_line("Would you like me to run the Full Scan?")
                        .margin(Thickness::new(0.0, 16.0, 0.0, 0.0)),
                )),
            ),
    )
}

pub(crate) fn ai_conversation_body(palette: Palette, state: VisualState) -> View {
    let content: View = if state == VisualState::AiConversationBottomCompact {
        StackPanel::new().spacing(13.0).children((
            Border::new()
                .horizontal_alignment(HorizontalAlignment::Right)
                .margin(Thickness::new(0.0, -17.0, 10.0, 0.0))
                .content(
                    Border::new()
                        .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                        .background(Color::rgb(77, 166, 229))
                        .corner_radius(10.0)
                        .content(
                            TextBlock::new()
                                .text("Help me understand and fix “Excessive Temporary Files”.")
                                .font_size(12.5)
                                .foreground(Color::rgb(255, 255, 255)),
                        ),
                ),
            ai_error_message(palette),
            ai_user_message(palette, "What does CPU usage mean?"),
            ai_response_message(palette),
        ))
    } else {
        StackPanel::new().spacing(13.0).children((
            ai_user_message(
                palette,
                "Help me understand and fix “Excessive Temporary Files”.",
            ),
            ai_error_message(palette),
            ai_user_message(palette, "What does CPU usage mean?"),
            ai_response_message(palette),
        ))
    };

    ScrollViewer::new()
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Visible)
        .content(
            Border::new()
                .padding(Thickness::new(18.0, 14.0, 12.0, 14.0))
                .content(content),
        )
}

pub(crate) fn ai_issue_to_chat_body(palette: Palette) -> View {
    ScrollViewer::new()
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
        .content(
            Border::new()
                .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                .content(StackPanel::new().spacing(13.0).children((
                    ai_user_message(
                        palette,
                        "Help me understand and fix “Excessive Temporary Files”.",
                    ),
                    ai_assistant_message(
                        palette,
                        StackPanel::new().spacing(7.0).children((
                            Border::new()
                                .height(25.0)
                                .padding(Thickness::xy(11.0, 0.0))
                                .background(palette.active)
                                .corner_radius(999.0)
                                .horizontal_alignment(HorizontalAlignment::Left)
                                .content(
                                    TextBlock::new()
                                        .text("◯  Reasoning")
                                        .font_size(11.5)
                                        .foreground(palette.muted)
                                        .vertical_alignment(VerticalAlignment::Center),
                                ),
                            Border::new()
                                .width(176.0)
                                .height(42.0)
                                .padding(Thickness::xy(12.0, 0.0))
                                .background(palette.active)
                                .corner_radius(9.0)
                                .horizontal_alignment(HorizontalAlignment::Left)
                                .content(
                                    TextBlock::new()
                                        .text("◯▮")
                                        .font_size(15.0)
                                        .foreground(palette.accent)
                                        .vertical_alignment(VerticalAlignment::Center),
                                ),
                        )),
                    ),
                    Border::new()
                        .width(560.0)
                        .height(198.0)
                        .padding(Thickness::new(14.0, 13.0, 14.0, 13.0))
                        .background(Color::argb(195, 65, 58, 31))
                        .border_brush(palette.warn)
                        .border_thickness(1.0)
                        .corner_radius(9.0)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .content(StackPanel::new().spacing(9.0).children((
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .spacing(9.0)
                                .children((
                                    Image::new()
                                        .source_data(EncodedImage::from_static(ISSUE_WARN_DARK))
                                        .width(14.0)
                                        .height(14.0),
                                    TextBlock::new()
                                        .text("Continue with ChatGPT via Codex?")
                                        .font_size(15.0)
                                        .font_weight(FontWeight::BOLD),
                                )),
                            TextBlock::new()
                                .text("The private provider could not finish. Continuing sends this question and its selected diagnostic context to a subscription cloud provider. This choice is remembered and can be changed in Settings.")
                                .font_size(11.5)
                                .text_wrapping(TextWrapping::Wrap),
                            TextBlock::new()
                                .text("This provider cannot fit a reliable evidence packet: evidence budget is too small; at least 561 characters are required, but only 343 are available")
                                .font_size(11.5)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::Wrap),
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .spacing(8.0)
                                .children((
                                    Button::new()
                                        .height(32.0)
                                        .resource_overrides(primary_button_resources())
                                        .content("Allow cloud fallback"),
                                    Button::new().height(32.0).content("Keep data local"),
                                )),
                        ))),
                ))),
        )
}

pub(crate) fn bot_avatar(size: f64) -> View {
    Border::new()
        .width(size)
        .height(size)
        .corner_radius(size / 2.0)
        .content(
            Image::new()
                .source_data(EncodedImage::from_static(BOT_AVATAR))
                .width(size)
                .height(size)
                .stretch(Stretch::UniformToFill),
        )
}

pub(crate) fn primary_button_resources() -> ResourceOverrides {
    ResourceOverrides::new()
        .set("ButtonBackground", Color::rgb(15, 108, 189))
        .set("ButtonBackgroundPointerOver", Color::rgb(12, 90, 160))
        .set("ButtonBackgroundPressed", Color::rgb(7, 66, 111))
        .set("ButtonBackgroundDisabled", Color::argb(115, 15, 108, 189))
        .set("ButtonForeground", Color::rgb(254, 254, 254))
        .set("ButtonForegroundDisabled", Color::argb(150, 254, 254, 254))
        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
        .set("ButtonPadding", Thickness::xy(15.0, 0.0))
        .set("ControlCornerRadius", CornerRadius::uniform(7.0))
}

pub(crate) fn report_provider_attribution(
    fallback_provider: Option<&str>,
    provider_use: Option<&ProviderUse>,
) -> String {
    let Some(provider_use) = provider_use else {
        return fallback_provider.map_or_else(
            || "Generated by the local AI assistant".to_string(),
            |provider| format!("Generated by {provider}"),
        );
    };

    let provider = if provider_use.provider_id.trim().is_empty() {
        fallback_provider.unwrap_or("the AI assistant")
    } else {
        provider_use.provider_id.as_str()
    };
    let actual_models = provider_use.actual_models.join(", ");
    let requested_model = provider_use
        .requested_model
        .as_deref()
        .filter(|model| !model.trim().is_empty());

    let mut attribution = format!("Generated by {provider}");
    if !actual_models.is_empty() {
        attribution.push_str(" · ");
        attribution.push_str(&actual_models);
        if requested_model.is_some_and(|requested| {
            !provider_use
                .actual_models
                .iter()
                .any(|actual| actual == requested)
        }) {
            attribution.push_str(" (requested ");
            attribution.push_str(requested_model.unwrap_or_default());
            attribution.push(')');
        }
    } else if let Some(requested) = requested_model {
        attribution.push_str(" · requested ");
        attribution.push_str(requested);
    }
    if let Some(source) = provider_use
        .fallback_from
        .as_deref()
        .filter(|source| !source.trim().is_empty())
    {
        attribution.push_str(" · fallback from ");
        attribution.push_str(source);
    }
    attribution
}

#[allow(clippy::too_many_arguments)] // mirror ai_page's explicit view-parameter style
pub(crate) fn ai_scan_report_workspace(
    palette: Palette,
    workspace_height: f64,
    report_text: Option<&str>,
    report_provider: Option<&str>,
    report_provider_use: Option<&ProviderUse>,
    report_generating: bool,
    report_error: Option<&str>,
    deterministic_visual: bool,
    ai_enabled: bool,
    provider_loading: bool,
    provider_ready: bool,
    open_settings: Callback<()>,
    has_scan: bool,
    generate: Callback<()>,
    regenerate: Callback<()>,
    cancel: Callback<()>,
    copy: Callback<()>,
    preparation: AiPreparationUi<'_>,
    cancel_preparation: Callback<()>,
    retry_preparation: Callback<()>,
) -> View {
    // Same state order as the Tauri ScanReportPanel: an existing report is
    // always shown, otherwise the reasons a report cannot start come before
    // the invitation to generate one.
    let has_report = report_text.is_some();
    let can_generate = deterministic_visual || (ai_enabled && provider_ready && !provider_loading);
    let empty_state = |icon: FaIcon, title: &'static str, detail: &'static str, action: View| {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(9.0)
            .children((
                Border::new()
                    .width(48.0)
                    .height(48.0)
                    .background(palette.active)
                    .corner_radius(10.0)
                    .content(icons::path(icon).width(23.0).height(23.0)),
                TextBlock::new()
                    .text(title)
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD)
                    .text_wrapping(TextWrapping::Wrap)
                    .horizontal_alignment(HorizontalAlignment::Center),
                TextBlock::new()
                    .text(detail)
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(470.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                action,
            ))
    };
    let body: View = if preparation.is_report() {
        ai_preparation_panel(
            palette,
            "Preparing report evidence",
            "The health report will start automatically after the prerequisite scan completes.",
            preparation,
            cancel_preparation,
            retry_preparation,
        )
    } else if !has_scan && !has_report && !report_generating {
        empty_state(
            FaIcon::FileExport,
            "Run a scan to create a report",
            "A Quick Scan will run first, then the AI report will generate automatically.",
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(generate)
                .automation_name("Run Quick Scan and generate report")
                .content("Run Quick Scan & Generate"),
        )
    } else if !deterministic_visual && !ai_enabled && !has_report {
        empty_state(
            FaIcon::Gear,
            "AI insights are turned off",
            "Enable AI insights in Settings to create a report.",
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(open_settings)
                .automation_name("Open Settings")
                .content("Open Settings"),
        )
    } else if !deterministic_visual && provider_loading && !provider_ready && !has_report {
        empty_state(
            FaIcon::Refresh,
            "Checking AI provider…",
            "Report actions will be available when the provider check finishes.",
            View::empty(),
        )
    } else if !deterministic_visual && !provider_ready && !has_report {
        empty_state(
            FaIcon::CircleInfo,
            "Connect an AI provider",
            "Configure a local, subscription, or API provider before generating a report.",
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(open_settings)
                .automation_name("Configure AI")
                .content("Configure AI"),
        )
    } else if let Some(error) = report_error {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(9.0)
            .children((
                icons::path(FaIcon::TriangleExclamation)
                    .width(30.0)
                    .height(30.0),
                TextBlock::new()
                    .text("Report could not be generated")
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD)
                    .horizontal_alignment(HorizontalAlignment::Center),
                TextBlock::new()
                    .text(error.to_string())
                    .font_size(12.5)
                    .foreground(palette.err)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(470.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                Button::new()
                    .is_enabled(can_generate)
                    .on_click(regenerate)
                    .automation_name("Try again")
                    .content("Try again"),
            ))
    } else if has_report || report_generating {
        let text = report_text.unwrap_or_default();
        let header_actions: View = if report_generating {
            Button::new()
                .on_click(cancel)
                .automation_name("Stop report generation")
                .content("Stop")
        } else {
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    Button::new()
                        .on_click(copy)
                        .automation_name("Copy report")
                        .content("Copy"),
                    Button::new()
                        .is_enabled(can_generate)
                        .on_click(regenerate)
                        .automation_name("Regenerate report")
                        .content("Regenerate"),
                ))
        };
        let content: View = if text.is_empty() {
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    icons::path(FaIcon::Refresh).width(14.0).height(14.0),
                    TextBlock::new()
                        .text("Reading the latest scan…")
                        .font_size(12.5)
                        .foreground(palette.muted),
                ))
        } else {
            render_markdown_lite(
                text,
                MarkdownStyle::with_palette(palette.text, palette.card_strong, palette.border),
            )
        };
        ScrollViewer::new()
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .content(
                Border::new()
                    .padding(Thickness::new(24.0, 20.0, 24.0, 20.0))
                    .content(
                        StackPanel::new().spacing(12.0).children((
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .column_spacing(12.0)
                                .children((
                                    TextBlock::new()
                                        .text("Scan health report")
                                        .font_size(17.0)
                                        .font_weight(FontWeight::BOLD)
                                        .vertical_alignment(VerticalAlignment::Center),
                                    Border::new().grid_column(1).content(header_actions),
                                )),
                            TextBlock::new()
                                .text(report_provider_attribution(
                                    report_provider,
                                    report_provider_use,
                                ))
                                .font_size(11.5)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::Wrap),
                            content,
                        )),
                    ),
            )
    } else {
        empty_state(
            FaIcon::FileExport,
            "Ready to create your report",
            "A focused health report will summarize collected diagnostics, errors, risks, and next steps.",
            Button::new()
                .height(32.0)
                .resource_overrides(primary_button_resources())
                .on_click(generate)
                .automation_name("Generate report")
                .content("Generate report"),
        )
    };
    Border::new()
        .height(workspace_height)
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::policy::tests::provider_status;

    #[test]
    fn ai_provider_pill_is_fixture_exact_but_live_state_driven() {
        assert_eq!(
            ai_provider_pill_content("auto", true, true, None, false, None),
            (
                "Phi Silica".to_string(),
                "·  On device".to_string(),
                None,
                true,
                false
            )
        );
        assert_eq!(
            ai_provider_pill_content("auto", false, true, None, true, None),
            (
                "Checking AI provider".to_string(),
                "·  Please wait".to_string(),
                None,
                false,
                false
            )
        );
        assert_eq!(
            ai_provider_pill_content(
                "auto",
                false,
                false,
                Some(&provider_status(AIProvider::OpenAI)),
                false,
                None,
            ),
            (
                "AI disabled".to_string(),
                "·  Settings".to_string(),
                None,
                false,
                false
            )
        );
        assert_eq!(
            ai_provider_pill_content(
                "auto",
                false,
                true,
                Some(&provider_status(AIProvider::OpenAI)),
                false,
                None,
            ),
            (
                "OpenAI".to_string(),
                "·  API cloud".to_string(),
                None,
                true,
                true
            )
        );
        assert_eq!(
            ai_provider_pill_content(
                "auto",
                false,
                true,
                Some(&provider_status(AIProvider::OpenAI)),
                false,
                Some("worker stopped"),
            ),
            (
                "AI unavailable".to_string(),
                "·  Check Settings".to_string(),
                None,
                false,
                false
            )
        );
    }

    #[test]
    fn an_explicit_preference_never_falls_back_in_the_pill() {
        // Anthropic is preferred but only OpenAI is available: the pill says
        // so instead of silently advertising OpenAI.
        assert_eq!(
            ai_provider_pill_content(
                "anthropic",
                false,
                true,
                Some(&provider_status(AIProvider::OpenAI)),
                false,
                None,
            ),
            (
                "No provider".to_string(),
                "·  Not connected".to_string(),
                None,
                false,
                false
            )
        );
        // The preferred provider is available: it is named even while a
        // background refresh is running.
        assert_eq!(
            ai_provider_pill_content(
                "openai",
                false,
                true,
                Some(&provider_status(AIProvider::OpenAI)),
                true,
                None,
            ),
            (
                "OpenAI".to_string(),
                "·  API cloud".to_string(),
                None,
                true,
                true
            )
        );
    }
}
