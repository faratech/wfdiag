//! The root component: one dispatcher, one engine, one snapshot.
//!
//! # One engine, one snapshot
//!
//! This shell owns exactly one [`AppService`]. Every diagnostic scan, issue
//! projection, history query, provider probe, settings write, export render,
//! chat turn, report, analysis, fix plan, remediation, model catalog and
//! subscription operation is an [`AppCommand`] dispatched into it, and every
//! answer arrives as an [`AppEvent`] drained from it. The seventeen worker
//! runtimes, their receivers, their wait tasks, their request-id counters and
//! their per-domain staleness guards are gone: the facade owns all of them and
//! guarantees that an event a host receives is already current.
//!
//! # One screen, one struct
//!
//! [`WfdiagShell`] holds no page's state. Each screen and each dialog owns a
//! struct of its own, its own message enum, and its own `update` /
//! `on_app_event` / `view`; this component is the dispatcher that hands one
//! message to the owner named by its [`Message`] variant and then performs the
//! [`crate::app::screen::Effect`]s that owner asked for. Screens read the
//! chrome through [`crate::app::screen::ScreenCx`] and
//! [`crate::app::screen::ShellEnv`] and can never write it, which is what
//! keeps `update` a `match` and `view` chrome plus one `match page`.
//!
//! Everything a screen renders is refreshed from [`AppSnapshot`] after every
//! drain (`sync_from_snapshot`); its `on_app_event` only adds the status text
//! and the UI reactions (focus, notifications, navigation) that a read model
//! cannot express.

#![deny(unsafe_code)]

pub(crate) mod bootstrap;
pub(crate) mod chrome;
pub(crate) mod consts;
pub(crate) mod message;
pub(crate) mod native_msg;
pub(crate) mod native_route;
pub(crate) mod orchestration;
pub(crate) mod policy;
pub(crate) mod screen;
pub(crate) mod shell;
pub(crate) mod shell_msg;
pub(crate) mod shell_route;
pub(crate) mod state;
pub(crate) mod tasks;

use crate::app::bootstrap::{EngineBoot, start_application_service};
use crate::app::consts::WINDOW_COMMAND_POLL;
use crate::app::message::Message;
use crate::app::policy::{
    configured_provider_setup_index, diagnostics_uses_compact_layout, effective_window_theme,
    load_live_test_settings, navigation_rail_forced_collapsed, page_host_scrolls,
    pending_system_info, window_theme_from_setting,
};
use crate::app::screen::{ScanEnv, ShellEnv};
use crate::app::shell::ShellState;
use crate::app::shell_msg::ShellMsg;
use crate::app::state::{AiMode, Page};
use crate::app::tasks::spawn_instance_watch;
use crate::dialogs::about::state::AboutDialog;
use crate::dialogs::action_review::state::ActionReviewDialog;
use crate::dialogs::export::state::ExportState;
use crate::dialogs::palette::state::PaletteDialog;
use crate::dialogs::settings::state::SettingsDialog;
use crate::dialogs::shortcuts_help::state::ShortcutHelpDialog;
use crate::dialogs::update_notice::state::UpdateNoticeDialog;
use crate::fixtures;
use crate::fixtures::knobs::{
    fixture_mode, initial_page_override, initial_window_height, initial_window_width,
    live_test_fixture_from_env, settings_dialog_open_override, startup_theme_setting, visual_state,
};
use crate::fixtures::visual::{
    LiveTestFixture, VisualState, fixture_258_system_info, remediation_partial_visual_run,
};
use crate::platform::{instance, notifications, ui_wake, window};
use crate::screens::ai::state::AiScreen;
use crate::screens::diagnostics::state::DiagnosticsScreen;
use crate::screens::history::state::HistoryScreen;
use crate::screens::issues::state::IssuesScreen;
use crate::screens::monitor::state::MonitorScreen;
use crate::screens::processes::state::ProcessesScreen;
use crate::widgets::palette_colors::Palette;
use std::sync::Arc;
use wfdiag_app::{AppCommand, AppEventReceiver, AppService, UiWakeHandler};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_diagnostics::{DiagnosticOutput, ScanKind};
use wfdiag_native_settings::AppSettings;
use wfdiag_ui_core::DiagnosticTaskResult;
use windows_reactor::*;

