//! The settings dialog and its rows.

#![deny(unsafe_code)]

use crate::app::consts::{
    AI_PROVIDER_IDS, AI_PROVIDER_LABELS, PROVIDER_KEY_LABELS, PROVIDER_SETUP_LABELS,
    QUICK_SCAN_TASK_IDS, SETTINGS_MAX_CONCURRENT_TASKS,
};
use crate::app::policy::{
    PhiPreferenceGate, codex_model_options, provider_setup_model, provider_setup_provider,
    selected_setting_index, subscription_auth_provider_for_setup,
    subscription_install_progress_label,
};
use crate::screens::ai::view::primary_button_resources;
use crate::widgets::badges::status_pill;
use crate::widgets::chrome::fa_icon_label;
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use wfdiag_app::domain::catalog::CatalogState as ProviderCatalogUiState;
use wfdiag_app::domain::subscriptions::AccountState as SubscriptionAuthUiState;
use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthState;
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallProgress;
use wfdiag_native_ai_chat::{SubscriptionAuthOperation, SubscriptionAuthProvider};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_diagnostics::DiagnosticTask;
use wfdiag_native_settings::{AppSettings, CloudFallbackPolicy, ProviderKeyId};
use windows_reactor::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_dialog(
    palette: Palette,
    theme: WindowTheme,
    bottom: bool,
    settings: &AppSettings,
    phi_preference_gate: &PhiPreferenceGate,
    provider_setup_partial: bool,
    provider_setup_index: usize,
    provider_catalog_state: Option<&ProviderCatalogUiState>,
    subscription_auth_state: Option<&SubscriptionAuthUiState>,
    subscription_auth_runtime_error: Option<&str>,
    subscription_install_active: bool,
    subscription_install_progress: Option<&SubscriptionInstallProgress>,
    subscription_install_error: Option<&str>,
    editable: bool,
    can_save: bool,
    saving: bool,
    operation_status: Option<(String, bool)>,
    theme_changed: Callback<Option<usize>>,
    export_format_changed: Callback<Option<usize>>,
    auto_save_changed: Callback<bool>,
    notifications_changed: Callback<bool>,
    scan_on_startup_changed: Callback<bool>,
    close_to_tray_changed: Callback<bool>,
    max_concurrent_tasks_changed: Callback<Option<f64>>,
    ai_enabled_changed: Callback<bool>,
    preferred_ai_provider_changed: Callback<Option<usize>>,
    cloud_fallback_changed: Callback<Option<usize>>,
    network_grounding_changed: Callback<bool>,
    codex_cli_path_changed: Callback<String>,
    codex_model_changed: Callback<Option<usize>>,
    provider_setup_changed: Callback<Option<usize>>,
    provider_model_changed: Callback<Option<usize>>,
    refresh_provider_models: Callback<()>,
    cancel_provider_models: Callback<()>,
    refresh_subscription_auth: Callback<()>,
    start_subscription_sign_in: Callback<()>,
    start_subscription_sign_out: Callback<()>,
    cancel_subscription_auth: Callback<()>,
    request_subscription_install: Callback<()>,
    cancel_subscription_install: Callback<()>,
    provider_text_changed: Callback<(usize, String)>,
    cancel: Callback<()>,
    save: Callback<()>,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
    provider_keys_set: [bool; ProviderKeyId::ALL.len()],
    key_busy: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
    scan_catalog: &[DiagnosticTask],
    toggle_quick_task: Callback<String>,
) -> View {
    let actions: View = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .horizontal_alignment(HorizontalAlignment::Right)
        .vertical_alignment(VerticalAlignment::Center)
        .children((
            Button::new()
                .width(71.0)
                .height(32.0)
                .is_enabled(!saving)
                .on_click(cancel.clone())
                .content("Cancel"),
            Button::new()
                .width(59.0)
                .height(32.0)
                .is_enabled(can_save)
                .resource_overrides(primary_button_resources())
                .on_click(save)
                .content(if saving { "Saving…" } else { "Save" }),
        ));
    let footer: View = if let Some((status, is_error)) = operation_status {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .column_spacing(12.0)
            .children((
                TextBlock::new()
                    .text(status)
                    .font_size(10.5)
                    .foreground(if is_error { palette.err } else { palette.muted })
                    .text_wrapping(TextWrapping::Wrap)
                    .vertical_alignment(VerticalAlignment::Center),
                Border::new().grid_column(1).content(actions),
            ))
    } else {
        actions
    };

    Border::new()
        .grid_row_span(2)
        // Reactor does not yet expose the CSS backdrop-filter used by the
        // Store shell. A slightly stronger scrim suppresses the otherwise
        // sharp page detail while retaining the same modal hierarchy.
        .background(Color::argb(140, 0, 0, 0))
        .content(
            Border::new()
                .width(640.0)
                .height(810.0)
                .margin(Thickness::new(0.0, 0.0, 12.0, 0.0))
                .automation_name("Settings dialog")
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center)
                .background(palette.card_strong)
                .border_brush(palette.border)
                .border_thickness(1.0)
                .corner_radius(10.0)
                .content(
                    Grid::new()
                        .rows([
                            GridLength::Pixel(58.0),
                            GridLength::Star(1.0),
                            GridLength::Pixel(60.0),
                        ])
                        .children((
                            Border::new()
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
                                        .column_spacing(11.0)
                                        .children((
                                            Border::new()
                                                .width(3.0)
                                                .height(15.0)
                                                .background(palette.accent)
                                                .corner_radius(999.0)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBlock::new()
                                                .text("Settings")
                                                .grid_column(1)
                                                .font_size(13.0)
                                                .font_weight(FontWeight::SEMI_BOLD)
                                                .automation_heading_level(
                                                    AutomationHeadingLevel::Level1,
                                                )
                                                .vertical_alignment(VerticalAlignment::Center),
                                            Button::new()
                                                .grid_column(2)
                                                .width(34.0)
                                                .height(34.0)
                                                .style(ButtonStyle::Subtle)
                                                // Keep the Subtle style's theme resources intact.
                                                // Replacing its full button resource set here makes
                                                // WinUI raise E_FAIL while opening Settings.
                                                .resource_overrides(
                                                    ResourceOverrides::new().set(
                                                        "ButtonPadding",
                                                        Thickness::uniform(0.0),
                                                    ),
                                                )
                                                .horizontal_content_alignment(
                                                    HorizontalAlignment::Center,
                                                )
                                                .vertical_content_alignment(
                                                    VerticalAlignment::Center,
                                                )
                                                .is_enabled(!saving)
                                                .on_click(cancel.clone())
                                                .automation_name("Close Settings")
                                                .content(
                                                    Viewbox::new()
                                                        .width(12.0)
                                                        .height(12.0)
                                                        .stretch(Stretch::Uniform)
                                                        .slot(
                                                            ViewboxSlot::Child,
                                                            FontIcon::new().glyph("\u{E711}"),
                                                        ),
                                                ),
                                        )),
                                ),
                            Border::new()
                                .grid_row(1)
                                .padding(Thickness::new(14.0, 0.0, 23.0, 0.0))
                                .content(settings_content(
                                    theme,
                                    bottom,
                                    settings,
                                    phi_preference_gate,
                                    provider_setup_partial,
                                    provider_setup_index,
                                    provider_catalog_state,
                                    subscription_auth_state,
                                    subscription_auth_runtime_error,
                                    subscription_install_active,
                                    subscription_install_progress,
                                    subscription_install_error,
                                    editable,
                                    scan_catalog,
                                    toggle_quick_task,
                                    provider_key_drafts,
                                    provider_keys_set,
                                    key_busy,
                                    key_draft_changed,
                                    key_store,
                                    key_clear,
                                    theme_changed,
                                    export_format_changed,
                                    auto_save_changed,
                                    notifications_changed,
                                    scan_on_startup_changed,
                                    close_to_tray_changed,
                                    max_concurrent_tasks_changed,
                                    ai_enabled_changed,
                                    preferred_ai_provider_changed,
                                    cloud_fallback_changed,
                                    network_grounding_changed,
                                    codex_cli_path_changed,
                                    codex_model_changed,
                                    provider_setup_changed,
                                    provider_model_changed,
                                    refresh_provider_models,
                                    cancel_provider_models,
                                    refresh_subscription_auth,
                                    start_subscription_sign_in,
                                    start_subscription_sign_out,
                                    cancel_subscription_auth,
                                    request_subscription_install,
                                    cancel_subscription_install,
                                    provider_text_changed,
                                )),
                            Border::new()
                                .grid_row(2)
                                .padding(Thickness::xy(18.0, 0.0))
                                .background(palette.card_strong)
                                .border_brush(palette.border)
                                .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                                .content(footer),
                        )),
                ),
        )
}

