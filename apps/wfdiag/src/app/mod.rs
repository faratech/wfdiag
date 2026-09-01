//! The root component: its state fields and the Reactor `Component` impl.

#![deny(unsafe_code)]

pub(crate) mod consts;
pub(crate) mod message;
pub(crate) mod orchestration;
pub(crate) mod policy;
pub(crate) mod state;
pub(crate) mod tasks;

use crate::ai::chat_tools::NativeChatRuntime;
use crate::app::consts::{
    APP_BADGE, APP_VERSION, PROCESS_LIVE_REFRESH_INTERVAL, PROCESS_PAGE_SIZE,
    PROVIDER_SETUP_LABELS, STATUS_INFO_DARK, STATUS_INFO_LIGHT, STATUS_OK_DARK, STATUS_OK_LIGHT,
    STATUS_WARN_DARK, STATUS_WARN_LIGHT, WALLPAPER_DARK, WALLPAPER_LIGHT,
};
use crate::app::message::{
    ActionReviewSurface, HistoryAckKind, Message, PaletteFocusAction, SettingsDialogAction,
};
use crate::app::policy::{
    AiWorkerKind, ReactorPackageIdentitySource, StartupScanGate, action_proposal_contains_repair,
    action_proposal_matches_snapshot, apply_startup_scan_preference,
    authoritative_result_set_is_complete, authoritative_ui_results,
    configured_provider_setup_index, diagnostics_uses_compact_layout, effective_window_theme,
    export_format_label, export_task_catalog, history_comparison_refresh_target,
    history_label_draft_for_selection, history_retention_tuple, history_tag_draft_for_selection,
    history_task_catalog, history_task_diff_result_is_current, history_trends_baseline_changed,
    load_live_test_settings, navigation_rail_forced_collapsed, pending_export_write_is_current,
    pending_system_info, phi_preference_gate, privilege_label,
    provider_models_auto_discovery_allowed, reactor_ai_provider_runtime, reactor_settings_service,
    scan_kind_label, shared_diagnostic_executor, subscription_auth_provider_for_setup,
    subscription_auth_state_index, update_notice_timer_callback_is_current,
    window_theme_from_setting,
};
use crate::app::state::{
    AiMode, AiPreparationUi, ChatAttempt, ChatDisplayMessage, CloudFallbackConsent,
    DiagnosticAnalysisDisplay, DiagnosticScanPolicy, DiagnosticSnapshot, ExportWriteKind,
    FullScanConsent, HistoryRetentionPolicy, HistoryTaskDiffProjection, IssuePrioritizationDisplay,
    MonitorHistory, Page, PendingActionApproval, PendingAiIntent, PendingCloudFallbackPolicyUpdate,
    PendingDiagnosticAnalysis, PendingExport, PendingExportAction, PendingFixPlan,
    PendingIssuePrioritization, PendingProviderCatalogRequest, PendingProviderKeyChange,
    PendingSettingsSave, PendingSubscriptionAuth, PendingSubscriptionInstall,
    ProviderCatalogUiState, SubscriptionAuthUiState, SubscriptionInstallPrompt,
    TargetedDiagnosticOverlay,
};
use crate::app::tasks::{
    spawn_export_file_write, spawn_instance_watch, spawn_support_package_write, spawn_system_wait,
    spawn_update_delay,
};
use crate::dialogs::about::about_dialog;
use crate::dialogs::action_review::action_review_presentation;
use crate::dialogs::palette::{
    PALETTE_MAX_RESULTS, command_palette_footer, command_palette_highlighted_label,
    command_palette_key_chip, palette_visible_matches,
};
use crate::dialogs::settings::settings_dialog;
use crate::fixtures;
use crate::fixtures::knobs::{initial_window_dimension, live_test_fixture_from_env};
use crate::fixtures::visual::{
    LiveTestFixture, VisualState, fixture_258_system_info, fixture_monitor_empty_stats,
    fixture_system_stats, remediation_partial_visual_run,
};
use crate::platform::external::{
    launch_email_compose_draft, launch_export_external_action, write_text_to_clipboard,
};
use crate::platform::{focus, instance, ui_wake, window};
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
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};
use wfdiag_native_ai_analysis::{
    AnalysisWorkerEvent, FixPlanRoute, FixPlanWorkerEvent, NativeAnalysisRuntime,
    NativeFixPlanRuntime, ValidatedFixPlan,
};
use wfdiag_native_ai_chat::workers::provider_setup::{
    ProviderSetupRuntime, ProviderSetupWorkerEvent,
};
use wfdiag_native_ai_chat::workers::subscription_auth::{
    SubscriptionAuthRuntime, SubscriptionAuthWorkerEvent,
};
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallFallbackReason, SubscriptionInstallProgress, SubscriptionInstallRuntime,
    SubscriptionInstallWorkerEvent,
};
use wfdiag_native_ai_chat::{
    ChatWorkerEvent, ProviderUse, SubscriptionAuthOperation, SubscriptionAuthProvider,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderStatus, FoundryCliEndpointSource, NativeAiProviderRuntime,
    PackageIdentitySource, ProviderPreferenceSettingsValidator, ReqwestOllamaSource, SharedAiCache,
    next_auto_local_route,
};
use wfdiag_native_ai_report::{NativeReportRuntime, ReportWorkerEvent};
use wfdiag_native_diagnostics::{
    DiagnosticOutput, DiagnosticRuntime, DiagnosticTask, NativeDiagnosticRuntime, ScanKind,
    SharedScanEvidence,
};
use wfdiag_native_export::{ExportCompleted, ExportExternalAction, ExportPayload, ExportRuntime};
use wfdiag_native_history::{
    ComparisonSummary, HistoryRuntimeConfig, NativeHistoryRuntime, ScanStorage, ScanSummary,
    TaskTrend,
};
use wfdiag_native_issues::projection::{
    PendingIssueDetection, canonical_issue_metadata_snapshot, issue_projection_matches_evidence,
    project_issues,
};
use wfdiag_native_issues::{Issue, IssueDetectionCompleted, IssueRuntime, RemediationSummary};
use wfdiag_native_monitor::{
    MonitorProfile, NativeMonitorRuntime, NetworkConnection, ProcessPage, ProcessSortDirection,
    ProcessSortKey,
};
use wfdiag_native_projection::process_identity::{ProcessIdentity, reconcile_process_selection_by};
use wfdiag_native_remediation::broker::{ActionApproval, ActionProposal};
use wfdiag_native_remediation::remediation;
use wfdiag_native_remediation::runtime::{
    ActionRunEvent, ActionRunSummary, ActionWorkerEvent, NativeActionRuntime,
};
use wfdiag_native_settings::{
    AppSettings, CloudFallbackPolicy, ProviderCredentialTransaction, ProviderKeyId,
    SettingsCommand, SettingsEvent, SettingsRuntime, SettingsService,
};
use wfdiag_native_system::{
    ArchitectureSnapshot, SystemCompleted, SystemInfo, SystemRequest, SystemRequestKind,
    SystemRuntime,
};
use wfdiag_native_update::policy::AboutExternalAction;
use wfdiag_native_update::{NativeUpdateRuntime, UpdateInfo};
use wfdiag_ui_core::{
    DiagnosticTaskResult, SystemStats, TaskProgressStatus, UiEvent, UiEventReceiver, UiWakeHandler,
};
use windows_reactor::*;

pub(crate) struct WfdiagShell {
    page: Page,
    live_test_fixture: Option<LiveTestFixture>,
    theme: WindowTheme,
    effective_color_scheme: ColorScheme,
    window_size: WindowSize,
    requested_client_width: f64,
    requested_client_height: f64,
    pane_open: bool,
    about_open: bool,
    about_close_reference: ElementRef<Button>,
    about_dialog_epoch: u64,
    about_action_error: Option<String>,
    about_launch_task: Option<ComponentTask>,
    update_runtime: Option<NativeUpdateRuntime>,
    update_delay_task: Option<ComponentTask>,
    update_check_task: Option<ComponentTask>,
    update_info: Option<UpdateInfo>,
    update_notice_visible: bool,
    update_notice_epoch: u64,
    update_notice_timer_generation: u64,
    update_notice_task: Option<ComponentTask>,
    update_notice_started_at: Option<Instant>,
    update_notice_remaining: Duration,
    settings_open: bool,
    settings_runtime: Option<SettingsRuntime>,
    settings_receiver: Option<Arc<Mutex<mpsc::Receiver<SettingsEvent>>>>,
    settings_wait: Option<ComponentTask>,
    settings_snapshot: AppSettings,
    settings_draft: AppSettings,
    settings_dialog_epoch: u64,
    settings_request_id: u64,
    settings_load_request_id: Option<u64>,
    settings_pending_save: Option<PendingSettingsSave>,
    settings_loading: bool,
    settings_saving: bool,
    settings_error: Option<String>,
    settings_save_error: Option<String>,
    startup_scan_gate: StartupScanGate,
    system_runtime: Option<SystemRuntime>,
    system_receiver: Option<Arc<Mutex<mpsc::Receiver<SystemCompleted>>>>,
    system_wait: Option<ComponentTask>,
    system_info_request_id: Option<u64>,
    architecture_request_id: Option<u64>,
    system_info: SystemInfo,
    architecture: Option<ArchitectureSnapshot>,
    system_error: Option<String>,
    issues: Vec<Issue>,
    issue_maintenance: Vec<RemediationSummary>,
    issue_runtime: Option<IssueRuntime>,
    issue_receiver: Option<Arc<Mutex<mpsc::Receiver<IssueDetectionCompleted>>>>,
    issue_wait: Option<ComponentTask>,
    issue_prepare_task: Option<ComponentTask>,
    issue_request_id: u64,
    issue_committed_epoch: u64,
    issue_source_session_id: Option<String>,
    issue_source_results: Option<SharedScanEvidence>,
    issue_projected_epoch: Option<u64>,
    issue_projected_session_id: Option<String>,
    issue_pending: Option<PendingIssueDetection>,
    issue_enqueued_request_id: Option<u64>,
    issue_error: Option<String>,
    monitoring_paused: bool,
    monitoring_paused_by_lifecycle: bool,
    process_filter: String,
    process_page: Option<ProcessPage>,
    process_sort_key: ProcessSortKey,
    process_sort_direction: ProcessSortDirection,
    process_offset: usize,
    process_request_id: u64,
    process_request_task: Option<ComponentTask>,
    process_last_refresh_started_at: Option<Instant>,
    process_loading: bool,
    process_error: Option<String>,
    selected_process: Option<ProcessIdentity>,
    history_runtime: Option<Arc<NativeHistoryRuntime>>,
    history_retention_policy: Arc<RwLock<HistoryRetentionPolicy>>,
    history_summaries: Vec<ScanSummary>,
    history_filter: String,
    selected_history_id: Option<String>,
    history_comparison: Option<ComparisonSummary>,
    history_request_id: u64,
    history_request_task: Option<ComponentTask>,
    history_compare_request_id: u64,
    history_compare_task: Option<ComponentTask>,
    history_expanded_task_id: Option<String>,
    history_task_diff: Option<HistoryTaskDiffProjection>,
    history_task_diff_error: Option<String>,
    history_task_diff_request_id: u64,
    history_task_diff_task: Option<ComponentTask>,
    history_loading: bool,
    history_error: Option<String>,
    history_comparison_error: Option<String>,
    chat_input: String,
    chat_composer_reference: ElementRef<TextBox>,
    chat_focus_revision: u64,
    chat_answer: Option<String>,
    ai_worker_settings: Option<SettingsService>,
    ai_worker_cache: SharedAiCache,
    ai_worker_policy: std::ffi::OsString,
    chat_runtime: Option<NativeChatRuntime>,
    chat_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<ChatWorkerEvent>>>>,
    chat_wait: Option<ComponentTask>,
    chat_request_id: u64,
    chat_pending: Option<u64>,
    chat_attempt: Option<ChatAttempt>,
    chat_last_prompt: Option<String>,
    full_scan_consent: Option<FullScanConsent>,
    cloud_fallback_consent: Option<CloudFallbackConsent>,
    cloud_fallback_policy_update: Option<PendingCloudFallbackPolicyUpdate>,
    chat_messages: Vec<ChatDisplayMessage>,
    report_runtime: Option<NativeReportRuntime>,
    report_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<ReportWorkerEvent>>>>,
    report_wait: Option<ComponentTask>,
    report_prepare_task: Option<ComponentTask>,
    report_request_id: u64,
    report_pending: Option<u64>,
    report_text: Option<String>,
    report_provider: Option<String>,
    report_provider_use: Option<ProviderUse>,
    report_source_session_id: Option<String>,
    report_error: Option<String>,
    pending_ai_intent: Option<PendingAiIntent>,
    pending_ai_preparation_error: Option<String>,
    fix_plan_runtime: Option<NativeFixPlanRuntime>,
    fix_plan_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<FixPlanWorkerEvent>>>>,
    fix_plan_wait: Option<ComponentTask>,
    fix_plan_request_id: u64,
    fix_plan_pending: Option<PendingFixPlan>,
    fix_plan: Option<ValidatedFixPlan>,
    fix_plan_error: Option<String>,
    action_runtime: Option<NativeActionRuntime>,
    action_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<ActionWorkerEvent>>>>,
    action_wait: Option<ComponentTask>,
    action_run_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<ActionRunEvent>>>>,
    action_run_wait: Option<ComponentTask>,
    action_active_run: Option<ActionRunSummary>,
    action_run_history: Vec<ActionRunSummary>,
    action_expanded_runs: HashSet<String>,
    action_request_id: u64,
    action_pending: Option<u64>,
    action_pending_approval: Option<PendingActionApproval>,
    action_review: Option<ActionProposal>,
    repair_confirm: Option<ActionProposal>,
    admin_relaunch_task: Option<ComponentTask>,
    instance_wait: Option<ComponentTask>,
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
    provider_key_drafts: [String; ProviderKeyId::ALL.len()],
    provider_credential_transaction: ProviderCredentialTransaction,
    provider_key_busy: bool,
    provider_key_pending: Option<PendingProviderKeyChange>,
    provider_setup_index: usize,
    provider_setup_runtime: Option<ProviderSetupRuntime>,
    provider_setup_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<ProviderSetupWorkerEvent>>>>,
    provider_setup_wait: Option<ComponentTask>,
    provider_setup_error: Option<String>,
    provider_catalogs: Vec<ProviderCatalogUiState>,
    provider_catalog_request_id: u64,
    provider_catalog_pending: Option<PendingProviderCatalogRequest>,
    provider_catalog_refresh_revision: u64,
    provider_catalog_refresh_task: Option<ComponentTask>,
    provider_catalog_refresh_after_cancel: bool,
    subscription_auth_runtime: Option<SubscriptionAuthRuntime>,
    subscription_auth_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<SubscriptionAuthWorkerEvent>>>>,
    subscription_auth_wait: Option<ComponentTask>,
    subscription_auth_error: Option<String>,
    subscription_auth_states: Vec<SubscriptionAuthUiState>,
    subscription_auth_operation_id: u64,
    subscription_auth_pending: Option<PendingSubscriptionAuth>,
    subscription_install_runtime: Option<SubscriptionInstallRuntime>,
    subscription_install_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<SubscriptionInstallWorkerEvent>>>>,
    subscription_install_wait: Option<ComponentTask>,
    subscription_install_request_id: u64,
    subscription_install_pending: Option<PendingSubscriptionInstall>,
    subscription_install_prompt: Option<SubscriptionInstallPrompt>,
    subscription_install_progress: Option<SubscriptionInstallProgress>,
    subscription_install_error: Option<String>,
    history_clear_confirm: bool,
    history_label_draft: String,
    history_label_editing: bool,
    history_tag_draft: String,
    history_ack_busy: bool,
    history_wait: Option<ComponentTask>,
    history_trends: Option<Vec<TaskTrend>>,
    history_trends_loading: bool,
    history_trends_error: Option<String>,
    history_trends_request_id: u64,
    history_trends_baseline_id: Option<String>,
    selected_result_task_id: Option<String>,
    diagnostic_filter: String,
    diagnostic_raw_output: bool,
    analysis_runtime: Option<NativeAnalysisRuntime>,
    analysis_receiver: Option<Arc<Mutex<std::sync::mpsc::Receiver<AnalysisWorkerEvent>>>>,
    analysis_wait: Option<ComponentTask>,
    analysis_request_id: u64,
    analysis_pending: Option<PendingDiagnosticAnalysis>,
    diagnostic_analyses: HashMap<String, DiagnosticAnalysisDisplay>,
    issue_prioritization_pending: Option<PendingIssuePrioritization>,
    issue_prioritization: IssuePrioritizationDisplay,
    network_connections: Option<Vec<NetworkConnection>>,
    network_loading: bool,
    ai_mode: AiMode,
    ai_provider_runtime: Option<NativeAiProviderRuntime>,
    ai_provider_status: Option<AIProviderStatus>,
    ai_settings_ready: bool,
    ai_status_request_id: u64,
    ai_status_task: Option<ComponentTask>,
    ai_status_loading: bool,
    ai_status_error: Option<String>,
    export_runtime: Option<ExportRuntime>,
    export_receiver: Option<Arc<Mutex<mpsc::Receiver<ExportCompleted>>>>,
    export_wait: Option<ComponentTask>,
    export_request_id: u64,
    export_pending: Option<PendingExport>,
    export_error: Option<String>,
    status: String,
    diagnostic_results: Vec<DiagnosticTaskResult>,
    previous_diagnostic_snapshot: Option<DiagnosticSnapshot>,
    targeted_diagnostic_overlay: Option<TargetedDiagnosticOverlay>,
    diagnostic_catalog: Vec<DiagnosticTask>,
    diagnostic_runtime: Option<DiagnosticRuntime>,
    diagnostic_receiver: Option<Arc<UiEventReceiver>>,
    diagnostic_wait: Option<ComponentTask>,
    diagnostic_start_task: Option<ComponentTask>,
    diagnostic_run_task: Option<ComponentTask>,
    diagnostic_cancel_task: Option<ComponentTask>,
    diagnostic_finalization_task: Option<ComponentTask>,
    diagnostic_history_save_task: Option<ComponentTask>,
    diagnostic_scan_kind: Option<ScanKind>,
    diagnostic_scan_policy: Option<DiagnosticScanPolicy>,
    diagnostic_expected_task_ids: Vec<String>,
    diagnostic_task_statuses: HashMap<String, TaskProgressStatus>,
    diagnostic_session_id: Option<String>,
    diagnostic_scan_start: Option<Instant>,
    diagnostic_duration_ms: u64,
    diagnostic_total: usize,
    diagnostic_completed: usize,
    diagnostic_errors: usize,
    diagnostic_current_task: Option<String>,
    diagnostic_starting: bool,
    diagnostic_running: bool,
    diagnostic_cancelling: bool,
    diagnostic_finalizing: bool,
    diagnostic_cancel_requested: bool,
    deterministic_visual: bool,
    is_admin: bool,
    latest_system_stats: Option<SystemStats>,
    monitor_history: MonitorHistory,
    monitor_error: Option<String>,
    native_monitor: Option<Arc<NativeMonitorRuntime>>,
    backend_receiver: Option<Arc<UiEventReceiver>>,
    backend_wait: Option<ComponentTask>,
    visual_state: VisualState,
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

