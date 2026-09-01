//! Pure decision helpers lifted out of the component.
//!
//! Nothing here touches component state, Windows, or a worker: every function
//! is a total mapping from its arguments, which is what keeps them testable.

#![deny(unsafe_code)]

use crate::app::consts::{
    AI_WORKSPACE_MIN_HEIGHT, AI_WORKSPACE_VERTICAL_CHROME, CODEX_MODEL_IDS,
    DIAGNOSTICS_COMPACT_BREAKPOINT, PROCESS_DETAILS_COLUMN_WIDTH, PROCESS_WIDE_CONTENT_MIN_WIDTH,
    PROVIDER_SETUP_PROVIDERS, QUICK_DETECTION_SOURCE_TASK_IDS, QUICK_SCAN_TASK_IDS,
    SHELL_CONTENT_HORIZONTAL_CHROME, WINDOW_HOOK_RETRY_MAX, WINDOW_HOOK_RETRY_MIN,
};
use crate::app::message::HistoryChangeKind;
use crate::app::state::{
    DiagnosticScanPolicy, ExportWriteKind, HistoryRetentionPolicy, HistoryTrendBadge,
    PendingAiProviderGate, PendingExport, PendingExportAction, PendingSettingsSave,
};
use crate::platform::save_picker::{SavePickerError, SavePickerOutcome, ValidatedExportPath};
use crate::platform::window;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallProgress, SubscriptionInstallStage,
};
use wfdiag_native_ai_chat::{SubscriptionAuthOperation, SubscriptionAuthProvider};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, AIProviderStatus, FoundryCliEndpointSource,
    ModelCatalogRequest, NativeAiProviderRuntime, PackageIdentitySource,
    ProcessSubscriptionCliStatusSource, ProviderManagementService, ProviderModelDefaults,
    ProviderProbeBundle, ProviderSelectionState, SettingsServiceProviderConfigurationSource,
    SharedAiCache, parse_provider_preference, provider_preference_for_runtime,
};
use wfdiag_native_diagnostics::{DiagnosticTask, ScanEvidence, ScanKind, SharedScanEvidence};
use wfdiag_native_export::{ExportTask, ReportFormat};
use wfdiag_native_history::{
    ComparisonSummary, DiagnosticTask as HistoryDiagnosticTask, ScanRecord, ScanSummary,
    TaskChangeSummary, TaskTrend, Timestamp,
};
use wfdiag_native_issues::{Issue, RemediationTier};
use wfdiag_native_phi::WindowsPhiStatusSource;
use wfdiag_native_remediation::broker::{ActionProposal, ActionSnapshot};
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
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo, SystemRequestKind};
use wfdiag_native_update::policy::UpdateThrottle;
use wfdiag_native_update::{SignatureProvider, WindowsPackageSignatureProvider};
use wfdiag_ui_core::DiagnosticTaskResult;
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

pub(crate) const fn settings_ai_status_probe_needed(
    settings_open: bool,
    status_known: bool,
    status_loading: bool,
) -> bool {
    settings_open && !status_known && !status_loading
}

