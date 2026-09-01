//! The root component: its view state and the Reactor `Component` impl.
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
//! What is left here is genuinely presentational: which page is open, which
//! overlay is up, what the user is typing, and the status line. Those fields
//! are refreshed from [`AppSnapshot`] after every drain
//! (`sync_from_snapshot`); the per-domain `apply_*_event` handlers only add
//! the status text and the UI reactions (focus, notifications, navigation)
//! that a read model cannot express.

#![deny(unsafe_code)]

pub(crate) mod consts;
pub(crate) mod message;
pub(crate) mod orchestration;
pub(crate) mod policy;
pub(crate) mod state;
pub(crate) mod tasks;

use crate::app::consts::{
    APP_BADGE, APP_VERSION, PROVIDER_SETUP_LABELS, STATUS_INFO_DARK, STATUS_INFO_LIGHT,
    STATUS_OK_DARK, STATUS_OK_LIGHT, STATUS_WARN_DARK, STATUS_WARN_LIGHT, WALLPAPER_DARK,
    WALLPAPER_LIGHT, WINDOW_COMMAND_POLL,
};
use crate::app::message::{Message, PaletteFocusAction, SettingsDialogAction};
use crate::app::policy::{
    configured_provider_setup_index, diagnostics_uses_compact_layout, effective_window_theme,
    load_live_test_settings, navigation_rail_forced_collapsed, pending_system_info,
    phi_preference_gate, privilege_label, reactor_provider_backend,
    subscription_auth_provider_for_setup, subscription_auth_state_index,
    update_notice_timer_callback_is_current, window_theme_from_setting,
};
use crate::app::state::{
    AiMode, AiPreparationUi, ChatDisplayMessage, CloudFallbackConsent, DiagnosticAnalysisDisplay,
    FullScanConsent, HistoryTaskDiffProjection, IssuePrioritizationDisplay, MonitorHistory, Page,
    PendingExportAction,
};
use crate::app::tasks::spawn_instance_watch;
use crate::dialogs::about::about_dialog;
use crate::dialogs::action_review::action_review_presentation;
use crate::dialogs::palette::{
    PALETTE_MAX_RESULTS, command_palette_footer, command_palette_highlighted_label,
    command_palette_key_chip, palette_visible_matches,
};
use crate::dialogs::settings::settings_dialog;
use crate::fixtures;
use crate::fixtures::knobs::{
    fixture_mode, initial_page_override, initial_window_height, initial_window_width,
    live_test_fixture_from_env, settings_dialog_open_override, startup_theme_setting, visual_state,
};
use crate::fixtures::visual::{
    LiveTestFixture, VisualState, fixture_258_system_info, fixture_monitor_empty_stats,
    fixture_system_stats, remediation_partial_visual_run,
};
use crate::platform::external::write_text_to_clipboard;
use crate::platform::{focus, instance, notifications, ui_wake, window};
use crate::screens::ai::view::ai_page;
use crate::screens::diagnostics::view::{
    diagnostic_matches_filter, diagnostics_page, format_diagnostic_duration,
};
use crate::screens::history::view::history_page;
use crate::screens::issues::view::issues_page;
use crate::screens::monitor::view::monitor_page;
use crate::screens::processes::view::processes_page;
use crate::widgets::chrome::{fa_icon_label, machine_card, nav_brand, nav_button, nav_section};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wfdiag_app::domain::ai_intent::PendingAiIntent;
use wfdiag_app::domain::catalog::CatalogState;
use wfdiag_app::domain::scan::ScanPhase;
use wfdiag_app::domain::subscriptions::{AccountState, InstallPrompt};
use wfdiag_app::ports::monitor::{
    NetworkConnection, ProcessPage, ProcessSortDirection, ProcessSortKey,
};
use wfdiag_app::ports::native::{WindowsPortOverrides, windows_ports_with};
use wfdiag_app::{
    AppCommand, AppConfig, AppEventReceiver, AppService, DispatchOutcome, ElevationPort,
    UiWakeHandler,
};
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallFallbackReason, SubscriptionInstallProgress,
};
use wfdiag_native_ai_chat::{ProviderUse, SubscriptionAuthProvider};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderStatus, PackageIdentitySource, ProviderPreferenceSettingsValidator,
    SharedAiCache,
};
use wfdiag_native_diagnostics::{DiagnosticOutput, DiagnosticTask, ScanKind};
use wfdiag_native_history::{ComparisonSummary, ScanStorage, ScanSummary, TaskTrend};
use wfdiag_native_issues::projection::project_issues;
use wfdiag_native_issues::{Issue, RemediationSummary};
use wfdiag_native_projection::process_identity::ProcessIdentity;
use wfdiag_native_remediation::broker::ActionProposal;
use wfdiag_native_remediation::runtime::ActionRunSummary;
use wfdiag_native_settings::{AppSettings, ProviderCredentialTransaction, ProviderKeyId};
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use wfdiag_native_update::UpdateInfo;
use wfdiag_native_update::policy::AboutExternalAction;
use wfdiag_ui_core::{DiagnosticTaskResult, SystemStats, TaskProgressStatus};
use windows_reactor::*;

/// Relaunching this process elevated, as the engine's [`ElevationPort`].
///
/// The UAC prompt and its COM call block; the facade already runs this on its
/// own thread, so nothing here touches the WinUI dispatcher.
#[derive(Debug, Default, Clone, Copy)]
struct ReactorElevation;

impl ElevationPort for ReactorElevation {
    fn restart_as_admin(&self) -> Result<bool, String> {
        wfdiag_native_remediation::elevation::relaunch_self_elevated_with_flag(
            crate::app::consts::ELEVATED_RELAUNCH_FLAG,
        )
    }
}

/// Everything the shell renders, plus the one engine handle it drives.
// Independent presentational facts, each read by a different surface. There is
// no state machine here to merge them into: the state machines all moved into
// `wfdiag-app`.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct WfdiagShell {
    /// The one application service. `None` only in the deterministic visual
    /// and fixture modes, which must never start a worker or touch the system.
    app: Option<AppService>,
    /// The engine's event stream, drained on every coalesced wake.
    app_events: Option<AppEventReceiver>,

    // ---- chrome and layout --------------------------------------------
    page: Page,
    live_test_fixture: Option<LiveTestFixture>,
    theme: WindowTheme,
    effective_color_scheme: ColorScheme,
    window_size: WindowSize,
    requested_client_width: f64,
    requested_client_height: f64,
    pane_open: bool,
    deterministic_visual: bool,
    visual_state: VisualState,
    status: String,
    /// One-shot latch for #206 (toast failure) and #207 (degraded instance
    /// watch): both are session-level facts, so the status line states each
    /// at most once instead of re-announcing it on every scan or wake.
    notification_failure_reported: bool,
    degraded_instance_watch_reported: bool,

    // ---- About and the update notice ----------------------------------
    about_open: bool,
    about_close_reference: ElementRef<Button>,
    about_dialog_epoch: u64,
    about_action_error: Option<String>,
    about_launch_task: Option<ComponentTask>,
    update_info: Option<UpdateInfo>,
    update_notice_visible: bool,
    update_notice_epoch: u64,
    update_notice_timer_generation: u64,
    update_notice_task: Option<ComponentTask>,
    update_notice_started_at: Option<Instant>,
    update_notice_remaining: Duration,

    // ---- Settings dialog ----------------------------------------------
    settings_open: bool,
    settings_snapshot: AppSettings,
    settings_draft: AppSettings,
    settings_dialog_epoch: u64,
    /// The dialog epoch of the Save that is still being persisted.
    settings_save_epoch: Option<u64>,
    settings_loading: bool,
    settings_saving: bool,
    settings_error: Option<String>,
    settings_save_error: Option<String>,
    provider_key_drafts: [String; ProviderKeyId::ALL.len()],
    provider_credential_transaction: ProviderCredentialTransaction,
    provider_key_busy: bool,
    provider_setup_index: usize,
    provider_setup_error: Option<String>,
    provider_catalogs: Vec<CatalogState>,
    subscription_auth_error: Option<String>,
    subscription_auth_states: Vec<AccountState>,
    subscription_install_prompt: Option<InstallPrompt>,
    subscription_install_progress: Option<SubscriptionInstallProgress>,
    subscription_install_error: Option<String>,
    subscription_install_busy: bool,

    // ---- host identity -------------------------------------------------
    system_info: SystemInfo,
    architecture: Option<ArchitectureSnapshot>,
    system_error: Option<String>,
    is_admin: bool,

    // ---- diagnostics ---------------------------------------------------
    diagnostic_results: Vec<DiagnosticTaskResult>,
    diagnostic_catalog: Vec<DiagnosticTask>,
    diagnostic_expected_task_ids: Vec<String>,
    diagnostic_task_statuses: HashMap<String, TaskProgressStatus>,
    diagnostic_scan_kind: Option<ScanKind>,
    diagnostic_session_id: Option<String>,
    diagnostic_duration_ms: u64,
    diagnostic_total: usize,
    diagnostic_completed: usize,
    diagnostic_errors: usize,
    diagnostic_current_task: Option<String>,
    scan_phase: ScanPhase,
    selected_result_task_id: Option<String>,
    diagnostic_filter: String,
    diagnostic_raw_output: bool,
    diagnostic_analyses: BTreeMap<String, DiagnosticAnalysisDisplay>,

    // ---- issues --------------------------------------------------------
    issues: Vec<Issue>,
    issue_maintenance: Vec<RemediationSummary>,
    issue_error: Option<String>,
    issue_refreshing: bool,
    /// The evidence the visible issue projection came from, so the Issues page
    /// can grey out a projection that a newer scan has already replaced.
    issue_projected_session_id: Option<String>,
    issue_prioritization: IssuePrioritizationDisplay,
    fix_plan: Option<ValidatedFixPlan>,
    fix_plan_busy: bool,
    fix_plan_error: Option<String>,

    // ---- remediation ---------------------------------------------------
    action_active_run: Option<ActionRunSummary>,
    action_run_history: Vec<ActionRunSummary>,
    action_expanded_runs: HashSet<String>,
    action_review: Option<ActionProposal>,
    repair_confirm: Option<ActionProposal>,
    action_busy: bool,

    // ---- live monitoring and processes ---------------------------------
    monitoring_paused: bool,
    monitoring_paused_by_lifecycle: bool,
    latest_system_stats: Option<SystemStats>,
    monitor_history: MonitorHistory,
    monitor_error: Option<String>,
    network_connections: Option<Vec<NetworkConnection>>,
    network_loading: bool,
    process_filter: String,
    process_page: Option<ProcessPage>,
    process_sort_key: ProcessSortKey,
    process_sort_direction: ProcessSortDirection,
    process_offset: usize,
    process_loading: bool,
    process_error: Option<String>,
    selected_process: Option<ProcessIdentity>,
    /// The process-filter debounce. This is the one remaining
    /// `spawn_background` on the data path: it is pure typing latency, not a
    /// worker wait.
    process_debounce_revision: u64,
    process_debounce_task: Option<ComponentTask>,
    process_last_refresh_started_at: Option<Instant>,

    // ---- scan history ---------------------------------------------------
    history_summaries: Vec<ScanSummary>,
    history_filter: String,
    selected_history_id: Option<String>,
    history_comparison: Option<ComparisonSummary>,
    history_comparing: bool,
    history_comparison_error: Option<String>,
    history_expanded_task_id: Option<String>,
    history_task_diff: Option<HistoryTaskDiffProjection>,
    history_task_diff_loading: bool,
    history_task_diff_error: Option<String>,
    history_loading: bool,
    history_error: Option<String>,
    history_clear_confirm: bool,
    history_label_draft: String,
    history_label_editing: bool,
    history_tag_draft: String,
    history_ack_busy: bool,
    history_trends: Option<Vec<TaskTrend>>,
    history_trends_loading: bool,
    history_trends_error: Option<String>,
    history_trends_baseline_id: Option<String>,

    // ---- AI ------------------------------------------------------------
    ai_mode: AiMode,
    chat_input: String,
    chat_composer_reference: ElementRef<TextBox>,
    chat_focus_revision: u64,
    chat_answer: Option<String>,
    chat_messages: Vec<ChatDisplayMessage>,
    /// Which rendered turn the current stream belongs to. The engine's own
    /// `Auto` retries are invisible, so one turn is one pair of bubbles.
    chat_turn: u64,
    /// Whether the current turn already has its pair of rendered bubbles. An
    /// `Auto` fallback retry is the same logical turn, so it must not push a
    /// second pair.
    chat_turn_open: bool,
    chat_streaming: bool,
    chat_last_prompt: Option<String>,
    full_scan_consent: Option<FullScanConsent>,
    cloud_fallback_consent: Option<CloudFallbackConsent>,
    report_text: Option<String>,
    report_provider: Option<String>,
    report_provider_use: Option<ProviderUse>,
    report_generating: bool,
    report_error: Option<String>,
    pending_ai_intent: Option<PendingAiIntent>,
    pending_ai_preparation_error: Option<String>,
    ai_provider_status: Option<AIProviderStatus>,
    ai_status_loading: bool,
    ai_status_error: Option<String>,

    // ---- export ---------------------------------------------------------
    /// Guards a save-picker answer against a superseded request (#140).
    export_picker_epoch: u64,
    export_picker_busy: bool,
    export_pending: Option<PendingExportAction>,
    export_write_task: Option<ComponentTask>,
    export_error: Option<String>,

    // ---- palette, shortcuts, window integration --------------------------
    palette_open: bool,
    palette_query: String,
    palette_active_index: usize,
    palette_query_reference: ElementRef<TextBox>,
    palette_button_reference: ElementRef<Button>,
    palette_result_references: [ElementRef<Button>; PALETTE_MAX_RESULTS],
    palette_dialog_epoch: u64,
    palette_focus_task: Option<ComponentTask>,
    shortcut_help_open: bool,
    window_hook_installed: bool,
    window_hook_retry_failures: u8,
    window_hook_retry_task: Option<ComponentTask>,
    window_lifecycle_revision: u64,
    window_usable: bool,
    instance_wait: Option<ComponentTask>,
}