        let visual_state = VisualState::from_env();
        let live_test_fixture = live_test_fixture_from_env();
        let (default_width, default_height) = visual_state.default_size();
        let width = initial_window_dimension("WFDIAG_REACTOR_WIDTH", default_width);
        let height = initial_window_dimension("WFDIAG_REACTOR_HEIGHT", default_height);
        let initial_page = std::env::var("WFDIAG_REACTOR_PAGE")
            .ok()
            .as_deref()
            .and_then(Page::from_tag)
            .unwrap_or_else(|| visual_state.default_page());
        let fixture_mode = std::env::var("WFDIAG_REACTOR_FIXTURE")
            .ok()
            .is_some_and(|value| value.eq_ignore_ascii_case("populated"));
        let diagnostic_results = if visual_state.has_scan()
            || live_test_fixture.is_some_and(LiveTestFixture::injects_scan)
            || (fixture_mode && !matches!(initial_page, Page::Monitor | Page::Processes))
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
        let (native_monitor, backend_receiver, backend_wait, monitor_error) =
            if deterministic_visual || !matches!(initial_page, Page::Monitor | Page::Processes) {
                (None, None, None, None)
            } else {
                match NativeMonitorRuntime::start_with_profile(MonitorProfile::SystemOnly) {
                    Ok((runtime, receiver)) => {
                        let receiver = Arc::new(receiver);
                        receiver.set_wake_handler(UiWakeHandler::new(ui_wake::notify));
                        (Some(Arc::new(runtime)), Some(receiver), None, None)
                    }
                    Err(error) => {
                        status = format!("Native monitoring unavailable · {error}");
                        (
                            None,
                            None,
                            None,
                            Some(format!("Native monitoring could not start: {error}")),
                        )
                    }
                }
            };
        let initial_system_info = if deterministic_visual {
            fixture_258_system_info()
        } else {
            pending_system_info()
        };
        let is_admin = initial_system_info.is_admin;
        let (
            system_runtime,
            system_receiver,
            system_wait,
            system_info_request_id,
            architecture_request_id,
            system_error,
        ) = if deterministic_visual {
            (None, None, None, None, None, None)
        } else {
            match SystemRuntime::start() {
                Ok((runtime, receiver)) => {
                    let system_info_request_id = 1;
                    let architecture_request_id = 2;
                    let system_info_result = runtime.enqueue(SystemRequest {
                        request_id: system_info_request_id,
                        kind: SystemRequestKind::SystemInfo,
                    });
                    let architecture_result = runtime.enqueue(SystemRequest {
                        request_id: architecture_request_id,
                        kind: SystemRequestKind::Architecture,
                    });
                    let pending_system_info =
                        system_info_result.is_ok().then_some(system_info_request_id);
                    let pending_architecture = architecture_result
                        .is_ok()
                        .then_some(architecture_request_id);
                    let startup_error = system_info_result
                        .err()
                        .or_else(|| architecture_result.err())
                        .map(|error| error.to_string());

                    if pending_system_info.is_none() && pending_architecture.is_none() {
                        (None, None, None, None, None, startup_error)
                    } else {
                        let receiver = Arc::new(Mutex::new(receiver));
                        let wait = spawn_system_wait(context, Arc::clone(&receiver));
                        (
                            Some(runtime),
                            Some(receiver),
                            Some(wait),
                            pending_system_info,
                            pending_architecture,
                            startup_error,
                        )
                    }
                }
                Err(error) => (None, None, None, None, None, Some(error.to_string())),
            }
        };
        if let Some(error) = system_error.as_deref() {
            status = format!("Native system identity unavailable · {error}");
        }
        let (diagnostic_catalog, diagnostic_runtime, diagnostic_receiver, diagnostic_wait) =
            if deterministic_visual {
                (Vec::new(), None, None, None)
            } else {
                let capacity =
                    NonZeroUsize::new(256).expect("diagnostic event capacity is non-zero");
                let (runtime, receiver) = NativeDiagnosticRuntime::start(capacity);
                let catalog = runtime.available_tasks();
                let receiver = Arc::new(receiver);
                receiver.set_wake_handler(UiWakeHandler::new(ui_wake::notify));
                (catalog, Some(runtime), Some(receiver), None)
            };
        let export_fixture = live_test_fixture == Some(LiveTestFixture::ExportFallback);
        let (export_runtime, export_receiver, export_error) =
            if deterministic_visual && !export_fixture {
                (None, None, None)
            } else {
                match ExportRuntime::start(export_task_catalog(&diagnostic_catalog)) {
                    Ok((runtime, receiver)) => {
                        (Some(runtime), Some(Arc::new(Mutex::new(receiver))), None)
                    }
                    Err(error) => (
                        None,
                        None,
                        Some(format!("Native report generation is unavailable: {error}")),
                    ),
                }
            };
        let has_fixture_scan = !diagnostic_results.is_empty();
        let issue_metadata = canonical_issue_metadata_snapshot();
        let issues = if deterministic_visual && has_fixture_scan {
            fixtures::fixture_258_issues()
        } else {
            Vec::new()
        };
        let issue_maintenance = issue_metadata.maintenance;
        let (issue_runtime, issue_receiver, issue_error) = if deterministic_visual {
            (None, None, None)
        } else {
            match IssueRuntime::start(issue_metadata.remediations) {
                Ok((runtime, receiver)) => {
                    (Some(runtime), Some(Arc::new(Mutex::new(receiver))), None)
                }
                Err(error) => (
                    None,
                    None,
                    Some(format!("Native issue detection is unavailable: {error}")),
                ),
            }
        };
        let mut settings_defaults = AppSettings::default();
        if deterministic_visual {
            // Preserve the Store 2.5.8 screenshot fixtures. These two visible
            // controls intentionally differ from the shipping persistence
            // defaults and must never leak into a live settings file.
            settings_defaults.network_grounding_enabled = true;
            settings_defaults.codex_model = Some("gpt-5.6-luna".to_string());
        }
        if export_fixture {
            match load_live_test_settings() {
                Ok(settings) => settings_defaults = settings,
                Err(error) => {
                    status = format!("Validation fixture settings unavailable · {error}");
                }
            }
        }
        let history_retention_policy = Arc::new(RwLock::new(HistoryRetentionPolicy::from(
            &settings_defaults,
        )));
        let (history_runtime, history_error) = if deterministic_visual {
            (None, None)
        } else {
            let retention_provider = Arc::clone(&history_retention_policy);
            let task_catalog = history_task_catalog(&diagnostic_catalog);
            match ScanStorage::default_storage_directory()
                .map(|storage_dir| {
                    HistoryRuntimeConfig::new(
                        storage_dir,
                        move || history_retention_tuple(&retention_provider),
                        move || task_catalog.clone(),
                    )
                })
                .map_err(std::io::Error::other)
                .and_then(NativeHistoryRuntime::start)
            {
                Ok(runtime) => (Some(Arc::new(runtime)), None),
                Err(error) => (
                    None,
                    Some(format!("Native history is unavailable: {error}")),
                ),
            }
        };
        let package_identity = (!deterministic_visual).then(|| {
            Arc::new(ReactorPackageIdentitySource::default()) as Arc<dyn PackageIdentitySource>
        });
        let settings_service = package_identity.as_ref().map(|identity| {
            let validator = Arc::new(ProviderPreferenceSettingsValidator::new(Arc::clone(
                identity,
            )));
            reactor_settings_service(validator)
        });
        let (
            settings_runtime,
            settings_receiver,
            settings_wait,
            settings_request_id,
            settings_load_request_id,
            settings_loading,
            settings_error,
        ) = if deterministic_visual {
            (None, None, None, 0, None, false, None)
        } else {
            let service = settings_service
                .as_ref()
                .expect("live settings service is constructed above")
                .clone();
            match SettingsRuntime::start_with_wake(service, Arc::new(ui_wake::notify)) {
                Ok((runtime, receiver)) => {
                    let request_id = 1;
                    if let Err(error) = runtime.send(SettingsCommand::Load { request_id }) {
                        (None, None, None, 0, None, false, Some(error.to_string()))
                    } else {
                        let receiver = Arc::new(Mutex::new(receiver));
                        (
                            Some(runtime),
                            Some(receiver),
                            None,
                            request_id,
                            Some(request_id),
                            true,
                            None,
                        )
                    }
                }
                Err(error) => (None, None, None, 0, None, false, Some(error.to_string())),
            }
        };
        let (
            provider_setup_runtime,
            provider_setup_receiver,
            provider_setup_wait,
            provider_setup_error,
        ) = if deterministic_visual {
            (None, None, None, None)
        } else if let Some(settings) = settings_service.as_ref() {
            match ProviderSetupRuntime::start(
                settings.clone(),
                Arc::new(FoundryCliEndpointSource::new()),
                Arc::new(ReqwestOllamaSource),
                Arc::new(ui_wake::notify),
            ) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    (Some(runtime), Some(receiver), None, None)
                }
                Err(error) => (
                    None,
                    None,
                    None,
                    Some(format!("Native model discovery is unavailable: {error}")),
                ),
            }
        } else {
            (
                None,
                None,
                None,
                Some("Native settings are unavailable for model discovery".to_string()),
            )
        };
        let (
            subscription_auth_runtime,
            subscription_auth_receiver,
            subscription_auth_wait,
            subscription_auth_error,
        ) = if deterministic_visual {
            (None, None, None, None)
        } else if let Some(settings) = settings_service.as_ref() {
            match SubscriptionAuthRuntime::start(settings.clone(), Arc::new(ui_wake::notify)) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    (Some(runtime), Some(receiver), None, None)
                }
                Err(error) => (
                    None,
                    None,
                    None,
                    Some(format!(
                        "Subscription account controls are unavailable: {error}"
                    )),
                ),
            }
        } else {
            (
                None,
                None,
                None,
                Some("Native settings are unavailable for subscription accounts".to_string()),
            )
        };
        let (
            subscription_install_runtime,
            subscription_install_receiver,
            subscription_install_wait,
            subscription_install_error,
        ) = if deterministic_visual {
            (None, None, None, None)
        } else {
            match SubscriptionInstallRuntime::start(Arc::new(ui_wake::notify)) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    (Some(runtime), Some(receiver), None, None)
                }
                Err(error) => (
                    None,
                    None,
                    None,
                    Some(format!(
                        "Subscription CLI installation is unavailable: {error}"
                    )),
                ),
            }
        };
        // Retain construction inputs without starting the expensive AI
        // execution workers. The first real chat/report/analysis/fix-plan
        // request initializes only the worker it needs. The harness policy is
        // evaluated at that same boundary, preserving WFDIAG_NO_WORKERS.
        let ai_worker_policy = std::env::var_os("WFDIAG_NO_WORKERS").unwrap_or_default();
        let skip_workers = ai_worker_policy.as_os_str();
        let ai_worker_settings = settings_service.clone();
        // Provider management and on-demand report/analysis workers share
        // cache ownership, so Settings "clear AI cache" invalidates all of
        // them even when a worker is initialized later.
        let shared_ai_cache = SharedAiCache::new(100);
        let (
            action_runtime,
            action_receiver,
            action_wait,
            action_run_receiver,
            action_run_wait,
            action_active_run,
            action_run_history,
            rehydrated_action_review,
        ) = if visual_state == VisualState::RemediationPartial {
            (
                None,
                None,
                None,
                None,
                None,
                None,
                vec![remediation_partial_visual_run()],
                None,
            )
        } else if deterministic_visual && live_test_fixture != Some(LiveTestFixture::DeviceManager)
        {
            // Screenshot and validation fixtures never execute anything. The
            // single exception is the feature-gated Device Manager fixture,
            // whose selection gate below rejects every other catalog ID.
            (None, None, None, None, None, None, Vec::new(), None)
        } else if skip_workers == "action"
            || skip_workers == "only-analysis"
            || skip_workers == "only-fix-plan"
            || skip_workers == "only-report"
            || skip_workers == "only-chat"
            || skip_workers == "only-instance"
            || skip_workers == "none"
        {
            (None, None, None, None, None, None, Vec::new(), None)
        } else {
            match NativeActionRuntime::start(Some(Arc::new(ui_wake::notify))) {
                Ok((runtime, receiver)) => {
                    let (run_events, snapshot) = runtime.subscribe_run_events();
                    let receiver = Arc::new(Mutex::new(receiver));
                    let run_receiver = Arc::new(Mutex::new(run_events));
                    let review = snapshot.pending_proposals.into_iter().next();
                    (
                        Some(runtime),
                        Some(receiver),
                        None,
                        Some(run_receiver),
                        None,
                        snapshot.active_run,
                        snapshot.history,
                        review,
                    )
                }
                Err(error) => {
                    status = format!("Native remediation unavailable · {error}");
                    (None, None, None, None, None, None, Vec::new(), None)
                }
            }
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
        let window_lifecycle_revision = window::lifecycle_snapshot().revision;
        let instance_wait = if deterministic_visual
            || skip_workers == "instance"
            || instance::activation_wake_registered()
        {
            None
        } else {
            Some(spawn_instance_watch(context, window_lifecycle_revision))
        };
        let (ai_provider_runtime, ai_status_error) = if deterministic_visual {
            (None, None)
        } else {
            match reactor_ai_provider_runtime(
                settings_service
                    .as_ref()
                    .expect("live settings service is constructed above")
                    .clone(),
                Arc::clone(
                    package_identity
                        .as_ref()
                        .expect("live package identity is constructed above"),
                ),
                shared_ai_cache.clone(),
            ) {
                Ok(runtime) => (Some(runtime), None),
                Err(error) => (
                    None,
                    Some(format!(
                        "Native AI provider discovery is unavailable: {error}"
                    )),
                ),
            }
        };
        let settings_open = visual_state == VisualState::SettingsBottom
            || std::env::var("WFDIAG_REACTOR_SETTINGS")
                .ok()
                .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        let update_delay_task = (!deterministic_visual).then(|| spawn_update_delay(context));
        // Validation knob: WFDIAG_REACTOR_THEME=light|dark|system selects
        // the startup theme (default dark, the Store 2.5.8 baseline).
        let initial_theme =
            window_theme_from_setting(&std::env::var("WFDIAG_REACTOR_THEME").unwrap_or_default());
        let initial_color_scheme = if initial_theme == WindowTheme::Light {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        };
        let initial_provider_setup_index = configured_provider_setup_index(&settings_defaults);

        let mut component = Self {
            page: initial_page,
            live_test_fixture,
            theme: initial_theme,
            effective_color_scheme: initial_color_scheme,
            window_size: WindowSize { width, height },
            requested_client_width: width,
            requested_client_height: height,
            pane_open: !settings_defaults.nav_rail_collapsed,
            about_open: false,
            about_close_reference: ElementRef::new(),
            about_dialog_epoch: 0,
            about_action_error: None,
            about_launch_task: None,
            update_runtime: None,
            update_delay_task,
            update_check_task: None,
            update_info: None,
            update_notice_visible: false,
            update_notice_epoch: 0,
            update_notice_timer_generation: 0,
            update_notice_task: None,
            update_notice_started_at: None,
            update_notice_remaining: Duration::ZERO,
            settings_open,
            settings_runtime,
            settings_receiver,
            settings_wait,
            settings_snapshot: settings_defaults.clone(),
            settings_draft: settings_defaults,
            settings_dialog_epoch: u64::from(settings_open),
            settings_request_id,
            settings_load_request_id,
            settings_pending_save: None,
            settings_loading,
            settings_saving: false,
            settings_error,
            settings_save_error: None,
            startup_scan_gate: if deterministic_visual || settings_load_request_id.is_none() {
                StartupScanGate::Consumed
            } else {
                StartupScanGate::AwaitingSettings
            },
            system_runtime,
            system_receiver,
            system_wait,
            system_info_request_id,
            architecture_request_id,
            system_info: initial_system_info,
            architecture: None,
            system_error,
            issues,
            issue_maintenance,
            issue_runtime,
            issue_receiver,
            issue_wait: None,
            issue_prepare_task: None,
            issue_request_id: 0,
            issue_committed_epoch: u64::from(has_fixture_scan),
            issue_source_session_id: has_fixture_scan.then(|| "visual-fixture".to_string()),
            issue_source_results: None,
            issue_projected_epoch: has_fixture_scan.then_some(1),
            issue_projected_session_id: has_fixture_scan.then(|| "visual-fixture".to_string()),
            issue_pending: None,
            issue_enqueued_request_id: None,
            issue_error,
            monitoring_paused: false,
            monitoring_paused_by_lifecycle: false,
            process_filter: String::new(),
            process_page: None,
            process_sort_key: ProcessSortKey::CpuPercent,
            process_sort_direction: ProcessSortDirection::Desc,
            process_offset: 0,
            process_request_id: 0,
            process_request_task: None,
            process_last_refresh_started_at: None,
            process_loading: false,
            process_error: None,
            selected_process: None,
            history_runtime,
            history_retention_policy,
            history_summaries: Vec::new(),
            history_filter: String::new(),
            selected_history_id: None,
            history_comparison: None,
            history_request_id: 0,
            history_request_task: None,
            history_compare_request_id: 0,
            history_compare_task: None,
            history_expanded_task_id: None,
            history_task_diff: None,
            history_task_diff_error: None,
            history_task_diff_request_id: 0,
            history_task_diff_task: None,
            history_loading: false,
            history_error,
            history_comparison_error: None,
            chat_input: String::new(),
            chat_composer_reference: ElementRef::new(),
            chat_focus_revision: 0,
            chat_answer: None,
            ai_worker_settings,
            ai_worker_cache: shared_ai_cache,
            ai_worker_policy,
            chat_runtime: None,
            chat_receiver: None,
            chat_wait: None,
            chat_request_id: 0,
            chat_pending: None,
            chat_attempt: None,
            chat_last_prompt: None,
            full_scan_consent: None,
            cloud_fallback_consent: None,
            cloud_fallback_policy_update: None,
            chat_messages: Vec::new(),
            report_runtime: None,
            report_receiver: None,
            report_wait: None,
            report_prepare_task: None,
            report_request_id: 0,
            report_pending: None,
            report_text: None,
            report_provider: None,
            report_provider_use: None,
            report_source_session_id: None,
            report_error: None,
            pending_ai_intent: None,
            pending_ai_preparation_error: None,
            fix_plan_runtime: None,
            fix_plan_receiver: None,
            fix_plan_wait: None,
            fix_plan_request_id: 0,
            fix_plan_pending: None,
            fix_plan: None,
            fix_plan_error: None,
            action_runtime,
            action_receiver,
            action_wait,
            action_run_receiver,
            action_run_wait,
            action_active_run,
            action_run_history,
            action_expanded_runs,
            action_request_id: 0,
            action_pending: None,
            action_pending_approval: None,
            action_review: rehydrated_action_review,
            repair_confirm: None,
            admin_relaunch_task: None,
            instance_wait,
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
            provider_key_drafts: Default::default(),
            provider_credential_transaction: ProviderCredentialTransaction::new(),
            provider_key_busy: false,
            provider_key_pending: None,
            provider_setup_index: initial_provider_setup_index,
            provider_setup_runtime,
            provider_setup_receiver,
            provider_setup_wait,
            provider_setup_error,
            provider_catalogs: (0..PROVIDER_SETUP_LABELS.len())
                .map(|_| ProviderCatalogUiState::default())
                .collect(),
            provider_catalog_request_id: 0,
            provider_catalog_pending: None,
            provider_catalog_refresh_revision: 0,
            provider_catalog_refresh_task: None,
            provider_catalog_refresh_after_cancel: false,
            subscription_auth_runtime,
            subscription_auth_receiver,
            subscription_auth_wait,
            subscription_auth_error,
            subscription_auth_states: vec![
                SubscriptionAuthUiState::default(),
                SubscriptionAuthUiState::default(),
            ],
            subscription_auth_operation_id: 0,
            subscription_auth_pending: None,
            subscription_install_runtime,
            subscription_install_receiver,
            subscription_install_wait,
            subscription_install_request_id: 0,
            subscription_install_pending: None,
            subscription_install_prompt: None,
            subscription_install_progress: None,
            subscription_install_error,
            history_clear_confirm: false,
            history_label_draft: String::new(),
            history_label_editing: false,
            history_tag_draft: String::new(),
            history_ack_busy: false,
            history_wait: None,
            history_trends: None,
            history_trends_loading: false,
            history_trends_error: None,
            history_trends_request_id: 0,
            history_trends_baseline_id: None,
            selected_result_task_id: None,
            diagnostic_filter: String::new(),
            diagnostic_raw_output: false,
            analysis_runtime: None,
            analysis_receiver: None,
            analysis_wait: None,
            analysis_request_id: 0,
            analysis_pending: None,
            diagnostic_analyses: HashMap::new(),
            issue_prioritization_pending: None,
            issue_prioritization: IssuePrioritizationDisplay::default(),
            network_connections: None,
            network_loading: false,
            ai_mode: AiMode::Assistant,
            ai_provider_runtime,
            ai_provider_status: None,
            ai_settings_ready: deterministic_visual,
            ai_status_request_id: 0,
            ai_status_task: None,
            ai_status_loading: settings_loading,
            ai_status_error,
            export_runtime,
            export_receiver,
            export_wait: None,
            export_request_id: 0,
            export_pending: None,
            export_error,
            status,
            diagnostic_results,
            previous_diagnostic_snapshot: None,
            targeted_diagnostic_overlay: None,
            diagnostic_catalog,
            diagnostic_runtime,
            diagnostic_receiver,
            diagnostic_wait,
            diagnostic_start_task: None,
            diagnostic_run_task: None,
            diagnostic_cancel_task: None,
            diagnostic_finalization_task: None,
            diagnostic_history_save_task: None,
            diagnostic_scan_kind: if has_fixture_scan {
                Some(ScanKind::Quick)
            } else {
                None
            },
            diagnostic_scan_policy: None,
            diagnostic_expected_task_ids: Vec::new(),
            diagnostic_task_statuses: HashMap::new(),
            diagnostic_session_id: None,
            diagnostic_scan_start: None,
            diagnostic_duration_ms: if has_fixture_scan { 2_300 } else { 0 },
            diagnostic_total: if has_fixture_scan { 17 } else { 0 },
            diagnostic_completed: if has_fixture_scan { 17 } else { 0 },
            diagnostic_errors: 0,
            diagnostic_current_task: None,
            diagnostic_starting: false,
            diagnostic_running: false,
            diagnostic_cancelling: false,
            diagnostic_finalizing: false,
            diagnostic_cancel_requested: false,
            deterministic_visual,
            is_admin,
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
            monitor_error,
            native_monitor,
            backend_receiver,
            backend_wait,
            visual_state,
        };

        if initial_page == Page::Processes && !deterministic_visual {
            component.request_process_page(context, false);
        }
        if initial_page == Page::History && !deterministic_visual {
            component.request_history_list(context);
        }
        component
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        self.ensure_window_hook(context);
        match message {
            Message::NativeSignalReady => {
                for pending in self.drain_native_messages() {
                    self.update(pending, context);
                }
            }
            Message::WindowHookBootstrap => {}
            Message::Navigate(Some(tag)) => {
                if let Some(page) = Page::from_tag(&tag) {
                    self.navigate_to_page(page, context);
                } else if tag == "quick-scan" {
                    self.transition_to_page(Page::Diagnostics);
                    self.begin_diagnostic_scan(ScanKind::Quick, context);
                } else {
                    match tag.as_str() {
                        "export" => self.request_export_to_file(context),
                        "share" => self.request_share_to_windowsforum(context),
                        "email" => self.request_email_report(context),
                        _ => (),
                    }
                }
            }
            Message::WindowSize(size) => self.window_size = size,
            Message::ColorSchemeChanged(color_scheme) => {
                self.effective_color_scheme = color_scheme;
            }
            Message::TogglePane => self.toggle_navigation_rail(context),
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
                        self.schedule_provider_model_refresh(context);
                    }
                }
            }
            Message::StoreProviderKey(index) => {
                self.submit_provider_key(index, true);
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
            Message::SetDiagnosticRaw(raw) => {
                self.diagnostic_raw_output = raw;
            }
            Message::SelectDiagnosticResult(task_id) => {
                self.selected_result_task_id = Some(task_id.clone());
                self.diagnostic_raw_output = false;
                self.status = format!("Selected diagnostic: {task_id}");
            }
            Message::AnalyzeSelectedDiagnostic => {
                self.begin_selected_diagnostic_analysis(false, context);
            }
            Message::RetrySelectedDiagnosticAnalysis => {
                self.begin_selected_diagnostic_analysis(true, context);
            }
            Message::CancelDiagnosticAnalysis => {
                if let Some(pending) = self.analysis_pending.as_ref()
                    && self
                        .analysis_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.cancel(pending.request_id))
                {
                    self.status = "Cancelling the diagnostic interpretation…".to_string();
                }
            }
            Message::AnalysisWorkerEventReceived(event) => {
                self.analysis_wait = None;
                if self
                    .issue_prioritization_pending
                    .as_ref()
                    .is_some_and(|pending| pending.request_id == event.request_id())
                {
                    self.apply_issue_prioritization_event(*event, context);
                    return;
                }
                let Some(pending_request_id) = self
                    .analysis_pending
                    .as_ref()
                    .map(|pending| pending.request_id)
                else {
                    self.resume_analysis_wait(context);
                    return;
                };
                if pending_request_id != event.request_id() {
                    self.resume_analysis_wait(context);
                    return;
                }
                let task_id = self
                    .analysis_pending
                    .as_ref()
                    .map(|pending| pending.attempt.generation.task_id.clone())
                    .unwrap_or_default();
                match *event {
                    AnalysisWorkerEvent::Ack {
                        provider_use,
                        grounding,
                        cached,
                        ..
                    } => {
                        let display = self.diagnostic_analyses.entry(task_id).or_default();
                        display.provider_use = Some(provider_use);
                        display.grounding = grounding;
                        display.cached = cached;
                        display.busy = true;
                        self.status = if cached {
                            "Loading the cached diagnostic interpretation…".to_string()
                        } else {
                            "The AI provider is interpreting the diagnostic…".to_string()
                        };
                        self.resume_analysis_wait(context);
                    }
                    AnalysisWorkerEvent::Done {
                        interpretation,
                        provider_use,
                        grounding,
                        cached,
                        ..
                    } => {
                        self.analysis_pending = None;
                        let provider = provider_use.provider_id.clone();
                        let display = self.diagnostic_analyses.entry(task_id).or_default();
                        display.interpretation = Some(interpretation);
                        display.provider_use = Some(provider_use);
                        display.grounding = grounding;
                        display.cached = cached;
                        display.error = None;
                        display.busy = false;
                        self.status = if cached {
                            format!("Diagnostic interpretation ready · {provider} · cached")
                        } else {
                            format!("Diagnostic interpretation ready · {provider}")
                        };
                    }
                    AnalysisWorkerEvent::Failed {
                        route,
                        provider_use,
                        grounding,
                        message,
                        retryable,
                        ..
                    } => {
                        let Some(mut pending) = self.analysis_pending.take() else {
                            self.resume_analysis_wait(context);
                            return;
                        };
                        let next_local = retryable.then(|| {
                            next_auto_local_route(
                                route.preference,
                                &pending.attempt.tried,
                                route.availability,
                            )
                        });
                        if let Some(Some(provider)) = next_local {
                            pending.attempt.tried.push(provider);
                            pending.attempt.generation.route.provider = provider;
                            pending.attempt.generation.route.fallback_from =
                                Some(pending.attempt.initial_provider);
                            if self.queue_diagnostic_analysis(pending.attempt, context) {
                                return;
                            }
                        }
                        let display = self.diagnostic_analyses.entry(task_id).or_default();
                        display.provider_use = Some(provider_use);
                        display.grounding = grounding;
                        display.error = Some(message.clone());
                        display.busy = false;
                        self.status = format!("Diagnostic interpretation failed · {message}");
                    }
                    AnalysisWorkerEvent::Cancelled {
                        provider_use,
                        grounding,
                        ..
                    } => {
                        self.analysis_pending = None;
                        let display = self.diagnostic_analyses.entry(task_id).or_default();
                        display.provider_use = Some(provider_use);
                        display.grounding = grounding;
                        display.busy = false;
                        display.error = None;
                        self.status = "Diagnostic interpretation cancelled".to_string();
                    }
                }
            }
            Message::AnalysisWorkerStopped => {
                self.analysis_wait = None;
                self.analysis_pending = None;
                self.issue_prioritization_pending = None;
                self.analysis_receiver = None;
                self.analysis_runtime = None;
                for display in self.diagnostic_analyses.values_mut() {
                    display.busy = false;
                }
                self.issue_prioritization.busy = false;
                self.status = "Native one-shot AI worker stopped".to_string();
            }
            Message::RequestNetworkConnections => {
                if self.deterministic_visual || self.network_loading {
                    return;
                }
                self.network_loading = true;
                context.spawn_background_with_rejection(
                    move |_cancellation| {
                        // Reuse the shared scan executor instead of building a
                        // fresh current-thread runtime per click.
                        let result =
                            match shared_diagnostic_executor() {
                                Ok(runtime) => Ok(runtime
                                    .block_on(wfdiag_native_monitor::get_network_connections())),
                                Err(error) => Err(error),
                            };
                        Message::NetworkConnectionsFinished(Box::new(result))
                    },
                    Message::NetworkConnectionsFinished(Box::new(Err(
                        "The Reactor background queue rejected the connections query".to_string(),
                    ))),
                );
            }
            Message::NetworkConnectionsFinished(result) => {
                self.network_loading = false;
                match *result {
                    Ok(connections) => self.network_connections = Some(connections),
                    Err(message) => self.status = message,
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
            Message::ClearProviderKey(index) => {
                if let Some(draft) = self.provider_key_drafts.get_mut(index) {
                    draft.clear();
                }
                self.submit_provider_key(index, false);
                self.schedule_provider_model_refresh(context);
            }
            Message::ToggleClearHistoryConfirm(open) => {
                self.history_clear_confirm = open;
            }
            Message::BeginHistoryLabelEdit => {
                let Some(scan_id) = self.selected_history_id.as_deref() else {
                    return;
                };
                self.history_label_draft =
                    history_label_draft_for_selection(&self.history_summaries, scan_id);
                self.history_label_editing = true;
            }
            Message::CancelHistoryLabelEdit => {
                if let Some(scan_id) = self.selected_history_id.as_deref() {
                    self.history_label_draft =
                        history_label_draft_for_selection(&self.history_summaries, scan_id);
                }
                self.history_label_editing = false;
            }
            Message::HistoryLabelDraftChanged(value) => {
                self.history_label_draft = value;
            }
            Message::SaveHistoryLabel => {
                self.request_history_label_save(context);
            }
            Message::RequestHistoryTrends => self.request_history_trends(context),
            Message::HistoryTrendsFinished { request_id, result } => {
                if request_id != self.history_trends_request_id {
                    return;
                }
                self.history_trends_loading = false;
                match *result {
                    Ok(trends) => {
                        self.history_trends = Some(trends);
                        self.history_trends_error = None;
                    }
                    Err(message) => self.history_trends_error = Some(message),
                }
            }
            Message::ClearHistoryConfirmed => {
                self.request_history_clear(context);
            }
            Message::HistoryTagDraftChanged(value) => {
                self.history_tag_draft = value;
            }
            Message::SaveHistoryTags => {
                self.request_history_tags_save(context);
            }
            Message::HistoryAckFinished { kind, result } => {
                self.history_wait = None;
                self.history_ack_busy = false;
                match (kind, result) {
                    (HistoryAckKind::Clear, Ok(())) => {
                        if let Some(task) = self.history_request_task.take() {
                            task.cancel();
                        }
                        if let Some(task) = self.history_compare_task.take() {
                            task.cancel();
                        }
                        self.history_loading = false;
                        self.history_summaries.clear();
                        self.selected_history_id = None;
                        self.history_comparison = None;
                        self.clear_history_task_diff();
                        self.invalidate_history_trends();
                        self.history_label_draft.clear();
                        self.history_label_editing = false;
                        self.history_tag_draft.clear();
                        self.status = "Scan history cleared".to_string();
                    }
                    (HistoryAckKind::Label, Ok(())) => {
                        self.history_label_editing = false;
                        self.status = if self.history_label_draft.trim().is_empty() {
                            "Label removed".to_string()
                        } else {
                            "Label saved".to_string()
                        };
                        self.request_history_list(context);
                    }
                    (HistoryAckKind::Tags, Ok(())) => {
                        self.status = "Tags saved".to_string();
                        self.request_history_list(context);
                    }
                    (HistoryAckKind::Label, Err(message)) => {
                        self.status = format!("Could not save label · {message}");
                    }
                    (HistoryAckKind::Tags, Err(message)) => {
                        self.status = format!("Could not save tags · {message}");
                    }
                    (HistoryAckKind::Clear, Err(message)) => {
                        self.status = format!("Could not clear history · {message}");
                    }
                }
            }
            Message::AboutClosed { epoch } => self.close_about(epoch),
            Message::AboutExternalRequested { epoch, action } => {
                self.request_about_external_action(epoch, action, context);
            }
            Message::AboutExternalFinished { epoch, result } => {
                if !self.about_dialog_is_current(epoch) {
                    return;
                }
                self.about_launch_task = None;
                self.about_action_error = result.err();
            }
            Message::AboutExternalRejected { epoch } => {
                if !self.about_dialog_is_current(epoch) {
                    return;
                }
                self.about_launch_task = None;
                self.about_action_error =
                    Some("The link could not enter the Reactor background queue".to_string());
            }
            Message::UpdateStartupDue { throttle } => {
                self.begin_update_check(throttle, context);
            }
            Message::UpdateStartupSkipped => {
                self.update_delay_task = None;
            }
            Message::UpdateDelayCancelled => {
                self.update_delay_task = None;
            }
            Message::UpdateDelayRejected => {
                self.update_delay_task = None;
            }
            Message::UpdateCheckFinished(result) => {
                self.apply_update_check_result(result, context);
            }
            Message::UpdateCheckCancelled => {
                self.update_check_task = None;
                self.update_runtime = None;
            }
            Message::UpdateCheckRejected => {
                self.update_check_task = None;
                self.update_runtime = None;
            }
            Message::UpdateNoticeClosed { epoch } => self.close_update_notice(epoch),
            Message::UpdateNoticeExpired {
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
            Message::UpdateNoticePointerEntered { epoch } => {
                self.pause_update_notice(epoch);
            }
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
            Message::UpdateNoticeTimerRejected {
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
            Message::OpenSettings => self.open_settings(context),
            Message::SettingsDialog { epoch, action } => {
                self.apply_settings_dialog_action(epoch, action, context);
            }
            Message::SettingsRuntimeEvent(event) => {
                self.apply_settings_event(*event, context);
            }
            Message::SettingsWorkerStopped => {
                self.settings_wait = None;
                apply_startup_scan_preference(&mut self.startup_scan_gate, false);
                self.settings_loading = false;
                self.settings_saving = false;
                self.settings_load_request_id = None;
                self.settings_pending_save = None;
                self.provider_key_pending = None;
                self.provider_key_busy = false;
                window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                self.settings_error = Some("Native settings worker stopped".to_string());
                self.settings_save_error = None;
                self.settings_receiver = None;
                self.settings_runtime = None;
                self.status = "Native settings persistence stopped".to_string();
            }
            Message::ProviderModelsRefreshDue {
                dialog_epoch,
                refresh_revision,
                setup_index,
            } => {
                if refresh_revision == self.provider_catalog_refresh_revision {
                    self.provider_catalog_refresh_task = None;
                    if self.settings_dialog_is_current(dialog_epoch)
                        && setup_index == self.provider_setup_index
                    {
                        if let Some(provider) = subscription_auth_provider_for_setup(setup_index)
                            && self.subscription_auth_pending.is_none()
                        {
                            self.begin_subscription_auth_operation(
                                provider,
                                SubscriptionAuthOperation::Status,
                                context,
                            );
                        }
                        // Claude's ACP catalog uses a pinned `npx -y` adapter
                        // which may populate the local package cache. Keep that
                        // material side effect behind the explicit Refresh
                        // button even though the shipping React pane auto-loads.
                        if provider_models_auto_discovery_allowed(setup_index) {
                            self.begin_provider_model_refresh(setup_index, context);
                        }
                    }
                }
            }
            Message::ProviderModelsRefreshCancelled { refresh_revision } => {
                if refresh_revision == self.provider_catalog_refresh_revision {
                    self.provider_catalog_refresh_task = None;
                }
            }
            Message::ProviderModelsRefreshRejected { refresh_revision } => {
                if refresh_revision == self.provider_catalog_refresh_revision {
                    self.provider_catalog_refresh_task = None;
                    if let Some(state) = self.provider_catalogs.get_mut(self.provider_setup_index) {
                        state.loading = false;
                        state.error = Some(
                            "The Reactor background queue rejected model refresh scheduling"
                                .to_string(),
                        );
                        state.stale = state.catalog.is_some();
                    }
                }
            }
            Message::RefreshProviderModels => {
                if let Some(task) = self.provider_catalog_refresh_task.take() {
                    task.cancel();
                }
                self.provider_catalog_refresh_revision =
                    self.provider_catalog_refresh_revision.wrapping_add(1);
                if let Some(provider) =
                    subscription_auth_provider_for_setup(self.provider_setup_index)
                    && self.subscription_auth_pending.is_none()
                {
                    self.begin_subscription_auth_operation(
                        provider,
                        SubscriptionAuthOperation::Status,
                        context,
                    );
                }
                self.begin_provider_model_refresh(self.provider_setup_index, context);
            }
            Message::CancelProviderModels => self.cancel_provider_model_request(),
            Message::ProviderSetupWorkerEventReceived(event) => {
                self.apply_provider_setup_event(*event, context);
            }
            Message::ProviderSetupWorkerStopped => {
                self.stop_provider_setup_delivery("Native model discovery worker stopped");
            }
            Message::RefreshSubscriptionAuth(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    SubscriptionAuthOperation::Status,
                    context,
                );
            }
            Message::StartSubscriptionSignIn(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    SubscriptionAuthOperation::SignIn,
                    context,
                );
            }
            Message::StartSubscriptionSignOut(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    SubscriptionAuthOperation::SignOut,
                    context,
                );
            }
            Message::CancelSubscriptionAuth => self.cancel_subscription_auth(),
            Message::SubscriptionAuthWorkerEventReceived(event) => {
                self.apply_subscription_auth_event(*event, context);
            }
            Message::SubscriptionAuthWorkerStopped => {
                self.stop_subscription_auth_delivery("Native subscription account worker stopped");
            }
            Message::RequestSubscriptionInstall(provider) => {
                self.request_subscription_install(provider);
            }
            Message::SubscriptionInstallPromptClosed { prompt, result } => {
                if self.subscription_install_prompt != Some(prompt) {
                    return;
                }
                self.subscription_install_prompt = None;
                if result == ContentDialogResult::Primary {
                    self.begin_subscription_install(prompt, context);
                }
            }
            Message::CancelSubscriptionInstall => self.cancel_subscription_install(),
            Message::SubscriptionInstallWorkerEventReceived(event) => {
                self.apply_subscription_install_event(*event, context);
            }
            Message::SubscriptionInstallWorkerStopped => {
                self.stop_subscription_install_delivery(
                    "Native subscription CLI installer worker stopped",
                );
            }
            Message::SystemRuntimeCompleted(completion) => {
                self.apply_system_completion(*completion, context);
            }
            Message::SystemWorkerStopped => {
                self.stop_system_delivery("Native system information worker stopped");
                self.maybe_begin_startup_scan(context);
            }
            Message::SystemWaitCancelled => {
                self.system_wait = None;
            }
            Message::SystemWaitRejected => {
                self.stop_system_delivery(
                    "The Reactor background queue rejected native system information delivery",
                );
                self.maybe_begin_startup_scan(context);
            }
            Message::IssueRuntimeCompleted(completion) => {
                self.apply_issue_completion(*completion, context);
            }
            Message::IssueRequestPrepared(prepared) => {
                self.apply_prepared_issue_request(*prepared, context);
            }
            Message::IssueRequestPreparationCancelled(pending) => {
                self.apply_issue_preparation_failure(
                    pending,
                    "Native issue request preparation was cancelled",
                );
            }
            Message::IssueRequestPreparationRejected(pending) => {
                self.apply_issue_preparation_failure(
                    pending,
                    "The Reactor background queue rejected issue request preparation",
                );
            }
            Message::IssueWorkerStopped => {
                self.stop_issue_delivery("Native issue detection worker stopped");
            }
            Message::IssueWaitCancelled => {
                self.issue_wait = None;
            }
            Message::IssueWaitRejected => {
                self.stop_issue_delivery(
                    "The Reactor background queue rejected native issue delivery",
                );
            }
            Message::RequestQuickScan => {
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            Message::RequestFullScan => {
                self.begin_diagnostic_scan(ScanKind::Full, context);
            }
            Message::CancelScan => self.request_diagnostic_cancel(context),
            Message::DiagnosticSessionStarted {
                session_id,
                scan_kind,
                task_count,
            } => {
                if !self.diagnostic_starting || self.diagnostic_scan_kind != Some(scan_kind) {
                    // A stale start completion must not replace newer visible evidence.
                    self.status = "Ignored a stale diagnostic session".to_string();
                    return;
                }
                self.diagnostic_start_task = None;
                self.diagnostic_starting = false;
                self.diagnostic_running = true;
                self.diagnostic_total = task_count;
                self.diagnostic_completed = 0;
                self.diagnostic_errors = 0;
                self.diagnostic_duration_ms = 0;
                self.diagnostic_current_task = None;
                self.diagnostic_session_id = Some(session_id.clone());
                if self.targeted_diagnostic_overlay.is_none() {
                    self.diagnostic_results.clear();
                }
                self.status = format!("{} started", scan_kind_label(scan_kind));

                if self.diagnostic_cancel_requested {
                    self.request_diagnostic_cancel(context);
                } else {
                    self.launch_diagnostic_run(session_id, context);
                }
            }
            Message::DiagnosticSessionStartFailed { error } => {
                self.diagnostic_start_task = None;
                self.diagnostic_starting = false;
                self.diagnostic_cancel_requested = false;
                self.restore_previous_diagnostics();
                self.reset_diagnostic_activity();
                self.mark_pending_ai_preparation_error(format!(
                    "The prerequisite scan could not start: {error}"
                ));
                self.status = format!("Could not start diagnostics · {error}");
            }
            Message::DiagnosticRunFinished {
                session_id,
                cancelled,
                authoritative_results,
            } => {
                if self.diagnostic_session_id.as_deref() != Some(&session_id) {
                    return;
                }

                // Every event was accepted before `run_session` returned. Drain
                // here as well as in the normal waiter so the final counters do
                // not depend on cross-thread message ordering.
                let pending = self
                    .diagnostic_receiver
                    .as_ref()
                    .map_or_else(Vec::new, |receiver| receiver.drain());
                for event in pending {
                    self.apply_diagnostic_event(event);
                }
                self.update_diagnostic_counts();
                let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);

                if cancelled {
                    self.restore_previous_diagnostics();
                    self.reset_diagnostic_activity();
                    self.mark_pending_ai_preparation_error(
                        "The prerequisite scan was stopped. Retry when ready.",
                    );
                    self.status = format!("{label} stopped · previous results restored");
                } else {
                    let authoritative_results = match authoritative_results {
                        Ok(results) => results,
                        Err(error) => {
                            let stopped =
                                self.diagnostic_cancel_requested || self.diagnostic_cancelling;
                            self.restore_previous_diagnostics();
                            self.reset_diagnostic_activity();
                            self.mark_pending_ai_preparation_error(if stopped {
                                "The prerequisite scan was stopped. Retry when ready.".to_string()
                            } else {
                                format!("The prerequisite scan failed: {error}")
                            });
                            self.status = if stopped {
                                format!("{label} stopped · previous results restored")
                            } else {
                                format!("{label} failed · {error} · previous results restored")
                            };
                            return;
                        }
                    };
                    let expected_count = self.diagnostic_expected_task_ids.len();
                    if !authoritative_result_set_is_complete(
                        &authoritative_results,
                        &self.diagnostic_expected_task_ids,
                    ) {
                        let completed_count = authoritative_results.len();
                        self.restore_previous_diagnostics();
                        self.reset_diagnostic_activity();
                        self.mark_pending_ai_preparation_error(
                            "The prerequisite scan returned incomplete evidence. Retry the scan.",
                        );
                        self.status = format!(
                            "{label} returned an invalid result set ({completed_count} results for {expected_count} expected checks) · previous results restored"
                        );
                        return;
                    }
                    let results = authoritative_ui_results(
                        &session_id,
                        &authoritative_results,
                        &self.diagnostic_catalog,
                    );
                    if self.targeted_diagnostic_overlay.is_some() {
                        if let Err(error) = self.commit_targeted_diagnostic_overlay(
                            &session_id,
                            &authoritative_results,
                            context,
                        ) {
                            self.restore_previous_diagnostics();
                            self.reset_diagnostic_activity();
                            self.status = format!(
                                "Targeted Scan failed · {error} · previous results restored"
                            );
                        }
                        return;
                    }
                    self.diagnostic_results = results;
                    self.update_diagnostic_counts();
                    // Issue detection is tied to the complete authoritative
                    // scan commit, not to optional history persistence. If a
                    // Stop raced with natural completion, this still refreshes
                    // issues before `begin_completed_diagnostic_finalization`
                    // elects to skip history auto-save.
                    self.commit_issue_evidence(session_id.clone(), authoritative_results, context);
                    self.begin_completed_diagnostic_finalization(session_id, context);
                }
            }
            Message::DiagnosticRunRejected => {
                let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                self.mark_pending_ai_preparation_error(
                    "The prerequisite scan could not enter the background queue.",
                );
                self.status = format!("{label} could not enter the Reactor background queue");
                self.diagnostic_run_task = None;
                self.request_diagnostic_cancel(context);
            }
            Message::DiagnosticFinalizationElapsed { session_id } => {
                self.begin_completed_scan_history_save(session_id, context);
            }
            Message::DiagnosticFinalizationCancelled { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_finalization_task = None;
                    self.finish_completed_diagnostic_scan(None, context);
                }
            }
            Message::DiagnosticFinalizationRejected { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_finalization_task = None;
                    self.finish_completed_diagnostic_scan(
                        Some("the Reactor background queue rejected scan finalization".to_string()),
                        context,
                    );
                }
            }
            Message::DiagnosticHistorySaveFinished { session_id, result } => {
                if !self.diagnostic_finalizing
                    || self.diagnostic_session_id.as_deref() != Some(&session_id)
                {
                    return;
                }
                self.diagnostic_history_save_task = None;
                let saved = result.is_ok();
                if saved {
                    self.history_error = None;
                }
                self.finish_completed_diagnostic_scan(result.err(), context);
                if saved && self.page == Page::History {
                    self.request_history_list(context);
                }
            }
            Message::DiagnosticHistorySaveWaitCancelled { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_history_save_task = None;
                    self.finish_completed_diagnostic_scan(
                        Some("the scan-history acknowledgement was cancelled".to_string()),
                        context,
                    );
                }
            }
            Message::DiagnosticHistorySaveRejected { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_history_save_task = None;
                    self.finish_completed_diagnostic_scan(
                        Some(
                            "the Reactor background queue rejected scan-history delivery"
                                .to_string(),
                        ),
                        context,
                    );
                }
            }
            Message::DiagnosticCancelFinished { session_id, error } => {
                if self.diagnostic_session_id.as_deref() != Some(&session_id)
                    || !self.diagnostic_cancelling
                {
                    return;
                }
                self.diagnostic_cancel_task = None;
                if let Some(error) = error {
                    self.diagnostic_cancelling = false;
                    self.diagnostic_cancel_requested = false;
                    self.status = format!("Could not stop the scan · {error}");
                    if self.diagnostic_run_task.is_none() && self.diagnostic_running {
                        self.launch_diagnostic_run(session_id, context);
                    }
                } else if self.diagnostic_run_task.is_none() {
                    // A run rejected before it entered the Reactor queue has
                    // no completion message to reconcile. The successful
                    // cancellation acknowledgement is terminal in that case.
                    let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                    self.restore_previous_diagnostics();
                    self.reset_diagnostic_activity();
                    self.mark_pending_ai_preparation_error(
                        "The prerequisite scan was stopped. Retry when ready.",
                    );
                    self.status = format!("{label} stopped · previous results restored");
                } else {
                    // Do not make the cancellation acknowledgement terminal
                    // while `run_session` still owns the authoritative result.
                    // If Stop raced with natural completion, its full exact
                    // map must be allowed to commit issues; the eventual run
                    // completion decides between commit and restore.
                    self.status = format!(
                        "Stopping {} after in-flight checks finish…",
                        self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
                    );
                }
            }
            Message::DiagnosticCancelRejected { session_id } => {
                if self.diagnostic_session_id.as_deref() != Some(&session_id)
                    || !self.diagnostic_cancelling
                {
                    return;
                }
                self.diagnostic_cancel_task = None;
                self.diagnostic_cancelling = false;
                self.diagnostic_cancel_requested = false;
                self.status =
                    "The stop request could not enter the Reactor background queue".to_string();
                if self.diagnostic_run_task.is_none()
                    && self.diagnostic_running
                    && let Some(session_id) = self.diagnostic_session_id.clone()
                {
                    self.launch_diagnostic_run(session_id, context);
                }
            }
            Message::DiagnosticBatch { events, terminated } => {
                for event in events {
                    self.apply_diagnostic_event(event);
                }
                if terminated {
                    self.diagnostic_wait = None;
                    if let Some(receiver) = self.diagnostic_receiver.take() {
                        receiver.close();
                    }
                    self.diagnostic_runtime.take();
                    if self.diagnostics_busy() && !self.diagnostic_finalizing {
                        self.restore_previous_diagnostics();
                        self.reset_diagnostic_activity();
                    }
                    if !self.diagnostic_finalizing {
                        self.mark_pending_ai_preparation_error(
                            "Native diagnostic delivery stopped before the prerequisite scan completed.",
                        );
                        self.status = "Native diagnostic event delivery stopped".to_string();
                    }
                }
            }
            Message::AiStatusFinished { request_id, result } => {
                if request_id != self.ai_status_request_id {
                    return;
                }
                self.ai_status_task = None;
                self.ai_status_loading = false;
                match result {
                    Ok(status) => {
                        let active = status.active_provider;
                        self.ai_provider_status = Some(*status);
                        self.ai_status_error = None;
                        if self.page == Page::Ai {
                            self.status = if active == AIProvider::None {
                                "AI provider check complete · no provider is ready".to_string()
                            } else {
                                format!("AI provider ready · {active}")
                            };
                        }
                        if active != AIProvider::None && self.pending_ai_intent.is_some() {
                            self.resume_pending_ai_intent(context);
                        } else if active == AIProvider::None && self.pending_ai_intent.is_some() {
                            self.mark_pending_ai_preparation_error(
                                "Set up an available AI provider before continuing",
                            );
                        }
                    }
                    Err(error) => {
                        self.ai_provider_status = None;
                        self.ai_status_error = Some(error.clone());
                        self.mark_pending_ai_preparation_error(error.clone());
                        if self.page == Page::Ai {
                            self.status = format!("AI provider check failed · {error}");
                        }
                    }
                }
            }
            Message::AiStatusCancelled { request_id } => {
                if request_id == self.ai_status_request_id {
                    self.ai_status_task = None;
                    self.ai_status_loading = false;
                    self.mark_pending_ai_preparation_error(
                        "AI provider discovery was cancelled. Retry when ready.",
                    );
                }
            }
            Message::AiStatusRejected { request_id } => {
                if request_id == self.ai_status_request_id {
                    self.ai_status_task = None;
                    self.ai_status_loading = false;
                    self.ai_provider_status = None;
                    self.ai_status_error = Some(
                        "The Reactor background queue rejected AI provider discovery".to_string(),
                    );
                    self.mark_pending_ai_preparation_error(
                        "The Reactor background queue rejected AI provider discovery",
                    );
                }
            }
            Message::CancelChat => {
                if let Some(pending) = self.cloud_fallback_policy_update.take() {
                    self.finish_chat_attempt_cancelled(pending.consent.attempt.logical_request_id);
                    return;
                }
                if let Some(consent) = self.cloud_fallback_consent.take() {
                    self.finish_chat_attempt_cancelled(consent.attempt.logical_request_id);
                    return;
                }
                let logical_request_id = self.chat_attempt.as_ref().map_or_else(
                    || self.chat_pending.unwrap_or_default(),
                    |attempt| attempt.logical_request_id,
                );
                if self
                    .chat_runtime
                    .as_ref()
                    .is_some_and(NativeChatRuntime::cancel)
                {
                    self.status = "Cancelling the AI response…".to_string();
                } else if self.chat_pending.is_some() {
                    self.finish_chat_attempt_cancelled(logical_request_id);
                    self.status =
                        "AI response stopped locally · native cancellation was unavailable"
                            .to_string();
                }
            }
            Message::NewConversation => {
                if self.chat_pending.is_some() || self.chat_interaction_blocked() {
                    self.status =
                        "Finish or cancel the current AI request before starting a new conversation"
                            .to_string();
                    return;
                }
                // An uninitialized lazy worker has no native conversation to
                // reset; clearing the local transcript is already complete.
                let reset = self
                    .chat_runtime
                    .as_ref()
                    .is_none_or(NativeChatRuntime::new_session);
                self.chat_pending = None;
                self.chat_attempt = None;
                self.chat_answer = None;
                self.chat_input.clear();
                self.chat_focus_revision = self.chat_focus_revision.wrapping_add(1);
                self.chat_last_prompt = None;
                self.chat_messages.clear();
                self.full_scan_consent = None;
                self.cloud_fallback_consent = None;
                self.cloud_fallback_policy_update = None;
                if matches!(self.pending_ai_intent, Some(PendingAiIntent::Chat { .. })) {
                    self.pending_ai_intent = None;
                    self.pending_ai_preparation_error = None;
                }
                self.status = if reset {
                    "New AI conversation started".to_string()
                } else {
                    "The native AI conversation could not be reset".to_string()
                };
            }
            Message::AllowCloudFallback => {
                self.persist_cloud_fallback_decision(CloudFallbackPolicy::Allow);
            }
            Message::NeverCloudFallback => {
                self.persist_cloud_fallback_decision(CloudFallbackPolicy::Never);
            }
            Message::ApproveFullScan => {
                if self.chat_pending.is_some() {
                    self.status =
                        "Wait for the current AI response to finish before starting the Full Scan"
                            .to_string();
                    return;
                }
                if self.diagnostics_busy() {
                    self.status = "Wait for the active scan to finish before starting a Full Scan"
                        .to_string();
                    return;
                }
                let Some(consent) = self.full_scan_consent.take() else {
                    return;
                };
                let current_scan_id = self
                    .diagnostic_results
                    .first()
                    .map(|result| result.session_id.as_str());
                if current_scan_id != Some(consent.source_scan_id.as_str()) {
                    self.status =
                        "The scan changed; ask again before running a Full Scan".to_string();
                    return;
                }
                self.pending_ai_intent = Some(PendingAiIntent::Chat {
                    prompt: consent.original_prompt,
                });
                self.pending_ai_preparation_error = None;
                self.begin_diagnostic_scan(ScanKind::Full, context);
            }
            Message::DismissFullScan => {
                self.full_scan_consent = None;
                self.status = "Full Scan request dismissed".to_string();
            }
            Message::ChatWorkerEventsBatch(events) => {
                self.chat_wait = None;
                for event in events {
                    self.apply_chat_worker_event(event, context);
                }
            }
            Message::ChatWorkerStopped => {
                self.chat_wait = None;
                self.chat_pending = None;
                self.chat_attempt = None;
                self.cloud_fallback_consent = None;
                self.chat_receiver = None;
                self.chat_runtime = None;
                self.status = "Native AI chat worker stopped".to_string();
            }
            Message::GenerateReport => self.begin_report_generation(false, context),
            Message::RegenerateReport => self.begin_report_generation(true, context),
            Message::CancelPendingAiIntent => {
                if self.pending_ai_intent.take().is_some() {
                    self.pending_ai_preparation_error = None;
                    self.status = if self.diagnostics_busy() {
                        "AI request cancelled · the active diagnostic scan will continue"
                            .to_string()
                    } else {
                        "AI request cancelled".to_string()
                    };
                }
            }
            Message::RetryPendingAiIntent => {
                if self.pending_ai_intent.is_none() {
                    return;
                }
                self.pending_ai_preparation_error = None;
                if !self.diagnostic_results.is_empty() {
                    self.resume_pending_ai_intent(context);
                } else if self.diagnostics_busy() {
                    self.status = "Waiting for the active scan before continuing AI…".to_string();
                } else {
                    self.status = "Retrying the prerequisite Quick Scan…".to_string();
                    self.begin_diagnostic_scan(ScanKind::Quick, context);
                }
            }
            Message::ExplainLatestScan => {
                self.transition_to_page(Page::Ai);
                self.ai_mode = AiMode::ScanReport;
                self.begin_report_generation(false, context);
            }
            Message::ReportGenerationPrepared {
                request_id,
                generation,
            } => {
                self.report_prepare_task = None;
                if self.report_pending != Some(request_id) {
                    return;
                }
                if let Err(error) = self.ensure_report_runtime() {
                    self.report_pending = None;
                    self.report_source_session_id = None;
                    self.report_error = Some(error.clone());
                    self.status = error;
                    return;
                }
                if self
                    .report_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.generate(request_id, *generation))
                {
                    self.resume_report_wait(context);
                } else {
                    self.report_pending = None;
                    self.report_source_session_id = None;
                    self.report_error = Some("The native report queue is unavailable".to_string());
                    self.status = "The native report queue is unavailable".to_string();
                }
            }
            Message::ReportGenerationPreparationCancelled { request_id } => {
                self.report_prepare_task = None;
                if self.report_pending == Some(request_id) {
                    self.report_pending = None;
                    self.report_text = None;
                    self.report_provider = None;
                    self.report_provider_use = None;
                    self.report_source_session_id = None;
                    self.status = "AI report cancelled".to_string();
                }
            }
            Message::ReportGenerationPreparationRejected { request_id } => {
                self.report_prepare_task = None;
                if self.report_pending == Some(request_id) {
                    self.report_pending = None;
                    self.report_text = None;
                    self.report_provider = None;
                    self.report_provider_use = None;
                    self.report_source_session_id = None;
                    self.report_error = Some(
                        "The Reactor background queue rejected report preparation".to_string(),
                    );
                    self.status =
                        "The Reactor background queue rejected report preparation".to_string();
                }
            }
            Message::CancelReport => {
                if let Some(task) = self.report_prepare_task.take() {
                    task.cancel();
                    self.report_pending = None;
                    self.report_text = None;
                    self.report_provider = None;
                    self.report_provider_use = None;
                    self.report_source_session_id = None;
                    self.report_error = None;
                    self.status = "AI report cancelled".to_string();
                    return;
                }
                if let (Some(runtime), Some(pending)) =
                    (self.report_runtime.as_ref(), self.report_pending)
                    && runtime.cancel(pending)
                {
                    self.status = "Cancelling the AI report…".to_string();
                    return;
                }
                if self.report_pending.is_some() {
                    self.report_pending = None;
                    self.report_text = None;
                    self.report_provider = None;
                    self.report_provider_use = None;
                    self.report_source_session_id = None;
                    self.report_error = None;
                    self.status = "AI report stopped locally · native cancellation was unavailable"
                        .to_string();
                }
            }
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
            Message::ReportWorkerEventReceived(event) => {
                // See the chat path above: clear the completed one-event wait
                // before deltas rearm it.
                self.report_wait = None;
                let Some(pending) = self.report_pending else {
                    return;
                };
                if pending != event.request_id() {
                    self.resume_report_wait(context);
                    return;
                }
                let source_is_current = self.report_source_session_id.as_deref()
                    == self
                        .diagnostic_results
                        .first()
                        .map(|result| result.session_id.as_str());
                if !source_is_current {
                    if matches!(
                        *event,
                        ReportWorkerEvent::Ack { .. } | ReportWorkerEvent::Delta { .. }
                    ) {
                        self.resume_report_wait(context);
                    } else {
                        self.report_pending = None;
                        self.report_text = None;
                        self.report_provider = None;
                        self.report_provider_use = None;
                        self.report_source_session_id = None;
                    }
                    return;
                }
                match *event {
                    ReportWorkerEvent::Ack {
                        provider,
                        provider_use,
                        ..
                    } => {
                        self.report_provider = Some(provider);
                        self.report_provider_use = Some(provider_use);
                        self.resume_report_wait(context);
                    }
                    ReportWorkerEvent::Delta { text, .. } => {
                        self.report_text
                            .get_or_insert_with(String::new)
                            .push_str(&text);
                        self.resume_report_wait(context);
                    }
                    ReportWorkerEvent::Done {
                        provider,
                        provider_use,
                        cached,
                        report,
                        ..
                    } => {
                        self.report_pending = None;
                        if let Some(report) = report {
                            self.report_text = Some(report);
                        }
                        self.report_provider = Some(provider.clone());
                        self.report_provider_use = Some(provider_use);
                        self.status = if cached {
                            format!("AI report ready · {provider} · cached")
                        } else {
                            format!("AI report ready · {provider}")
                        };
                    }
                    ReportWorkerEvent::Failed { message, .. } => {
                        self.report_pending = None;
                        self.report_text = None;
                        self.report_provider = None;
                        self.report_provider_use = None;
                        self.report_source_session_id = None;
                        self.report_error = Some(message.clone());
                        self.status = message;
                    }
                    ReportWorkerEvent::Cancelled { .. } => {
                        self.report_pending = None;
                        self.report_text = None;
                        self.report_provider = None;
                        self.report_provider_use = None;
                        self.report_source_session_id = None;
                        self.report_error = None;
                        self.status = "AI report cancelled".to_string();
                    }
                }
            }
            Message::ReportWorkerStopped => {
                self.report_wait = None;
                self.report_pending = None;
                self.report_receiver = None;
                self.report_runtime = None;
                self.status = "Native AI report worker stopped".to_string();
            }
            Message::RunRemediation(remediation_id) => {
                if self.deterministic_visual
                    && self.live_test_fixture != Some(LiveTestFixture::DeviceManager)
                {
                    self.status = "Visual fixture mode · remediation is disabled".to_string();
                    return;
                }
                let Some(spec) = remediation::find(&remediation_id) else {
                    self.status = format!("Unknown remediation '{remediation_id}'");
                    return;
                };
                let issue_id = self
                    .issues
                    .iter()
                    .find(|issue| {
                        issue.status == wfdiag_native_issues::IssueStatus::Detected
                            && issue.remediation.as_ref().map(|item| item.id.as_str())
                                == Some(remediation_id.as_str())
                    })
                    .map(|issue| issue.id.clone());
                if issue_id.is_none() && !spec.maintenance {
                    self.status =
                        format!("'{}' is no longer mapped to a detected issue", spec.label);
                    return;
                }
                self.prepare_remediation(remediation_id, issue_id, context);
            }
            Message::ActionReviewDialogClosed {
                proposal_id,
                result,
            } => {
                let Some(proposal) = self.action_review.take() else {
                    return;
                };
                if proposal.proposal_id != proposal_id {
                    self.action_review = Some(proposal);
                    return;
                }
                let admin_blocked = !self.is_admin
                    && proposal
                        .actions
                        .iter()
                        .any(|action| action.remediation.admin_required);
                if result == ContentDialogResult::Primary && admin_blocked {
                    self.action_review = Some(proposal);
                    self.request_admin_relaunch(context);
                } else if result == ContentDialogResult::Primary {
                    let approval = if action_proposal_contains_repair(&proposal) {
                        ActionApproval::RepairConfirmed
                    } else {
                        ActionApproval::Reviewed
                    };
                    self.approve_action_proposal(
                        proposal,
                        approval,
                        ActionReviewSurface::Review,
                        context,
                    );
                } else {
                    self.discard_action_proposal_or_restore(PendingActionApproval {
                        proposal,
                        return_surface: ActionReviewSurface::Review,
                    });
                }
            }
            Message::RepairDialogClosed {
                proposal_id,
                result,
            } => {
                let Some(proposal) = self.repair_confirm.take() else {
                    return;
                };
                if proposal.proposal_id != proposal_id {
                    self.repair_confirm = Some(proposal);
                    return;
                }
                if result == ContentDialogResult::Primary {
                    self.approve_action_proposal(
                        proposal,
                        ActionApproval::RepairConfirmed,
                        ActionReviewSurface::RepairConfirmation,
                        context,
                    );
                } else {
                    self.discard_action_proposal_or_restore(PendingActionApproval {
                        proposal,
                        return_surface: ActionReviewSurface::RepairConfirmation,
                    });
                }
            }
            Message::ActionWorkerEventReceived(event) => {
                self.action_wait = None;
                let Some(pending) = self.action_pending else {
                    return;
                };
                if pending != event.request_id() {
                    self.resume_action_wait(context);
                    return;
                }
                match *event {
                    ActionWorkerEvent::Prepared { proposal, .. } => {
                        self.action_pending = None;
                        self.action_pending_approval = None;
                        if action_proposal_matches_snapshot(&proposal, &self.action_snapshot()) {
                            let action_count = proposal.actions.len();
                            self.action_review = Some(proposal);
                            self.status = format!(
                                "Review {action_count} vetted remediation action{}",
                                if action_count == 1 { "" } else { "s" }
                            );
                        } else {
                            if let Some(runtime) = self.action_runtime.as_ref() {
                                let _ = runtime.discard(proposal.proposal_id);
                            }
                            self.status =
                                "Discarded a stale remediation preview; review the current issue again"
                                    .to_string();
                        }
                    }
                    ActionWorkerEvent::Done { execution, .. } => {
                        self.action_pending = None;
                        self.action_pending_approval = None;
                        let any_success =
                            execution.summary.actions.iter().any(|item| {
                                item.result.as_ref().is_some_and(|result| result.success)
                            });
                        self.apply_action_run_summary(execution.summary);
                        if any_success {
                            // The fix may have changed what detection sees.
                            // Refresh the authoritative projection even when
                            // the user navigated away while a long-running
                            // repair was active; returning to Issues must not
                            // show a known-stale pre-repair result set.
                            if !self.deterministic_visual {
                                self.request_issue_detection(context);
                            }
                        }
                    }
                    ActionWorkerEvent::Failed { message, .. } => {
                        self.action_pending = None;
                        let restored = self.action_pending_approval.take().is_some_and(|pending| {
                            self.restore_action_approval_if_current(pending)
                        });
                        self.status = if restored {
                            format!("{message} · the staged action is still available for review")
                        } else {
                            message
                        };
                    }
                    ActionWorkerEvent::NeedsRepairConfirmation { proposal, .. } => {
                        self.action_pending = None;
                        self.action_pending_approval = None;
                        if self.restore_action_approval_if_current(PendingActionApproval {
                            proposal,
                            return_surface: ActionReviewSurface::RepairConfirmation,
                        }) {
                            self.status = "Explicit repair confirmation is required".to_string();
                        } else {
                            self.status =
                                "The remediation preview changed before repair confirmation"
                                    .to_string();
                        }
                    }
                }
            }
            Message::ActionWorkerStopped => {
                self.action_wait = None;
                self.action_pending = None;
                self.action_pending_approval = None;
                self.action_receiver = None;
                self.action_runtime = None;
                self.status = "Native remediation worker stopped".to_string();
            }
            Message::InstanceWaitCancelled => {
                self.instance_wait = None;
            }
            Message::WindowHookRetryReady => {
                self.window_hook_retry_task = None;
                self.ensure_window_hook(context);
            }
            Message::WindowHookRetryRejected => {
                self.window_hook_retry_task = None;
                self.status = "Native window integration retry was interrupted; the next UI action will retry"
                    .to_string();
            }
            Message::ActionRunEventReceived(event) => {
                self.action_run_wait = None;
                let summary = event.summary;
                let refresh_issues = summary.status.terminal()
                    && summary
                        .actions
                        .iter()
                        .any(|action| action.result.as_ref().is_some_and(|result| result.success));
                self.apply_action_run_summary(summary);
                if refresh_issues && !self.deterministic_visual {
                    self.request_issue_detection(context);
                }
                self.resume_action_run_wait(context);
            }
            Message::ActionRunStreamStopped => {
                self.action_run_wait = None;
                self.action_run_receiver = None;
                if self.action_active_run.is_some() {
                    self.status =
                        "Remediation status delivery stopped; the last known state is shown"
                            .to_string();
                }
            }
            Message::CancelActionRun => {
                let Some(run_id) = self
                    .action_active_run
                    .as_ref()
                    .map(|run| run.run_id.clone())
                else {
                    self.status = "No remediation run is active".to_string();
                    return;
                };
                let result = self
                    .action_runtime
                    .as_ref()
                    .ok_or_else(|| "Native remediation is unavailable".to_string())
                    .and_then(|runtime| runtime.cancel(&run_id));
                match result {
                    Ok(summary) => self.apply_action_run_summary(summary),
                    Err(error) => self.status = format!("Could not stop remediation · {error}"),
                }
            }
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
            Message::AskAiAboutIssue(issue_id) => {
                let Some(issue) = self.issues.iter().find(|issue| issue.id == issue_id) else {
                    return;
                };
                let remediation = issue.remediation.as_ref().map_or_else(
                    || "No vetted remediation is mapped to this issue.".to_string(),
                    |remediation| {
                        format!(
                            "Vetted remediation: {} (catalog id {}).",
                            remediation.label, remediation.id
                        )
                    },
                );
                let prompt = format!(
                    "Explain this detected Windows issue and give safe next steps using the scan evidence and read-only tools. Issue id: {}. Title: {}. Category: {}. Severity: {:?}. Description: {}. Recommendation: {}. {} Do not claim any repair was run; stage only vetted catalog actions if useful.",
                    issue.id,
                    issue.title,
                    issue.category,
                    issue.severity,
                    issue.description,
                    issue.recommendation,
                    remediation,
                );
                self.transition_to_page(Page::Ai);
                self.ai_mode = AiMode::Assistant;
                self.begin_chat_send(prompt, context);
            }
            Message::PrioritizeIssues => {
                self.begin_issue_prioritization(self.issue_prioritization.text.is_some(), context);
            }
            Message::CancelIssuePrioritization => {
                if let Some(pending) = self.issue_prioritization_pending.as_ref()
                    && self
                        .analysis_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.cancel(pending.request_id))
                {
                    self.status = "Cancelling issue prioritization…".to_string();
                }
            }
            Message::ProposeFixPlan => {
                self.begin_fix_plan(context);
            }
            Message::CancelFixPlan => {
                if let Some(pending) = self.fix_plan_pending.as_ref()
                    && self
                        .fix_plan_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.cancel(pending.request_id))
                {
                    self.status = "Cancelling the AI fix plan…".to_string();
                }
            }
            Message::ReviewFixPlanActions(selection) => {
                self.prepare_action_selection(selection, context);
            }
            Message::FixPlanWorkerEventReceived(event) => {
                self.fix_plan_wait = None;
                let Some(pending) = self
                    .fix_plan_pending
                    .as_ref()
                    .filter(|pending| pending.request_id == event.request_id())
                    .cloned()
                else {
                    if self.fix_plan_pending.is_some() {
                        self.resume_fix_plan_wait(context);
                    }
                    return;
                };
                match *event {
                    FixPlanWorkerEvent::Ack { provider_use, .. } => {
                        self.status = format!(
                            "Generating vetted fix plan with {}…",
                            provider_use.provider_id
                        );
                        self.resume_fix_plan_wait(context);
                    }
                    FixPlanWorkerEvent::Done { plan, .. } => {
                        self.fix_plan_pending = None;
                        let current = self.action_snapshot();
                        if plan.scan_fingerprint != current.scan_fingerprint
                            || plan.catalog_fingerprint != current.catalog_fingerprint
                        {
                            self.fix_plan = None;
                            self.fix_plan_error = Some(
                                "The scan or remediation catalog changed while the plan was being generated. Generate a fresh plan."
                                    .to_string(),
                            );
                            self.status = "Discarded a stale AI fix plan".to_string();
                        } else {
                            let entry_count = plan.entries.len();
                            let provider = plan.provider_use.provider_id.clone();
                            self.fix_plan = Some(plan);
                            self.fix_plan_error = None;
                            self.status = format!(
                                "Fix plan ready · {entry_count} vetted action{} · {provider}",
                                if entry_count == 1 { "" } else { "s" }
                            );
                        }
                    }
                    FixPlanWorkerEvent::Failed {
                        route,
                        message,
                        retryable,
                        ..
                    } => {
                        self.fix_plan_pending = None;
                        let mut attempt = pending.attempt;
                        let next = retryable.then(|| {
                            next_auto_local_route(
                                route.preference,
                                &attempt.tried,
                                route.availability,
                            )
                        });
                        if let Some(Some(provider)) = next {
                            attempt.tried.push(provider);
                            attempt.generation.route = FixPlanRoute {
                                preference: route.preference,
                                provider,
                                availability: route.availability,
                                fallback_from: Some(route.provider),
                            };
                            self.status = format!(
                                "{} could not generate the plan · trying private provider {provider}…",
                                route.provider
                            );
                            let _ = self.queue_fix_plan(attempt, context);
                        } else {
                            let message = if attempt.tried.len() > 1 {
                                format!(
                                    "AI fix planning failed after private providers starting with {}: {message}",
                                    attempt.initial_provider
                                )
                            } else {
                                message
                            };
                            self.fix_plan = None;
                            self.fix_plan_error = Some(message.clone());
                            self.status = message;
                        }
                    }
                    FixPlanWorkerEvent::Cancelled { .. } => {
                        self.fix_plan_pending = None;
                        self.fix_plan_error = None;
                        self.status = "AI fix plan cancelled".to_string();
                    }
                }
            }
            Message::FixPlanWorkerStopped => {
                self.fix_plan_wait = None;
                self.fix_plan_pending = None;
                self.fix_plan_receiver = None;
                self.fix_plan_runtime = None;
                self.fix_plan_error = Some("Native AI fix-plan worker stopped".to_string());
                self.status = "Native AI fix-plan worker stopped".to_string();
            }
            Message::RestartAsAdmin => {
                self.request_admin_relaunch(context);
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
                match command {
                    window::TRAY_COMMAND_SHOW => {
                        match instance::main_window_hwnd() {
                            Some(window) => {
                                // Honor the intent captured when the user
                                // opened the menu (or left-clicked), not
                                // live visibility at drain time.
                                if window::take_tray_menu_intent() == window::TRAY_MENU_INTENT_HIDE
                                {
                                    window::hide(window);
                                } else {
                                    window::restore(window);
                                }
                            }
                            None => instance::activate_main_window(),
                        }
                    }
                    window::TRAY_COMMAND_QUICK_SCAN => {
                        self.transition_to_page(Page::Diagnostics);
                        self.begin_diagnostic_scan(ScanKind::Quick, context);
                    }
                    window::TRAY_COMMAND_EXIT => {
                        window::request_forced_close();
                        if !context.window().request_close() {
                            // Reactor declined the close (never seen in
                            // practice). Disarm the forced close so the next
                            // ordinary title-bar close still honours
                            // close-to-tray (#197), and drop the tray icon.
                            window::cancel_forced_close();
                            if let Some(window) = instance::main_window_hwnd() {
                                window::remove_tray_icon(window);
                            }
                        }
                    }
                    _ => (),
                }
            }
            Message::RestartAsAdminFinished(result) => {
                self.admin_relaunch_task = None;
                match result {
                    Ok(true) => {
                        self.status = "Relaunching with administrator rights…".to_string();
                        // The elevated child waits on the process-owned
                        // single-instance mutex. Close this copy for real (not
                        // to tray) so that child can become the new primary.
                        window::request_forced_close();
                        if !context.window().request_close() {
                            window::cancel_forced_close();
                            self.status = "The elevated copy launched, but this window could not close · exit the current copy and try again"
                                .to_string();
                        }
                    }
                    // Dismissed UAC prompt — keep running, no error.
                    Ok(false) => self.status = "Administrator relaunch was cancelled".to_string(),
                    Err(message) => self.status = message,
                }
            }
            Message::ExportRuntimeCompleted(completed) => {
                self.export_wait = None;
                let Some(pending) = self.export_pending.take() else {
                    return;
                };
                if completed.request_id != pending.request_id {
                    self.export_pending = Some(pending);
                    self.resume_export_wait(context);
                    return;
                }
                let request_id = pending.request_id;
                match (pending.action, completed.result) {
                    (
                        PendingExportAction::ShareToWindowsForum,
                        Ok(ExportPayload::WindowsForumPost(post)),
                    ) => match write_text_to_clipboard(&post) {
                        Ok(()) => match launch_export_external_action(
                            ExportExternalAction::WindowsForumNewThread,
                        ) {
                            Ok(()) => {
                                self.export_error = None;
                                self.status = "Report ready to share · copied to clipboard · paste with Ctrl+V"
                                    .to_string();
                            }
                            Err(error) => {
                                self.export_error = Some(error.to_string());
                                self.status = "Report copied to clipboard, but Windows could not open the forum"
                                    .to_string();
                            }
                        },
                        Err(error) => {
                            self.export_error = Some(error.to_string());
                            self.status = "Failed to prepare share. Please try again.".to_string();
                        }
                    },
                    (PendingExportAction::EmailReport, Ok(ExportPayload::Email(email))) => {
                        match write_text_to_clipboard(&email.clipboard_body) {
                            Ok(()) => match launch_email_compose_draft(&email) {
                                Ok(()) => {
                                    self.export_error = None;
                                    self.status = "Email ready · report copied to clipboard · paste with Ctrl+V"
                                    .to_string();
                                }
                                Err(error) => {
                                    self.export_error = Some(error.to_string());
                                    self.status = "Report copied to clipboard, but Windows could not open a new email"
                                    .to_string();
                                }
                            },
                            Err(error) => {
                                self.export_error = Some(error.to_string());
                                self.status = "Failed to prepare email. Please try exporting the report instead."
                                .to_string();
                            }
                        }
                    }
                    (
                        PendingExportAction::CopyDiagnosticReport,
                        Ok(ExportPayload::ForumClipboard(report)),
                    ) => match write_text_to_clipboard(&report) {
                        Ok(()) => {
                            self.export_error = None;
                            self.status = "Diagnostic report copied to the clipboard".to_string();
                        }
                        Err(error) => {
                            self.export_error = Some(error.to_string());
                            self.status = "Failed to copy the diagnostic report. Please try again."
                                .to_string();
                        }
                    },
                    (
                        PendingExportAction::SupportPackage { paths },
                        Ok(ExportPayload::SupportPackage(payload)),
                    ) => {
                        self.export_error = None;
                        self.status = "Writing JSON, TXT, and HTML support reports…".to_string();
                        self.export_pending = Some(PendingExport {
                            request_id,
                            action: PendingExportAction::SupportPackage {
                                paths: paths.clone(),
                            },
                        });
                        spawn_support_package_write(context, request_id, paths, payload);
                    }
                    (
                        PendingExportAction::SaveToFile { path },
                        Ok(ExportPayload::Report(content)),
                    ) => {
                        self.export_error = None;
                        self.status =
                            format!("Writing {} report…", export_format_label(path.format()));
                        self.export_pending = Some(PendingExport {
                            request_id,
                            action: PendingExportAction::SaveToFile { path: path.clone() },
                        });
                        spawn_export_file_write(context, request_id, path, content);
                    }
                    (_, Ok(_)) => {
                        self.export_error =
                            Some("Native export worker returned an unexpected payload".to_string());
                        self.status = "Failed to prepare share. Please try again.".to_string();
                    }
                    (PendingExportAction::EmailReport, Err(error)) => {
                        self.export_error = Some(error.to_string());
                        self.status =
                            "Failed to prepare email. Please try exporting the report instead."
                                .to_string();
                    }
                    (_, Err(error)) => {
                        self.export_error = Some(error.to_string());
                        self.status = "Failed to prepare share. Please try again.".to_string();
                    }
                }
            }
            Message::ExportFileSaved { request_id, result } => {
                if !pending_export_write_is_current(
                    self.export_pending.as_ref(),
                    request_id,
                    ExportWriteKind::File,
                ) {
                    return;
                }
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
            Message::SupportPackageSaved { request_id, result } => {
                if !pending_export_write_is_current(
                    self.export_pending.as_ref(),
                    request_id,
                    ExportWriteKind::SupportPackage,
                ) {
                    return;
                }
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
            Message::ExportWorkerStopped => {
                self.export_wait = None;
                self.export_pending = None;
                self.export_receiver = None;
                self.export_runtime = None;
                self.export_error = Some("Native report generation worker stopped".to_string());
                self.status = "Native report generation is unavailable".to_string();
            }
            Message::ExportWaitCancelled => {
                self.export_wait = None;
            }
            Message::ExportWaitRejected => {
                self.export_wait = None;
                self.export_pending = None;
                self.export_error =
                    Some("The Reactor background queue rejected report delivery".to_string());
                self.status = "Failed to prepare share. Please try again.".to_string();
            }
            Message::SetAiMode(mode) => self.ai_mode = mode,
            Message::ToggleMonitoring => {
                let pause = !self.monitoring_paused;
                if self.native_monitor.is_none() {
                    self.status = "Native monitoring control is unavailable".to_string();
                } else if !pause && !self.window_usable {
                    // Preserve the user's resume intent without waking the
                    // monitor while the app is hidden, minimized, or inactive.
                    self.monitoring_paused_by_lifecycle = true;
                    self.status =
                        "Live monitoring will resume when the window is active".to_string();
                } else {
                    let accepted = self.native_monitor.as_ref().is_some_and(|runtime| {
                        if pause {
                            runtime.pause()
                        } else {
                            runtime.resume()
                        }
                    });
                    if accepted {
                        self.monitoring_paused = pause;
                        self.monitoring_paused_by_lifecycle = false;
                        if !pause && let Some(runtime) = self.native_monitor.as_ref() {
                            let _ = runtime.refresh();
                        }
                        self.status = if pause {
                            "Live monitoring paused".to_string()
                        } else {
                            "Live monitoring resumed".to_string()
                        };
                    } else {
                        self.status = "Native monitoring control is unavailable".to_string();
                    }
                }
            }
            Message::Refresh => {
                if self.page == Page::Ai {
                    self.request_ai_provider_status(context);
                    self.status = if self.ai_status_loading {
                        "Checking AI providers…".to_string()
                    } else {
                        self.ai_status_error.clone().unwrap_or_else(|| {
                            "Native AI provider discovery is unavailable".to_string()
                        })
                    };
                } else if self.page == Page::Issues {
                    self.status = if self.deterministic_visual {
                        "Visual fixture mode · live issue refresh disabled".to_string()
                    } else if self.issue_source_results.is_none() {
                        "Run a completed scan before refreshing issues".to_string()
                    } else if self.request_issue_detection(context) {
                        "Refreshing issues from the latest completed scan…".to_string()
                    } else {
                        self.issue_error
                            .clone()
                            .unwrap_or_else(|| "Native issue detection is unavailable".to_string())
                    };
                } else {
                    let accepted = self.request_monitor_refresh(context);
                    if self.page == Page::Processes && accepted {
                        self.request_process_page(context, false);
                    }
                    self.status = if accepted {
                        format!("{} refresh requested", self.page.nav_label())
                    } else {
                        "Native monitoring refresh is unavailable".to_string()
                    };
                }
            }
            Message::ProcessFilterChanged(value) => {
                self.process_filter = value;
                self.process_offset = 0;
                self.selected_process = None;
                self.request_process_page(context, true);
            }
            Message::ProcessSort(sort_key) => self.set_process_sort(sort_key, context),
            Message::ProcessPrevious => {
                self.process_offset = self.process_offset.saturating_sub(PROCESS_PAGE_SIZE);
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
            Message::ProcessQueryFinished { request_id, result } => {
                if request_id != self.process_request_id || self.page != Page::Processes {
                    return;
                }
                self.process_request_task = None;
                self.process_loading = false;
                match result {
                    Ok(page) => {
                        self.process_offset = page.offset;
                        self.selected_process = reconcile_process_selection_by(
                            self.selected_process,
                            &page.items,
                            |row| ProcessIdentity::new(row.pid, row.start_time),
                        );
                        self.status = format!(
                            "Process inventory · {} of {} shown",
                            page.items.len(),
                            page.total
                        );
                        self.process_page = Some(page);
                        self.process_error = None;
                    }
                    Err(error) => {
                        self.process_error = Some(error.clone());
                        self.status = format!("Could not refresh processes · {error}");
                    }
                }
            }
            Message::ProcessQueryDiscarded { request_id } => {
                if request_id == self.process_request_id {
                    self.process_request_task = None;
                    self.process_loading = false;
                }
            }
            Message::ProcessQueryRejected { request_id } => {
                if request_id == self.process_request_id {
                    self.process_request_task = None;
                    self.process_loading = false;
                    self.process_error =
                        Some("The Reactor background queue rejected the process query".to_string());
                    self.status = "Process refresh could not start".to_string();
                }
            }
            Message::SelectProcess(identity) => self.selected_process = identity,
            Message::RefreshHistory => self.request_history_list(context),
            Message::HistoryFilterChanged(value) => self.history_filter = value,
            Message::SelectHistory(scan_id) => {
                self.history_label_draft =
                    history_label_draft_for_selection(&self.history_summaries, &scan_id);
                self.history_label_editing = false;
                self.history_tag_draft =
                    history_tag_draft_for_selection(&self.history_summaries, &scan_id);
                self.selected_history_id = Some(scan_id.clone());
                self.request_history_comparison(scan_id, context);
            }
            Message::HistoryListFinished { request_id, result } => {
                if request_id != self.history_request_id {
                    return;
                }
                self.history_request_task = None;
                self.history_loading = false;
                match result {
                    Ok(summaries) => {
                        let previous_latest_id =
                            self.history_summaries.first().map(|scan| scan.id.clone());
                        let current_latest_id = summaries.first().map(|scan| scan.id.clone());
                        let trends_baseline_changed = history_trends_baseline_changed(
                            self.history_trends_baseline_id.as_deref(),
                            current_latest_id.as_deref(),
                        );
                        let comparison_refresh_target = history_comparison_refresh_target(
                            previous_latest_id.as_deref(),
                            &summaries,
                            self.selected_history_id.as_deref(),
                        );
                        if self.selected_history_id.as_ref().is_some_and(|selected| {
                            !summaries.iter().any(|scan| &scan.id == selected)
                        }) {
                            self.selected_history_id = None;
                            self.history_comparison = None;
                            self.clear_history_task_diff();
                            self.history_label_draft.clear();
                            self.history_label_editing = false;
                            self.history_tag_draft.clear();
                            if let Some(task) = self.history_compare_task.take() {
                                task.cancel();
                            }
                        }
                        self.status = format!("History loaded · {} scans", summaries.len());
                        self.history_summaries = summaries;
                        self.history_error = None;
                        if trends_baseline_changed {
                            self.invalidate_history_trends();
                            if current_latest_id.is_some() {
                                self.request_history_trends(context);
                            }
                        }
                        if let Some(selected_id) = comparison_refresh_target {
                            self.request_history_comparison(selected_id, context);
                        } else if !self.history_label_editing
                            && let Some(selected_id) = self.selected_history_id.as_deref()
                        {
                            self.history_label_draft = history_label_draft_for_selection(
                                &self.history_summaries,
                                selected_id,
                            );
                        }
                    }
                    Err(error) => {
                        self.history_error = Some(error.clone());
                        self.status = format!("Could not load history · {error}");
                    }
                }
            }
            Message::HistoryCompareFinished { request_id, result } => {
                if request_id != self.history_compare_request_id {
                    return;
                }
                self.history_compare_task = None;
                match result {
                    Ok(comparison) => {
                        self.status =
                            format!("History comparison · {} changes", comparison.total_changes);
                        self.history_comparison = Some(*comparison);
                        self.history_comparison_error = None;
                    }
                    Err(error) => {
                        self.history_comparison = None;
                        self.history_comparison_error = Some(error.clone());
                        self.status = format!("Could not compare history · {error}");
                    }
                }
            }
            Message::ToggleHistoryTaskDetail(task_id) => {
                self.toggle_history_task_detail(task_id, context);
            }
            Message::HistoryTaskDiffFinished {
                request_id,
                task_id,
                result,
            } => {
                if !history_task_diff_result_is_current(
                    request_id,
                    self.history_task_diff_request_id,
                    &task_id,
                    self.history_expanded_task_id.as_deref(),
                ) {
                    return;
                }
                self.history_task_diff_task = None;
                match result {
                    Ok(projection) => {
                        self.status = "Stored task details loaded".to_string();
                        self.history_task_diff = Some(*projection);
                        self.history_task_diff_error = None;
                    }
                    Err(error) => {
                        self.history_task_diff = None;
                        self.history_task_diff_error = Some(error.clone());
                        self.status = format!("Could not load stored task details · {error}");
                    }
                }
            }
            Message::HistoryTaskDiffRejected {
                request_id,
                task_id,
            } => {
                if !history_task_diff_result_is_current(
                    request_id,
                    self.history_task_diff_request_id,
                    &task_id,
                    self.history_expanded_task_id.as_deref(),
                ) {
                    return;
                }
                self.history_task_diff_task = None;
                self.history_task_diff = None;
                self.history_task_diff_error = Some(
                    "The Reactor background queue rejected the task-detail request".to_string(),
                );
                self.status = "Stored task details could not start".to_string();
            }
            Message::HistoryQueryRejected {
                request_id,
                comparison,
            } => {
                if comparison {
                    if request_id != self.history_compare_request_id {
                        return;
                    }
                    self.history_compare_task = None;
                    self.history_comparison_error = Some(
                        "The Reactor background queue rejected the history comparison".to_string(),
                    );
                } else {
                    if request_id != self.history_request_id {
                        return;
                    }
                    self.history_request_task = None;
                    self.history_loading = false;
                    self.history_error = Some(
                        "The Reactor background queue rejected the history request".to_string(),
                    );
                }
                self.status = "History request could not start".to_string();
            }
            Message::ChatInputChanged(value) => {
                if !self.chat_interaction_blocked() {
                    self.chat_input = value;
                }
            }
            Message::UsePrompt(value) => {
                if self.chat_pending.is_some() || self.chat_interaction_blocked() {
                    self.status = "Finish or cancel the current AI request before starting another"
                        .to_string();
                } else {
                    self.begin_chat_send(value, context);
                }
            }
            Message::SendChat => {
                self.begin_chat_send(self.chat_input.clone(), context);
            }
            Message::BackendBatch { events, terminated } => {
                let process_refresh_due = self.process_last_refresh_started_at.is_none_or(|last| {
                    Instant::now().duration_since(last) >= PROCESS_LIVE_REFRESH_INTERVAL
                });
                let process_tick = !terminated
                    && !self.monitoring_paused
                    && self.page == Page::Processes
                    && self.process_request_task.is_none()
                    && process_refresh_due
                    && events
                        .iter()
                        .any(|event| matches!(event, UiEvent::SystemStats(_)));
                for event in events {
                    self.apply_backend_event(event);
                }
                if terminated {
                    self.backend_wait = None;
                    self.monitoring_paused = true;
                    self.monitoring_paused_by_lifecycle = false;
                    self.invalidate_process_page_request();
                    self.process_error = Some("Native monitoring worker stopped".to_string());
                    self.monitor_error = Some("Native monitoring worker stopped".to_string());
                    if let Some(receiver) = self.backend_receiver.take() {
                        receiver.close();
                    }
                    self.native_monitor.take();
                    self.status = "Native monitoring worker stopped".to_string();
                } else {
                    if process_tick {
                        self.request_process_page(context, false);
                    }
                }
            }
            Message::Navigate(None) => {}
        }
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
        let issue_projection_current = issue_projection_matches_evidence(
            self.issue_projected_epoch,
            self.issue_projected_session_id.as_deref(),
            self.issue_committed_epoch,
            self.issue_source_session_id.as_deref(),
        );
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
            && self.ai_worker_available(AiWorkerKind::Analysis)
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
                self.diagnostic_cancelling,
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
                self.cloud_fallback_consent.as_ref().or_else(|| {
                    self.cloud_fallback_policy_update
                        .as_ref()
                        .map(|pending| &pending.consent)
                }),
                self.ai_provider_status.as_ref(),
                self.ai_status_loading,
                self.ai_status_error.as_deref(),
                AiPreparationUi {
                    intent: self.pending_ai_intent.as_ref(),
                    error: self.pending_ai_preparation_error.as_deref(),
                    scan_busy: self.diagnostics_busy(),
                    scan_cancelling: self.diagnostic_cancelling,
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
                self.report_pending.is_some(),
                self.report_error.as_deref(),
                !self.diagnostic_results.is_empty(),
                context.message(Message::GenerateReport),
                context.message(Message::RegenerateReport),
                context.message(Message::CancelReport),
                context.message(Message::CopyReport),
                self.chat_pending.is_some(),
                context.message(Message::CancelChat),
            ),
            Page::Issues => issues_page(
                palette,
                effective_theme,
                &self.issues,
                &self.issue_maintenance,
                self.fix_plan.as_ref(),
                self.fix_plan_pending.is_some(),
                self.fix_plan_error.as_deref(),
                &self.issue_prioritization,
                self.action_active_run.as_ref(),
                &self.action_run_history,
                &self.action_expanded_runs,
                self.action_pending.is_some()
                    || self.action_review.is_some()
                    || self.repair_confirm.is_some()
                    || self.action_active_run.is_some(),
                self.settings_snapshot.ai_enabled,
                self.is_admin,
                self.issue_pending.is_some(),
                self.issue_error.as_deref(),
                self.issue_source_session_id.is_some(),
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
                self.history_compare_task.is_some(),
                self.history_comparison_error.as_deref(),
                self.history_expanded_task_id.as_deref(),
                self.history_task_diff.as_ref(),
                self.history_task_diff_task.is_some(),
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
        let settings_editable = !self.settings_loading
            && !self.settings_saving
            && self.subscription_install_pending.is_none();
        let settings_can_save =
            settings_editable && (self.deterministic_visual || self.settings_runtime.is_some());
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
            let subscription_install_active = self
                .subscription_install_pending
                .is_some_and(|pending| pending.provider == subscription_provider);
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
                SubscriptionInstallPrompt::Winget { provider, .. } => {
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
                SubscriptionInstallPrompt::VendorFallback {
                    provider, reason, ..
                } => {
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
