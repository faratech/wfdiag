//! Pure decision helpers lifted out of the component.
//!
//! Nothing here touches component state, Windows, or a worker: every function
//! is a total mapping from its arguments, which is what keeps them testable.

#![deny(unsafe_code)]

use crate::app::consts::{
    AI_WORKSPACE_MIN_HEIGHT, AI_WORKSPACE_VERTICAL_CHROME, CODEX_MODEL_IDS,
    DIAGNOSTICS_COMPACT_BREAKPOINT, PROCESS_DETAILS_COLUMN_WIDTH, PROCESS_WIDE_CONTENT_MIN_WIDTH,
    PROVIDER_SETUP_PROVIDERS, SHELL_CONTENT_HORIZONTAL_CHROME, WINDOW_HOOK_RETRY_MAX,
    WINDOW_HOOK_RETRY_MIN,
};
use crate::app::message::HistoryChangeKind;
use crate::app::state::{HistoryTrendBadge, Page};
use crate::platform::window;
use std::sync::Arc;
use std::time::Duration;
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallProgress, SubscriptionInstallStage,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, AIProviderStatus, FoundryCliEndpointSource,
    PackageIdentitySource, ProcessSubscriptionCliStatusSource, ProviderManagementBackend,
    ProviderManagementService, ProviderModelDefaults, ProviderProbeBundle, ProviderSelectionState,
    SettingsServiceProviderConfigurationSource, SharedAiCache, parse_provider_preference,
};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_export::ReportFormat;
use wfdiag_native_history::{ComparisonSummary, ScanSummary, TaskChangeSummary, TaskTrend};
use wfdiag_native_issues::RemediationTier;
use wfdiag_native_phi::WindowsPhiStatusSource;
use wfdiag_native_remediation::broker::ActionProposal;
use wfdiag_native_remediation::remediation;
use wfdiag_native_remediation::runtime::{ActionRunStatus, ActionRunSummary};
#[cfg(feature = "settings-test-path")]
use wfdiag_native_settings::{
    AllowAllSettings, ShippingSettingsStorage, WindowsDpapiCredentialStorage,
};
use wfdiag_native_settings::{
    AppSettings, ProviderKeyId, SettingsService, SettingsValidator,
    windows_shipping_settings_service,
};
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use wfdiag_native_update::{SignatureProvider, WindowsPackageSignatureProvider};
use windows_reactor::*;

pub(crate) fn provider_setup_provider(index: usize) -> Option<AIProvider> {
    PROVIDER_SETUP_PROVIDERS.get(index).copied()
}

pub(crate) fn provider_setup_index_for_provider(provider: AIProvider) -> Option<usize> {
    PROVIDER_SETUP_PROVIDERS
        .iter()
        .position(|candidate| *candidate == provider)
}

pub(crate) fn configured_provider_setup_index(settings: &AppSettings) -> usize {
    let explicit = parse_provider_preference(&settings.preferred_ai_provider);
    if explicit != AIProviderPreference::Auto {
        let provider = match explicit {
            AIProviderPreference::Auto => AIProvider::None,
            AIProviderPreference::OpenAI => AIProvider::OpenAI,
            AIProviderPreference::PhiSilica => AIProvider::PhiSilica,
            AIProviderPreference::FoundryLocal => AIProvider::FoundryLocal,
            AIProviderPreference::Ollama => AIProvider::Ollama,
            AIProviderPreference::CustomOpenAI => AIProvider::CustomOpenAI,
            AIProviderPreference::CodexCli => AIProvider::CodexCli,
            AIProviderPreference::ClaudeCode => AIProvider::ClaudeCode,
            AIProviderPreference::Anthropic => AIProvider::Anthropic,
            AIProviderPreference::Gemini => AIProvider::Gemini,
            AIProviderPreference::DeepSeek => AIProvider::DeepSeek,
        };
        if let Some(index) = provider_setup_index_for_provider(provider) {
            return index;
        }
    }

    if settings.phi_silica_laf_token.is_some() {
        0
    } else if settings.local_ai_endpoint.is_some() {
        1
    } else if settings.ollama_endpoint.is_some() || settings.ollama_model.is_some() {
        2
    } else if settings.custom_endpoint.is_some()
        || settings.custom_model.is_some()
        || settings.custom_api_key_set
    {
        9
    } else if settings.codex_cli_path.is_some() || settings.codex_model.is_some() {
        3
    } else if settings.claude_cli_path.is_some() || settings.claude_model.is_some() {
        4
    } else if settings.open_ai_api_key_set {
        5
    } else if settings.anthropic_api_key_set || settings.anthropic_model.is_some() {
        6
    } else if settings.gemini_api_key_set || settings.gemini_model.is_some() {
        7
    } else if settings.deepseek_api_key_set || settings.deepseek_model.is_some() {
        8
    } else {
        5
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PhiPreferenceGate {
    Checking,
    Ready,
    Blocked(String),
}

impl PhiPreferenceGate {
    pub(crate) fn blocking_reason(&self) -> Option<&str> {
        match self {
            Self::Checking => Some(
                "Checking whether Phi Silica is available on this PC. Wait for the check to finish before selecting it.",
            ),
            Self::Ready => None,
            Self::Blocked(reason) => Some(reason),
        }
    }
}

pub(crate) fn phi_preference_gate(
    provider_status: Option<&AIProviderStatus>,
    provider_loading: bool,
) -> PhiPreferenceGate {
    if provider_loading || provider_status.is_none() {
        return PhiPreferenceGate::Checking;
    }
    let status = provider_status.expect("provider status checked above");
    if status.phi_silica_available && status.phi_silica_ready {
        PhiPreferenceGate::Ready
    } else {
        PhiPreferenceGate::Blocked(
            status.phi_silica_message.clone().unwrap_or_else(|| {
                "Phi Silica is unavailable or not ready on this PC.".to_string()
            }),
        )
    }
}

pub(crate) fn validate_phi_preference(
    preference: &str,
    gate: &PhiPreferenceGate,
) -> Result<(), String> {
    if preference.eq_ignore_ascii_case("phi_silica")
        && let Some(reason) = gate.blocking_reason()
    {
        return Err(reason.to_string());
    }
    Ok(())
}

pub(crate) fn subscription_auth_provider_for_setup(
    index: usize,
) -> Option<SubscriptionAuthProvider> {
    match index {
        3 => Some(SubscriptionAuthProvider::Codex),
        4 => Some(SubscriptionAuthProvider::ClaudeCode),
        _ => None,
    }
}

pub(crate) fn subscription_auth_state_index(provider: SubscriptionAuthProvider) -> usize {
    match provider {
        SubscriptionAuthProvider::Codex => 0,
        SubscriptionAuthProvider::ClaudeCode => 1,
    }
}

pub(crate) const fn subscription_install_progress_label(
    progress: SubscriptionInstallProgress,
) -> &'static str {
    match progress.stage {
        SubscriptionInstallStage::CheckingExisting => "Checking for an existing CLI installation…",
        SubscriptionInstallStage::ResolvingInstaller => "Resolving the approved installer…",
        SubscriptionInstallStage::InstallingWinget => "Installing with Windows Package Manager…",
        SubscriptionInstallStage::InstallingVendorFallback => {
            "Running the separately approved vendor installer…"
        }
        SubscriptionInstallStage::Verifying => "Verifying the installed CLI path…",
        SubscriptionInstallStage::Completed => "CLI installation verified",
    }
}

pub(crate) fn provider_setup_model(index: usize, settings: &AppSettings) -> Option<&str> {
    match index {
        1 => settings.local_ai_model.as_deref(),
        2 => settings.ollama_model.as_deref(),
        3 => settings.codex_model.as_deref(),
        4 => settings.claude_model.as_deref(),
        5 => settings.open_ai_model.as_deref(),
        6 => settings.anthropic_model.as_deref(),
        7 => settings.gemini_model.as_deref(),
        8 => settings.deepseek_model.as_deref(),
        9 => settings.custom_model.as_deref(),
        _ => None,
    }
}

pub(crate) fn set_provider_setup_model(
    index: usize,
    settings: &mut AppSettings,
    model: Option<String>,
) {
    match index {
        1 => settings.local_ai_model = model,
        2 => settings.ollama_model = model,
        3 => settings.codex_model = model,
        4 => settings.claude_model = model,
        5 => settings.open_ai_model = model,
        6 => settings.anthropic_model = model,
        7 => settings.gemini_model = model,
        8 => settings.deepseek_model = model,
        9 => settings.custom_model = model,
        _ => {}
    }
}

pub(crate) fn set_provider_key_configured(
    settings: &mut AppSettings,
    provider: ProviderKeyId,
    configured: bool,
) {
    match provider {
        ProviderKeyId::OpenAI => settings.open_ai_api_key_set = configured,
        ProviderKeyId::Anthropic => settings.anthropic_api_key_set = configured,
        ProviderKeyId::Gemini => settings.gemini_api_key_set = configured,
        ProviderKeyId::DeepSeek => settings.deepseek_api_key_set = configured,
        ProviderKeyId::Custom => settings.custom_api_key_set = configured,
    }
}

pub(crate) fn set_provider_key_value(
    settings: &mut AppSettings,
    provider: ProviderKeyId,
    value: Option<String>,
) {
    match provider {
        ProviderKeyId::OpenAI => settings.open_ai_api_key = value,
        ProviderKeyId::Anthropic => settings.anthropic_api_key = value,
        ProviderKeyId::Gemini => settings.gemini_api_key = value,
        ProviderKeyId::DeepSeek => settings.deepseek_api_key = value,
        ProviderKeyId::Custom => settings.custom_api_key = value,
    }
}

pub(crate) fn strip_provider_key_values(settings: &mut AppSettings) {
    for provider in ProviderKeyId::ALL {
        set_provider_key_value(settings, provider, None);
    }
}

pub(crate) fn non_empty_provider_draft(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// The unsaved provider-setup values a catalog refresh discovers with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderCatalogDraft {
    pub(crate) api_key: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) cli_path: Option<String>,
}

