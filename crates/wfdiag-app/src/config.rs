//! Host-supplied configuration for one [`crate::AppService`].

use crate::ports::monitor::MonitorProfileKind;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

/// The diagnostic event bus capacity used by the shipping shell.
pub const DIAGNOSTIC_EVENT_CAPACITY: usize = 256;
/// How long a worker reply may take before it becomes a typed timeout.
pub const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the reply watcher wakes the host while work is outstanding.
pub const DEFAULT_REPLY_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The scan executor's worker-thread count, matching the shipping shell.
pub const DEFAULT_EXECUTOR_THREADS: usize = 5;
/// The default per-worker teardown budget.
pub const DEFAULT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);

/// Everything a host chooses about one service instance.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// Where scan history lives. `None` starts no history worker, and every
    /// history command is then rejected as unavailable.
    pub history_storage_dir: Option<PathBuf>,
    /// Capacity of the diagnostic event bus.
    pub diagnostic_event_capacity: NonZeroUsize,
    /// Capacity of the outbound [`crate::AppEvent`] queue.
    pub event_capacity: usize,
    /// Whether live monitoring starts with the service.
    pub start_monitor: bool,
    /// How much telemetry the collector gathers.
    pub monitor_profile: MonitorProfileKind,
    /// How long a worker reply may take.
    pub reply_timeout: Duration,
    /// How often the reply watcher wakes the host.
    pub reply_poll_interval: Duration,
    /// Worker threads for the scan executor.
    pub executor_threads: usize,
    /// Per-worker teardown budget used by [`crate::AppService::shutdown`].
    pub shutdown_budget: Duration,
    /// Whether this is a debug build. A debug build never contacts the GitHub
    /// update channel, exactly like a Store install.
    pub debug_build: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            history_storage_dir: None,
            diagnostic_event_capacity: NonZeroUsize::new(DIAGNOSTIC_EVENT_CAPACITY)
                .expect("the shipping diagnostic capacity is non-zero"),
            event_capacity: 1024,
            start_monitor: false,
            monitor_profile: MonitorProfileKind::SystemOnly,
            reply_timeout: DEFAULT_REPLY_TIMEOUT,
            reply_poll_interval: DEFAULT_REPLY_POLL_INTERVAL,
            executor_threads: DEFAULT_EXECUTOR_THREADS,
            shutdown_budget: DEFAULT_SHUTDOWN_BUDGET,
            debug_build: cfg!(debug_assertions),
        }
    }
}

impl AppConfig {
    /// Use the shipping scan-history directory.
    ///
    /// # Errors
    ///
    /// Returns the storage layer's diagnostic when the directory cannot be
    /// resolved (no `APPDATA`, for instance).
    pub fn with_shipping_history_dir(self) -> Result<Self, String> {
        let directory = wfdiag_native_history::ScanStorage::default_storage_directory()?;
        Ok(self.with_history_dir(directory))
    }

    /// Store history under `directory` and start the history worker.
    #[must_use]
    pub fn with_history_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.history_storage_dir = Some(directory.into());
        self
    }

    /// Start live monitoring with the service.
    #[must_use]
    pub const fn with_monitor(mut self, start: bool) -> Self {
        self.start_monitor = start;
        self
    }

    /// Declare whether this is a debug build, which silences update checks.
    #[must_use]
    pub const fn with_debug_build(mut self, debug_build: bool) -> Self {
        self.debug_build = debug_build;
        self
    }

    /// Replace the worker-reply deadline.
    #[must_use]
    pub const fn with_reply_timeout(mut self, timeout: Duration) -> Self {
        self.reply_timeout = timeout;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, DEFAULT_REPLY_TIMEOUT, DIAGNOSTIC_EVENT_CAPACITY};

    #[test]
    fn defaults_match_the_shipping_shell() {
        let config = AppConfig::default();
        assert_eq!(
            config.diagnostic_event_capacity.get(),
            DIAGNOSTIC_EVENT_CAPACITY
        );
        assert_eq!(config.reply_timeout, DEFAULT_REPLY_TIMEOUT);
        assert!(config.history_storage_dir.is_none());
        assert!(!config.start_monitor);
    }

    #[test]
    fn the_builders_compose() {
        let config = AppConfig::default()
            .with_history_dir("/tmp/wfdiag-history")
            .with_monitor(true)
            .with_reply_timeout(std::time::Duration::from_secs(1));
        assert!(config.history_storage_dir.is_some());
        assert!(config.start_monitor);
        assert_eq!(config.reply_timeout, std::time::Duration::from_secs(1));
    }
}