/// Everything the shell renders, plus the one engine handle it drives.
// Independent presentational facts, each read by a different surface. There is
// no state machine here to merge them into: the state machines all moved into
// `wfdiag-app`.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct WfdiagShell {
    /// The one application service. `None` only in the deterministic visual
    /// and fixture modes, which must never start a worker or touch the system.
    pub(crate) app: Option<AppService>,
    /// The engine's event stream, drained on every coalesced wake.
    pub(crate) app_events: Option<AppEventReceiver>,

    /// The chrome's own view state: page, theme, window, host identity and
    /// the status line. Screens read it and never write it.
    pub(crate) shell: ShellState,

    // ---- About and the update notice ----------------------------------
    pub(crate) about: AboutDialog,
    pub(crate) update_notice: UpdateNoticeDialog,

    // ---- Settings dialog ----------------------------------------------
    pub(crate) settings: SettingsDialog,

    // ---- diagnostics ---------------------------------------------------
    pub(crate) diagnostics: DiagnosticsScreen,

    // ---- issues and remediation ----------------------------------------
    pub(crate) issues: IssuesScreen,
    pub(crate) action_review: ActionReviewDialog,

    // ---- live monitoring and processes ---------------------------------
    pub(crate) monitor: MonitorScreen,
    pub(crate) processes: ProcessesScreen,

    // ---- scan history ---------------------------------------------------
    pub(crate) history: HistoryScreen,

    // ---- AI ------------------------------------------------------------
    pub(crate) ai: AiScreen,

    // ---- export ---------------------------------------------------------
    pub(crate) export: ExportState,

    // ---- palette, shortcuts, window integration --------------------------
    pub(crate) palette: PaletteDialog,
    pub(crate) shortcuts: ShortcutHelpDialog,
}

impl Component for WfdiagShell {
    type Message = Message;
    type Input = ();

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        // Producers wake the native window, whose UI-thread callback enqueues
        // one lightweight drain message. The process hook is single-assignment
        // while the local sender may be replaced if Reactor remounts a scope.
        let _ = ui_wake::install(|| {
            let _ = window::post_ui_wake();
        });
        let ui_sender = context.sender();
        window::set_ui_wake_handler(move || {
            let _ = ui_sender.send(Message::NativeSignalReady);
        });

        // Every knob below resolves to its production default with no
        // environment access unless the `validation` feature is on (#186).
        let visual_state = visual_state();
        let live_test_fixture = live_test_fixture_from_env();
        let (default_width, default_height) = visual_state.default_size();
        let width = initial_window_width(default_width);
        let height = initial_window_height(default_height);
        let initial_page = initial_page_override().unwrap_or_else(|| visual_state.default_page());
        let fixture_mode = fixture_mode();
        let diagnostic_results = if visual_state.has_scan()
            || live_test_fixture.is_some_and(LiveTestFixture::injects_scan)
            || (fixture_mode && !initial_page.consumes_live_telemetry())
        {
            vec![DiagnosticTaskResult::new(
                "visual-fixture",
                "computer_system",
                Arc::new(DiagnosticOutput {
                    success: true,
                    output: "Visual fixture; real results arrive through UiEvent::DiagnosticResult"
                        .to_string(),
                    error: None,
                    duration_ms: 29,
                }),
            )]
        } else {
            Vec::new()
        };
        let mut status = if diagnostic_results.is_empty() {
            "Ready — no scan data".to_string()
        } else {
            "17 collected · 0 errors".to_string()
        };
        let deterministic_visual =
            fixture_mode || visual_state != VisualState::Live || live_test_fixture.is_some();

        // One engine, started once. The fixture modes deliberately keep
        // `app: None`, which is what makes a screenshot capture incapable of
        // touching WMI, the registry, the network, or the user's settings.
        let (app, app_events) = if deterministic_visual {
            (None, None)
        } else {
            match start_application_service(true) {
                Ok((service, events)) => {
                    events.set_wake_handler(UiWakeHandler::new(ui_wake::notify));
                    (Some(service), Some(events))
                }
                Err(error) => {
                    status = format!("The diagnostic engine could not start · {error}");
                    (None, None)
                }
            }
        };

