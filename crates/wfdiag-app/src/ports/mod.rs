//! Every environmental boundary the application service depends on.
//!
//! [`AppPorts`] is the complete list. A real Windows shell builds it from
//! [`native::windows_ports`]; a headless test builds it from [`mock`]. The
//! service itself never names a platform API, which is what lets its
//! integration tests run on Linux with no GUI.

pub mod mock;
pub mod monitor;
#[cfg(windows)]
pub mod native;

use std::fmt;
use std::sync::Arc;
use wfdiag_native_ai_provider::ProviderManagementBackend;
use wfdiag_native_diagnostics::DiagnosticExecutor;
use wfdiag_native_issues::Timestamp;
use wfdiag_native_settings::{CredentialStorage, SettingsStorage, SettingsValidator};
use wfdiag_native_system::SystemProvider;
use wfdiag_native_update::{CurrentVersionProvider, ReleaseHttp, SignatureProvider};

pub use monitor::{MonitorPort, MonitorProfileKind, NoopMonitor};

/// Clock and environment inputs that must be injectable for determinism.
///
/// Issue detection is a pure function of scan evidence plus these two values,
/// so they are read here rather than inside a detector.
pub trait EnvironmentPort: Send + Sync {
    /// The current UTC time used for issue detection and history records.
    fn now(&self) -> Timestamp;
    /// Milliseconds since the Unix epoch, for the update throttle.
    fn now_millis(&self) -> u64;
    /// How many entries the temporary directory holds, when countable.
    fn temp_file_count(&self) -> Option<usize>;
}

/// The real clock and temporary directory.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvironment;

impl EnvironmentPort for SystemEnvironment {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn now_millis(&self) -> u64 {
        wfdiag_native_update::policy::unix_time_millis()
    }

    fn temp_file_count(&self) -> Option<usize> {
        std::fs::read_dir(std::env::temp_dir())
            .ok()
            .map(Iterator::count)
    }
}

/// Persistence for the once-per-day startup update throttle.
pub trait UpdateThrottlePort: Send + Sync {
    /// Whether a passive check may run at `now_millis`.
    fn should_check(&self, now_millis: u64) -> bool;

    /// Record that a check completed.
    ///
    /// # Errors
    ///
    /// Returns the persistence diagnostic. Failure is fail-open: the next
    /// launch simply checks again.
    fn record(&self, now_millis: u64) -> Result<(), String>;
}

/// A throttle that always allows and never persists. This is the default for
/// hosts that do not ship the GitHub update channel.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysCheckThrottle;

impl UpdateThrottlePort for AlwaysCheckThrottle {
    fn should_check(&self, _now_millis: u64) -> bool {
        true
    }

    fn record(&self, _now_millis: u64) -> Result<(), String> {
        Ok(())
    }
}

/// Relaunching the application with administrator rights.
pub trait ElevationPort: Send + Sync {
    /// Ask the host to relaunch elevated.
    ///
    /// `Ok(true)` means the elevated process started and this one should
    /// exit; `Ok(false)` means the user declined.
    ///
    /// # Errors
    ///
    /// Returns a message when the relaunch could not be attempted.
    fn restart_as_admin(&self) -> Result<bool, String>;
}

/// An elevation port for hosts that cannot elevate.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedElevation;

impl ElevationPort for UnsupportedElevation {
    fn restart_as_admin(&self) -> Result<bool, String> {
        Err("Elevation is not supported on this host".to_string())
    }
}

/// Every boundary the application service depends on, in one bundle.
#[derive(Clone)]
pub struct AppPorts {
    /// Executes diagnostic tasks.
    pub diagnostics: Arc<dyn DiagnosticExecutor>,
    /// Reads host identity and CPU architecture.
    pub system: Arc<dyn SystemProvider>,
    /// Reads and writes the settings document.
    pub settings_storage: Arc<dyn SettingsStorage>,
    /// Reads and writes provider API keys.
    pub credentials: Arc<dyn CredentialStorage>,
    /// Admission policy applied before a settings document is saved.
    pub settings_validator: Arc<dyn SettingsValidator>,
    /// Fetches the latest public release.
    pub release_http: Arc<dyn ReleaseHttp>,
    /// Classifies the running package's signature.
    pub signature: Arc<dyn SignatureProvider>,
    /// Reports the running application version.
    pub current_version: Arc<dyn CurrentVersionProvider>,
    /// Probes and mutates AI provider selection.
    pub provider_backend: Arc<dyn ProviderManagementBackend>,
    /// Starts live system monitoring.
    pub monitor: Arc<dyn MonitorPort>,
    /// Relaunches elevated.
    pub elevation: Arc<dyn ElevationPort>,
    /// Clock and temporary-directory inputs.
    pub environment: Arc<dyn EnvironmentPort>,
    /// Persists the update-check throttle.
    pub update_throttle: Arc<dyn UpdateThrottlePort>,
}

impl fmt::Debug for AppPorts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("AppPorts").finish_non_exhaustive()
    }
}

impl AppPorts {
    /// A complete in-memory port bundle for headless tests.
    ///
    /// Use [`mock::MockPorts`] instead when the test needs to script an
    /// executor, inspect what was saved, or advance the clock.
    #[must_use]
    pub fn mock() -> Self {
        mock::MockPorts::new().into_ports()
    }
}
