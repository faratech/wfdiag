//! Real Windows wiring for [`AppPorts`].
//!
//! Everything here is `cfg(windows)`. The monitor port is the interesting
//! part: it is the only adapter that converts between this crate's portable
//! telemetry projections and the `#![cfg(windows)]` collector crate.

use std::sync::Arc;
use tokio::sync::oneshot;
use wfdiag_native_ai_provider::ProviderManagementBackend;
use wfdiag_native_monitor::{
    MonitorProfile, NativeMonitorRuntime, ProcessQueryOutcome as NativeProcessQueryOutcome,
};
use wfdiag_native_settings::{
    AllowAllSettings, ShippingSettingsStorage, WindowsDpapiCredentialStorage,
};
use wfdiag_native_update::{
    ReqwestReleaseHttp, StaticCurrentVersion, WindowsPackageSignatureProvider,
};

use super::ai::AiPorts;
use super::monitor::{
    MonitorHandle, MonitorPort, MonitorProfileKind, MonitorSession, NetworkConnection,
    NetworkConnectionsReply, ProcessPage, ProcessPageReply, ProcessQuery, ProcessQueryOutcome,
    ProcessRow, ProcessSortDirection, ProcessSortKey,
};
use super::{AppPorts, ElevationPort, SystemEnvironment, UnsupportedElevation, UpdateThrottlePort};
use wfdiag_native_ai_provider::{FoundryCliEndpointSource, ReqwestOllamaSource, SharedAiCache};
use wfdiag_native_settings::SettingsService;

impl From<ProcessSortKey> for wfdiag_native_monitor::ProcessSortKey {
    fn from(value: ProcessSortKey) -> Self {
        match value {
            ProcessSortKey::Name => Self::Name,
            ProcessSortKey::Pid => Self::Pid,
            ProcessSortKey::CpuPercent => Self::CpuPercent,
            ProcessSortKey::MemoryPercent => Self::MemoryPercent,
            ProcessSortKey::MemoryMb => Self::MemoryMb,
            ProcessSortKey::Status => Self::Status,
            ProcessSortKey::ThreadCount => Self::ThreadCount,
            ProcessSortKey::GpuPercent => Self::GpuPercent,
            ProcessSortKey::NpuPercent => Self::NpuPercent,
        }
    }
}

impl From<ProcessSortDirection> for wfdiag_native_monitor::ProcessSortDirection {
    fn from(value: ProcessSortDirection) -> Self {
        match value {
            ProcessSortDirection::Asc => Self::Asc,
            ProcessSortDirection::Desc => Self::Desc,
        }
    }
}

impl From<ProcessQuery> for wfdiag_native_monitor::ProcessQuery {
    fn from(value: ProcessQuery) -> Self {
        Self {
            search: value.search,
            sort_by: value.sort_by.into(),
            sort_direction: value.sort_direction.into(),
            offset: value.offset,
            limit: value.limit,
        }
    }
}

impl From<&wfdiag_native_monitor::ProcessRow> for ProcessRow {
    fn from(row: &wfdiag_native_monitor::ProcessRow) -> Self {
        Self {
            pid: row.pid,
            parent_pid: row.parent_pid,
            name: row.name.clone(),
            cpu_percent: row.cpu_percent,
            memory_percent: row.memory_percent,
            memory_mb: row.memory_mb,
            virtual_memory_mb: row.virtual_memory_mb,
            gpu_percent: row.gpu_percent,
            gpu_memory_mb: row.gpu_memory_mb,
            npu_percent: row.npu_percent,
            npu_memory_mb: row.npu_memory_mb,
            cpu_time_secs: row.cpu_time_secs,
            start_time: row.start_time,
            status: row.status.clone(),
            thread_count: row.thread_count,
            handle_count: row.handle_count,
            priority: row.priority,
            io_read_bytes: row.io_read_bytes,
            io_write_bytes: row.io_write_bytes,
        }
    }
}

impl From<wfdiag_native_monitor::ProcessPage> for ProcessPage {
    fn from(page: wfdiag_native_monitor::ProcessPage) -> Self {
        Self {
            captured_at: page.captured_at,
            total: page.total,
            offset: page.offset,
            limit: page.limit,
            items: page.items.iter().map(ProcessRow::from).collect(),
        }
    }
}