/// What the selected provider needs before discovery can even be attempted.
///
/// `Ok(None)` means the provider has no catalog at all (Phi Silica); `Err` is
/// the user-facing reason the Settings pane shows instead of an empty list.
pub(crate) fn provider_catalog_draft(
    setup_index: usize,
    settings: &AppSettings,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
) -> Result<Option<ProviderCatalogDraft>, String> {
    let Some(provider) = provider_setup_provider(setup_index) else {
        return Err("The selected provider is not recognized".to_string());
    };
    if provider == AIProvider::PhiSilica {
        return Ok(None);
    }
    let (api_key, key_configured) = match provider {
        AIProvider::OpenAI => (
            non_empty_provider_draft(&provider_key_drafts[0]),
            settings.open_ai_api_key_set,
        ),
        AIProvider::Anthropic => (
            non_empty_provider_draft(&provider_key_drafts[1]),
            settings.anthropic_api_key_set,
        ),
        AIProvider::Gemini => (
            non_empty_provider_draft(&provider_key_drafts[2]),
            settings.gemini_api_key_set,
        ),
        AIProvider::DeepSeek => (
            non_empty_provider_draft(&provider_key_drafts[3]),
            settings.deepseek_api_key_set,
        ),
        AIProvider::CustomOpenAI => (
            non_empty_provider_draft(&provider_key_drafts[4]),
            settings.custom_api_key_set,
        ),
        _ => (None, false),
    };
    if matches!(
        provider,
        AIProvider::OpenAI | AIProvider::Anthropic | AIProvider::Gemini | AIProvider::DeepSeek
    ) && api_key.is_none()
        && !key_configured
    {
        return Err("Enter an API key to load the available models.".to_string());
    }

    let endpoint = match provider {
        AIProvider::FoundryLocal => settings.local_ai_endpoint.clone(),
        AIProvider::Ollama => settings.ollama_endpoint.clone(),
        AIProvider::CustomOpenAI => settings.custom_endpoint.clone(),
        _ => None,
    };
    if provider == AIProvider::CustomOpenAI
        && endpoint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return Err("Enter an endpoint URL to load the available models.".to_string());
    }
    let cli_path = match provider {
        AIProvider::CodexCli => settings.codex_cli_path.clone(),
        AIProvider::ClaudeCode => settings.claude_cli_path.clone(),
        _ => None,
    };
    Ok(Some(ProviderCatalogDraft {
        api_key,
        endpoint,
        cli_path,
    }))
}

/// The status line one live scan transition produces.
pub(crate) fn scan_progress_text(
    label: &str,
    cancelling: bool,
    completed: usize,
    total: usize,
    task_name: &str,
) -> String {
    if cancelling {
        format!("Stopping {label} · {task_name}")
    } else {
        format!("{label} · {completed} of {total} collected · {task_name}")
    }
}

/// The status line one arrived task result produces.
pub(crate) fn scan_result_text(
    label: &str,
    running: bool,
    completed: usize,
    total: usize,
    errors: usize,
) -> String {
    if running {
        format!("{label} · {completed} of {total} collected · {errors} errors")
    } else {
        format!("{label} complete · {completed} collected · {errors} errors")
    }
}

/// The status line a finished scan produces.
///
/// History persistence is optional evidence: a scan whose auto-save failed is
/// still a complete scan, and says so before it says history was not saved.
pub(crate) fn scan_complete_text(
    label: &str,
    completed: usize,
    errors: usize,
    history_failed: bool,
) -> String {
    if history_failed {
        format!("{label} complete · {completed} collected · {errors} errors · history not saved")
    } else {
        format!("{label} complete · {completed} collected · {errors} errors")
    }
}