        // #200: the persisted theme is loaded synchronously inside
        // `AppService::start`, so the very first frame paints in the user's
        // theme instead of flashing the hard-coded Dark default.
        let engine = EngineBoot::capture(app.as_ref());
        let mut settings_defaults = engine.settings.clone();
        if deterministic_visual {
            // Preserve the Store 2.5.8 screenshot fixtures. These two visible
            // controls intentionally differ from the shipping persistence
            // defaults and must never leak into a live settings file.
            settings_defaults = AppSettings::default();
            settings_defaults.network_grounding_enabled = true;
            settings_defaults.codex_model = Some("gpt-5.6-luna".to_string());
        }
        let export_fixture = live_test_fixture == Some(LiveTestFixture::ExportFallback);
        if export_fixture {
            match load_live_test_settings() {
                Ok(settings) => settings_defaults = settings,
                Err(error) => {
                    status = format!("Validation fixture settings unavailable · {error}");
                }
            }
        }
        let settings_error = engine.settings_error.clone();
        if let Some(error) = settings_error.as_deref() {
            status = format!("Settings could not be loaded · {error}");
        }

        let diagnostic_catalog = engine.catalog.clone();
        let issue_maintenance = engine.maintenance.clone();
        let has_fixture_scan = !diagnostic_results.is_empty();
        let issues = if deterministic_visual && has_fixture_scan {
            fixtures::fixture_258_issues()
        } else {
            engine.issues.clone()
        };

        // Remediation previews and run history survive a process restart; the
        // engine rehydrates them at start, so adopt them for the first frame.
        let (action_active_run, action_run_history, action_review) =
            if visual_state == VisualState::RemediationPartial {
                (None, vec![remediation_partial_visual_run()], None)
            } else {
                (
                    engine.active_run.clone(),
                    engine.run_history.clone(),
                    engine.review.clone(),
                )
            };
        let action_expanded_runs = action_active_run
            .iter()
            .chain(
                action_run_history
                    .iter()
                    .filter(|_| visual_state == VisualState::RemediationPartial),
            )
            .map(|run| run.run_id.clone())
            .collect();

        let initial_system_info = if deterministic_visual {
            fixture_258_system_info()
        } else {
            engine
                .system_info
                .clone()
                .unwrap_or_else(pending_system_info)
        };
        let is_admin = initial_system_info.is_admin;

        let window_lifecycle_revision = window::lifecycle_snapshot().revision;
        let instance_wait = if deterministic_visual || instance::activation_wake_registered() {
            None
        } else {
            Some(spawn_instance_watch(context, window_lifecycle_revision))
        };
        // #207: a live watch here means the kernel wait registration was
        // refused and tray/activation delivery is polling instead. Say so from
        // the first frame rather than only after the first re-arm; the same
        // latch keeps `arm_instance_watch` from repeating it.
        let degraded_instance_watch = instance_wait.is_some();
        if degraded_instance_watch {
            status = format!(
                "Tray and single-instance events are polling every {} ms · Windows refused the \
                 event-driven registration",
                WINDOW_COMMAND_POLL.as_millis()
            );
        }

        let settings_open =
            visual_state == VisualState::SettingsBottom || settings_dialog_open_override();
        // Validation knob: WFDIAG_REACTOR_THEME=light|dark|system selects
        // the startup theme. Without the `validation` feature this is always
        // the empty override, which falls through to the persisted setting
        // the engine already loaded (#186, #200).
        let theme_override = startup_theme_setting();
        let initial_theme = if theme_override.is_empty() {
            window_theme_from_setting(&settings_defaults.theme)
        } else {
            window_theme_from_setting(&theme_override)
        };
        let initial_color_scheme = if initial_theme == WindowTheme::Light {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        };
        let initial_provider_setup_index = configured_provider_setup_index(&settings_defaults);