impl From<&wfdiag_native_monitor::NetworkConnection> for NetworkConnection {
    fn from(connection: &wfdiag_native_monitor::NetworkConnection) -> Self {
        Self {
            protocol: connection.protocol.clone(),
            local_addr: connection.local_addr.clone(),
            remote_addr: connection.remote_addr.clone(),
            status: connection.status.clone(),
        }
    }
}

impl From<MonitorProfileKind> for MonitorProfile {
    fn from(value: MonitorProfileKind) -> Self {
        match value {
            MonitorProfileKind::SystemOnly => Self::SystemOnly,
            MonitorProfileKind::Legacy {
                include_process_adapter_stats,
            } => Self::Legacy {
                include_process_adapter_stats,
            },
        }
    }
}

/// The shipping Windows telemetry collector behind the portable port.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsMonitor;

struct WindowsMonitorHandle {
    runtime: Arc<NativeMonitorRuntime>,
}

impl MonitorHandle for WindowsMonitorHandle {
    fn pause(&self) -> bool {
        self.runtime.pause()
    }

    fn resume(&self) -> bool {
        self.runtime.resume()
    }

    fn refresh(&self) -> bool {
        self.runtime.refresh()
    }

    fn request_processes(&self, query: ProcessQuery) -> Result<ProcessPageReply, String> {
        let native = self
            .runtime
            .request_processes(query.into())
            .map_err(|error| error.to_string())?;
        let (sender, receiver) = oneshot::channel();
        // The collector answers on its own worker; translate on a short-lived
        // thread so neither the UI thread nor the collector is blocked.
        std::thread::Builder::new()
            .name("wfdiag-app-process-page".to_string())
            .spawn(move || {
                let outcome = match native.blocking_recv() {
                    Ok(NativeProcessQueryOutcome::Page(page)) => {
                        ProcessQueryOutcome::Page(Box::new(page.into()))
                    }
                    Ok(NativeProcessQueryOutcome::Superseded) | Err(_) => {
                        ProcessQueryOutcome::Superseded
                    }
                };
                let _ = sender.send(outcome);
            })
            .map_err(|error| error.to_string())?;
        Ok(receiver)
    }

    fn request_network_connections(&self) -> Result<NetworkConnectionsReply, String> {
        let (sender, receiver) = oneshot::channel();
        std::thread::Builder::new()
            .name("wfdiag-app-connections".to_string())
            .spawn(move || {
                let connections = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map(|runtime| {
                        runtime.block_on(wfdiag_native_monitor::get_network_connections())
                    })
                    .unwrap_or_default();
                let _ = sender.send(connections.iter().map(NetworkConnection::from).collect());
            })
            .map_err(|error| error.to_string())?;
        Ok(receiver)
    }
}

impl MonitorPort for WindowsMonitor {
    fn start(&self, profile: MonitorProfileKind) -> Result<Option<MonitorSession>, String> {
        let (runtime, events) = NativeMonitorRuntime::start_with_profile(profile.into())
            .map_err(|error| error.to_string())?;
        Ok(Some(MonitorSession {
            handle: Box::new(WindowsMonitorHandle {
                runtime: Arc::new(runtime),
            }),
            events,
        }))
    }
}

/// The shipping once-a-day update throttle stored beside settings.json.
#[derive(Debug, Clone)]
pub struct ShippingUpdateThrottle {
    throttle: wfdiag_native_update::policy::UpdateThrottle,
}

impl ShippingUpdateThrottle {
    /// Resolve the throttle file beside the shipping settings file.
    ///
    /// # Errors
    ///
    /// Returns the settings-layer diagnostic when the configuration directory
    /// cannot be resolved.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            throttle: wfdiag_native_update::policy::UpdateThrottle::shipping()?,
        })
    }
}

impl UpdateThrottlePort for ShippingUpdateThrottle {
    fn should_check(&self, now_millis: u64) -> bool {
        self.throttle.should_check_at(now_millis)
    }

    fn record(&self, now_millis: u64) -> Result<(), String> {
        self.throttle.record_at(now_millis)
    }
}