/// The status line a worker that missed its reply deadline produces (#195).
pub(crate) fn worker_timeout_text(worker: &str) -> String {
    format!("The {worker} worker did not answer in time · try again")
}

/// Map a subscription CLI's provider wire id back to its enum.
pub(crate) fn subscription_provider_from_wire(wire: &str) -> Option<SubscriptionAuthProvider> {
    match provider_from_wire(wire) {
        AIProvider::CodexCli => Some(SubscriptionAuthProvider::Codex),
        AIProvider::ClaudeCode => Some(SubscriptionAuthProvider::ClaudeCode),
        _ => None,
    }
}

/// Resolve a provider wire id back to its enum.
///
/// The ids are the ones [`AIProvider`]'s `Display` writes, which is what the
/// settings document and every engine event carry.
pub(crate) fn provider_from_wire(wire: &str) -> AIProvider {
    const PROVIDERS: [AIProvider; 10] = [
        AIProvider::OpenAI,
        AIProvider::PhiSilica,
        AIProvider::FoundryLocal,
        AIProvider::Ollama,
        AIProvider::CustomOpenAI,
        AIProvider::CodexCli,
        AIProvider::ClaudeCode,
        AIProvider::Anthropic,
        AIProvider::Gemini,
        AIProvider::DeepSeek,
    ];
    PROVIDERS
        .into_iter()
        .find(|provider| provider.to_string() == wire)
        .unwrap_or(AIProvider::None)
}

/// The status-line text for one refused command.
///
/// The engine already phrases its refusals for a user ("Enable AI insights in
/// Settings before sending"), so the typed detail is the message; only the
/// structural reasons get the enum's own prose.
pub(crate) fn rejection_text(reason: &wfdiag_app::RejectReason) -> String {
    match reason {
        wfdiag_app::RejectReason::Busy { detail }
        | wfdiag_app::RejectReason::Invalid { detail }
        | wfdiag_app::RejectReason::NotReady { detail } => detail.clone(),
        wfdiag_app::RejectReason::WorkerUnavailable { detail, .. } => detail.clone(),
        other => other.to_string(),
    }
}

/// Build the AI provider backend the engine probes through.
///
/// Package identity, the shared response cache and the shipping model
/// defaults are application composition, which is why this stays with the
/// shell rather than moving into `wfdiag-app`.
pub(crate) fn reactor_provider_backend(
    settings: SettingsService,
    identity: Arc<dyn PackageIdentitySource>,
    cache: SharedAiCache,
) -> Arc<dyn ProviderManagementBackend> {
    let probes = ProviderProbeBundle::shipping_networks(
        Arc::new(SettingsServiceProviderConfigurationSource::new(settings)),
        identity,
        Arc::new(WindowsPhiStatusSource),
        Arc::new(FoundryCliEndpointSource::new()),
        Arc::new(ProcessSubscriptionCliStatusSource::new()),
    );
    Arc::new(ProviderManagementService::new(
        probes,
        ProviderSelectionState::default(),
        Arc::new(cache),
        ProviderModelDefaults {
            foundry: "phi-4-mini".to_string(),
            openai: "gpt-5-nano".to_string(),
            anthropic: "claude-sonnet-5".to_string(),
            gemini: "gemini-3.6-flash".to_string(),
            deepseek: "deepseek-v4-flash".to_string(),
        },
    ))
}

/// The settings store the engine should use, when it is not the shipping one.
///
/// Only the `settings-test-path` validation feature ever redirects it.
pub(crate) fn reactor_settings_storage() -> Option<Arc<dyn wfdiag_native_settings::SettingsStorage>>
{
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = crate::fixtures::knobs::settings_test_path() {
        return Some(Arc::new(ShippingSettingsStorage::at_path(path.into())));
    }
    None
}

/// The update throttle the engine should use, when it is not the shipping one.
pub(crate) fn reactor_update_throttle_port() -> Option<Arc<dyn wfdiag_app::UpdateThrottlePort>> {
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = crate::fixtures::knobs::settings_test_path() {
        return Some(Arc::new(IsolatedUpdateThrottle {
            throttle: wfdiag_native_update::policy::UpdateThrottle::beside_settings_file(
                std::path::Path::new(&path),
            ),
        }));
    }
    None
}

/// The validation-only throttle stored beside an isolated settings file.
#[cfg(feature = "settings-test-path")]
#[derive(Debug)]
struct IsolatedUpdateThrottle {
    throttle: wfdiag_native_update::policy::UpdateThrottle,
}

#[cfg(feature = "settings-test-path")]
impl wfdiag_app::UpdateThrottlePort for IsolatedUpdateThrottle {
    fn should_check(&self, now_millis: u64) -> bool {
        self.throttle.should_check_at(now_millis)
    }

    fn record(&self, now_millis: u64) -> Result<(), String> {
        self.throttle.record_at(now_millis)
    }
}

pub(crate) fn window_theme_from_setting(value: &str) -> WindowTheme {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "system" => WindowTheme::System,
        "light" => WindowTheme::Light,
        _ => WindowTheme::Dark,
    }
}

pub(crate) fn window_theme_setting(theme: WindowTheme) -> &'static str {
    match theme {
        WindowTheme::System => "auto",
        WindowTheme::Light => "light",
        WindowTheme::Dark => "dark",
    }
}

pub(crate) fn effective_window_theme(theme: WindowTheme, color_scheme: ColorScheme) -> WindowTheme {
    match theme {
        WindowTheme::System => match color_scheme {
            ColorScheme::Light => WindowTheme::Light,
            ColorScheme::Dark => WindowTheme::Dark,
        },
        explicit => explicit,
    }
}

pub(crate) fn navigation_rail_forced_collapsed(client_width: f64) -> bool {
    client_width <= 1100.0
}

pub(crate) fn diagnostics_uses_compact_layout(client_width: f64) -> bool {
    client_width < DIAGNOSTICS_COMPACT_BREAKPOINT
}

/// Whether the shell's page host scrolls the open page (#193).
///
/// The host owns the **one** `ScrollViewer` in the window and always shows its
/// scrollbar when it has one. Pages return plain content: a viewer nested
/// inside this one is measured with unbounded height, so it never scrolls and
/// everything past the fold becomes unreachable by pointer, keyboard and UI
/// Automation alike.
///
/// The single exception is the full-width Diagnostics layout, which sizes
/// itself to the viewport and must not scroll at all; its compact layout does.
pub(crate) const fn page_host_scrolls(page: Page, diagnostics_compact: bool) -> bool {
    !matches!(page, Page::Diagnostics) || diagnostics_compact
}