/// Quick Scan customization section: pick which catalog tasks a customized
/// Quick Scan runs. Saved through the normal settings Save path
/// (`quick_scan_tasks`); an empty selection restores the shipping defaults.
pub(crate) fn settings_quick_scan_tasks_section(
    palette: Palette,
    catalog: &[DiagnosticTask],
    settings: &AppSettings,
    editable: bool,
    toggle_task: Callback<String>,
) -> View {
    let effective: Vec<String> = settings
        .quick_scan_tasks
        .clone()
        .filter(|tasks| !tasks.is_empty())
        .unwrap_or_else(|| {
            QUICK_SCAN_TASK_IDS
                .iter()
                .map(|id| (*id).to_string())
                .collect()
        });
    let rows: Vec<KeyedView> = catalog
        .iter()
        .map(|task| {
            let checked = effective.iter().any(|id| id == &task.id);
            let toggle = toggle_task.clone();
            let task_id = task.id.clone();
            let admin_note = if task.admin_required {
                Some(" · admin")
            } else {
                None
            };
            let note_text = admin_note.map(|note| format!("{}{}", task.description, note));
            let hint: View = note_text
                .as_deref()
                .map(|hint| {
                    View::from(
                        TextBlock::new()
                            .text(hint.to_string())
                            .font_size(11.0)
                            .foreground(palette.muted),
                    )
                })
                .unwrap_or_else(View::empty);
            KeyedView::new(
                task.id.clone(),
                Border::new()
                    .height(44.0)
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        Grid::new()
                            .columns([GridLength::Star(1.0), GridLength::Pixel(24.0)])
                            .children((
                                Border::new()
                                    .vertical_alignment(VerticalAlignment::Center)
                                    .content(StackPanel::new().spacing(2.0).children((
                                        TextBlock::new().text(task.name.clone()).font_size(12.5),
                                        hint,
                                    ))),
                                Border::new()
                                    .grid_column(1)
                                    .width(24.0)
                                    .height(32.0)
                                    .margin(Thickness::new(0.0, 0.0, 4.0, 0.0))
                                    .horizontal_alignment(HorizontalAlignment::Right)
                                    .vertical_alignment(VerticalAlignment::Center)
                                    .content(
                                        CheckBox::new()
                                            .is_checked(checked)
                                            .is_enabled(editable)
                                            .automation_name(format!(
                                                "Quick Scan task: {}",
                                                task.name
                                            ))
                                            .on_is_checked_changed(move |_| {
                                                let _ = toggle.call(task_id.clone());
                                            })
                                            .width(14.0)
                                            .height(14.0),
                                    ),
                            )),
                    ),
            )
        })
        .collect();
    StackPanel::new()
        .spacing(4.0)
        .children((
            settings_section(palette, "QUICK SCAN TASKS"),
            Border::new()
                .padding(Thickness::new(0.0, 8.0, 0.0, 5.0))
                .content(
                    TextBlock::new()
                        .text("Choose which diagnostics a customized Quick Scan runs. Detection-only tasks stay included automatically; an empty selection restores the defaults.")
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap),
                ),
            StackPanel::new().keyed_children(rows),
        ))
}

