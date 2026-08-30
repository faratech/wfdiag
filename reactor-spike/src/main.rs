#![windows_subsystem = "windows"]

mod action_support;
mod chat_support;
mod export_support;
mod icons;
mod instance_support;
mod issue_support;
mod notification_support;
mod window_support;
mod report_support;
mod save_picker;
mod update_support;

use export_support::{
    ExportExternalAction, current_export_date_strings, launch_export_external_action,
    write_text_to_clipboard,
};
use action_support::{ActionWorkerEvent, NativeActionRuntime};
use chat_support::{CHAT_WAIT_POLL, ChatWorkerEvent, NativeChatRuntime};
use report_support::{NativeReportRuntime, ReportWorkerEvent, ReportScan};
use wfdiag_native_remediation::remediation;
use save_picker::{SavePickerOutcome, ValidatedExportPath};
use icons::FaIcon;
use issue_support::{
    PendingIssueDetection, PreparedIssueDetection, advance_nonzero_generation,
    canonical_issue_metadata_snapshot, fixture_258_issues, issue_projection_matches_evidence,
    pending_issue_preparation_is_current, prepare_issue_detection, project_issues,
    take_current_issue_completion,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};
use wfdiag_native_ai_provider::{
    ReqwestOllamaSource,
    AIProvider, AIProviderPreference, AIProviderStatus, FoundryCliEndpointSource,
    NativeAiProviderRuntime, PackageIdentitySource, ProcessSubscriptionCliStatusSource,
    ProviderManagementService, ProviderModelDefaults, ProviderPreferenceSettingsValidator,
    ProviderPreferenceStatusReply, ProviderProbeBundle, ProviderSelectionState,
    ProviderStatusReply, SettingsServiceProviderConfigurationSource, SharedAiCache,
    provider_preference_for_runtime,
};
use wfdiag_native_diagnostics::{
    DiagnosticOutput, DiagnosticRuntime, DiagnosticTask, NativeDiagnosticRuntime, ScanKind,
};
use wfdiag_native_export::{
    ExportCompleted, ExportMetadata, ExportPayload, ExportRequest, ExportRequestKind,
    ExportRuntime, ExportTask, ReportFormat, TaskResult as ExportTaskResult,
};
use wfdiag_native_history::{
    ComparisonSummary, DiagnosticTask as HistoryDiagnosticTask, HistoryReply, HistoryRuntimeConfig,
    NativeHistoryRuntime, ScanRecord, ScanStorage, ScanSummary, TaskResult as HistoryTaskResult,
    Timestamp,
};
use wfdiag_native_issues::{
    Issue, IssueDetectionCompleted, IssueRuntime, IssueSeverity, RemediationSummary,
    RemediationTier, Timestamp as IssueTimestamp,
};
use wfdiag_native_monitor::{
    NativeMonitorRuntime, ProcessPage, ProcessQuery, ProcessRow, ProcessSortDirection,
    ProcessSortKey,
};
use wfdiag_native_phi::WindowsPhiStatusSource;
use wfdiag_native_settings::{
    AppSettings, CloudFallbackPolicy, ProviderKeyId, SettingsCommand, SettingsEvent,
    SettingsRuntime, SettingsService, SettingsValidator, windows_shipping_settings_service,
};
#[cfg(feature = "settings-test-path")]
use wfdiag_native_settings::{ShippingSettingsStorage, WindowsDpapiCredentialStorage};
use wfdiag_native_system::{
    ArchitectureSnapshot, SystemCompleted, SystemInfo, SystemPayload, SystemRequest,
    SystemRequestKind, SystemRuntime,
};
use wfdiag_native_update::{
    NativeUpdateRuntime, SignatureProvider, UpdateInfo, UpdateReply, UpdateService,
    WindowsPackageSignatureProvider,
};
use wfdiag_ui_core::{
    ChatEvent, DiagnosticTaskResult, SystemStats, TaskProgressStatus, UiEvent, UiEventReceiver,
};
use windows_reactor::*;

use update_support::{
    AboutExternalAction, NOTICE_DURATION, START_DELAY, UpdateThrottle, launch_external_action,
    trusted_release_url, unix_time_millis,
};

const APP_VERSION: &str = env!("WFDIAG_APP_VERSION");
const ABOUT_DESCRIPTION: &str = "A native Windows diagnostics tool by WindowsForum.com. Runs hardware, driver, storage, network, security and log diagnostics locally — with optional on-device or cloud AI analysis.";
const VERSION_PROBE_FLAG: &str = "--wfdiag-version-probe";
const VERSION_PROBE_FILE_ENV: &str = "WFDIAG_REACTOR_VERSION_PROBE_FILE";
const APP_BADGE: &[u8] = include_bytes!("../../public/wf-ds/app-badge.png");
const BOT_AVATAR: &[u8] = include_bytes!("../../public/wf-ds/chatgpt-bot-avatar.webp");
const STETHOSCOPE_LIGHT: &[u8] = include_bytes!("../assets/stethoscope-light.png");
const STETHOSCOPE_DARK: &[u8] = include_bytes!("../assets/stethoscope-dark.png");
const STATUS_INFO_LIGHT: &[u8] = include_bytes!("../assets/status-circle-info-light.png");
const STATUS_INFO_DARK: &[u8] = include_bytes!("../assets/status-circle-info-dark.png");
const STATUS_OK_LIGHT: &[u8] = include_bytes!("../assets/status-circle-check-light.png");
const STATUS_OK_DARK: &[u8] = include_bytes!("../assets/status-circle-check-dark.png");
const STATUS_WARN_LIGHT: &[u8] = include_bytes!("../assets/status-triangle-exclamation-light.png");
const STATUS_WARN_DARK: &[u8] = include_bytes!("../assets/status-triangle-exclamation-dark.png");
const WAND_LIGHT: &[u8] = include_bytes!("../assets/wand-magic-sparkles-light.png");
const WAND_DARK: &[u8] = include_bytes!("../assets/wand-magic-sparkles-dark.png");
const DESKTOP_LIGHT: &[u8] = include_bytes!("../assets/desktop-light.png");
const DESKTOP_DARK: &[u8] = include_bytes!("../assets/desktop-dark.png");
const ISSUE_USER_SHIELD_LIGHT: &[u8] = include_bytes!("../assets/issue-user-shield-light.png");
const ISSUE_USER_SHIELD_DARK: &[u8] = include_bytes!("../assets/issue-user-shield-dark.png");
const ISSUE_INFO_LIGHT: &[u8] = include_bytes!("../assets/issue-circle-info-light.png");
const ISSUE_INFO_DARK: &[u8] = include_bytes!("../assets/issue-circle-info-dark.png");
const ISSUE_WARN_LIGHT: &[u8] = include_bytes!("../assets/issue-triangle-exclamation-light.png");
const ISSUE_WARN_DARK: &[u8] = include_bytes!("../assets/issue-triangle-exclamation-dark.png");
const ISSUE_SHIELD_LIGHT: &[u8] = include_bytes!("../assets/issue-shield-halved-light.png");
const ISSUE_SHIELD_DARK: &[u8] = include_bytes!("../assets/issue-shield-halved-dark.png");
const ISSUE_STETHOSCOPE_LIGHT: &[u8] = include_bytes!("../assets/issue-stethoscope-light.png");
const ISSUE_STETHOSCOPE_DARK: &[u8] = include_bytes!("../assets/issue-stethoscope-dark.png");
// WinUI has no per-element backdrop-blur brush at the pinned Reactor revision.
// These are deterministic, pre-blurred derivatives of the two canonical WF assets.
const WALLPAPER_LIGHT: &[u8] = include_bytes!("../assets/bg-24H4-native-blurred.webp");
const WALLPAPER_DARK: &[u8] = include_bytes!("../assets/bg-24H4-oled-native-blurred.webp");

// This is the exact 2.5.8 default Quick Scan union from `useScanner.ts`:
// baseline inventory plus the cheap, non-admin issue-detection sources.
const QUICK_SCAN_TASK_IDS: [&str; 17] = [
    "comp_system",
    "os_info",
    "processor",
    "physical_memory",
    "disk_drive",
    "logical_disk",
    "network_adapter",
    "systeminfo",
    "pending_reboot",
    "device_errors",
    "defender_status",
    "event_codes_critical",
    "services",
    "performance",
    "startup_command",
    "hosts_file",
    "firewall_status",
];

// Store 2.5.8 always unions these cheap, non-admin issue sources into a
// customised Quick Scan. Issue detection itself is intentionally integrated
// separately; retaining the sources here preserves the scan contract.
const QUICK_DETECTION_SOURCE_TASK_IDS: [&str; 11] = [
    "logical_disk",
    "network_adapter",
    "pending_reboot",
    "device_errors",
    "defender_status",
    "event_codes_critical",
    "services",
    "performance",
    "startup_command",
    "hosts_file",
    "firewall_status",
];

const PROCESS_PAGE_SIZE: usize = 100;
const PROCESS_FILTER_DEBOUNCE: Duration = Duration::from_millis(180);
const SCAN_FINALIZATION_DELAY: Duration = Duration::from_millis(500);
const SETTINGS_MAX_CONCURRENT_TASKS: u32 = 16;

const AI_PROVIDER_LABELS: [&str; 11] = [
    "Auto",
    "Phi Silica (on-device)",
    "Foundry Local (local server)",
    "Ollama (local server)",
    "Custom endpoint",
    "ChatGPT via Codex CLI (subscription)",
    "Claude via Claude Code CLI (subscription)",
    "OpenAI (cloud)",
    "Anthropic Claude (cloud)",
    "Google Gemini (cloud)",
    "DeepSeek (cloud)",
];
const AI_PROVIDER_IDS: [&str; 11] = [
    "auto",
    "phi_silica",
    "foundry_local",
    "ollama",
    "custom_openai",
    "codex_cli",
    "claude_code",
    "openai",
    "anthropic",
    "gemini",
    "deepseek",
];
const CODEX_MODEL_IDS: [&str; 3] = ["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.5"];

fn window_theme_from_setting(value: &str) -> WindowTheme {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "system" => WindowTheme::System,
        "light" => WindowTheme::Light,
        _ => WindowTheme::Dark,
    }
}

fn window_theme_setting(theme: WindowTheme) -> &'static str {
    match theme {
        WindowTheme::System => "auto",
        WindowTheme::Light => "light",
        WindowTheme::Dark => "dark",
    }
}

fn selected_setting_index(value: &str, values: &[&str]) -> Option<usize> {
    Some(
        values
            .iter()
            .position(|candidate| value.eq_ignore_ascii_case(candidate))
            .unwrap_or_default(),
    )
}

fn codex_model_options(settings: &AppSettings) -> (Vec<String>, Option<usize>) {
    let mut options = vec!["Use CLI default".to_string()];
    options.extend(CODEX_MODEL_IDS.into_iter().map(ToString::to_string));
    let selected = match settings.codex_model.as_deref().map(str::trim) {
        None | Some("") => 0,
        Some(model) => CODEX_MODEL_IDS
            .iter()
            .position(|candidate| model.eq_ignore_ascii_case(candidate))
            .map_or_else(
                || {
                    options.push(model.to_string());
                    options.len() - 1
                },
                |index| index + 1,
            ),
    };
    (options, Some(selected))
}

#[derive(Debug, Default)]
struct ReactorPackageIdentitySource {
    provider: WindowsPackageSignatureProvider,
}

impl PackageIdentitySource for ReactorPackageIdentitySource {
    fn has_package_identity(&self) -> bool {
        SignatureProvider::has_package_identity(&self.provider)
    }
}

fn provider_preference_id(preference: AIProviderPreference) -> &'static str {
    match preference {
        AIProviderPreference::Auto => "auto",
        AIProviderPreference::OpenAI => "openai",
        AIProviderPreference::PhiSilica => "phi_silica",
        AIProviderPreference::FoundryLocal => "foundry_local",
        AIProviderPreference::Ollama => "ollama",
        AIProviderPreference::CustomOpenAI => "custom_openai",
        AIProviderPreference::CodexCli => "codex_cli",
        AIProviderPreference::ClaudeCode => "claude_code",
        AIProviderPreference::Anthropic => "anthropic",
        AIProviderPreference::Gemini => "gemini",
        AIProviderPreference::DeepSeek => "deepseek",
    }
}

fn normalize_provider_preference_for_runtime(settings: &mut AppSettings) {
    let identity = ReactorPackageIdentitySource::default();
    let preference = provider_preference_for_runtime(
        &settings.preferred_ai_provider,
        identity.has_package_identity(),
    );
    settings.preferred_ai_provider = provider_preference_id(preference).to_string();
}

fn reactor_settings_service(validator: Arc<dyn SettingsValidator>) -> SettingsService {
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = std::env::var_os("WFDIAG_REACTOR_SETTINGS_TEST_PATH") {
        return SettingsService::new(
            Arc::new(ShippingSettingsStorage::at_path(path.into())),
            Arc::new(WindowsDpapiCredentialStorage::new()),
            validator,
        );
    }
    windows_shipping_settings_service(validator)
}

fn reactor_ai_provider_runtime(
    settings: SettingsService,
    identity: Arc<dyn PackageIdentitySource>,
) -> Result<NativeAiProviderRuntime, String> {
    let probes = ProviderProbeBundle::shipping_networks(
        Arc::new(SettingsServiceProviderConfigurationSource::new(settings)),
        Arc::clone(&identity),
        Arc::new(WindowsPhiStatusSource),
        Arc::new(FoundryCliEndpointSource::new()),
        Arc::new(ProcessSubscriptionCliStatusSource::new()),
    );
    let service = ProviderManagementService::new(
        probes,
        ProviderSelectionState::default(),
        Arc::new(SharedAiCache::new(100)),
        ProviderModelDefaults {
            foundry: "phi-4-mini".to_string(),
            openai: "gpt-5-nano".to_string(),
            anthropic: "claude-sonnet-4-6".to_string(),
            gemini: "gemini-3.6-flash".to_string(),
            deepseek: "deepseek-v4-flash".to_string(),
        },
    );
    NativeAiProviderRuntime::start(Arc::new(service)).map_err(|error| error.to_string())
}

fn reactor_update_throttle() -> Option<UpdateThrottle> {
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = std::env::var_os("WFDIAG_REACTOR_SETTINGS_TEST_PATH") {
        let path = std::path::PathBuf::from(path);
        return Some(UpdateThrottle::beside_settings_file(&path));
    }
    UpdateThrottle::shipping().ok()
}

fn scan_kind_label(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Targeted Scan",
    }
}

fn scan_kind_history_tag(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Manual Diagnostic",
    }
}

fn scan_concurrency_from_settings(max_concurrent_tasks: u32) -> usize {
    if max_concurrent_tasks == 0 {
        5
    } else {
        usize::try_from(max_concurrent_tasks).unwrap_or(5)
    }
}

fn select_scan_tasks(
    catalog: &[DiagnosticTask],
    scan_kind: ScanKind,
    is_admin: bool,
    custom_quick_tasks: Option<&[String]>,
) -> Vec<String> {
    let custom_quick_tasks = custom_quick_tasks.filter(|tasks| !tasks.is_empty());
    catalog
        .iter()
        .filter(|task| match scan_kind {
            ScanKind::Quick => custom_quick_tasks.map_or_else(
                || QUICK_SCAN_TASK_IDS.contains(&task.id.as_str()),
                |tasks| {
                    tasks.iter().any(|task_id| task_id == &task.id)
                        || QUICK_DETECTION_SOURCE_TASK_IDS.contains(&task.id.as_str())
                },
            ),
            ScanKind::Full | ScanKind::Targeted => is_admin || !task.admin_required,
        })
        .map(|task| task.id.clone())
        .collect()
}

fn history_task_catalog(catalog: &[DiagnosticTask]) -> Vec<HistoryDiagnosticTask> {
    catalog
        .iter()
        .map(|task| HistoryDiagnosticTask {
            id: task.id.clone(),
            name: task.name.clone(),
            description: task.description.clone(),
            category: task.category.clone(),
            admin_required: task.admin_required,
        })
        .collect()
}

fn export_task_catalog(catalog: &[DiagnosticTask]) -> Vec<ExportTask> {
    catalog
        .iter()
        .map(|task| ExportTask::new(&task.id, &task.name, &task.category))
        .collect()
}

fn system_identity_blocks_scan(
    deterministic_visual: bool,
    system_info_request_id: Option<u64>,
) -> bool {
    !deterministic_visual && system_info_request_id.is_some()
}

fn build_diagnostic_executor() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(5)
        .thread_name("wfdiag-diagnostic")
        .build()
        .map_err(|error| format!("could not create the diagnostic worker pool: {error}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Diagnostics,
    Monitor,
    Processes,
    Ai,
    Issues,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AiMode {
    Assistant,
    ScanReport,
}

/// Deterministic Store 2.5.8 visual states used only by screenshot/QA automation.
/// The normal application path remains `Live` unless the environment variable is set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum VisualState {
    #[default]
    Live,
    MonitorEmpty,
    ProcessesEmpty,
    HistoryEmpty,
    AiEmptyCompact,
    IssueToChat,
    AiConversationDesktop,
    AiConversationTopCompact,
    AiConversationBottomCompact,
    SettingsBottom,
}

impl VisualState {
    fn from_env() -> Self {
        match std::env::var("WFDIAG_REACTOR_VISUAL_STATE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('_', "-")
            .as_str()
        {
            "monitor-empty-desktop-dark" | "monitor-empty" => Self::MonitorEmpty,
            "processes-empty-desktop-dark" | "processes-empty" => Self::ProcessesEmpty,
            "history-empty-desktop-dark" | "history-empty" => Self::HistoryEmpty,
            "ai-empty-compact-dark" | "ai-empty-compact" => Self::AiEmptyCompact,
            "issue-to-chat-desktop-dark" | "issue-to-chat" => Self::IssueToChat,
            "ai-conversation-desktop-dark" | "ai-conversation-desktop" => {
                Self::AiConversationDesktop
            }
            "ai-conversation-top-compact-dark" | "ai-conversation-top-compact" => {
                Self::AiConversationTopCompact
            }
            "ai-conversation-bottom-compact-dark" | "ai-conversation-bottom-compact" => {
                Self::AiConversationBottomCompact
            }
            "settings-bottom-desktop-dark" | "settings-bottom" => Self::SettingsBottom,
            _ => Self::Live,
        }
    }

    fn default_page(self) -> Page {
        match self {
            Self::MonitorEmpty | Self::SettingsBottom => Page::Monitor,
            Self::ProcessesEmpty => Page::Processes,
            Self::HistoryEmpty => Page::History,
            Self::AiEmptyCompact
            | Self::IssueToChat
            | Self::AiConversationDesktop
            | Self::AiConversationTopCompact
            | Self::AiConversationBottomCompact => Page::Ai,
            Self::Live => Page::Diagnostics,
        }
    }

    fn default_size(self) -> (f64, f64) {
        match self {
            Self::MonitorEmpty | Self::ProcessesEmpty | Self::HistoryEmpty => (1440.0, 1000.0),
            Self::AiEmptyCompact
            | Self::AiConversationTopCompact
            | Self::AiConversationBottomCompact => (900.0, 800.0),
            Self::IssueToChat | Self::AiConversationDesktop | Self::SettingsBottom => {
                (1440.0, 900.0)
            }
            Self::Live => (1200.0, 800.0),
        }
    }

    fn has_scan(self) -> bool {
        matches!(
            self,
            Self::IssueToChat
                | Self::AiConversationDesktop
                | Self::AiConversationTopCompact
                | Self::AiConversationBottomCompact
        )
    }

    fn is_conversation(self) -> bool {
        matches!(
            self,
            Self::IssueToChat
                | Self::AiConversationDesktop
                | Self::AiConversationTopCompact
                | Self::AiConversationBottomCompact
        )
    }
}

impl Page {
    const ALL: [Self; 6] = [
        Self::Diagnostics,
        Self::Monitor,
        Self::Processes,
        Self::Ai,
        Self::Issues,
        Self::History,
    ];

    fn tag(self) -> &'static str {
        match self {
            Self::Diagnostics => "diagnostics",
            Self::Monitor => "monitor",
            Self::Processes => "processes",
            Self::Ai => "ai",
            Self::Issues => "issues",
            Self::History => "history",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Diagnostics => "System Analysis",
            Self::Monitor => "Live Monitor",
            Self::Processes => "Processes",
            Self::Ai => "AI Analysis",
            Self::Issues => "Issues",
            Self::History => "History",
        }
    }

    fn nav_label(self) -> &'static str {
        match self {
            Self::Diagnostics => "Diagnostics",
            Self::Monitor => "Live Monitor",
            Self::Processes => "Processes",
            Self::Ai => "AI Analysis",
            Self::Issues => "Issues",
            Self::History => "History",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Diagnostics => "Read-only diagnostics across hardware, storage, network and logs",
            Self::Monitor => "Real-time CPU, memory, disk, network and NPU telemetry",
            Self::Processes => "Running processes with live resource usage",
            Self::Ai => "Ask about this PC or turn the latest scan into a focused health report",
            Self::Issues => "Problems detected in the latest scan, with one-click fixes",
            Self::History => "Past scans — spot drift and regressions over time",
        }
    }

    fn icon(self) -> FaIcon {
        match self {
            Self::Diagnostics => FaIcon::Diagnostics,
            Self::Monitor => FaIcon::Monitor,
            Self::Processes => FaIcon::Processes,
            Self::Ai => FaIcon::Ai,
            Self::Issues => FaIcon::Issues,
            Self::History => FaIcon::History,
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.tag() == tag)
    }
}

#[derive(Clone, Copy)]
struct Palette {
    panel: Color,
    card: Color,
    card_strong: Color,
    border: Color,
    dim: Color,
    active: Color,
    accent: Color,
    ok: Color,
    ok_bg: Color,
    warn: Color,
    warn_bg: Color,
    err: Color,
    err_bg: Color,
    text: Color,
    muted: Color,
}

impl Palette {
    fn for_theme(theme: WindowTheme) -> Self {
        if theme == WindowTheme::Light {
            Self {
                panel: Color::argb(140, 255, 255, 255),
                card: Color::argb(168, 255, 255, 255),
                card_strong: Color::rgb(255, 255, 255),
                border: Color::argb(23, 9, 20, 32),
                dim: Color::argb(34, 243, 247, 250),
                active: Color::argb(28, 15, 108, 189),
                accent: Color::rgb(15, 108, 189),
                ok: Color::rgb(28, 157, 91),
                ok_bg: Color::argb(42, 80, 205, 137),
                warn: Color::rgb(168, 127, 0),
                warn_bg: Color::argb(42, 241, 188, 0),
                err: Color::rgb(217, 33, 78),
                err_bg: Color::argb(40, 241, 65, 108),
                text: Color::rgb(26, 27, 27),
                muted: Color::rgb(95, 96, 96),
            }
        } else {
            Self {
                panel: Color::argb(153, 26, 27, 29),
                card: Color::argb(13, 255, 255, 255),
                card_strong: Color::rgb(43, 44, 46),
                border: Color::argb(20, 255, 255, 255),
                dim: Color::argb(112, 7, 9, 12),
                active: Color::argb(38, 77, 163, 232),
                accent: Color::rgb(77, 163, 232),
                ok: Color::rgb(80, 205, 137),
                ok_bg: Color::argb(44, 80, 205, 137),
                warn: Color::rgb(241, 188, 0),
                warn_bg: Color::argb(38, 241, 188, 0),
                err: Color::rgb(241, 65, 108),
                err_bg: Color::argb(48, 241, 65, 108),
                text: Color::rgb(255, 255, 255),
                muted: Color::rgb(207, 207, 207),
            }
        }
    }
}

#[derive(Clone)]
enum SettingsDialogAction {
    ThemeSelectionChanged(Option<usize>),
    ExportFormatSelectionChanged(Option<usize>),
    AiEnabledChanged(bool),
    PreferredAiProviderSelectionChanged(Option<usize>),
    CloudFallbackSelectionChanged(Option<usize>),
    NetworkGroundingChanged(bool),
    CodexCliPathChanged(String),
    CodexModelSelectionChanged(Option<usize>),
    ScanOnStartupChanged(bool),
    CloseToTrayChanged(bool),
    MaxConcurrentTasksChanged(Option<f64>),
    AutoSaveChanged(bool),
    NotificationsChanged(bool),
    Cancel,
    Save,
}

impl SettingsDialogAction {
    fn changes_draft(&self) -> bool {
        !matches!(
            self,
            Self::ThemeSelectionChanged(None)
                | Self::ExportFormatSelectionChanged(None)
                | Self::PreferredAiProviderSelectionChanged(None)
                | Self::CloudFallbackSelectionChanged(None)
                | Self::CodexModelSelectionChanged(None)
                | Self::MaxConcurrentTasksChanged(None)
                | Self::Cancel
                | Self::Save
        )
    }
}

/// Which history maintenance operation an acknowledgement completes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryAckKind {
    Tags,
    Clear,
}

#[derive(Clone)]
enum Message {
    Navigate(Option<String>),
    WindowSize(WindowSize),
    TogglePane,
    OpenAbout,
    AboutClosed {
        epoch: u64,
    },
    AboutExternalRequested {
        epoch: u64,
        action: AboutExternalAction,
    },
    AboutExternalFinished {
        epoch: u64,
        result: Result<(), String>,
    },
    AboutExternalRejected {
        epoch: u64,
    },
    UpdateStartupDue {
        throttle: Option<UpdateThrottle>,
    },
    UpdateStartupSkipped,
    UpdateDelayCancelled,
    UpdateDelayRejected,
    UpdateCheckFinished(Result<Option<UpdateInfo>, String>),
    UpdateCheckCancelled,
    UpdateCheckRejected,
    UpdateNoticeClosed {
        epoch: u64,
    },
    UpdateNoticeExpired {
        epoch: u64,
        timer_generation: u64,
    },
    UpdateNoticePointerEntered {
        epoch: u64,
    },
    UpdateNoticePointerExited {
        epoch: u64,
    },
    UpdateNoticeTimerCancelled {
        epoch: u64,
        timer_generation: u64,
    },
    UpdateNoticeTimerRejected {
        epoch: u64,
        timer_generation: u64,
    },
    OpenSettings,
    SettingsDialog {
        epoch: u64,
        action: SettingsDialogAction,
    },
    SettingsRuntimeEvent(Box<SettingsEvent>),
    SettingsWorkerStopped,
    SettingsWaitCancelled,
    SettingsWaitRejected,
    SystemRuntimeCompleted(Box<SystemCompleted>),
    SystemWorkerStopped,
    SystemWaitCancelled,
    SystemWaitRejected,
    IssueRuntimeCompleted(Box<IssueDetectionCompleted>),
    IssueRequestPrepared(Box<PreparedIssueDetection>),
    IssueRequestPreparationCancelled(PendingIssueDetection),
    IssueRequestPreparationRejected(PendingIssueDetection),
    IssueWorkerStopped,
    IssueWaitCancelled,
    IssueWaitRejected,
    RequestQuickScan,
    RequestFullScan,
    CancelScan,
    DiagnosticSessionStarted {
        session_id: String,
        scan_kind: ScanKind,
        task_count: usize,
    },
    DiagnosticSessionStartFailed {
        error: String,
    },
    DiagnosticRunFinished {
        session_id: String,
        cancelled: bool,
        authoritative_results: Result<HashMap<String, DiagnosticOutput>, String>,
    },
    DiagnosticRunRejected,
    DiagnosticFinalizationElapsed {
        session_id: String,
    },
    DiagnosticFinalizationCancelled {
        session_id: String,
    },
    DiagnosticFinalizationRejected {
        session_id: String,
    },
    DiagnosticHistorySaveFinished {
        session_id: String,
        result: Result<(), String>,
    },
    DiagnosticHistorySaveWaitCancelled {
        session_id: String,
    },
    DiagnosticHistorySaveRejected {
        session_id: String,
    },
    DiagnosticCancelFinished {
        session_id: String,
        error: Option<String>,
    },
    DiagnosticCancelRejected {
        session_id: String,
    },
    DiagnosticBatch {
        events: Vec<UiEvent>,
        terminated: bool,
    },
    DiagnosticWaitRejected,
    AiStatusFinished {
        request_id: u64,
        result: Result<Box<AIProviderStatus>, String>,
    },
    AiStatusCancelled {
        request_id: u64,
    },
    AiStatusRejected {
        request_id: u64,
    },
    ExportRuntimeCompleted(Box<ExportCompleted>),
    /// The rendered report file was written to the validated user path. The
    /// write happens on a background worker; the error is already a string.
    ExportFileSaved(Box<Result<std::path::PathBuf, String>>),
    ExportWorkerStopped,
    ExportWaitCancelled,
    ExportWaitRejected,
    SetAiMode(AiMode),
    ToggleMonitoring,
    Refresh,
    ProcessFilterChanged(String),
    ProcessSort(ProcessSortKey),
    ProcessPrevious,
    ProcessNext,
    ProcessQueryFinished {
        request_id: u64,
        result: Result<ProcessPage, String>,
    },
    ProcessQueryDiscarded {
        request_id: u64,
    },
    ProcessQueryRejected {
        request_id: u64,
    },
    SelectProcess(Option<u32>),
    RefreshHistory,
    HistoryFilterChanged(String),
    SelectHistory(String),
    HistoryListFinished {
        request_id: u64,
        result: Result<Vec<ScanSummary>, String>,
    },
    HistoryCompareFinished {
        request_id: u64,
        result: Result<Box<ComparisonSummary>, String>,
    },
    HistoryQueryRejected {
        request_id: u64,
        comparison: bool,
    },
    ChatInputChanged(String),
    UsePrompt(String),
    SendChat,
    ChatWorkerEventReceived(Box<ChatWorkerEvent>),
    ChatWorkerStopped,
    ChatWaitCancelled,
    ChatWaitRejected,
    ReportWorkerEventReceived(Box<ReportWorkerEvent>),
    ReportWorkerStopped,
    ReportWaitCancelled,
    ReportWaitRejected,
    GenerateReport,
    CancelReport,
    RunRemediation(String),
    AskAiAboutIssue(String),
    ProposeFixPlan,
    TogglePalette,
    ClosePalette,
    PaletteQueryChanged(String),
    PaletteCommand(String),
    ShowShortcutHelp,
    CloseShortcutHelp,
    ProviderKeyDraftChanged(usize, String),
    StoreProviderKey(usize),
    ClearProviderKey(usize),
    ToggleClearHistoryConfirm(bool),
    ClearHistoryConfirmed,
    HistoryTagDraftChanged(String),
    SaveHistoryTags,
    HistoryAckFinished {
        kind: HistoryAckKind,
        result: Result<(), String>,
    },
    RepairDialogClosed {
        remediation_id: String,
        result: ContentDialogResult,
    },
    ActionWorkerEventReceived(Box<ActionWorkerEvent>),
    ActionWorkerStopped,
    ActionWaitCancelled,
    ActionWaitRejected,
    RestartAsAdmin,
    RestartAsAdminFinished(Result<bool, String>),
    InstanceActivated,
    TrayCommand(u8),
    BackendBatch {
        events: Vec<UiEvent>,
        terminated: bool,
    },
    BackendWaitRejected,
}

