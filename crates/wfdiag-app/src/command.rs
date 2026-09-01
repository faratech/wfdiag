//! The single command surface into the application service.
//!
//! One enum in, one event stream out. A host never talks to a worker runtime,
//! never holds a request id, and never compares one: it dispatches a command
//! and reads [`crate::AppEvent`]s.
//!
//! Variants are grouped by domain. Every domain the workspace has a runtime
//! for is routed through this facade: scanning, history, monitoring, providers,
//! settings, export, updates, agentic chat, the AI report, per-task analysis,
//! issue prioritisation, fix plans, remediation execution, model catalogs, and
//! subscription CLI management. A command that cannot run right now is refused
//! with a typed [`RejectReason`], never a silent no-op.

use crate::ids::RequestId;
use crate::ports::monitor::ProcessQuery;
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_export::ExportRequestKind;
use wfdiag_native_remediation::broker::ActionRequest;
use wfdiag_native_settings::{AppSettings, ProviderCredentialTransaction, SettingsUpdate};

/// Which worker a command was routed to, for rejection reporting and for
/// [`crate::AppEvent::WorkerStopped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerKind {
    /// Diagnostic scan coordinator.
    Diagnostics,
    /// Issue detection.
    Issues,
    /// Scan history.
    History,
    /// Report/export rendering.
    Export,
    /// Host identity and architecture.
    System,
    /// Settings and credentials.
    Settings,
    /// AI provider management.
    Provider,
    /// GitHub update check.
    Update,
    /// Live system monitoring.
    Monitor,
}

impl WorkerKind {
    /// A stable lowercase name for logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Diagnostics => "diagnostics",
            Self::Issues => "issues",
            Self::History => "history",
            Self::Export => "export",
            Self::System => "system",
            Self::Settings => "settings",
            Self::Provider => "provider",
            Self::Update => "update",
            Self::Monitor => "monitor",
        }
    }
}

/// Why an update check is being requested.
pub use crate::domain::update::UpdateCheckReason;

/// Which subscription-CLI account operation to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionOperation {
    /// Read the account state without changing it.
    Status,
    /// Begin an interactive sign-in.
    SignIn,
    /// Sign out.
    SignOut,
}

/// A provider-credential mutation.
#[derive(Clone, Debug)]
pub enum ProviderCredentialCommand {
    /// Apply a staged transaction as one compensating unit.
    Commit(Box<ProviderCredentialTransaction>),
    /// Store one provider key immediately.
    Store {
        /// The provider whose key is being set, as a wire id.
        provider: String,
        /// The key itself. An empty value clears instead.
        key: String,
    },
    /// Clear one provider key immediately.
    Clear {
        /// The provider whose key is being removed, as a wire id.
        provider: String,
    },
}