pub(crate) fn provider_models_auto_discovery_allowed(index: usize) -> bool {
    provider_setup_provider(index)
        .is_some_and(|provider| !matches!(provider, AIProvider::PhiSilica | AIProvider::ClaudeCode))
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

pub(crate) fn subscription_auth_completion_refreshes_models(
    operation: SubscriptionAuthOperation,
) -> bool {
    operation != SubscriptionAuthOperation::Status
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

pub(crate) fn provider_catalog_request_for_draft(
    setup_index: usize,
    settings: &AppSettings,
    provider_key_drafts: &[String; ProviderKeyId::ALL.len()],
) -> Result<Option<ModelCatalogRequest>, String> {
    let Some(provider) = provider_setup_provider(setup_index) else {
        return Err("The selected provider is not recognized".to_string());
    };
    if provider == AIProvider::PhiSilica {
        return Ok(None);
    }
    let (draft_api_key, key_configured) = match provider {
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
    ) && draft_api_key.is_none()
        && !key_configured
    {
        return Err("Enter an API key to load the available models.".to_string());
    }

    let draft_endpoint = match provider {
        AIProvider::FoundryLocal => settings.local_ai_endpoint.clone(),
        AIProvider::Ollama => settings.ollama_endpoint.clone(),
        AIProvider::CustomOpenAI => settings.custom_endpoint.clone(),
        _ => None,
    };
    if provider == AIProvider::CustomOpenAI
        && draft_endpoint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return Err("Enter an endpoint URL to load the available models.".to_string());
    }
    let draft_cli_path = match provider {
        AIProvider::CodexCli => settings.codex_cli_path.clone(),
        AIProvider::ClaudeCode => settings.claude_cli_path.clone(),
        _ => None,
    };
    Ok(Some(ModelCatalogRequest {
        provider,
        draft_api_key,
        draft_endpoint,
        draft_cli_path,
    }))
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

pub(crate) fn provider_preference_id(preference: AIProviderPreference) -> &'static str {
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

pub(crate) fn normalize_provider_preference_for_runtime(settings: &mut AppSettings) {
    let identity = ReactorPackageIdentitySource::default();
    let preference = provider_preference_for_runtime(
        &settings.preferred_ai_provider,
        identity.has_package_identity(),
    );
    settings.preferred_ai_provider = provider_preference_id(preference).to_string();
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

pub(crate) fn reactor_ai_provider_runtime(
    settings: SettingsService,
    identity: Arc<dyn PackageIdentitySource>,
    cache: SharedAiCache,
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
        Arc::new(cache),
        ProviderModelDefaults {
            foundry: "phi-4-mini".to_string(),
            openai: "gpt-5-nano".to_string(),
            anthropic: "claude-sonnet-5".to_string(),
            gemini: "gemini-3.6-flash".to_string(),
            deepseek: "deepseek-v4-flash".to_string(),
        },
    );
    NativeAiProviderRuntime::start(Arc::new(service)).map_err(|error| error.to_string())
}

pub(crate) fn reactor_update_throttle() -> Option<UpdateThrottle> {
    #[cfg(feature = "settings-test-path")]
    if let Some(path) = crate::fixtures::knobs::settings_test_path() {
        let path = std::path::PathBuf::from(path);
        return Some(UpdateThrottle::beside_settings_file(&path));
    }
    UpdateThrottle::shipping().ok()
}

pub(crate) fn scan_kind_label(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Targeted Scan",
    }
}

pub(crate) fn scan_kind_history_tag(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Manual Diagnostic",
    }
}

pub(crate) fn requires_scan_data(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.contains("in general")
        || normalized.contains("generally")
        || normalized.starts_with("define ")
        || normalized == "what is windows?"
        || normalized == "what is windows"
        || (normalized.starts_with("what does ") && normalized.contains(" mean"))
    {
        return false;
    }
    let casual = normalized.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '.' | '!' | '?' | ',')
    });
    if matches!(
        casual,
        "hi" | "hello"
            | "hey"
            | "thanks"
            | "thank you"
            | "good morning"
            | "good afternoon"
            | "good evening"
            | "who are you"
            | "what can you do"
            | "tell me a joke"
    ) || casual.starts_with("write me a ")
        || casual.starts_with("write a ")
    {
        return false;
    }
    true
}

pub(crate) fn scan_concurrency_from_settings(max_concurrent_tasks: u32) -> usize {
    if max_concurrent_tasks == 0 {
        5
    } else {
        usize::try_from(max_concurrent_tasks).unwrap_or(5)
    }
}

