//! Compile-time constants shared by the shell: asset payloads, task id
//! unions, layout breakpoints, and the timing budgets the update loop uses.

#![deny(unsafe_code)]

use std::time::Duration;
use wfdiag_native_ai_provider::AIProvider;

pub(crate) const APP_VERSION: &str = env!("WFDIAG_APP_VERSION");

pub(crate) const HISTORY_TREND_SCAN_LIMIT: usize = 10;

pub(crate) const ABOUT_DESCRIPTION: &str = "A native Windows diagnostics tool by WindowsForum.com. Runs hardware, driver, storage, network, security and log diagnostics locally — with optional on-device or cloud AI analysis.";

/// Validation-only entry point (#212): the probe itself lives in
/// `fixtures::knobs` and is compiled out without the `validation` feature, so
/// its flag and destination variable are too.
#[cfg(feature = "validation")]
pub(crate) const VERSION_PROBE_FLAG: &str = "--wfdiag-version-probe";

pub(crate) const ELEVATED_RELAUNCH_FLAG: &str = "--wfdiag-elevated-relaunch";

#[cfg(feature = "validation")]
pub(crate) const VERSION_PROBE_FILE_ENV: &str = "WFDIAG_REACTOR_VERSION_PROBE_FILE";

pub(crate) const APP_BADGE: &[u8] = include_bytes!("../../../../public/wf-ds/app-badge.png");

pub(crate) const BOT_AVATAR: &[u8] =
    include_bytes!("../../../../public/wf-ds/chatgpt-bot-avatar.webp");

pub(crate) const STETHOSCOPE_LIGHT: &[u8] = include_bytes!("../../assets/stethoscope-light.png");

pub(crate) const STETHOSCOPE_DARK: &[u8] = include_bytes!("../../assets/stethoscope-dark.png");

pub(crate) const STATUS_INFO_LIGHT: &[u8] =
    include_bytes!("../../assets/status-circle-info-light.png");

pub(crate) const STATUS_INFO_DARK: &[u8] =
    include_bytes!("../../assets/status-circle-info-dark.png");

pub(crate) const STATUS_OK_LIGHT: &[u8] =
    include_bytes!("../../assets/status-circle-check-light.png");

pub(crate) const STATUS_OK_DARK: &[u8] =
    include_bytes!("../../assets/status-circle-check-dark.png");

pub(crate) const STATUS_WARN_LIGHT: &[u8] =
    include_bytes!("../../assets/status-triangle-exclamation-light.png");

pub(crate) const STATUS_WARN_DARK: &[u8] =
    include_bytes!("../../assets/status-triangle-exclamation-dark.png");

pub(crate) const WAND_LIGHT: &[u8] = include_bytes!("../../assets/wand-magic-sparkles-light.png");

pub(crate) const WAND_DARK: &[u8] = include_bytes!("../../assets/wand-magic-sparkles-dark.png");

pub(crate) const DESKTOP_LIGHT: &[u8] = include_bytes!("../../assets/desktop-light.png");

pub(crate) const DESKTOP_DARK: &[u8] = include_bytes!("../../assets/desktop-dark.png");

pub(crate) const ISSUE_USER_SHIELD_LIGHT: &[u8] =
    include_bytes!("../../assets/issue-user-shield-light.png");

pub(crate) const ISSUE_USER_SHIELD_DARK: &[u8] =
    include_bytes!("../../assets/issue-user-shield-dark.png");

pub(crate) const ISSUE_INFO_LIGHT: &[u8] =
    include_bytes!("../../assets/issue-circle-info-light.png");

pub(crate) const ISSUE_INFO_DARK: &[u8] = include_bytes!("../../assets/issue-circle-info-dark.png");

pub(crate) const ISSUE_WARN_LIGHT: &[u8] =
    include_bytes!("../../assets/issue-triangle-exclamation-light.png");

pub(crate) const ISSUE_WARN_DARK: &[u8] =
    include_bytes!("../../assets/issue-triangle-exclamation-dark.png");

pub(crate) const ISSUE_SHIELD_LIGHT: &[u8] =
    include_bytes!("../../assets/issue-shield-halved-light.png");

pub(crate) const ISSUE_SHIELD_DARK: &[u8] =
    include_bytes!("../../assets/issue-shield-halved-dark.png");

pub(crate) const ISSUE_STETHOSCOPE_LIGHT: &[u8] =
    include_bytes!("../../assets/issue-stethoscope-light.png");

pub(crate) const ISSUE_STETHOSCOPE_DARK: &[u8] =
    include_bytes!("../../assets/issue-stethoscope-dark.png");

