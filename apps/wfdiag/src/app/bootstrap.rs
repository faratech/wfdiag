//! Starting the one application service, and the first frame's facts.
//!
//! Everything environmental is chosen here and nowhere else: the settings
//! store, the provider-preference admission rule, the update throttle beside
//! it, the AI provider backend, and the elevation policy.

#![deny(unsafe_code)]

use crate::app::consts::APP_VERSION;
use crate::app::policy::reactor_provider_backend;
use std::sync::Arc;
use wfdiag_app::ports::native::{WindowsPortOverrides, windows_ports_with};
use wfdiag_app::{AppConfig, AppEventReceiver, AppService, ElevationPort};
use wfdiag_native_ai_provider::{
    PackageIdentitySource, ProviderPreferenceSettingsValidator, SharedAiCache,
};
use wfdiag_native_diagnostics::DiagnosticTask;
use wfdiag_native_history::ScanStorage;
use wfdiag_native_issues::{Issue, RemediationSummary};
use wfdiag_native_remediation::broker::ActionProposal;
use wfdiag_native_remediation::runtime::ActionRunSummary;
use wfdiag_native_settings::AppSettings;
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};

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

/// Start the one application service the shell drives.
///
/// Everything environmental is chosen here and nowhere else: the settings
/// store (redirected by the `settings-test-path` validation feature), the
/// provider-preference admission rule, the update throttle beside it, the AI
/// provider backend, and the elevation policy. The startup-scan gate, the
/// per-worker teardown budget and every request-id counter belong to the
/// service.
pub(crate) fn start_application_service(
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
pub(crate) struct EngineBoot {
    pub(crate) settings: AppSettings,
    pub(crate) settings_error: Option<String>,
    pub(crate) catalog: Vec<DiagnosticTask>,
    pub(crate) maintenance: Vec<RemediationSummary>,
    pub(crate) issues: Vec<Issue>,
    pub(crate) active_run: Option<ActionRunSummary>,
    pub(crate) run_history: Vec<ActionRunSummary>,
    pub(crate) review: Option<ActionProposal>,
    pub(crate) system_info: Option<SystemInfo>,
    pub(crate) architecture: Option<ArchitectureSnapshot>,
    pub(crate) system_error: Option<String>,
    pub(crate) session_id: Option<String>,
}

impl EngineBoot {
    pub(crate) fn capture(app: Option<&AppService>) -> Self {
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