/// Build the shipping Windows port bundle.
///
/// The AI provider backend and the elevation policy stay with the shell: both
/// depend on application composition (package identity, cache ownership,
/// relaunch UX) that this crate deliberately does not own.
///
/// # Panics
///
/// Panics only if the compiled-in `0.0.0` fallback — used when
/// `current_version` is not valid semantic version syntax — ever stops parsing.
#[must_use]
pub fn windows_ports(
    provider_backend: Arc<dyn ProviderManagementBackend>,
    elevation: Option<Arc<dyn ElevationPort>>,
    current_version: &str,
) -> AppPorts {
    windows_ports_with(
        WindowsPortOverrides::default(),
        provider_backend,
        elevation,
        current_version,
    )
}

/// The shipping choices a host may replace.
///
/// A validation build redirects the settings document to an isolated path and
/// puts the update throttle beside it; the shell also enforces its own
/// provider-preference admission rule at the settings layer. Everything left
/// `None` keeps the shipping default.
#[derive(Clone, Default)]
pub struct WindowsPortOverrides {
    /// Where the settings document lives.
    pub settings_storage: Option<Arc<dyn wfdiag_native_settings::SettingsStorage>>,
    /// The admission policy applied before a settings document is saved.
    pub settings_validator: Option<Arc<dyn wfdiag_native_settings::SettingsValidator>>,
    /// Where the once-a-day update-check throttle is persisted.
    pub update_throttle: Option<Arc<dyn UpdateThrottlePort>>,
}

impl std::fmt::Debug for WindowsPortOverrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsPortOverrides")
            .field("settings_storage", &self.settings_storage.is_some())
            .field("settings_validator", &self.settings_validator.is_some())
            .field("update_throttle", &self.update_throttle.is_some())
            .finish()
    }
}

/// Build the Windows port bundle with host overrides.
///
/// # Panics
///
/// Panics only if the compiled-in `0.0.0` fallback — used when
/// `current_version` is not valid semantic version syntax — ever stops parsing.
#[must_use]
pub fn windows_ports_with(
    overrides: WindowsPortOverrides,
    provider_backend: Arc<dyn ProviderManagementBackend>,
    elevation: Option<Arc<dyn ElevationPort>>,
    current_version: &str,
) -> AppPorts {
    let settings_storage: Arc<dyn wfdiag_native_settings::SettingsStorage> = overrides
        .settings_storage
        .unwrap_or_else(|| Arc::new(ShippingSettingsStorage::new()));
    let credentials: Arc<dyn wfdiag_native_settings::CredentialStorage> =
        Arc::new(WindowsDpapiCredentialStorage::new());
    let settings_validator: Arc<dyn wfdiag_native_settings::SettingsValidator> = overrides
        .settings_validator
        .unwrap_or_else(|| Arc::new(AllowAllSettings));
    // The AI resolvers read live settings and DPAPI keys per request, so they
    // are built from the same storages the service's own settings service uses.
    let ai = AiPorts::shipping(
        SettingsService::new(
            Arc::clone(&settings_storage),
            Arc::clone(&credentials),
            Arc::clone(&settings_validator),
        ),
        Arc::new(FoundryCliEndpointSource::new()),
        Arc::new(ReqwestOllamaSource),
        SharedAiCache::new(32),
    );
    AppPorts {
        diagnostics: Arc::new(wfdiag_native_diagnostics::NativeDiagnosticExecutor),
        system: Arc::new(wfdiag_native_system::NativeSystemProvider),
        settings_storage,
        credentials,
        settings_validator,
        release_http: Arc::new(ReqwestReleaseHttp),
        signature: Arc::new(WindowsPackageSignatureProvider::new()),
        current_version: Arc::new(
            StaticCurrentVersion::parse(current_version)
                .unwrap_or_else(|_| StaticCurrentVersion::parse("0.0.0").expect("valid semver")),
        ),
        provider_backend,
        monitor: Arc::new(WindowsMonitor),
        elevation: elevation.unwrap_or_else(|| Arc::new(UnsupportedElevation)),
        environment: Arc::new(SystemEnvironment),
        update_throttle: overrides.update_throttle.unwrap_or_else(|| {
            ShippingUpdateThrottle::new().map_or_else(
                |_| Arc::new(super::AlwaysCheckThrottle) as Arc<dyn UpdateThrottlePort>,
                |throttle| Arc::new(throttle) as Arc<dyn UpdateThrottlePort>,
            )
        }),
        ai,
    }
}