        let mut component = Self {
            app,
            app_events,
            shell: ShellState {
                page: initial_page,
                live_test_fixture,
                theme: initial_theme,
                effective_color_scheme: initial_color_scheme,
                window_size: WindowSize { width, height },
                requested_client_width: width,
                requested_client_height: height,
                pane_open: !settings_defaults.nav_rail_collapsed,
                deterministic_visual,
                visual_state,
                status,
                notification_failure_reported: false,
                degraded_instance_watch_reported: degraded_instance_watch,
                settings: settings_defaults.clone(),
                system_info: initial_system_info,
                architecture: engine.architecture.clone(),
                system_error: engine.system_error.clone(),
                is_admin,
                window_hook_installed: false,
                window_hook_retry_failures: 0,
                window_hook_retry_task: None,
                window_lifecycle_revision,
                window_usable: true,
                instance_wait,
            },
            about: AboutDialog::default(),
            update_notice: UpdateNoticeDialog::default(),
            settings: {
                let mut dialog = SettingsDialog::new(
                    settings_defaults,
                    settings_open,
                    initial_provider_setup_index,
                );
                dialog.error = settings_error;
                dialog
            },
            diagnostics: DiagnosticsScreen {
                results: diagnostic_results,
                catalog: diagnostic_catalog,
                scan_kind: has_fixture_scan.then_some(ScanKind::Quick),
                duration_ms: if has_fixture_scan { 2_300 } else { 0 },
                total: if has_fixture_scan { 17 } else { 0 },
                completed: if has_fixture_scan { 17 } else { 0 },
                ..DiagnosticsScreen::default()
            },
            issues: IssuesScreen {
                issues,
                maintenance: issue_maintenance,
                projected_session_id: has_fixture_scan
                    .then(|| "visual-fixture".to_string())
                    .or_else(|| engine.session_id.clone()),
                active_run: action_active_run,
                run_history: action_run_history,
                expanded_runs: action_expanded_runs,
                ..IssuesScreen::default()
            },
            action_review: ActionReviewDialog {
                review: action_review,
                repair_confirm: None,
            },
            monitor: MonitorScreen::new(visual_state, fixture_mode),
            processes: ProcessesScreen::default(),
            history: HistoryScreen::default(),
            ai: AiScreen::default(),
            export: ExportState::default(),
            palette: PaletteDialog::default(),
            shortcuts: ShortcutHelpDialog::default(),
        };

        // The startup-scan gate now lives in the engine: `Start` arms it, and
        // the scan runs once settings and host identity have both settled.
        component.dispatch(AppCommand::Start {
            startup_scan: component.shell.settings.scan_on_startup,
        });
        window::set_close_to_tray(component.shell.settings.close_to_tray);
        // Keep the collector warm but idle until a live surface consumes it.
        if !initial_page.consumes_live_telemetry() {
            component.dispatch(AppCommand::SetMonitorPaused { paused: true });
            component.monitor.paused = true;
        }
        if initial_page == Page::Processes && !deterministic_visual {
            component.request_process_page(context, false);
        }
        if initial_page == Page::History && !deterministic_visual {
            component.request_history_list(context);
        }
        component
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        self.ensure_window_hook(context);
        match message {
            Message::NativeSignalReady => {
                // #206: the toast worker posts a wake as soon as it records a
                // failure; the atomic guard inside makes this a no-op read on
                // every ordinary wake.
                if let Some(error) = notifications::take_toast_failure() {
                    self.report_notification_failure(error);
                }
                for pending in self.drain_native_messages() {
                    self.update(pending, context);
                }
            }
            Message::App(events) => self.apply_app_events(events, context),
            Message::WindowHookBootstrap => {}
            Message::WindowSize(size) => self.shell.window_size = size,
            Message::ColorSchemeChanged(color_scheme) => {
                self.shell.effective_color_scheme = color_scheme;
            }
            Message::Shell(message) => self.route_shell(message, context),
            Message::Native(message) => self.route_native(message, context),
            Message::Palette(message) => self.route_palette(message, context),
            Message::Shortcuts(message) => self.route_shortcuts(message),
            Message::Settings(message) => self.route_settings(message, context),
            Message::About(message) => self.route_about(message, context),
            Message::UpdateNotice(message) => self.route_update_notice(message, context),
            Message::Export(message) => self.route_export(message),
            Message::Ai(message) => self.route_ai(message, context),
            Message::Diagnostics(message) => self.route_diagnostics(message, context),
            Message::Monitor(message) => self.route_monitor(message, context),
            Message::Processes(message) => self.route_processes(message, context),
            Message::History(message) => self.route_history(message, context),
            Message::Issues(message) => self.route_issues(message, context),
            Message::ActionReview(message) => self.route_action_review(message, context),
        }
        // Every arm above may have dispatched. `AppService::dispatch` updates
        // the read model synchronously (a started scan is `Starting` the
        // instant it is accepted), so the frame this message produces must be
        // rendered from the snapshot as it is now, not as it was last wake.
        self.sync_from_snapshot();
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title(format!(
            "WindowsForum Diagnostics — {}",
            self.shell.page.nav_label()
        ));
        context.window_visuals(
            WindowVisuals::new()
                .theme(self.shell.theme)
                .backdrop(WindowBackdrop::Acrylic)
                .client_size(
                    self.shell.requested_client_width,
                    self.shell.requested_client_height,
                )
                .constraints(WindowConstraints {
                    min_width: Some(720.0),
                    min_height: Some(540.0),
                    max_width: None,
                    max_height: None,
                }),
        );
        context.on_color_scheme(context.callback(Message::ColorSchemeChanged));
        context.on_window_size(context.callback(Message::WindowSize));