/// Everything a host can ask the engine to do.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AppCommand {
    // ---- lifecycle -----------------------------------------------------
    /// The host is ready. Arms the startup-scan gate and schedules the
    /// passive update check.
    Start {
        /// Whether an automatic startup scan may run at all on this host.
        startup_scan: bool,
    },
    /// The window became visible or hidden. Hiding pauses live monitoring.
    WindowVisibility {
        /// Whether the window is visible.
        visible: bool,
    },
    /// Begin an orderly stop. Call [`crate::AppService::shutdown`] afterwards
    /// to join the workers within a budget.
    Shutdown,

    // ---- scanning ------------------------------------------------------
    /// Run a quick or full scan.
    StartScan {
        /// Which scan to run.
        kind: ScanKind,
    },
    /// Re-run exactly the listed tasks. A single task id re-runs that row in
    /// place inside the committed scan.
    StartTargetedScan {
        /// The tasks to run.
        task_ids: Vec<String>,
    },
    /// Stop the running scan. In-flight tasks finish; queued tasks skip.
    CancelScan,
    /// Re-evaluate issues against the currently committed evidence.
    RefreshIssues,

    // ---- history -------------------------------------------------------
    /// List stored scans, newest first.
    ListHistory,
    /// Load one stored scan in full.
    LoadHistoryScan {
        /// The scan to load.
        scan_id: String,
    },
    /// Compare two stored scans.
    CompareHistory {
        /// The newer scan.
        current_id: String,
        /// The baseline scan.
        previous_id: String,
        /// Return only the lightweight summary rows.
        summary_only: bool,
    },
    /// Compare the committed scan with the newest other stored scan.
    CompareCurrentToLatest,
    /// Fetch both outputs for one task across two stored scans.
    HistoryTaskDiff {
        /// The newer scan.
        current_id: String,
        /// The baseline scan.
        previous_id: String,
        /// The task to diff.
        task_id: String,
    },
    /// Save (or clear) a stored scan's label.
    SaveHistoryLabel {
        /// The scan to label.
        scan_id: String,
        /// The new label; `None` clears it.
        label: Option<String>,
    },
    /// Replace a stored scan's tags.
    SaveHistoryTags {
        /// The scan to tag.
        scan_id: String,
        /// The complete tag set.
        tags: Vec<String>,
    },
    /// Compute per-task failure trends over recent scans.
    HistoryTrends {
        /// How many scans to consider.
        limit: usize,
    },
    /// Delete every stored scan.
    ClearHistory,

    // ---- monitoring ----------------------------------------------------
    /// Request one immediate telemetry sample.
    MonitorRefresh,
    /// Pause or resume one-second sampling.
    SetMonitorPaused {
        /// Whether sampling is paused.
        paused: bool,
    },
    /// Request one page of the process explorer.
    RequestProcessPage(ProcessQuery),
    /// Request the current network-connection list.
    RequestNetworkConnections,

    // ---- providers -----------------------------------------------------
    /// Refresh AI provider availability.
    RequestProviderStatus,
    /// Apply an AI provider preference and refresh status atomically.
    SetProviderPreference {
        /// The preference wire id (`auto`, `openai`, `phi_silica`, …).
        preference: String,
    },
    /// Drop cached AI responses.
    ClearAiCache {
        /// Limit the drop to one chat session.
        session_id: Option<String>,
    },
    /// List the models the local Ollama server offers.
    ListOllamaModels,

    // ---- settings ------------------------------------------------------
    /// Reload the settings document and credential availability.
    LoadSettings,
    /// Persist a complete settings document.
    SaveSettings(Box<AppSettings>),
    /// Apply one typed mutation and persist it.
    UpdateSetting(SettingsUpdate),
    /// Store, clear, or commit provider credentials.
    ProviderCredential(ProviderCredentialCommand),

    // ---- export --------------------------------------------------------
    /// Render an export payload from the committed scan.
    ExportResults {
        /// Which payload to render.
        kind: Box<ExportRequestKind>,
    },

    // ---- system and updates -------------------------------------------
    /// Check for a newer public release.
    CheckForUpdates {
        /// Whether this is the passive startup check or a user request.
        reason: UpdateCheckReason,
    },
    /// Re-read host identity.
    RequestSystemInfo,
    /// Re-read CPU architecture.
    RequestArchitecture,
    /// Ask the host to relaunch with administrator rights.
    RestartAsAdmin,

    // ---- agentic chat ---------------------------------------------------
    /// Send a chat message to the AI assistant.
    ChatSend {
        /// The user's prompt.
        prompt: String,
    },
    /// Cancel the streaming chat turn.
    ChatCancel,
    /// Start a new chat session.
    ChatReset,
    /// Answer a local-to-cloud fallback prompt.
    CloudFallbackDecision {
        /// Whether the user allowed the cloud provider.
        allow: bool,
    },
    /// Generate the one-click AI scan report.
    GenerateReport {
        /// Ignore the cached report and regenerate.
        force_refresh: bool,
    },
    /// Cancel report generation.
    CancelReport,
    /// Explain one diagnostic task's output.
    AnalyzeDiagnostic {
        /// The task to explain.
        task_id: String,
        /// Bypass the response cache and ask the provider again.
        force_refresh: bool,
    },
    /// Cancel the running analysis.
    CancelAnalysis,
    /// Rank the detected issues with the AI provider.
    PrioritizeIssues {
        /// Bypass the response cache and ask the provider again.
        force_refresh: bool,
    },
    /// Ask the AI provider for a catalog-only fix plan.
    GenerateFixPlan,
    /// Cancel fix-plan generation.
    CancelFixPlan,
    /// Build an action proposal for one remediation.
    PrepareRemediation {
        /// The remediation catalog id.
        remediation_id: String,
        /// The detected issue authorising it. `None` is only valid for a
        /// maintenance action.
        issue_id: Option<String>,
    },
    /// Build one action proposal covering several remediations at once.
    ///
    /// This is the fix-plan "review these together" path: the plan already
    /// named catalog ids against a specific evidence and catalog fingerprint,
    /// and both are carried here so the broker refuses a preview whose
    /// evidence moved on while the user was reading the plan.
    PrepareRemediations {
        /// The catalog ids with their authorising issues, in plan order.
        actions: Vec<ActionRequest>,
        /// The evidence fingerprint the selection was made against.
        expected_scan_fingerprint: Option<String>,
        /// The catalog fingerprint the selection was made against.
        expected_catalog_fingerprint: Option<String>,
    },
    /// Approve a prepared action proposal.
    ///
    /// `confirm_repair` carries the **second**, repair-specific confirmation.
    /// The gate itself lives in the remediation broker: approving a Repair
    /// preview without it produces
    /// [`crate::ActionEvent::RepairConfirmationRequired`] and runs nothing.
    ApproveAction {
        /// The proposal to run.
        proposal_id: String,
        /// Whether the user gave the repair-specific confirmation.
        confirm_repair: bool,
    },
    /// Discard a prepared action proposal.
    DiscardProposal {
        /// The proposal to drop.
        proposal_id: String,
    },
    /// Cancel a running remediation.
    CancelAction {
        /// The run to stop.
        run_id: String,
    },
    /// Refresh a provider's model catalog.
    RefreshModelCatalog {
        /// The provider wire id.
        provider: String,
        /// The unsaved API key to discover with, when the user is typing one.
        draft_api_key: Option<String>,
        /// The unsaved endpoint to discover with.
        draft_endpoint: Option<String>,
        /// The unsaved CLI path to discover with.
        draft_cli_path: Option<String>,
        /// Whether this is an explicit Refresh, which is never debounced.
        forced: bool,
    },
    /// Cancel a model-catalog refresh.
    CancelModelCatalog,
    /// Probe, sign in to, or sign out of a subscription CLI.
    SubscriptionAuth {
        /// The provider wire id.
        provider: String,
        /// Which operation to run.
        operation: SubscriptionOperation,
    },
    /// Cancel subscription authentication.
    CancelSubscriptionAuth,
    /// Ask to install a subscription CLI. This only raises the confirmation:
    /// nothing is installed until [`AppCommand::ConfirmSubscriptionInstall`].
    InstallSubscriptionCli {
        /// The provider wire id.
        provider: String,
    },
    /// Answer the pending installation confirmation.
    ///
    /// The vendor PowerShell bootstrap raises a **second** confirmation of its
    /// own; accepting the first never implies the second.
    ConfirmSubscriptionInstall {
        /// Whether the user accepted.
        accepted: bool,
    },
    /// Cancel a subscription CLI installation.
    CancelSubscriptionInstall,
}