/// Start the one application service the shell drives.
///
/// Everything environmental is chosen here and nowhere else: the settings
/// store (redirected by the `settings-test-path` validation feature), the
/// provider-preference admission rule, the update throttle beside it, the AI
/// provider backend, and the elevation policy. The startup-scan gate, the
/// per-worker teardown budget and every request-id counter belong to the
/// service.
fn start_application_service(
    start_monitor: bool,
) -> Result<(AppService, AppEventReceiver), String> {
    let identity: Arc<dyn PackageIdentitySource> =
        Arc::new(crate::app::policy::ReactorPackageIdentitySource::default());
    let validator = Arc::new(ProviderPreferenceSettingsValidator::new(Arc::clone(
        &identity,
    )));
    let settings_service =
        crate::app::policy::reactor_settings_service(validator.clone() as Arc<_>);
    let provider_backend =
        reactor_provider_backend(settings_service, identity, SharedAiCache::new(100));
    let overrides = WindowsPortOverrides {
        settings_storage: crate::app::policy::reactor_settings_storage(),
        settings_validator: Some(validator as Arc<_>),
        update_throttle: crate::app::policy::reactor_update_throttle_port(),
    };
    let ports = windows_ports_with(
        overrides,
        provider_backend,
        Some(Arc::new(ReactorElevation) as Arc<dyn ElevationPort>),
        APP_VERSION,
    );
    let mut config = AppConfig::default()
        .with_monitor(start_monitor)
        .with_debug_build(cfg!(debug_assertions));
    // History is optional evidence: a host with no resolvable storage
    // directory still scans, exports and chats, and every history command is
    // then refused with a typed reason instead of hanging.
    if let Ok(directory) = ScanStorage::default_storage_directory() {
        config = config.with_history_dir(directory);
    }
    AppService::start(config, ports).map_err(|error| error.to_string())
}

/// The engine facts the very first frame needs, captured before the service is
/// moved into the component.
///
/// [`AppService::start`] loads the persisted settings synchronously and
/// rehydrates any remediation preview that survived a previous process, so the
/// first published view is already correct instead of flashing defaults (#200).
struct EngineBoot {
    settings: AppSettings,
    settings_error: Option<String>,
    catalog: Vec<DiagnosticTask>,
    maintenance: Vec<RemediationSummary>,
    issues: Vec<Issue>,
    active_run: Option<ActionRunSummary>,
    run_history: Vec<ActionRunSummary>,
    review: Option<ActionProposal>,
    system_info: Option<SystemInfo>,
    architecture: Option<ArchitectureSnapshot>,
    system_error: Option<String>,
    session_id: Option<String>,
}