#[derive(Clone)]
struct DiagnosticSnapshot {
    results: Vec<DiagnosticTaskResult>,
    scan_kind: Option<ScanKind>,
    duration_ms: u64,
    total: usize,
    completed: usize,
    errors: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingExportAction {
    ShareToWindowsForum,
    /// Write the rendered report to the user-chosen, policy-validated path.
    SaveToFile { path: ValidatedExportPath },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingExport {
    request_id: u64,
    action: PendingExportAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiagnosticScanPolicy {
    auto_save: bool,
    max_concurrent_tasks: usize,
    history_tag: String,
}

impl DiagnosticScanPolicy {
    fn snapshot(settings: &AppSettings, scan_kind: ScanKind) -> Self {
        Self {
            auto_save: settings.auto_save,
            max_concurrent_tasks: scan_concurrency_from_settings(settings.max_concurrent_tasks),
            history_tag: scan_kind_history_tag(scan_kind).to_string(),
        }
    }
}

fn scan_policy_requests_auto_save(policy: Option<&DiagnosticScanPolicy>) -> bool {
    policy.is_some_and(|policy| policy.auto_save)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistoryRetentionPolicy {
    retain_history: bool,
    history_limit: u32,
}

impl From<&AppSettings> for HistoryRetentionPolicy {
    fn from(settings: &AppSettings) -> Self {
        Self {
            retain_history: settings.retain_history,
            history_limit: settings.history_limit,
        }
    }
}

fn history_retention_tuple(policy: &RwLock<HistoryRetentionPolicy>) -> (bool, u32) {
    policy.read().map_or((true, 30), |policy| {
        (policy.retain_history, policy.history_limit)
    })
}

fn update_history_retention_policy(
    policy: &RwLock<HistoryRetentionPolicy>,
    settings: &AppSettings,
) {
    if let Ok(mut policy) = policy.write() {
        *policy = HistoryRetentionPolicy::from(settings);
    }
}

fn authoritative_ui_results(
    session_id: &str,
    results: &HashMap<String, DiagnosticOutput>,
    catalog: &[DiagnosticTask],
) -> Vec<DiagnosticTaskResult> {
    let mut results = results
        .iter()
        .map(|(task_id, result)| DiagnosticTaskResult {
            session_id: session_id.to_string(),
            task_id: task_id.clone(),
            success: result.success,
            output: result.output.clone(),
            error: result.error.clone(),
            duration_ms: result.duration_ms,
        })
        .collect::<Vec<_>>();
    results.sort_by_key(|result| {
        catalog
            .iter()
            .position(|task| task.id == result.task_id)
            .unwrap_or(usize::MAX)
    });
    results
}

fn authoritative_result_set_is_complete(
    results: &HashMap<String, DiagnosticOutput>,
    expected_task_ids: &[String],
) -> bool {
    let expected = expected_task_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    expected.len() == expected_task_ids.len()
        && results.len() == expected_task_ids.len()
        && results
            .keys()
            .all(|task_id| expected.contains(task_id.as_str()))
}

fn build_history_scan_record(
    session_id: String,
    system_info: &SystemInfo,
    results: &[DiagnosticTaskResult],
    duration_ms: u64,
    history_tag: String,
) -> ScanRecord {
    let results = results
        .iter()
        .map(|result| {
            (
                result.task_id.clone(),
                HistoryTaskResult {
                    success: result.success,
                    output: result.output.clone(),
                    error: result.error.clone(),
                    duration_ms: result.duration_ms,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let success_count = results.values().filter(|result| result.success).count();
    let failure_count = results.len().saturating_sub(success_count);

    ScanRecord {
        id: session_id,
        timestamp: Timestamp::now(),
        computer_name: system_info.computer_name.clone(),
        os_version: system_info.os_version.clone(),
        is_admin: system_info.is_admin,
        task_count: results.len(),
        success_count,
        failure_count,
        results,
        duration_ms,
        label: None,
        tags: vec![history_tag],
    }
}

struct PendingSettingsSave {
    request_id: u64,
    dialog_epoch: u64,
    submitted: AppSettings,
}

fn settings_dialog_callback_is_current(
    settings_open: bool,
    current_epoch: u64,
    callback_epoch: u64,
) -> bool {
    settings_open && current_epoch == callback_epoch
}

fn about_dialog_callback_is_current(
    about_open: bool,
    current_epoch: u64,
    callback_epoch: u64,
) -> bool {
    about_open && current_epoch == callback_epoch
}

fn update_notice_remaining_after_elapsed(remaining: Duration, elapsed: Duration) -> Duration {
    remaining.saturating_sub(elapsed)
}

fn update_notice_timer_callback_is_current(
    notice_visible: bool,
    current_epoch: u64,
    current_timer_generation: u64,
    callback_epoch: u64,
    callback_timer_generation: u64,
) -> bool {
    notice_visible
        && current_epoch == callback_epoch
        && current_timer_generation == callback_timer_generation
}

fn take_matching_pending_settings_save(
    pending: &mut Option<PendingSettingsSave>,
    request_id: u64,
) -> Option<PendingSettingsSave> {
    pending.take_if(|candidate| candidate.request_id == request_id)
}

fn take_matching_system_request(
    system_info_request_id: &mut Option<u64>,
    architecture_request_id: &mut Option<u64>,
    request_id: u64,
) -> Option<SystemRequestKind> {
    if system_info_request_id
        .take_if(|pending| *pending == request_id)
        .is_some()
    {
        Some(SystemRequestKind::SystemInfo)
    } else if architecture_request_id
        .take_if(|pending| *pending == request_id)
        .is_some()
    {
        Some(SystemRequestKind::Architecture)
    } else {
        None
    }
}

fn fixture_258_system_info() -> SystemInfo {
    SystemInfo {
        computer_name: "ANDROMEDA".to_string(),
        os_version: "Windows 11 Professional (25H2)".to_string(),
        is_admin: false,
    }
}

fn pending_system_info() -> SystemInfo {
    SystemInfo {
        computer_name: "This PC".to_string(),
        os_version: "Windows".to_string(),
        is_admin: false,
    }
}

fn privilege_label(is_admin: bool) -> &'static str {
    if is_admin {
        "Administrator"
    } else {
        "Standard user"
    }
}

fn machine_card_accessibility_name(
    system_info: &SystemInfo,
    architecture: Option<&ArchitectureSnapshot>,
    system_error: Option<&str>,
) -> String {
    let mut label = format!(
        "Computer {}, {}, {}",
        system_info.computer_name,
        system_info.os_version,
        privilege_label(system_info.is_admin)
    );
    if let Some(architecture) = architecture {
        label.push_str(", ");
        label.push_str(&architecture.emulation_status);
    }
    if let Some(error) = system_error {
        label.push_str(", system information warning: ");
        label.push_str(error);
    }
    label
}

struct WfdiagSpike {
    page: Page,
    theme: WindowTheme,
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
    issue_source_results: Option<Arc<HashMap<String, DiagnosticOutput>>>,
    issue_projected_epoch: Option<u64>,
    issue_projected_session_id: Option<String>,
    issue_pending: Option<PendingIssueDetection>,
    issue_enqueued_request_id: Option<u64>,
    issue_error: Option<String>,
    monitoring_paused: bool,
    process_filter: String,
    process_page: Option<ProcessPage>,
    process_sort_key: ProcessSortKey,
    process_sort_direction: ProcessSortDirection,
    process_offset: usize,
    process_request_id: u64,
    process_request_task: Option<ComponentTask>,
    process_loading: bool,
    process_error: Option<String>,
    selected_process_pid: Option<u32>,
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
    history_loading: bool,
    history_error: Option<String>,
    chat_input: String,
    chat_answer: Option<String>,
    chat_runtime: Option<NativeChatRuntime>,
    chat_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<ChatWorkerEvent>>>>,
    chat_wait: Option<ComponentTask>,
    chat_request_id: u64,
    chat_pending: Option<u64>,
    report_runtime: Option<NativeReportRuntime>,
    report_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<ReportWorkerEvent>>>>,
    report_wait: Option<ComponentTask>,
    report_request_id: u64,
    report_pending: Option<u64>,
    report_text: Option<String>,
    report_provider: Option<String>,
    report_error: Option<String>,
    action_runtime: Option<NativeActionRuntime>,
    action_receiver:
        Option<Arc<Mutex<std::sync::mpsc::Receiver<ActionWorkerEvent>>>>,
    action_wait: Option<ComponentTask>,
    action_request_id: u64,
    action_pending: Option<u64>,
    repair_confirm: Option<RemediationSummary>,
    admin_relaunch_task: Option<ComponentTask>,
    instance_wait: Option<ComponentTask>,
    palette_open: bool,
    palette_query: String,
    shortcut_help_open: bool,
    window_hook_installed: bool,
    provider_key_drafts: [String; 4],
    provider_key_busy: bool,
    history_clear_confirm: bool,
    history_tag_draft: String,
    history_ack_busy: bool,
    history_wait: Option<ComponentTask>,
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
    native_monitor: Option<Arc<NativeMonitorRuntime>>,
    backend_receiver: Option<Arc<UiEventReceiver>>,
    backend_wait: Option<ComponentTask>,
    visual_state: VisualState,
}

fn initial_window_dimension(name: &str, fallback: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 540.0)
        .unwrap_or(fallback)
}

fn fixture_system_stats() -> SystemStats {
    SystemStats {
        cpu_utilization: 10.3,
        per_cpu_utilization: vec![10.3; 12],
        cpu_frequency: 2_980,
        memory_total_gb: 63.5,
        memory_used_gb: 51.4,
        memory_available_gb: 12.1,
        memory_utilization: 81.0,
        swap_total_gb: 8.0,
        swap_used_gb: 1.0,
        swap_utilization: 12.5,
        storage_used_percent: 64.3,
        disk_utilization: 64.3,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        disks: Vec::new(),
        network_upload_kb: 150.0,
        network_download_kb: 679.44,
        gpu_available: true,
        gpu_name: Some("Adreno 741".to_string()),
        gpu_utilization: Some(23.9),
        gpu_memory_used_mb: 2.17 * 1024.0,
        gpu_memory_total_mb: 8.12 * 1024.0,
        npu_available: true,
        npu_name: Some(
            "Snapdragon(R) X Elite - X1E80100 - Qualcomm(R) Hexagon(TM) NPU".to_string(),
        ),
        npu_utilization: Some(0.0),
        npu_memory_used_mb: 0.0,
        npu_memory_total_mb: 0.0,
        top_processes: Vec::new(),
        timestamp: 0,
    }
}

fn fixture_monitor_empty_stats() -> SystemStats {
    SystemStats {
        cpu_utilization: 71.2,
        per_cpu_utilization: vec![71.2; 12],
        cpu_frequency: 2_980,
        memory_total_gb: 63.5,
        memory_used_gb: 51.8,
        memory_available_gb: 11.7,
        memory_utilization: 81.6,
        swap_total_gb: 8.0,
        swap_used_gb: 1.0,
        swap_utilization: 12.5,
        storage_used_percent: 0.0,
        disk_utilization: 0.0,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        disks: Vec::new(),
        network_upload_kb: 0.0,
        network_download_kb: 0.0,
        gpu_available: true,
        gpu_name: Some("Adreno 741".to_string()),
        gpu_utilization: Some(0.0),
        gpu_memory_used_mb: 2.21 * 1024.0,
        gpu_memory_total_mb: 8.12 * 1024.0,
        npu_available: true,
        npu_name: Some(
            "Snapdragon(R) X Elite - X1E80100 - Qualcomm(R) Hexagon(TM) NPU".to_string(),
        ),
        npu_utilization: Some(0.0),
        npu_memory_used_mb: 0.0,
        npu_memory_total_mb: 0.0,
        top_processes: Vec::new(),
        timestamp: 0,
    }
}

const MONITOR_HISTORY_SAMPLES: usize = 60;

#[derive(Clone, Copy, Debug, Default)]
struct MonitorSample {
    cpu: f64,
    memory: f64,
    storage: f64,
    network_mb: f64,
    gpu: f64,
    npu: f64,
}

impl MonitorSample {
    fn from_stats(stats: &SystemStats) -> Self {
        Self {
            cpu: f64::from(stats.cpu_utilization),
            memory: f64::from(stats.memory_utilization),
            storage: f64::from(stats.storage_used_percent),
            network_mb: (stats.network_upload_kb + stats.network_download_kb) / 1024.0,
            gpu: f64::from(stats.gpu_utilization.unwrap_or_default()),
            npu: f64::from(stats.npu_utilization.unwrap_or_default()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MonitorMetric {
    Cpu,
    Memory,
    Storage,
    Network,
    Gpu,
    Npu,
}

#[derive(Debug, Default)]
struct MonitorHistory {
    samples: VecDeque<MonitorSample>,
}

impl MonitorHistory {
    fn push_stats(&mut self, stats: &SystemStats) {
        self.samples.push_back(MonitorSample::from_stats(stats));
        while self.samples.len() > MONITOR_HISTORY_SAMPLES {
            self.samples.pop_front();
        }
    }

    fn series(&self, metric: MonitorMetric) -> Vec<f64> {
        self.samples
            .iter()
            .map(|sample| match metric {
                MonitorMetric::Cpu => sample.cpu,
                MonitorMetric::Memory => sample.memory,
                MonitorMetric::Storage => sample.storage,
                MonitorMetric::Network => sample.network_mb,
                MonitorMetric::Gpu => sample.gpu,
                MonitorMetric::Npu => sample.npu,
            })
            .collect()
    }

    fn fixture_258() -> Self {
        let cpu = [10.3, 63.0, 20.0, 17.0, 10.3];
        let memory = [81.0; 5];
        let storage = [64.3; 5];
        let network = [0.60, 1.15, 0.55, 2.00, 0.81];
        let gpu = [21.0, 27.0, 23.0, 23.0, 23.9];
        let npu = [0.0; 5];
        let samples = (0..5)
            .map(|index| MonitorSample {
                cpu: cpu[index],
                memory: memory[index],
                storage: storage[index],
                network_mb: network[index],
                gpu: gpu[index],
                npu: npu[index],
            })
            .collect();
        Self { samples }
    }
}

fn spawn_backend_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<UiEventReceiver>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            while !cancellation.is_cancelled()
                && !receiver.wait_for_events_timeout(Duration::from_millis(100))
            {}
            let events = receiver.drain();
            let terminated = receiver.is_terminated();
            Message::BackendBatch { events, terminated }
        },
        Message::BackendWaitRejected,
    )
}

fn spawn_diagnostic_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<UiEventReceiver>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            while !cancellation.is_cancelled()
                && !receiver.wait_for_events_timeout(Duration::from_millis(100))
            {}
            let events = receiver.drain();
            let terminated = receiver.is_terminated();
            Message::DiagnosticBatch { events, terminated }
        },
        Message::DiagnosticWaitRejected,
    )
}

fn spawn_ai_status_wait(
    context: &ComponentContext<WfdiagSpike>,
    request_id: u64,
    mut reply: ProviderStatusReply,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::AiStatusCancelled { request_id };
            }
            match reply.try_recv() {
                Ok(status) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Ok(Box::new(status)),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Err(
                            "Native AI provider worker stopped before replying".to_string(),
                        ),
                    };
                }
            }
        },
        Message::AiStatusRejected { request_id },
    )
}

fn spawn_ai_preference_status_wait(
    context: &ComponentContext<WfdiagSpike>,
    request_id: u64,
    mut reply: ProviderPreferenceStatusReply,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::AiStatusCancelled { request_id };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: result.map(Box::new),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Err("Native AI provider worker stopped before applying Settings"
                            .to_string()),
                    };
                }
            }
        },
        Message::AiStatusRejected { request_id },
    )
}

fn spawn_diagnostic_finalization_delay(
    context: &ComponentContext<WfdiagSpike>,
    session_id: String,
) -> ComponentTask {
    let rejection_session_id = session_id.clone();
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + SCAN_FINALIZATION_DELAY;
            loop {
                if cancellation.is_cancelled() {
                    return Message::DiagnosticFinalizationCancelled { session_id };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::DiagnosticFinalizationElapsed { session_id };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
        },
        Message::DiagnosticFinalizationRejected {
            session_id: rejection_session_id,
        },
    )
}

fn spawn_history_ack_wait(
    context: &ComponentContext<WfdiagSpike>,
    kind: HistoryAckKind,
    mut reply: HistoryReply<()>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::HistoryAckFinished {
                    kind,
                    result: Err("The Reactor background queue rejected the request".to_string()),
                };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::HistoryAckFinished {
                        kind,
                        result: result.map_err(|error| error.to_string()),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::HistoryAckFinished {
                        kind,
                        result: Err("Native history worker stopped".to_string()),
                    };
                }
            }
        },
        Message::HistoryAckFinished {
            kind,
            result: Err("The Reactor background queue rejected the request".to_string()),
        },
    )
}

fn spawn_history_save_wait(
    context: &ComponentContext<WfdiagSpike>,
    session_id: String,
    mut reply: HistoryReply<()>,
) -> ComponentTask {
    let rejection_session_id = session_id.clone();
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::DiagnosticHistorySaveWaitCancelled { session_id };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::DiagnosticHistorySaveFinished { session_id, result };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::DiagnosticHistorySaveFinished {
                        session_id,
                        result: Err(
                            "native history worker stopped before acknowledging the scan"
                                .to_string(),
                        ),
                    };
                }
            }
        },
        Message::DiagnosticHistorySaveRejected {
            session_id: rejection_session_id,
        },
    )
}

fn spawn_settings_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<mpsc::Receiver<SettingsEvent>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::SettingsWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::SettingsWorkerStopped,
            };
            match received {
                Ok(event) => return Message::SettingsRuntimeEvent(Box::new(event)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::SettingsWorkerStopped;
                }
            }
        },
        Message::SettingsWaitRejected,
    )
}

fn spawn_system_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<mpsc::Receiver<SystemCompleted>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::SystemWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::SystemWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::SystemRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::SystemWorkerStopped;
                }
            }
        },
        Message::SystemWaitRejected,
    )
}

fn spawn_issue_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<mpsc::Receiver<IssueDetectionCompleted>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::IssueWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::IssueWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::IssueRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::IssueWorkerStopped;
                }
            }
        },
        Message::IssueWaitRejected,
    )
}

fn spawn_chat_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<ChatWorkerEvent>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ChatWaitCancelled;
            }
            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(CHAT_WAIT_POLL),
                Err(_) => return Message::ChatWorkerStopped,
            };
            match received {
                Ok(event) => return Message::ChatWorkerEventReceived(Box::new(event)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::ChatWorkerStopped;
                }
            }
        },
        Message::ChatWaitRejected,
    )
}

fn spawn_instance_watch(context: &ComponentContext<WfdiagSpike>) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ActionWaitCancelled;
            }
            if instance_support::activation_requested() {
                return Message::InstanceActivated;
            }
            let command = window_support::take_tray_command();
            if command != window_support::TRAY_COMMAND_NONE {
                return Message::TrayCommand(command);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        },
        Message::ActionWaitCancelled,
    )
}

fn spawn_action_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<ActionWorkerEvent>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ActionWaitCancelled;
            }
            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(CHAT_WAIT_POLL),
                Err(_) => return Message::ActionWorkerStopped,
            };
            match received {
                Ok(event) => return Message::ActionWorkerEventReceived(Box::new(event)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::ActionWorkerStopped;
                }
            }
        },
        Message::ActionWaitRejected,
    )
}

/// `relaunch_self_elevated` blocks on the UAC prompt and COM; keep it off
/// the WinUI thread.
fn spawn_relaunch_as_admin(context: &ComponentContext<WfdiagSpike>) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |_cancellation| {
            Message::RestartAsAdminFinished(
                wfdiag_native_remediation::elevation::relaunch_self_elevated(),
            )
        },
        Message::RestartAsAdminFinished(Err(
            "The Reactor background queue rejected the elevation hand-off".to_string(),
        )),
    )
}

fn spawn_report_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<ReportWorkerEvent>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ReportWaitCancelled;
            }
            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(CHAT_WAIT_POLL),
                Err(_) => return Message::ReportWorkerStopped,
            };
            match received {
                Ok(event) => return Message::ReportWorkerEventReceived(Box::new(event)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::ReportWorkerStopped;
                }
            }
        },
        Message::ReportWaitRejected,
    )
}

fn spawn_export_wait(
    context: &ComponentContext<WfdiagSpike>,
    receiver: Arc<Mutex<mpsc::Receiver<ExportCompleted>>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ExportWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::ExportWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::ExportRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::ExportWorkerStopped;
                }
            }
        },
        Message::ExportWaitRejected,
    )
}

/// User-facing label for a report format, matching the save-dialog filter
/// names.
#[must_use]
const fn export_format_label(format: ReportFormat) -> &'static str {
    match format {
        ReportFormat::Json => "JSON",
        ReportFormat::Text => "TXT",
        ReportFormat::Html => "HTML",
    }
}

fn spawn_export_file_write(
    context: &ComponentContext<WfdiagSpike>,
    path: ValidatedExportPath,
    content: String,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            // The path was policy-validated on the UI thread by the save
            // picker; only the (potentially large) file write happens here.
            let result = if cancellation.is_cancelled() {
                Err("The export was cancelled".to_string())
            } else {
                std::fs::write(path.as_path(), content)
                    .map(|()| path.into_path())
                    .map_err(|error| error.to_string())
            };
            Message::ExportFileSaved(Box::new(result))
        },
        Message::ExportWaitRejected,
    )
}

fn spawn_issue_request_preparation(
    context: &ComponentContext<WfdiagSpike>,
    pending: PendingIssueDetection,
    results: Arc<HashMap<String, DiagnosticOutput>>,
) -> ComponentTask {
    let rejected = pending.clone();
    context.spawn_background_with_rejection(
        move |cancellation| {
            if cancellation.is_cancelled() {
                return Message::IssueRequestPreparationCancelled(pending);
            }

            // These two OS-dependent inputs and the potentially large result
            // map clone stay off the Reactor UI thread.
            let now = IssueTimestamp::now();
            let temp_file_count = std::fs::read_dir(std::env::temp_dir())
                .ok()
                .map(|entries| entries.count());
            let prepared = prepare_issue_detection(
                pending.request_id,
                pending.committed_epoch,
                pending.session_id.clone(),
                (*results).clone(),
                now,
                temp_file_count,
            );
            if cancellation.is_cancelled() {
                Message::IssueRequestPreparationCancelled(prepared.pending)
            } else {
                Message::IssueRequestPrepared(Box::new(prepared))
            }
        },
        Message::IssueRequestPreparationRejected(rejected),
    )
}

fn spawn_update_delay(context: &ComponentContext<WfdiagSpike>) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            if cancellation.is_cancelled() {
                return Message::UpdateDelayCancelled;
            }
            let throttle = reactor_update_throttle();
            if throttle
                .as_ref()
                .is_some_and(|throttle| !throttle.should_check_at(unix_time_millis()))
            {
                return Message::UpdateStartupSkipped;
            }

            let deadline = Instant::now() + START_DELAY;
            loop {
                if cancellation.is_cancelled() {
                    return Message::UpdateDelayCancelled;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::UpdateStartupDue { throttle };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::UpdateDelayRejected,
    )
}

fn spawn_update_wait(
    context: &ComponentContext<WfdiagSpike>,
    mut reply: UpdateReply,
    throttle: Option<UpdateThrottle>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::UpdateCheckCancelled;
            }
            match reply.try_recv() {
                Ok(update) => {
                    // Match the Store hook: a completed backend check consumes
                    // the daily attempt even when its deliberately silent
                    // result is `None`. Persistence failure remains fail-open.
                    if let Some(throttle) = throttle.as_ref() {
                        let _ = throttle.record_at(unix_time_millis());
                    }
                    return Message::UpdateCheckFinished(Ok(update));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::UpdateCheckFinished(Err(
                        "Native update worker stopped before replying".to_string(),
                    ));
                }
            }
        },
        Message::UpdateCheckRejected,
    )
}

fn spawn_update_notice_timer(
    context: &ComponentContext<WfdiagSpike>,
    epoch: u64,
    timer_generation: u64,
    duration: Duration,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + duration;
            loop {
                if cancellation.is_cancelled() {
                    return Message::UpdateNoticeTimerCancelled {
                        epoch,
                        timer_generation,
                    };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::UpdateNoticeExpired {
                        epoch,
                        timer_generation,
                    };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::UpdateNoticeTimerRejected {
            epoch,
            timer_generation,
        },
    )
}

fn spawn_about_external_action(
    context: &ComponentContext<WfdiagSpike>,
    epoch: u64,
    action: AboutExternalAction,
    update: Option<UpdateInfo>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |_| Message::AboutExternalFinished {
            epoch,
            result: launch_external_action(action, update.as_ref()),
        },
        Message::AboutExternalRejected { epoch },
    )
}

impl WfdiagSpike {
    fn next_ai_status_request_id(&mut self) -> u64 {
        self.ai_status_request_id = self.ai_status_request_id.wrapping_add(1);
        if self.ai_status_request_id == 0 {
            self.ai_status_request_id = 1;
        }
        self.ai_status_request_id
    }

    fn request_ai_provider_status(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            return;
        }