/// Why a dispatched command was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// The service is shutting down.
    Terminating,
    /// The owning worker never started, or has stopped.
    WorkerUnavailable {
        /// Which worker.
        worker: WorkerKind,
        /// The startup or stop diagnostic.
        detail: String,
    },
    /// The command conflicts with work already in progress.
    Busy {
        /// What is in progress.
        detail: String,
    },
    /// The command's arguments are not usable.
    Invalid {
        /// Why.
        detail: String,
    },
    /// A prerequisite has not finished (settings load, identity probe).
    NotReady {
        /// What is still pending.
        detail: String,
    },
    /// A request-identity counter is exhausted; the domain is shut down
    /// rather than risking a stale reply matching a fresh request.
    IdentityExhausted,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Terminating => formatter.write_str("the application service is shutting down"),
            Self::WorkerUnavailable { worker, detail } => {
                write!(
                    formatter,
                    "the {} worker is unavailable: {detail}",
                    worker.as_str()
                )
            }
            Self::Busy { detail } => write!(formatter, "busy: {detail}"),
            Self::Invalid { detail } => write!(formatter, "invalid request: {detail}"),
            Self::NotReady { detail } => write!(formatter, "not ready: {detail}"),
            Self::IdentityExhausted => {
                formatter.write_str("a request-identity counter is exhausted")
            }
        }
    }
}

impl std::error::Error for RejectReason {}

/// What [`crate::AppService::dispatch`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The command was accepted. `request` is present when a worker reply
    /// will follow; the host still never needs to compare it.
    Accepted {
        /// The request identity, for logging only.
        request: Option<RequestId>,
    },
    /// The command had nothing to do (already in that state).
    Ignored {
        /// Why nothing happened.
        detail: &'static str,
    },
    /// The command was refused.
    Rejected(RejectReason),
}

impl DispatchOutcome {
    /// An acceptance with no reply to await.
    #[must_use]
    pub const fn accepted() -> Self {
        Self::Accepted { request: None }
    }

    /// An acceptance whose worker reply will arrive later.
    #[must_use]
    pub const fn accepted_request(request: RequestId) -> Self {
        Self::Accepted {
            request: Some(request),
        }
    }

    /// Whether the command was accepted.
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    /// The rejection, if the command was refused.
    #[must_use]
    pub const fn rejection(&self) -> Option<&RejectReason> {
        match self {
            Self::Rejected(reason) => Some(reason),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatchOutcome, RejectReason, WorkerKind};

    #[test]
    fn rejections_render_the_owning_worker() {
        let reason = RejectReason::WorkerUnavailable {
            worker: WorkerKind::History,
            detail: "storage directory is unavailable".to_string(),
        };
        assert_eq!(
            reason.to_string(),
            "the history worker is unavailable: storage directory is unavailable"
        );
        assert!(!DispatchOutcome::Rejected(reason).is_accepted());
    }

    #[test]
    fn a_refused_command_is_a_typed_rejection_not_a_silent_noop() {
        let outcome = DispatchOutcome::Rejected(RejectReason::Busy {
            detail: "a chat turn is already streaming".to_string(),
        });
        assert!(!outcome.is_accepted());
        assert_eq!(
            outcome.rejection().map(ToString::to_string).as_deref(),
            Some("busy: a chat turn is already streaming")
        );
    }
}