pub(crate) fn process_layout_metrics(client_width: f64, pane_expanded: bool) -> (bool, f64) {
    let pane_width = if pane_expanded { 230.0 } else { 64.0 };
    let content_width = (client_width - pane_width - SHELL_CONTENT_HORIZONTAL_CHROME).max(1.0);
    let compact = content_width < PROCESS_WIDE_CONTENT_MIN_WIDTH;
    let details_width = if compact {
        0.0
    } else {
        PROCESS_DETAILS_COLUMN_WIDTH
    };
    (compact, (content_width - details_width).max(1.0))
}

pub(crate) fn ai_workspace_height(client_height: f64) -> f64 {
    (client_height - AI_WORKSPACE_VERTICAL_CHROME).max(AI_WORKSPACE_MIN_HEIGHT)
}

pub(crate) fn selected_setting_index(value: &str, values: &[&str]) -> Option<usize> {
    Some(
        values
            .iter()
            .position(|candidate| value.eq_ignore_ascii_case(candidate))
            .unwrap_or_default(),
    )
}

pub(crate) fn codex_model_options(settings: &AppSettings) -> (Vec<String>, Option<usize>) {
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
pub(crate) struct ReactorPackageIdentitySource {
    pub(crate) provider: WindowsPackageSignatureProvider,
}

impl PackageIdentitySource for ReactorPackageIdentitySource {
    fn has_package_identity(&self) -> bool {
        SignatureProvider::has_package_identity(&self.provider)
    }
}

pub(crate) fn provider_display_name(provider: AIProvider) -> &'static str {
    match provider {
        AIProvider::None => "No provider",
        AIProvider::PhiSilica => "Phi Silica",
        AIProvider::FoundryLocal => "Foundry Local",
        AIProvider::Ollama => "Ollama",
        AIProvider::CustomOpenAI => "Custom endpoint",
        AIProvider::CodexCli => "Codex CLI",
        AIProvider::ClaudeCode => "Claude Code",
        AIProvider::OpenAI => "OpenAI",
        AIProvider::Anthropic => "Anthropic Claude",
        AIProvider::Gemini => "Google Gemini",
        AIProvider::DeepSeek => "DeepSeek",
    }
}

pub(crate) fn reactor_settings_service(validator: Arc<dyn SettingsValidator>) -> SettingsService {
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = crate::fixtures::knobs::settings_test_path() {
        return SettingsService::new(
            Arc::new(ShippingSettingsStorage::at_path(path.into())),
            Arc::new(WindowsDpapiCredentialStorage::new()),
            validator,
        );
    }
    windows_shipping_settings_service(validator)
}

#[cfg(feature = "settings-test-path")]
pub(crate) fn load_live_test_settings() -> Result<AppSettings, String> {
    let path = crate::fixtures::knobs::settings_test_path()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            format!(
                "{} is required for this validation fixture",
                crate::fixtures::knobs::settings_test_path_env_name()
            )
        })?;
    SettingsService::new(
        Arc::new(ShippingSettingsStorage::at_path(path.into())),
        Arc::new(WindowsDpapiCredentialStorage::new()),
        Arc::new(AllowAllSettings),
    )
    .load_nonsecret_settings()
    .map_err(|error| format!("could not load isolated validation settings: {error}"))
}

#[cfg(not(feature = "settings-test-path"))]
pub(crate) fn load_live_test_settings() -> Result<AppSettings, String> {
    Err("the settings-test-path validation feature is not enabled".to_string())
}

pub(crate) fn scan_kind_label(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Targeted Scan",
    }
}

pub(crate) fn action_run_status_text(summary: &ActionRunSummary) -> String {
    let total = summary.actions.len();
    let current = summary
        .current_index
        .and_then(|index| summary.actions.get(index))
        .map(|action| action.label.as_str());
    match summary.status {
        ActionRunStatus::Running => current.map_or_else(
            || "Running vetted remediation…".to_string(),
            |label| format!("Running remediation · {label}"),
        ),
        ActionRunStatus::CancelRequested => {
            "Stopping remediation after the current safe boundary…".to_string()
        }
        ActionRunStatus::Succeeded if action_run_schedules_restart(summary) => {
            "Restart scheduled · Windows will restart in 60 seconds; save your work now · run “shutdown /a” to cancel"
                .to_string()
        }
        ActionRunStatus::Succeeded if action_run_is_open_tool_handoff(summary) => {
            "Tool opened · complete the action in Windows, then re-run the relevant diagnostic"
                .to_string()
        }
        ActionRunStatus::Succeeded => format!(
            "Remediation complete · {total} action{} succeeded{}",
            if total == 1 { "" } else { "s" },
            if summary.requires_restart() {
                " · restart required"
            } else {
                ""
            }
        ),
        ActionRunStatus::Partial => format!(
            "Remediation finished with partial results · {total} action{} reviewed{}",
            if total == 1 { "" } else { "s" },
            if summary.requires_restart() {
                " · restart required"
            } else {
                ""
            }
        ),
        ActionRunStatus::Failed => "Remediation failed · review the step details".to_string(),
        ActionRunStatus::Cancelled => "Remediation cancelled".to_string(),
    }
}

pub(crate) fn action_proposal_schedules_restart(proposal: &ActionProposal) -> bool {
    proposal
        .actions
        .iter()
        .any(|action| action.remediation.id == "restart_system")
}

pub(crate) fn action_proposal_contains_repair(proposal: &ActionProposal) -> bool {
    proposal
        .actions
        .iter()
        .any(|action| action.remediation.tier == RemediationTier::Repair)
}

pub(crate) fn action_run_schedules_restart(summary: &ActionRunSummary) -> bool {
    summary
        .actions
        .iter()
        .any(|action| action.remediation_id == "restart_system")
}

pub(crate) fn action_run_is_open_tool_handoff(summary: &ActionRunSummary) -> bool {
    !summary.actions.is_empty()
        && summary.actions.iter().all(|action| {
            remediation::find(&action.remediation_id)
                .is_some_and(|spec| spec.tier == RemediationTier::OpenTool)
        })
}

pub(crate) const fn chat_completion_notice(finish_reason: &str) -> Option<&'static str> {
    match finish_reason.as_bytes() {
        b"length" => Some("Response stopped at the provider’s output limit."),
        b"tool_budget" => {
            Some("Tool budget reached; this answer uses the evidence already gathered.")
        }
        _ => None,
    }
}

pub(crate) fn settings_dialog_callback_is_current(
    settings_open: bool,
    current_epoch: u64,
    callback_epoch: u64,
) -> bool {
    settings_open && current_epoch == callback_epoch
}

pub(crate) fn about_dialog_callback_is_current(
    about_open: bool,
    current_epoch: u64,
    callback_epoch: u64,
) -> bool {
    about_open && current_epoch == callback_epoch
}

pub(crate) fn update_notice_remaining_after_elapsed(
    remaining: Duration,
    elapsed: Duration,
) -> Duration {
    remaining.saturating_sub(elapsed)
}