impl EngineBoot {
    fn capture(app: Option<&AppService>) -> Self {
        let Some(snapshot) = app.map(AppService::snapshot) else {
            return Self {
                settings: AppSettings::default(),
                settings_error: None,
                catalog: Vec::new(),
                maintenance: wfdiag_native_issues::projection::canonical_issue_metadata_snapshot()
                    .maintenance,
                issues: Vec::new(),
                active_run: None,
                run_history: Vec::new(),
                review: None,
                system_info: None,
                architecture: None,
                system_error: None,
                session_id: None,
            };
        };
        Self {
            settings: snapshot.settings.clone(),
            settings_error: snapshot.settings_error.clone(),
            catalog: snapshot.catalog.clone(),
            maintenance: snapshot.maintenance_remediations(),
            issues: snapshot.issues.clone(),
            active_run: snapshot.actions.active_run.clone(),
            run_history: snapshot.actions.history.clone(),
            review: snapshot.actions.review.clone(),
            system_info: snapshot.system_info.clone(),
            architecture: snapshot.architecture.clone(),
            system_error: snapshot.system_error.clone(),
            session_id: snapshot.scan.effective_session_id(),
        }
    }
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
            about_open: false,
            about_close_reference: ElementRef::new(),
            about_dialog_epoch: 0,
            about_action_error: None,
            about_launch_task: None,
            update_info: None,
            update_notice_visible: false,
            update_notice_epoch: 0,
            update_notice_timer_generation: 0,
            update_notice_task: None,
            update_notice_started_at: None,
            update_notice_remaining: Duration::ZERO,
            settings_open,
            settings_snapshot: settings_defaults.clone(),
            settings_draft: settings_defaults,
            settings_dialog_epoch: u64::from(settings_open),
            settings_save_epoch: None,
            settings_loading: false,
            settings_saving: false,
            settings_error,
            settings_save_error: None,
            provider_key_drafts: Default::default(),
            provider_credential_transaction: ProviderCredentialTransaction::new(),
            provider_key_busy: false,
            provider_setup_index: initial_provider_setup_index,
            provider_setup_error: None,
            provider_catalogs: (0..PROVIDER_SETUP_LABELS.len())
                .map(|_| CatalogState::default())
                .collect(),
            subscription_auth_error: None,
            subscription_auth_states: vec![AccountState::default(), AccountState::default()],
            subscription_install_prompt: None,
            subscription_install_progress: None,
            subscription_install_error: None,
            subscription_install_busy: false,
            system_info: initial_system_info,
            architecture: engine.architecture.clone(),
            system_error: engine.system_error.clone(),
            is_admin,
            diagnostic_results,
            diagnostic_catalog,
            diagnostic_expected_task_ids: Vec::new(),
            diagnostic_task_statuses: HashMap::new(),
            diagnostic_scan_kind: has_fixture_scan.then_some(ScanKind::Quick),
            diagnostic_session_id: None,
            diagnostic_duration_ms: if has_fixture_scan { 2_300 } else { 0 },
            diagnostic_total: if has_fixture_scan { 17 } else { 0 },
            diagnostic_completed: if has_fixture_scan { 17 } else { 0 },
            diagnostic_errors: 0,
            diagnostic_current_task: None,
            scan_phase: ScanPhase::Idle,
            selected_result_task_id: None,
            diagnostic_filter: String::new(),
            diagnostic_raw_output: false,
            diagnostic_analyses: BTreeMap::new(),
            issues,
            issue_maintenance,
            issue_error: None,
            issue_refreshing: false,
            issue_projected_session_id: has_fixture_scan
                .then(|| "visual-fixture".to_string())
                .or_else(|| engine.session_id.clone()),
            issue_prioritization: IssuePrioritizationDisplay::default(),
            fix_plan: None,
            fix_plan_busy: false,
            fix_plan_error: None,
            action_active_run,
            action_run_history,
            action_expanded_runs,
            action_review,
            repair_confirm: None,
            action_busy: false,
            monitoring_paused: false,
            monitoring_paused_by_lifecycle: false,
            latest_system_stats: match visual_state {
                VisualState::MonitorEmpty => Some(fixture_monitor_empty_stats()),
                VisualState::SettingsBottom => Some(fixture_system_stats()),
                _ if fixture_mode => Some(fixture_system_stats()),
                _ => None,
            },
            monitor_history: match visual_state {
                VisualState::MonitorEmpty => {
                    let mut history = MonitorHistory::default();
                    history.push_stats(&fixture_monitor_empty_stats());
                    history
                }
                VisualState::SettingsBottom => MonitorHistory::fixture_258(),
                _ if fixture_mode => MonitorHistory::fixture_258(),
                _ => MonitorHistory::default(),
            },
            monitor_error: None,
            network_connections: None,
            network_loading: false,
            process_filter: String::new(),
            process_page: None,
            process_sort_key: ProcessSortKey::CpuPercent,
            process_sort_direction: ProcessSortDirection::Desc,
            process_offset: 0,
            process_loading: false,
            process_error: None,
            selected_process: None,
            process_debounce_revision: 0,
            process_debounce_task: None,
            process_last_refresh_started_at: None,
            history_summaries: Vec::new(),
            history_filter: String::new(),
            selected_history_id: None,
            history_comparison: None,
            history_comparing: false,
            history_comparison_error: None,
            history_expanded_task_id: None,
            history_task_diff: None,
            history_task_diff_loading: false,
            history_task_diff_error: None,
            history_loading: false,
            history_error: None,
            history_clear_confirm: false,
            history_label_draft: String::new(),
            history_label_editing: false,
            history_tag_draft: String::new(),
            history_ack_busy: false,
            history_trends: None,
            history_trends_loading: false,
            history_trends_error: None,
            history_trends_baseline_id: None,
            ai_mode: AiMode::Assistant,
            chat_input: String::new(),
            chat_composer_reference: ElementRef::new(),
            chat_focus_revision: 0,
            chat_answer: None,
            chat_messages: Vec::new(),
            chat_turn: 0,
            chat_turn_open: false,
            chat_streaming: false,
            chat_last_prompt: None,
            full_scan_consent: None,
            cloud_fallback_consent: None,
            report_text: None,
            report_provider: None,
            report_provider_use: None,
            report_generating: false,
            report_error: None,
            pending_ai_intent: None,
            pending_ai_preparation_error: None,
            ai_provider_status: None,
            ai_status_loading: false,
            ai_status_error: None,
            export_picker_epoch: 0,
            export_picker_busy: false,
            export_pending: None,
            export_write_task: None,
            export_error: None,
            palette_open: false,
            palette_query: String::new(),
            palette_active_index: 0,
            palette_query_reference: ElementRef::new(),
            palette_button_reference: ElementRef::new(),
            palette_result_references: std::array::from_fn(|_| ElementRef::new()),
            palette_dialog_epoch: 0,
            palette_focus_task: None,
            shortcut_help_open: false,
            window_hook_installed: false,
            window_hook_retry_failures: 0,
            window_hook_retry_task: None,
            window_lifecycle_revision,
            window_usable: true,
            instance_wait,
        };

        // The startup-scan gate now lives in the engine: `Start` arms it, and
        // the scan runs once settings and host identity have both settled.
        component.dispatch(AppCommand::Start {
            startup_scan: component.settings_snapshot.scan_on_startup,
        });
        window::set_close_to_tray(component.settings_snapshot.close_to_tray);
        // Keep the collector warm but idle until a live surface consumes it.
        if !initial_page.consumes_live_telemetry() {
            component.dispatch(AppCommand::SetMonitorPaused { paused: true });
            component.monitoring_paused = true;
        }
        if initial_page == Page::Processes && !deterministic_visual {
            component.request_process_page(context, false);
        }
        if initial_page == Page::History && !deterministic_visual {
            component.request_history_list();
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
            Message::Navigate(Some(tag)) => {
                if let Some(page) = Page::from_tag(&tag) {
                    self.navigate_to_page(page, context);
                } else if tag == "quick-scan" {
                    self.transition_to_page(Page::Diagnostics);
                    self.begin_diagnostic_scan(ScanKind::Quick);
                } else {
                    match tag.as_str() {
                        "export" => self.request_export_to_file(),
                        "share" => self.request_share_to_windowsforum(),
                        "email" => self.request_email_report(),
                        _ => (),
                    }
                }
            }
            Message::Navigate(None) => {}
            Message::WindowSize(size) => self.window_size = size,
            Message::ColorSchemeChanged(color_scheme) => {
                self.effective_color_scheme = color_scheme;
            }
            Message::TogglePane => self.toggle_navigation_rail(),
            Message::ToggleTheme => {
                self.handle_palette_command("toggle-theme".to_string(), context);
            }
            Message::OpenAbout => self.open_about(),
            Message::TogglePalette => {
                self.set_palette_visibility(!self.palette_open, context);
            }
            Message::ClosePalette => {
                self.set_palette_visibility(false, context);
            }
            Message::PaletteFocusReady { epoch, action } => {
                if self.palette_dialog_epoch == epoch {
                    self.palette_focus_task = None;
                    match action {
                        PaletteFocusAction::FocusQuery if self.palette_open => {
                            let _ = self.palette_query_reference.request_focus();
                        }
                        PaletteFocusAction::RestorePrevious if !self.palette_open => {
                            // A ContentDialog is a native popup. Reactivate its
                            // owner before restoring the exact XAML element so
                            // the disappearing InputSite cannot retain global
                            // keyboard focus.
                            instance::activate_main_window();
                            if !focus::restore_pre_palette_focus() {
                                let _ = self.palette_button_reference.request_focus();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::PaletteFocusCancelled { epoch } | Message::PaletteFocusRejected { epoch } => {
                if self.palette_dialog_epoch == epoch {
                    self.palette_focus_task = None;
                }
            }
            Message::PaletteQueryChanged(value) => {
                self.palette_query = value;
                self.palette_active_index = 0;
            }
            Message::PaletteActiveChanged(index) => {
                let match_count =
                    palette_visible_matches(self.palette_command_specs(), &self.palette_query)
                        .len();
                self.palette_active_index = index.min(match_count.saturating_sub(1));
            }
            Message::PaletteCommand(tag) => {
                self.set_palette_visibility(false, context);
                self.handle_palette_command(tag, context);
            }
            Message::ShowShortcutHelp => self.shortcut_help_open = true,
            Message::CloseShortcutHelp => self.shortcut_help_open = false,
            Message::ProviderKeyDraftChanged(index, value) => {
                if let Some(draft) = self.provider_key_drafts.get_mut(index) {
                    *draft = value;
                    let setup_uses_key = matches!(
                        (self.provider_setup_index, index),
                        (5, 0) | (6, 1) | (7, 2) | (8, 3) | (9, 4)
                    );
                    if setup_uses_key {
                        self.request_provider_model_refresh(false);
                    }
                }
            }
            Message::StoreProviderKey(index) => self.submit_provider_key(index, true),
            Message::ClearProviderKey(index) => {
                if let Some(draft) = self.provider_key_drafts.get_mut(index) {
                    draft.clear();
                }
                self.submit_provider_key(index, false);
                self.request_provider_model_refresh(false);
            }
            Message::DiagnosticFilterChanged(value) => {
                self.diagnostic_filter = value;
                let selected_visible = self.selected_result_task_id.as_deref().is_some_and(|id| {
                    self.diagnostic_results.iter().any(|result| {
                        result.task_id == id
                            && diagnostic_matches_filter(
                                result,
                                &self.diagnostic_catalog,
                                &self.diagnostic_filter,
                            )
                    })
                });
                if !selected_visible {
                    self.selected_result_task_id = self
                        .diagnostic_results
                        .iter()
                        .find(|result| {
                            diagnostic_matches_filter(
                                result,
                                &self.diagnostic_catalog,
                                &self.diagnostic_filter,
                            )
                        })
                        .map(|result| result.task_id.clone());
                    self.diagnostic_raw_output = false;
                }
            }
            Message::SetDiagnosticRaw(raw) => self.diagnostic_raw_output = raw,
            Message::SelectDiagnosticResult(task_id) => {
                self.selected_result_task_id = Some(task_id.clone());
                self.diagnostic_raw_output = false;
                self.status = format!("Selected diagnostic: {task_id}");
            }
            Message::AnalyzeSelectedDiagnostic => self.begin_selected_diagnostic_analysis(false),
            Message::RetrySelectedDiagnosticAnalysis => {
                self.begin_selected_diagnostic_analysis(true);
            }
            Message::CancelDiagnosticAnalysis => {
                if self.dispatch(AppCommand::CancelAnalysis).is_accepted() {
                    self.status = "Cancelling the diagnostic interpretation…".to_string();
                }
            }
            Message::RequestNetworkConnections => {
                if self.deterministic_visual || self.network_loading {
                    return;
                }
                // #198: the request goes through the engine, which drops a
                // reply that a newer request has already superseded.
                match self.dispatch(AppCommand::RequestNetworkConnections) {
                    DispatchOutcome::Accepted { .. } => self.network_loading = true,
                    outcome => self.report_rejection(&outcome),
                }
            }
            Message::ToggleQuickScanTask(task_id) => {
                let tasks = self
                    .settings_draft
                    .quick_scan_tasks
                    .get_or_insert_with(Vec::new);
                match tasks.iter().position(|existing| existing == &task_id) {
                    Some(position) => {
                        tasks.remove(position);
                    }
                    None => tasks.push(task_id.clone()),
                }
                self.status = format!(
                    "Quick Scan customization: {} tasks selected · press Save to apply",
                    tasks.len()
                );
            }
            Message::ToggleClearHistoryConfirm(open) => self.history_clear_confirm = open,
            Message::BeginHistoryLabelEdit => self.begin_history_label_edit(),
            Message::CancelHistoryLabelEdit => self.cancel_history_label_edit(),
            Message::HistoryLabelDraftChanged(value) => self.history_label_draft = value,
            Message::SaveHistoryLabel => self.request_history_label_save(),
            Message::RequestHistoryTrends => self.request_history_trends(),
            Message::ClearHistoryConfirmed => self.request_history_clear(),
            Message::HistoryTagDraftChanged(value) => self.history_tag_draft = value,
            Message::SaveHistoryTags => self.request_history_tags_save(),
            Message::AboutClosed { epoch } => self.close_about(epoch),
            Message::AboutExternalRequested { epoch, action } => {
                self.request_about_external_action(epoch, action, context);
            }
            Message::AboutExternalFinished { epoch, result } => {
                if self.about_dialog_is_current(epoch) {
                    self.about_launch_task = None;
                    self.about_action_error = result.err();
                }
            }
            Message::AboutExternalRejected { epoch } => {
                if self.about_dialog_is_current(epoch) {
                    self.about_launch_task = None;
                    self.about_action_error =
                        Some("The link could not enter the Reactor background queue".to_string());
                }
            }
            Message::UpdateNoticeClosed { epoch } => self.close_update_notice(epoch),
            Message::UpdateNoticeExpired {
                epoch,
                timer_generation,
            }
            | Message::UpdateNoticeTimerRejected {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.update_notice_visible,
                    self.update_notice_epoch,
                    self.update_notice_timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.update_notice_task = None;
                    self.update_notice_visible = false;
                    self.update_notice_started_at = None;
                    self.update_notice_remaining = Duration::ZERO;
                }
            }
            Message::UpdateNoticePointerEntered { epoch } => self.pause_update_notice(epoch),
            Message::UpdateNoticePointerExited { epoch } => {
                self.resume_update_notice(epoch, context);
            }
            Message::UpdateNoticeTimerCancelled {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.update_notice_visible,
                    self.update_notice_epoch,
                    self.update_notice_timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.update_notice_task = None;
                }
            }
            Message::OpenSettings => self.open_settings(),
            Message::SettingsDialog { epoch, action } => {
                self.apply_settings_dialog_action(epoch, action);
            }
            Message::RefreshProviderModels => {
                if let Some(provider) =
                    subscription_auth_provider_for_setup(self.provider_setup_index)
                {
                    self.begin_subscription_auth_operation(
                        provider,
                        wfdiag_app::SubscriptionOperation::Status,
                    );
                }
                self.request_provider_model_refresh(true);
            }
            Message::CancelProviderModels => self.cancel_provider_model_request(),
            Message::RefreshSubscriptionAuth(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::Status,
                );
            }
            Message::StartSubscriptionSignIn(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::SignIn,
                );
            }
            Message::StartSubscriptionSignOut(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::SignOut,
                );
            }
            Message::CancelSubscriptionAuth => self.cancel_subscription_auth(),
            Message::RequestSubscriptionInstall(provider) => {
                self.request_subscription_install(provider);
            }
            Message::SubscriptionInstallPromptClosed { prompt, result } => {
                if self.subscription_install_prompt != Some(prompt) {
                    return;
                }
                self.answer_subscription_install_prompt(result == ContentDialogResult::Primary);
            }
            Message::CancelSubscriptionInstall => self.cancel_subscription_install(),
            Message::RequestQuickScan => self.begin_diagnostic_scan(ScanKind::Quick),
            Message::RequestFullScan => self.begin_diagnostic_scan(ScanKind::Full),
            Message::CancelScan => self.request_diagnostic_cancel(),
            Message::ExportPickerFinished {
                epoch,
                kind,
                outcome,
            } => self.apply_export_picker_reply(epoch, kind, *outcome),
            Message::ExportFileSaved { epoch, result } => {
                if epoch != self.export_picker_epoch {
                    return;
                }
                self.export_write_task = None;
                self.export_pending = None;
                match *result {
                    Ok(path) => {
                        self.export_error = None;
                        self.status = format!("Results saved to {}", path.display());
                    }
                    Err(error) => {
                        self.export_error = Some(error);
                        self.status =
                            "Failed to save the file. Please try a different location.".to_string();
                    }
                }
            }
            Message::SupportPackageSaved { epoch, result } => {
                if epoch != self.export_picker_epoch {
                    return;
                }
                self.export_write_task = None;
                self.export_pending = None;
                match *result {
                    Ok(paths) => {
                        self.export_error = None;
                        self.status = format!(
                            "Support package saved · {} · {} · {}",
                            paths.json.display(),
                            paths.text.display(),
                            paths.html.display()
                        );
                    }
                    Err(error) => {
                        self.status = format!(
                            "Support package could not be written completely · {error} · Try exporting individual files"
                        );
                        self.export_error = Some(error);
                    }
                }
            }
            Message::SetAiMode(mode) => self.ai_mode = mode,
            Message::ToggleMonitoring => self.toggle_monitoring(),
            Message::Refresh => self.refresh_current_page(context),
            Message::ProcessFilterChanged(value) => {
                self.process_filter = value;
                self.process_offset = 0;
                self.selected_process = None;
                self.request_process_page(context, true);
            }
            Message::ProcessSort(sort_key) => self.set_process_sort(sort_key, context),
            Message::ProcessPrevious => {
                self.process_offset = self
                    .process_offset
                    .saturating_sub(crate::app::consts::PROCESS_PAGE_SIZE);
                self.selected_process = None;
                self.request_process_page(context, false);
            }
            Message::ProcessNext => {
                if let Some(page) = self.process_page.as_ref()
                    && page.offset.saturating_add(page.items.len()) < page.total
                {
                    self.process_offset = page.offset.saturating_add(page.limit);
                    self.selected_process = None;
                    self.request_process_page(context, false);
                }
            }
            Message::ProcessQueryDue { revision } => {
                if revision == self.process_debounce_revision {
                    self.process_debounce_task = None;
                    self.send_process_page_request();
                }
            }
            Message::ProcessQueryDebounceEnded { revision } => {
                if revision == self.process_debounce_revision {
                    self.process_debounce_task = None;
                    self.process_loading = false;
                }
            }
            Message::SelectProcess(identity) => self.selected_process = identity,
            Message::RefreshHistory => self.request_history_list(),
            Message::HistoryFilterChanged(value) => self.history_filter = value,
            Message::SelectHistory(scan_id) => self.select_history_scan(scan_id),
            Message::ToggleHistoryTaskDetail(task_id) => self.toggle_history_task_detail(task_id),
            Message::ChatInputChanged(value) => {
                if !self.chat_interaction_blocked() {
                    self.chat_input = value;
                }
            }
            Message::UsePrompt(value) => {
                if self.chat_streaming || self.chat_interaction_blocked() {
                    self.status = "Finish or cancel the current AI request before starting another"
                        .to_string();
                } else {
                    self.begin_chat_send(value);
                }
            }
            Message::SendChat => self.begin_chat_send(self.chat_input.clone()),
            Message::CancelChat => self.cancel_chat(),
            Message::NewConversation => self.begin_new_conversation(),
            Message::AllowCloudFallback => self.answer_cloud_fallback(true),
            Message::NeverCloudFallback => self.answer_cloud_fallback(false),
            Message::ApproveFullScan => self.approve_full_scan(),
            Message::DismissFullScan => {
                self.full_scan_consent = None;
                self.status = "Full Scan request dismissed".to_string();
            }
            Message::GenerateReport => self.begin_report_generation(false),
            Message::RegenerateReport => self.begin_report_generation(true),
            Message::CancelReport => self.cancel_report(),
            Message::CancelPendingAiIntent => self.cancel_pending_ai_intent(),
            Message::RetryPendingAiIntent => self.retry_pending_ai_intent(),
            Message::CopyReport => {
                let Some(report) = self.report_text.as_deref() else {
                    self.status = "There is no completed AI report to copy".to_string();
                    return;
                };
                match write_text_to_clipboard(report) {
                    Ok(()) => self.status = "AI report copied to the clipboard".to_string(),
                    Err(error) => self.status = format!("Could not copy the AI report · {error}"),
                }
            }
            Message::ExplainLatestScan => {
                self.transition_to_page(Page::Ai);
                self.ai_mode = AiMode::ScanReport;
                self.begin_report_generation(false);
            }
            Message::RunRemediation(remediation_id) => self.run_remediation(remediation_id),
            Message::AskAiAboutIssue(issue_id) => self.ask_ai_about_issue(&issue_id),
            Message::PrioritizeIssues => {
                self.begin_issue_prioritization(self.issue_prioritization.text.is_some());
            }
            Message::CancelIssuePrioritization => {
                if self.dispatch(AppCommand::CancelAnalysis).is_accepted() {
                    self.status = "Cancelling issue prioritization…".to_string();
                }
            }
            Message::ProposeFixPlan => self.begin_fix_plan(),
            Message::CancelFixPlan => {
                if self.dispatch(AppCommand::CancelFixPlan).is_accepted() {
                    self.status = "Cancelling the AI fix plan…".to_string();
                }
            }
            Message::ReviewFixPlanActions(selection) => self.prepare_action_selection(selection),
            Message::ActionReviewDialogClosed {
                proposal_id,
                result,
            } => self.close_action_review(&proposal_id, result),
            Message::RepairDialogClosed {
                proposal_id,
                result,
            } => self.close_repair_confirmation(&proposal_id, result),
            Message::CancelActionRun => self.cancel_action_run(),
            Message::ActionRunExpandedChanged { run_id, expanded } => {
                let visible = self
                    .action_active_run
                    .as_ref()
                    .is_some_and(|run| run.run_id == run_id)
                    || self
                        .action_run_history
                        .iter()
                        .any(|run| run.run_id == run_id);
                if visible {
                    if expanded {
                        self.action_expanded_runs.insert(run_id);
                    } else {
                        self.action_expanded_runs.remove(&run_id);
                    }
                }
            }
            Message::RestartAsAdmin => self.request_admin_relaunch(),
            Message::InstanceWaitCancelled => self.instance_wait = None,
            Message::WindowHookRetryReady => {
                self.window_hook_retry_task = None;
                self.ensure_window_hook(context);
            }
            Message::WindowHookRetryRejected => {
                self.window_hook_retry_task = None;
                self.status = "Native window integration retry was interrupted; the next UI action will retry"
                    .to_string();
            }
            Message::InstanceActivated => {
                // Another launch asked this instance to the foreground.
                instance::activate_main_window();
                self.arm_instance_watch(context, self.window_lifecycle_revision);
            }
            Message::WindowLifecycleChanged(observed) => {
                // Coalesce rapid deactivate/reactivate or hide/show pairs so
                // a queued intermediate snapshot cannot cause a visible
                // pause/resume flicker after the window is already usable.
                let current = window::lifecycle_snapshot();
                let snapshot = if current.revision == observed.revision {
                    observed
                } else {
                    current
                };
                self.arm_instance_watch(context, snapshot.revision);
                self.apply_window_lifecycle(snapshot, context);
                if snapshot.focused && self.palette_open {
                    // Re-activation (Alt+Tab, AppActivate, or an instance
                    // handoff) must preserve the palette's editing target.
                    // This is lifecycle-driven and adds no idle polling.
                    let _ = self.palette_query_reference.request_focus();
                }
            }
            Message::GlobalShortcut(shortcut) => {
                self.arm_instance_watch(context, self.window_lifecycle_revision);
                self.handle_global_shortcut(shortcut, context);
            }
            Message::TrayCommand(command) => {
                self.arm_instance_watch(context, self.window_lifecycle_revision);
                self.handle_tray_command(command, context);
            }
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
            self.page.nav_label()
        ));
        context.window_visuals(
            WindowVisuals::new()
                .theme(self.theme)
                .backdrop(WindowBackdrop::Acrylic)
                .client_size(self.requested_client_width, self.requested_client_height)
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
        let window_hook_bootstrap = !self.deterministic_visual
            && !self.window_hook_installed
            && self.window_hook_retry_task.is_none();
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

        let effective_theme = effective_window_theme(self.theme, self.effective_color_scheme);
        let palette = Palette::for_theme(effective_theme);
        let narrow = self.window_size.width < 940.0;
        let diagnostics_compact = diagnostics_uses_compact_layout(self.window_size.width);
        let rail_forced_collapsed = navigation_rail_forced_collapsed(self.window_size.width);
        // Keep content-specific compact layouts independent from the shipping
        // shell's 1100 px forced rail collapse.
        // Keep the user's expanded preference so the full pane returns when the
        // window grows again, but never let it consume the compact content area.
        let pane_expanded = self.pane_open && !rail_forced_collapsed;
        // The engine only publishes issues for evidence it has committed, so
        // "current" is simply: the projection names the visible scan.
        let issue_projection_current = self.issue_projected_session_id.as_deref()
            == self
                .diagnostic_results
                .first()
                .map(|result| result.session_id.as_str());
        let selected_analysis = self
            .selected_result_task_id
            .as_deref()
            .or_else(|| {
                self.diagnostic_results
                    .first()
                    .map(|result| result.task_id.as_str())
            })
            .and_then(|task_id| self.diagnostic_analyses.get(task_id));
        let diagnostic_ai_available = self.settings_snapshot.ai_enabled
            && !self.deterministic_visual
            && self
                .ai_provider_status
                .as_ref()
                .is_some_and(|status| status.active_provider != AIProvider::None);
        let chat_composer_reference = self.chat_composer_reference.clone();
        let focus_chat_composer = self.page == Page::Ai
            && self.ai_mode == AiMode::Assistant
            && self.chat_focus_revision > 0;
        context.use_effect(
            "chat-composer-focus",
            (focus_chat_composer, self.chat_focus_revision),
            move || {
                if focus_chat_composer {
                    let _ = chat_composer_reference.request_focus();
                }
                None
            },
        );
        let page = match self.page {
            Page::Diagnostics => diagnostics_page(
                palette,
                effective_theme,
                diagnostics_compact,
                &self.diagnostic_results,
                &self.diagnostic_catalog,
                &self.diagnostic_expected_task_ids,
                &self.diagnostic_task_statuses,
                self.diagnostics_busy(),
                self.scan_cancelling(),
                self.diagnostic_completed,
                self.diagnostic_total,
                self.diagnostic_current_task.as_deref(),
                self.diagnostic_duration_ms,
                context.message(Message::RequestQuickScan),
                context.message(Message::RequestFullScan),
                context.message(Message::CancelScan),
                context.message(Message::ExplainLatestScan),
                self.selected_result_task_id.clone(),
                context.callback(Message::SelectDiagnosticResult),
                &self.diagnostic_filter,
                context.callback(Message::DiagnosticFilterChanged),
                self.diagnostic_raw_output,
                context.message(Message::SetDiagnosticRaw(false)),
                context.message(Message::SetDiagnosticRaw(true)),
                selected_analysis,
                diagnostic_ai_available,
                context.message(Message::AnalyzeSelectedDiagnostic),
                context.message(Message::RetrySelectedDiagnosticAnalysis),
                context.message(Message::CancelDiagnosticAnalysis),
            ),
            Page::Monitor => monitor_page(
                palette,
                narrow,
                self.monitoring_paused,
                self.monitor_error.as_deref(),
                self.latest_system_stats.as_ref(),
                &self.monitor_history,
                context.message(Message::ToggleMonitoring),
                context.message(Message::Refresh),
                self.network_connections.as_deref(),
                self.network_loading,
                context.message(Message::RequestNetworkConnections),
            ),
            Page::Processes => processes_page(
                palette,
                self.window_size.width,
                pane_expanded,
                &self.process_filter,
                self.process_page.as_ref(),
                self.process_loading,
                self.process_error.as_deref(),
                self.process_sort_key,
                self.process_sort_direction,
                self.deterministic_visual,
                self.selected_process,
                self.monitoring_paused,
                context.callback(Message::ProcessFilterChanged),
                context.callback(Message::ProcessSort),
                context.message(Message::ProcessPrevious),
                context.message(Message::ProcessNext),
                context.callback(Message::SelectProcess),
                context.message(Message::ToggleMonitoring),
                context.message(Message::Refresh),
            ),
            Page::Ai => ai_page(
                palette,
                narrow,
                self.window_size.height,
                self.visual_state,
                self.deterministic_visual,
                self.settings_snapshot.ai_enabled,
                self.ai_mode,
                &self.chat_input,
                &self.chat_composer_reference,
                self.chat_answer.as_deref(),
                &self.chat_messages,
                self.full_scan_consent.as_ref(),
                self.cloud_fallback_consent.as_ref(),
                self.ai_provider_status.as_ref(),
                self.ai_status_loading,
                self.ai_status_error.as_deref(),
                AiPreparationUi {
                    intent: self.pending_ai_intent.as_ref(),
                    error: self.pending_ai_preparation_error.as_deref(),
                    scan_busy: self.diagnostics_busy(),
                    scan_cancelling: self.scan_cancelling(),
                    completed: self.diagnostic_completed,
                    total: self.diagnostic_total,
                    current_task: self.diagnostic_current_task.as_deref(),
                },
                context.message(Message::SetAiMode(AiMode::Assistant)),
                context.message(Message::SetAiMode(AiMode::ScanReport)),
                context.callback(Message::ChatInputChanged),
                context.callback(Message::UsePrompt),
                context.message(Message::SendChat),
                context.message(Message::NewConversation),
                context.message(Message::AllowCloudFallback),
                context.message(Message::NeverCloudFallback),
                context.message(Message::ApproveFullScan),
                context.message(Message::DismissFullScan),
                context.message(Message::OpenSettings),
                context.message(Message::CancelPendingAiIntent),
                context.message(Message::RetryPendingAiIntent),
                self.report_text.as_deref(),
                self.report_provider.as_deref(),
                self.report_provider_use.as_ref(),
                self.report_generating,
                self.report_error.as_deref(),
                !self.diagnostic_results.is_empty(),
                context.message(Message::GenerateReport),
                context.message(Message::RegenerateReport),
                context.message(Message::CancelReport),
                context.message(Message::CopyReport),
                self.chat_streaming,
                context.message(Message::CancelChat),
            ),
            Page::Issues => issues_page(
                palette,
                effective_theme,
                &self.issues,
                &self.issue_maintenance,
                self.fix_plan.as_ref(),
                self.fix_plan_busy,
                self.fix_plan_error.as_deref(),
                &self.issue_prioritization,
                self.action_active_run.as_ref(),
                &self.action_run_history,
                &self.action_expanded_runs,
                self.action_busy
                    || self.action_review.is_some()
                    || self.repair_confirm.is_some()
                    || self.action_active_run.is_some(),
                self.settings_snapshot.ai_enabled,
                self.is_admin,
                self.issue_refreshing,
                self.issue_error.as_deref(),
                !self.diagnostic_results.is_empty(),
                issue_projection_current,
                context.message(Message::RequestQuickScan),
                context.callback(Message::RunRemediation),
                context.callback(Message::AskAiAboutIssue),
                context.message(Message::PrioritizeIssues),
                context.message(Message::CancelIssuePrioritization),
                context.message(Message::ProposeFixPlan),
                context.message(Message::CancelFixPlan),
                context.callback(Message::ReviewFixPlanActions),
                context.message(Message::CancelActionRun),
                context.callback(|(run_id, expanded)| Message::ActionRunExpandedChanged {
                    run_id,
                    expanded,
                }),
                context.message(Message::RestartAsAdmin),
            ),
            Page::History => history_page(
                palette,
                narrow,
                self.deterministic_visual,
                self.visual_state == VisualState::HistoryEmpty,
                &self.history_summaries,
                &self.history_filter,
                self.selected_history_id.as_deref(),
                self.history_label_draft.as_str(),
                self.history_label_editing,
                self.history_tag_draft.as_str(),
                self.history_comparison.as_ref(),
                self.history_comparing,
                self.history_comparison_error.as_deref(),
                self.history_expanded_task_id.as_deref(),
                self.history_task_diff.as_ref(),
                self.history_task_diff_loading,
                self.history_task_diff_error.as_deref(),
                self.history_loading,
                self.history_error.as_deref(),
                self.history_ack_busy,
                context.message(Message::RefreshHistory),
                context.callback(Message::HistoryFilterChanged),
                context.callback(Message::SelectHistory),
                context.message(Message::BeginHistoryLabelEdit),
                context.message(Message::CancelHistoryLabelEdit),
                context.callback(Message::HistoryLabelDraftChanged),
                context.message(Message::SaveHistoryLabel),
                context.callback(Message::HistoryTagDraftChanged),
                context.message(Message::SaveHistoryTags),
                context.callback(Message::ToggleHistoryTaskDetail),
                context.message(Message::ToggleClearHistoryConfirm(true)),
                context.message(Message::ClearHistoryConfirmed),
                context.message(Message::ToggleClearHistoryConfirm(false)),
                self.history_clear_confirm,
                self.history_trends.as_deref(),
                self.history_trends_loading,
                self.history_trends_error.as_deref(),
                context.message(Message::RequestHistoryTrends),
            ),
        };

        let status_icon = if self.diagnostic_results.is_empty() {
            if effective_theme == WindowTheme::Light {
                STATUS_INFO_LIGHT
            } else {
                STATUS_INFO_DARK
            }
        } else if self.diagnostic_results.iter().any(|result| !result.success) {
            if effective_theme == WindowTheme::Light {
                STATUS_WARN_LIGHT
            } else {
                STATUS_WARN_DARK
            }
        } else if effective_theme == WindowTheme::Light {
            STATUS_OK_LIGHT
        } else {
            STATUS_OK_DARK
        };

        let fixture_scan = self
            .diagnostic_results
            .iter()
            .any(|result| result.session_id == "visual-fixture");
        let elapsed_prefix = if self.diagnostic_results.is_empty()
            || matches!(self.page, Page::Monitor | Page::Processes)
        {
            String::new()
        } else if fixture_scan {
            match self.page {
                Page::Issues => "1.8s · ".to_string(),
                _ => "2.3s · ".to_string(),
            }
        } else if self.diagnostic_duration_ms > 0 {
            format!(
                "{} · ",
                format_diagnostic_duration(self.diagnostic_duration_ms)
            )
        } else {
            String::new()
        };
        let privilege = privilege_label(self.is_admin);

        let status_bar = Border::new()
            .grid_row(1)
            .height(33.0)
            .padding(Thickness::xy(18.0, 0.0))
            .border_brush(palette.border)
            .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(7.0)
                            .vertical_alignment(VerticalAlignment::Center)
                            .children((
                                Image::new()
                                    .source_data(EncodedImage::from_static(status_icon))
                                    .width(11.0)
                                    .height(11.0),
                                TextBlock::new()
                                    .text(self.status.clone())
                                    .foreground(palette.muted)
                                    .font_size(11.5)
                                    .vertical_alignment(VerticalAlignment::Center),
                            )),
                        TextBlock::new()
                            .text(format!(
                                "{elapsed_prefix}{privilege}    wfdiag {APP_VERSION} · WindowsForum.com"
                            ))
                            .grid_column(1)
                            .foreground(palette.muted)
                            .font_size(11.5)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
            );

        let page_host: View = match (self.page, diagnostics_compact) {
            (Page::Diagnostics, false) => Border::new()
                .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
                .content(page),
            (Page::Diagnostics, true) => ScrollViewer::new()
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .content(
                    Border::new()
                        .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
                        .content(page),
                ),
            _ => ScrollViewer::new()
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
                .content(
                    Border::new()
                        .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
                        .content(page),
                ),
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

        let issue_badge = issue_projection_current
            .then(|| project_issues(&self.issues).counts.nav_badge_count())
            .flatten()
            .map(|count| count.to_string());
        let primary_nav = Page::ALL
            .into_iter()
            .map(|page| {
                KeyedView::new(
                    page.tag(),
                    nav_button(
                        palette,
                        page.icon(),
                        page.nav_label(),
                        page == self.page,
                        pane_expanded,
                        if page == Page::Issues {
                            issue_badge.as_deref()
                        } else {
                            None
                        },
                        context.message(Message::Navigate(Some(page.tag().to_string()))),
                        true,
                    ),
                )
            })
            .collect::<Vec<_>>();

        let tools_enabled = !self.diagnostic_results.is_empty() && self.export_pending.is_none();
        let tools_nav = [
            (FaIcon::FileExport, "Export Report", "export"),
            (FaIcon::ShareNodes, "Share to Forum", "share"),
        ]
        .into_iter()
        .map(|(symbol, label, tag)| {
            KeyedView::new(
                tag,
                nav_button(
                    palette,
                    symbol,
                    label,
                    false,
                    pane_expanded,
                    None,
                    context.message(Message::Navigate(Some(tag.to_string()))),
                    tools_enabled,
                ),
            )
        })
        .collect::<Vec<_>>();

        let pane_toggle: View = if rail_forced_collapsed {
            View::empty()
        } else {
            nav_button(
                palette,
                if pane_expanded {
                    FaIcon::AnglesLeft
                } else {
                    FaIcon::AnglesRight
                },
                if pane_expanded { "Collapse" } else { "Expand" },
                false,
                pane_expanded,
                None,
                context.message(Message::TogglePane),
                true,
            )
        };

        let pane_footer = StackPanel::new().spacing(2.0).children((
            nav_button(
                palette,
                FaIcon::Settings,
                "Settings",
                false,
                pane_expanded,
                None,
                context.message(Message::OpenSettings),
                true,
            ),
            nav_button(
                palette,
                FaIcon::CircleInfo,
                "About",
                false,
                pane_expanded,
                None,
                context.message(Message::OpenAbout),
                true,
            ),
            pane_toggle,
            machine_card(
                palette,
                pane_expanded,
                &self.system_info,
                self.architecture.as_ref(),
                self.system_error.as_deref(),
            ),
        ));

        let navigation_rail = Border::new()
            .grid_column(0)
            .padding(if pane_expanded {
                Thickness::new(14.0, 4.0, 12.0, 14.0)
            } else {
                Thickness::new(4.0, 4.0, 12.0, 14.0)
            })
            .content(
                Grid::new()
                    .rows([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        StackPanel::new().children((
                            nav_brand(palette, pane_expanded),
                            StackPanel::new().spacing(2.0).keyed_children(primary_nav),
                            nav_section("TOOLS", pane_expanded, palette),
                            StackPanel::new().spacing(2.0).keyed_children(tools_nav),
                        )),
                        Border::new().grid_row(2).content(pane_footer),
                    )),
            );

        let title_brand = Border::new()
            .grid_row(0)
            .padding(Thickness::new(16.0, 0.0, 0.0, 0.0))
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(9.0)
                    .children((
                        Image::new()
                            .source_data(EncodedImage::from_static(APP_BADGE))
                            .width(17.0)
                            .height(17.0),
                        TextBlock::new()
                            .text("WindowsForum Diagnostics")
                            .font_size(12.0)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .foreground(palette.muted)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
            );

        let title_bar = TitleBar::new()
            .grid_row(0)
            .height(42.0)
            .min_height(42.0)
            .max_height(42.0)
            .preferred_height(WindowTitleBarHeight::Standard)
            .title("")
            .subtitle("")
            .is_back_button_visible(false)
            .is_pane_toggle_button_visible(false);

        // The shell publishes frequently while telemetry and scans are live.
        // A closed palette must stay a zero-cost overlay: do not allocate its
        // specs, fuzzy-match them, or construct row controls until it opens.
        let palette_rows = if self.palette_open {
            let palette_matches =
                palette_visible_matches(self.palette_command_specs(), &self.palette_query);
            let palette_match_count = palette_matches.len();
            let active_palette_index = self
                .palette_active_index
                .min(palette_match_count.saturating_sub(1));
            let mut palette_rows = Vec::new();
            let mut previous_section = None;
            for (index, matched) in palette_matches.into_iter().enumerate() {
                let match_indices = matched.indices;
                let command = matched.command;
                if previous_section != Some(command.section) {
                    previous_section = Some(command.section);
                    palette_rows.push(KeyedView::new(
                        format!("palette-section:{}", command.section),
                        TextBlock::new()
                            .text(command.section.to_ascii_uppercase())
                            .margin(Thickness::new(12.0, 10.0, 12.0, 4.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .foreground(palette.muted)
                            .automation_heading_level(AutomationHeadingLevel::Level3),
                    ));
                }
                let tag = command.tag.into_owned();
                let execute = context.message(Message::PaletteCommand(tag.clone()));
                let activate = context.callback(move |_| Message::PaletteActiveChanged(index));
                let active = index == active_palette_index;
                let icon = command.icon;
                let label = command.label.into_owned();
                let automation_label = label.clone();
                let automation_id = format!("palette-item-{}", tag.replace(':', "-"));
                let enabled = command.enabled;
                let label_view =
                    command_palette_highlighted_label(palette, label, &match_indices, enabled);
                let shortcut: View = command.shortcut.map_or_else(View::empty, |shortcut| {
                    command_palette_key_chip(palette, shortcut.into_owned())
                });
                palette_rows.push(KeyedView::new(
                    tag,
                    Border::new()
                        .background(if active {
                            palette.active
                        } else {
                            Color::transparent()
                        })
                        .corner_radius(6.0)
                        .on_pointer_entered(activate)
                        .content(
                            Button::new()
                                .height(36.0)
                                .style(ButtonStyle::Subtle)
                                .horizontal_alignment(HorizontalAlignment::Stretch)
                                .horizontal_content_alignment(HorizontalAlignment::Stretch)
                                .vertical_content_alignment(VerticalAlignment::Center)
                                // Popup realization in WinUI 3 cannot repackage
                                // boxed Thickness/CornerRadius values from a local
                                // resource dictionary. Keep only brush overrides;
                                // the Subtle style supplies native geometry.
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", palette.active)
                                        .set("ButtonBackgroundPressed", palette.active)
                                        .set(
                                            "ButtonForeground",
                                            if active {
                                                palette.accent
                                            } else {
                                                palette.muted
                                            },
                                        )
                                        .set("ButtonForegroundDisabled", palette.muted),
                                )
                                .is_enabled(enabled)
                                .automation_name(automation_label)
                                .automation_id(automation_id)
                                .element_ref(&self.palette_result_references[index])
                                .on_click(execute)
                                .content(
                                    Grid::new()
                                        .columns([
                                            GridLength::Pixel(22.0),
                                            GridLength::Star(1.0),
                                            GridLength::Pixel(88.0),
                                        ])
                                        .column_spacing(8.0)
                                        .children((
                                            icons::path(icon)
                                                .width(14.0)
                                                .height(14.0)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            label_view,
                                            Border::new()
                                                .grid_column(2)
                                                .horizontal_alignment(HorizontalAlignment::Right)
                                                .vertical_alignment(VerticalAlignment::Center)
                                                .content(shortcut),
                                        )),
                                ),
                        ),
                ));
            }
            if palette_rows.is_empty() {
                palette_rows.push(KeyedView::new(
                    "palette-empty",
                    TextBlock::new()
                        .text("No matching commands")
                        .margin(Thickness::new(16.0, 30.0, 16.0, 30.0))
                        .font_size(13.0)
                        .foreground(palette.muted)
                        .horizontal_alignment(HorizontalAlignment::Center),
                ));
            }
            palette_rows
        } else {
            Vec::new()
        };
        let palette_open = self.palette_open;
        let palette_epoch = self.palette_dialog_epoch;
        let palette_query_reference = self.palette_query_reference.clone();
        context.use_effect(
            "command-palette-focus",
            (palette_open, palette_epoch),
            move || {
                if palette_open {
                    let _ = palette_query_reference.request_focus();
                }
                None
            },
        );
        // Keep the palette in a permanently mounted ContentDialog. Reactor's
        // dialog lifecycle is designed for live open/close transitions; a
        // conditional Grid subtree can fail a native Children insertion.
        // Expensive result rows are still created only while it is open.
        let query_changed = context.callback(Message::PaletteQueryChanged);
        let palette_width = (self.window_size.width - 48.0).clamp(360.0, 560.0);
        let palette_list_height = (self.window_size.height * 0.60 - 94.0).clamp(220.0, 430.0);
        let close_palette = context.message(Message::ClosePalette);
        let palette_dialog = ContentDialog::new()
            .is_open(self.palette_open)
            .on_closed(move |_| {
                let _ = close_palette.call(());
            })
            .content(
                Border::new().width(palette_width).content(
                    Grid::new()
                        .automation_name("Command palette")
                        .rows([
                            GridLength::Pixel(54.0),
                            GridLength::Auto,
                            GridLength::Pixel(39.0),
                        ])
                        .children((
                            Border::new()
                                .padding(Thickness::new(16.0, 7.0, 13.0, 7.0))
                                .border_brush(palette.border)
                                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                                .content(
                                    Grid::new()
                                        .columns([
                                            GridLength::Pixel(19.0),
                                            GridLength::Star(1.0),
                                            GridLength::Pixel(42.0),
                                        ])
                                        .column_spacing(9.0)
                                        .children((
                                            icons::path(FaIcon::MagnifyingGlass)
                                                .width(14.0)
                                                .height(14.0)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBox::new()
                                                .grid_column(1)
                                                .height(40.0)
                                                .text(self.palette_query.clone())
                                                .automation_name("Search commands")
                                                .placeholder_text(
                                                    "Search commands, screens and diagnostics…",
                                                )
                                                .background(Color::transparent())
                                                .border_thickness(Thickness::uniform(0.0))
                                                .on_text_changed(query_changed)
                                                .element_ref(&self.palette_query_reference),
                                            Border::new()
                                                .grid_column(2)
                                                .horizontal_alignment(HorizontalAlignment::Right)
                                                .vertical_alignment(VerticalAlignment::Center)
                                                .content(command_palette_key_chip(palette, "Esc")),
                                        )),
                                ),
                            ScrollViewer::new()
                                .grid_row(1)
                                .max_height(palette_list_height)
                                .margin(Thickness::uniform(6.0))
                                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                                .content(
                                    StackPanel::new().spacing(2.0).keyed_children(palette_rows),
                                ),
                            Border::new()
                                .grid_row(2)
                                .padding(Thickness::xy(14.0, 0.0))
                                .border_brush(palette.border)
                                .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                                .content(command_palette_footer(palette, palette_width >= 450.0)),
                        )),
                ),
            );
        let shortcut_rows: &[(&str, &str)] = &[
            ("Ctrl+K", "Open the command palette"),
            ("Ctrl+1 … Ctrl+6", "Switch between screens"),
            ("Ctrl+Shift+Q", "Run a Quick Scan"),
            ("Ctrl+Shift+F", "Run a Full Scan"),
            ("Ctrl+R", "Refresh"),
            ("Ctrl+/", "Show this shortcut list"),
            ("Esc", "Close dialogs and overlays"),
        ];
        // Keep the dialog node mounted and toggle its native open property.
        // Inserting a late ContentDialog after several empty overlay siblings
        // can desynchronize the current windows-reactor child index; a stable
        // node also preserves clean row geometry across repeated opens.
        let close_shortcuts = context.message(Message::CloseShortcutHelp);
        let shortcut_dialog = ContentDialog::new()
            .title("Keyboard Shortcuts")
            .is_open(self.shortcut_help_open)
            .close_button_text("Close")
            .on_closed(move |_| {
                let _ = close_shortcuts.call(());
            })
            .content(
                Border::new()
                    .width(400.0)
                    .background(palette.card_strong)
                    .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                    .content(
                        StackPanel::new().spacing(6.0).keyed_children(
                            shortcut_rows
                                .iter()
                                .map(|(keys, description)| {
                                    KeyedView::new(
                                        *keys,
                                        Grid::new()
                                            .min_height(28.0)
                                            .columns([GridLength::Star(1.0), GridLength::Auto])
                                            .column_spacing(16.0)
                                            .children((
                                                TextBlock::new()
                                                    .text((*description).to_string())
                                                    .font_size(13.0)
                                                    .text_wrapping(TextWrapping::Wrap)
                                                    .vertical_alignment(VerticalAlignment::Center),
                                                TextBlock::new()
                                                    .grid_column(1)
                                                    .text((*keys).to_string())
                                                    .font_size(12.0)
                                                    .font_weight(FontWeight::SEMI_BOLD)
                                                    .foreground(palette.muted)
                                                    .vertical_alignment(VerticalAlignment::Center),
                                            )),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        ),
                    ),
            );

        let title_settings = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 144.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(context.message(Message::OpenSettings))
            .automation_name("Open Settings")
            .content(icons::path(FaIcon::Settings));

        let title_palette = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 190.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(context.message(Message::TogglePalette))
            .element_ref(&self.palette_button_reference)
            .automation_name("Open the command palette")
            .content(fa_icon_label(FaIcon::MagnifyingGlass, ""));
        let title_help = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 236.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(context.message(Message::ShowShortcutHelp))
            .automation_name("Keyboard shortcuts")
            .content(icons::path(FaIcon::CircleInfo));
        let (theme_icon, theme_automation_name) = if effective_theme == WindowTheme::Dark {
            (FaIcon::Sun, "Switch to light theme")
        } else {
            (FaIcon::Moon, "Switch to dark theme")
        };
        let title_theme = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 282.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(context.message(Message::ToggleTheme))
            .automation_name(theme_automation_name)
            .content(icons::path(theme_icon));
        let title_actions = Grid::new().grid_row(0).children((
            title_theme,
            title_help,
            title_palette,
            title_settings,
        ));

        let body = Grid::new()
            .grid_row(1)
            .columns([
                GridLength::Pixel(if pane_expanded { 230.0 } else { 64.0 }),
                GridLength::Star(1.0),
            ])
            .children((navigation_rail, content_panel));

        let light_wallpaper = Border::new()
            .grid_row_span(2)
            .opacity(if effective_theme == WindowTheme::Light {
                1.0
            } else {
                0.0
            })
            .opacity_transition(std::time::Duration::from_millis(500))
            .content(
                Image::new()
                    .source_data(EncodedImage::from_static(WALLPAPER_LIGHT))
                    .stretch(Stretch::UniformToFill),
            );
        let dark_wallpaper = Border::new()
            .grid_row_span(2)
            .opacity(if effective_theme == WindowTheme::Light {
                0.0
            } else {
                1.0
            })
            .opacity_transition(std::time::Duration::from_millis(500))
            .content(
                Image::new()
                    .source_data(EncodedImage::from_static(WALLPAPER_DARK))
                    .stretch(Stretch::UniformToFill),
            );

        let settings_status = if self.settings_loading {
            Some(("Loading settings…".to_string(), false))
        } else if self.settings_saving {
            Some(("Saving settings…".to_string(), false))
        } else if let Some(error) = self.settings_save_error.as_ref() {
            Some((format!("Settings were not saved: {error}"), true))
        } else {
            self.settings_error.as_ref().map(|error| {
                (
                    format!("Settings persistence is unavailable: {error}"),
                    true,
                )
            })
        };
        let settings_editable =
            !self.settings_loading && !self.settings_saving && !self.subscription_install_busy;
        let settings_can_save =
            settings_editable && (self.deterministic_visual || self.app.is_some());
        let settings_phi_gate =
            phi_preference_gate(self.ai_provider_status.as_ref(), self.ai_status_loading);
        let settings: View = if self.settings_open {
            let epoch = self.settings_dialog_epoch;
            let subscription_provider =
                subscription_auth_provider_for_setup(self.provider_setup_index)
                    .unwrap_or(SubscriptionAuthProvider::Codex);
            let subscription_state = subscription_auth_provider_for_setup(
                self.provider_setup_index,
            )
            .and_then(|provider| {
                self.subscription_auth_states
                    .get(subscription_auth_state_index(provider))
            });
            let subscription_install_active = self.subscription_install_busy
                && subscription_auth_provider_for_setup(self.provider_setup_index)
                    == Some(subscription_provider);
            settings_dialog(
                palette,
                effective_theme,
                self.visual_state == VisualState::SettingsBottom,
                &self.settings_draft,
                &settings_phi_gate,
                self.deterministic_visual,
                self.provider_setup_index,
                self.provider_catalogs.get(self.provider_setup_index),
                subscription_state,
                self.subscription_auth_error.as_deref(),
                subscription_install_active,
                self.subscription_install_progress.as_ref(),
                self.subscription_install_error.as_deref(),
                settings_editable,
                settings_can_save,
                self.settings_saving,
                settings_status,
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ThemeSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ExportFormatSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::AutoSaveChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::NotificationsChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ScanOnStartupChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::CloseToTrayChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::MaxConcurrentTasksChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::AiEnabledChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::PreferredAiProviderSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::CloudFallbackSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::NetworkGroundingChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::CodexCliPathChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::CodexModelSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ProviderSetupSelectionChanged(value),
                }),
                context.callback(move |value| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ProviderModelSelectionChanged(value),
                }),
                context.message(Message::RefreshProviderModels),
                context.message(Message::CancelProviderModels),
                context.message(Message::RefreshSubscriptionAuth(subscription_provider)),
                context.message(Message::StartSubscriptionSignIn(subscription_provider)),
                context.message(Message::StartSubscriptionSignOut(subscription_provider)),
                context.message(Message::CancelSubscriptionAuth),
                context.message(Message::RequestSubscriptionInstall(subscription_provider)),
                context.message(Message::CancelSubscriptionInstall),
                context.callback(move |(field, value)| Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::ProviderTextChanged(field, value),
                }),
                context.message(Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::Cancel,
                }),
                context.message(Message::SettingsDialog {
                    epoch,
                    action: SettingsDialogAction::Save,
                }),
                &self.provider_key_drafts,
                [
                    self.settings_snapshot.open_ai_api_key_set,
                    self.settings_snapshot.anthropic_api_key_set,
                    self.settings_snapshot.gemini_api_key_set,
                    self.settings_snapshot.deepseek_api_key_set,
                    self.settings_snapshot.custom_api_key_set,
                ],
                self.provider_key_busy,
                context
                    .callback(move |(index, value)| Message::ProviderKeyDraftChanged(index, value)),
                context.callback(Message::StoreProviderKey),
                context.callback(Message::ClearProviderKey),
                &self.diagnostic_catalog,
                context.callback(Message::ToggleQuickScanTask),
            )
        } else {
            View::empty()
        };

        let update_notice: View = if self.update_notice_visible {
            self.update_info
                .as_ref()
                .map_or_else(View::empty, |update| {
                    let epoch = self.update_notice_epoch;
                    Border::new()
                        .grid_row_span(2)
                        .width(430.0)
                        .margin(Thickness::new(0.0, 0.0, 0.0, 28.0))
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Bottom)
                        .on_pointer_entered(
                            context
                                .callback(move |_| Message::UpdateNoticePointerEntered { epoch }),
                        )
                        .on_pointer_exited(
                            context.callback(move |_| Message::UpdateNoticePointerExited { epoch }),
                        )
                        .content(
                            InfoBar::new()
                                .title(format!("Update available: v{}", update.version))
                                .message("Open About to download the new version")
                                .severity(InfoBarSeverity::Informational)
                                .is_open(true)
                                .is_closable(true)
                                .on_closed(context.message(Message::UpdateNoticeClosed { epoch })),
                        )
                })
        } else {
            View::empty()
        };

        let about_epoch = self.about_dialog_epoch;
        let about_is_open = self.about_open;
        let about_close_reference = self.about_close_reference.clone();
        context.use_effect(
            "about-header-close-focus",
            (about_is_open, about_epoch),
            move || {
                if about_is_open {
                    let _ = about_close_reference.request_focus();
                }
                None
            },
        );
        let about_scrim: View = if self.about_open {
            // ContentDialog supplies the modal surface and its own light-dismiss
            // layer. The Store 2.5.8 React modal is about 21% darker at matched
            // wallpaper samples, so add the measured residual opacity behind
            // the native popup. Reactor has no public per-element backdrop-blur
            // projection at the pinned revision.
            Border::new()
                .grid_row_span(2)
                .background(Color::argb(52, 0, 0, 0))
                .into()
        } else {
            View::empty()
        };
        let about = about_dialog(
            palette,
            self.about_open,
            &self.about_close_reference,
            self.update_info.as_ref(),
            self.about_action_error.as_deref(),
            self.about_launch_task.is_none(),
            context.callback(move |_| Message::AboutClosed { epoch: about_epoch }),
            context.message(Message::AboutExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::DownloadUpdate,
            }),
            context.message(Message::AboutExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::WindowsForum,
            }),
            context.message(Message::AboutExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::GithubRepository,
            }),
            context.message(Message::AboutClosed { epoch: about_epoch }),
        );

        let action_review_dialog = if let Some(proposal) = self.action_review.as_ref() {
            let presentation = action_review_presentation(proposal, self.is_admin);
            let mut preview = format!(
                "Review the exact, catalog-backed action{}. This approval expires after 10 minutes and can be used only once.\n",
                if presentation.batch { "s" } else { "" }
            );
            for action in &proposal.actions {
                preview.push('\n');
                preview.push_str(&action.remediation.label);
                preview.push_str(" — ");
                preview.push_str(&action.remediation.description);
                for step in &action.steps {
                    preview.push_str("\n  · ");
                    preview.push_str(step);
                }
            }
            if presentation.admin_blocked {
                preview.push_str(
                    "\n\nThis action needs administrator rights. Restart the app as administrator first.",
                );
            } else if proposal
                .actions
                .iter()
                .any(|action| action.remediation.admin_required)
            {
                preview.push_str("\n\nRuns with administrator rights.");
            }
            if presentation.schedules_restart {
                preview.push_str(
                    "\n\nSave your work first. Windows will restart 60 seconds after approval; run shutdown /a to cancel.",
                );
            } else if presentation.requires_restart {
                preview.push_str("\n\nA restart is required for this change to take effect.");
            }
            if presentation.long_running {
                preview.push_str(
                    "\n\nThis can take 10–30 minutes. Keep the app open until it finishes.",
                );
                preview.push_str(if presentation.can_stop {
                    " You can stop it safely."
                } else {
                    " It cannot be stopped safely once it starts."
                });
            }
            let proposal_id = proposal.proposal_id.clone();
            let on_closed = context.callback(move |result| Message::ActionReviewDialogClosed {
                proposal_id: proposal_id.clone(),
                result,
            });
            ContentDialog::new()
                .title(presentation.title)
                .is_open(true)
                .primary_button_text(presentation.primary_label)
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(438.0)
                        .background(palette.card_strong)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .content(
                            TextBlock::new()
                                .text(preview)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        let repair_dialog = if let Some(proposal) = self.repair_confirm.as_ref() {
            // This second, explicit gate is reached only after the broker has
            // revalidated the immutable preview and identified a Repair tier.
            let presentation = action_review_presentation(proposal, self.is_admin);
            let mut preview = String::from("Confirm the exact repair steps below:\n");
            for action in &proposal.actions {
                for step in &action.steps {
                    preview.push_str("\n· ");
                    preview.push_str(step);
                }
            }
            if presentation.schedules_restart {
                preview.push_str(
                    "\n\nSave your work first. Windows will restart 60 seconds after approval; run shutdown /a to cancel.",
                );
            } else if presentation.requires_restart {
                preview.push_str("\n\nA restart is required for this change to take effect.");
            }
            let proposal_id = proposal.proposal_id.clone();
            let on_closed = context.callback(move |result| Message::RepairDialogClosed {
                proposal_id: proposal_id.clone(),
                result,
            });
            ContentDialog::new()
                .title(presentation.title)
                .is_open(true)
                .primary_button_text(if presentation.schedules_restart {
                    "Schedule restart"
                } else {
                    "Run repair once"
                })
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(412.0)
                        .background(palette.card_strong)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .content(
                            TextBlock::new()
                                .text(preview)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        let subscription_install_dialog = if let Some(prompt) = self.subscription_install_prompt {
            let (title, primary, body) = match prompt {
                InstallPrompt::Winget { provider } => {
                    let cli = match provider {
                        SubscriptionAuthProvider::Codex => "OpenAI Codex CLI",
                        SubscriptionAuthProvider::ClaudeCode => "Anthropic Claude Code CLI",
                    };
                    (
                        format!("Install {cli}?"),
                        "Install with winget",
                        format!(
                            "WFDiag will ask Windows Package Manager to install the official {cli} package. The installer runs only after this confirmation. After installation, WFDiag verifies the absolute executable path and does not sign in automatically."
                        ),
                    )
                }
                InstallPrompt::VendorFallback { provider, reason } => {
                    let cli = match provider {
                        SubscriptionAuthProvider::Codex => "OpenAI Codex CLI",
                        SubscriptionAuthProvider::ClaudeCode => "Anthropic Claude Code CLI",
                    };
                    let reason = match reason {
                        SubscriptionInstallFallbackReason::WingetUnavailable => {
                            "Windows Package Manager is unavailable"
                        }
                        SubscriptionInstallFallbackReason::WingetFailed => {
                            "Windows Package Manager could not complete the installation"
                        }
                        SubscriptionInstallFallbackReason::ExplicitApprovalMissing => {
                            "the vendor fallback has not been approved"
                        }
                    };
                    (
                        "Run the vendor PowerShell installer?".to_string(),
                        "Run vendor installer",
                        format!(
                            "{reason}. This is a separate approval to download and execute the current {cli} PowerShell bootstrap maintained by the vendor. Vendor scripts can change over time. WFDiag contains the process tree, applies a timeout, verifies the installed executable, and still does not sign in automatically."
                        ),
                    )
                }
            };
            let on_closed = context.callback(move |result| {
                Message::SubscriptionInstallPromptClosed { prompt, result }
            });
            ContentDialog::new()
                .title(title)
                .is_open(true)
                .primary_button_text(primary)
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(430.0)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .background(palette.card_strong)
                        .content(
                            TextBlock::new()
                                .text(body)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        Grid::new()
            .rows([GridLength::Pixel(42.0), GridLength::Star(1.0)])
            // Keep Reactor's native accelerators for Ctrl+R and the numpad
            // aliases. The isolated window subclass supplies the shipping
            // main-row/K/slash/Shift chords through the component watcher.
            .key_accelerators(KeyAccelerators::new([
                KeyAccelerator::new(
                    AcceleratorKey::R,
                    AcceleratorModifiers::Control,
                    context.message(Message::Refresh),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad1,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("diagnostics".to_string()))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad2,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("monitor".to_string()))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad3,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("processes".to_string()))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad4,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("ai".to_string()))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad5,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("issues".to_string()))),
                ),
                KeyAccelerator::new(
                    AcceleratorKey::NumberPad6,
                    AcceleratorModifiers::Control,
                    context.message(Message::Navigate(Some("history".to_string()))),
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
