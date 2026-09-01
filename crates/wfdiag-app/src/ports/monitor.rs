//! The live-monitoring port and its portable telemetry projections.
//!
//! `wfdiag-native-monitor` is a `#![cfg(windows)]` crate, so the application
//! service never names it directly. Everything the engine needs from live
//! monitoring is expressed here: a start/pause/refresh handle, an on-demand
//! process page, and the network-connection list. Off Windows (and in every
//! headless test) [`NoopMonitor`] answers instead, which is what lets the
//! whole crate build and test on Linux.

use std::fmt;
use tokio::sync::oneshot;
use wfdiag_ui_core::UiEventReceiver;

/// How much telemetry the collector should gather.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorProfileKind {
    /// One-second system telemetry with no process enumeration. Native shells
    /// use this; process pages are requested on demand instead.
    #[default]
    SystemOnly,
    /// Legacy behaviour: every sample also enumerates processes.
    Legacy {
        /// Whether per-process GPU/NPU adapter statistics are collected.
        include_process_adapter_stats: bool,
    },
}

/// Which column a process page is sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessSortKey {
    /// Process image name.
    Name,
    /// Process id.
    Pid,
    /// CPU utilisation.
    #[default]
    CpuPercent,
    /// Working set as a share of physical memory.
    MemoryPercent,
    /// Working set in megabytes.
    MemoryMb,
    /// Reported process status.
    Status,
    /// Thread count.
    ThreadCount,
    /// GPU utilisation.
    GpuPercent,
    /// NPU utilisation.
    NpuPercent,
}

/// Sort direction for a process page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessSortDirection {
    /// Ascending.
    Asc,
    /// Descending.
    #[default]
    Desc,
}

/// One on-demand process-page request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessQuery {
    /// Case-insensitive name filter; empty matches everything.
    pub search: String,
    /// The sort column.
    pub sort_by: ProcessSortKey,
    /// The sort direction.
    pub sort_direction: ProcessSortDirection,
    /// Row offset into the sorted, filtered set.
    pub offset: usize,
    /// Maximum rows returned.
    pub limit: usize,
}

impl Default for ProcessQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            sort_by: ProcessSortKey::default(),
            sort_direction: ProcessSortDirection::default(),
            offset: 0,
            limit: 100,
        }
    }
}

/// One row of a process page.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessRow {
    /// Process id.
    pub pid: u32,
    /// Parent process id.
    pub parent_pid: u32,
    /// Image name.
    pub name: String,
    /// CPU utilisation percentage.
    pub cpu_percent: f32,
    /// Working set as a share of physical memory.
    pub memory_percent: f32,
    /// Working set in megabytes.
    pub memory_mb: f64,
    /// Virtual size in megabytes.
    pub virtual_memory_mb: f64,
    /// GPU utilisation, when the adapter reported any.
    pub gpu_percent: Option<f32>,
    /// GPU memory in megabytes, when reported.
    pub gpu_memory_mb: Option<f64>,
    /// NPU utilisation, when reported.
    pub npu_percent: Option<f32>,
    /// NPU memory in megabytes, when reported.
    pub npu_memory_mb: Option<f64>,
    /// Accumulated CPU time in seconds.
    pub cpu_time_secs: u64,
    /// Process start time, Unix seconds.
    pub start_time: i64,
    /// Reported status.
    pub status: String,
    /// Thread count.
    pub thread_count: u32,
    /// Handle count.
    pub handle_count: u32,
    /// Base priority.
    pub priority: i32,
    /// Bytes read.
    pub io_read_bytes: u64,
    /// Bytes written.
    pub io_write_bytes: u64,
}

/// One page of the process explorer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessPage {
    /// When the underlying enumeration ran, Unix seconds.
    pub captured_at: i64,
    /// Total rows matching the filter.
    pub total: usize,
    /// The offset this page starts at.
    pub offset: usize,
    /// The requested page size.
    pub limit: usize,
    /// The rows themselves.
    pub items: Vec<ProcessRow>,
}

/// One active or listening network connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConnection {
    /// TCP or UDP.
    pub protocol: String,
    /// Local address and port.
    pub local_addr: String,
    /// Remote address and port.
    pub remote_addr: String,
    /// Connection state.
    pub status: String,
}

/// Result of one queued process query.
#[derive(Debug)]
pub enum ProcessQueryOutcome {
    /// The requested page.
    Page(Box<ProcessPage>),
    /// A newer query replaced this one before it could run.
    Superseded,
}

/// Reply handle for a process page.
pub type ProcessPageReply = oneshot::Receiver<ProcessQueryOutcome>;
/// Reply handle for the network-connection list.
pub type NetworkConnectionsReply = oneshot::Receiver<Vec<NetworkConnection>>;

/// A running collector.
pub trait MonitorHandle: Send + Sync {
    /// Stop sampling without tearing the collector down.
    fn pause(&self) -> bool;
    /// Resume one-second sampling.
    fn resume(&self) -> bool;
    /// Request one immediate sample.
    fn refresh(&self) -> bool;

    /// Queue an on-demand process page.
    ///
    /// # Errors
    ///
    /// Returns a message when the collector has stopped.
    fn request_processes(&self, query: ProcessQuery) -> Result<ProcessPageReply, String>;

    /// Queue the network-connection list.
    ///
    /// # Errors
    ///
    /// Returns a message when the collector has stopped.
    fn request_network_connections(&self) -> Result<NetworkConnectionsReply, String>;
}

/// A started collector plus the event stream it publishes to.
pub struct MonitorSession {
    /// Control handle.
    pub handle: Box<dyn MonitorHandle>,
    /// The `SystemStats` event stream.
    pub events: UiEventReceiver,
}

impl fmt::Debug for MonitorSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MonitorSession")
            .finish_non_exhaustive()
    }
}

/// The live-monitoring boundary.
pub trait MonitorPort: Send + Sync {
    /// Start collecting.
    ///
    /// Returning `Ok(None)` means "this host has no live monitoring" and is
    /// not an error: the engine simply reports monitoring as unavailable.
    ///
    /// # Errors
    ///
    /// Returns a message when the collector exists but could not start.
    fn start(&self, profile: MonitorProfileKind) -> Result<Option<MonitorSession>, String>;
}

/// The monitor for hosts without live telemetry (every non-Windows host, and
/// every headless test that does not script its own).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMonitor;

impl MonitorPort for NoopMonitor {
    fn start(&self, _profile: MonitorProfileKind) -> Result<Option<MonitorSession>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{MonitorPort, MonitorProfileKind, NoopMonitor, ProcessQuery};

    #[test]
    fn the_noop_monitor_reports_no_session_without_failing() {
        let session = NoopMonitor
            .start(MonitorProfileKind::SystemOnly)
            .expect("the no-op monitor never fails");
        assert!(session.is_none());
    }

    #[test]
    fn the_default_process_query_matches_the_shipping_page_size() {
        let query = ProcessQuery::default();
        assert_eq!(query.limit, 100);
        assert_eq!(query.offset, 0);
        assert!(query.search.is_empty());
    }
}