pub(crate) fn select_scan_tasks(
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

pub(crate) fn issue_prioritization_payload(issues: &[Issue]) -> Result<String, String> {
    let rows = issues
        .iter()
        .filter(|issue| {
            issue.detected && issue.status == wfdiag_native_issues::IssueStatus::Detected
        })
        .map(|issue| {
            serde_json::json!({
                "id": issue.id,
                "severity": issue.severity,
                "title": issue.title,
                "description": issue.description,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&rows)
        .map_err(|error| format!("could not serialize detected issues: {error}"))
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

pub(crate) fn action_proposal_matches_snapshot(
    proposal: &ActionProposal,
    snapshot: &ActionSnapshot,
) -> bool {
    proposal.scan_fingerprint == snapshot.scan_fingerprint
        && proposal.catalog_fingerprint == snapshot.catalog_fingerprint
        && proposal.actions.iter().all(|action| {
            action
                .issue_id
                .as_deref()
                .map_or(action.remediation.maintenance, |issue_id| {
                    snapshot.detected_issues.iter().any(|issue| {
                        issue.issue_id == issue_id
                            && issue.remediation_id.as_deref()
                                == Some(action.remediation.id.as_str())
                    })
                })
        })
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

pub(crate) fn history_task_catalog(catalog: &[DiagnosticTask]) -> Vec<HistoryDiagnosticTask> {
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

pub(crate) fn export_task_catalog(catalog: &[DiagnosticTask]) -> Vec<ExportTask> {
    catalog
        .iter()
        .map(|task| ExportTask::new(&task.id, &task.name, &task.category))
        .collect()
}

pub(crate) fn system_identity_blocks_scan(
    deterministic_visual: bool,
    system_info_request_id: Option<u64>,
) -> bool {
    !deterministic_visual && system_info_request_id.is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupScanGate {
    AwaitingSettings,
    Armed,
    Consumed,
}

pub(crate) fn apply_startup_scan_preference(gate: &mut StartupScanGate, scan_on_startup: bool) {
    if *gate == StartupScanGate::AwaitingSettings {
        *gate = if scan_on_startup {
            StartupScanGate::Armed
        } else {
            StartupScanGate::Consumed
        };
    }
}

pub(crate) fn take_startup_scan_when_ready(
    gate: &mut StartupScanGate,
    deterministic_visual: bool,
    settings_loading: bool,
    system_info_request_id: Option<u64>,
    architecture_request_id: Option<u64>,
) -> bool {
    if deterministic_visual {
        *gate = StartupScanGate::Consumed;
        return false;
    }
    if *gate != StartupScanGate::Armed
        || settings_loading
        || system_info_request_id.is_some()
        || architecture_request_id.is_some()
    {
        return false;
    }

    // Consume before dispatch so every later completion/retry is a no-op even
    // when the diagnostic runtime rejects the startup scan.
    *gate = StartupScanGate::Consumed;
    true
}

/// The scan executor's process-wide runtime. Building a 5-worker multi-thread
/// runtime per scan start/run/cancel created and destroyed ~10 threads per
/// scan and blocked on the shutdown barrier while the user watched "Stop
/// scan". `enable_all()` also arms the IO/time drivers any diagnostic task
/// that uses tokio networking or timers needs (without them such use panics
/// inside the executor and surfaces as a misleading queue rejection).
pub(crate) fn shared_diagnostic_executor() -> Result<&'static tokio::runtime::Runtime, String> {
    static EXECUTOR: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(5)
                .thread_name("wfdiag-diagnostic")
                .enable_all()
                .build()
                .map_err(|error| format!("could not create the diagnostic worker pool: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn next_process_request_id(current: u64) -> u64 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

pub(crate) fn pending_ai_provider_gate(
    ai_enabled: bool,
    provider_loading: bool,
    provider_status: Option<&AIProviderStatus>,
) -> PendingAiProviderGate {
    if !ai_enabled {
        PendingAiProviderGate::Disabled
    } else if provider_loading {
        PendingAiProviderGate::Waiting
    } else {
        match provider_status {
            Some(status) if status.active_provider != AIProvider::None => {
                PendingAiProviderGate::Ready
            }
            Some(_) => PendingAiProviderGate::Unavailable,
            None => PendingAiProviderGate::Refresh,
        }
    }
}

pub(crate) fn pending_export_write_is_current(
    pending: Option<&PendingExport>,
    request_id: u64,
    kind: ExportWriteKind,
) -> bool {
    pending.is_some_and(|pending| {
        pending.request_id == request_id
            && matches!(
                (&pending.action, kind),
                (
                    PendingExportAction::SaveToFile { .. },
                    ExportWriteKind::File
                ) | (
                    PendingExportAction::SupportPackage { .. },
                    ExportWriteKind::SupportPackage
                )
            )
    })
}

pub(crate) fn scan_policy_requests_auto_save(policy: Option<&DiagnosticScanPolicy>) -> bool {
    policy.is_some_and(|policy| policy.auto_save)
}

pub(crate) fn history_retention_tuple(policy: &RwLock<HistoryRetentionPolicy>) -> (bool, u32) {
    policy.read().map_or((true, 30), |policy| {
        (policy.retain_history, policy.history_limit)
    })
}

pub(crate) fn update_history_retention_policy(
    policy: &RwLock<HistoryRetentionPolicy>,
    settings: &AppSettings,
) {
    if let Ok(mut policy) = policy.write() {
        *policy = HistoryRetentionPolicy::from(settings);
    }
}

pub(crate) fn authoritative_ui_results(
    session_id: &str,
    results: &ScanEvidence,
    catalog: &[DiagnosticTask],
) -> Vec<DiagnosticTaskResult> {
    let mut results = results
        .iter()
        .map(|(task_id, result)| DiagnosticTaskResult::new(session_id, task_id, Arc::clone(result)))
        .collect::<Vec<_>>();
    results.sort_by_key(|result| {
        catalog
            .iter()
            .position(|task| task.id == result.task_id)
            .unwrap_or(usize::MAX)
    });
    results
}

pub(crate) fn merge_targeted_diagnostic_result(
    mut prior: Vec<DiagnosticTaskResult>,
    target_task_id: &str,
    mut replacement: DiagnosticTaskResult,
    base_session_id: Option<&str>,
    catalog: &[DiagnosticTask],
) -> Result<Vec<DiagnosticTaskResult>, String> {
    if replacement.task_id != target_task_id {
        return Err(format!(
            "targeted rerun returned `{}` instead of `{target_task_id}`",
            replacement.task_id
        ));
    }
    let replacement_index = prior
        .iter()
        .position(|result| result.task_id == target_task_id);

    if let Some(session_id) = base_session_id {
        replacement.session_id = session_id.to_string();
    }
    let prior_order = prior
        .iter()
        .enumerate()
        .map(|(index, result)| (result.task_id.clone(), index))
        .collect::<HashMap<_, _>>();
    if let Some(replacement_index) = replacement_index {
        prior[replacement_index] = replacement;
    } else {
        prior.push(replacement);
    }
    prior.sort_by_key(|result| {
        catalog
            .iter()
            .position(|task| task.id == result.task_id)
            .map_or_else(
                || {
                    (
                        1,
                        prior_order
                            .get(&result.task_id)
                            .copied()
                            .unwrap_or(usize::MAX),
                    )
                },
                |index| (0, index),
            )
    });
    Ok(prior)
}

pub(crate) fn diagnostic_output_snapshot(results: &[DiagnosticTaskResult]) -> SharedScanEvidence {
    Arc::new(
        results
            .iter()
            .map(|result| (result.task_id.clone(), Arc::clone(&result.result)))
            .collect(),
    )
}

pub(crate) fn authoritative_result_set_is_complete(
    results: &ScanEvidence,
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

pub(crate) fn build_history_scan_record(
    session_id: String,
    system_info: &SystemInfo,
    results: &[DiagnosticTaskResult],
    duration_ms: u64,
    history_tag: String,
) -> ScanRecord {
    let results = results
        .iter()
        .map(|result| (result.task_id.clone(), Arc::clone(&result.result)))
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

pub(crate) fn take_matching_pending_settings_save(
    pending: &mut Option<PendingSettingsSave>,
    request_id: u64,
) -> Option<PendingSettingsSave> {
    pending.take_if(|candidate| candidate.request_id == request_id)
}

pub(crate) fn take_matching_system_request(
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiWorkerKind {
    Analysis,
    Chat,
    FixPlan,
    Report,
}

impl AiWorkerKind {
    pub(crate) const fn display_name(self) -> &'static str {
        match self {
            Self::Analysis => "one-shot diagnostic AI",
            Self::Chat => "AI chat",
            Self::FixPlan => "AI fix planning",
            Self::Report => "AI report generation",
        }
    }
}

/// Interpret the explicit worker-isolation switch used by the native UI
/// harness. An absent/empty/unknown policy is the production path and keeps
/// every AI worker enabled.
pub(crate) fn ai_worker_enabled(policy: &std::ffi::OsStr, worker: AiWorkerKind) -> bool {
    match policy.to_str().unwrap_or_default() {
        "analysis" => worker != AiWorkerKind::Analysis,
        "chat" => worker != AiWorkerKind::Chat,
        "fix-plan" => worker != AiWorkerKind::FixPlan,
        "report" => worker != AiWorkerKind::Report,
        "only-analysis" => worker == AiWorkerKind::Analysis,
        "only-chat" => worker == AiWorkerKind::Chat,
        "only-fix-plan" => worker == AiWorkerKind::FixPlan,
        "only-report" => worker == AiWorkerKind::Report,
        "only-action" | "only-instance" | "none" => false,
        _ => true,
    }
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

pub(crate) fn history_task_diff_result_is_current(
    request_id: u64,
    current_request_id: u64,
    task_id: &str,
    expanded_task_id: Option<&str>,
) -> bool {
    request_id == current_request_id && expanded_task_id == Some(task_id)
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

pub(crate) fn resolve_export_picker_selection(
    result: Result<SavePickerOutcome, SavePickerError>,
) -> Result<Option<ValidatedExportPath>, String> {
    match result {
        Ok(SavePickerOutcome::Cancelled) => Ok(None),
        Ok(SavePickerOutcome::Selected(path)) => Ok(Some(path)),
        Err(error) => Err(error.to_string()),
    }
}

// `write_version_probe_if_requested` / `version_probe_document` moved to
// `fixtures::knobs` (#212): the probe reads the command line and an
// environment variable, so it now lives with the other knobs and is compiled
// out entirely without the `validation` feature.

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::message::SettingsDialogAction;
    use crate::app::state::{DiagnosticSnapshot, TargetedDiagnosticOverlay};
    use crate::fixtures::visual::fixture_258_system_info;
    use crate::platform::save_picker::ValidatedSupportPackagePaths;
    use crate::screens::history::view::history_comparison_placeholder;
    use wfdiag_native_diagnostics::DiagnosticOutput;

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
    fn process_request_ids_advance_when_started_or_invalidated() {
        assert_eq!(next_process_request_id(0), 1);
        assert_eq!(next_process_request_id(41), 42);
        assert_eq!(next_process_request_id(u64::MAX), 1);
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
    fn production_worker_policy_enables_every_lazy_ai_worker() {
        let default_policy = std::ffi::OsStr::new("");
        for worker in [
            AiWorkerKind::Analysis,
            AiWorkerKind::Chat,
            AiWorkerKind::FixPlan,
            AiWorkerKind::Report,
        ] {
            assert!(ai_worker_enabled(default_policy, worker));
        }

        let chat_only = std::ffi::OsStr::new("only-chat");
        assert!(ai_worker_enabled(chat_only, AiWorkerKind::Chat));
        assert!(!ai_worker_enabled(chat_only, AiWorkerKind::Report));
        assert!(!ai_worker_enabled(chat_only, AiWorkerKind::Analysis));
        assert!(!ai_worker_enabled(chat_only, AiWorkerKind::FixPlan));

        let skip_chat = std::ffi::OsStr::new("chat");
        assert!(!ai_worker_enabled(skip_chat, AiWorkerKind::Chat));
        assert!(ai_worker_enabled(skip_chat, AiWorkerKind::Report));

        let no_ai_workers = std::ffi::OsStr::new("none");
        for worker in [
            AiWorkerKind::Analysis,
            AiWorkerKind::Chat,
            AiWorkerKind::FixPlan,
            AiWorkerKind::Report,
        ] {
            assert!(!ai_worker_enabled(no_ai_workers, worker));
        }
    }

    #[test]
    fn automatic_model_discovery_never_bootstraps_the_claude_adapter() {
        assert!(provider_models_auto_discovery_allowed(3));
        assert!(!provider_models_auto_discovery_allowed(4));
        assert!(!provider_models_auto_discovery_allowed(0));
        assert!(!provider_models_auto_discovery_allowed(usize::MAX));
    }

    #[test]
    fn account_status_checks_do_not_create_a_model_refresh_loop() {
        assert!(!subscription_auth_completion_refreshes_models(
            SubscriptionAuthOperation::Status
        ));
        assert!(subscription_auth_completion_refreshes_models(
            SubscriptionAuthOperation::SignIn
        ));
        assert!(subscription_auth_completion_refreshes_models(
            SubscriptionAuthOperation::SignOut
        ));
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
        assert!(provider_catalog_request_for_draft(6, &settings, &drafts).is_err());

        drafts[1] = "  test-anthropic-secret  ".to_string();
        let request = provider_catalog_request_for_draft(6, &settings, &drafts)
            .expect("request should be valid")
            .expect("Anthropic has a catalog");
        assert_eq!(request.provider, AIProvider::Anthropic);
        assert_eq!(
            request.draft_api_key.as_deref(),
            Some("test-anthropic-secret")
        );
        assert!(!format!("{request:?}").contains("test-anthropic-secret"));
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
    fn pending_ai_provider_gate_preserves_intent_until_readiness_is_terminal() {
        let ready = provider_status(AIProvider::OpenAI);
        let unavailable = provider_status(AIProvider::None);

        assert_eq!(
            pending_ai_provider_gate(false, false, Some(&ready)),
            PendingAiProviderGate::Disabled
        );
        assert_eq!(
            pending_ai_provider_gate(true, true, None),
            PendingAiProviderGate::Waiting
        );
        assert_eq!(
            pending_ai_provider_gate(true, false, None),
            PendingAiProviderGate::Refresh
        );
        assert_eq!(
            pending_ai_provider_gate(true, false, Some(&unavailable)),
            PendingAiProviderGate::Unavailable
        );
        assert_eq!(
            pending_ai_provider_gate(true, false, Some(&ready)),
            PendingAiProviderGate::Ready
        );
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
    fn settings_provider_probe_is_independent_of_committed_ai_enablement() {
        let committed = AppSettings {
            ai_enabled: false,
            ..AppSettings::default()
        };
        let mut draft = committed.clone();
        draft.ai_enabled = true;

        assert!(!committed.ai_enabled);
        assert!(draft.ai_enabled);
        assert!(settings_ai_status_probe_needed(true, false, false));
        assert!(!settings_ai_status_probe_needed(false, false, false));
        assert!(!settings_ai_status_probe_needed(true, true, false));
        assert!(!settings_ai_status_probe_needed(true, false, true));
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

    fn diagnostic_result(
        session_id: &str,
        task_id: &str,
        success: bool,
        output: &str,
    ) -> DiagnosticTaskResult {
        DiagnosticTaskResult::new(
            session_id,
            task_id,
            Arc::new(DiagnosticOutput {
                success,
                output: output.to_string(),
                error: (!success).then(|| "failed".to_string()),
                duration_ms: 10,
            }),
        )
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
        let policy = DiagnosticScanPolicy::snapshot(&settings, ScanKind::Full, false);
        settings.auto_save = true;
        settings.max_concurrent_tasks = 2;

        assert_eq!(policy.max_concurrent_tasks, 9);
        assert_eq!(policy.history_tag, "Full Scan");
        assert!(!scan_policy_requests_auto_save(Some(&policy)));
        assert!(!scan_policy_requests_auto_save(None));
        assert_eq!(scan_concurrency_from_settings(0), 5);

        let auto_save_settings = AppSettings {
            auto_save: true,
            ..AppSettings::default()
        };
        let initial_targeted =
            DiagnosticScanPolicy::snapshot(&auto_save_settings, ScanKind::Targeted, false);
        assert!(scan_policy_requests_auto_save(Some(&initial_targeted)));
        let targeted_rerun =
            DiagnosticScanPolicy::snapshot(&auto_save_settings, ScanKind::Targeted, true);
        assert!(!scan_policy_requests_auto_save(Some(&targeted_rerun)));
        assert_eq!(targeted_rerun.history_tag, "Manual Diagnostic");
    }

    #[test]
    fn targeted_rerun_commits_only_the_replacement_in_catalog_order() {
        let catalog = vec![
            task("first", false),
            task("second", false),
            task("third", false),
        ];
        let base = DiagnosticSnapshot {
            // Deliberately out of catalog order. The commit establishes the
            // canonical ordering while retaining an unknown prior row last.
            results: vec![
                diagnostic_result("scan-base", "second", false, "old second"),
                diagnostic_result("scan-base", "orphan", true, "orphan unchanged"),
                diagnostic_result("scan-base", "first", true, "first unchanged"),
            ],
            scan_kind: Some(ScanKind::Full),
            task_ids: vec!["second".into(), "orphan".into(), "first".into()],
            session_id: Some("scan-base".to_string()),
            duration_ms: 812,
            total: 3,
            completed: 3,
            errors: 1,
        };
        let overlay = TargetedDiagnosticOverlay::for_committed_session(
            ScanKind::Targeted,
            &["second".to_string()],
            base,
        )
        .expect("an existing result should use an overlay transaction");
        let committed = overlay
            .commit(
                diagnostic_result("scan-rerun", "second", true, "new second"),
                &catalog,
            )
            .expect("the matching replacement should commit");

        assert_eq!(
            committed
                .results
                .iter()
                .map(|result| result.task_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "orphan"]
        );
        assert_eq!(committed.results[0].output, "first unchanged");
        assert_eq!(committed.results[1].output, "new second");
        assert_eq!(committed.results[1].session_id, "scan-base");
        assert_eq!(committed.results[2].output, "orphan unchanged");
        assert_eq!(committed.scan_kind, Some(ScanKind::Full));
        assert_eq!(
            committed.task_ids,
            [
                "second".to_string(),
                "orphan".to_string(),
                "first".to_string()
            ]
        );
        assert_eq!(committed.session_id.as_deref(), Some("scan-base"));
        assert_eq!(committed.duration_ms, 812);
        assert_eq!(committed.total, 3);
        assert_eq!(committed.completed, 3);
        assert_eq!(committed.errors, 0);

        let issue_evidence = diagnostic_output_snapshot(&committed.results);
        assert_eq!(issue_evidence.len(), 3);
        assert_eq!(issue_evidence["second"].output, "new second");
        assert_eq!(issue_evidence["first"].output, "first unchanged");
    }

    #[test]
    fn targeted_rerun_rollback_restores_the_exact_committed_snapshot() {
        let base = DiagnosticSnapshot {
            results: vec![diagnostic_result("scan-base", "first", true, "old")],
            scan_kind: Some(ScanKind::Quick),
            task_ids: vec!["first".to_string()],
            session_id: Some("scan-base".to_string()),
            duration_ms: 55,
            total: 1,
            completed: 1,
            errors: 0,
        };
        let mut overlay = TargetedDiagnosticOverlay::for_committed_session(
            ScanKind::Targeted,
            &["first".to_string()],
            base.clone(),
        )
        .expect("an existing result should use an overlay transaction");
        overlay.stage(diagnostic_result("scan-rerun", "first", false, "partial"));
        assert_eq!(overlay.staged_counts(), (1, 1));
        assert!(
            overlay
                .commit(
                    diagnostic_result("scan-rerun", "wrong", true, "wrong"),
                    &[task("first", false)],
                )
                .is_err()
        );
        assert_eq!(overlay.rollback(), base);

        assert!(
            TargetedDiagnosticOverlay::for_committed_session(
                ScanKind::Targeted,
                &["missing".to_string()],
                DiagnosticSnapshot {
                    results: Vec::new(),
                    scan_kind: None,
                    task_ids: Vec::new(),
                    session_id: None,
                    duration_ms: 0,
                    total: 0,
                    completed: 0,
                    errors: 0,
                },
            )
            .is_none()
        );
    }

    #[test]
    fn targeted_rerun_adds_a_task_absent_from_the_committed_scan() {
        let base = DiagnosticSnapshot {
            results: vec![diagnostic_result("scan-base", "first", true, "first")],
            scan_kind: Some(ScanKind::Quick),
            task_ids: vec!["first".to_string()],
            session_id: Some("scan-base".to_string()),
            duration_ms: 55,
            total: 1,
            completed: 1,
            errors: 0,
        };
        let overlay = TargetedDiagnosticOverlay::for_committed_session(
            ScanKind::Targeted,
            &["second".to_string()],
            base,
        )
        .expect("a committed session overlays even when it did not select the target");
        let committed = overlay
            .commit(
                diagnostic_result("scan-rerun", "second", true, "second"),
                &[task("first", false), task("second", false)],
            )
            .expect("the new target should append to the committed task set");

        assert_eq!(
            committed
                .results
                .iter()
                .map(|result| result.task_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(committed.results[1].session_id, "scan-base");
        assert_eq!(
            committed.task_ids,
            ["first".to_string(), "second".to_string()]
        );
        assert_eq!(committed.total, 2);
        assert_eq!(committed.completed, 2);
        assert_eq!(committed.errors, 0);
        assert_eq!(committed.scan_kind, Some(ScanKind::Quick));
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
            DiagnosticTaskResult::new(
                "scan-7",
                "os_info",
                Arc::new(DiagnosticOutput {
                    success: true,
                    output: "ok".to_string(),
                    error: None,
                    duration_ms: 12,
                }),
            ),
            DiagnosticTaskResult::new(
                "scan-7",
                "logical_disk",
                Arc::new(DiagnosticOutput {
                    success: false,
                    output: "partial".to_string(),
                    error: Some("denied".to_string()),
                    duration_ms: 34,
                }),
            ),
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
                Arc::new(DiagnosticOutput {
                    success: false,
                    output: "two".to_string(),
                    error: Some("failed".to_string()),
                    duration_ms: 2,
                }),
            ),
            (
                "first".to_string(),
                Arc::new(DiagnosticOutput {
                    success: true,
                    output: "one".to_string(),
                    error: None,
                    duration_ms: 1,
                }),
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
            ("first".to_string(), Arc::new(output())),
            ("second".to_string(), Arc::new(output())),
        ]);
        assert!(authoritative_result_set_is_complete(&exact, &expected));

        let missing = HashMap::from([("first".to_string(), Arc::new(output()))]);
        assert!(!authoritative_result_set_is_complete(&missing, &expected));

        let extra = HashMap::from([
            ("first".to_string(), Arc::new(output())),
            ("second".to_string(), Arc::new(output())),
            ("stale".to_string(), Arc::new(output())),
        ]);
        assert!(!authoritative_result_set_is_complete(&extra, &expected));

        let same_size_substitution = HashMap::from([
            ("first".to_string(), Arc::new(output())),
            ("stale".to_string(), Arc::new(output())),
        ]);
        assert!(!authoritative_result_set_is_complete(
            &same_size_substitution,
            &expected,
        ));

        let duplicate_expected = vec!["first".to_string(), "first".to_string()];
        assert!(!authoritative_result_set_is_complete(
            &HashMap::from([("first".to_string(), Arc::new(output()))]),
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
    fn persisted_startup_scan_waits_for_both_initializers_and_is_consumed_once() {
        let mut gate = StartupScanGate::AwaitingSettings;
        assert!(!take_startup_scan_when_ready(
            &mut gate, false, false, None, None
        ));

        apply_startup_scan_preference(&mut gate, true);
        assert!(!take_startup_scan_when_ready(
            &mut gate, false, true, None, None
        ));
        assert!(!take_startup_scan_when_ready(
            &mut gate,
            false,
            false,
            Some(1),
            None
        ));
        assert!(!take_startup_scan_when_ready(
            &mut gate,
            false,
            false,
            None,
            Some(2)
        ));
        assert!(take_startup_scan_when_ready(
            &mut gate, false, false, None, None
        ));
        assert_eq!(gate, StartupScanGate::Consumed);
        assert!(!take_startup_scan_when_ready(
            &mut gate, false, false, None, None
        ));
    }

    #[test]
    fn startup_scan_is_suppressed_when_disabled_or_in_visual_mode() {
        let mut disabled = StartupScanGate::AwaitingSettings;
        apply_startup_scan_preference(&mut disabled, false);
        assert!(!take_startup_scan_when_ready(
            &mut disabled,
            false,
            false,
            None,
            None
        ));

        let mut visual = StartupScanGate::Armed;
        assert!(!take_startup_scan_when_ready(
            &mut visual,
            true,
            false,
            None,
            None
        ));
        assert_eq!(visual, StartupScanGate::Consumed);
    }

    #[test]
    fn export_picker_cancellation_is_silent_but_errors_keep_their_detail() {
        let cancelled = resolve_export_picker_selection(Ok(SavePickerOutcome::Cancelled)).unwrap();
        assert!(cancelled.is_none());

        let error = resolve_export_picker_selection(Err(SavePickerError::InvalidUtcDate {
            year: 2026,
            month: 2,
            day: 30,
        }))
        .unwrap_err();
        assert_eq!(error, "invalid UTC export date 2026-02-30");
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
    fn export_write_completion_requires_matching_request_and_delivery_kind() {
        let pending = PendingExport {
            request_id: 17,
            action: PendingExportAction::SupportPackage {
                paths: ValidatedSupportPackagePaths {
                    json: "case.json".into(),
                    text: "case.txt".into(),
                    html: "case.html".into(),
                },
            },
        };

        assert!(pending_export_write_is_current(
            Some(&pending),
            17,
            ExportWriteKind::SupportPackage
        ));
        assert!(!pending_export_write_is_current(
            Some(&pending),
            16,
            ExportWriteKind::SupportPackage
        ));
        assert!(!pending_export_write_is_current(
            Some(&pending),
            17,
            ExportWriteKind::File
        ));
        assert!(!pending_export_write_is_current(
            None,
            17,
            ExportWriteKind::SupportPackage
        ));
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
    fn history_task_detail_completion_requires_matching_generation_and_expansion() {
        assert!(history_task_diff_result_is_current(
            7,
            7,
            "os_info",
            Some("os_info")
        ));
        assert!(!history_task_diff_result_is_current(
            6,
            7,
            "os_info",
            Some("os_info")
        ));
        assert!(!history_task_diff_result_is_current(
            7,
            7,
            "os_info",
            Some("cpu_info")
        ));
        assert!(!history_task_diff_result_is_current(7, 7, "os_info", None));
    }
}