        if let Some(task) = self.ai_status_task.take() {
            task.cancel();
        }
        let request_id = self.next_ai_status_request_id();
        if !self.ai_settings_ready {
            self.ai_provider_status = None;
            self.ai_status_loading = self.settings_loading;
            self.ai_status_error =
                (!self.settings_loading).then(|| "AI settings could not be loaded".to_string());
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = None;
            return;
        }
        let Some(runtime) = self.ai_provider_runtime.as_ref() else {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = Some("Native AI provider discovery is unavailable".to_string());
            return;
        };
        match runtime.request_status() {
            Ok(reply) => {
                self.ai_status_loading = true;
                self.ai_status_error = None;
                self.ai_status_task = Some(spawn_ai_status_wait(context, request_id, reply));
            }
            Err(error) => {
                self.ai_provider_status = None;
                self.ai_status_loading = false;
                self.ai_status_error = Some(error.to_string());
            }
        }
    }

    fn sync_ai_provider_preference(&mut self, preference: &str, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            if let Some(task) = self.ai_status_task.take() {
                task.cancel();
            }
            self.next_ai_status_request_id();
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = None;
            return;
        }
        if let Some(task) = self.ai_status_task.take() {
            task.cancel();
        }
        let request_id = self.next_ai_status_request_id();
        let Some(runtime) = self.ai_provider_runtime.as_ref() else {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = Some("Native AI provider discovery is unavailable".to_string());
            return;
        };
        match runtime.request_set_preference_and_status(preference.to_string()) {
            Ok(reply) => {
                self.ai_provider_status = None;
                self.ai_status_loading = true;
                self.ai_status_error = None;
                self.ai_status_task =
                    Some(spawn_ai_preference_status_wait(context, request_id, reply));
            }
            Err(error) => {
                self.ai_provider_status = None;
                self.ai_status_loading = false;
                self.ai_status_error = Some(error.to_string());
            }
        }
    }

    fn next_update_notice_timer_generation(&mut self) -> u64 {
        self.update_notice_timer_generation = self.update_notice_timer_generation.wrapping_add(1);
        if self.update_notice_timer_generation == 0 {
            self.update_notice_timer_generation = 1;
        }
        self.update_notice_timer_generation
    }

    fn next_about_dialog_epoch(&mut self) -> u64 {
        self.about_dialog_epoch = self.about_dialog_epoch.wrapping_add(1);
        if self.about_dialog_epoch == 0 {
            self.about_dialog_epoch = 1;
        }
        self.about_dialog_epoch
    }

    fn about_dialog_is_current(&self, epoch: u64) -> bool {
        about_dialog_callback_is_current(self.about_open, self.about_dialog_epoch, epoch)
    }

    fn open_about(&mut self) {
        if self.about_open || self.settings_open || self.settings_saving {
            return;
        }
        self.next_about_dialog_epoch();
        self.about_action_error = None;
        self.about_open = true;
    }

    fn close_about(&mut self, epoch: u64) {
        if !self.about_dialog_is_current(epoch) {
            return;
        }
        if let Some(task) = self.about_launch_task.take() {
            task.cancel();
        }
        self.about_action_error = None;
        self.about_open = false;
    }

    fn request_about_external_action(
        &mut self,
        epoch: u64,
        action: AboutExternalAction,
        context: &ComponentContext<Self>,
    ) {
        if !self.about_dialog_is_current(epoch) || self.about_launch_task.is_some() {
            return;
        }
        self.about_action_error = None;
        self.about_launch_task = Some(spawn_about_external_action(
            context,
            epoch,
            action,
            self.update_info.clone(),
        ));
    }

    fn begin_update_check(
        &mut self,
        throttle: Option<UpdateThrottle>,
        context: &ComponentContext<Self>,
    ) {
        self.update_delay_task = None;
        if self.deterministic_visual
            || self.update_runtime.is_some()
            || self.update_check_task.is_some()
        {
            return;
        }

        let Ok(service) = UpdateService::shipping_from_str(APP_VERSION, cfg!(debug_assertions))
        else {
            return;
        };
        let Ok(runtime) = NativeUpdateRuntime::start(service) else {
            return;
        };
        let Ok(reply) = runtime.request_check() else {
            return;
        };

        self.update_check_task = Some(spawn_update_wait(context, reply, throttle));
        self.update_runtime = Some(runtime);
    }

    fn apply_update_check_result(
        &mut self,
        result: Result<Option<UpdateInfo>, String>,
        context: &ComponentContext<Self>,
    ) {
        self.update_check_task = None;
        self.update_runtime = None;

        let Ok(Some(update)) = result else {
            // Store builds, debug builds, offline hosts, malformed responses,
            // and runtime failures all intentionally remain invisible.
            return;
        };
        if trusted_release_url(&update).is_none() {
            return;
        }

        self.update_info = Some(update);
        self.update_notice_epoch = self.update_notice_epoch.wrapping_add(1);
        if self.update_notice_epoch == 0 {
            self.update_notice_epoch = 1;
        }
        let epoch = self.update_notice_epoch;
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        self.update_notice_visible = true;
        self.update_notice_started_at = Some(Instant::now());
        self.update_notice_remaining = NOTICE_DURATION;
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice_task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice_remaining,
        ));
    }

    fn close_update_notice(&mut self, epoch: u64) {
        if !self.update_notice_visible || self.update_notice_epoch != epoch {
            return;
        }
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        self.update_notice_visible = false;
        self.update_notice_started_at = None;
        self.update_notice_remaining = Duration::ZERO;
    }

    fn pause_update_notice(&mut self, epoch: u64) {
        if !self.update_notice_visible || self.update_notice_epoch != epoch {
            return;
        }
        let Some(started_at) = self.update_notice_started_at.take() else {
            return;
        };
        self.update_notice_remaining = update_notice_remaining_after_elapsed(
            self.update_notice_remaining,
            started_at.elapsed(),
        );
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        // Invalidate every completion from the cancelled timer immediately;
        // resume may be queued behind that completion on the UI dispatcher.
        self.next_update_notice_timer_generation();
        if self.update_notice_remaining.is_zero() {
            self.update_notice_visible = false;
        }
    }

    fn resume_update_notice(&mut self, epoch: u64, context: &ComponentContext<Self>) {
        if !self.update_notice_visible
            || self.update_notice_epoch != epoch
            || self.update_notice_started_at.is_some()
        {
            return;
        }
        if self.update_notice_remaining.is_zero() {
            self.update_notice_visible = false;
            return;
        }
        self.update_notice_started_at = Some(Instant::now());
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice_task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice_remaining,
        ));
    }

    fn next_settings_request_id(&mut self) -> u64 {
        self.settings_request_id = self.settings_request_id.wrapping_add(1);
        if self.settings_request_id == 0 {
            self.settings_request_id = 1;
        }
        self.settings_request_id
    }

    fn next_settings_dialog_epoch(&mut self) -> u64 {
        self.settings_dialog_epoch = self.settings_dialog_epoch.wrapping_add(1);
        if self.settings_dialog_epoch == 0 {
            self.settings_dialog_epoch = 1;
        }
        self.settings_dialog_epoch
    }

    fn settings_dialog_is_current(&self, epoch: u64) -> bool {
        settings_dialog_callback_is_current(self.settings_open, self.settings_dialog_epoch, epoch)
    }

    fn resume_settings_wait(&mut self, context: &ComponentContext<Self>) {
        let Some(receiver) = self.settings_receiver.as_ref().map(Arc::clone) else {
            self.settings_wait = None;
            return;
        };
        self.settings_wait = Some(spawn_settings_wait(context, receiver));
    }

    fn resume_system_wait(&mut self, context: &ComponentContext<Self>) {
        if self.system_info_request_id.is_none() && self.architecture_request_id.is_none() {
            self.system_wait = None;
            return;
        }
        let Some(receiver) = self.system_receiver.as_ref().map(Arc::clone) else {
            self.system_wait = None;
            return;
        };
        self.system_wait = Some(spawn_system_wait(context, receiver));
    }

    fn resume_issue_wait(&mut self, context: &ComponentContext<Self>) {
        let has_enqueued_current_request = self
            .issue_pending
            .as_ref()
            .is_some_and(|pending| self.issue_enqueued_request_id == Some(pending.request_id));
        if self.issue_wait.is_some() || !has_enqueued_current_request {
            return;
        }
        let Some(receiver) = self.issue_receiver.as_ref().map(Arc::clone) else {
            self.issue_wait = None;
            return;
        };
        self.issue_wait = Some(spawn_issue_wait(context, receiver));
    }

    fn resume_chat_wait(&mut self, context: &ComponentContext<Self>) {
        if self.chat_wait.is_some() {
            return;
        }
        if let Some(receiver) = self.chat_receiver.as_ref() {
            self.chat_wait = Some(spawn_chat_wait(context, Arc::clone(receiver)));
        }
    }

    /// Store or clear a provider credential through the settings worker.
    /// The draft text never persists to settings.json — only the DPAPI
    /// credential file for the provider's closed key id.
    fn submit_provider_key(&mut self, index: usize, store: bool) {
        const KEY_IDS: [ProviderKeyId; 4] = [
            ProviderKeyId::OpenAI,
            ProviderKeyId::Anthropic,
            ProviderKeyId::Gemini,
            ProviderKeyId::Custom,
        ];
        let Some(provider) = KEY_IDS.get(index).copied() else {
            return;
        };
        if self.deterministic_visual {
            self.status =
                "Visual fixture mode · credential changes are disabled".to_string();
            return;
        }
        let key = self.provider_key_drafts[index].clone();
        if store && key.trim().is_empty() {
            self.status = "Enter an API key first".to_string();
            return;
        }
        let request_id = self.next_settings_request_id();
        // The settings worker borrow is taken last: nothing above may hold a
        // borrow across the &mut request-id bump.
        let Some(settings_runtime) = self.settings_runtime.as_ref() else {
            self.status = "Native settings persistence is unavailable".to_string();
            return;
        };
        let command = if store {
            SettingsCommand::StoreProviderKey {
                request_id,
                provider,
                key,
            }
        } else {
            SettingsCommand::ClearProviderKey { request_id, provider }
        };
        if let Err(error) = settings_runtime.send(command) {
            self.status = format!("Credential change failed: {error}");
            return;
        }
        self.provider_key_busy = true;
        self.status = if store {
            "Saving API key…".to_string()
        } else {
            "Clearing API key…".to_string()
        };
    }

    /// Save the tag draft for the selected scan through the history worker.
    fn request_history_tags_save(&mut self, context: &ComponentContext<Self>) {
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
                self.history_wait = Some(spawn_history_ack_wait(
                    context,
                    HistoryAckKind::Tags,
                    reply,
                ));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Clear all stored scans after the explicit confirmation dialog.
    fn request_history_clear(&mut self, context: &ComponentContext<Self>) {
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

    /// Install the tray + close-to-tray hook once the WinUI window exists.
    /// Runs on the UI thread (subclassing requires the owning thread); the
    /// bool guard makes it a cheap no-op afterwards.
    fn ensure_window_hook(&mut self) {
        if self.window_hook_installed || self.deterministic_visual {
            return;
        }
        let Some(window) = instance_support::main_window_hwnd() else {
            return;
        };
        if let Err(error) =
            window_support::install(window, "WindowsForum Diagnostics")
        {
            self.status = error;
            self.window_hook_installed = true;
            return;
        }
        window_support::set_close_to_tray(self.settings_snapshot.close_to_tray);
        self.window_hook_installed = true;
    }

    /// Execute one command-palette entry. Page tags reuse the navigation
    /// path; action tags mirror their titlebar/nav equivalents.
    fn handle_palette_command(&mut self, tag: String, context: &ComponentContext<Self>) {
        if let Some(page) = Page::from_tag(&tag) {
            let entering_processes = page == Page::Processes && self.page != Page::Processes;
            let entering_history = page == Page::History && self.page != Page::History;
            let entering_ai = page == Page::Ai && self.page != Page::Ai;
            self.page = page;
            if entering_processes {
                self.process_offset = 0;
                self.selected_process_pid = None;
                self.request_process_page(context, false);
            }
            if entering_history {
                self.request_history_list(context);
            }
            if entering_ai {
                self.request_ai_provider_status(context);
            }
            return;
        }
        match tag.as_str() {
            "quick-scan" => {
                self.page = Page::Diagnostics;
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            "full-scan" => {
                self.page = Page::Diagnostics;
                self.begin_diagnostic_scan(ScanKind::Full, context);
            }
            "export" => self.request_export_to_file(context),
            "share" => self.request_share_to_windowsforum(context),
            "settings" => self.open_settings(),
            "about" => self.open_about(),
            "shortcut-help" => self.shortcut_help_open = true,
            "toggle-theme" => {
                let next = match self.theme {
                    WindowTheme::Dark => "light",
                    _ => "dark",
                };
                self.settings_snapshot.theme = next.to_string();
                self.theme = match self.theme {
                    WindowTheme::Dark => WindowTheme::Light,
                    _ => WindowTheme::Dark,
                };
            }
            _ => (),
        }
    }

    /// Dispatch an authorized catalog execution. The confirmed flag is only
    /// ever true after the Repair confirmation dialog's Primary button.
    fn execute_remediation(
        &mut self,
        remediation_id: String,
        confirmed: bool,
        context: &ComponentContext<Self>,
    ) {
        let Some(runtime) = self.action_runtime.as_ref() else {
            self.status = "Native remediation is unavailable".to_string();
            return;
        };
        if self.action_pending.is_some() {
            self.status = "A remediation is already running…".to_string();
            return;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.action_request_id) else {
            self.status = "Native remediation request identity was exhausted".to_string();
            return;
        };
        self.action_pending = Some(request_id);
        runtime.execute(request_id, remediation_id.clone(), confirmed);
        self.status = format!("Running '{remediation_id}'…");
        self.resume_action_wait(context);
    }

    fn resume_action_wait(&mut self, context: &ComponentContext<Self>) {
        if self.action_wait.is_some() {
            return;
        }
        if let Some(receiver) = self.action_receiver.as_ref() {
            self.action_wait = Some(spawn_action_wait(context, Arc::clone(receiver)));
        }
    }

    /// Fire-and-forget scan-completion toast, mirroring the shipping
    /// plugin's behavior when notifications are enabled. Best effort: any
    /// failure is silent (the scan itself succeeded).
    fn notify_scan_completion(&self) {
        if self.deterministic_visual || !self.settings_snapshot.show_notifications {
            return;
        }
        let collected = self.diagnostic_results.len();
        let errors = self
            .diagnostic_results
            .iter()
            .filter(|result| !result.success)
            .count();
        let _ = std::thread::Builder::new()
            .name("wfdiag-reactor-toast".to_string())
            .spawn(move || {
                let _ = notification_support::show_scan_complete_toast(collected, errors);
            });
    }

    fn resume_report_wait(&mut self, context: &ComponentContext<Self>) {
        if self.report_wait.is_some() {
            return;
        }
        if let Some(receiver) = self.report_receiver.as_ref() {
            self.report_wait = Some(spawn_report_wait(context, Arc::clone(receiver)));
        }
    }

    fn resume_export_wait(&mut self, context: &ComponentContext<Self>) {
        if self.export_wait.is_some() || self.export_pending.is_none() {
            return;
        }
        let Some(receiver) = self.export_receiver.as_ref().map(Arc::clone) else {
            self.export_wait = None;
            return;
        };
        self.export_wait = Some(spawn_export_wait(context, receiver));
    }

    fn export_results_snapshot(&self) -> Option<Arc<HashMap<String, ExportTaskResult>>> {
        let current_session = self
            .diagnostic_results
            .first()
            .map(|result| result.session_id.as_str())?;

        if !self.diagnostics_busy()
            && self.issue_source_session_id.as_deref() == Some(current_session)
            && self
                .issue_source_results
                .as_ref()
                .is_some_and(|results| results.len() == self.diagnostic_results.len())
        {
            return self.issue_source_results.as_ref().map(Arc::clone);
        }

        Some(Arc::new(
            self.diagnostic_results
                .iter()
                .map(|result| {
                    (
                        result.task_id.clone(),
                        ExportTaskResult {
                            success: result.success,
                            output: result.output.clone(),
                            error: result.error.clone(),
                            duration_ms: result.duration_ms,
                        },
                    )
                })
                .collect(),
        ))
    }

    fn request_share_to_windowsforum(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · sharing is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before sharing a report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status = "Failed to prepare share. Please try again.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::WindowsForumPost {
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare share. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::ShareToWindowsForum,
        });
        self.export_error = None;
        self.status = "Preparing report for WindowsForum…".to_string();
        self.resume_export_wait(context);
    }

    /// Export the latest completed scan to a user-chosen file, mirroring the
    /// Store 2.5.8 flow: the native save dialog runs synchronously on this
    /// UI thread (owner-validated by `save_picker`), while rendering and
    /// file I/O stay on workers. A dialog cancellation is a silent no-op,
    /// exactly like the shipping `save()` dialog path.
    fn request_export_to_file(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · file export is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before exporting a report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let Ok(format) = ReportFormat::try_from(self.settings_snapshot.export_format.as_str())
        else {
            self.status = "The selected export format is not available".to_string();
            return;
        };
        let Ok(SavePickerOutcome::Selected(path)) = save_picker::show_export_save_picker(format)
        else {
            return;
        };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status = "Failed to prepare export. Please try again.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::SavedReport {
                format,
                include_raw: true,
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare export. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::SaveToFile { path },
        });
        self.export_error = None;
        self.status = format!("Preparing {} export…", export_format_label(format));
        self.resume_export_wait(context);
    }

    fn stop_issue_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if let Some(task) = self.issue_wait.take() {
            task.cancel();
        }
        if let Some(task) = self.issue_prepare_task.take() {
            task.cancel();
        }
        self.issue_pending = None;
        self.issue_enqueued_request_id = None;
        self.issue_receiver = None;
        self.issue_runtime = None;
        self.issue_error = Some(reason.clone());
        if self.page == Page::Issues {
            self.status = reason;
        }
    }

    /// Re-run issue detection against the exact last committed native result
    /// map. The committed epoch changes only in `commit_issue_evidence`; a
    /// refresh advances the request id but deliberately retains that epoch.
    fn request_issue_detection(&mut self, context: &ComponentContext<Self>) -> bool {
        // Deterministic screenshots must not start the worker, read the clock,
        // or enumerate the user's temp directory.
        if self.deterministic_visual {
            return false;
        }
        let Some(session_id) = self.issue_source_session_id.clone() else {
            return false;
        };
        let Some(results) = self.issue_source_results.as_ref().map(Arc::clone) else {
            return false;
        };
        if self.issue_runtime.is_none() {
            if self.issue_error.is_none() {
                self.issue_error = Some("Native issue detection is unavailable".to_string());
            }
            return false;
        }

        let Some(request_id) = advance_nonzero_generation(&mut self.issue_request_id) else {
            self.stop_issue_delivery("Native issue request identity was exhausted");
            return false;
        };
        let pending = PendingIssueDetection {
            request_id,
            committed_epoch: self.issue_committed_epoch,
            session_id,
        };
        let superseded_prepare = self.issue_prepare_task.take();
        self.issue_pending = Some(pending.clone());
        self.issue_enqueued_request_id = None;
        self.issue_error = None;
        if let Some(task) = superseded_prepare {
            // The new guard is installed first, so a late cancellation message
            // from the old task cannot clear this request.
            task.cancel();
        }
        self.issue_prepare_task = Some(spawn_issue_request_preparation(context, pending, results));
        true
    }

    fn issue_preparation_is_current(&self, pending: &PendingIssueDetection) -> bool {
        pending_issue_preparation_is_current(
            self.issue_pending.as_ref(),
            pending,
            self.issue_committed_epoch,
            self.issue_source_session_id.as_deref(),
        )
    }

    fn apply_prepared_issue_request(
        &mut self,
        prepared: PreparedIssueDetection,
        context: &ComponentContext<Self>,
    ) {
        if !self.issue_preparation_is_current(&prepared.pending) {
            return;
        }
        self.issue_prepare_task = None;
        let request_id = prepared.pending.request_id;
        let enqueue_result = self
            .issue_runtime
            .as_ref()
            .ok_or_else(|| "native issue worker is unavailable".to_string())
            .and_then(|runtime| {
                runtime
                    .enqueue(prepared.request)
                    .map_err(|error| error.to_string())
            });
        match enqueue_result {
            Ok(()) => {
                self.issue_enqueued_request_id = Some(request_id);
                self.issue_error = None;
                self.resume_issue_wait(context);
            }
            Err(error) => {
                self.stop_issue_delivery(format!("Native issue detection stopped · {error}"));
            }
        }
    }

    fn apply_issue_preparation_failure(
        &mut self,
        pending: PendingIssueDetection,
        reason: &'static str,
    ) {
        if !self.issue_preparation_is_current(&pending) {
            return;
        }
        self.issue_prepare_task = None;
        self.issue_pending = None;
        self.issue_enqueued_request_id = None;
        self.issue_error = Some(reason.to_string());
        if self.page == Page::Issues {
            self.status = reason.to_string();
        }
    }

    /// Commit one complete authoritative diagnostic snapshot and immediately
    /// queue issue detection, before any optional history-save branch.
    fn commit_issue_evidence(
        &mut self,
        session_id: String,
        results: HashMap<String, DiagnosticOutput>,
        context: &ComponentContext<Self>,
    ) {
        let Some(committed_epoch) = advance_nonzero_generation(&mut self.issue_committed_epoch)
        else {
            self.stop_issue_delivery("Native issue evidence identity was exhausted");
            return;
        };
        debug_assert_eq!(committed_epoch, self.issue_committed_epoch);
        self.issue_source_session_id = Some(session_id);
        self.issue_source_results = Some(Arc::new(results));
        // Keep the last successfully projected issues visible until the new
        // guarded worker response succeeds. Preparation/enqueue/delivery
        // failures therefore cannot blank previously useful evidence.
        self.issue_error = None;
        let _ = self.request_issue_detection(context);
    }

    fn apply_issue_completion(
        &mut self,
        completion: IssueDetectionCompleted,
        context: &ComponentContext<Self>,
    ) {
        self.issue_wait = None;
        let current_session_id = self.issue_source_session_id.clone();
        if take_current_issue_completion(
            &mut self.issue_pending,
            &completion,
            self.issue_committed_epoch,
            current_session_id.as_deref(),
        )
        .is_some()
        {
            self.issue_enqueued_request_id = None;
            self.issues = completion.issues;
            self.issue_projected_epoch = Some(self.issue_committed_epoch);
            self.issue_projected_session_id = current_session_id;
            self.issue_error = None;
            if self.page == Page::Issues && !self.diagnostics_busy() {
                self.status = project_issues(&self.issues).counts.summary_text();
            }
        }
        // A stale response can precede a newer queued response. Its guard must
        // remain pending and delivery must continue until that response lands.
        self.resume_issue_wait(context);
    }

    fn apply_system_completion(
        &mut self,
        completion: SystemCompleted,
        context: &ComponentContext<Self>,
    ) {
        self.system_wait = None;
        let Some(request_kind) = take_matching_system_request(
            &mut self.system_info_request_id,
            &mut self.architecture_request_id,
            completion.request_id,
        ) else {
            // A completion from a superseded startup query must never replace
            // newer shell identity. Keep waiting for the current request ids.
            self.resume_system_wait(context);
            return;
        };

        match completion.result {
            Ok(SystemPayload::SystemInfo(info))
                if request_kind == SystemRequestKind::SystemInfo =>
            {
                self.is_admin = info.is_admin;
                self.system_info = info;
            }
            Ok(SystemPayload::Architecture(architecture))
                if request_kind == SystemRequestKind::Architecture =>
            {
                self.architecture = Some(architecture);
            }
            Ok(_) => {
                let error = "Native system worker returned the wrong payload".to_string();
                self.system_error = Some(error.clone());
                self.status = error;
            }
            Err(error) => {
                let error = error.to_string();
                self.system_error = Some(error.clone());
                self.status = format!("Could not read native system identity · {error}");
            }
        }

        self.resume_system_wait(context);
    }

    fn stop_system_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.system_wait = None;
        self.system_info_request_id = None;
        self.architecture_request_id = None;
        self.system_error = Some(reason.clone());
        self.system_receiver = None;
        self.system_runtime = None;
        self.status = reason;
    }

    fn open_settings(&mut self) {
        if self.settings_open || self.settings_saving || self.about_open {
            return;
        }
        self.next_settings_dialog_epoch();
        self.settings_draft = self.settings_snapshot.clone();
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        self.settings_save_error = None;
        self.settings_open = true;
    }

    fn cancel_settings(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) || self.settings_saving {
            return;
        }
        self.settings_draft = self.settings_snapshot.clone();
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        self.settings_save_error = None;
        self.settings_open = false;
    }

    fn apply_settings_dialog_action(&mut self, epoch: u64, action: SettingsDialogAction) {
        if !self.settings_dialog_is_current(epoch) {
            return;
        }
        if !self.settings_loading && !self.settings_saving && action.changes_draft() {
            self.settings_save_error = None;
        }

        match action {
            SettingsDialogAction::Cancel => self.cancel_settings(epoch),
            SettingsDialogAction::Save => self.request_settings_save(epoch),
            _ if self.settings_loading || self.settings_saving => {}
            SettingsDialogAction::ThemeSelectionChanged(Some(index)) => {
                self.theme = match index {
                    0 => WindowTheme::System,
                    1 => WindowTheme::Light,
                    _ => WindowTheme::Dark,
                };
                self.settings_draft.theme = window_theme_setting(self.theme).to_string();
            }
            SettingsDialogAction::ThemeSelectionChanged(None)
            | SettingsDialogAction::ExportFormatSelectionChanged(None)
            | SettingsDialogAction::PreferredAiProviderSelectionChanged(None)
            | SettingsDialogAction::CloudFallbackSelectionChanged(None)
            | SettingsDialogAction::CodexModelSelectionChanged(None)
            | SettingsDialogAction::MaxConcurrentTasksChanged(None) => {}
            SettingsDialogAction::ExportFormatSelectionChanged(Some(index)) => {
                self.settings_draft.export_format = match index {
                    1 => "json",
                    2 => "html",
                    _ => "text",
                }
                .to_string();
            }
            SettingsDialogAction::AiEnabledChanged(value) => {
                self.settings_draft.ai_enabled = value;
            }
            SettingsDialogAction::PreferredAiProviderSelectionChanged(Some(index)) => {
                if let Some(provider) = AI_PROVIDER_IDS.get(index) {
                    self.settings_draft.preferred_ai_provider = (*provider).to_string();
                }
            }
            SettingsDialogAction::CloudFallbackSelectionChanged(Some(index)) => {
                self.settings_draft.cloud_fallback_policy = match index {
                    1 => CloudFallbackPolicy::Allow,
                    2 => CloudFallbackPolicy::Never,
                    _ => CloudFallbackPolicy::Ask,
                };
            }
            SettingsDialogAction::NetworkGroundingChanged(value) => {
                self.settings_draft.network_grounding_enabled = value;
            }
            SettingsDialogAction::CodexCliPathChanged(value) => {
                self.settings_draft.codex_cli_path =
                    if value.is_empty() { None } else { Some(value) };
            }
            SettingsDialogAction::CodexModelSelectionChanged(Some(index)) => match index {
                0 => self.settings_draft.codex_model = None,
                1..=3 => {
                    self.settings_draft.codex_model =
                        CODEX_MODEL_IDS.get(index - 1).map(ToString::to_string);
                }
                _ => {}
            },
            SettingsDialogAction::ScanOnStartupChanged(value) => {
                self.settings_draft.scan_on_startup = value;
            }
            SettingsDialogAction::CloseToTrayChanged(value) => {
                self.settings_draft.close_to_tray = value;
                window_support::set_close_to_tray(value);
            }
            SettingsDialogAction::MaxConcurrentTasksChanged(Some(value)) => {
                if value.is_finite() {
                    self.settings_draft.max_concurrent_tasks =
                        (value.round() as u32).clamp(1, SETTINGS_MAX_CONCURRENT_TASKS);
                }
            }
            SettingsDialogAction::AutoSaveChanged(value) => {
                self.settings_draft.auto_save = value;
            }
            SettingsDialogAction::NotificationsChanged(value) => {
                self.settings_draft.show_notifications = value;
            }
        }
    }

    fn request_settings_save(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) {
            return;
        }
        if self.deterministic_visual {
            self.settings_snapshot = self.settings_draft.clone();
            self.settings_open = false;
            self.settings_save_error = None;
            self.status = "Settings saved".to_string();
            return;
        }
        if self.settings_loading || self.settings_saving {
            return;
        }
        if self.settings_runtime.is_none() {
            self.settings_save_error =
                Some("Native settings persistence is unavailable".to_string());
            return;
        }

        let request_id = self.next_settings_request_id();
        let submitted = self.settings_draft.clone();
        self.settings_pending_save = Some(PendingSettingsSave {
            request_id,
            dialog_epoch: epoch,
            submitted: submitted.clone(),
        });
        self.settings_saving = true;
        self.settings_save_error = None;
        let command = SettingsCommand::Save {
            request_id,
            settings: Box::new(submitted),
        };
        if let Err(error) = self
            .settings_runtime
            .as_ref()
            .expect("settings runtime availability checked above")
            .send(command)
        {
            self.settings_pending_save = None;
            self.settings_saving = false;
            self.settings_save_error = Some(error.to_string());
            self.status = "Settings were not saved".to_string();
        }
    }

    fn apply_settings_event(&mut self, event: SettingsEvent, context: &ComponentContext<Self>) {
        self.settings_wait = None;
        let mut worker_stopped = false;
        match event {
            SettingsEvent::Loaded { request_id, result } => {
                if self.settings_load_request_id != Some(request_id) {
                    self.resume_settings_wait(context);
                    return;
                }
                self.settings_load_request_id = None;
                self.settings_loading = false;
                match result {
                    Ok(mut settings) => {
                        normalize_provider_preference_for_runtime(&mut settings);
                        let provider_preference = settings.preferred_ai_provider.clone();
                        self.theme = window_theme_from_setting(&settings.theme);
                        update_history_retention_policy(&self.history_retention_policy, &settings);
                        self.settings_snapshot = settings.clone();
                        self.settings_draft = settings;
                        self.settings_error = None;
                        self.ai_settings_ready = true;
                        self.sync_ai_provider_preference(&provider_preference, context);
                    }
                    Err(error) => {
                        self.ai_settings_ready = false;
                        self.ai_provider_status = None;
                        self.ai_status_loading = false;
                        self.ai_status_error = Some("AI settings could not be loaded".to_string());
                        self.settings_error = Some(error.to_string());
                        self.status = "Settings could not be loaded".to_string();
                    }
                }
            }
            SettingsEvent::Saved { request_id, result } => {
                let Some(pending) = take_matching_pending_settings_save(
                    &mut self.settings_pending_save,
                    request_id,
                ) else {
                    self.resume_settings_wait(context);
                    return;
                };
                self.settings_saving = false;
                match result {
                    Ok(()) => {
                        let provider_preference = pending.submitted.preferred_ai_provider.clone();
                        let closes_current_dialog =
                            self.settings_dialog_is_current(pending.dialog_epoch);
                        update_history_retention_policy(
                            &self.history_retention_policy,
                            &pending.submitted,
                        );
                        self.settings_snapshot = pending.submitted.clone();
                        if closes_current_dialog || !self.settings_open {
                            self.settings_draft = pending.submitted;
                            self.theme = window_theme_from_setting(&self.settings_draft.theme);
                        }
                        self.settings_error = None;
                        self.settings_save_error = None;
                        if closes_current_dialog {
                            self.settings_open = false;
                        }
                        self.status = "Settings saved".to_string();
                        self.sync_ai_provider_preference(&provider_preference, context);
                    }
                    Err(error) => {
                        self.settings_save_error = Some(error.to_string());
                        self.status = "Settings were not saved".to_string();
                    }
                }
            }
            SettingsEvent::ProviderKeyStored { result, .. } => {
                self.provider_key_busy = false;
                self.status = match result {
                    Ok(()) => "API key saved".to_string(),
                    Err(error) => format!("Credential change failed: {error}"),
                };
            }
            SettingsEvent::ProviderKeyCleared { result, .. } => {
                self.provider_key_busy = false;
                self.status = match result {
                    Ok(()) => "API key cleared".to_string(),
                    Err(error) => format!("Credential change failed: {error}"),
                };
            }
            SettingsEvent::Stopped => worker_stopped = true,
            SettingsEvent::Updated { .. } => {}
        }

        if worker_stopped {
            self.settings_loading = false;
            self.settings_saving = false;
            self.settings_load_request_id = None;
            self.settings_pending_save = None;
            self.settings_error = Some("Native settings worker stopped".to_string());
            self.settings_save_error = None;
            self.settings_receiver = None;
            self.settings_runtime = None;
            self.status = "Native settings persistence stopped".to_string();
        } else {
            self.resume_settings_wait(context);
        }
    }

    fn request_process_page(&mut self, context: &ComponentContext<Self>, debounce_filter: bool) {
        if self.deterministic_visual {
            return;
        }

        let Some(runtime) = self.native_monitor.as_ref().map(Arc::clone) else {
            self.process_loading = false;
            self.process_error = Some("Native process inventory is unavailable".to_string());
            return;
        };

        self.process_request_id = self.process_request_id.wrapping_add(1);
        if self.process_request_id == 0 {
            self.process_request_id = 1;
        }
        let request_id = self.process_request_id;
        let query = ProcessQuery {
            search: self.process_filter.clone(),
            sort_by: self.process_sort_key,
            sort_direction: self.process_sort_direction,
            offset: self.process_offset,
            limit: PROCESS_PAGE_SIZE,
        };

        // Replacing the task cancels a previous debounce. Completions from a
        // query already running on the monitor worker are still harmless: the
        // monotonically increasing request id rejects stale results below.
        self.process_request_task.take();
        self.process_loading = true;
        self.process_error = None;
        self.process_request_task = Some(context.spawn_background_with_rejection(
            move |cancellation| {
                if debounce_filter {
                    let started = Instant::now();
                    while started.elapsed() < PROCESS_FILTER_DEBOUNCE {
                        if cancellation.is_cancelled() {
                            return Message::ProcessQueryDiscarded { request_id };
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                if cancellation.is_cancelled() {
                    return Message::ProcessQueryDiscarded { request_id };
                }

                let result = runtime
                    .request_processes(query)
                    .map_err(|error| error.to_string())
                    .and_then(|receiver| {
                        receiver
                            .blocking_recv()
                            .map_err(|_| "native process worker closed the query".to_string())
                    });
                Message::ProcessQueryFinished { request_id, result }
            },
            Message::ProcessQueryRejected { request_id },
        ));
    }

    fn set_process_sort(&mut self, sort_key: ProcessSortKey, context: &ComponentContext<Self>) {
        if self.process_sort_key == sort_key {
            self.process_sort_direction = match self.process_sort_direction {
                ProcessSortDirection::Asc => ProcessSortDirection::Desc,
                ProcessSortDirection::Desc => ProcessSortDirection::Asc,
            };
        } else {
            self.process_sort_key = sort_key;
            self.process_sort_direction = match sort_key {
                ProcessSortKey::Name | ProcessSortKey::Pid | ProcessSortKey::Status => {
                    ProcessSortDirection::Asc
                }
                _ => ProcessSortDirection::Desc,
            };
        }
        self.process_offset = 0;
        self.selected_process_pid = None;
        self.request_process_page(context, false);
    }

    fn request_history_list(&mut self, context: &ComponentContext<Self>) {
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
        self.history_request_task.take();
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

    fn request_history_comparison(
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
        self.history_compare_task.take();
        self.history_comparison = None;
        self.history_error = None;

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
            self.history_error = Some("Native history is unavailable".to_string());
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

    fn diagnostics_busy(&self) -> bool {
        self.diagnostic_starting
            || self.diagnostic_running
            || self.diagnostic_cancelling
            || self.diagnostic_finalizing
    }

    fn update_diagnostic_counts(&mut self) {
        self.diagnostic_completed = self.diagnostic_results.len();
        self.diagnostic_errors = self
            .diagnostic_results
            .iter()
            .filter(|result| !result.success)
            .count();
        if let Some(started) = self.diagnostic_scan_start {
            self.diagnostic_duration_ms = started.elapsed().as_millis() as u64;
        }
    }

    fn restore_previous_diagnostics(&mut self) {
        if let Some(previous) = self.previous_diagnostic_snapshot.take() {
            self.diagnostic_results = previous.results;
            self.diagnostic_scan_kind = previous.scan_kind;
            self.diagnostic_duration_ms = previous.duration_ms;
            self.diagnostic_total = previous.total;
            self.diagnostic_completed = previous.completed;
            self.diagnostic_errors = previous.errors;
        } else {
            self.diagnostic_results.clear();
            self.diagnostic_scan_kind = None;
            self.diagnostic_duration_ms = 0;
            self.diagnostic_total = 0;
            self.diagnostic_completed = 0;
            self.diagnostic_errors = 0;
        }
    }

    fn reset_diagnostic_activity(&mut self) {
        if let Some(task) = self.diagnostic_finalization_task.take() {
            task.cancel();
        }
        if let Some(task) = self.diagnostic_history_save_task.take() {
            task.cancel();
        }
        self.diagnostic_starting = false;
        self.diagnostic_running = false;
        self.diagnostic_cancelling = false;
        self.diagnostic_finalizing = false;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_start_task = None;
        self.diagnostic_run_task = None;
        self.diagnostic_cancel_task = None;
        self.diagnostic_scan_policy = None;
        self.diagnostic_expected_task_ids.clear();
        self.diagnostic_current_task = None;
        self.diagnostic_scan_start = None;
    }

    fn finish_completed_diagnostic_scan(&mut self, history_error: Option<String>) {
        let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
        self.previous_diagnostic_snapshot = None;
        self.reset_diagnostic_activity();
        self.notify_scan_completion();
        if let Some(error) = history_error {
            self.history_error = Some(format!("Scan history was not saved: {error}"));
            self.status = format!(
                "{label} complete · {} collected · {} errors · history not saved",
                self.diagnostic_completed, self.diagnostic_errors
            );
        } else {
            self.status = format!(
                "{label} complete · {} collected · {} errors",
                self.diagnostic_completed, self.diagnostic_errors
            );
        }
    }

    fn begin_completed_diagnostic_finalization(
        &mut self,
        session_id: String,
        context: &ComponentContext<Self>,
    ) {
        self.previous_diagnostic_snapshot = None;
        self.diagnostic_running = false;
        let committed_after_stop = self.diagnostic_cancel_requested || self.diagnostic_cancelling;
        self.diagnostic_cancelling = false;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_run_task = None;
        self.diagnostic_current_task = None;

        let auto_save = scan_policy_requests_auto_save(self.diagnostic_scan_policy.as_ref());
        if !auto_save || committed_after_stop {
            self.finish_completed_diagnostic_scan(None);
            return;
        }

        self.diagnostic_finalizing = true;
        self.status = format!(
            "Finalizing {}…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        self.diagnostic_finalization_task =
            Some(spawn_diagnostic_finalization_delay(context, session_id));
    }

    fn begin_completed_scan_history_save(
        &mut self,
        session_id: String,
        context: &ComponentContext<Self>,
    ) {
        self.diagnostic_finalization_task = None;
        if !self.diagnostic_finalizing || self.diagnostic_session_id.as_deref() != Some(&session_id)
        {
            return;
        }

        let Some(history_tag) = self
            .diagnostic_scan_policy
            .as_ref()
            .map(|policy| policy.history_tag.clone())
        else {
            self.finish_completed_diagnostic_scan(Some(
                "the scan-start history policy was unavailable".to_string(),
            ));
            return;
        };
        self.update_diagnostic_counts();
        let scan = build_history_scan_record(
            session_id.clone(),
            &self.system_info,
            &self.diagnostic_results,
            self.diagnostic_duration_ms,
            history_tag,
        );
        let Some(runtime) = self.history_runtime.as_ref().map(Arc::clone) else {
            self.finish_completed_diagnostic_scan(Some(
                "native scan history is unavailable".to_string(),
            ));
            return;
        };
        let reply = match runtime.request_save(scan) {
            Ok(reply) => reply,
            Err(error) => {
                self.finish_completed_diagnostic_scan(Some(error.to_string()));
                return;
            }
        };

        self.status = format!(
            "Saving {} history…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        self.diagnostic_history_save_task =
            Some(spawn_history_save_wait(context, session_id, reply));
    }

    fn begin_diagnostic_scan(&mut self, scan_kind: ScanKind, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            // Screenshot fixtures must never launch WMI, commands, or mutate the
            // captured Store 2.5.8 state.
            self.status = "Visual fixture mode · live scanning disabled".to_string();
            return;
        }
        if self.diagnostics_busy() {
            let label = self
                .diagnostic_scan_kind
                .map_or("Diagnostic scan", scan_kind_label);
            self.status = format!("{label} is already running");
            return;
        }
        if system_identity_blocks_scan(self.deterministic_visual, self.system_info_request_id) {
            self.status = "Detecting administrator access…".to_string();
            return;
        }
        if self.settings_loading {
            self.status = "Loading scan settings…".to_string();
            return;
        }

        let Some(runtime) = self.diagnostic_runtime.clone() else {
            self.status = "Native diagnostics are unavailable".to_string();
            return;
        };
        let policy = DiagnosticScanPolicy::snapshot(&self.settings_snapshot, scan_kind);
        let task_ids = select_scan_tasks(
            &self.diagnostic_catalog,
            scan_kind,
            self.is_admin,
            self.settings_snapshot.quick_scan_tasks.as_deref(),
        );
        if task_ids.is_empty() {
            self.status = format!("{} has no available tasks", scan_kind_label(scan_kind));
            return;
        }

        let task_count = task_ids.len();
        self.diagnostic_expected_task_ids = task_ids.clone();
        self.previous_diagnostic_snapshot = Some(DiagnosticSnapshot {
            results: self.diagnostic_results.clone(),
            scan_kind: self.diagnostic_scan_kind,
            duration_ms: self.diagnostic_duration_ms,
            total: self.diagnostic_total,
            completed: self.diagnostic_completed,
            errors: self.diagnostic_errors,
        });
        self.diagnostic_scan_kind = Some(scan_kind);
        self.diagnostic_scan_policy = Some(policy);
        self.diagnostic_total = task_count;
        self.diagnostic_completed = 0;
        self.diagnostic_errors = 0;
        self.diagnostic_duration_ms = 0;
        self.diagnostic_current_task = None;
        self.diagnostic_starting = true;
        self.diagnostic_cancel_requested = false;
        self.diagnostic_scan_start = Some(Instant::now());
        self.status = format!("Starting {}…", scan_kind_label(scan_kind));

        self.diagnostic_start_task = Some(context.spawn_background_with_rejection(
            move |_| match build_diagnostic_executor() {
                Ok(executor) => match executor.block_on(runtime.start_session(task_ids, scan_kind))
                {
                    Ok(session_id) => Message::DiagnosticSessionStarted {
                        session_id,
                        scan_kind,
                        task_count,
                    },
                    Err(error) => Message::DiagnosticSessionStartFailed {
                        error: error.to_string(),
                    },
                },
                Err(error) => Message::DiagnosticSessionStartFailed { error },
            },
            Message::DiagnosticSessionStartFailed {
                error: "the Reactor background queue rejected the scan request".to_string(),
            },
        ));
    }

    fn launch_diagnostic_run(&mut self, session_id: String, context: &ComponentContext<Self>) {
        let Some(runtime) = self.diagnostic_runtime.clone() else {
            self.restore_previous_diagnostics();
            self.reset_diagnostic_activity();
            self.status = "Native diagnostics stopped before the scan began".to_string();
            return;
        };
        let Some(max_concurrent_tasks) = self
            .diagnostic_scan_policy
            .as_ref()
            .map(|policy| policy.max_concurrent_tasks)
        else {
            self.restore_previous_diagnostics();
            self.reset_diagnostic_activity();
            self.status = "Scan settings were unavailable after startup".to_string();
            return;
        };
        let run_session_id = session_id.clone();
        self.diagnostic_run_task = Some(context.spawn_background_with_rejection(
            move |_| {
                match build_diagnostic_executor() {
                    Ok(executor) => match executor
                        .block_on(runtime.run_session(run_session_id.clone(), max_concurrent_tasks))
                    {
                        Ok(result) if result.cancelled => Message::DiagnosticRunFinished {
                            session_id: result.session_id,
                            cancelled: true,
                            authoritative_results: Err("scan was cancelled".to_string()),
                        },
                        Ok(result) => {
                            let authoritative_results = executor
                                .block_on(runtime.session_results(&result.session_id))
                                .map_err(|error| error.to_string());
                            Message::DiagnosticRunFinished {
                                session_id: result.session_id,
                                cancelled: false,
                                authoritative_results,
                            }
                        }
                        Err(error) => Message::DiagnosticRunFinished {
                            session_id: run_session_id,
                            cancelled: false,
                            authoritative_results: Err(error.to_string()),
                        },
                    },
                    Err(error) => Message::DiagnosticRunFinished {
                        session_id: run_session_id,
                        cancelled: false,
                        authoritative_results: Err(error),
                    },
                }
            },
            Message::DiagnosticRunRejected,
        ));
    }

    fn request_diagnostic_cancel(&mut self, context: &ComponentContext<Self>) {
        if self.diagnostic_starting {
            self.diagnostic_cancel_requested = true;
            self.status = "Stopping scan as soon as startup completes…".to_string();
            return;
        }
        if self.diagnostic_finalizing {
            if let Some(task) = self.diagnostic_finalization_task.take() {
                task.cancel();
                self.finish_completed_diagnostic_scan(None);
            }
            return;
        }
        if !self.diagnostic_running || self.diagnostic_cancelling {
            return;
        }
        let (Some(runtime), Some(session_id)) = (
            self.diagnostic_runtime.clone(),
            self.diagnostic_session_id.clone(),
        ) else {
            self.status = "The active diagnostic session could not be identified".to_string();
            return;
        };

        self.diagnostic_cancelling = true;
        self.diagnostic_cancel_requested = true;
        self.status = format!(
            "Stopping {} after in-flight checks finish…",
            self.diagnostic_scan_kind.map_or("scan", scan_kind_label)
        );
        let cancel_session_id = session_id.clone();
        let rejection_session_id = session_id.clone();
        self.diagnostic_cancel_task = Some(context.spawn_background_with_rejection(
            move |_| match build_diagnostic_executor() {
                Ok(executor) => {
                    let error = executor
                        .block_on(runtime.cancel_session(&cancel_session_id))
                        .err()
                        .map(|error| error.to_string());
                    Message::DiagnosticCancelFinished {
                        session_id: cancel_session_id,
                        error,
                    }
                }
                Err(error) => Message::DiagnosticCancelFinished {
                    session_id: cancel_session_id,
                    error: Some(error),
                },
            },
            Message::DiagnosticCancelRejected {
                session_id: rejection_session_id,
            },
        ));
    }

    fn apply_diagnostic_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::TaskProgress(progress) => {
                if self.diagnostic_session_id.as_deref() != Some(&progress.session_id) {
                    return;
                }
                let task_name = progress.task_name.unwrap_or_else(|| {
                    self.diagnostic_catalog
                        .iter()
                        .find(|task| task.id == progress.task_id)
                        .map_or(progress.task_id, |task| task.name.clone())
                });
                if progress.status == TaskProgressStatus::Running {
                    self.diagnostic_current_task = Some(task_name.clone());
                }
                self.update_diagnostic_counts();
                if self.diagnostic_running {
                    let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                    self.status = if self.diagnostic_cancelling {
                        format!("Stopping {label} · {task_name}")
                    } else {
                        format!(
                            "{label} · {} of {} collected · {task_name}",
                            self.diagnostic_completed, self.diagnostic_total
                        )
                    };
                }
            }
            UiEvent::DiagnosticResult(result) => {
                if self.diagnostic_session_id.as_deref() != Some(&result.session_id) {
                    return;
                }
                if let Some(existing) = self
                    .diagnostic_results
                    .iter_mut()
                    .find(|existing| existing.task_id == result.task_id)
                {
                    *existing = result;
                } else {
                    self.diagnostic_results.push(result);
                }
                self.diagnostic_results.sort_by_key(|result| {
                    self.diagnostic_catalog
                        .iter()
                        .position(|task| task.id == result.task_id)
                        .unwrap_or(usize::MAX)
                });
                self.update_diagnostic_counts();
                let label = self.diagnostic_scan_kind.map_or("Scan", scan_kind_label);
                self.status = if self.diagnostic_running {
                    format!(
                        "{label} · {} of {} collected · {} errors",
                        self.diagnostic_completed, self.diagnostic_total, self.diagnostic_errors
                    )
                } else {
                    format!(
                        "{label} complete · {} collected · {} errors",
                        self.diagnostic_completed, self.diagnostic_errors
                    )
                };
            }
            _ => {}
        }
    }

    fn apply_backend_event(&mut self, event: UiEvent) {
        match event {
            diagnostic @ (UiEvent::TaskProgress(_) | UiEvent::DiagnosticResult(_)) => {
                self.apply_diagnostic_event(diagnostic);
            }
            UiEvent::SystemStats(stats) => {
                if !self.monitoring_paused && matches!(self.page, Page::Monitor | Page::Processes) {
                    self.status = format!(
                        "Live sample · CPU {:.0}% · memory {:.0}%",
                        stats.cpu_utilization, stats.memory_utilization
                    );
                }
                self.monitor_history.push_stats(&stats);
                self.latest_system_stats = Some(stats);
            }
            UiEvent::Chat(ChatEvent::Delta(delta)) => self
                .chat_answer
                .get_or_insert_with(String::new)
                .push_str(&delta.text),
            UiEvent::Chat(ChatEvent::Done(done)) => {
                self.status = format!("AI response complete · {}", done.provider);
            }
            UiEvent::Chat(ChatEvent::Error(error)) => {
                self.status = format!("AI error · {}", error.message);
            }
            UiEvent::Chat(_) => self.status = "AI activity received".to_string(),
            UiEvent::Report(_) => self.status = "AI report activity received".to_string(),
            UiEvent::ActionStatus(_) => {
                self.status = "Remediation status received".to_string();
            }
            UiEvent::QuickScan(_) => self.status = "Quick scan requested".to_string(),
        }
    }
}

impl Component for WfdiagSpike {
    type Message = Message;
    type Input = ();

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let visual_state = VisualState::from_env();
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
            || (fixture_mode && !matches!(initial_page, Page::Monitor | Page::Processes))
        {
            vec![DiagnosticTaskResult {
                session_id: "visual-fixture".to_string(),
                task_id: "computer_system".to_string(),
                success: true,
                output: "Visual fixture; real results arrive through UiEvent::DiagnosticResult"
                    .to_string(),
                error: None,
                duration_ms: 29,
            }]
        } else {
            Vec::new()
        };
        let mut status = if diagnostic_results.is_empty() {
            "Ready — no scan data".to_string()
        } else {
            "17 collected · 0 errors".to_string()
        };
        let deterministic_visual = fixture_mode || visual_state != VisualState::Live;
        let (native_monitor, backend_receiver, backend_wait) = if deterministic_visual {
            (None, None, None)
        } else {
            match NativeMonitorRuntime::start(false) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(receiver);
                    let wait = spawn_backend_wait(context, Arc::clone(&receiver));
                    (Some(Arc::new(runtime)), Some(receiver), Some(wait))
                }
                Err(error) => {
                    status = format!("Native monitoring unavailable · {error}");
                    (None, None, None)
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
                let wait = spawn_diagnostic_wait(context, Arc::clone(&receiver));
                (catalog, Some(runtime), Some(receiver), Some(wait))
            };
        let (export_runtime, export_receiver, export_error) = if deterministic_visual {
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
            fixture_258_issues()
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
            match SettingsRuntime::start(service) {
                Ok((runtime, receiver)) => {
                    let request_id = 1;
                    if let Err(error) = runtime.send(SettingsCommand::Load { request_id }) {
                        (None, None, None, 0, None, false, Some(error.to_string()))
                    } else {
                        let receiver = Arc::new(Mutex::new(receiver));
                        let wait = spawn_settings_wait(context, Arc::clone(&receiver));
                        (
                            Some(runtime),
                            Some(receiver),
                            Some(wait),
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
        let (chat_runtime, chat_receiver, chat_wait) = if deterministic_visual {
            (None, None, None)
        } else if let Some(settings) = settings_service.as_ref() {
            match NativeChatRuntime::start(
                settings.clone(),
                Arc::new(FoundryCliEndpointSource::new()),
                Arc::new(ReqwestOllamaSource),
            ) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    let wait = spawn_chat_wait(context, Arc::clone(&receiver));
                    (Some(runtime), Some(receiver), Some(wait))
                }
                Err(error) => {
                    status = format!("Native AI chat unavailable · {error}");
                    (None, None, None)
                }
            }
        } else {
            (None, None, None)
        };
        let (report_runtime, report_receiver, report_wait) = if deterministic_visual {
            (None, None, None)
        } else if let Some(settings) = settings_service.as_ref() {
            match NativeReportRuntime::start(
                settings.clone(),
                Arc::new(FoundryCliEndpointSource::new()),
                Arc::new(ReqwestOllamaSource),
            ) {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    let wait = spawn_report_wait(context, Arc::clone(&receiver));
                    (Some(runtime), Some(receiver), Some(wait))
                }
                Err(error) => {
                    status = format!("Native AI report unavailable · {error}");
                    (None, None, None)
                }
            }
        } else {
            (None, None, None)
        };
        let (action_runtime, action_receiver, action_wait) = if deterministic_visual {
            // Fixture mode never executes anything; the worker is not built.
            (None, None, None)
        } else {
            match NativeActionRuntime::start() {
                Ok((runtime, receiver)) => {
                    let receiver = Arc::new(Mutex::new(receiver));
                    let wait = spawn_action_wait(context, Arc::clone(&receiver));
                    (Some(runtime), Some(receiver), Some(wait))
                }
                Err(error) => {
                    status = format!("Native remediation unavailable · {error}");
                    (None, None, None)
                }
            }
        };
        let instance_wait = if deterministic_visual {
            None
        } else {
            Some(spawn_instance_watch(context))
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

        let mut component = Self {
            page: initial_page,
            theme: WindowTheme::Dark,
            window_size: WindowSize { width, height },
            requested_client_width: width,
            requested_client_height: height,
            pane_open: true,
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
            process_filter: String::new(),
            process_page: None,
            process_sort_key: ProcessSortKey::CpuPercent,
            process_sort_direction: ProcessSortDirection::Desc,
            process_offset: 0,
            process_request_id: 0,
            process_request_task: None,
            process_loading: false,
            process_error: None,
            selected_process_pid: None,
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
            history_loading: false,
            history_error,
            chat_input: String::new(),
            chat_answer: None,
            chat_runtime,
            chat_receiver,
            chat_wait,
            chat_request_id: 0,
            chat_pending: None,
            report_runtime,
            report_receiver,
            report_wait,
            report_request_id: 0,
            report_pending: None,
            report_text: None,
            report_provider: None,
            report_error: None,
            action_runtime,
            action_receiver,
            action_wait,
            action_request_id: 0,
            action_pending: None,
            repair_confirm: None,
            admin_relaunch_task: None,
            instance_wait,
            palette_open: false,
            palette_query: String::new(),
            shortcut_help_open: false,
            window_hook_installed: false,
            provider_key_drafts: Default::default(),
            provider_key_busy: false,
            history_clear_confirm: false,
            history_tag_draft: String::new(),
            history_ack_busy: false,
            history_wait: None,
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
        self.ensure_window_hook();
        match message {
            Message::Navigate(Some(tag)) => {
                if let Some(page) = Page::from_tag(&tag) {
                    let entering_processes =
                        page == Page::Processes && self.page != Page::Processes;
                    let entering_history = page == Page::History && self.page != Page::History;
                    let entering_ai = page == Page::Ai && self.page != Page::Ai;
                    self.page = page;
                    if entering_processes {
                        self.process_offset = 0;
                        self.selected_process_pid = None;
                        self.request_process_page(context, false);
                    }
                    if entering_history {
                        self.request_history_list(context);
                    }
                    if entering_ai {
                        self.request_ai_provider_status(context);
                    }
                } else if tag == "quick-scan" {
                    self.page = Page::Diagnostics;
                    self.begin_diagnostic_scan(ScanKind::Quick, context);
                } else {
                    match tag.as_str() {
                        "export" => self.request_export_to_file(context),
                        "share" => self.request_share_to_windowsforum(context),
                        _ => (),
                    }
                }
            }
            Message::WindowSize(size) => self.window_size = size,
            Message::TogglePane => self.pane_open = !self.pane_open,
            Message::OpenAbout => self.open_about(),
            Message::TogglePalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
            }
            Message::ClosePalette => self.palette_open = false,
            Message::PaletteQueryChanged(value) => self.palette_query = value,
            Message::PaletteCommand(tag) => {
                self.palette_open = false;
                self.handle_palette_command(tag, context);
            }
            Message::ShowShortcutHelp => self.shortcut_help_open = true,
            Message::CloseShortcutHelp => self.shortcut_help_open = false,
            Message::ProviderKeyDraftChanged(index, value) => {
                if let Some(draft) = self.provider_key_drafts.get_mut(index) {
                    *draft = value;
                }
            }
            Message::StoreProviderKey(index) => {
                self.submit_provider_key(index, true);
            }
            Message::ClearProviderKey(index) => {
                self.provider_key_drafts[index] = String::new();
                self.submit_provider_key(index, false);
            }
            Message::ToggleClearHistoryConfirm(open) => {
                self.history_clear_confirm = open;
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
                        self.history_summaries.clear();
                        self.selected_history_id = None;
                        self.history_comparison = None;
                        self.history_tag_draft.clear();
                        self.status = "Scan history cleared".to_string();
                    }
                    (HistoryAckKind::Tags, Ok(())) => {
                        self.status = "Tags saved".to_string();
                        self.request_history_list(context);
                    }
                    (_, Err(message)) => self.status = message,
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
            Message::OpenSettings => self.open_settings(),
            Message::SettingsDialog { epoch, action } => {
                self.apply_settings_dialog_action(epoch, action);
            }
            Message::SettingsRuntimeEvent(event) => {
                self.apply_settings_event(*event, context);
            }
            Message::SettingsWorkerStopped => {
                self.settings_wait = None;
                self.settings_loading = false;
                self.settings_saving = false;
                self.settings_load_request_id = None;
                self.settings_pending_save = None;
                self.settings_error = Some("Native settings worker stopped".to_string());
                self.settings_save_error = None;
                self.settings_receiver = None;
                self.settings_runtime = None;
                self.status = "Native settings persistence stopped".to_string();
            }
            Message::SettingsWaitCancelled => {
                self.settings_wait = None;
            }
            Message::SettingsWaitRejected => {
                self.settings_wait = None;
                self.settings_loading = false;
                self.settings_saving = false;
                self.settings_load_request_id = None;
                self.settings_pending_save = None;
                self.settings_error =
                    Some("The Reactor background queue rejected settings delivery".to_string());
                self.settings_save_error = None;
                self.settings_receiver = None;
                self.settings_runtime = None;
                self.status = "Native settings delivery could not start".to_string();
            }
            Message::SystemRuntimeCompleted(completion) => {
                self.apply_system_completion(*completion, context);
            }
            Message::SystemWorkerStopped => {
                self.stop_system_delivery("Native system information worker stopped");
            }
            Message::SystemWaitCancelled => {
                self.system_wait = None;
            }
            Message::SystemWaitRejected => {
                self.stop_system_delivery(
                    "The Reactor background queue rejected native system information delivery",
                );
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
                self.diagnostic_results.clear();
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
                    self.diagnostic_session_id = None;
                    self.reset_diagnostic_activity();
                    self.status = format!("{label} stopped · previous results restored");
                } else {
                    let authoritative_results = match authoritative_results {
                        Ok(results) => results,
                        Err(error) => {
                            let stopped =
                                self.diagnostic_cancel_requested || self.diagnostic_cancelling;
                            self.restore_previous_diagnostics();
                            self.diagnostic_session_id = None;
                            self.reset_diagnostic_activity();
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
                        self.diagnostic_session_id = None;
                        self.reset_diagnostic_activity();
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
                    self.finish_completed_diagnostic_scan(None);
                }
            }
            Message::DiagnosticFinalizationRejected { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_finalization_task = None;
                    self.finish_completed_diagnostic_scan(Some(
                        "the Reactor background queue rejected scan finalization".to_string(),
                    ));
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
                self.finish_completed_diagnostic_scan(result.err());
                if saved && self.page == Page::History {
                    self.request_history_list(context);
                }
            }
            Message::DiagnosticHistorySaveWaitCancelled { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_history_save_task = None;
                    self.finish_completed_diagnostic_scan(Some(
                        "the scan-history acknowledgement was cancelled".to_string(),
                    ));
                }
            }
            Message::DiagnosticHistorySaveRejected { session_id } => {
                if self.diagnostic_finalizing
                    && self.diagnostic_session_id.as_deref() == Some(&session_id)
                {
                    self.diagnostic_history_save_task = None;
                    self.finish_completed_diagnostic_scan(Some(
                        "the Reactor background queue rejected scan-history delivery".to_string(),
                    ));
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
                    self.diagnostic_session_id = None;
                    self.reset_diagnostic_activity();
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
                        self.diagnostic_session_id = None;
                        self.reset_diagnostic_activity();
                    }
                    if !self.diagnostic_finalizing {
                        self.status = "Native diagnostic event delivery stopped".to_string();
                    }
                } else if let Some(receiver) = self.diagnostic_receiver.as_ref() {
                    self.diagnostic_wait =
                        Some(spawn_diagnostic_wait(context, Arc::clone(receiver)));
                }
            }
            Message::DiagnosticWaitRejected => {
                self.diagnostic_wait = None;
                if let Some(receiver) = self.diagnostic_receiver.take() {
                    receiver.close();
                }
                self.diagnostic_runtime.take();
                if self.diagnostics_busy() && !self.diagnostic_finalizing {
                    self.restore_previous_diagnostics();
                    self.diagnostic_session_id = None;
                    self.reset_diagnostic_activity();
                }
                if !self.diagnostic_finalizing {
                    self.status = "Native diagnostic delivery could not continue".to_string();
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
                    }
                    Err(error) => {
                        self.ai_provider_status = None;
                        self.ai_status_error = Some(error.clone());
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
                }
            }
            Message::ChatWorkerEventReceived(event) => {
                let Some(pending) = self.chat_pending else {
                    return;
                };
                if pending != event.request_id() {
                    self.resume_chat_wait(context);
                    return;
                }
                match *event {
                    ChatWorkerEvent::Delta { text, .. } => {
                        self.chat_answer
                            .get_or_insert_with(String::new)
                            .push_str(&text);
                        self.resume_chat_wait(context);
                    }
                    ChatWorkerEvent::ToolActivity { summary, .. } => {
                        self.status = summary;
                        self.resume_chat_wait(context);
                    }
                    ChatWorkerEvent::Done { provider, .. } => {
                        self.chat_pending = None;
                        self.chat_wait = None;
                        self.status = format!("AI response complete · {provider}");
                    }
                    ChatWorkerEvent::Failed { message, .. } => {
                        self.chat_pending = None;
                        self.chat_wait = None;
                        self.chat_answer = None;
                        self.status = message;
                    }
                    ChatWorkerEvent::Cancelled { .. } => {
                        self.chat_pending = None;
                        self.chat_wait = None;
                        self.status = "AI response cancelled".to_string();
                    }
                }
            }
            Message::ChatWorkerStopped => {
                self.chat_wait = None;
                self.chat_pending = None;
                self.chat_receiver = None;
                self.chat_runtime = None;
                self.status = "Native AI chat worker stopped".to_string();
            }
            Message::ChatWaitCancelled => {
                self.chat_wait = None;
            }
            Message::ChatWaitRejected => {
                self.chat_wait = None;
                self.chat_pending = None;
                self.status = "The native chat queue rejected a worker hand-off".to_string();
            }
            Message::GenerateReport => {
                if self.deterministic_visual {
                    self.status = "Visual fixture mode · report generation is disabled".to_string();
                    return;
                }
                if self.report_pending.is_some() {
                    self.status = "A report is already being generated…".to_string();
                    return;
                }
                if !self.settings_snapshot.ai_enabled {
                    self.status =
                        "Enable AI insights in Settings before generating a report".to_string();
                    return;
                }
                let Some(provider) = self
                    .ai_provider_status
                    .as_ref()
                    .map(|status| status.active_provider)
                    .filter(|provider| *provider != AIProvider::None)
                else {
                    self.status = "Set up an available AI provider before generating".to_string();
                    return;
                };
                let Some(runtime) = self.report_runtime.as_ref() else {
                    self.status = self
                        .report_error
                        .clone()
                        .unwrap_or_else(|| "Native AI report generation is unavailable".to_string());
                    return;
                };
                let Some(session_id) = self
                    .diagnostic_results
                    .first()
                    .map(|result| result.session_id.clone())
                else {
                    self.status = "Run a scan before generating a report".to_string();
                    return;
                };
                let Some(results) = self.export_results_snapshot() else {
                    self.status = "Run a scan before generating a report".to_string();
                    return;
                };
                let Some(request_id) = advance_nonzero_generation(&mut self.report_request_id)
                else {
                    self.status = "Native report request identity was exhausted".to_string();
                    return;
                };
                self.report_text = None;
                self.report_provider = None;
                self.report_error = None;
                self.report_pending = Some(request_id);
                runtime.generate(
                    request_id,
                    ReportScan {
                        session_id,
                        results: (*results).clone(),
                    },
                    provider,
                    false,
                );
                self.status = "Preparing AI report…".to_string();
                self.resume_report_wait(context);
            }
            Message::CancelReport => {
                if let (Some(runtime), Some(pending)) =
                    (self.report_runtime.as_ref(), self.report_pending)
                    && runtime.cancel(pending)
                {
                    self.status = "Cancelling the AI report…".to_string();
                }
            }
            Message::ReportWorkerEventReceived(event) => {
                let Some(pending) = self.report_pending else {
                    return;
                };
                if pending != event.request_id() {
                    self.resume_report_wait(context);
                    return;
                }
                match *event {
                    ReportWorkerEvent::Delta { text, .. } => {
                        self.report_text
                            .get_or_insert_with(String::new)
                            .push_str(&text);
                        self.resume_report_wait(context);
                    }
                    ReportWorkerEvent::Done { provider, cached, .. } => {
                        self.report_pending = None;
                        self.report_wait = None;
                        self.report_provider = Some(provider.clone());
                        self.status = if cached {
                            format!("AI report ready · {provider} · cached")
                        } else {
                            format!("AI report ready · {provider}")
                        };
                    }
                    ReportWorkerEvent::Failed { message, .. } => {
                        self.report_pending = None;
                        self.report_wait = None;
                        self.report_error = Some(message.clone());
                        self.status = message;
                    }
                    ReportWorkerEvent::Cancelled { .. } => {
                        self.report_pending = None;
                        self.report_wait = None;
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
            Message::ReportWaitCancelled => {
                self.report_wait = None;
            }
            Message::ReportWaitRejected => {
                self.report_wait = None;
                self.report_pending = None;
                self.status = "The native report queue rejected a worker hand-off".to_string();
            }
            Message::RunRemediation(remediation_id) => {
                if self.deterministic_visual {
                    self.status =
                        "Visual fixture mode · remediation is disabled".to_string();
                    return;
                }
                let Some(spec) = remediation::find(&remediation_id) else {
                    self.status = format!("Unknown remediation '{remediation_id}'");
                    return;
                };
                if spec.tier == RemediationTier::Repair {
                    // Production execution of a Repair is reachable only after
                    // this explicit confirmation; the engine's own gate would
                    // still refuse an unauthorized run.
                    self.repair_confirm = Some(spec.summary());
                    return;
                }
                self.execute_remediation(remediation_id, false, context);
            }
            Message::RepairDialogClosed {
                remediation_id,
                result,
            } => {
                let confirmed = self.repair_confirm.take().is_some()
                    && result == ContentDialogResult::Primary;
                if confirmed {
                    self.execute_remediation(remediation_id, true, context);
                }
            }
            Message::ActionWorkerEventReceived(event) => {
                let Some(pending) = self.action_pending else {
                    return;
                };
                if pending != event.request_id() {
                    self.resume_action_wait(context);
                    return;
                }
                match *event {
                    ActionWorkerEvent::Done { result, .. } => {
                        self.action_pending = None;
                        self.action_wait = None;
                        let success = result.success;
                        self.status = result.message.clone();
                        if success {
                            // The fix may have changed what detection sees.
                            if self.page == Page::Issues && !self.deterministic_visual {
                                self.request_issue_detection(context);
                            }
                        }
                    }
                    ActionWorkerEvent::Failed { message, .. } => {
                        self.action_pending = None;
                        self.action_wait = None;
                        self.status = message;
                    }
                    ActionWorkerEvent::NeedsConfirmation { remediation_id, .. } => {
                        self.action_pending = None;
                        self.action_wait = None;
                        if let Some(spec) = remediation::find(&remediation_id) {
                            self.repair_confirm = Some(spec.summary());
                        }
                    }
                }
            }
            Message::ActionWorkerStopped => {
                self.action_wait = None;
                self.action_pending = None;
                self.action_receiver = None;
                self.action_runtime = None;
                self.status = "Native remediation worker stopped".to_string();
            }
            Message::ActionWaitCancelled => {
                self.action_wait = None;
            }
            Message::ActionWaitRejected => {
                self.action_wait = None;
                self.action_pending = None;
                self.status = "The native remediation queue rejected a worker hand-off".to_string();
            }
            Message::AskAiAboutIssue(issue_id) => {
                let Some(issue) = self.issues.iter().find(|issue| issue.id == issue_id) else {
                    return;
                };
                self.chat_input = format!(
                    "How do I fix \"{}\"? ({} — {})",
                    issue.title, issue.category, issue.description
                );
                self.ai_mode = AiMode::Assistant;
                self.status = "Ask added to the chat input · press Send".to_string();
            }
            Message::ProposeFixPlan => {
                // The model may only reference the vetted catalog's labels;
                // its output never reaches execution — the user still runs
                // each action through the tier-gated buttons above.
                let detected: Vec<String> = self
                    .issues
                    .iter()
                    .filter(|issue| issue.detected)
                    .map(|issue| format!("- {} ({})", issue.title, issue.category))
                    .collect();
                let catalog: Vec<String> = self
                    .issue_maintenance
                    .iter()
                    .map(|entry| format!("- {} ({:?})", entry.label, entry.tier))
                    .collect();
                self.chat_input = format!(
                    "Propose an ordered fix plan for these detected issues: {}. Only suggest these vetted actions: {}",
                    detected.join("; "),
                    catalog.join(" "),
                );
                self.ai_mode = AiMode::Assistant;
                self.status = "Fix-plan prompt added to the chat input · press Send".to_string();
            }
            Message::RestartAsAdmin => {
                if self.deterministic_visual {
                    self.status = "Visual fixture mode · elevation is disabled".to_string();
                    return;
                }
                if self.admin_relaunch_task.is_none() {
                    self.admin_relaunch_task = Some(spawn_relaunch_as_admin(context));
                }
            }
            Message::InstanceActivated => {
                // Another launch asked this instance to the foreground.
                instance_support::activate_main_window();
                self.instance_wait = Some(spawn_instance_watch(context));
            }
            Message::TrayCommand(command) => {
                self.instance_wait = Some(spawn_instance_watch(context));
                match command {
                    window_support::TRAY_COMMAND_SHOW => {
                        match instance_support::main_window_hwnd() {
                            Some(window) if window_support::is_visible(window) => {
                                window_support::hide(window);
                            }
                            Some(window) => window_support::restore(window),
                            None => instance_support::activate_main_window(),
                        }
                    }
                    window_support::TRAY_COMMAND_QUICK_SCAN => {
                        self.page = Page::Diagnostics;
                        self.begin_diagnostic_scan(ScanKind::Quick, context);
                    }
                    window_support::TRAY_COMMAND_EXIT => {
                        window_support::request_forced_close();
                        if !context.window().request_close() {
                            // Reactor declined the close (never seen in
                            // practice); at least drop the tray icon.
                            if let Some(window) = instance_support::main_window_hwnd() {
                                window_support::remove_tray_icon(window);
                            }
                        }
                    }
                    _ => (),
                }
            }
            Message::RestartAsAdminFinished(result) => {
                self.admin_relaunch_task = None;
                match result {
                    Ok(true) => self.status = "Relaunching with administrator rights…".to_string(),
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
                    (
                        PendingExportAction::SaveToFile { path },
                        Ok(ExportPayload::Report(content)),
                    ) => {
                        self.export_error = None;
                        self.status =
                            format!("Writing {} report…", export_format_label(path.format()));
                        spawn_export_file_write(context, path, content);
                    }
                    (_, Ok(_)) => {
                        self.export_error =
                            Some("Native export worker returned an unexpected payload".to_string());
                        self.status = "Failed to prepare share. Please try again.".to_string();
                    }
                    (_, Err(error)) => {
                        self.export_error = Some(error.to_string());
                        self.status = "Failed to prepare share. Please try again.".to_string();
                    }
                }
            }
            Message::ExportFileSaved(result) => match *result {
                Ok(path) => {
                    self.export_error = None;
                    self.status = format!("Results saved to {}", path.display());
                }
                Err(error) => {
                    self.export_error = Some(error);
                    self.status =
                        "Failed to save the file. Please try a different location.".to_string();
                }
            },
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
                let accepted = self.native_monitor.as_ref().is_some_and(|runtime| {
                    if pause {
                        runtime.pause()
                    } else {
                        runtime.resume()
                    }
                });
                if accepted {
                    self.monitoring_paused = pause;
                    self.status = if pause {
                        "Live monitoring paused".to_string()
                    } else {
                        "Live monitoring resumed".to_string()
                    };
                } else {
                    self.status = "Native monitoring control is unavailable".to_string();
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
                    let accepted = self
                        .native_monitor
                        .as_ref()
                        .is_some_and(|runtime| runtime.refresh());
                    if self.page == Page::Processes {
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
                self.selected_process_pid = None;
                self.request_process_page(context, true);
            }
            Message::ProcessSort(sort_key) => self.set_process_sort(sort_key, context),
            Message::ProcessPrevious => {
                self.process_offset = self.process_offset.saturating_sub(PROCESS_PAGE_SIZE);
                self.selected_process_pid = None;
                self.request_process_page(context, false);
            }
            Message::ProcessNext => {
                if let Some(page) = self.process_page.as_ref()
                    && page.offset.saturating_add(page.items.len()) < page.total
                {
                    self.process_offset = page.offset.saturating_add(page.limit);
                    self.selected_process_pid = None;
                    self.request_process_page(context, false);
                }
            }
            Message::ProcessQueryFinished { request_id, result } => {
                if request_id != self.process_request_id {
                    return;
                }
                self.process_request_task = None;
                self.process_loading = false;
                match result {
                    Ok(page) => {
                        self.process_offset = page.offset;
                        if self
                            .selected_process_pid
                            .is_some_and(|pid| !page.items.iter().any(|row| row.pid == pid))
                        {
                            self.selected_process_pid = None;
                        }
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
            Message::SelectProcess(pid) => self.selected_process_pid = pid,
            Message::RefreshHistory => self.request_history_list(context),
            Message::HistoryFilterChanged(value) => self.history_filter = value,
            Message::SelectHistory(scan_id) => {
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
                        if self.selected_history_id.as_ref().is_some_and(|selected| {
                            !summaries.iter().any(|scan| &scan.id == selected)
                        }) {
                            self.selected_history_id = None;
                            self.history_comparison = None;
                            self.history_compare_task = None;
                        }
                        self.status = format!("History loaded · {} scans", summaries.len());
                        self.history_summaries = summaries;
                        self.history_error = None;
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
                        self.history_error = None;
                    }
                    Err(error) => {
                        self.history_comparison = None;
                        self.history_error = Some(error.clone());
                        self.status = format!("Could not compare history · {error}");
                    }
                }
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
                } else {
                    if request_id != self.history_request_id {
                        return;
                    }
                    self.history_request_task = None;
                    self.history_loading = false;
                }
                self.history_error =
                    Some("The Reactor background queue rejected the history request".to_string());
                self.status = "History request could not start".to_string();
            }
            Message::ChatInputChanged(value) => self.chat_input = value,
            Message::UsePrompt(value) => self.chat_input = value,
            Message::SendChat => {
                if !self.chat_input.trim().is_empty() {
                    if self.deterministic_visual {
                        self.chat_answer = Some(format!(
                            "I checked the fixture scan for “{}”. The urgent finding is low space on C:. Free at least 30 GB, then review the four recent Kernel-Power events.",
                            self.chat_input.trim()
                        ));
                        self.chat_input.clear();
                        self.status = "AI response complete · OpenAI cloud".to_string();
                    } else if !self.settings_snapshot.ai_enabled {
                        self.chat_answer = None;
                        self.status = "Enable AI insights in Settings before sending".to_string();
                    } else if self.chat_pending.is_some() {
                        self.status = "A response is already streaming…".to_string();
                    } else if self
                        .ai_provider_status
                        .as_ref()
                        .is_some_and(|status| status.active_provider != AIProvider::None)
                        && self.chat_runtime.is_some()
                    {
                        let provider = self
                            .ai_provider_status
                            .as_ref()
                            .map(|status| status.active_provider)
                            .expect("provider status was just checked");
                        let Some(runtime) = self.chat_runtime.as_ref() else {
                            return;
                        };
                        let Some(request_id) =
                            advance_nonzero_generation(&mut self.chat_request_id)
                        else {
                            self.status = "Native chat request identity was exhausted".to_string();
                            return;
                        };
                        let prompt = self.chat_input.trim().to_string();
                        self.chat_answer = None;
                        self.chat_pending = Some(request_id);
                        runtime.send(request_id, prompt, provider);
                        self.chat_input.clear();
                        self.status = "Asking the AI assistant…".to_string();
                        self.resume_chat_wait(context);
                    } else {
                        self.chat_answer = None;
                        self.status = "Set up an available AI provider before sending".to_string();
                    }
                }
            }
            Message::BackendBatch { events, terminated } => {
                let process_tick = !terminated
                    && !self.monitoring_paused
                    && self.page == Page::Processes
                    && self.process_request_task.is_none()
                    && events
                        .iter()
                        .any(|event| matches!(event, UiEvent::SystemStats(_)));
                for event in events {
                    self.apply_backend_event(event);
                }
                if terminated {
                    self.backend_wait = None;
                    self.monitoring_paused = true;
                    self.process_request_task = None;
                    self.process_loading = false;
                    self.process_error = Some("Native monitoring worker stopped".to_string());
                    if let Some(receiver) = self.backend_receiver.take() {
                        receiver.close();
                    }
                    self.native_monitor.take();
                    self.status = "Native monitoring worker stopped".to_string();
                } else if let Some(receiver) = self.backend_receiver.as_ref() {
                    self.backend_wait = Some(spawn_backend_wait(context, Arc::clone(receiver)));
                    if process_tick {
                        self.request_process_page(context, false);
                    }
                }
            }
            Message::BackendWaitRejected => {
                self.backend_wait = None;
                self.monitoring_paused = true;
                self.process_request_task = None;
                self.process_loading = false;
                self.process_error = Some("Native monitoring delivery stopped".to_string());
                if let Some(receiver) = self.backend_receiver.take() {
                    receiver.close();
                }
                self.native_monitor.take();
                self.status = "Native monitoring delivery stopped".to_string();
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
        context.on_window_size(context.callback(Message::WindowSize));

        let palette = Palette::for_theme(self.theme);
        let narrow = self.window_size.width < 940.0;
        // Store 2.5.8 switches to the compact 64 px rail at this breakpoint.
        // Keep the user's expanded preference so the full pane returns when the
        // window grows again, but never let it consume the compact content area.
        let pane_expanded = self.pane_open && !narrow;
        let issue_projection_current = issue_projection_matches_evidence(
            self.issue_projected_epoch,
            self.issue_projected_session_id.as_deref(),
            self.issue_committed_epoch,
            self.issue_source_session_id.as_deref(),
        );
        let page = match self.page {
            Page::Diagnostics => diagnostics_page(
                palette,
                self.theme,
                narrow,
                &self.diagnostic_results,
                &self.diagnostic_catalog,
                self.diagnostics_busy(),
                self.diagnostic_cancelling,
                self.diagnostic_completed,
                self.diagnostic_total,
                self.diagnostic_current_task.as_deref(),
                self.diagnostic_duration_ms,
                context.message(Message::RequestQuickScan),
                context.message(Message::RequestFullScan),
                context.message(Message::CancelScan),
            ),
            Page::Monitor => monitor_page(
                palette,
                narrow,
                self.monitoring_paused,
                self.latest_system_stats.as_ref(),
                &self.monitor_history,
                context.message(Message::ToggleMonitoring),
                context.message(Message::Refresh),
            ),
            Page::Processes => processes_page(
                palette,
                narrow,
                self.window_size.width,
                pane_expanded,
                &self.process_filter,
                self.process_page.as_ref(),
                self.process_loading,
                self.process_error.as_deref(),
                self.process_sort_key,
                self.process_sort_direction,
                self.deterministic_visual,
                self.selected_process_pid,
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
                self.chat_answer.as_deref(),
                self.ai_provider_status.as_ref(),
                self.ai_status_loading,
                self.ai_status_error.as_deref(),
                context.message(Message::SetAiMode(AiMode::Assistant)),
                context.message(Message::SetAiMode(AiMode::ScanReport)),
                context.callback(Message::ChatInputChanged),
                context.callback(Message::UsePrompt),
                context.message(Message::SendChat),
                context.message(Message::OpenSettings),
                self.report_text.as_deref(),
                self.report_provider.as_deref(),
                self.report_pending.is_some(),
                self.report_error.as_deref(),
                !self.diagnostic_results.is_empty(),
                context.message(Message::GenerateReport),
                context.message(Message::CancelReport),
            ),
            Page::Issues => issues_page(
                palette,
                self.theme,
                &self.issues,
                &self.issue_maintenance,
                self.is_admin,
                self.issue_pending.is_some(),
                self.issue_error.as_deref(),
                self.issue_source_session_id.is_some(),
                issue_projection_current,
                context.message(Message::RequestQuickScan),
                context.callback(Message::RunRemediation),
                context.callback(Message::AskAiAboutIssue),
                context.message(Message::ProposeFixPlan),
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
                self.history_tag_draft.as_str(),
                self.history_comparison.as_ref(),
                self.history_compare_task.is_some(),
                self.history_loading,
                self.history_error.as_deref(),
                self.history_ack_busy,
                context.message(Message::RefreshHistory),
                context.callback(Message::HistoryFilterChanged),
                context.callback(Message::SelectHistory),
                context.callback(Message::HistoryTagDraftChanged),
                context.message(Message::SaveHistoryTags),
                context.message(Message::ToggleClearHistoryConfirm(true)),
                context.message(Message::ClearHistoryConfirmed),
                context.message(Message::ToggleClearHistoryConfirm(false)),
                self.history_clear_confirm,
            ),
        };

        let status_icon = if self.diagnostic_results.is_empty() {
            if self.theme == WindowTheme::Light {
                STATUS_INFO_LIGHT
            } else {
                STATUS_INFO_DARK
            }
        } else if self.diagnostic_results.iter().any(|result| !result.success) {
            if self.theme == WindowTheme::Light {
                STATUS_WARN_LIGHT
            } else {
                STATUS_WARN_DARK
            }
        } else if self.theme == WindowTheme::Light {
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

        let page_host: View = if self.page == Page::Diagnostics {
            Border::new()
                .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
                .content(page)
        } else {
            ScrollViewer::new()
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
                .content(
                    Border::new()
                        .padding(Thickness::new(26.0, 14.0, 26.0, 18.0))
                        .content(page),
                )
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

        let pane_toggle: View = if narrow {
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

        let palette_commands: &[(&str, &str)] = &[
            ("Diagnostics", "diagnostics"),
            ("Live Monitor", "monitor"),
            ("Processes", "processes"),
            ("AI Analysis", "ai"),
            ("Issues", "issues"),
            ("History", "history"),
            ("Quick Scan", "quick-scan"),
            ("Full Scan", "full-scan"),
            ("Export report", "export"),
            ("Share to WindowsForum", "share"),
            ("Settings", "settings"),
            ("About", "about"),
            ("Toggle theme", "toggle-theme"),
            ("Keyboard shortcuts", "shortcut-help"),
        ];
        let palette_query_lower = self.palette_query.to_ascii_lowercase();
        let palette_rows: Vec<KeyedView> = palette_commands
            .iter()
            .filter(|(label, _)| {
                self.palette_query.is_empty()
                    || label.to_ascii_lowercase().contains(&palette_query_lower)
            })
            .map(|(label, tag)| {
                let execute = context.message(Message::PaletteCommand((*tag).to_string()));
                KeyedView::new(
                    *tag,
                    Button::new()
                        .width(430.0)
                        .style(ButtonStyle::Subtle)
                        .horizontal_alignment(HorizontalAlignment::Left)
                        .on_click(execute)
                        .content(
                            TextBlock::new()
                                .text((*label).to_string())
                                .horizontal_alignment(HorizontalAlignment::Left),
                        ),
                )
            })
            .collect();
        let palette_dialog = if self.palette_open {
            let query_changed = context.callback(Message::PaletteQueryChanged);
            let close = context.message(Message::ClosePalette);
            ContentDialog::new()
                .title("Command palette")
                .is_open(true)
                .close_button_text("Close")
                .on_closed(move |_| {
                    let _ = close.call(());
                })
                .content(
                    StackPanel::new()
                        .width(460.0)
                        .spacing(9.0)
                        .children((
                            Border::new()
                                .background(palette.card_strong)
                                .border_brush(palette.border)
                                .border_thickness(1.0)
                                .corner_radius(6.0)
                                .padding(Thickness::new(10.0, 7.0, 10.0, 7.0))
                                .content(
                                    TextBox::new()
                                        .placeholder_text("Type a command…")
                                        .on_text_changed(query_changed),
                                ),
                            ScrollViewer::new()
                                .max_height(360.0)
                                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                                .content(
                                    StackPanel::new().spacing(2.0).keyed_children(palette_rows),
                                ),
                        )),
                )
        } else {
            View::empty()
        };
        let shortcut_rows: &[(&str, &str)] = &[
            ("Ctrl+K", "Open the command palette"),
            ("Ctrl+1 … Ctrl+6", "Switch between screens"),
            ("Ctrl+Shift+Q", "Run a Quick Scan"),
            ("Ctrl+Shift+F", "Run a Full Scan"),
            ("Ctrl+R", "Refresh"),
            ("Ctrl+/", "Show this shortcut list"),
            ("Esc", "Close dialogs and overlays"),
        ];
        let shortcut_dialog = if self.shortcut_help_open {
            let close = context.message(Message::CloseShortcutHelp);
            ContentDialog::new()
                .title("Keyboard Shortcuts")
                .is_open(true)
                .close_button_text("Close")
                .on_closed(move |_| {
                    let _ = close.call(());
                })
                .content(
                    Border::new()
                        .width(400.0)
                        .background(palette.card_strong)
                        .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                        .content(StackPanel::new().spacing(6.0).keyed_children(
                            shortcut_rows
                                .iter()
                                .map(|(keys, description)| {
                                    KeyedView::new(
                                        *keys,
                                        Grid::new()
                                            .columns([GridLength::Star(1.0), GridLength::Auto])
                                            .children((
                                                TextBlock::new()
                                                    .text((*description).to_string())
                                                    .font_size(13.0),
                                                TextBlock::new()
                                                    .text((*keys).to_string())
                                                    .font_size(12.0)
                                                    .foreground(palette.muted),
                                            )),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )),
                )
        } else {
            View::empty()
        };

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

        let body = Grid::new()
            .grid_row(1)
            .columns([
                GridLength::Pixel(if pane_expanded { 230.0 } else { 64.0 }),
                GridLength::Star(1.0),
            ])
            .children((navigation_rail, content_panel));

        let light_wallpaper = Border::new()
            .grid_row_span(2)
            .opacity(if self.theme == WindowTheme::Light {
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
            .opacity(if self.theme == WindowTheme::Light {
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
        let settings_editable = !self.settings_loading && !self.settings_saving;
        let settings_can_save =
            settings_editable && (self.deterministic_visual || self.settings_runtime.is_some());
        let settings: View = if self.settings_open {
            let epoch = self.settings_dialog_epoch;
            settings_dialog(
                palette,
                self.theme,
                self.visual_state == VisualState::SettingsBottom,
                &self.settings_draft,
                !self.deterministic_visual,
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
                    self.settings_snapshot.custom_api_key_set,
                ],
                self.provider_key_busy,
                context.callback(move |(index, value)| Message::ProviderKeyDraftChanged(index, value)),
                context.callback(Message::StoreProviderKey),
                context.callback(Message::ClearProviderKey),
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

        let repair_dialog = if let Some(summary) = self.repair_confirm.as_ref() {
            // Repair preview comes only from catalog constants; no argv is
            // ever constructed from model or user input.
            let steps = remediation::find(&summary.id)
                .map(|spec| spec.preview_steps())
                .unwrap_or_default();
            let mut preview = String::from(
                "This repair alters system state and may require elevation. It will run:\n",
            );
            for step in &steps {
                preview.push_str("\n· ");
                preview.push_str(step);
            }
            if summary.requires_restart {
                preview.push_str("\n\nA restart is required afterwards.");
            }
            let remediation_id = summary.id.clone();
            let on_closed = context.callback(move |result| Message::RepairDialogClosed {
                remediation_id: remediation_id.clone(),
                result,
            });
            ContentDialog::new()
                .title("Run this repair?")
                .is_open(true)
                .primary_button_text("Run repair")
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
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        Grid::new()
            .rows([GridLength::Pixel(42.0), GridLength::Star(1.0)])
            // Reactor's pinned accelerator enum can only express Control plus
            // these keys (no main-row digits, K, slash, or Shift), so the
            // shipping Ctrl+1..6/K///Shift+Q/Shift+F set is upstream-blocked;
            // the palette and shortcut list stay reachable from the titlebar,
            // and Ctrl+Numpad1..6 cover screen switching where expressible.
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
                title_palette,
                title_help,
                title_settings,
                body,
                update_notice,
                settings,
                about_scrim,
                about,
                repair_dialog,
                palette_dialog,
                shortcut_dialog,
            ))
    }
}

#[allow(clippy::too_many_arguments)]
fn about_dialog(
    palette: Palette,
    open: bool,
    close_reference: &ElementRef<Button>,
    update: Option<&UpdateInfo>,
    action_error: Option<&str>,
    actions_enabled: bool,
    on_closed: Callback<ContentDialogResult>,
    on_download: Callback<()>,
    on_windowsforum: Callback<()>,
    on_github: Callback<()>,
    on_close: Callback<()>,
) -> View {
    let update_status: View = update.map_or_else(View::empty, |update| {
        Border::new()
            .margin(Thickness::new(0.0, 14.0, 0.0, 0.0))
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .spacing(6.0)
                    .children((
                        icons::path(FaIcon::CircleUp),
                        TextBlock::new()
                            .text(format!("Version {} is available.", update.version))
                            .font_size(13.0)
                            .foreground(palette.muted),
                    )),
            )
    });

    let action_error: View = action_error.map_or_else(View::empty, |error| {
        TextBlock::new()
            .text(error)
            .font_size(12.0)
            .foreground(palette.err)
            .text_wrapping(TextWrapping::Wrap)
            .horizontal_alignment(HorizontalAlignment::Center)
            .margin(Thickness::new(0.0, 10.0, 0.0, 0.0))
            .into()
    });

    let header_close = on_close.clone();
    let mut action_buttons = Vec::new();
    if let Some(update) = update {
        action_buttons.push(KeyedView::new(
            "download-update",
            Button::new()
                .resource_overrides(primary_button_resources())
                .is_enabled(actions_enabled)
                .on_click(on_download)
                .content(format!("Download v{}", update.version)),
        ));
    }
    let close_button = Button::new()
        .width(60.667)
        .height(31.333)
        .automation_name("Close");
    let close_button = if update.is_some() {
        close_button.style(ButtonStyle::Default)
    } else {
        close_button.resource_overrides(primary_button_resources())
    }
    .on_click(on_close)
    .content("Close");
    action_buttons.extend([
        KeyedView::new(
            "windowsforum",
            Button::new()
                .width(141.333)
                .height(31.333)
                .automation_name("WindowsForum")
                .is_enabled(actions_enabled)
                .on_click(on_windowsforum)
                .content(about_icon_label(FaIcon::Globe, "WindowsForum")),
        ),
        KeyedView::new(
            "github",
            Button::new()
                .width(92.667)
                .height(31.333)
                .automation_name("GitHub")
                .is_enabled(actions_enabled)
                .on_click(on_github)
                .content(about_icon_label(FaIcon::Github, "GitHub")),
        ),
        KeyedView::new("close", close_button),
    ]);

    // Reactor's pinned ContentDialog surface accepts only a string title. Keep
    // that real native title for modal/UIA semantics, then cover its visual row
    // with the Store 2.5.8 header chrome from inside the dialog content. The
    // negative margins extend only into ContentDialog's own measured padding;
    // focus confinement, Escape handling, and native Hide/Closed behavior stay
    // owned by WinUI.
    let header_overlay = Border::new()
        .height(59.0)
        .margin(Thickness::new(-22.0, -64.0, -22.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .background(palette.card_strong)
        .border_brush(Color::rgb(53, 54, 56))
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .padding(Thickness::new(17.0, 0.0, 22.0, 0.0))
        .content(
            Grid::new()
                .columns([
                    GridLength::Pixel(3.0),
                    GridLength::Star(1.0),
                    GridLength::Pixel(29.333),
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
                        .grid_column(1)
                        .text("About")
                        .font_size(13.0)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                    Button::new()
                        .grid_column(2)
                        .width(29.333)
                        .height(29.333)
                        .element_ref(close_reference)
                        .vertical_alignment(VerticalAlignment::Center)
                        .resource_overrides(
                            ResourceOverrides::new()
                                .set("ButtonBackground", Color::transparent())
                                .set("ButtonBackgroundPointerOver", palette.active)
                                .set("ButtonBackgroundPressed", palette.active)
                                .set("ButtonForeground", palette.muted)
                                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                .set("ButtonPadding", Thickness::uniform(7.0))
                                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
                        )
                        .horizontal_content_alignment(HorizontalAlignment::Center)
                        .vertical_content_alignment(VerticalAlignment::Center)
                        .automation_name("Close")
                        .automation_id("about-close")
                        .on_click(header_close)
                        .content(
                            Viewbox::new()
                                .width(12.0)
                                .height(12.0)
                                .stretch(Stretch::Uniform)
                                .slot(
                                    ViewboxSlot::Child,
                                    FontIcon::new()
                                        // Segoe Fluent Icons: Cancel. The
                                        // Viewbox prevents the font baseline
                                        // from clipping this glyph to one half.
                                        .glyph("\u{E711}"),
                                ),
                        ),
                )),
        );

    ContentDialog::new()
        .title("About")
        .is_open(open)
        .on_closed(on_closed)
        .content(
            Grid::new()
                // Store 2.5.8's card is ~457 DIPs wide while its description
                // line box is ~385 DIPs. ContentDialog contributes ~45 DIPs of
                // native padding, so a 412-DIP content slot reproduces the card
                // width and the body remains centered at its measured width.
                .width(412.0)
                .background(palette.card_strong)
                .children((
                    StackPanel::new()
                        .width(385.0)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .spacing(0.0)
                        .children((
                            Border::new()
                                .width(56.0)
                                .height(56.0)
                                .margin(Thickness::new(0.0, 8.0, 0.0, 31.5))
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .content(
                                    Image::new()
                                        .source_data(EncodedImage::from_static(APP_BADGE))
                                        .width(36.0)
                                        .height(36.0),
                                ),
                            TextBlock::new()
                                .text("WindowsForum Diagnostics")
                                .font_size(20.0)
                                .font_weight(FontWeight::BOLD)
                                .horizontal_alignment(HorizontalAlignment::Center),
                            TextBlock::new()
                                .text(format!("Version {APP_VERSION}"))
                                .font_size(13.0)
                                .foreground(palette.muted)
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .margin(Thickness::new(0.0, 4.0, 0.0, 16.0)),
                            TextBlock::new()
                                .text(ABOUT_DESCRIPTION)
                                .automation_name(ABOUT_DESCRIPTION)
                                .font_size(13.0)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::Wrap)
                                .margin(Thickness::new(0.0, 4.0, 0.0, 0.0)),
                            update_status,
                            action_error,
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .spacing(7.0)
                                .margin(Thickness::new(0.0, 23.5, 0.0, 2.0))
                                .keyed_children(action_buttons),
                        )),
                    header_overlay,
                )),
        )
}

fn about_icon_label(icon: FaIcon, label: &'static str) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(6.0)
        .children((icons::path(icon), TextBlock::new().text(label)))
}

fn nav_brand(palette: Palette, expanded: bool) -> View {
    let content: View = if expanded {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(11.0)
            .children((
                Image::new()
                    .source_data(EncodedImage::from_static(APP_BADGE))
                    .width(32.0)
                    .height(32.0),
                StackPanel::new().spacing(1.0).children((
                    TextBlock::new()
                        .text("WindowsForum")
                        .font_size(13.5)
                        .font_weight(FontWeight::BOLD),
                    TextBlock::new()
                        .text(format!("Diagnostics · {APP_VERSION}"))
                        .font_size(11.0)
                        .foreground(palette.muted),
                )),
            ))
    } else {
        Image::new()
            .source_data(EncodedImage::from_static(APP_BADGE))
            .width(32.0)
            .height(32.0)
            .horizontal_alignment(HorizontalAlignment::Center)
            .into()
    };

    Border::new()
        .padding(if expanded {
            Thickness::new(10.0, 10.0, 10.0, 18.0)
        } else {
            Thickness::new(0.0, 10.0, 0.0, 18.0)
        })
        .content(content)
}

fn nav_section(label: &'static str, expanded: bool, palette: Palette) -> View {
    if expanded {
        Border::new()
            .margin(Thickness::new(11.0, 20.0, 11.0, 7.0))
            .content(
                TextBlock::new()
                    .text(label)
                    .font_size(10.5)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.muted),
            )
    } else {
        Border::new().height(12.0).into()
    }
}

#[allow(clippy::too_many_arguments)]
fn nav_button(
    palette: Palette,
    icon: FaIcon,
    label: &'static str,
    selected: bool,
    expanded: bool,
    badge: Option<&str>,
    action: Callback<()>,
    enabled: bool,
) -> View {
    let content: View = if expanded {
        let badge: View = if let Some(value) = badge {
            Border::new()
                .grid_column(2)
                .min_width(18.0)
                .height(18.0)
                .padding(Thickness::xy(5.0, 0.0))
                .background(palette.warn_bg)
                .corner_radius(999.0)
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    TextBlock::new()
                        .text(value)
                        .font_size(10.5)
                        .font_weight(FontWeight::BOLD)
                        .foreground(palette.warn)
                        .horizontal_alignment(HorizontalAlignment::Center),
                )
        } else {
            View::empty()
        };
        Grid::new()
            .columns([
                GridLength::Pixel(17.0),
                GridLength::Star(1.0),
                GridLength::Auto,
            ])
            .column_spacing(11.0)
            .children((
                icons::path(icon),
                TextBlock::new()
                    .text(label)
                    .grid_column(1)
                    .font_size(13.0)
                    .font_weight(if selected {
                        FontWeight::SEMI_BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .vertical_alignment(VerticalAlignment::Center),
                badge,
            ))
    } else {
        icons::path(icon).into()
    };

    let button = Button::new()
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .horizontal_content_alignment(if expanded {
            HorizontalAlignment::Left
        } else {
            HorizontalAlignment::Center
        })
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
                .set("ButtonBackgroundDisabled", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.card)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.text)
                .set("ButtonForegroundDisabled", palette.muted)
                .set("ButtonBorderBrushDisabled", Color::transparent())
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set(
                    "ButtonPadding",
                    if expanded {
                        Thickness::xy(11.0, 0.0)
                    } else {
                        Thickness::uniform(0.0)
                    },
                )
                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
        )
        .is_enabled(enabled)
        .on_click(action)
        .automation_name(label)
        .content(content);

    if selected {
        Grid::new().children((
            button,
            Border::new()
                .width(3.0)
                .height(17.0)
                .margin(Thickness::new(0.0, 0.0, 0.0, 0.0))
                .background(palette.accent)
                .corner_radius(999.0)
                .horizontal_alignment(HorizontalAlignment::Left)
                .vertical_alignment(VerticalAlignment::Center),
        ))
    } else {
        button
    }
}

fn machine_card(
    palette: Palette,
    expanded: bool,
    system_info: &SystemInfo,
    architecture: Option<&ArchitectureSnapshot>,
    system_error: Option<&str>,
) -> View {
    if !expanded {
        return View::empty();
    }

    Border::new()
        .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
        .padding(Thickness::new(12.0, 11.0, 12.0, 11.0))
        .background(palette.card)
        .corner_radius(8.0)
        .automation_name(machine_card_accessibility_name(
            system_info,
            architecture,
            system_error,
        ))
        .content(StackPanel::new().spacing(7.0).children((
            machine_icon_label(FaIcon::Desktop, system_info.computer_name.clone()),
            machine_icon_label(FaIcon::Windows, system_info.os_version.clone()),
            machine_icon_label(FaIcon::UserShield, privilege_label(system_info.is_admin)),
        )))
}

fn machine_icon_label(icon: FaIcon, label: impl Into<String>) -> View {
    Grid::new()
        .columns([GridLength::Pixel(17.0), GridLength::Star(1.0)])
        .column_spacing(8.0)
        .children((
            icons::path(icon),
            TextBlock::new()
                .text(label)
                .grid_column(1)
                .text_trimming(TextTrimming::CharacterEllipsis),
        ))
}

fn fa_icon_label(icon: FaIcon, label: impl Into<String>) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children((icons::path(icon), label.into()))
}

fn page_header(palette: Palette, page: Page, trailing: impl Into<View>) -> View {
    Grid::new()
        .columns([GridLength::Star(1.0), GridLength::Auto])
        .children((
            StackPanel::new().spacing(3.0).children((
                TextBlock::new()
                    .text(page.title())
                    .font_size(21.0)
                    .font_weight(FontWeight::BOLD)
                    .automation_heading_level(AutomationHeadingLevel::Level1),
                TextBlock::new()
                    .text(page.subtitle())
                    .font_size(12.5)
                    .foreground(palette.muted),
            )),
            Border::new().grid_column(1).content(trailing),
        ))
}

fn status_pill(label: impl Into<String>, foreground: Color, background: Color) -> View {
    Border::new()
        .height(24.0)
        .background(background)
        .corner_radius(999.0)
        .padding(Thickness::xy(12.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .content(
            TextBlock::new()
                .text(label)
                .foreground(foreground)
                .font_size(11.5)
                .font_weight(FontWeight::SEMI_BOLD)
                .vertical_alignment(VerticalAlignment::Center),
        )
}

fn icon_status_pill(
    label: impl Into<String>,
    icon: &'static [u8],
    foreground: Color,
    background: Color,
) -> View {
    Border::new()
        .height(24.0)
        .background(background)
        .corner_radius(999.0)
        .padding(Thickness::xy(12.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .children((
                    Image::new()
                        .source_data(EncodedImage::from_static(icon))
                        .width(11.0)
                        .height(11.0)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(label)
                        .foreground(foreground)
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

fn placed(view: impl Into<View>, column: i32, row: i32) -> View {
    Border::new()
        .grid_column(column)
        .grid_row(row)
        .content(view)
}

fn monitor_graph(palette: Palette, series: &[f64], max: f64) -> View {
    const WIDTH: f64 = 300.0;
    const HEIGHT: f64 = 72.0;
    const BASELINE: f64 = 68.0;
    const FILL_STRIPS: usize = 48;

    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        1.0
    };
    let graph_y = |value: f64| {
        let value = if value.is_finite() { value } else { 0.0 };
        BASELINE - (value / max).clamp(0.0, 1.0) * (HEIGHT - 12.0)
    };
    let points = if series.len() > 1 {
        series
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    index as f64 * (WIDTH / (series.len() - 1) as f64),
                    graph_y(*value),
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![(0.0, BASELINE), (WIDTH, BASELINE)]
    };
    let chart_fill = Color::argb(
        if palette.accent.r == 77 { 36 } else { 31 },
        palette.accent.r,
        palette.accent.g,
        palette.accent.b,
    );

    let mut graph_children = Vec::with_capacity(FILL_STRIPS + points.len());
    for strip in 0..FILL_STRIPS {
        let sample_progress = (strip as f64 + 0.5) / FILL_STRIPS as f64;
        let y = if series.len() > 1 {
            let sample_position = sample_progress * (series.len() - 1) as f64;
            let lower = sample_position.floor() as usize;
            let upper = (lower + 1).min(series.len() - 1);
            let fraction = sample_position - lower as f64;
            let value = series[lower] + (series[upper] - series[lower]) * fraction;
            graph_y(value)
        } else {
            BASELINE
        };
        graph_children.push(KeyedView::new(
            format!("area-{strip}"),
            Rectangle::new()
                .width(WIDTH / FILL_STRIPS as f64 + 0.05)
                .height(HEIGHT - y)
                .fill(chart_fill)
                .canvas_left(strip as f64 * WIDTH / FILL_STRIPS as f64)
                .canvas_top(y),
        ));
    }
    for (index, segment) in points.windows(2).enumerate() {
        graph_children.push(KeyedView::new(
            format!("line-{index}"),
            Line::new()
                .stroke(palette.accent)
                .stroke_thickness(1.6)
                .x1(segment[0].0)
                .y1(segment[0].1)
                .x2(segment[1].0)
                .y2(segment[1].1),
        ));
    }

    Viewbox::new()
        .height(62.0)
        .margin(Thickness::new(0.0, 12.0, 0.0, 0.0))
        .stretch(Stretch::Fill)
        .slot(
            ViewboxSlot::Child,
            Canvas::new()
                .width(WIDTH)
                .height(HEIGHT)
                .keyed_children(graph_children),
        )
}

fn monitor_axis_label(
    palette: Palette,
    label: &'static str,
    column: i32,
    alignment: HorizontalAlignment,
) -> View {
    TextBlock::new()
        .text(label)
        .grid_column(column)
        .font_size(9.5)
        .foreground(palette.muted)
        .opacity(0.7)
        .horizontal_alignment(alignment)
        .into()
}

fn monitor_axis(palette: Palette) -> View {
    Grid::new()
        .margin(Thickness::new(0.0, 5.0, 0.0, 0.0))
        .columns([
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ])
        .children((
            monitor_axis_label(palette, "-60s", 0, HorizontalAlignment::Left),
            monitor_axis_label(palette, "-45", 2, HorizontalAlignment::Center),
            monitor_axis_label(palette, "-30", 4, HorizontalAlignment::Center),
            monitor_axis_label(palette, "-15", 6, HorizontalAlignment::Center),
            monitor_axis_label(palette, "now", 8, HorizontalAlignment::Right),
        ))
}

fn metric_card(
    palette: Palette,
    name: &str,
    hint: &str,
    value: &str,
    unit: &'static str,
    series: &[f64],
    max: f64,
) -> View {
    Border::new()
        .height(156.0)
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .padding(Thickness::new(17.0, 15.0, 17.0, 10.0))
        .content(
            StackPanel::new().children((
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        StackPanel::new().spacing(4.0).children((
                            TextBlock::new()
                                .text(name)
                                .font_size(10.5)
                                .font_weight(FontWeight::SEMI_BOLD)
                                .foreground(palette.muted),
                            TextBlock::new()
                                .text(hint)
                                .font_size(11.5)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::NoWrap)
                                .text_trimming(TextTrimming::CharacterEllipsis),
                        )),
                        StackPanel::new()
                            .grid_column(1)
                            .orientation(Orientation::Horizontal)
                            .spacing(3.0)
                            .margin(Thickness::new(0.0, -6.0, 0.0, 6.0))
                            .vertical_alignment(VerticalAlignment::Top)
                            .children((
                                TextBlock::new()
                                    .text(value)
                                    .font_size(26.0)
                                    .font_weight(FontWeight::LIGHT),
                                TextBlock::new()
                                    .text(unit)
                                    .margin(Thickness::new(0.0, 13.0, 0.0, 0.0))
                                    .font_size(12.0)
                                    .font_weight(FontWeight::NORMAL)
                                    .foreground(palette.muted),
                            )),
                    )),
                monitor_graph(palette, series, max),
                monitor_axis(palette),
            )),
        )
}

fn format_diagnostic_duration(duration_ms: u64) -> String {
    if duration_ms == 0 {
        "—".to_string()
    } else if duration_ms >= 1_000 {
        format!("{:.1} s", duration_ms as f64 / 1_000.0)
    } else {
        format!("{duration_ms} ms")
    }
}

fn diagnostics_scanning_page(
    palette: Palette,
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
            Button::new()
                .height(33.0)
                .margin(Thickness::new(0.0, 24.0, 0.0, 0.0))
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

fn live_collected_statistic(palette: Palette, collected: usize, completed: usize) -> View {
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

fn diagnostic_output_preview(result: &DiagnosticTaskResult) -> String {
    let output = result
        .error
        .as_deref()
        .filter(|error| !error.trim().is_empty())
        .unwrap_or(&result.output);
    if output.is_empty() {
        return "No output was returned by this diagnostic.".to_string();
    }

    const MAX_PREVIEW_CHARS: usize = 48_000;
    let mut preview: String = output.chars().take(MAX_PREVIEW_CHARS + 1).collect();
    if preview.chars().count() > MAX_PREVIEW_CHARS {
        preview = preview.chars().take(MAX_PREVIEW_CHARS).collect();
        preview.push_str("\n\n… Output preview truncated; the complete result remains in memory.");
    }
    preview
}

fn diagnostics_live_results_page(
    palette: Palette,
    theme: WindowTheme,
    narrow: bool,
    results: &[DiagnosticTaskResult],
    catalog: &[DiagnosticTask],
    duration_ms: u64,
) -> View {
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

    let rows = results
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
    let task_rows = rows
        .iter()
        .enumerate()
        .map(|(index, (task_id, name, category, passed, duration_ms))| {
            let first_in_group = index == 0 || rows[index - 1].2 != *category;
            let group_header: View = if first_in_group {
                let group_count = rows
                    .iter()
                    .filter(|(_, _, row_category, _, _)| row_category == category)
                    .count();
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
            let duration = format_diagnostic_duration(*duration_ms);
            let task_row = Border::new()
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
                                .background(if *passed { palette.accent } else { palette.err })
                                .corner_radius(999.0)
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBlock::new()
                                .text(name.clone())
                                .grid_column(1)
                                .font_size(12.5)
                                .font_weight(if index == 0 {
                                    FontWeight::SEMI_BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_trimming(TextTrimming::CharacterEllipsis)
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBlock::new()
                                .text(duration)
                                .grid_column(2)
                                .font_size(10.5)
                                .foreground(palette.muted)
                                .vertical_alignment(VerticalAlignment::Center),
                        )),
                );
            KeyedView::new(
                task_id.clone(),
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
                                            results.len(),
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

    let selected = &results[0];
    let selected_task = catalog.iter().find(|task| task.id == selected.task_id);
    let selected_name =
        selected_task.map_or_else(|| selected.task_id.clone(), |task| task.name.clone());
    let selected_category = selected_task.map_or("Other", |task| task.category.as_str());
    let selected_duration = format_diagnostic_duration(selected.duration_ms);
    let selected_output = diagnostic_output_preview(selected);
    let desktop_icon = if theme == WindowTheme::Light {
        DESKTOP_LIGHT
    } else {
        DESKTOP_DARK
    };
    let selected_status = if selected.success {
        "● Collected"
    } else {
        "● Error"
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
                                    Border::new()
                                        .grid_column(2)
                                        .width(90.0)
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
                                )),
                        ),
                    ScrollViewer::new()
                        .grid_row(1)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                        .content(
                            Border::new()
                                .padding(Thickness::uniform(16.0))
                                .content(
                                    TextBlock::new()
                                        .text(selected_output)
                                        .font_size(12.0)
                                        .foreground(if selected.success {
                                            palette.text
                                        } else {
                                            palette.err
                                        })
                                        .is_text_selection_enabled(true)
                                        .text_wrapping(TextWrapping::Wrap),
                                ),
                        ),
                )),
        );

    let body: View = if narrow {
        StackPanel::new()
            .spacing(14.0)
            .children((task_card, detail_card))
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

#[allow(clippy::too_many_arguments)]
fn diagnostics_page(
    palette: Palette,
    theme: WindowTheme,
    narrow: bool,
    results: &[DiagnosticTaskResult],
    catalog: &[DiagnosticTask],
    scan_active: bool,
    scan_cancelling: bool,
    completed: usize,
    total: usize,
    current_task: Option<&str>,
    duration_ms: u64,
    quick_scan: Callback<()>,
    full_scan: Callback<()>,
    cancel_scan: Callback<()>,
) -> View {
    if scan_active {
        return diagnostics_scanning_page(
            palette,
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
        StackPanel::new()
            .spacing(14.0)
            .children((task_card, detail_card))
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

fn diagnostics_empty_page(
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

fn collected_statistic(palette: Palette) -> View {
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

fn small_segment_button(palette: Palette, label: &'static str, width: f64) -> View {
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

fn detail_kv_row(palette: Palette, row: i32, label: &'static str, value: &'static str) -> View {
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

fn statistic(label: &str, value: &str, color: Color) -> View {
    StackPanel::new().spacing(1.0).children((
        TextBlock::new()
            .text(label)
            .font_size(10.0)
            .font_weight(FontWeight::BOLD),
        TextBlock::new()
            .text(value)
            .font_size(22.0)
            .foreground(color),
    ))
}

fn monitor_action_button(
    palette: Palette,
    icon: FaIcon,
    label: &'static str,
    width: f64,
    action: Callback<()>,
) -> View {
    Button::new()
        .width(width)
        .height(32.0)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", palette.card)
                .set("ButtonBackgroundPointerOver", palette.card)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonBorderBrush", palette.border)
                .set("ButtonBorderBrushPointerOver", palette.border)
                .set("ButtonBorderThemeThickness", Thickness::uniform(1.0))
                .set("ButtonPadding", Thickness::xy(15.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(7.0)),
        )
        .on_click(action)
        .automation_name(label)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    icons::path(icon).width(12.0).height(12.0),
                    TextBlock::new()
                        .text(label)
                        .font_size(13.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                )),
        )
}

fn monitor_status_pill(palette: Palette, paused: bool) -> View {
    let foreground = if paused { palette.warn } else { palette.ok };
    Border::new()
        .height(22.0)
        .background(if paused {
            palette.warn_bg
        } else {
            palette.ok_bg
        })
        .corner_radius(999.0)
        .padding(Thickness::new(12.0, 0.0, 8.0, 0.0))
        .vertical_alignment(VerticalAlignment::Center)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(7.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    Ellipse::new()
                        .width(7.0)
                        .height(7.0)
                        .fill(foreground)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(if paused { "Paused" } else { "Live · sampling" })
                        .foreground(foreground)
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

fn monitor_page(
    palette: Palette,
    narrow: bool,
    paused: bool,
    stats: Option<&SystemStats>,
    history: &MonitorHistory,
    toggle: Callback<()>,
    refresh: Callback<()>,
) -> View {
    let actions = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(14.0)
        .margin(Thickness::new(0.0, 1.0, 0.0, -1.0))
        .children((
            monitor_action_button(
                palette,
                if paused { FaIcon::Play } else { FaIcon::Pause },
                if paused { "Resume" } else { "Pause" },
                88.0,
                toggle,
            ),
            monitor_action_button(palette, FaIcon::Refresh, "Refresh", 96.0, refresh),
        ));
    let (
        cpu_hint,
        cpu_value,
        memory_hint,
        memory_value,
        storage_hint,
        storage_value,
        network_hint,
        network_value,
        gpu_hint,
        gpu_value,
        npu_hint,
        npu_value,
        hardware_summary,
        show_gpu,
        show_npu,
    ) = if let Some(stats) = stats {
        let network_mb = (stats.network_upload_kb + stats.network_download_kb) / 1024.0;
        let gpu_percent = f64::from(stats.gpu_utilization.unwrap_or_default());
        let npu_percent = f64::from(stats.npu_utilization.unwrap_or_default());
        let gpu_hint = if stats.gpu_available && stats.gpu_memory_total_mb > 0.0 {
            format!(
                "{:.2} GB / {:.2} GB",
                stats.gpu_memory_used_mb / 1024.0,
                stats.gpu_memory_total_mb / 1024.0
            )
        } else {
            stats
                .gpu_name
                .clone()
                .unwrap_or_else(|| "Graphics adapter".to_string())
        };
        let npu_hint = if stats.npu_available && stats.npu_memory_total_mb > 0.0 {
            format!(
                "{:.2} GB / {:.2} GB",
                stats.npu_memory_used_mb / 1024.0,
                stats.npu_memory_total_mb / 1024.0
            )
        } else {
            stats
                .npu_name
                .clone()
                .unwrap_or_else(|| "Neural processing unit".to_string())
        };
        let mut hardware_summary = format!(
            "{} threads · {:.1} GB RAM",
            stats.per_cpu_utilization.len(),
            stats.memory_total_gb
        );
        if stats.gpu_available {
            hardware_summary.push_str(" · GPU: ");
            hardware_summary.push_str(stats.gpu_name.as_deref().unwrap_or("present"));
        }
        if stats.npu_available {
            hardware_summary.push_str(" · NPU: ");
            hardware_summary.push_str(stats.npu_name.as_deref().unwrap_or("present"));
        }
        (
            format!("{:.2} GHz", stats.cpu_frequency as f64 / 1000.0),
            format!("{:.1}", stats.cpu_utilization),
            format!(
                "{:.1} / {:.1} GB used",
                stats.memory_used_gb, stats.memory_total_gb
            ),
            format!("{:.1}", stats.memory_utilization),
            "Provisioned storage capacity used".to_string(),
            format!("{:.1}", stats.storage_used_percent),
            "Up + down throughput".to_string(),
            format!("{network_mb:.2}"),
            gpu_hint,
            if stats.gpu_available {
                format!("{gpu_percent:.1}")
            } else {
                "—".to_string()
            },
            npu_hint,
            if stats.npu_available {
                format!("{npu_percent:.1}")
            } else {
                "—".to_string()
            },
            hardware_summary,
            stats.gpu_available,
            stats.npu_available,
        )
    } else {
        (
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Provisioned storage capacity used".to_string(),
            "—".to_string(),
            "Up + down throughput".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for sample".to_string(),
            "—".to_string(),
            "Waiting for the first native telemetry sample…".to_string(),
            true,
            true,
        )
    };

    let cpu_series = history.series(MonitorMetric::Cpu);
    let memory_series = history.series(MonitorMetric::Memory);
    let storage_series = history.series(MonitorMetric::Storage);
    let network_series = history.series(MonitorMetric::Network);
    let gpu_series = history.series(MonitorMetric::Gpu);
    let npu_series = history.series(MonitorMetric::Npu);
    let network_max = network_series
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(2.0_f64, f64::max)
        * 1.2;

    let mut cards = vec![
        (
            "cpu",
            metric_card(
                palette,
                "CPU",
                &cpu_hint,
                &cpu_value,
                "%",
                &cpu_series,
                100.0,
            ),
        ),
        (
            "memory",
            metric_card(
                palette,
                "MEMORY",
                &memory_hint,
                &memory_value,
                "%",
                &memory_series,
                100.0,
            ),
        ),
        (
            "storage",
            metric_card(
                palette,
                "STORAGE",
                &storage_hint,
                &storage_value,
                "%",
                &storage_series,
                100.0,
            ),
        ),
        (
            "network",
            metric_card(
                palette,
                "NETWORK",
                &network_hint,
                &network_value,
                "MB/s",
                &network_series,
                network_max,
            ),
        ),
    ];
    if show_gpu {
        cards.push((
            "gpu",
            metric_card(
                palette,
                "GPU",
                &gpu_hint,
                &gpu_value,
                "%",
                &gpu_series,
                100.0,
            ),
        ));
    }
    if show_npu {
        cards.push((
            "npu",
            metric_card(
                palette,
                "NPU",
                &npu_hint,
                &npu_value,
                "%",
                &npu_series,
                100.0,
            ),
        ));
    }
    let metrics: View = if narrow {
        StackPanel::new().spacing(14.0).keyed_children(
            cards
                .into_iter()
                .map(|(key, card)| KeyedView::new(key, card)),
        )
    } else {
        Grid::new()
            .columns([
                GridLength::Star(1.0),
                GridLength::Star(1.0),
                GridLength::Star(1.0),
            ])
            .rows([GridLength::Auto, GridLength::Auto])
            .column_spacing(14.0)
            .row_spacing(14.0)
            .keyed_children(cards.into_iter().enumerate().map(|(index, (key, card))| {
                KeyedView::new(
                    key,
                    Border::new()
                        .grid_column((index % 3) as i32)
                        .grid_row((index / 3) as i32)
                        .content(card),
                )
            }))
    };
    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Monitor, View::empty()),
        Border::new()
            .height(32.0)
            .margin(Thickness::new(0.0, 6.0, 0.0, 0.0))
            .content(
                Grid::new()
                    .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
                    .column_spacing(14.0)
                    .children((
                        monitor_status_pill(palette, paused),
                        TextBlock::new()
                            .text(hardware_summary)
                            .grid_column(1)
                            .vertical_alignment(VerticalAlignment::Center)
                            .font_size(12.0)
                            .foreground(palette.muted)
                            .text_wrapping(TextWrapping::NoWrap)
                            .text_trimming(TextTrimming::CharacterEllipsis),
                        Border::new().grid_column(2).content(actions),
                    )),
            ),
        Border::new()
            .margin(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(metrics),
    ))
}

fn table_header(label: &str, column: i32) -> TextBlock {
    TextBlock::new()
        .text(label)
        .grid_column(column)
        .margin(Thickness::new(8.0, 8.0, 8.0, 8.0))
        .font_size(10.0)
        .font_weight(FontWeight::BOLD)
}

fn palette_track() -> Color {
    Color::argb(26, 255, 255, 255)
}

fn table_cell(text: impl Into<String>, column: i32) -> TextBlock {
    TextBlock::new()
        .text(text)
        .grid_column(column)
        .margin(Thickness::xy(7.0, 0.0))
        .font_size(10.5)
        .vertical_alignment(VerticalAlignment::Center)
}

#[derive(Clone, Copy)]
struct ProcessFixture258 {
    name: &'static str,
    pid: u32,
    cpu: f64,
    memory: &'static str,
    memory_percent: f64,
    status: &'static str,
    threads: u32,
}

const PROCESS_ROWS_258: [ProcessFixture258; 19] = [
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 36840,
        cpu: 2.3,
        memory: "127.9 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 40,
    },
    ProcessFixture258 {
        name: "vmmemWSL",
        pid: 13884,
        cpu: 2.3,
        memory: "14.81 GB",
        memory_percent: 23.3,
        status: "Running",
        threads: 104,
    },
    ProcessFixture258 {
        name: "WorkloadsSessionHost.exe",
        pid: 21428,
        cpu: 2.2,
        memory: "48.3 MB",
        memory_percent: 0.1,
        status: "Running",
        threads: 8,
    },
    ProcessFixture258 {
        name: "System",
        pid: 4,
        cpu: 1.2,
        memory: "10.6 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 597,
    },
    ProcessFixture258 {
        name: "Taskmgr.exe",
        pid: 18736,
        cpu: 0.9,
        memory: "208.1 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 24,
    },
    ProcessFixture258 {
        name: "dwm.exe",
        pid: 2692,
        cpu: 0.9,
        memory: "194.4 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 18,
    },
    ProcessFixture258 {
        name: "msedgewebview2.exe",
        pid: 30728,
        cpu: 0.6,
        memory: "131.6 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 18,
    },
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 10892,
        cpu: 0.6,
        memory: "26.1 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 11,
    },
    ProcessFixture258 {
        name: "Paltalk.exe",
        pid: 24468,
        cpu: 0.6,
        memory: "345.9 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 89,
    },
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 33984,
        cpu: 0.5,
        memory: "310.8 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 53,
    },
    ProcessFixture258 {
        name: "WindowsTerminal.exe",
        pid: 25396,
        cpu: 0.4,
        memory: "175.4 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 21,
    },
    ProcessFixture258 {
        name: "WorkloadsSessionHost.exe",
        pid: 22104,
        cpu: 0.4,
        memory: "150.8 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 25,
    },
    ProcessFixture258 {
        name: "audiodg.exe",
        pid: 2832,
        cpu: 0.4,
        memory: "32.3 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 35,
    },
    ProcessFixture258 {
        name: "msedgewebview2.exe",
        pid: 39200,
        cpu: 0.3,
        memory: "326.1 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 17,
    },
    ProcessFixture258 {
        name: "svchost.exe",
        pid: 9852,
        cpu: 0.3,
        memory: "104.6 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 9,
    },
    ProcessFixture258 {
        name: "MsMpEng.exe",
        pid: 5772,
        cpu: 0.3,
        memory: "513.6 MB",
        memory_percent: 0.8,
        status: "Running",
        threads: 62,
    },
    ProcessFixture258 {
        name: "Discord.exe",
        pid: 21100,
        cpu: 0.2,
        memory: "736.4 MB",
        memory_percent: 1.1,
        status: "Running",
        threads: 592,
    },
    ProcessFixture258 {
        name: "SystemSettings.exe",
        pid: 19340,
        cpu: 0.2,
        memory: "96.8 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 21,
    },
    ProcessFixture258 {
        name: "PhoneExperienceHost.exe",
        pid: 17364,
        cpu: 0.2,
        memory: "142.7 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 32,
    },
];

impl ProcessFixture258 {
    fn virtual_memory(self) -> &'static str {
        match self.pid {
            13884 => "18.42 GB",
            21100 => "2.31 GB",
            5772 => "1.27 GB",
            33984 | 36840 => "1.14 GB",
            _ => "684.0 MB",
        }
    }

    fn handles(self) -> u32 {
        self.threads.saturating_mul(14).saturating_add(173)
    }

    fn cpu_time_secs(self) -> u64 {
        (self.pid as u64 % 1_200).saturating_add(self.threads as u64 * 3)
    }

    fn read(self) -> &'static str {
        match self.pid {
            4 => "6.81 GB",
            13884 => "2.07 GB",
            5772 => "1.42 GB",
            _ => "148.3 MB",
        }
    }

    fn written(self) -> &'static str {
        match self.pid {
            4 => "1.72 GB",
            13884 => "614.6 MB",
            21100 => "284.2 MB",
            _ => "42.1 MB",
        }
    }
}

#[derive(Clone)]
struct ProcessViewRow {
    name: String,
    pid: u32,
    cpu: f64,
    memory: String,
    memory_percent: f64,
    virtual_memory: String,
    status: String,
    threads: u32,
    handles: u32,
    cpu_time_secs: u64,
    read: String,
    written: String,
}

impl ProcessViewRow {
    fn icon(&self) -> FaIcon {
        let name = self.name.to_ascii_lowercase();
        if name.contains("msmpeng") {
            FaIcon::ShieldHalved
        } else if name == "system" {
            FaIcon::Microchip
        } else if name.contains("dwm") {
            FaIcon::Desktop
        } else if name.contains("svchost") || name.contains("workload") {
            FaIcon::Gear
        } else if name.contains("terminal") {
            FaIcon::List
        } else {
            FaIcon::Windows
        }
    }
}

impl From<ProcessFixture258> for ProcessViewRow {
    fn from(process: ProcessFixture258) -> Self {
        Self {
            name: process.name.to_string(),
            pid: process.pid,
            cpu: process.cpu,
            memory: process.memory.to_string(),
            memory_percent: process.memory_percent,
            virtual_memory: process.virtual_memory().to_string(),
            status: process.status.to_string(),
            threads: process.threads,
            handles: process.handles(),
            cpu_time_secs: process.cpu_time_secs(),
            read: process.read().to_string(),
            written: process.written().to_string(),
        }
    }
}

impl From<&ProcessRow> for ProcessViewRow {
    fn from(process: &ProcessRow) -> Self {
        Self {
            name: process.name.clone(),
            pid: process.pid,
            cpu: f64::from(process.cpu_percent),
            memory: format_megabytes(process.memory_mb),
            memory_percent: f64::from(process.memory_percent),
            virtual_memory: format_megabytes(process.virtual_memory_mb),
            status: process.status.clone(),
            threads: process.thread_count,
            handles: process.handle_count,
            cpu_time_secs: process.cpu_time_secs,
            read: format_bytes(process.io_read_bytes),
            written: format_bytes(process.io_write_bytes),
        }
    }
}

fn format_megabytes(value: f64) -> String {
    if !value.is_finite() || value < 0.0 {
        return "—".to_string();
    }
    if value >= 1024.0 {
        format!("{:.2} GB", value / 1024.0)
    } else {
        format!("{value:.1} MB")
    }
}

fn format_bytes(value: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * KIB;
    const GIB: f64 = 1024.0 * MIB;
    let value = value as f64;
    if value >= GIB {
        format!("{:.2} GB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{} B", value as u64)
    }
}

#[allow(clippy::too_many_arguments)]
fn processes_page(
    palette: Palette,
    narrow: bool,
    window_width: f64,
    pane_expanded: bool,
    filter: &str,
    process_page: Option<&ProcessPage>,
    loading: bool,
    error: Option<&str>,
    sort_key: ProcessSortKey,
    sort_direction: ProcessSortDirection,
    deterministic_visual: bool,
    selected_pid: Option<u32>,
    paused: bool,
    filter_changed: Callback<String>,
    sort_processes: Callback<ProcessSortKey>,
    previous: Callback<()>,
    next: Callback<()>,
    select_process: Callback<Option<u32>>,
    toggle: Callback<()>,
    refresh: Callback<()>,
) -> View {
    // ItemsRepeater's default StackLayout measures each realized child at its
    // desired width rather than stretching it to the table. Derive the row
    // width from the shell's responsive geometry so every row shares the
    // header's column boundaries in expanded, collapsed, and compact modes.
    let pane_width = if pane_expanded { 230.0 } else { 64.0 };
    let details_width = if narrow { 0.0 } else { 300.0 + 12.0 };
    let row_width = (window_width - pane_width - 72.0 - details_width).max(1.0);
    let needle = filter.trim().to_ascii_lowercase();
    let (display_rows, total, offset, limit) = if deterministic_visual {
        let rows = PROCESS_ROWS_258
            .into_iter()
            .filter(|process| {
                needle.is_empty()
                    || process.name.to_ascii_lowercase().contains(&needle)
                    || process.pid.to_string().contains(&needle)
                    || process.status.to_ascii_lowercase().contains(&needle)
            })
            .map(ProcessViewRow::from)
            .collect::<Vec<_>>();
        let total = if needle.is_empty() { 450 } else { rows.len() };
        (rows, total, 0, PROCESS_PAGE_SIZE)
    } else if let Some(page) = process_page {
        (
            page.items.iter().map(ProcessViewRow::from).collect(),
            page.total,
            page.offset,
            page.limit,
        )
    } else {
        (Vec::new(), 0, 0, PROCESS_PAGE_SIZE)
    };
    let visible = display_rows.len();
    let selected = selected_pid.and_then(|pid| {
        display_rows
            .iter()
            .find(|process| process.pid == pid)
            .cloned()
    });
    let rows = display_rows
        .iter()
        .map(|process| {
            KeyedView::new(
                process.pid,
                process_row_258(
                    palette,
                    narrow,
                    row_width,
                    process,
                    selected_pid == Some(process.pid),
                    select_process.clone(),
                ),
            )
        })
        .collect::<Vec<_>>();

    let actions = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children((
            Button::new()
                .on_click(refresh)
                .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
            Button::new().on_click(toggle).content(fa_icon_label(
                if paused { FaIcon::Play } else { FaIcon::Pause },
                if paused { "Resume" } else { "Pause" },
            )),
        ));
    let start = if total == 0 { 0 } else { offset + 1 };
    let end = if deterministic_visual && needle.is_empty() {
        (offset + limit).min(total)
    } else {
        offset.saturating_add(visible).min(total)
    };
    let mut summary = format!("Showing {start}–{end} of {total} processes");
    if loading && process_page.is_some() && !deterministic_visual {
        summary.push_str(" · Refreshing…");
    }

    let toolbar: View = if narrow {
        StackPanel::new().spacing(8.0).children((
            TextBox::new()
                .height(32.0)
                .text(filter)
                .placeholder_text("Filter processes…")
                .on_text_changed(filter_changed),
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Auto])
                .children((
                    TextBlock::new()
                        .text(summary)
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .vertical_alignment(VerticalAlignment::Center),
                    Border::new().grid_column(1).content(actions),
                )),
        ))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(12.0)
                    .children((
                        TextBox::new()
                            .width(260.0)
                            .height(32.0)
                            .text(filter)
                            .placeholder_text("Filter processes…")
                            .on_text_changed(filter_changed),
                        TextBlock::new()
                            .text(summary)
                            .font_size(11.5)
                            .foreground(palette.muted)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
                Border::new().grid_column(1).content(actions),
            ))
    };

    let rows_view: View = if visible == 0 {
        let message = if let Some(error) = error {
            format!("Could not load processes: {error}")
        } else if loading && !deterministic_visual {
            "Loading processes…".to_string()
        } else if !filter.trim().is_empty() {
            format!("No processes match “{}”.", filter.trim())
        } else {
            "No running processes were returned.".to_string()
        };
        Border::new().height(180.0).content(
            TextBlock::new()
                .text(message)
                .font_size(12.0)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center),
        )
    } else {
        ItemsRepeater::new()
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .items(rows)
            .into()
    };

    let table = Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().children((
            process_header_258(palette, narrow, sort_key, sort_direction, sort_processes),
            rows_view,
            process_pagination_258(
                palette,
                start,
                end,
                total,
                offset > 0 && !loading,
                end < total && !loading,
                previous,
                next,
            ),
        )));
    let detail = selected
        .as_ref()
        .map(|process| process_details_258(palette, process, select_process))
        .unwrap_or_else(View::empty);
    let layout: View = if narrow {
        StackPanel::new().spacing(12.0).children((table, detail))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Pixel(300.0)])
            .column_spacing(12.0)
            .children((table, placed(detail, 1, 0)))
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Processes, View::empty()),
        Border::new()
            .margin(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .content(toolbar),
        Border::new()
            .margin(Thickness::new(0.0, -4.0, 0.0, 0.0))
            .content(layout),
    ))
}

fn process_columns_258(narrow: bool) -> [GridLength; 6] {
    if narrow {
        [
            GridLength::Star(1.0),
            GridLength::Pixel(58.0),
            GridLength::Pixel(104.0),
            GridLength::Pixel(130.0),
            GridLength::Pixel(78.0),
            GridLength::Pixel(64.0),
        ]
    } else {
        [
            GridLength::Star(1.0),
            GridLength::Pixel(70.0),
            GridLength::Pixel(140.0),
            GridLength::Pixel(150.0),
            GridLength::Pixel(84.0),
            GridLength::Pixel(110.0),
        ]
    }
}

fn process_header_258(
    palette: Palette,
    narrow: bool,
    sort_key: ProcessSortKey,
    sort_direction: ProcessSortDirection,
    sort_processes: Callback<ProcessSortKey>,
) -> View {
    Grid::new()
        .height(37.0)
        .columns(process_columns_258(narrow))
        .background(palette.card_strong)
        .children((
            process_header_cell_258(
                palette,
                "PROCESS",
                0,
                ProcessSortKey::Name,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                "PID",
                1,
                ProcessSortKey::Pid,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                "CPU",
                2,
                ProcessSortKey::CpuPercent,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                "MEMORY",
                3,
                ProcessSortKey::MemoryMb,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                "STATUS",
                4,
                ProcessSortKey::Status,
                sort_key,
                sort_direction,
                sort_processes.clone(),
            ),
            process_header_cell_258(
                palette,
                "THREADS",
                5,
                ProcessSortKey::ThreadCount,
                sort_key,
                sort_direction,
                sort_processes,
            ),
        ))
}

fn process_header_cell_258(
    palette: Palette,
    label: &'static str,
    column: i32,
    column_key: ProcessSortKey,
    active_key: ProcessSortKey,
    direction: ProcessSortDirection,
    sort_processes: Callback<ProcessSortKey>,
) -> View {
    let active = column_key == active_key;
    let arrow: View = if active {
        TextBlock::new()
            .text(match direction {
                ProcessSortDirection::Asc => "↑",
                ProcessSortDirection::Desc => "↓",
            })
            .font_size(10.5)
            .foreground(palette.accent)
            .into()
    } else {
        View::empty()
    };
    Button::new()
        .grid_column(column)
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.muted)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::xy(18.0, 0.0))
                .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
        )
        .automation_name(format!("Sort processes by {label}"))
        .on_click(move || {
            let _ = sort_processes.call(column_key);
        })
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(4.0)
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    TextBlock::new()
                        .text(label)
                        .font_size(10.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .foreground(palette.muted),
                    arrow,
                )),
        )
}

fn process_row_258(
    palette: Palette,
    narrow: bool,
    row_width: f64,
    process: &ProcessViewRow,
    selected: bool,
    select_process: Callback<Option<u32>>,
) -> View {
    let select = select_process.clone();
    let pid = process.pid;
    let row = Grid::new()
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .columns(process_columns_258(narrow))
        .children((
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(9.0)
                .margin(Thickness::xy(18.0, 0.0))
                .vertical_alignment(VerticalAlignment::Center)
                .children((
                    icons::path(process.icon()),
                    TextBlock::new()
                        .text(process.name.clone())
                        .font_size(12.0)
                        .text_trimming(TextTrimming::CharacterEllipsis),
                )),
            process_table_cell_258(palette, process.pid.to_string(), 1),
            process_cpu_cell_258(palette, process.cpu, 2),
            process_memory_cell_258(palette, process, 3),
            process_table_cell_258(palette, process.status.clone(), 4),
            process_table_cell_258(palette, process.threads.to_string(), 5),
        ));

    Border::new()
        .width(row_width)
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Button::new()
                .height(37.0)
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .horizontal_content_alignment(HorizontalAlignment::Stretch)
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
                        .set("ButtonPadding", Thickness::uniform(0.0))
                        .set("ControlCornerRadius", CornerRadius::uniform(0.0)),
                )
                .automation_name(format!("{} PID {}", process.name, process.pid))
                .on_click(move || {
                    let _ = select.call(Some(pid));
                })
                .content(row),
        )
}

fn process_table_cell_258(palette: Palette, text: impl Into<String>, column: i32) -> TextBlock {
    TextBlock::new()
        .text(text)
        .grid_column(column)
        .margin(Thickness::xy(18.0, 0.0))
        .font_size(11.5)
        .foreground(palette.muted)
        .vertical_alignment(VerticalAlignment::Center)
}

fn process_percent_stack_258(palette: Palette, percent: f64) -> View {
    StackPanel::new().spacing(3.0).children((
        TextBlock::new()
            .text(format!("{percent:.1}%"))
            .font_size(11.5)
            .foreground(palette.muted)
            .horizontal_alignment(HorizontalAlignment::Right),
        Border::new()
            .width(56.0)
            .height(4.0)
            .background(palette_track())
            .corner_radius(999.0)
            .horizontal_alignment(HorizontalAlignment::Left)
            .content(
                Border::new()
                    .width((percent.clamp(0.0, 100.0) * 0.56).max(1.0))
                    .height(4.0)
                    .background(if percent > 80.0 {
                        palette.err
                    } else if percent > 50.0 {
                        palette.warn
                    } else {
                        palette.accent
                    })
                    .corner_radius(999.0)
                    .horizontal_alignment(HorizontalAlignment::Left),
            ),
    ))
}

fn process_cpu_cell_258(palette: Palette, percent: f64, column: i32) -> View {
    Border::new()
        .grid_column(column)
        .margin(Thickness::xy(18.0, 3.0))
        .content(process_percent_stack_258(palette, percent))
}

fn process_memory_cell_258(palette: Palette, process: &ProcessViewRow, column: i32) -> View {
    Grid::new()
        .grid_column(column)
        .margin(Thickness::xy(18.0, 0.0))
        .columns([GridLength::Star(1.0), GridLength::Pixel(56.0)])
        .column_spacing(8.0)
        .children((
            TextBlock::new()
                .text(process.memory.clone())
                .font_size(11.5)
                .foreground(palette.muted)
                .horizontal_alignment(HorizontalAlignment::Right)
                .vertical_alignment(VerticalAlignment::Center),
            Border::new()
                .grid_column(1)
                .margin(Thickness::new(0.0, 3.0, 0.0, 3.0))
                .content(process_percent_stack_258(palette, process.memory_percent)),
        ))
}

#[allow(clippy::too_many_arguments)]
fn process_pagination_258(
    palette: Palette,
    start: usize,
    end: usize,
    total: usize,
    can_previous: bool,
    can_next: bool,
    previous: Callback<()>,
    next: Callback<()>,
) -> View {
    let range = format!("{start}–{end} of {total}");
    Border::new().height(45.0).content(
        Grid::new()
            .columns([
                GridLength::Star(1.0),
                GridLength::Auto,
                GridLength::Pixel(94.0),
                GridLength::Auto,
            ])
            .column_spacing(9.0)
            .children((
                Button::new()
                    .grid_column(1)
                    .height(30.0)
                    .is_enabled(can_previous)
                    .on_click(previous)
                    .vertical_alignment(VerticalAlignment::Center)
                    .content("Previous"),
                TextBlock::new()
                    .text(range)
                    .grid_column(2)
                    .font_size(11.5)
                    .foreground(palette.muted)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .vertical_alignment(VerticalAlignment::Center),
                Button::new()
                    .grid_column(3)
                    .height(30.0)
                    .margin(Thickness::new(0.0, 0.0, 12.0, 0.0))
                    .is_enabled(can_next)
                    .on_click(next)
                    .vertical_alignment(VerticalAlignment::Center)
                    .content("Next"),
            )),
    )
}

fn process_details_258(
    palette: Palette,
    process: &ProcessViewRow,
    select_process: Callback<Option<u32>>,
) -> View {
    let close = select_process.clone();
    Border::new()
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(StackPanel::new().children((
            Border::new()
                .padding(Thickness::new(15.0, 13.0, 10.0, 13.0))
                .border_brush(palette.border)
                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .children((
                            StackPanel::new().spacing(2.0).children((
                                TextBlock::new()
                                    .text(process.name.clone())
                                    .font_size(13.0)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .text_trimming(TextTrimming::CharacterEllipsis),
                                TextBlock::new()
                                    .text(format!("PID {}", process.pid))
                                    .font_size(11.0)
                                    .foreground(palette.muted),
                            )),
                            Button::new()
                                .grid_column(1)
                                .width(28.0)
                                .height(28.0)
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", palette.active)
                                        .set("ButtonBackgroundPressed", palette.active)
                                        .set("ButtonForeground", palette.muted)
                                        .set(
                                            "ButtonBorderThemeThickness",
                                            Thickness::uniform(0.0),
                                        )
                                        .set("ButtonPadding", Thickness::uniform(6.0))
                                        .set(
                                            "ControlCornerRadius",
                                            CornerRadius::uniform(5.0),
                                        ),
                                )
                                .automation_name("Close process details")
                                .on_click(move || {
                                    let _ = close.call(None);
                                })
                                .content(icons::path(FaIcon::Xmark)),
                        )),
                ),
            Border::new()
                .padding(Thickness::new(15.0, 7.0, 15.0, 7.0))
                .content(StackPanel::new().children((
                    process_detail_row_258(palette, "CPU", format!("{:.1}%", process.cpu)),
                    process_detail_row_258(
                        palette,
                        "Memory",
                        format!("{} ({:.1}%)", process.memory, process.memory_percent),
                    ),
                    process_detail_row_258(
                        palette,
                        "Virtual memory",
                        process.virtual_memory.clone(),
                    ),
                    process_detail_row_258(palette, "Threads", process.threads.to_string()),
                    process_detail_row_258(palette, "Handles", process.handles.to_string()),
                    process_detail_row_258(
                        palette,
                        "CPU time",
                        format!("{}s", process.cpu_time_secs),
                    ),
                    process_detail_row_258(palette, "Read", process.read.clone()),
                    process_detail_row_258(palette, "Written", process.written.clone()),
                ))),
            Border::new()
                .padding(Thickness::new(15.0, 10.0, 15.0, 14.0))
                .content(
                    TextBlock::new()
                        .text("Path, owner, architecture, and elevation are omitted when Windows does not expose them without an additional privileged query.")
                        .font_size(10.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap),
                ),
        )))
}

fn process_detail_row_258(
    palette: Palette,
    label: impl Into<String>,
    value: impl Into<String>,
) -> View {
    Border::new()
        .min_height(32.0)
        .border_brush(palette.border)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(
            Grid::new()
                .columns([GridLength::Star(1.0), GridLength::Star(1.25)])
                .column_spacing(10.0)
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
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .text_wrapping(TextWrapping::Wrap)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}

fn ai_provider_pill_content(
    deterministic_visual: bool,
    ai_enabled: bool,
    status: Option<&AIProviderStatus>,
    loading: bool,
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
    if loading {
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
    let active = status.map_or(AIProvider::None, |status| status.active_provider);
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
    let model = status.and_then(|status| {
        status
            .providers
            .iter()
            .find(|provider| provider.id == active)
            .and_then(|provider| provider.model.clone())
    });
    (
        provider.to_string(),
        execution.to_string(),
        model,
        active != AIProvider::None,
        cloud,
    )
}

#[allow(clippy::too_many_arguments)]
fn ai_page(
    palette: Palette,
    narrow: bool,
    window_height: f64,
    visual_state: VisualState,
    deterministic_visual: bool,
    ai_enabled: bool,
    mode: AiMode,
    input: &str,
    answer: Option<&str>,
    provider_status: Option<&AIProviderStatus>,
    provider_loading: bool,
    provider_error: Option<&str>,
    assistant_mode: Callback<()>,
    report_mode: Callback<()>,
    input_changed: Callback<String>,
    use_prompt: Callback<String>,
    send: Callback<()>,
    open_settings: Callback<()>,
    report_text: Option<&str>,
    report_provider: Option<&str>,
    report_generating: bool,
    report_error: Option<&str>,
    report_has_scan: bool,
    generate_report: Callback<()>,
    cancel_report: Callback<()>,
) -> View {
    let prompts = [
        "Summarize my latest scan",
        "What failed and why?",
        "Any security concerns?",
        "How do I free up disk space?",
    ];
    let prompt_buttons = prompts
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
        .collect::<Vec<_>>();

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
    let runtime_pill = Border::new()
        .min_width(179.0)
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
                    TextBlock::new()
                        .text(provider_label)
                        .font_size(12.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                    TextBlock::new()
                        .text(execution_label)
                        .font_size(12.0)
                        .foreground(palette.muted),
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
                        .on_click(open_settings)
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

    let workspace_height = (window_height - 243.0).max(if narrow { 550.0 } else { 650.0 });

    let workspace = if mode == AiMode::Assistant {
        ai_assistant_workspace(
            palette,
            narrow,
            workspace_height,
            visual_state,
            input,
            answer,
            prompt_buttons,
            input_changed,
            deterministic_visual,
            ai_enabled,
            provider_loading,
            provider_ready || deterministic_visual,
            configure_ai,
            send,
        )
    } else {
        ai_scan_report_workspace(
            palette,
            narrow,
            workspace_height,
            report_text,
            report_provider,
            report_generating,
            report_error,
            report_has_scan,
            generate_report,
            cancel_report,
        )
    };

    StackPanel::new().spacing(16.0).children((
        page_header(palette, Page::Ai, View::empty()),
        mode_bar,
        workspace,
    ))
}

fn ai_mode_button(
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
                        palette.card_strong
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

#[allow(clippy::too_many_arguments)]
fn ai_assistant_workspace(
    palette: Palette,
    narrow: bool,
    workspace_height: f64,
    visual_state: VisualState,
    input: &str,
    answer: Option<&str>,
    prompt_buttons: Vec<KeyedView>,
    input_changed: Callback<String>,
    deterministic_visual: bool,
    ai_enabled: bool,
    provider_loading: bool,
    provider_ready: bool,
    open_settings: Callback<()>,
    send: Callback<()>,
) -> View {
    let body: View = if visual_state == VisualState::IssueToChat {
        ai_issue_to_chat_body(palette)
    } else if matches!(
        visual_state,
        VisualState::AiConversationDesktop
            | VisualState::AiConversationTopCompact
            | VisualState::AiConversationBottomCompact
    ) {
        ai_conversation_body(palette, visual_state)
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
                                        .content(
                                            TextBlock::new()
                                                .text(answer)
                                                .font_size(12.5)
                                                .text_wrapping(TextWrapping::Wrap),
                                        ),
                                )),
                            )),
                    ),
            )
    } else if !deterministic_visual && provider_loading {
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
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(9.0)
            .children((
                icons::path(if ai_enabled {
                    FaIcon::CircleInfo
                } else {
                    FaIcon::Gear
                })
                .width(30.0)
                .height(30.0),
                TextBlock::new()
                    .text(if ai_enabled {
                        "Connect an AI provider"
                    } else {
                        "AI insights are turned off"
                    })
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD),
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
                    .max_width(520.0),
                Button::new()
                    .height(32.0)
                    .resource_overrides(primary_button_resources())
                    .on_click(open_settings)
                    .content(configure_label),
            ))
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

    let header_trailing: View = if visual_state.is_conversation() {
        StackPanel::new()
            .grid_column(1)
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
            ))
    } else {
        View::empty()
    };
    let prompts: View = if visual_state.is_conversation()
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

    let composer_placeholder = if !deterministic_visual && provider_loading {
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
                    GridLength::Pixel(58.0),
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
                                        .height(36.0)
                                        .text(input)
                                        .placeholder_text(composer_placeholder)
                                        .is_enabled(
                                            deterministic_visual
                                                || (ai_enabled
                                                    && provider_ready
                                                    && !provider_loading),
                                        )
                                        .on_text_changed(input_changed),
                                    Button::new()
                                        .grid_column(1)
                                        .width(83.0)
                                        .height(32.0)
                                        .resource_overrides(primary_button_resources())
                                        .is_enabled(provider_ready && !input.trim().is_empty())
                                        .on_click(send)
                                        .content(fa_icon_label(FaIcon::PaperPlane, "Send")),
                                )),
                        ),
                )),
        )
}