        // Effects run after windows-reactor publishes this view's native
        // CreateWindow commands. Use that ordering to bootstrap the Win32
        // bridge exactly once on the healthy path; bounded background retry
        // is reserved for a genuinely slow or rejected hook installation.
        let window_hook_bootstrap = !self.shell.deterministic_visual
            && !self.shell.window_hook_installed
            && self.shell.window_hook_retry_task.is_none();
        let window_hook_sender = context.sender();
        context.use_effect(
            "native-window-hook-bootstrap",
            window_hook_bootstrap,
            move || {
                if window_hook_bootstrap {
                    let _ = window_hook_sender.send(Message::WindowHookBootstrap);
                }
                None
            },
        );

        let effective_theme =
            effective_window_theme(self.shell.theme, self.shell.effective_color_scheme);
        let palette = Palette::for_theme(effective_theme);
        let narrow = self.shell.window_size.width < 940.0;
        let diagnostics_compact = diagnostics_uses_compact_layout(self.shell.window_size.width);
        let rail_forced_collapsed = navigation_rail_forced_collapsed(self.shell.window_size.width);
        // Keep content-specific compact layouts independent from the shipping
        // shell's 1100 px forced rail collapse.
        // Keep the user's expanded preference so the full pane returns when the
        // window grows again, but never let it consume the compact content area.
        let pane_expanded = self.shell.pane_open && !rail_forced_collapsed;
        // One read-only bundle of chrome facts, handed to whichever page is
        // open. A screen never reaches back into the shell for these.
        let env = ShellEnv {
            palette,
            theme: effective_theme,
            narrow,
            compact: diagnostics_compact,
            pane_expanded,
            window_size: self.shell.window_size,
            deterministic_visual: self.shell.deterministic_visual,
            visual_state: self.shell.visual_state,
            is_admin: self.shell.is_admin,
            monitoring_paused: self.monitor.paused,
            settings: &self.shell.settings,
            scan: ScanEnv {
                busy: self.diagnostics.busy(),
                cancelling: self.diagnostics.cancelling(),
                completed: self.diagnostics.completed,
                total: self.diagnostics.total,
                current_task: self.diagnostics.current_task.as_deref(),
                has_results: !self.diagnostics.results.is_empty(),
                session_id: self
                    .diagnostics
                    .results
                    .first()
                    .map(|result| result.session_id.as_str()),
            },
        };
        // The engine only publishes issues for evidence it has committed, so
        // "current" is simply: the projection names the visible scan.
        let issue_projection_current = self.issues.projected_session_id.as_deref()
            == self
                .diagnostics
                .results
                .first()
                .map(|result| result.session_id.as_str());
        let diagnostic_ai_available = self.shell.settings.ai_enabled
            && !self.shell.deterministic_visual
            && self
                .ai
                .provider_status
                .as_ref()
                .is_some_and(|status| status.active_provider != AIProvider::None);
        let chat_composer_reference = self.ai.composer_reference.clone();
        let focus_chat_composer = self.shell.page == Page::Ai
            && self.ai.mode == AiMode::Assistant
            && self.ai.focus_revision > 0;
        context.use_effect(
            "chat-composer-focus",
            (focus_chat_composer, self.ai.focus_revision),
            move || {
                if focus_chat_composer {
                    let _ = chat_composer_reference.request_focus();
                }
                None
            },
        );
        let page = match self.shell.page {
            Page::Diagnostics => self
                .diagnostics
                .view(&env, diagnostic_ai_available, context),
            Page::Monitor => self.monitor.view(&env, context),
            Page::Processes => self.processes.view(&env, context),
            Page::Ai => self.ai.view(&env, context),
            Page::Issues => {
                let competing_action_busy = self.issues.busy
                    || self.action_review.open()
                    || self.issues.active_run.is_some();
                self.issues.view(&env, competing_action_busy, context)
            }
            Page::History => self.history.view(&env, context),
        };