/// API keys section: DPAPI-backed credential entry per provider. Shared by
/// both settings layouts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_provider_keys_section(
    palette: Palette,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
    provider_keys_set: [bool; ProviderKeyId::ALL.len()],
    key_busy: bool,
    editable: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
) -> View {
    let rows: Vec<KeyedView> = PROVIDER_KEY_LABELS
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let draft_changed = key_draft_changed.clone();
            let store = key_store.clone();
            let clear = key_clear.clone();
            let set = provider_keys_set[index];
            let draft = &provider_key_drafts[index];
            KeyedView::new(
                *label,
                settings_wrapped_row(
                    palette,
                    label,
                    Some(if set {
                        "A key is stored for this provider"
                    } else {
                        "No key stored yet"
                    }),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(6.0)
                        .children((
                            PasswordBox::new()
                                .width(200.0)
                                .height(32.0)
                                .password(draft.clone())
                                .is_enabled(editable && !key_busy)
                                .on_password_changed(move |value| {
                                    let _ = draft_changed.call((index, value));
                                })
                                .automation_name(format!("{label} API key")),
                            Button::new()
                                .height(32.0)
                                .width(58.0)
                                .is_enabled(editable && !key_busy)
                                .on_click(move || {
                                    let _ = store.call(index);
                                })
                                .content("Stage"),
                            Button::new()
                                .height(32.0)
                                .width(68.0)
                                .is_enabled(editable && !key_busy && set)
                                .on_click(move || {
                                    let _ = clear.call(index);
                                })
                                .content("Remove"),
                        )),
                    58.0,
                ),
            )
        })
        .collect();
    StackPanel::new()
        .spacing(4.0)
        .children((
            settings_section(palette, "API KEYS"),
            Border::new()
                .padding(Thickness::new(0.0, 8.0, 0.0, 5.0))
                .content(
                    TextBlock::new()
                        .text("Credential edits remain in this dialog until you press Save. Cancel discards them; committed keys use Windows DPAPI and never enter settings.json.")
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap),
                ),
            StackPanel::new().keyed_children(rows),
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn provider_text_setting_row(
    palette: Palette,
    label: &'static str,
    hint: &'static str,
    value: Option<&str>,
    placeholder: &'static str,
    editable: bool,
    field: usize,
    changed: Callback<(usize, String)>,
) -> View {
    settings_wrapped_row(
        palette,
        label,
        Some(hint),
        TextBox::new()
            .width(260.0)
            .height(32.0)
            .text(value.unwrap_or_default())
            .placeholder_text(placeholder)
            .is_enabled(editable)
            .automation_name(label)
            .on_text_changed(move |value| {
                let _ = changed.call((field, value));
            }),
        92.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn provider_model_catalog_row(
    palette: Palette,
    setup_index: usize,
    current_model: Option<&str>,
    state: Option<&ProviderCatalogUiState>,
    editable: bool,
    selection_changed: Callback<Option<usize>>,
    refresh: Callback<()>,
    cancel: Callback<()>,
) -> View {
    if provider_setup_provider(setup_index) == Some(AIProvider::PhiSilica) {
        return View::empty();
    }
    let state = state.cloned().unwrap_or_default();
    let mut items = vec![
        state
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.default_model.as_deref())
            .map_or_else(
                || "Use provider default".to_string(),
                |model| format!("Default ({model})"),
            ),
    ];
    if let Some(catalog) = state.catalog.as_ref() {
        items.extend(catalog.models.iter().map(|model| {
            model
                .label
                .as_deref()
                .filter(|label| *label != model.id.as_str())
                .map_or_else(
                    || model.id.clone(),
                    |label| format!("{label} · {}", model.id),
                )
        }));
    }
    let selected_index = match current_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        None => Some(0),
        Some(current) => state.catalog.as_ref().and_then(|catalog| {
            catalog
                .models
                .iter()
                .position(|model| model.id == current)
                .map(|index| index + 1)
        }),
    };
    let status = if let Some(error) = state.error.as_deref() {
        if state.stale {
            format!("Could not refresh models: {error} · showing the last successful list")
        } else {
            format!("Could not load models: {error} · enter a model ID manually")
        }
    } else if let Some(blocked) = state.blocked.as_deref() {
        format!("{blocked} You can still enter a model ID manually.")
    } else if state.loading {
        if state.catalog.is_some() {
            "Refreshing models…".to_string()
        } else {
            "Loading models…".to_string()
        }
    } else if state.stale {
        "Showing the last successful model list.".to_string()
    } else if state
        .catalog
        .as_ref()
        .is_some_and(|catalog| catalog.models.is_empty())
    {
        "The provider reported no models; manual entry remains available.".to_string()
    } else {
        String::new()
    };
    let action = if state.loading {
        Button::new()
            .height(32.0)
            .width(72.0)
            .on_click(cancel)
            .content("Cancel")
    } else {
        Button::new()
            .height(32.0)
            .width(72.0)
            .is_enabled(editable)
            .on_click(refresh)
            .content("Refresh")
    };
    settings_wrapped_row(
        palette,
        "Available models",
        Some("Live provider catalog; manual model IDs above always remain available"),
        StackPanel::new().spacing(5.0).children((
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .children((
                    ComboBox::new()
                        .width(182.0)
                        .height(32.0)
                        .items_source(items)
                        .selected_index(selected_index)
                        .is_enabled(editable && state.catalog.is_some())
                        .automation_name("Available provider models")
                        .on_selection_changed(selection_changed),
                    action,
                )),
            TextBlock::new()
                .text(status)
                .font_size(10.5)
                .foreground(if state.error.is_some() {
                    palette.err
                } else {
                    palette.muted
                })
                .text_wrapping(TextWrapping::Wrap),
        )),
        112.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn subscription_auth_row(
    palette: Palette,
    setup_index: usize,
    state: Option<&SubscriptionAuthUiState>,
    runtime_error: Option<&str>,
    editable: bool,
    refresh: Callback<()>,
    sign_in: Callback<()>,
    sign_out: Callback<()>,
    cancel: Callback<()>,
) -> View {
    let Some(provider) = subscription_auth_provider_for_setup(setup_index) else {
        return View::empty();
    };
    let state = state.cloned().unwrap_or_default();
    let (label, hint) = match provider {
        SubscriptionAuthProvider::Codex => (
            "ChatGPT account",
            "Uses the Codex CLI login; this app never stores an OpenAI token",
        ),
        SubscriptionAuthProvider::ClaudeCode => (
            "Claude account",
            "Uses the Claude Code CLI login; this app never stores an Anthropic token",
        ),
    };
    let effective_error = state.error.as_deref().or(runtime_error);
    let status = match state.operation {
        Some(SubscriptionAuthOperation::Status) => "Checking…",
        Some(SubscriptionAuthOperation::SignIn) => "Waiting for browser…",
        Some(SubscriptionAuthOperation::SignOut) => "Signing out…",
        None => match state.status.as_ref().map(|status| status.state) {
            Some(SubscriptionAuthState::NotInstalled) => "CLI not detected",
            Some(SubscriptionAuthState::SignedOut) => "Signed out",
            Some(SubscriptionAuthState::SignedIn) => "Signed in",
            Some(SubscriptionAuthState::Unknown) => "Status unclear",
            None => "Not checked",
        },
    };
    let detail = if let Some(error) = effective_error {
        error.to_string()
    } else if state.operation == Some(SubscriptionAuthOperation::SignIn) {
        "Complete sign-in in the browser window opened by the vendor CLI.".to_string()
    } else {
        match state.status.as_ref() {
            Some(status) if status.state == SubscriptionAuthState::NotInstalled => format!(
                "Install the official {} CLI, then check again. WFDiag never installs command-line tools silently.",
                status.provider
            ),
            Some(status) if status.state == SubscriptionAuthState::Unknown => {
                "The CLI was found, but its account status could not be confirmed.".to_string()
            }
            Some(status) => status.path.as_ref().map_or_else(
                || "Account status was reported by the vendor CLI.".to_string(),
                |path| format!("CLI: {}", path.display()),
            ),
            None => "Check the locally installed vendor CLI account status.".to_string(),
        }
    };
    let action: View = if state.operation.is_some() {
        Button::new()
            .height(32.0)
            .width(82.0)
            .is_enabled(editable)
            .on_click(cancel)
            .content("Cancel")
    } else {
        match state.status.as_ref().map(|status| status.state) {
            Some(SubscriptionAuthState::SignedIn) => Button::new()
                .height(32.0)
                .width(82.0)
                .is_enabled(editable)
                .on_click(sign_out)
                .content("Sign out"),
            Some(SubscriptionAuthState::SignedOut | SubscriptionAuthState::Unknown) => {
                Button::new()
                    .height(32.0)
                    .width(82.0)
                    .is_enabled(editable)
                    .resource_overrides(primary_button_resources())
                    .on_click(sign_in)
                    .content("Sign in")
            }
            _ => Button::new()
                .height(32.0)
                .width(82.0)
                .is_enabled(editable)
                .on_click(refresh)
                .content("Check"),
        }
    };
    settings_wrapped_row(
        palette,
        label,
        Some(hint),
        StackPanel::new().width(260.0).spacing(5.0).children((
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .horizontal_alignment(HorizontalAlignment::Right)
                .children((status_pill(status, palette.accent, palette.active), action)),
            TextBlock::new()
                .text(detail)
                .font_size(10.5)
                .foreground(if effective_error.is_some() {
                    palette.err
                } else {
                    palette.muted
                })
                .text_wrapping(TextWrapping::Wrap),
        )),
        126.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn subscription_install_row(
    palette: Palette,
    setup_index: usize,
    auth_state: Option<&SubscriptionAuthUiState>,
    active: bool,
    progress: Option<&SubscriptionInstallProgress>,
    error: Option<&str>,
    editable: bool,
    install: Callback<()>,
    cancel: Callback<()>,
) -> View {
    let Some(provider) = subscription_auth_provider_for_setup(setup_index) else {
        return View::empty();
    };
    let not_installed = auth_state
        .and_then(|state| state.status.as_ref())
        .is_some_and(|status| status.state == SubscriptionAuthState::NotInstalled);
    if !not_installed && !active && error.is_none() {
        return View::empty();
    }
    let provider_label = match provider {
        SubscriptionAuthProvider::Codex => "Codex CLI",
        SubscriptionAuthProvider::ClaudeCode => "Claude Code CLI",
    };
    let install_label = match provider {
        SubscriptionAuthProvider::Codex => "Install Codex CLI",
        SubscriptionAuthProvider::ClaudeCode => "Install Claude Code CLI",
    };
    let detail = if active {
        progress.map_or_else(
            || "Preparing the approved installer…".to_string(),
            |progress| subscription_install_progress_label(*progress).to_string(),
        )
    } else if let Some(error) = error {
        error.to_string()
    } else {
        "Uses winget first. If winget cannot finish, WFDiag asks again before running the vendor's PowerShell installer. Installation never signs in automatically."
            .to_string()
    };
    let action: View = if active {
        Button::new()
            .height(32.0)
            .width(82.0)
            .on_click(cancel)
            .automation_name(format!("Cancel {provider_label} installation"))
            .content("Cancel")
    } else {
        Button::new()
            .height(32.0)
            .width(82.0)
            .is_enabled(editable)
            .resource_overrides(primary_button_resources())
            .on_click(install)
            .automation_name(format!("Install {provider_label}"))
            .content("Install")
    };
    settings_wrapped_row(
        palette,
        install_label,
        Some("Explicit confirmation is required before any installer runs"),
        StackPanel::new().width(260.0).spacing(6.0).children((
            action,
            TextBlock::new()
                .text(detail)
                .font_size(10.5)
                .foreground(if error.is_some() {
                    palette.err
                } else {
                    palette.muted
                })
                .text_wrapping(TextWrapping::Wrap),
        )),
        130.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn provider_setup_fields(
    palette: Palette,
    settings: &AppSettings,
    provider_setup_index: usize,
    editable: bool,
    provider_text_changed: Callback<(usize, String)>,
    codex_cli_path_changed: Callback<String>,
    codex_model_changed: Callback<Option<usize>>,
) -> View {
    let text_row = |label,
                    hint,
                    value: Option<&str>,
                    placeholder,
                    field,
                    callback: &Callback<(usize, String)>| {
        provider_text_setting_row(
            palette,
            label,
            hint,
            value,
            placeholder,
            editable,
            field,
            callback.clone(),
        )
    };
    match provider_setup_index {
        0 => text_row(
            "Phi Silica LAF token",
            "Optional Microsoft-issued token; generation still requires the Store identity on a Copilot+ PC",
            settings.phi_silica_laf_token.as_deref(),
            "Leave empty for the built-in token",
            0,
            &provider_text_changed,
        ),
        1 => StackPanel::new().children((
            text_row(
                "Foundry Local endpoint",
                "Optional. Empty auto-discovers the running Foundry Local service",
                settings.local_ai_endpoint.as_deref(),
                "http://127.0.0.1:55769",
                1,
                &provider_text_changed,
            ),
            text_row(
                "Foundry Local model",
                "Empty uses the service default; manual IDs remain available if discovery is unavailable",
                settings.local_ai_model.as_deref(),
                "Use service default",
                2,
                &provider_text_changed,
            ),
        )),
        2 => StackPanel::new().children((
            text_row(
                "Ollama endpoint",
                "Optional. Empty uses Ollama's default local port",
                settings.ollama_endpoint.as_deref(),
                "http://127.0.0.1:11434",
                3,
                &provider_text_changed,
            ),
            text_row(
                "Ollama model",
                "Empty uses the first installed model; manual entry remains available",
                settings.ollama_model.as_deref(),
                "Auto (first installed)",
                4,
                &provider_text_changed,
            ),
        )),
        3 => {
            let (model_items, model_index) = codex_model_options(settings);
            StackPanel::new().children((
                settings_wrapped_row(
                    palette,
                    "Codex CLI path",
                    Some("Optional. Empty auto-detects codex"),
                    TextBox::new()
                        .width(260.0)
                        .height(32.0)
                        .text(settings.codex_cli_path.clone().unwrap_or_default())
                        .placeholder_text("Auto-detected")
                        .is_enabled(editable)
                        .on_text_changed(codex_cli_path_changed),
                    92.0,
                ),
                settings_wrapped_row(
                    palette,
                    "Codex model",
                    Some("Optional. Empty uses the CLI default"),
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(model_items)
                        .selected_index(model_index)
                        .is_enabled(editable)
                        .on_selection_changed(codex_model_changed),
                    92.0,
                ),
            ))
        }
        4 => StackPanel::new().children((
            text_row(
                "Claude Code CLI path",
                "Optional. Empty auto-detects claude",
                settings.claude_cli_path.as_deref(),
                "Auto-detected",
                5,
                &provider_text_changed,
            ),
            text_row(
                "Claude Code model",
                "Optional. Empty uses the CLI default; Sonnet 5 is the app default for Anthropic API calls",
                settings.claude_model.as_deref(),
                "Use Claude Code default",
                6,
                &provider_text_changed,
            ),
        )),
        5 => text_row(
            "OpenAI model",
            "Empty uses the app default; enter a model ID manually if discovery is unavailable",
            settings.open_ai_model.as_deref(),
            "Use app default",
            7,
            &provider_text_changed,
        ),
        6 => text_row(
            "Anthropic model",
            "Empty uses claude-sonnet-5",
            settings.anthropic_model.as_deref(),
            "claude-sonnet-5",
            8,
            &provider_text_changed,
        ),
        7 => text_row(
            "Gemini model",
            "Empty discovers the newest supported GA model; manual entry remains available",
            settings.gemini_model.as_deref(),
            "gemini-3.6-flash",
            9,
            &provider_text_changed,
        ),
        8 => text_row(
            "DeepSeek model",
            "Empty uses the app default; manual entry remains available",
            settings.deepseek_model.as_deref(),
            "deepseek-v4-flash",
            10,
            &provider_text_changed,
        ),
        _ => StackPanel::new().children((
            text_row(
                "Endpoint URL",
                "OpenRouter, Groq, or any OpenAI-compatible /v1/chat/completions endpoint",
                settings.custom_endpoint.as_deref(),
                "https://openrouter.ai/api",
                11,
                &provider_text_changed,
            ),
            text_row(
                "Custom model",
                "Required. Enter the exact model ID documented by the provider",
                settings.custom_model.as_deref(),
                "Provider model ID",
                12,
                &provider_text_changed,
            ),
        )),
    }
}

pub(crate) fn settings_phi_preference_status(palette: Palette, gate: &PhiPreferenceGate) -> View {
    let (message, icon) = match gate {
        PhiPreferenceGate::Checking => {
            (gate.blocking_reason().unwrap_or_default(), FaIcon::Refresh)
        }
        PhiPreferenceGate::Ready => return View::empty(),
        PhiPreferenceGate::Blocked(_) => (
            gate.blocking_reason().unwrap_or_default(),
            FaIcon::CircleInfo,
        ),
    };

    Border::new()
        .padding(Thickness::new(0.0, 0.0, 0.0, 8.0))
        .automation_name("Phi Silica availability")
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(7.0)
                .children((
                    icons::path(icon).width(13.0).height(13.0),
                    TextBlock::new()
                        .text(message)
                        .font_size(10.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap)
                        .max_width(545.0),
                )),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_content(
    theme: WindowTheme,
    bottom: bool,
    settings: &AppSettings,
    phi_preference_gate: &PhiPreferenceGate,
    provider_setup_partial: bool,
    provider_setup_index: usize,
    provider_catalog_state: Option<&ProviderCatalogUiState>,
    subscription_auth_state: Option<&SubscriptionAuthUiState>,
    subscription_auth_runtime_error: Option<&str>,
    subscription_install_active: bool,
    subscription_install_progress: Option<&SubscriptionInstallProgress>,
    subscription_install_error: Option<&str>,
    editable: bool,
    scan_catalog: &[DiagnosticTask],
    toggle_quick_task: Callback<String>,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
    provider_keys_set: [bool; ProviderKeyId::ALL.len()],
    key_busy: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
    theme_changed: Callback<Option<usize>>,
    export_format_changed: Callback<Option<usize>>,
    auto_save_changed: Callback<bool>,
    notifications_changed: Callback<bool>,
    scan_on_startup_changed: Callback<bool>,
    close_to_tray_changed: Callback<bool>,
    max_concurrent_tasks_changed: Callback<Option<f64>>,
    ai_enabled_changed: Callback<bool>,
    preferred_ai_provider_changed: Callback<Option<usize>>,
    cloud_fallback_changed: Callback<Option<usize>>,
    network_grounding_changed: Callback<bool>,
    codex_cli_path_changed: Callback<String>,
    codex_model_changed: Callback<Option<usize>>,
    provider_setup_changed: Callback<Option<usize>>,
    provider_model_changed: Callback<Option<usize>>,
    refresh_provider_models: Callback<()>,
    cancel_provider_models: Callback<()>,
    refresh_subscription_auth: Callback<()>,
    start_subscription_sign_in: Callback<()>,
    start_subscription_sign_out: Callback<()>,
    cancel_subscription_auth: Callback<()>,
    request_subscription_install: Callback<()>,
    cancel_subscription_install: Callback<()>,
    provider_text_changed: Callback<(usize, String)>,
) -> View {
    let palette = Palette::for_theme(theme);
    if bottom {
        return settings_content_bottom(
            palette,
            theme,
            settings,
            provider_setup_partial,
            editable,
            scan_catalog,
            toggle_quick_task,
            provider_key_drafts,
            provider_keys_set,
            key_busy,
            key_draft_changed,
            key_store,
            key_clear,
            theme_changed,
            export_format_changed,
            auto_save_changed,
            notifications_changed,
            scan_on_startup_changed,
            close_to_tray_changed,
            max_concurrent_tasks_changed,
            codex_cli_path_changed,
            codex_model_changed,
        );
    }
    let theme_index = match theme {
        WindowTheme::System => 0,
        WindowTheme::Light => 1,
        WindowTheme::Dark => 2,
    };
    let export_format_index =
        selected_setting_index(&settings.export_format, &["text", "json", "html"]);
    let provider_index = selected_setting_index(&settings.preferred_ai_provider, &AI_PROVIDER_IDS);
    let mut provider_labels = AI_PROVIDER_LABELS.map(str::to_string);
    match phi_preference_gate {
        PhiPreferenceGate::Checking => provider_labels[1].push_str(" — checking"),
        PhiPreferenceGate::Blocked(_) => provider_labels[1].push_str(" — unavailable"),
        PhiPreferenceGate::Ready => {}
    }
    let cloud_fallback_index = Some(match settings.cloud_fallback_policy {
        CloudFallbackPolicy::Ask => 0,
        CloudFallbackPolicy::Allow => 1,
        CloudFallbackPolicy::Never => 2,
    });

    ScrollViewer::new()
        .width(593.0)
        .height(690.0)
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .content(
            StackPanel::new().children((
                settings_section(palette, "AI ASSISTANT"),
                Border::new()
                    .padding(Thickness::new(0.0, 8.0, 0.0, 5.0))
                    .content(
                        TextBlock::new()
                            .text("Choose how AI is used across Assistant, Scan Report, and issue explanations. Provider credentials are managed below.")
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::Wrap),
                    ),
                settings_check_row(
                    palette,
                    "Enable AI insights",
                    None,
                    CheckBox::new()
                        .is_checked(settings.ai_enabled)
                        .is_enabled(editable)
                        .automation_name("Enable AI insights")
                        .on_is_checked_changed(ai_enabled_changed),
                    44.0,
                ),
                StackPanel::new().children((
                    settings_row(
                        palette,
                        "AI provider",
                        Some("Auto picks local first, then configured cloud providers"),
                        ComboBox::new()
                            .width(260.0)
                            .height(32.0)
                            .items_source(provider_labels)
                            .selected_index(provider_index)
                            .is_enabled(editable)
                            .automation_name("AI provider")
                            .on_selection_changed(preferred_ai_provider_changed),
                        59.0,
                    ),
                    settings_phi_preference_status(palette, phi_preference_gate),
                )),
                settings_row(
                    palette,
                    "Cloud fallback",
                    Some("When Auto cannot finish with an on-device or local provider"),
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source([
                            "Ask every time",
                            "Allow automatically",
                            "Never use cloud fallback",
                        ])
                        .selected_index(cloud_fallback_index)
                        .is_enabled(editable)
                        .automation_name("Cloud fallback policy")
                        .on_selection_changed(cloud_fallback_changed),
                    59.0,
                ),
                settings_check_row(
                    palette,
                    "Web grounding",
                    Some("Allow supported providers to look up current public information"),
                    CheckBox::new()
                        .is_checked(settings.network_grounding_enabled)
                        .is_enabled(editable)
                        .automation_name("Enable web grounding")
                        .on_is_checked_changed(network_grounding_changed),
                    59.0,
                ),
                settings_section(palette, "PROVIDER SETUP"),
                Border::new()
                    .padding(Thickness::new(0.0, 8.0, 0.0, 5.0))
                    .content(
                        TextBlock::new()
                            .text("Configure credentials for any provider here — independent of which one is active above. Local providers keep prompts on this PC; subscription and API providers receive only the question and selected diagnostic context.")
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::Wrap),
                    ),
                settings_wrapped_row(
                    palette,
                    "Set up provider",
                    Some("Browse and edit any provider's settings, whether or not it's currently active"),
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(PROVIDER_SETUP_LABELS)
                        .selected_index(Some(provider_setup_index))
                        .is_enabled(editable)
                        .on_selection_changed(provider_setup_changed),
                    100.0,
                ),
                provider_setup_fields(
                    palette,
                    settings,
                    provider_setup_index,
                    editable,
                    provider_text_changed,
                    codex_cli_path_changed,
                    codex_model_changed,
                ),
                subscription_auth_row(
                    palette,
                    provider_setup_index,
                    subscription_auth_state,
                    subscription_auth_runtime_error,
                    editable,
                    refresh_subscription_auth,
                    start_subscription_sign_in,
                    start_subscription_sign_out,
                    cancel_subscription_auth,
                ),
                subscription_install_row(
                    palette,
                    provider_setup_index,
                    subscription_auth_state,
                    subscription_install_active,
                    subscription_install_progress,
                    subscription_install_error,
                    editable,
                    request_subscription_install,
                    cancel_subscription_install,
                ),
                provider_model_catalog_row(
                    palette,
                    provider_setup_index,
                    provider_setup_model(provider_setup_index, settings),
                    provider_catalog_state,
                    editable,
                    provider_model_changed,
                    refresh_provider_models,
                    cancel_provider_models,
                ),

                StackPanel::new().children((
                settings_provider_keys_section(
                    palette,
                    provider_key_drafts,
                    provider_keys_set,
                    key_busy,
                    editable,
                    key_draft_changed,
                    key_store,
                    key_clear,
                ),
                settings_quick_scan_tasks_section(
                    palette,
                    scan_catalog,
                    settings,
                    editable,
                    toggle_quick_task,
                ),
                    settings_section(palette, "GENERAL"),
                    settings_row(
                    palette,
                    "Theme",
                    None,
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(["System", "Light", "Dark"])
                        .selected_index(Some(theme_index))
                        .is_enabled(editable)
                        .automation_name("Theme")
                        .on_selection_changed(theme_changed),
                    54.0,
                ),
                    settings_row(
                    palette,
                    "Export format",
                    None,
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(["Text", "JSON", "HTML"])
                        .selected_index(export_format_index)
                        .is_enabled(editable)
                        .automation_name("Export format")
                        .on_selection_changed(export_format_changed),
                    54.0,
                ),
                    settings_check_row(
                    palette,
                    "Auto-save scans",
                    None,
                    CheckBox::new()
                        .is_checked(settings.auto_save)
                        .is_enabled(editable)
                        .automation_name("Auto-save scans")
                        .on_is_checked_changed(auto_save_changed),
                    52.0,
                ),
                    settings_check_row(
                    palette,
                    "Desktop notifications",
                    Some("Notify when a scan finishes in the background"),
                    CheckBox::new()
                        .is_checked(settings.show_notifications)
                        .is_enabled(editable)
                        .automation_name("Desktop notifications")
                        .on_is_checked_changed(notifications_changed),
                    62.0,
                ),
                    settings_check_row(
                    palette,
                    "Scan on startup",
                    None,
                    CheckBox::new()
                        .is_checked(settings.scan_on_startup)
                        .is_enabled(editable)
                        .automation_name("Scan on startup")
                        .on_is_checked_changed(scan_on_startup_changed),
                    52.0,
                ),
                    settings_check_row(
                    palette,
                    "Close to tray",
                    Some("Closing the window keeps the app running in the system tray"),
                    CheckBox::new()
                        .is_checked(settings.close_to_tray)
                        .is_enabled(editable)
                        .automation_name("Close to tray")
                        .on_is_checked_changed(close_to_tray_changed),
                    66.0,
                ),
                    settings_row(
                    palette,
                    "Max concurrent tasks",
                    None,
                    NumberBox::new()
                        .width(90.0)
                        .height(32.0)
                        .minimum(1.0)
                        .maximum(f64::from(SETTINGS_MAX_CONCURRENT_TASKS))
                        .value(Some(f64::from(settings.max_concurrent_tasks)))
                        .is_enabled(editable)
                        .automation_name("Max concurrent tasks")
                        .on_value_changed(max_concurrent_tasks_changed)
                        .horizontal_alignment(HorizontalAlignment::Right),
                    54.0,
                    ),
                )),
            )),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_content_bottom(
    palette: Palette,
    theme: WindowTheme,
    settings: &AppSettings,
    provider_setup_partial: bool,
    editable: bool,
    scan_catalog: &[DiagnosticTask],
    toggle_quick_task: Callback<String>,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
    provider_keys_set: [bool; ProviderKeyId::ALL.len()],
    key_busy: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
    theme_changed: Callback<Option<usize>>,
    export_format_changed: Callback<Option<usize>>,
    auto_save_changed: Callback<bool>,
    notifications_changed: Callback<bool>,
    scan_on_startup_changed: Callback<bool>,
    close_to_tray_changed: Callback<bool>,
    max_concurrent_tasks_changed: Callback<Option<f64>>,
    codex_cli_path_changed: Callback<String>,
    codex_model_changed: Callback<Option<usize>>,
) -> View {
    let theme_index = match theme {
        WindowTheme::System => 0,
        WindowTheme::Light => 1,
        WindowTheme::Dark => 2,
    };
    let export_format_index =
        selected_setting_index(&settings.export_format, &["text", "json", "html"]);
    let (codex_model_items, codex_model_index) = codex_model_options(settings);
    let top_tail = Border::new()
        .height(55.0)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            TextBox::new()
                .width(260.0)
                .height(32.0)
                .text(settings.codex_cli_path.clone().unwrap_or_default())
                .placeholder_text("Auto-detected")
                .is_enabled(editable)
                .on_text_changed(codex_cli_path_changed)
                .horizontal_alignment(HorizontalAlignment::Right)
                .vertical_alignment(VerticalAlignment::Top)
                .margin(Thickness::new(0.0, 10.0, 0.0, 0.0)),
        );
    let model = Border::new()
        .height(167.0)
        .padding(Thickness::new(0.0, 13.0, 0.0, 0.0))
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            StackPanel::new().children((
                StackPanel::new().spacing(2.0).children((
                    TextBlock::new()
                        .text("Model")
                        .font_size(13.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                    TextBlock::new()
                        .text("Optional. Empty uses the CLI’s default model")
                        .font_size(11.0)
                        .foreground(palette.muted),
                )),
                StackPanel::new()
                    .width(356.0)
                    .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
                    .horizontal_alignment(HorizontalAlignment::Right)
                    .children((
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(8.0)
                            .children((
                                ComboBox::new()
                                    .width(232.0)
                                    .height(32.0)
                                    .items_source(codex_model_items)
                                    .selected_index(codex_model_index)
                                    .is_enabled(editable)
                                    .automation_name("Codex model")
                                    .on_selection_changed(codex_model_changed),
                                Button::new()
                                    .width(112.0)
                                    .height(32.0)
                                    .is_enabled(!provider_setup_partial)
                                    .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
                            )),
                        Border::new()
                            .width(356.0)
                            .height(44.0)
                            .margin(Thickness::new(0.0, 4.0, 0.0, 0.0))
                            .padding(Thickness::xy(8.0, 0.0))
                            .background(Color::argb(190, 59, 70, 82))
                            .corner_radius(5.0)
                            .horizontal_alignment(HorizontalAlignment::Left)
                            .content(
                                Grid::new()
                                    .columns([GridLength::Star(1.0), GridLength::Auto])
                                    .children((
                                        StackPanel::new().spacing(1.0).children((
                                            TextBlock::new()
                                                .text("GPT-5.6-Luna")
                                                .font_size(11.0)
                                                .font_weight(FontWeight::SEMI_BOLD),
                                            TextBlock::new()
                                                .text("Fast and affordable agentic coding model.")
                                                .font_size(10.0),
                                        )),
                                        TextBlock::new()
                                            .text("gpt-5.6-luna")
                                            .grid_column(1)
                                            .font_size(10.5)
                                            .font_weight(FontWeight::SEMI_BOLD)
                                            .vertical_alignment(VerticalAlignment::Top),
                                    )),
                            ),
                        TextBlock::new()
                            .text("Provider default: gpt-5.6-sol")
                            .margin(Thickness::new(0.0, 4.0, 0.0, 0.0))
                            .font_size(10.5)
                            .foreground(palette.muted)
                            .horizontal_alignment(HorizontalAlignment::Right),
                    )),
            )),
        );
    let description_and_general = StackPanel::new().children((
        Border::new()
            .height(53.0)
            .padding(Thickness::new(0.0, 4.0, 0.0, 4.0))
            .content(
                TextBlock::new()
                    .text("Runs through OpenAI's Codex CLI with your ChatGPT plan — no API key, and this app never stores an OpenAI token.")
                    .font_size(13.0)
                    .text_wrapping(TextWrapping::Wrap),
            ),
        settings_provider_keys_section(
            palette,
            provider_key_drafts,
            provider_keys_set,
            key_busy,
            editable,
            key_draft_changed,
            key_store,
            key_clear,
        ),
        settings_quick_scan_tasks_section(
            palette,
            scan_catalog,
            settings,
            editable,
            toggle_quick_task,
        ),
        settings_section(palette, "GENERAL"),
    ));

    ScrollViewer::new()
        .width(593.0)
        .height(690.0)
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Visible)
        .content(
            StackPanel::new().children((
                top_tail,
                model,
                description_and_general,
                settings_row(
                    palette,
                    "Theme",
                    None,
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(["System", "Light", "Dark"])
                        .selected_index(Some(theme_index))
                        .is_enabled(editable)
                        .automation_name("Theme")
                        .on_selection_changed(theme_changed),
                    53.0,
                ),
                settings_row(
                    palette,
                    "Export format",
                    None,
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(["Text", "JSON", "HTML"])
                        .selected_index(export_format_index)
                        .is_enabled(editable)
                        .automation_name("Export format")
                        .on_selection_changed(export_format_changed),
                    53.0,
                ),
                settings_check_row(
                    palette,
                    "Auto-save scans",
                    None,
                    CheckBox::new()
                        .is_checked(settings.auto_save)
                        .is_enabled(editable)
                        .automation_name("Auto-save scans")
                        .on_is_checked_changed(auto_save_changed),
                    43.0,
                ),
                settings_check_row(
                    palette,
                    "Desktop notifications",
                    Some("Notify when a scan finishes in the background"),
                    CheckBox::new()
                        .is_checked(settings.show_notifications)
                        .is_enabled(editable)
                        .automation_name("Desktop notifications")
                        .on_is_checked_changed(notifications_changed),
                    60.0,
                ),
                settings_check_row(
                    palette,
                    "Scan on startup",
                    None,
                    CheckBox::new()
                        .is_checked(settings.scan_on_startup)
                        .is_enabled(editable)
                        .automation_name("Scan on startup")
                        .on_is_checked_changed(scan_on_startup_changed),
                    42.0,
                ),
                settings_check_row(
                    palette,
                    "Close to tray",
                    Some("Closing the window keeps the app running in the system tray"),
                    CheckBox::new()
                        .is_checked(settings.close_to_tray)
                        .is_enabled(editable)
                        .automation_name("Close to tray")
                        .on_is_checked_changed(close_to_tray_changed),
                    59.0,
                ),
                settings_row(
                    palette,
                    "Max concurrent tasks",
                    None,
                    NumberBox::new()
                        .width(90.0)
                        .height(32.0)
                        .minimum(1.0)
                        .maximum(f64::from(SETTINGS_MAX_CONCURRENT_TASKS))
                        .value(Some(f64::from(settings.max_concurrent_tasks)))
                        .is_enabled(editable)
                        .automation_name("Max concurrent tasks")
                        .on_value_changed(max_concurrent_tasks_changed)
                        .horizontal_alignment(HorizontalAlignment::Right),
                    67.0,
                ),
            )),
        )
}

pub(crate) fn settings_section(palette: Palette, label: &'static str) -> View {
    Border::new()
        .height(if label == "AI ASSISTANT" { 37.0 } else { 38.0 })
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            TextBlock::new()
                .text(label)
                .font_size(10.5)
                .font_weight(FontWeight::SEMI_BOLD)
                .foreground(palette.muted)
                .vertical_alignment(VerticalAlignment::Bottom)
                .margin(Thickness::new(0.0, 0.0, 0.0, 7.0)),
        )
}

pub(crate) fn settings_row(
    palette: Palette,
    label: &'static str,
    hint: Option<&'static str>,
    control: impl Into<View>,
    height: f64,
) -> View {
    let label = settings_label(palette, label, hint);

    Border::new()
        .height(height)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Pixel(260.0)])
                .column_spacing(16.0)
                .children((
                    Border::new()
                        .vertical_alignment(VerticalAlignment::Center)
                        .content(label),
                    Border::new()
                        .grid_column(1)
                        .vertical_alignment(VerticalAlignment::Center)
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .content(control),
                )),
        )
}

pub(crate) fn settings_label(
    palette: Palette,
    label: &'static str,
    hint: Option<&'static str>,
) -> View {
    if let Some(hint) = hint {
        StackPanel::new().spacing(2.0).children((
            TextBlock::new()
                .text(label)
                .font_size(14.0)
                .font_weight(FontWeight::SEMI_BOLD),
            TextBlock::new()
                .text(hint)
                .font_size(11.5)
                .foreground(palette.muted)
                .text_wrapping(TextWrapping::Wrap),
        ))
    } else {
        TextBlock::new()
            .text(label)
            .font_size(14.0)
            .font_weight(FontWeight::SEMI_BOLD)
            .vertical_alignment(VerticalAlignment::Center)
            .into()
    }
}

pub(crate) fn settings_wrapped_row(
    palette: Palette,
    label: &'static str,
    hint: Option<&'static str>,
    control: impl Into<View>,
    height: f64,
) -> View {
    Border::new()
        .height(height)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .rows([GridLength::Star(1.0), GridLength::Auto])
                .children((
                    Border::new()
                        .vertical_alignment(VerticalAlignment::Center)
                        .content(settings_label(palette, label, hint)),
                    Border::new()
                        .grid_row(1)
                        .margin(Thickness::new(0.0, 0.0, 0.0, 9.0))
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .content(control),
                )),
        )
}

pub(crate) fn settings_check_row(
    palette: Palette,
    label: &'static str,
    hint: Option<&'static str>,
    checkbox: CheckBox,
    height: f64,
) -> View {
    Border::new()
        .height(height)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Pixel(24.0)])
                .children((
                    Border::new()
                        .vertical_alignment(VerticalAlignment::Center)
                        .content(settings_label(palette, label, hint)),
                    Border::new()
                        .grid_column(1)
                        .width(24.0)
                        .height(32.0)
                        .margin(Thickness::new(0.0, 0.0, 4.0, 0.0))
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .vertical_alignment(VerticalAlignment::Center)
                        .content(checkbox.width(14.0).height(14.0)),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_rows_cover_the_closed_provider_set_including_deepseek() {
        assert_eq!(PROVIDER_KEY_LABELS.len(), ProviderKeyId::ALL.len());
        assert_eq!(ProviderKeyId::ALL[3], ProviderKeyId::DeepSeek);
        assert_eq!(PROVIDER_KEY_LABELS[3], "DeepSeek");
    }
}