fn ai_user_message(_palette: Palette, text: &'static str) -> View {
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

fn ai_assistant_message(palette: Palette, content: impl Into<View>) -> View {
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

fn ai_error_message(palette: Palette) -> View {
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

fn ai_response_message(palette: Palette) -> View {
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

fn ai_conversation_body(palette: Palette, state: VisualState) -> View {
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

fn ai_issue_to_chat_body(palette: Palette) -> View {
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

fn bot_avatar(size: f64) -> View {
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

fn primary_button_resources() -> ResourceOverrides {
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

#[allow(clippy::too_many_arguments)] // mirror ai_page's explicit view-parameter style
fn ai_scan_report_workspace(
    palette: Palette,
    narrow: bool,
    workspace_height: f64,
    report_text: Option<&str>,
    report_provider: Option<&str>,
    report_generating: bool,
    report_error: Option<&str>,
    has_scan: bool,
    generate: Callback<()>,
    cancel: Callback<()>,
) -> View {
    let body: View = if !has_scan {
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
                    .content(icons::path(FaIcon::FileExport).width(23.0).height(23.0)),
                TextBlock::new()
                    .text("Run a scan to create a report")
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .text("A focused health report will summarize collected diagnostics, errors, risks, and next steps.")
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(470.0),
            ))
    } else if let Some(error) = report_error {
        StackPanel::new()
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .spacing(10.0)
            .children((
                icons::path(FaIcon::TriangleExclamation)
                    .width(30.0)
                    .height(30.0),
                TextBlock::new()
                    .text("The report could not be generated")
                    .font_size(16.0)
                    .font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new()
                    .text(error.to_string())
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(470.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                Button::new().on_click(generate).content("Try again"),
            ))
    } else if report_generating {
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
                    .content(icons::path(FaIcon::WandMagicSparkles).width(23.0).height(23.0)),
                TextBlock::new()
                    .text("Generating report…")
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .text("The AI assistant is reviewing the latest scan.")
                    .font_size(12.5)
                    .foreground(palette.muted),
                Button::new().on_click(cancel).content("Cancel"),
            ))
    } else if let Some(text) = report_text {
        ScrollViewer::new()
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .content(
                Border::new().padding(Thickness::new(24.0, 20.0, 24.0, 20.0)).content(
                    StackPanel::new()
                        .spacing(12.0)
                        .children((
                            Grid::new()
                                .columns([GridLength::Star(1.0), GridLength::Auto])
                                .children((
                                    TextBlock::new()
                                        .text("Scan health report")
                                        .font_size(17.0)
                                        .font_weight(FontWeight::BOLD),
                                    Button::new().on_click(generate).content("Regenerate"),
                                )),
                            TextBlock::new()
                                .text(report_provider
                                    .map(|provider| format!("Generated by {provider}"))
                                    .unwrap_or_else(|| "Generated by the local AI assistant".to_string()))
                                .font_size(11.5)
                                .foreground(palette.muted),
                            TextBlock::new()
                                .text(text.to_string())
                                .font_size(13.0)
                                .text_wrapping(TextWrapping::Wrap),
                        )),
                ),
            )
    } else {
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
                    .content(icons::path(FaIcon::FileExport).width(23.0).height(23.0)),
                TextBlock::new()
                    .text("Ready to create your report")
                    .font_size(18.0)
                    .font_weight(FontWeight::BOLD),
                TextBlock::new()
                    .text("A focused health report will summarize collected diagnostics, errors, risks, and next steps.")
                    .font_size(12.5)
                    .foreground(palette.muted)
                    .text_wrapping(TextWrapping::Wrap)
                    .max_width(470.0)
                    .horizontal_alignment(HorizontalAlignment::Center),
                Button::new().on_click(generate).content("Generate report"),
            ))
    };
    Border::new()
        .height(workspace_height.max(if narrow { 550.0 } else { 650.0 }))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .content(body)
}

#[allow(clippy::too_many_arguments)]
fn issues_page(
    palette: Palette,
    theme: WindowTheme,
    issues: &[Issue],
    maintenance: &[RemediationSummary],
    is_admin: bool,
    detection_pending: bool,
    detection_error: Option<&str>,
    has_committed_evidence: bool,
    projection_current: bool,
    quick_scan: Callback<()>,
    run_remediation: Callback<String>,
    ask_ai: Callback<String>,
    propose_fix_plan: Callback<()>,
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

    if !projection.detected.is_empty() {
        children.push(KeyedView::new(
            "ai-assistance",
            issue_ai_assistance(palette, propose_fix_plan),
        ));
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
                None::<(&str, fn())>,
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
                Some(("Ask AI", ask_ai_callback)),
                153.0,
                190.0,
            ),
        ));
    }

    if projection.counts.passed > 0 {
        children.push(KeyedView::new(
            "passed",
            compact_issue_row(
                palette,
                &format!("{} checks passed", projection.counts.passed),
                true,
            ),
        ));
    }
    if projection.counts.unknown > 0 {
        children.push(KeyedView::new(
            "unknown",
            compact_issue_row(
                palette,
                &format!("Couldn’t verify ({})", projection.counts.unknown),
                true,
            ),
        ));
    }
    children.push(KeyedView::new(
        "maintenance",
        compact_issue_row(palette, "Maintenance", false),
    ));

    StackPanel::new().spacing(12.0).keyed_children(children)
}