        let status_bar = self.status_bar(&env);

        // #193: the page host owns the ONE ScrollViewer. Pages return plain
        // content and never nest a viewer of their own — a nested viewer is
        // measured with unbounded height by this one, so it never scrolls,
        // and the content past the fold becomes unreachable by pointer,
        // keyboard and UI Automation alike. The Diagnostics page keeps its
        // own two arms: at full width it lays itself out inside the viewport
        // and must not scroll at all.
        let page_body = Border::new()
            .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
            .content(page);
        let page_host: View = if page_host_scrolls(self.shell.page, diagnostics_compact) {
            ScrollViewer::new()
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .content(page_body)
        } else {
            page_body
        };

        let content_panel = Border::new()
            .grid_column(1)
            .margin(Thickness::new(-2.0, 2.0, 20.0, 14.0))
            .background(palette.panel)
            .border_brush(palette.border)
            .border_thickness(1.0)
            .corner_radius(10.0)
            .content(
                Grid::new()
                    .rows([GridLength::Star(1.0), GridLength::Auto])
                    .children((page_host, status_bar)),
            );

        let navigation_rail = self.navigation_rail(
            &env,
            issue_projection_current,
            rail_forced_collapsed,
            context,
        );
        // The shell publishes frequently while telemetry and scans are live.
        // A closed palette must stay a zero-cost overlay: do not allocate its
        // specs, fuzzy-match them, or construct row controls until it opens.
        let palette_dialog = self
            .palette
            .view(&env, self.palette_command_specs(), context);
        let shortcut_dialog = self.shortcuts.view(&env, context);
        let (title_brand, title_bar, title_actions) = self.title_bar(&env, context);
        let body = Grid::new()
            .grid_row(1)
            .columns([
                GridLength::Pixel(if pane_expanded { 230.0 } else { 64.0 }),
                GridLength::Star(1.0),
            ])
            .children((navigation_rail, content_panel));

        let (light_wallpaper, dark_wallpaper) = Self::wallpapers(&env);
        let (settings, subscription_install_dialog) = self.settings.view(
            &env,
            self.ai.provider_status.as_ref(),
            self.ai.status_loading,
            self.app.is_some(),
            &self.diagnostics.catalog,
            context,
        );
        let update_notice = self.update_notice.view(context);
        let (about_scrim, about) =
            self.about
                .overlay(&env, self.update_notice.info.as_ref(), context);
        let (action_review_dialog, repair_dialog) = self.action_review.overlay(&env, context);
        Grid::new()
            .rows([GridLength::Pixel(42.0), GridLength::Star(1.0)])
            // Keep Reactor's native accelerators for Ctrl+R and the numpad
            // aliases. The isolated window subclass supplies the shipping
            // main-row/K/slash/Shift chords through the component watcher.
            .key_accelerators(KeyAccelerators::new([
                KeyAccelerator::new(
                    AcceleratorKey::R,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Refresh)),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad1,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some(
                        "diagnostics".to_string(),
                    )))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad2,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some(
                        "monitor".to_string(),
                    )))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad3,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some(
                        "processes".to_string(),
                    )))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad4,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some("ai".to_string())))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad5,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some(
                        "issues".to_string(),
                    )))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad6,
                    AcceleratorModifiers::Control,
                    context.message(Message::Shell(ShellMsg::Navigate(Some(
                        "history".to_string(),
                    )))),
                ),
            ]))
            .children((
                light_wallpaper,
                dark_wallpaper,
                Border::new().grid_row_span(2).background(palette.dim),
                title_brand,
                title_bar,
                title_actions,
                body,
                update_notice,
                settings,
                about_scrim,
                about,
                // One permanent overlay host keeps the root Grid topology
                // stable. Dialog nodes remain mounted and toggle native open
                // state rather than being inserted into the live tree.
                Grid::new().grid_row_span(2).children((
                    palette_dialog,
                    shortcut_dialog,
                    action_review_dialog,
                    repair_dialog,
                    subscription_install_dialog,
                )),
            ))
    }
}
