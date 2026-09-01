//! The read model a host renders from.
//!
//! [`AppSnapshot`] is owned by the service and mutated only inside
//! [`crate::AppService::drain`]. A host reads it after every drain; it never
//! reconstructs state from the event stream, and it never compares an id.
//!
//! `settings` is populated **synchronously** during
//! [`crate::AppService::start`], before the settings worker exists, so the very
//! first frame can use the persisted theme instead of flashing the default one.

use crate::command::WorkerKind;
use crate::domain::invalidation::Invalidation;
use crate::domain::scan::{ScanPhase, ScanSnapshot};
use crate::ports::monitor::{NetworkConnection, ProcessPage};
use wfdiag_native_ai_provider::AIProviderStatus;
use wfdiag_native_diagnostics::DiagnosticTask;
use wfdiag_native_history::{ComparisonResult, ComparisonSummary, ScanSummary, TaskTrend};
use wfdiag_native_issues::{Issue, RemediationSummary};
use wfdiag_native_settings::AppSettings;
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use wfdiag_native_update::{UpdateInfo, UpdateOutcome};
use wfdiag_ui_core::SystemStats;

/// A worker that could not start, or has stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerUnavailable {
    /// Which worker.
    pub worker: WorkerKind,
    /// The diagnostic to show the user.
    pub detail: String,
}

/// Scan-history read model.
#[derive(Clone, Debug, Default)]
pub struct HistorySnapshot {
    /// Stored scans, newest first.
    pub summaries: Vec<ScanSummary>,
    /// A list or comparison request is in flight.
    pub loading: bool,
    /// The most recent history failure.
    pub error: Option<String>,
    /// The most recent full comparison.
    pub comparison: Option<ComparisonResult>,
    /// The most recent lightweight comparison.
    pub comparison_summary: Option<ComparisonSummary>,
    /// Per-task failure trends.
    pub trends: Vec<TaskTrend>,
}

/// Live-monitoring read model.
#[derive(Clone, Debug, Default)]
pub struct MonitorSnapshot {
    /// Whether a collector is running.
    pub available: bool,
    /// Whether sampling is paused.
    pub paused: bool,
    /// The newest telemetry sample.
    pub latest: Option<SystemStats>,
    /// The most recent monitoring failure.
    pub error: Option<String>,
    /// The most recent process page.
    pub process_page: Option<ProcessPage>,
    /// The most recent network-connection list.
    pub connections: Option<Vec<NetworkConnection>>,
}

/// Update-channel read model.
#[derive(Clone, Debug, Default)]
pub struct UpdateSnapshot {
    /// The outcome of the last completed check.
    pub last_outcome: Option<UpdateOutcome>,
    /// The newer release, when one was found.
    pub available: Option<UpdateInfo>,
    /// A check is in flight.
    pub in_flight: bool,
}

/// The complete application read model.
// Independent per-domain facts a host renders directly; there is no single
// state machine that would replace them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default)]
pub struct AppSnapshot {
    /// The persisted settings document, loaded synchronously at start.
    pub settings: AppSettings,
    /// A settings load or save is in flight.
    pub settings_loading: bool,
    /// The most recent settings failure.
    pub settings_error: Option<String>,
    /// The diagnostic task catalog.
    pub catalog: Vec<DiagnosticTask>,
    /// Host identity, once the system worker answers.
    pub system_info: Option<SystemInfo>,
    /// CPU architecture, once the system worker answers.
    pub architecture: Option<ArchitectureSnapshot>,
    /// The most recent system-identity failure.
    pub system_error: Option<String>,
    /// The committed (or in-flight) scan.
    pub scan: ScanSnapshot,
    /// Where the scan machine is.
    pub scan_phase: ScanPhase,
    /// The current issue projection.
    pub issues: Vec<Issue>,
    /// The most recent issue-detection failure.
    pub issue_error: Option<String>,
    /// Read-only remediation metadata, in stable id order.
    pub remediations: Vec<RemediationSummary>,
    /// Which AI-derived content the last transaction invalidated. The derived
    /// domains are declared extension points, so nothing consumes this yet;
    /// it is published so the wiring step has one authoritative answer.
    pub derived_invalidated: Invalidation,
    /// Scan history.
    pub history: HistorySnapshot,
    /// Live monitoring.
    pub monitor: MonitorSnapshot,
    /// The last AI provider status.
    pub provider_status: Option<AIProviderStatus>,
    /// A provider probe is in flight.
    pub provider_loading: bool,
    /// The update channel.
    pub update: UpdateSnapshot,
    /// Whether the host window is visible.
    pub window_visible: bool,
    /// Whether shutdown has begun.
    pub terminating: bool,
    /// Workers that could not start, or stopped.
    pub worker_errors: Vec<WorkerUnavailable>,
}

impl AppSnapshot {
    /// Whether the host is an administrator, once identity is known.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.system_info.as_ref().is_some_and(|info| info.is_admin)
    }

    /// Whether a scan occupies the engine.
    #[must_use]
    pub const fn scan_busy(&self) -> bool {
        !matches!(self.scan_phase, ScanPhase::Idle)
    }

    /// The remediations flagged as always-available maintenance actions.
    #[must_use]
    pub fn maintenance_remediations(&self) -> Vec<RemediationSummary> {
        self.remediations
            .iter()
            .filter(|summary| summary.maintenance)
            .cloned()
            .collect()
    }

    /// Record a worker that is unavailable, replacing any earlier record.
    pub(crate) fn record_worker_error(&mut self, worker: WorkerKind, detail: impl Into<String>) {
        let detail = detail.into();
        if let Some(existing) = self
            .worker_errors
            .iter_mut()
            .find(|error| error.worker == worker)
        {
            existing.detail = detail;
        } else {
            self.worker_errors
                .push(WorkerUnavailable { worker, detail });
        }
    }

    /// The recorded failure for one worker, if any.
    #[must_use]
    pub fn worker_error(&self, worker: WorkerKind) -> Option<&str> {
        self.worker_errors
            .iter()
            .find(|error| error.worker == worker)
            .map(|error| error.detail.as_str())
    }
}