pub(crate) fn update_notice_timer_callback_is_current(
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

pub(crate) fn pending_system_info() -> SystemInfo {
    SystemInfo {
        computer_name: "This PC".to_string(),
        os_version: "Windows".to_string(),
        is_admin: false,
    }
}

pub(crate) fn privilege_label(is_admin: bool) -> &'static str {
    if is_admin {
        "Administrator"
    } else {
        "Standard user"
    }
}

pub(crate) fn machine_card_accessibility_name(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitoringLifecycleAction {
    None,
    Pause,
    ResumeAndRefresh,
}

pub(crate) fn window_is_usable(snapshot: window::WindowLifecycleSnapshot) -> bool {
    snapshot.registered && snapshot.visible && !snapshot.minimized && snapshot.focused
}

pub(crate) fn global_shortcut_is_allowed(
    event: window::GlobalShortcutEvent,
    blocking_overlay_open: bool,
    palette_open: bool,
    diagnostics_busy: bool,
) -> bool {
    if matches!(
        event.command,
        window::GlobalShortcutCommand::PalettePrevious
            | window::GlobalShortcutCommand::PaletteNext
            | window::GlobalShortcutCommand::PaletteExecute
            | window::GlobalShortcutCommand::PaletteClose
    ) {
        return palette_open;
    }
    if event.command == window::GlobalShortcutCommand::TogglePalette && palette_open {
        return true;
    }
    if blocking_overlay_open {
        return false;
    }
    if event.editable_focused && event.command != window::GlobalShortcutCommand::TogglePalette {
        return false;
    }
    !diagnostics_busy
        || !matches!(
            event.command,
            window::GlobalShortcutCommand::QuickScan | window::GlobalShortcutCommand::FullScan
        )
}

pub(crate) fn monitoring_lifecycle_action(
    snapshot: window::WindowLifecycleSnapshot,
    monitoring_paused: bool,
    paused_by_lifecycle: bool,
) -> MonitoringLifecycleAction {
    if window_is_usable(snapshot) {
        if monitoring_paused && paused_by_lifecycle {
            MonitoringLifecycleAction::ResumeAndRefresh
        } else {
            MonitoringLifecycleAction::None
        }
    } else if monitoring_paused {
        MonitoringLifecycleAction::None
    } else {
        MonitoringLifecycleAction::Pause
    }
}

pub(crate) fn history_tag_draft_for_selection(
    summaries: &[ScanSummary],
    selected_id: &str,
) -> String {
    summaries
        .iter()
        .find(|summary| summary.id == selected_id)
        .map(|summary| summary.tags.join(", "))
        .unwrap_or_default()
}

pub(crate) fn history_display_label(scan: &ScanSummary) -> &str {
    scan.label
        .as_deref()
        .or_else(|| scan.tags.first().map(String::as_str))
        .unwrap_or("Scan")
}

pub(crate) fn history_label_draft_for_selection(
    summaries: &[ScanSummary],
    selected_id: &str,
) -> String {
    summaries
        .iter()
        .find(|summary| summary.id == selected_id)
        .and_then(|summary| {
            summary
                .label
                .as_deref()
                .or_else(|| summary.tags.first().map(String::as_str))
        })
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn history_scan_matches_filter(scan: &ScanSummary, needle: &str) -> bool {
    needle.is_empty()
        || scan.id.to_ascii_lowercase().contains(needle)
        || scan.computer_name.to_ascii_lowercase().contains(needle)
        || scan
            .label
            .as_deref()
            .is_some_and(|label| label.to_ascii_lowercase().contains(needle))
        || scan
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(needle))
        || scan
            .timestamp
            .to_iso_string()
            .to_ascii_lowercase()
            .contains(needle)
}

pub(crate) fn history_change_rows(
    comparison: &ComparisonSummary,
) -> Vec<(HistoryChangeKind, &TaskChangeSummary)> {
    comparison
        .new_failures
        .iter()
        .map(|change| (HistoryChangeKind::Regressed, change))
        .chain(
            comparison
                .new_successes
                .iter()
                .map(|change| (HistoryChangeKind::Recovered, change)),
        )
        .chain(
            comparison
                .status_unchanged
                .iter()
                .filter(|change| change.output_changed)
                .map(|change| (HistoryChangeKind::Changed, change)),
        )
        .collect()
}

pub(crate) fn history_comparison_refresh_target(
    previous_latest_id: Option<&str>,
    summaries: &[ScanSummary],
    selected_id: Option<&str>,
) -> Option<String> {
    let latest_id = summaries.first().map(|summary| summary.id.as_str());
    if previous_latest_id == latest_id {
        return None;
    }
    selected_id
        .filter(|selected| summaries.iter().any(|scan| scan.id == *selected))
        .map(str::to_string)
}

pub(crate) fn history_trends_baseline_changed(
    trends_baseline_id: Option<&str>,
    current_baseline_id: Option<&str>,
) -> bool {
    trends_baseline_id != current_baseline_id
}

pub(crate) fn history_trend_badge(
    trends: Option<&[TaskTrend]>,
    task_id: &str,
) -> Option<HistoryTrendBadge> {
    let trend = trends?
        .iter()
        .find(|trend| trend.task_id == task_id && trend.failed >= 2)?;
    Some(HistoryTrendBadge {
        label: format!("{}/{} errors", trend.failed, trend.scans_considered),
        description: format!(
            "This diagnostic had a collection error in {} of the last {} scans",
            trend.failed, trend.scans_considered
        ),
    })
}

pub(crate) fn window_hook_retry_delay(failures: u8) -> Duration {
    let exponent = u32::from(failures.saturating_sub(1).min(5));
    WINDOW_HOOK_RETRY_MIN
        .saturating_mul(1_u32 << exponent)
        .min(WINDOW_HOOK_RETRY_MAX)
}

/// User-facing label for a report format, matching the save-dialog filter
/// names.
#[must_use]
pub(crate) const fn export_format_label(format: ReportFormat) -> &'static str {
    match format {
        ReportFormat::Json => "JSON",
        ReportFormat::Text => "TXT",
        ReportFormat::Html => "HTML",
    }
}

/// Resolve persisted export settings defensively at the action boundary.
///
/// The settings service normalizes legacy, empty, and unsupported values to
/// `text` while loading and saving. Keeping the same fallback here prevents a
/// stale in-memory snapshot from turning an otherwise valid export click into
/// the old "selected export format is not available" dead end.
pub(crate) fn resolved_export_format(value: &str) -> ReportFormat {
    ReportFormat::try_from(value).unwrap_or(ReportFormat::Text)
}