// WinUI has no per-element backdrop-blur brush at the pinned Reactor revision.
// These are deterministic, pre-blurred derivatives of the two canonical WF assets.
pub(crate) const WALLPAPER_LIGHT: &[u8] =
    include_bytes!("../../assets/bg-24H4-native-blurred.webp");

pub(crate) const WALLPAPER_DARK: &[u8] =
    include_bytes!("../../assets/bg-24H4-oled-native-blurred.webp");

// This is the exact 2.5.8 default Quick Scan union from `useScanner.ts`:
// baseline inventory plus the cheap, non-admin issue-detection sources.
pub(crate) const QUICK_SCAN_TASK_IDS: [&str; 17] = [
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

pub(crate) const PROCESS_PAGE_SIZE: usize =
    wfdiag_native_projection::render::PROCESS_REPEATER_SLOTS;

pub(crate) const PROCESS_FILTER_DEBOUNCE: Duration = Duration::from_millis(180);

/// How often the Processes page re-reads the table while telemetry is live.
///
/// Telemetry itself ticks every second; enumerating processes that often is
/// wasteful, so the page rides every other sample.
pub(crate) const PROCESS_LIVE_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) const DIAGNOSTICS_COMPACT_BREAKPOINT: f64 = 840.0;

pub(crate) const PROCESS_WIDE_CONTENT_MIN_WIDTH: f64 = 1_012.0;

pub(crate) const SHELL_CONTENT_HORIZONTAL_CHROME: f64 = 72.0;

pub(crate) const PROCESS_DETAILS_COLUMN_WIDTH: f64 = 312.0;

pub(crate) const AI_WORKSPACE_VERTICAL_CHROME: f64 = 243.0;

pub(crate) const AI_WORKSPACE_MIN_HEIGHT: f64 = 240.0;

pub(crate) const PALETTE_FOCUS_DELAY: Duration = Duration::from_millis(125);

pub(crate) const PALETTE_RESTORE_DELAY: Duration = Duration::from_millis(200);

pub(crate) const SETTINGS_MAX_CONCURRENT_TASKS: u32 = 16;

/// Interval of the DEGRADED instance/lifecycle watch (#207).
///
/// This poll only runs when `RegisterWaitForSingleObject` refused to arm the
/// event-driven path; the healthy build never spins at all. It used to run at
/// 50 ms forever, which burned a wakeup 20×/second for the whole session for
/// no benefit — tray clicks and single-instance activation are not latency
/// critical, so the degraded fallback is a quarter-second and the shell says
/// so once in the status line.
pub(crate) const WINDOW_COMMAND_POLL: Duration = Duration::from_millis(250);

/// How long the shell waits for the engine's workers to stop on exit.
///
/// This is the budget `AppService::shutdown` reaps each worker inside; the
/// tray Exit path must not hang on a worker that will not stop.
pub(crate) const ENGINE_SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);

pub(crate) const WINDOW_HOOK_RETRY_MIN: Duration = Duration::from_millis(100);

pub(crate) const WINDOW_HOOK_RETRY_MAX: Duration = Duration::from_millis(3_200);

pub(crate) const PROVIDER_KEY_LABELS: [&str; 5] = [
    "OpenAI",
    "Anthropic Claude",
    "Google Gemini",
    "DeepSeek",
    "Custom endpoint",
];

pub(crate) const AI_PROVIDER_LABELS: [&str; 11] = [
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

pub(crate) const AI_PROVIDER_IDS: [&str; 11] = [
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

pub(crate) const CODEX_MODEL_IDS: [&str; 3] = ["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.5"];

pub(crate) const PROVIDER_SETUP_LABELS: [&str; 10] = [
    "Phi Silica (on-device NPU)",
    "Foundry Local (local server)",
    "Ollama (local server)",
    "ChatGPT via Codex CLI",
    "Claude via Claude Code CLI",
    "OpenAI",
    "Anthropic Claude",
    "Google Gemini",
    "DeepSeek",
    "Custom OpenAI-compatible endpoint",
];

pub(crate) const PROVIDER_SETUP_PROVIDERS: [AIProvider; 10] = [
    AIProvider::PhiSilica,
    AIProvider::FoundryLocal,
    AIProvider::Ollama,
    AIProvider::CodexCli,
    AIProvider::ClaudeCode,
    AIProvider::OpenAI,
    AIProvider::Anthropic,
    AIProvider::Gemini,
    AIProvider::DeepSeek,
    AIProvider::CustomOpenAI,
];