#[allow(clippy::too_many_arguments)]
fn issues_empty_page(
    palette: Palette,
    theme: WindowTheme,
    maintenance: &[RemediationSummary],
    has_committed_evidence: bool,
    detection_pending: bool,
    detection_error: Option<&str>,
    quick_scan: Callback<()>,
    run_remediation: Callback<String>,
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

    StackPanel::new().spacing(0.0).children((
        page_header(palette, Page::Issues, View::empty()),
        hero,
        maintenance_card(palette, maintenance, run_remediation),
    ))
}

fn issue_ai_assistance(palette: Palette, propose_fix_plan: Callback<()>) -> View {
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
                        .children((
                            issue_ghost_button(palette, FaIcon::RankingStar, "Prioritize"),
                            Button::new()
                                .height(32.0)
                                .on_click(propose_fix_plan)
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", palette.active)
                                        .set("ButtonBackgroundPressed", palette.active)
                                        .set("ButtonBackgroundDisabled", Color::transparent())
                                        .set("ButtonForeground", palette.text)
                                        .set("ButtonForegroundDisabled", palette.text)
                                        .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                        .set("ButtonPadding", Thickness::xy(10.0, 0.0)),
                                )
                                .content(fa_icon_label(FaIcon::ListCheck, "Propose fix plan")),
                        )),
                )),
        )
}