// `write_version_probe_if_requested` / `version_probe_document` moved to
// `fixtures::knobs` (#212): the probe reads the command line and an
// environment variable, so it now lives with the other knobs and is compiled
// out entirely without the `validation` feature.

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn the_page_host_owns_scrolling_for_every_page_but_wide_diagnostics() {
        // #193: the shell's page host is the ONE ScrollViewer, and it always
        // shows its bar. Wide Diagnostics lays itself out inside the viewport
        // and must not scroll; its compact layout must.
        assert!(page_host_scrolls(Page::Issues, false));
        assert!(page_host_scrolls(Page::Monitor, false));
        assert!(page_host_scrolls(Page::Processes, false));
        assert!(page_host_scrolls(Page::Ai, false));
        assert!(page_host_scrolls(Page::History, false));
        assert!(!page_host_scrolls(Page::Diagnostics, false));
        assert!(page_host_scrolls(Page::Diagnostics, true));
    }
    use crate::app::message::SettingsDialogAction;
    use crate::screens::history::view::history_comparison_placeholder;

    #[test]
    fn window_hook_retry_uses_bounded_exponential_backoff() {
        assert_eq!(window_hook_retry_delay(0), Duration::from_millis(100));
        assert_eq!(window_hook_retry_delay(1), Duration::from_millis(100));
        assert_eq!(window_hook_retry_delay(2), Duration::from_millis(200));
        assert_eq!(window_hook_retry_delay(6), Duration::from_millis(3_200));
        assert_eq!(
            window_hook_retry_delay(u8::MAX),
            Duration::from_millis(3_200)
        );
    }

    #[test]
    fn system_theme_follows_observed_window_color_scheme() {
        assert_eq!(
            effective_window_theme(WindowTheme::System, ColorScheme::Light),
            WindowTheme::Light
        );
        assert_eq!(
            effective_window_theme(WindowTheme::System, ColorScheme::Dark),
            WindowTheme::Dark
        );
        assert_eq!(
            effective_window_theme(WindowTheme::Light, ColorScheme::Dark),
            WindowTheme::Light
        );
        assert_eq!(
            effective_window_theme(WindowTheme::Dark, ColorScheme::Light),
            WindowTheme::Dark
        );
    }

    #[test]
    fn navigation_rail_matches_the_shipping_force_collapse_breakpoint() {
        assert!(!navigation_rail_forced_collapsed(1100.1));
        assert!(navigation_rail_forced_collapsed(1100.0));
        assert!(navigation_rail_forced_collapsed(720.0));
    }

    #[test]
    fn diagnostics_compact_layout_is_reserved_for_the_true_minimum_width() {
        assert!(!diagnostics_uses_compact_layout(900.0));
        assert!(!diagnostics_uses_compact_layout(840.0));
        assert!(diagnostics_uses_compact_layout(839.9));
        assert!(diagnostics_uses_compact_layout(720.0));
    }

    #[test]
    fn process_rows_match_the_available_table_width_in_every_shell_mode() {
        assert_eq!(process_layout_metrics(720.0, false), (true, 584.0));
        assert_eq!(process_layout_metrics(1_200.0, true), (true, 898.0));
        assert_eq!(process_layout_metrics(1_200.0, false), (false, 752.0));
        assert_eq!(process_layout_metrics(1_440.0, true), (false, 826.0));
    }

    #[test]
    fn ai_workspace_never_pushes_the_composer_below_the_minimum_window() {
        assert_eq!(ai_workspace_height(540.0), 297.0);
        assert_eq!(ai_workspace_height(800.0), 557.0);
        assert_eq!(ai_workspace_height(200.0), AI_WORKSPACE_MIN_HEIGHT);
    }

    #[test]
    fn global_shortcut_policy_matches_shipping_overlay_and_editable_guards() {
        use window::{GlobalShortcutCommand as Command, GlobalShortcutEvent as Event};

        let commands = [
            Command::TogglePalette,
            Command::Navigate(1),
            Command::ShowHelp,
            Command::QuickScan,
            Command::FullScan,
        ];
        for command in commands {
            assert!(global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: false,
                },
                false,
                false,
                false,
            ));
            assert!(!global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: false,
                },
                true,
                false,
                false,
            ));
        }

        assert!(global_shortcut_is_allowed(
            Event {
                command: Command::TogglePalette,
                editable_focused: true,
            },
            false,
            false,
            false,
        ));
        for command in [
            Command::Navigate(2),
            Command::ShowHelp,
            Command::QuickScan,
            Command::FullScan,
        ] {
            assert!(!global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: true,
                },
                false,
                false,
                false,
            ));
        }

        for command in [
            Command::PalettePrevious,
            Command::PaletteNext,
            Command::PaletteExecute,
            Command::PaletteClose,
        ] {
            assert!(!global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: true,
                },
                false,
                false,
                false,
            ));
            assert!(global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: true,
                },
                true,
                true,
                false,
            ));
        }
        assert!(global_shortcut_is_allowed(
            Event {
                command: Command::TogglePalette,
                editable_focused: true,
            },
            true,
            true,
            false,
        ));
    }

    #[test]
    fn active_scan_blocks_only_scan_shortcuts() {
        use window::{GlobalShortcutCommand as Command, GlobalShortcutEvent as Event};

        for command in [Command::QuickScan, Command::FullScan] {
            assert!(!global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: false,
                },
                false,
                false,
                true,
            ));
        }
        for command in [
            Command::TogglePalette,
            Command::Navigate(6),
            Command::ShowHelp,
        ] {
            assert!(global_shortcut_is_allowed(
                Event {
                    command,
                    editable_focused: false,
                },
                false,
                false,
                true,
            ));
        }
    }

    #[test]
    fn provider_catalog_requests_use_unsaved_credentials_without_logging_them() {
        let settings = AppSettings::default();
        let mut drafts: [String; ProviderKeyId::ALL.len()] = Default::default();
        assert!(provider_catalog_draft(6, &settings, &drafts).is_err());

        drafts[1] = "  test-anthropic-secret  ".to_string();
        let draft = provider_catalog_draft(6, &settings, &drafts)
            .expect("the draft should be usable")
            .expect("Anthropic has a catalog");
        assert_eq!(draft.api_key.as_deref(), Some("test-anthropic-secret"));
        assert!(!format!("{draft:?}").contains("test-anthropic-secret"));

        // Phi Silica has no catalog at all, which is not an error.
        assert_eq!(provider_catalog_draft(0, &settings, &drafts), Ok(None));
    }

    #[test]
    fn provider_setup_helpers_update_only_the_selected_setting() {
        let mut settings = AppSettings::default();
        set_provider_setup_model(6, &mut settings, Some("claude-sonnet-5".to_string()));
        assert_eq!(settings.anthropic_model.as_deref(), Some("claude-sonnet-5"));
        assert!(settings.open_ai_model.is_none());

        set_provider_key_configured(&mut settings, ProviderKeyId::Anthropic, true);
        assert!(settings.anthropic_api_key_set);
        assert!(!settings.open_ai_api_key_set);
        set_provider_key_configured(&mut settings, ProviderKeyId::Anthropic, false);
        assert!(!settings.anthropic_api_key_set);
    }

    // `version_probe_document_uses_the_canonical_build_version` moved with the
    // probe itself to `fixtures::knobs` (#212).

    pub(crate) fn provider_status(active_provider: AIProvider) -> AIProviderStatus {
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
    fn phi_preference_gate_blocks_unknown_and_unready_status_but_never_other_providers() {
        let checking = phi_preference_gate(None, true);
        assert_eq!(checking, PhiPreferenceGate::Checking);
        assert!(validate_phi_preference("phi_silica", &checking).is_err());
        assert!(validate_phi_preference("codex_cli", &checking).is_ok());

        let mut unavailable = provider_status(AIProvider::None);
        unavailable.phi_silica_message = Some("model is still preparing".to_string());
        let blocked = phi_preference_gate(Some(&unavailable), false);
        assert_eq!(
            blocked,
            PhiPreferenceGate::Blocked("model is still preparing".to_string())
        );
        assert_eq!(
            validate_phi_preference("phi_silica", &blocked),
            Err("model is still preparing".to_string())
        );

        let ready = provider_status(AIProvider::PhiSilica);
        let ready_gate = phi_preference_gate(Some(&ready), false);
        assert_eq!(ready_gate, PhiPreferenceGate::Ready);
        assert!(validate_phi_preference("phi_silica", &ready_gate).is_ok());
    }

    #[test]
    fn configured_provider_setup_seeding_matches_shipping_precedence() {
        let mut settings = AppSettings::default();
        assert_eq!(configured_provider_setup_index(&settings), 5);

        settings.anthropic_api_key_set = true;
        assert_eq!(configured_provider_setup_index(&settings), 6);

        settings.custom_endpoint = Some("https://example.invalid/v1".to_string());
        assert_eq!(configured_provider_setup_index(&settings), 9);

        settings.preferred_ai_provider = "deepseek".to_string();
        assert_eq!(configured_provider_setup_index(&settings), 8);
    }

    #[test]
    fn every_explicit_provider_maps_to_its_setup_pane() {
        for (index, provider) in PROVIDER_SETUP_PROVIDERS.into_iter().enumerate() {
            assert_eq!(provider_setup_index_for_provider(provider), Some(index));
        }
        assert_eq!(provider_setup_index_for_provider(AIProvider::None), None);
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
                // Same identity as the Store 2.5.8 fixture
                // (`fixtures::visual::fixture_258_system_info`); pinned inline
                // so this accessibility contract holds in every feature shape.
                &wfdiag_native_system::SystemInfo {
                    computer_name: "ANDROMEDA".to_string(),
                    os_version: "Windows 11 Professional (25H2)".to_string(),
                    is_admin: false,
                },
                Some(&architecture),
                Some("architecture probe degraded"),
            ),
            "Computer ANDROMEDA, Windows 11 Professional (25H2), Standard user, x64 app running on ARM64 hardware, system information warning: architecture probe degraded"
        );
        assert_eq!(privilege_label(true), "Administrator");
    }

    #[test]
    fn export_action_falls_back_to_text_for_stale_or_unsupported_settings() {
        assert_eq!(resolved_export_format("text"), ReportFormat::Text);
        assert_eq!(resolved_export_format("json"), ReportFormat::Json);
        assert_eq!(resolved_export_format("html"), ReportFormat::Html);
        assert_eq!(resolved_export_format(""), ReportFormat::Text);
        assert_eq!(resolved_export_format("pdf"), ReportFormat::Text);
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

    #[test]
    fn window_lifecycle_pauses_only_when_unusable_and_resumes_only_its_own_pause() {
        let usable = window::WindowLifecycleSnapshot {
            registered: true,
            visible: true,
            focused: true,
            ..Default::default()
        };
        assert_eq!(
            monitoring_lifecycle_action(usable, false, false),
            MonitoringLifecycleAction::None
        );

        for unusable in [
            window::WindowLifecycleSnapshot {
                visible: false,
                ..usable
            },
            window::WindowLifecycleSnapshot {
                minimized: true,
                ..usable
            },
            window::WindowLifecycleSnapshot {
                focused: false,
                ..usable
            },
        ] {
            assert_eq!(
                monitoring_lifecycle_action(unusable, false, false),
                MonitoringLifecycleAction::Pause
            );
        }

        assert_eq!(
            monitoring_lifecycle_action(usable, true, true),
            MonitoringLifecycleAction::ResumeAndRefresh
        );
        assert_eq!(
            monitoring_lifecycle_action(usable, true, false),
            MonitoringLifecycleAction::None
        );
    }

    #[test]
    fn selecting_history_reseeds_the_editable_tag_draft() {
        let summary = ScanSummary {
            id: "scan-2".to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs: 0 },
            computer_name: "TEST-PC".to_string(),
            task_count: 2,
            success_count: 2,
            failure_count: 0,
            duration_ms: 42,
            label: None,
            tags: vec!["Quick Scan".to_string(), "before-update".to_string()],
        };

        assert_eq!(
            history_tag_draft_for_selection(&[summary], "scan-2"),
            "Quick Scan, before-update"
        );
        assert_eq!(history_tag_draft_for_selection(&[], "missing"), "");
    }

    #[test]
    fn history_label_display_draft_and_filter_use_the_first_tag_fallback() {
        let mut summary = ScanSummary {
            id: "scan-2".to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs: 0 },
            computer_name: "TEST-PC".to_string(),
            task_count: 2,
            success_count: 2,
            failure_count: 0,
            duration_ms: 42,
            label: None,
            tags: vec!["Quick Scan".to_string(), "before-update".to_string()],
        };

        assert_eq!(history_display_label(&summary), "Quick Scan");
        assert_eq!(
            history_label_draft_for_selection(&[summary.clone()], "scan-2"),
            "Quick Scan"
        );
        assert!(history_scan_matches_filter(&summary, "before-update"));
        assert!(history_scan_matches_filter(&summary, "test-pc"));
        assert!(!history_scan_matches_filter(&summary, "after-update"));

        summary.label = Some("Baseline".to_string());
        assert_eq!(history_display_label(&summary), "Baseline");
        assert!(history_scan_matches_filter(&summary, "baseline"));
        assert!(history_scan_matches_filter(&summary, "quick scan"));

        summary.label = None;
        summary.tags.clear();
        assert_eq!(history_display_label(&summary), "Scan");
        assert_eq!(history_label_draft_for_selection(&[summary], "scan-2"), "");
    }

    #[test]
    fn history_comparison_rows_include_regressions_recoveries_and_output_changes() {
        let summary = |id: &str| ScanSummary {
            id: id.to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs: 0 },
            computer_name: "TEST-PC".to_string(),
            task_count: 3,
            success_count: 2,
            failure_count: 1,
            duration_ms: 42,
            label: None,
            tags: Vec::new(),
        };
        let change = |task_id: &str, output_changed: bool| TaskChangeSummary {
            task_id: task_id.to_string(),
            task_name: task_id.to_string(),
            category: "System".to_string(),
            current_success: true,
            previous_success: true,
            output_changed,
        };
        let comparison = ComparisonSummary {
            current_scan: summary("latest"),
            previous_scan: summary("older"),
            total_changes: 3,
            new_failures: vec![change("regressed", true)],
            new_successes: vec![change("recovered", true)],
            status_unchanged: vec![change("unchanged", false), change("changed", true)],
        };

        assert_eq!(
            history_change_rows(&comparison)
                .into_iter()
                .map(|(kind, change)| (kind, change.task_id.as_str()))
                .collect::<Vec<_>>(),
            [
                (HistoryChangeKind::Regressed, "regressed"),
                (HistoryChangeKind::Recovered, "recovered"),
                (HistoryChangeKind::Changed, "changed"),
            ]
        );
    }

    #[test]
    fn history_refresh_rebases_only_a_preserved_selection_when_latest_changes() {
        let summary = |id: &str| ScanSummary {
            id: id.to_string(),
            timestamp: wfdiag_native_history::Timestamp { secs: 0 },
            computer_name: "TEST-PC".to_string(),
            task_count: 1,
            success_count: 1,
            failure_count: 0,
            duration_ms: 1,
            label: None,
            tags: Vec::new(),
        };
        let scans = vec![summary("new-latest"), summary("selected")];

        assert_eq!(
            history_comparison_refresh_target(Some("old-latest"), &scans, Some("selected")),
            Some("selected".to_string())
        );
        assert_eq!(
            history_comparison_refresh_target(Some("new-latest"), &scans, Some("selected")),
            None
        );
        assert_eq!(
            history_comparison_refresh_target(Some("old-latest"), &scans, Some("missing")),
            None
        );
    }

    #[test]
    fn history_trend_refresh_tracks_the_current_baseline_identity() {
        assert!(!history_trends_baseline_changed(None, None));
        assert!(history_trends_baseline_changed(None, Some("latest")));
        assert!(!history_trends_baseline_changed(
            Some("latest"),
            Some("latest")
        ));
        assert!(history_trends_baseline_changed(
            Some("old-latest"),
            Some("new-latest")
        ));
        assert!(history_trends_baseline_changed(Some("latest"), None));
    }

    #[test]
    fn history_trend_badge_matches_react_threshold_and_task_lookup() {
        let one_failure = [TaskTrend {
            task_id: "os_info".to_string(),
            failed: 1,
            seen_in: 10,
            scans_considered: 10,
        }];
        assert_eq!(history_trend_badge(Some(&one_failure), "os_info"), None);

        let recurring = [
            TaskTrend {
                task_id: "other".to_string(),
                failed: 9,
                seen_in: 10,
                scans_considered: 10,
            },
            TaskTrend {
                task_id: "os_info".to_string(),
                failed: 2,
                seen_in: 7,
                scans_considered: 10,
            },
        ];
        assert_eq!(
            history_trend_badge(Some(&recurring), "os_info"),
            Some(HistoryTrendBadge {
                label: "2/10 errors".to_string(),
                description: "This diagnostic had a collection error in 2 of the last 10 scans"
                    .to_string(),
            })
        );
        assert_eq!(history_trend_badge(Some(&recurring), "missing"), None);
        assert_eq!(history_trend_badge(None, "os_info"), None);
    }

    #[test]
    fn scan_status_text_distinguishes_running_stopping_and_complete() {
        assert_eq!(
            scan_progress_text("Quick Scan", false, 3, 17, "TPM"),
            "Quick Scan · 3 of 17 collected · TPM"
        );
        assert_eq!(
            scan_progress_text("Quick Scan", true, 3, 17, "TPM"),
            "Stopping Quick Scan · TPM"
        );
        assert_eq!(
            scan_result_text("Full Scan", true, 4, 17, 1),
            "Full Scan · 4 of 17 collected · 1 errors"
        );
        assert_eq!(
            scan_result_text("Full Scan", false, 17, 17, 1),
            "Full Scan complete · 17 collected · 1 errors"
        );
    }

    #[test]
    fn a_failed_history_save_never_hides_a_completed_scan() {
        assert_eq!(
            scan_complete_text("Quick Scan", 17, 0, false),
            "Quick Scan complete · 17 collected · 0 errors"
        );
        let failed = scan_complete_text("Quick Scan", 17, 0, true);
        assert!(failed.starts_with("Quick Scan complete · 17 collected · 0 errors"));
        assert!(failed.ends_with("· history not saved"));
    }

    #[test]
    fn provider_wire_ids_round_trip_and_unknown_ids_are_none() {
        for provider in [
            AIProvider::OpenAI,
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal,
            AIProvider::Ollama,
            AIProvider::CustomOpenAI,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
            AIProvider::Anthropic,
            AIProvider::Gemini,
            AIProvider::DeepSeek,
        ] {
            assert_eq!(provider_from_wire(&provider.to_string()), provider);
        }
        assert_eq!(provider_from_wire("open_a_i"), AIProvider::None);
        assert_eq!(provider_from_wire(""), AIProvider::None);

        assert_eq!(
            subscription_provider_from_wire(&AIProvider::CodexCli.to_string()),
            Some(SubscriptionAuthProvider::Codex)
        );
        assert_eq!(
            subscription_provider_from_wire(&AIProvider::ClaudeCode.to_string()),
            Some(SubscriptionAuthProvider::ClaudeCode)
        );
        assert_eq!(
            subscription_provider_from_wire(&AIProvider::OpenAI.to_string()),
            None
        );
    }

    #[test]
    fn a_refusal_shows_the_engines_own_wording_not_the_enum_name() {
        use wfdiag_app::{RejectReason, WorkerKind};
        assert_eq!(
            rejection_text(&RejectReason::NotReady {
                detail: "Enable AI insights in Settings before sending".to_string(),
            }),
            "Enable AI insights in Settings before sending"
        );
        assert_eq!(
            rejection_text(&RejectReason::Busy {
                detail: "a chat turn is already streaming".to_string(),
            }),
            "a chat turn is already streaming"
        );
        assert_eq!(
            rejection_text(&RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: "native history is unavailable".to_string(),
            }),
            "native history is unavailable"
        );
        // A structural refusal has no user-facing detail of its own, so the
        // enum's prose is the message.
        assert_eq!(
            rejection_text(&RejectReason::IdentityExhausted),
            RejectReason::IdentityExhausted.to_string()
        );
        assert_eq!(
            worker_timeout_text("history"),
            "The history worker did not answer in time · try again"
        );
    }
}