fn issue_detection_notice(palette: Palette, text: &str, is_error: bool) -> View {
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

fn maintenance_card(
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

fn maintenance_row(palette: Palette, remediation: &RemediationSummary, run: impl Fn() + 'static) -> View {
    let mut title = remediation.label.clone();
    if remediation.tier == RemediationTier::Repair {
        title.push_str(" repair");
    }
    if remediation.admin_required {
        title.push_str(" admin");
    }

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
                        .resource_overrides(
                            ResourceOverrides::new().set("ButtonForegroundDisabled", palette.text),
                        )
                        .vertical_alignment(VerticalAlignment::Center)
                        .content("Run"),
                )),
        )
}

fn issue_primary_button_resources() -> ResourceOverrides {
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

fn issue_ghost_button(palette: Palette, icon: FaIcon, label: &str) -> View {
    Button::new()
        .height(32.0)
        .is_enabled(false)
        .resource_overrides(
            ResourceOverrides::new()
                .set("ButtonBackground", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.active)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonBackgroundDisabled", Color::transparent())
                .set("ButtonForeground", palette.text)
                .set("ButtonForegroundDisabled", palette.text)
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set("ButtonPadding", Thickness::xy(10.0, 0.0)),
        )
        .content(fa_icon_label(icon, label))
}

#[allow(clippy::too_many_arguments)]
fn issue_card(
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
    secondary_action: Option<(&str, impl Fn() + 'static)>,
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

    let secondary_action: View = if let Some((label, on_click)) = secondary_action {
        Button::new()
            .width(action_width)
            .style(ButtonStyle::Subtle)
            .on_click(on_click)
            .resource_overrides(
                ResourceOverrides::new().set("ButtonForegroundDisabled", palette.text),
            )
            .content(fa_icon_label(FaIcon::CommentDots, label))
    } else {
        View::empty()
    };
    let primary_action: View = if let Some((label, icon, on_click)) = primary_action {
        Button::new()
            .width(action_width)
            .height(32.0)
            .on_click(on_click)
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

fn issue_chip(
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

fn compact_issue_row(palette: Palette, label: &str, chevron: bool) -> View {
    let content: View = if chevron {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .vertical_alignment(VerticalAlignment::Center)
            .children((
                icons::path(FaIcon::ChevronRight).width(9.0).height(9.0),
                TextBlock::new()
                    .text(label)
                    .font_size(12.0)
                    .font_weight(FontWeight::BOLD),
            ))
    } else {
        TextBlock::new()
            .text(label)
            .font_size(12.0)
            .font_weight(FontWeight::BOLD)
            .into()
    };

    Border::new()
        .height(40.0)
        .padding(Thickness::xy(14.0, 0.0))
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(8.0)
        .content(content)
}

fn issue_severity_visual(
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

fn remediation_icon(remediation: &RemediationSummary) -> FaIcon {
    if remediation.id.contains("temp") || remediation.id.contains("recycle") {
        return FaIcon::Broom;
    }
    match remediation.tier {
        RemediationTier::OpenTool => FaIcon::ArrowUpRightFromSquare,
        RemediationTier::AutoSafe | RemediationTier::Repair => FaIcon::WandMagicSparkles,
    }
}

#[allow(clippy::too_many_arguments)]
fn history_page(
    palette: Palette,
    narrow: bool,
    deterministic_visual: bool,
    fixture_empty: bool,
    summaries: &[ScanSummary],
    filter: &str,
    selected_id: Option<&str>,
    selected_tags: &str,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    loading: bool,
    error: Option<&str>,
    ack_busy: bool,
    refresh: Callback<()>,
    filter_changed: Callback<String>,
    select_history: Callback<String>,
    tag_changed: Callback<String>,
    save_tags: Callback<()>,
    clear_request: Callback<()>,
    clear_confirmed: Callback<()>,
    clear_cancelled: Callback<()>,
    clear_confirm_open: bool,
) -> View {
    if deterministic_visual {
        return history_fixture_page(palette, narrow, fixture_empty);
    }
    history_live_page(
        palette,
        narrow,
        summaries,
        filter,
        selected_id,
        selected_tags,
        comparison,
        comparison_loading,
        loading,
        error,
        ack_busy,
        refresh,
        filter_changed,
        select_history,
        tag_changed,
        save_tags,
        clear_request,
        clear_confirmed,
        clear_cancelled,
        clear_confirm_open,
    )
}

#[allow(clippy::too_many_arguments)]
fn history_live_page(
    palette: Palette,
    narrow: bool,
    summaries: &[ScanSummary],
    filter: &str,
    selected_id: Option<&str>,
    selected_tags: &str,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    loading: bool,
    error: Option<&str>,
    ack_busy: bool,
    refresh: Callback<()>,
    filter_changed: Callback<String>,
    select_history: Callback<String>,
    tag_changed: Callback<String>,
    save_tags: Callback<()>,
    clear_request: Callback<()>,
    clear_confirmed: Callback<()>,
    clear_cancelled: Callback<()>,
    clear_confirm_open: bool,
) -> View {
    let needle = filter.trim().to_ascii_lowercase();
    let filtered = summaries
        .iter()
        .filter(|scan| {
            needle.is_empty()
                || scan.id.to_ascii_lowercase().contains(&needle)
                || scan.computer_name.to_ascii_lowercase().contains(&needle)
                || scan
                    .label
                    .as_deref()
                    .unwrap_or("Quick Scan")
                    .to_ascii_lowercase()
                    .contains(&needle)
                || scan
                    .timestamp
                    .to_iso_string()
                    .to_ascii_lowercase()
                    .contains(&needle)
        })
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
                        scan.label.as_deref().unwrap_or("Quick Scan"),
                        scan.timestamp.to_iso_string()
                    ))
                    .on_click(move || {
                        let _ = select.call(scan_id.clone());
                    })
                    .content(history_row(
                        palette,
                        &history_timestamp(scan),
                        scan.label.as_deref().unwrap_or("Quick Scan"),
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
    let diff = history_live_comparison(
        palette,
        selected_id,
        latest_id,
        comparison,
        comparison_loading,
        error,
    );
    let body: View = if narrow {
        StackPanel::new().spacing(14.0).children((sessions, diff))
    } else {
        Grid::new()
            .columns([GridLength::Star(1.4), GridLength::Star(1.0)])
            .column_spacing(14.0)
            .children((sessions, placed(diff, 1, 0)))
    };
    let count_label = if loading && !summaries.is_empty() {
        format!("{} scans · refreshing…", summaries.len())
    } else {
        format!("{} scans", summaries.len())
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
                            .content(fa_icon_label(FaIcon::Trash, "Clear history")),
                    )),
            )),
        body,
        {
            let tags_editor: View = selected_id
                .and_then(|id| summaries.iter().find(|scan| scan.id == id))
                .map(|summary| {
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
                                .text(if selected_tags.is_empty() {
                                    summary.tags.join(", ")
                                } else {
                                    selected_tags.to_string()
                                })
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

fn history_timestamp(scan: &ScanSummary) -> String {
    scan.timestamp.format("%m/%d/%Y\n%H:%M:%S UTC")
}

fn history_list_message(palette: Palette, message: &str) -> View {
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

fn history_section_header(palette: Palette, title: &str, hint: Option<&str>) -> View {
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

fn history_live_comparison(
    palette: Palette,
    selected_id: Option<&str>,
    latest_id: Option<&str>,
    comparison: Option<&ComparisonSummary>,
    comparison_loading: bool,
    error: Option<&str>,
) -> View {
    let body: View = if let Some(comparison) = comparison {
        let changed = comparison
            .status_unchanged
            .iter()
            .filter(|change| change.output_changed)
            .collect::<Vec<_>>();
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
                    changed.len().to_string(),
                    palette.text,
                ),
            ),
        ];
        rows.extend(changed.into_iter().map(|change| {
            KeyedView::new(
                format!("changed-{}", change.task_id),
                history_change(
                    palette,
                    "changed",
                    change.task_name.clone(),
                    "",
                    palette.accent,
                    palette.active,
                ),
            )
        }));
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
            body,
        )))
}

fn history_comparison_placeholder(
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

fn history_fixture_page(palette: Palette, narrow: bool, empty: bool) -> View {
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
                        history_change(palette, "changed", "System Services", "", palette.accent, palette.active),
                        history_change(palette, "changed", "Logical Disks", "", palette.accent, palette.active),
                        history_change(palette, "changed", "Operating System", "", palette.accent, palette.active),
                        history_change(palette, "changed", "Processor", "", palette.accent, palette.active),
                        history_change(palette, "changed", "System Information", "", palette.accent, palette.active),
                        history_change(palette, "changed", "Startup Commands", "", palette.accent, palette.active),
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

fn history_empty_page(palette: Palette, narrow: bool) -> View {
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

fn history_columns() -> [GridLength; 6] {
    [
        GridLength::Pixel(22.0),
        GridLength::Pixel(132.0),
        GridLength::Star(1.0),
        GridLength::Pixel(72.0),
        GridLength::Pixel(58.0),
        GridLength::Pixel(60.0),
    ]
}

fn history_header(palette: Palette) -> View {
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
fn history_row(
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

fn history_metric(
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

fn history_change(
    palette: Palette,
    kind: impl Into<String>,
    label: impl Into<String>,
    detail: impl Into<String>,
    color: Color,
    background: Color,
) -> View {
    let kind = kind.into();
    let label = label.into();
    let detail = detail.into();
    let detail_is_empty = detail.is_empty();
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
            icons::path(FaIcon::ChevronRight)
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
            TextBlock::new()
                .text(detail)
                .grid_column(3)
                .font_size(11.0)
                .font_weight(FontWeight::BOLD)
                .foreground(if detail_is_empty {
                    palette.muted
                } else {
                    palette.warn
                })
                .vertical_alignment(VerticalAlignment::Center),
        ))
}

#[allow(clippy::too_many_arguments)]
fn settings_dialog(
    palette: Palette,
    theme: WindowTheme,
    bottom: bool,
    settings: &AppSettings,
    provider_setup_partial: bool,
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
    cancel: Callback<()>,
    save: Callback<()>,
    provider_key_drafts: &[String; 4],
    provider_keys_set: [bool; 4],
    key_busy: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
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
                                    provider_setup_partial,
                                    editable,
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


/// API keys section: DPAPI-backed credential entry per provider. Shared by
/// both settings layouts.
#[allow(clippy::too_many_arguments)]
fn settings_provider_keys_section(
    palette: Palette,
    provider_key_drafts: &[String; 4],
    provider_keys_set: [bool; 4],
    key_busy: bool,
    editable: bool,
    key_draft_changed: Callback<(usize, String)>,
    key_store: Callback<usize>,
    key_clear: Callback<usize>,
) -> View {
    const KEY_PROVIDERS: [(&str, usize); 4] = [
        ("OpenAI", 0),
        ("Anthropic Claude", 1),
        ("Google Gemini", 2),
        ("Custom endpoint", 3),
    ];
    let rows: Vec<KeyedView> = KEY_PROVIDERS
        .iter()
        .map(|(label, index)| {
            let draft_changed = key_draft_changed.clone();
            let store = key_store.clone();
            let clear = key_clear.clone();
            let index = *index;
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
                                .content("Save"),
                            Button::new()
                                .height(32.0)
                                .width(58.0)
                                .is_enabled(editable && !key_busy && set)
                                .on_click(move || {
                                    let _ = clear.call(index);
                                })
                                .content("Clear"),
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
                        .text("Keys are stored with Windows DPAPI per provider — never in settings.json and never in the cloud.")
                        .font_size(11.5)
                        .foreground(palette.muted)
                        .text_wrapping(TextWrapping::Wrap),
                ),
            StackPanel::new().keyed_children(rows),
        ))
}

#[allow(clippy::too_many_arguments)]
fn settings_content(
    theme: WindowTheme,
    bottom: bool,
    settings: &AppSettings,
    provider_setup_partial: bool,
    editable: bool,
    provider_key_drafts: &[String; 4],
    provider_keys_set: [bool; 4],
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
) -> View {
    let palette = Palette::for_theme(theme);
    if bottom {
        return settings_content_bottom(
            palette,
            theme,
            settings,
            provider_setup_partial,
            editable,
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
    let cloud_fallback_index = Some(match settings.cloud_fallback_policy {
        CloudFallbackPolicy::Ask => 0,
        CloudFallbackPolicy::Allow => 1,
        CloudFallbackPolicy::Never => 2,
    });
    let (codex_model_items, codex_model_index) = codex_model_options(settings);

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
                settings_row(
                    palette,
                    "AI provider",
                    Some("Auto picks local first, then configured cloud providers"),
                    ComboBox::new()
                        .width(260.0)
                        .height(32.0)
                        .items_source(AI_PROVIDER_LABELS)
                        .selected_index(provider_index)
                        .is_enabled(editable)
                        .automation_name("AI provider")
                        .on_selection_changed(preferred_ai_provider_changed),
                    59.0,
                ),
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
                        .items_source([
                            "Phi Silica (on-device NPU)",
                            "Foundry Local (local server)",
                            "Ollama (local server)",
                            "ChatGPT via Codex CLI (subscription)",
                            "Claude via Claude Code CLI (subscription)",
                            "OpenAI (cloud)",
                            "Anthropic Claude",
                            "Google Gemini",
                            "OpenRouter",
                            "Custom OpenAI-compatible endpoint",
                        ])
                        .selected_index(Some(3))
                        .is_enabled(!provider_setup_partial),
                    100.0,
                ),
                settings_row(
                    palette,
                    "ChatGPT account",
                    Some("OpenAI's own login opens in your browser; usage bills to your ChatGPT plan"),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((
                            status_pill(
                                if provider_setup_partial {
                                    "Native setup pending"
                                } else {
                                    "Signed in"
                                },
                                if provider_setup_partial {
                                    palette.warn
                                } else {
                                    palette.accent
                                },
                                if provider_setup_partial {
                                    palette.warn_bg
                                } else {
                                    palette.active
                                },
                            ),
                            Button::new()
                                .height(32.0)
                                .is_enabled(!provider_setup_partial)
                                .content("Sign out"),
                        )),
                    58.0,
                ),
                settings_wrapped_row(
                    palette,
                    "CLI path",
                    Some("Optional. Empty auto-detects codex — use Install CLI above if it is missing"),
                    TextBox::new()
                        .width(260.0)
                        .height(32.0)
                        .text(settings.codex_cli_path.clone().unwrap_or_default())
                        .placeholder_text("Auto-detected")
                        .is_enabled(editable)
                        .on_text_changed(codex_cli_path_changed)
                        .automation_name("Codex CLI path"),
                    100.0,
                ),
                settings_wrapped_row(
                    palette,
                    "Model",
                    Some("Optional. Empty uses the CLI's default model"),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(6.0)
                        .children((
                            ComboBox::new()
                                .width(182.0)
                                .height(32.0)
                                .items_source(codex_model_items)
                                .selected_index(codex_model_index)
                                .is_enabled(editable)
                                .automation_name("Codex model")
                                .on_selection_changed(codex_model_changed),
                            Button::new()
                                .height(32.0)
                                .is_enabled(!provider_setup_partial)
                                .content(fa_icon_label(FaIcon::Refresh, "Refresh")),
                        )),
                    100.0,
                ),
                Border::new()
                    .padding(Thickness::new(0.0, 10.0, 0.0, 12.0))
                    .border_brush(palette.border)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        TextBlock::new()
                            .text("Runs through OpenAI's Codex CLI with your ChatGPT plan — no API key, and this app never stores an OpenAI token.")
                            .font_size(13.0)
                            .text_wrapping(TextWrapping::Wrap),
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
fn settings_content_bottom(
    palette: Palette,
    theme: WindowTheme,
    settings: &AppSettings,
    provider_setup_partial: bool,
    editable: bool,
    provider_key_drafts: &[String; 4],
    provider_keys_set: [bool; 4],
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

fn settings_section(palette: Palette, label: &'static str) -> View {
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

fn settings_row(
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

fn settings_label(palette: Palette, label: &'static str, hint: Option<&'static str>) -> View {
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

fn settings_wrapped_row(
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

fn settings_check_row(
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

fn write_version_probe_if_requested() -> bool {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(VERSION_PROBE_FLAG)) {
        return false;
    }

    // The capture harness passes the destination through the environment so
    // paths containing spaces or non-ASCII characters never need shell
    // quoting. Reject additional arguments to keep this probe deterministic.
    if arguments.next().is_some() {
        std::process::exit(2);
    }
    let Some(path) = std::env::var_os(VERSION_PROBE_FILE_ENV).filter(|path| !path.is_empty())
    else {
        std::process::exit(2);
    };
    if std::fs::write(path, version_probe_document()).is_err() {
        std::process::exit(3);
    }
    true
}

fn version_probe_document() -> String {
    format!("{{\"schema\":1,\"application_version\":\"{APP_VERSION}\"}}\n")
}

fn main() {
    // This probe must remain ahead of App::run_component so version validation
    // never initializes WinUI or creates a visible window.
    if write_version_probe_if_requested() {
        return;
    }
    // Refuse a second instance before any WinUI work; the primary is asked
    // to come to the foreground via the activation event.
    if let instance_support::SingleInstanceDecision::Secondary =
        instance_support::acquire("com.windowsforum.diagnostics")
    {
        return;
    }
    App::run_component::<WfdiagSpike>(()).unwrap();
}

#[cfg(test)]
mod diagnostic_selection_tests {
    use super::*;

    #[test]
    fn version_probe_document_uses_the_canonical_build_version() {
        assert_eq!(
            version_probe_document(),
            format!(
                "{{\"schema\":1,\"application_version\":\"{}\"}}\n",
                env!("WFDIAG_APP_VERSION")
            )
        );
    }

    fn provider_status(active_provider: AIProvider) -> AIProviderStatus {
        AIProviderStatus {
            preferred_provider: active_provider,
            openai_available: active_provider == AIProvider::OpenAI,
            openai_api_key_set: active_provider == AIProvider::OpenAI,
            phi_silica_available: active_provider == AIProvider::PhiSilica,
            phi_silica_ready: active_provider == AIProvider::PhiSilica,
            phi_silica_message: None,
            foundry_local_available: active_provider == AIProvider::FoundryLocal,
            foundry_local_endpoint: None,
            active_provider,
            providers: Vec::new(),
        }
    }

    #[test]
    fn ai_provider_pill_is_fixture_exact_but_live_state_driven() {
        assert_eq!(
            ai_provider_pill_content(true, true, None, false, None),
            (
                "Phi Silica".to_string(),
                "·  On device".to_string(),
                None,
                true,
                false
            )
        );
        assert_eq!(
            ai_provider_pill_content(false, true, None, true, None),
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
    fn settings_dialog_epoch_rejects_closed_and_reopened_dialog_callbacks() {
        assert!(settings_dialog_callback_is_current(true, 7, 7));
        assert!(!settings_dialog_callback_is_current(false, 7, 7));
        assert!(!settings_dialog_callback_is_current(true, 8, 7));
        assert!(SettingsDialogAction::AutoSaveChanged(false).changes_draft());
        assert!(!SettingsDialogAction::ThemeSelectionChanged(None).changes_draft());
        assert!(!SettingsDialogAction::Save.changes_draft());
    }

    #[test]
    fn about_dialog_epoch_rejects_closed_and_reopened_action_callbacks() {
        assert!(about_dialog_callback_is_current(true, 11, 11));
        assert!(!about_dialog_callback_is_current(false, 11, 11));
        assert!(!about_dialog_callback_is_current(true, 12, 11));
    }

    #[test]
    fn update_notice_hover_pause_preserves_only_unelapsed_time() {
        assert_eq!(
            update_notice_remaining_after_elapsed(
                Duration::from_secs(5),
                Duration::from_millis(1_250)
            ),
            Duration::from_millis(3_750)
        );
        assert_eq!(
            update_notice_remaining_after_elapsed(Duration::from_secs(5), Duration::from_secs(6)),
            Duration::ZERO
        );
    }

    #[test]
    fn update_notice_rejects_old_timer_messages_after_resume_or_replacement() {
        // Hover-pause cancels generation 3; immediate resume starts generation
        // 4 in the same notice epoch. The late cancellation cannot clear it.
        assert!(!update_notice_timer_callback_is_current(true, 7, 4, 7, 3));
        assert!(update_notice_timer_callback_is_current(true, 7, 4, 7, 4));

        // A new update notice advances the epoch as well, so every completion
        // from the replaced notice is stale even if a generation wrapped.
        assert!(!update_notice_timer_callback_is_current(true, 8, 1, 7, 1));
        assert!(!update_notice_timer_callback_is_current(false, 8, 1, 8, 1));
    }

    #[test]
    fn about_update_icons_use_the_pinned_font_awesome_sources() {
        assert_eq!(FaIcon::CircleUp.source_name(), "circle-up");
        assert_eq!(FaIcon::Globe.source_name(), "globe");
        assert_eq!(FaIcon::Github.source_name(), "github");
        assert_eq!(FaIcon::CircleUp.data(), icons::CIRCLE_UP);
        assert_eq!(FaIcon::Globe.data(), icons::GLOBE);
        assert_eq!(FaIcon::Github.data(), icons::GITHUB);
    }

    #[test]
    fn settings_save_completion_takes_only_the_matching_submitted_payload() {
        let submitted = AppSettings {
            theme: "light".to_string(),
            export_format: "json".to_string(),
            ..AppSettings::default()
        };
        let mut pending = Some(PendingSettingsSave {
            request_id: 23,
            dialog_epoch: 5,
            submitted: submitted.clone(),
        });

        assert!(take_matching_pending_settings_save(&mut pending, 22).is_none());
        assert_eq!(pending.as_ref().map(|save| save.request_id), Some(23));

        let matched = take_matching_pending_settings_save(&mut pending, 23).unwrap();
        assert_eq!(matched.dialog_epoch, 5);
        assert_eq!(matched.submitted, submitted);
        assert!(pending.is_none());
    }

    #[test]
    fn system_completion_ids_reject_stale_results_and_clear_only_the_match() {
        let mut system_info_request_id = Some(41);
        let mut architecture_request_id = Some(42);

        assert_eq!(
            take_matching_system_request(
                &mut system_info_request_id,
                &mut architecture_request_id,
                40,
            ),
            None
        );
        assert_eq!(system_info_request_id, Some(41));
        assert_eq!(architecture_request_id, Some(42));

        assert_eq!(
            take_matching_system_request(
                &mut system_info_request_id,
                &mut architecture_request_id,
                42,
            ),
            Some(SystemRequestKind::Architecture)
        );
        assert_eq!(system_info_request_id, Some(41));
        assert_eq!(architecture_request_id, None);

        assert_eq!(
            take_matching_system_request(
                &mut system_info_request_id,
                &mut architecture_request_id,
                41,
            ),
            Some(SystemRequestKind::SystemInfo)
        );
        assert_eq!(system_info_request_id, None);
    }

    #[test]
    fn deterministic_machine_identity_remains_the_exact_store_2_5_8_fixture() {
        let system_info = fixture_258_system_info();
        assert_eq!(system_info.computer_name, "ANDROMEDA");
        assert_eq!(system_info.os_version, "Windows 11 Professional (25H2)");
        assert!(!system_info.is_admin);
        assert_eq!(privilege_label(system_info.is_admin), "Standard user");
        assert_eq!(
            machine_card_accessibility_name(&system_info, None, None),
            "Computer ANDROMEDA, Windows 11 Professional (25H2), Standard user"
        );
    }

    #[test]
    fn machine_identity_accessibility_exposes_native_architecture_and_errors() {
        let architecture = ArchitectureSnapshot {
            process_architecture: 9,
            process_architecture_name: "x64".to_string(),
            native_architecture: 12,
            native_architecture_name: "ARM64".to_string(),
            is_emulated: true,
            page_size: 4096,
            processor_count: 12,
            emulation_status: "x64 app running on ARM64 hardware".to_string(),
        };
        assert_eq!(
            machine_card_accessibility_name(
                &fixture_258_system_info(),
                Some(&architecture),
                Some("architecture probe degraded"),
            ),
            "Computer ANDROMEDA, Windows 11 Professional (25H2), Standard user, x64 app running on ARM64 hardware, system information warning: architecture probe degraded"
        );
        assert_eq!(privilege_label(true), "Administrator");
    }

    fn task(id: &str, admin_required: bool) -> DiagnosticTask {
        DiagnosticTask {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            category: "Test".to_string(),
            admin_required,
        }
    }

    #[test]
    fn quick_scan_is_the_exact_2_5_8_non_admin_union() {
        let mut catalog = QUICK_SCAN_TASK_IDS
            .iter()
            .map(|id| task(id, false))
            .collect::<Vec<_>>();
        catalog.push(task("event_logs", false));
        catalog.push(task("chkdsk", true));

        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Quick, true, None),
            QUICK_SCAN_TASK_IDS.map(str::to_string)
        );
    }

    #[test]
    fn customised_quick_scan_keeps_every_store_detection_source() {
        let catalog = vec![
            task("comp_system", false),
            task("os_info", false),
            task("defender_status", false),
            task("firewall_status", false),
        ];
        let custom = ["os_info".to_string()];

        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Quick, false, Some(&custom)),
            [
                "os_info".to_string(),
                "defender_status".to_string(),
                "firewall_status".to_string()
            ]
        );
    }

    #[test]
    fn scan_start_policy_snapshots_auto_save_concurrency_and_exact_tag() {
        let mut settings = AppSettings {
            auto_save: false,
            max_concurrent_tasks: 9,
            ..AppSettings::default()
        };
        let policy = DiagnosticScanPolicy::snapshot(&settings, ScanKind::Full);
        settings.auto_save = true;
        settings.max_concurrent_tasks = 2;

        assert_eq!(policy.max_concurrent_tasks, 9);
        assert_eq!(policy.history_tag, "Full Scan");
        assert!(!scan_policy_requests_auto_save(Some(&policy)));
        assert!(!scan_policy_requests_auto_save(None));
        assert_eq!(scan_concurrency_from_settings(0), 5);
    }

    #[test]
    fn history_retention_provider_tracks_only_committed_settings() {
        let policy = RwLock::new(HistoryRetentionPolicy {
            retain_history: true,
            history_limit: 30,
        });
        let settings = AppSettings {
            retain_history: false,
            history_limit: 7,
            ..AppSettings::default()
        };

        update_history_retention_policy(&policy, &settings);
        assert_eq!(history_retention_tuple(&policy), (false, 7));
    }

    #[test]
    fn completed_scan_record_matches_shipping_fields_and_converts_results_explicitly() {
        let system_info = SystemInfo {
            computer_name: "HISTORY-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: true,
        };
        let results = vec![
            DiagnosticTaskResult {
                session_id: "scan-7".to_string(),
                task_id: "os_info".to_string(),
                success: true,
                output: "ok".to_string(),
                error: None,
                duration_ms: 12,
            },
            DiagnosticTaskResult {
                session_id: "scan-7".to_string(),
                task_id: "logical_disk".to_string(),
                success: false,
                output: "partial".to_string(),
                error: Some("denied".to_string()),
                duration_ms: 34,
            },
        ];

        let record = build_history_scan_record(
            "scan-7".to_string(),
            &system_info,
            &results,
            742,
            "Quick Scan".to_string(),
        );
        assert_eq!(record.id, "scan-7");
        assert_eq!(record.computer_name, "HISTORY-PC");
        assert_eq!(record.os_version, "Windows 11");
        assert!(record.is_admin);
        assert_eq!(record.task_count, 2);
        assert_eq!(record.success_count, 1);
        assert_eq!(record.failure_count, 1);
        assert_eq!(record.duration_ms, 742);
        assert!(record.label.is_none());
        assert_eq!(record.tags, ["Quick Scan"]);
        let failed = &record.results["logical_disk"];
        assert!(!failed.success);
        assert_eq!(failed.output, "partial");
        assert_eq!(failed.error.as_deref(), Some("denied"));
        assert_eq!(failed.duration_ms, 34);
    }

    #[test]
    fn authoritative_session_snapshot_replaces_event_order_with_catalog_order() {
        let catalog = vec![task("first", false), task("second", false)];
        let snapshot = HashMap::from([
            (
                "second".to_string(),
                DiagnosticOutput {
                    success: false,
                    output: "two".to_string(),
                    error: Some("failed".to_string()),
                    duration_ms: 2,
                },
            ),
            (
                "first".to_string(),
                DiagnosticOutput {
                    success: true,
                    output: "one".to_string(),
                    error: None,
                    duration_ms: 1,
                },
            ),
        ]);

        let results = authoritative_ui_results("session-a", &snapshot, &catalog);
        assert_eq!(
            results
                .iter()
                .map(|result| result.task_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(
            results
                .iter()
                .all(|result| result.session_id == "session-a")
        );
        assert_eq!(results[1].error.as_deref(), Some("failed"));
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot["first"].output, "one");
    }

    #[test]
    fn authoritative_result_set_requires_the_exact_selected_task_ids() {
        let expected = vec!["first".to_string(), "second".to_string()];
        let output = || DiagnosticOutput {
            success: true,
            output: "ok".to_string(),
            error: None,
            duration_ms: 1,
        };
        let exact = HashMap::from([
            ("first".to_string(), output()),
            ("second".to_string(), output()),
        ]);
        assert!(authoritative_result_set_is_complete(&exact, &expected));

        let missing = HashMap::from([("first".to_string(), output())]);
        assert!(!authoritative_result_set_is_complete(&missing, &expected));

        let extra = HashMap::from([
            ("first".to_string(), output()),
            ("second".to_string(), output()),
            ("stale".to_string(), output()),
        ]);
        assert!(!authoritative_result_set_is_complete(&extra, &expected));

        let same_size_substitution = HashMap::from([
            ("first".to_string(), output()),
            ("stale".to_string(), output()),
        ]);
        assert!(!authoritative_result_set_is_complete(
            &same_size_substitution,
            &expected,
        ));

        let duplicate_expected = vec!["first".to_string(), "first".to_string()];
        assert!(!authoritative_result_set_is_complete(
            &HashMap::from([("first".to_string(), output())]),
            &duplicate_expected,
        ));
    }

    #[test]
    fn full_scan_includes_admin_checks_only_when_elevated() {
        let catalog = vec![task("os_info", false), task("chkdsk", true)];
        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Full, false, None),
            ["os_info".to_string()]
        );
        assert_eq!(
            select_scan_tasks(&catalog, ScanKind::Full, true, None),
            ["os_info".to_string(), "chkdsk".to_string()]
        );
    }

    #[test]
    fn live_scan_waits_for_privilege_identity_without_affecting_visual_fixtures() {
        assert!(system_identity_blocks_scan(false, Some(17)));
        assert!(!system_identity_blocks_scan(false, None));
        assert!(!system_identity_blocks_scan(true, Some(17)));
    }

    #[test]
    fn latest_history_scan_never_reports_a_pending_comparison() {
        assert_eq!(
            history_comparison_placeholder(Some("latest"), Some("latest"), false, None),
            "The latest scan is the comparison baseline. Select an earlier scan to compare."
        );
        assert_eq!(
            history_comparison_placeholder(Some("older"), Some("latest"), true, None),
            "Loading comparison…"
        );
        assert_eq!(
            history_comparison_placeholder(Some("older"), Some("latest"), false, None),
            "Comparison is unavailable. Select the scan again to retry."
        );
    }
}
