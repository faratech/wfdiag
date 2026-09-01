//! The application service: one command in, one event stream out.
//!
//! # Threading model
//!
//! The service is a **single-threaded core owned by the host**. It is `!Sync`
//! by construction (`&mut self` for [`AppService::dispatch`] and
//! [`AppService::drain`]) and must live on one thread — for a GUI shell, the UI
//! thread.
//!
//! * [`AppService::dispatch`] never blocks. It validates against the local
//!   state machines, hands work to a worker runtime or the scan executor, and
//!   returns a [`DispatchOutcome`] immediately.
//! * Workers wake the host through [`AppEventReceiver::set_wake_handler`]. The
//!   event buses and the settings runtime call it themselves; the crates that
//!   answer on a bare channel are covered by one
//!   [`ReplyWatcher`](crate::replies) thread that ticks only while something is
//!   outstanding.
//! * [`AppService::drain`] is the *only* place worker output is read, the only
//!   place [`AppSnapshot`] is mutated, and the only place staleness is judged.
//!   A host that receives an event never has to ask whether it is current.

mod ai;

use crate::command::{
    AppCommand, DispatchOutcome, ProviderCredentialCommand, RejectReason, UpdateCheckReason,
    WorkerKind,
};
use crate::config::AppConfig;
use crate::domain::actions::StagedReview;
use crate::domain::ai_intent::PendingAiIntent;
use crate::domain::catalog::RefreshThrottle;
use crate::domain::consent::{ChatAttempt, PendingConsent, PendingPolicyWrite};
use crate::domain::history::{
    RetentionPolicy, auto_save_allowed, build_scan_record, scan_concurrency, scan_kind_history_tag,
};
use crate::domain::invalidation::Invalidation;
use crate::domain::issues::IssueTracker;
use crate::domain::providers::PhiPreferenceGate;
use crate::domain::scan::{RunOutcome, ScanPhase, ScanPolicy, ScanState, select_scan_tasks};
use crate::domain::startup::{StartupReadiness, StartupScanGate};
use crate::domain::update::{START_DELAY, UpdateSchedule, schedule};
use crate::event::{
    AppEvent, AppEventReceiver, EventQueue, ExportEvent, HistoryEvent, HistoryRequest, IssuesEvent,
    MonitorEvent, ProviderEvent, ScanEvent, SettingsEvent as SettingsFact, SystemEvent,
    UpdateEvent,
};
use crate::ids::{Generation, Generations, RequestId, RequestIds};
use crate::ports::AppPorts;
use crate::ports::monitor::{NetworkConnection, ProcessQuery, ProcessQueryOutcome};
use crate::replies::{PendingReplies, ReplyFailure, ReplyWatcher};
use crate::snapshot::AppSnapshot;
use crate::workers::{AppWorkers, WorkerStopRecord};
use ai::{CatalogDraft, PendingAnalysis, PendingSubscriptionAuth, PendingSubscriptionInstall};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use wfdiag_native_ai_provider::AIProviderStatus;
use wfdiag_native_diagnostics::{DiagnosticRuntime, ScanKind};
use wfdiag_native_export::{ExportRequest, ExportRequestKind};
use wfdiag_native_history::{
    ComparisonResult, ComparisonSummary, ScanRecord, ScanSummary, TaskDiffDetail, TaskTrend,
};
use wfdiag_native_issues::SharedScanEvidence;
use wfdiag_native_settings::{
    AppSettings, ProviderKeyId, SettingsCommand, SettingsEvent, SettingsService, SettingsUpdate,
};
use wfdiag_native_system::{SystemPayload, SystemRequest, SystemRequestKind};
use wfdiag_native_update::UpdateOutcome;
use wfdiag_ui_core::UiEvent;

/// How many events one drain accepts from a single unbuffered channel before
/// yielding, so a saturated worker cannot starve the host.
const CHANNEL_DRAIN_LIMIT: usize = 256;

/// Why the service could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppStartError {
    /// The scan executor's Tokio runtime could not be created. Without it no
    /// scan can ever run, so this is the one fatal startup failure.
    Executor(String),
}

impl std::fmt::Display for AppStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Executor(reason) => {
                write!(formatter, "the scan executor could not start: {reason}")
            }
        }
    }
}

impl std::error::Error for AppStartError {}

/// What teardown achieved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Per-worker stop results, in teardown order.
    pub workers: Vec<WorkerStopRecord>,
    /// Whether the reply watcher exited inside its budget.
    pub watcher_stopped: bool,
    /// How long teardown took.
    pub elapsed: Duration,
}

impl ShutdownReport {
    /// Whether every worker stopped inside its budget.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.watcher_stopped
            && self
                .workers
                .iter()
                .all(|record| record.stopped_within_budget)
    }
}

/// What one history reply carries.
enum HistoryPayload {
    Unit(Result<(), String>),
    Summaries(Result<Vec<ScanSummary>, String>),
    Record(Result<Box<ScanRecord>, String>),
    Comparison(Result<Box<ComparisonResult>, String>),
    ComparisonSummary(Result<Box<ComparisonSummary>, String>),
    OptionalComparison(Result<Option<Box<ComparisonResult>>, String>),
    TaskDiff(Result<Box<TaskDiffDetail>, String>),
    Trends(Result<Vec<TaskTrend>, String>),
}

/// A message produced by a worker reply or a background task, before any
/// staleness guard has run.
enum Internal {
    ScanStarted {
        session_id: String,
        kind: ScanKind,
        total: usize,
    },
    ScanStartFailed {
        error: String,
    },
    ScanFinished {
        session_id: String,
        cancelled: bool,
        evidence: Result<SharedScanEvidence, String>,
    },
    ScanCancelFinished {
        session_id: String,
        error: Option<String>,
    },
    Elevation(Result<bool, String>),
    History {
        request: RequestId,
        kind: HistoryRequest,
        /// The scan a label/tag write targeted, echoed back for the host.
        scan_id: Option<String>,
        payload: HistoryPayload,
    },
    HistoryAutoSave {
        session_id: String,
        result: Result<(), String>,
    },
    Update {
        request: RequestId,
        outcome: Result<UpdateOutcome, String>,
    },
    ProviderStatus {
        request: RequestId,
        status: Result<Box<AIProviderStatus>, String>,
    },
    ProviderPreference {
        request: RequestId,
        preference: String,
        status: Result<Box<AIProviderStatus>, String>,
    },
    ProviderCacheCleared {
        request: RequestId,
    },
    ProviderOllamaModels {
        request: RequestId,
        models: Result<Vec<String>, String>,
    },
    /// The report's optional history baseline finished resolving.
    ReportBaseline {
        request: RequestId,
        generation: Box<wfdiag_native_ai_report::ReportGeneration>,
        comparison: Option<Box<ComparisonResult>>,
    },
    ProcessPage {
        request: RequestId,
        outcome: Result<ProcessQueryOutcome, String>,
    },
    NetworkConnections {
        request: RequestId,
        connections: Result<Vec<NetworkConnection>, String>,
    },
}

fn failure_text(failure: ReplyFailure) -> String {
    failure.to_string()
}

/// What a settings request was for, so its reply can be interpreted.
enum SettingsRequestKind {
    Load,
    Save(Box<AppSettings>),
    Update,
    Credential,
}

/// The application service.
// The flags are independent lifecycle facts owned by different domains
// (started, terminating, one outstanding issue request); merging them would
// couple domains that have nothing to do with each other.
#[allow(clippy::struct_excessive_bools)]
pub struct AppService {
    config: AppConfig,
    ports: AppPorts,
    settings_service: SettingsService,
    workers: AppWorkers,
    queue: Arc<EventQueue>,
    watcher: ReplyWatcher,
    replies: PendingReplies<Internal>,
    executor: Option<tokio::runtime::Runtime>,
    internal_tx: mpsc::Sender<Internal>,
    internal_rx: mpsc::Receiver<Internal>,
    snapshot: AppSnapshot,
    scan: ScanState,
    issues: IssueTracker,
    startup_gate: StartupScanGate,
    startup_scan_allowed: bool,
    requests: RequestIds,
    system_info_request: Option<RequestId>,
    architecture_request: Option<RequestId>,
    settings_requests: HashMap<u64, SettingsRequestKind>,
    export_requests: Vec<u64>,
    history_latest: HashMap<HistoryRequest, RequestId>,
    provider_status_request: Option<RequestId>,
    update_request: Option<RequestId>,
    process_page_request: Option<RequestId>,
    issue_outstanding: bool,
    update_startup_due: Option<Instant>,
    started: bool,
    terminating: bool,

    // ---- AI, remediation, and provider setup ---------------------------
    /// Versions the committed evidence a proposal or a ranking is bound to.
    evidence_generations: Generations,
    evidence_generation: Generation,
    chat_turns: RequestIds,
    chat_turn: Option<RequestId>,
    chat_pending: Option<RequestId>,
    chat_attempt: Option<ChatAttempt>,
    chat_consent: Option<PendingConsent>,
    chat_policy_write: Option<(RequestId, PendingPolicyWrite)>,
    pending_intent: Option<PendingAiIntent>,
    report_pending: Option<RequestId>,
    analysis_pending: Option<PendingAnalysis>,
    prioritization_pending: Option<(RequestId, Generation)>,
    fix_plan_pending: Option<RequestId>,
    action_pending: Option<RequestId>,
    action_pending_review: Option<StagedReview>,
    catalog_pending: Option<(RequestId, String)>,
    catalog_throttle: RefreshThrottle,
    catalog_retry: Option<(String, CatalogDraft)>,
    subscription_auth_pending: Option<PendingSubscriptionAuth>,
    subscription_install_pending: Option<PendingSubscriptionInstall>,
}

impl std::fmt::Debug for AppService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppService")
            .field("phase", &self.snapshot.scan_phase)
            .field("terminating", &self.terminating)
            .finish_non_exhaustive()
    }
}

impl AppService {
    /// Start every worker and return the service plus the host's event handle.
    ///
    /// The settings document is loaded **synchronously** here so the returned
    /// snapshot already carries the persisted theme; a shell can paint its
    /// first frame correctly instead of flashing the default one.
    ///
    /// # Errors
    ///
    /// Returns [`AppStartError::Executor`] when the scan executor's runtime
    /// cannot be created. Every other worker failure is recorded in the
    /// snapshot and rejected per command instead of failing the whole start.
    pub fn start(
        config: AppConfig,
        ports: AppPorts,
    ) -> Result<(Self, AppEventReceiver), AppStartError> {
        let queue = EventQueue::new(config.event_capacity);
        let receiver = AppEventReceiver::new(Arc::clone(&queue));

        let settings_service = SettingsService::new(
            Arc::clone(&ports.settings_storage),
            Arc::clone(&ports.credentials),
            Arc::clone(&ports.settings_validator),
        );
        let mut snapshot = AppSnapshot::default();
        match settings_service.load() {
            Ok(settings) => snapshot.settings = settings,
            Err(error) => {
                snapshot.settings_error = Some(error.to_string());
                snapshot.record_worker_error(WorkerKind::Settings, error.to_string());
            }
        }

        let executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(config.executor_threads.max(1))
            .thread_name("wfdiag-diagnostic")
            .enable_all()
            .build()
            .map_err(|error| AppStartError::Executor(error.to_string()))?;

        let retention = RetentionPolicy::from(&snapshot.settings);
        let (workers, failures) =
            AppWorkers::start(&config, &ports, &settings_service, retention, &queue);
        for failure in failures {
            snapshot.record_worker_error(failure.worker, failure.detail);
        }
        snapshot.catalog.clone_from(&workers.catalog);
        snapshot.remediations.clone_from(&workers.remediations);
        snapshot.monitor.available = workers.monitor.is_some();
        snapshot.window_visible = true;

        let watcher = ReplyWatcher::start(&queue, config.reply_poll_interval);
        let (internal_tx, internal_rx) = mpsc::channel();
        let mut service = Self {
            replies: PendingReplies::new(config.reply_timeout),
            config,
            ports,
            settings_service,
            workers,
            queue,
            watcher,
            executor: Some(executor),
            internal_tx,
            internal_rx,
            snapshot,
            scan: ScanState::new(),
            issues: IssueTracker::new(),
            startup_gate: StartupScanGate::default(),
            startup_scan_allowed: false,
            requests: RequestIds::new(),
            system_info_request: None,
            architecture_request: None,
            settings_requests: HashMap::new(),
            export_requests: Vec::new(),
            history_latest: HashMap::new(),
            provider_status_request: None,
            update_request: None,
            process_page_request: None,
            issue_outstanding: false,
            update_startup_due: None,
            started: false,
            terminating: false,
            evidence_generations: Generations::new(),
            evidence_generation: Generation::from_raw(0),
            chat_turns: RequestIds::new(),
            chat_turn: None,
            chat_pending: None,
            chat_attempt: None,
            chat_consent: None,
            chat_policy_write: None,
            pending_intent: None,
            report_pending: None,
            analysis_pending: None,
            prioritization_pending: None,
            fix_plan_pending: None,
            action_pending: None,
            action_pending_review: None,
            catalog_pending: None,
            catalog_throttle: RefreshThrottle::new(),
            catalog_retry: None,
            subscription_auth_pending: None,
            subscription_install_pending: None,
        };
        service.adopt_rehydrated_actions();
        service.record_ai_worker_errors();
        service.request_system_identity();
        Ok((service, receiver))
    }

    /// The read model. Valid immediately after [`Self::drain`].
    #[must_use]
    pub const fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
    }

    /// A second handle on the event stream, for another thread's wake wiring.
    #[must_use]
    pub fn events(&self) -> AppEventReceiver {
        AppEventReceiver::new(Arc::clone(&self.queue))
    }

    /// Route one command. Never blocks.
    #[allow(clippy::too_many_lines)]
    pub fn dispatch(&mut self, command: AppCommand) -> DispatchOutcome {
        if self.terminating && !matches!(command, AppCommand::Shutdown) {
            return DispatchOutcome::Rejected(RejectReason::Terminating);
        }
        let outcome = match command {
            AppCommand::Start { startup_scan } => self.start_host(startup_scan),
            AppCommand::WindowVisibility { visible } => self.set_window_visible(visible),
            AppCommand::Shutdown => self.begin_shutdown(),

            AppCommand::StartScan { kind } => self.start_scan(kind, None),
            AppCommand::StartTargetedScan { task_ids } => {
                self.start_scan(ScanKind::Targeted, Some(task_ids))
            }
            AppCommand::CancelScan => self.cancel_scan(),
            AppCommand::RefreshIssues => self.refresh_issues(),

            AppCommand::ListHistory => self.history_request(HistoryRequest::List),
            AppCommand::LoadHistoryScan { scan_id } => self.history_load(scan_id),
            AppCommand::CompareHistory {
                current_id,
                previous_id,
                summary_only,
            } => self.history_compare(current_id, previous_id, summary_only),
            AppCommand::CompareCurrentToLatest => self.history_compare_to_latest(),
            AppCommand::HistoryTaskDiff {
                current_id,
                previous_id,
                task_id,
            } => self.history_task_diff(current_id, previous_id, task_id),
            AppCommand::SaveHistoryLabel { scan_id, label } => self.history_label(scan_id, label),
            AppCommand::SaveHistoryTags { scan_id, tags } => self.history_tags(scan_id, tags),
            AppCommand::HistoryTrends { limit } => self.history_trends(limit),
            AppCommand::ClearHistory => self.history_clear(),

            AppCommand::MonitorRefresh => self.monitor_refresh(),
            AppCommand::SetMonitorPaused { paused } => self.set_monitor_paused(paused),
            AppCommand::RequestProcessPage(query) => self.request_process_page(query),
            AppCommand::RequestNetworkConnections => self.request_network_connections(),

            AppCommand::RequestProviderStatus => self.request_provider_status(),
            AppCommand::SetProviderPreference { preference } => {
                self.set_provider_preference(preference)
            }
            AppCommand::ClearAiCache { session_id } => self.clear_ai_cache(session_id),
            AppCommand::ListOllamaModels => self.list_ollama_models(),

            AppCommand::LoadSettings => self.settings_load(),
            AppCommand::SaveSettings(settings) => self.settings_save(settings),
            AppCommand::UpdateSetting(update) => self.settings_update(update),
            AppCommand::ProviderCredential(action) => self.settings_credential(action),

            AppCommand::ExportResults { kind } => self.export_results(*kind),

            AppCommand::CheckForUpdates { reason } => self.check_for_updates(reason),
            AppCommand::RequestSystemInfo => self.request_system(SystemRequestKind::SystemInfo),
            AppCommand::RequestArchitecture => self.request_system(SystemRequestKind::Architecture),
            AppCommand::RestartAsAdmin => self.restart_as_admin(),

            // ---- AI, remediation, and provider setup ----------------------
            AppCommand::ChatSend { prompt } => self.chat_send(&prompt),
            AppCommand::ChatCancel => self.chat_cancel(),
            AppCommand::ChatReset => self.chat_reset(),
            AppCommand::CloudFallbackDecision { allow } => self.cloud_fallback_decision(allow),
            AppCommand::GenerateReport { force_refresh } => self.generate_report(force_refresh),
            AppCommand::CancelReport => self.cancel_report(),
            AppCommand::AnalyzeDiagnostic {
                task_id,
                force_refresh,
            } => self.analyze_diagnostic(task_id, force_refresh),
            AppCommand::CancelAnalysis => self.cancel_analysis(),
            AppCommand::PrioritizeIssues { force_refresh } => self.prioritize_issues(force_refresh),
            AppCommand::GenerateFixPlan => self.generate_fix_plan(),
            AppCommand::CancelFixPlan => self.cancel_fix_plan(),
            AppCommand::PrepareRemediation {
                remediation_id,
                issue_id,
            } => self.prepare_remediation(remediation_id, issue_id),
            AppCommand::PrepareRemediations {
                actions,
                expected_scan_fingerprint,
                expected_catalog_fingerprint,
            } => self.prepare_remediations(
                actions,
                expected_scan_fingerprint,
                expected_catalog_fingerprint,
            ),
            AppCommand::ApproveAction {
                proposal_id,
                confirm_repair,
            } => self.approve_action(&proposal_id, confirm_repair),
            AppCommand::DiscardProposal { proposal_id } => self.discard_proposal(&proposal_id),
            AppCommand::CancelAction { run_id } => self.cancel_action(&run_id),
            AppCommand::RefreshModelCatalog {
                provider,
                draft_api_key,
                draft_endpoint,
                draft_cli_path,
                forced,
            } => self.refresh_model_catalog(
                &provider,
                CatalogDraft {
                    api_key: draft_api_key,
                    endpoint: draft_endpoint,
                    cli_path: draft_cli_path,
                },
                forced,
            ),
            AppCommand::CancelModelCatalog => self.cancel_model_catalog(),
            AppCommand::SubscriptionAuth {
                provider,
                operation,
            } => self.subscription_auth(&provider, operation),
            AppCommand::CancelSubscriptionAuth => self.cancel_subscription_auth(),
            AppCommand::InstallSubscriptionCli { provider } => {
                self.install_subscription_cli(&provider)
            }
            AppCommand::ConfirmSubscriptionInstall { accepted } => {
                self.confirm_subscription_install(accepted)
            }
            AppCommand::CancelSubscriptionInstall => self.cancel_subscription_install(),
        };
        self.publish_pending_work();
        outcome
    }

    /// Read every worker, apply every staleness guard, update the snapshot,
    /// and return the resulting events.
    pub fn drain(&mut self) -> Vec<AppEvent> {
        self.drain_internal();
        self.drain_diagnostic_events();
        self.drain_monitor_events();
        self.drain_settings_events();
        self.drain_issue_replies();
        self.drain_export_replies();
        self.drain_system_replies();
        self.drain_ai_events();
        self.poll_replies();
        // A reply handled above can queue more internal work (a finished scan
        // starts its history save); take that in the same batch.
        self.drain_internal();
        self.maybe_start_delayed_update_check();
        self.maybe_start_startup_scan();
        self.snapshot.scan_phase = self.scan.phase();
        self.snapshot.scan = self.scan.snapshot().clone();
        self.resume_pending_intent();
        self.publish_pending_work();
        self.queue.take()
    }

    /// Stop every worker in dependency order within `budget` per worker.
    #[must_use]
    pub fn shutdown(mut self, budget: Duration) -> ShutdownReport {
        let started = Instant::now();
        self.terminating = true;
        self.workers.stop_ai();
        for message in self.replies.drain_as_stopped() {
            self.apply_internal(message);
        }
        let workers = self.workers.stop(budget);
        let watcher_stopped = self.watcher.stop(budget);
        if let Some(executor) = self.executor.take() {
            executor.shutdown_timeout(budget);
        }
        self.queue.push(AppEvent::Terminated);
        self.queue.wake();
        ShutdownReport {
            workers,
            watcher_stopped,
            elapsed: started.elapsed(),
        }
    }

    // ---- lifecycle -----------------------------------------------------

    fn start_host(&mut self, startup_scan: bool) -> DispatchOutcome {
        if self.started {
            return DispatchOutcome::Ignored {
                detail: "the host already started",
            };
        }
        self.started = true;
        self.startup_scan_allowed = startup_scan;
        if !startup_scan {
            self.startup_gate.consume();
        }
        self.update_startup_due = Some(Instant::now() + START_DELAY);
        self.settings_load();
        self.snapshot.scan_phase = self.scan.phase();
        self.queue.push(AppEvent::Started {
            snapshot: Box::new(self.snapshot.clone()),
        });
        DispatchOutcome::accepted()
    }

    fn set_window_visible(&mut self, visible: bool) -> DispatchOutcome {
        self.snapshot.window_visible = visible;
        // Hiding the window stops one-second sampling; it is the single
        // biggest idle cost the shell has. A host without live monitoring
        // still gets an acceptance: the visibility itself was recorded.
        let _ = self.set_monitor_paused(!visible);
        DispatchOutcome::accepted()
    }

    fn begin_shutdown(&mut self) -> DispatchOutcome {
        if self.terminating {
            return DispatchOutcome::Ignored {
                detail: "shutdown already began",
            };
        }
        self.terminating = true;
        self.snapshot.terminating = true;
        if self.scan.is_busy() {
            let _ = self.cancel_scan();
        }
        DispatchOutcome::accepted()
    }

    // ---- scanning ------------------------------------------------------

    fn diagnostics(&self) -> Result<DiagnosticRuntime, RejectReason> {
        self.workers
            .diagnostics
            .clone()
            .ok_or_else(|| RejectReason::WorkerUnavailable {
                worker: WorkerKind::Diagnostics,
                detail: self
                    .snapshot
                    .worker_error(WorkerKind::Diagnostics)
                    .unwrap_or("native diagnostics are unavailable")
                    .to_string(),
            })
    }

    fn start_scan(&mut self, kind: ScanKind, task_ids: Option<Vec<String>>) -> DispatchOutcome {
        if self.scan.is_busy() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a diagnostic scan is already running".to_string(),
            });
        }
        if self.snapshot.settings_loading {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "scan settings are still loading".to_string(),
            });
        }
        if self.system_info_request.is_some() {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "administrator access is still being detected".to_string(),
            });
        }
        let runtime = match self.diagnostics() {
            Ok(runtime) => runtime,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let task_ids = match task_ids {
            Some(requested) => {
                let available: Vec<String> = requested
                    .into_iter()
                    .filter(|task_id| {
                        self.snapshot.catalog.iter().any(|task| {
                            &task.id == task_id
                                && (self.snapshot.is_admin() || !task.admin_required)
                        })
                    })
                    .collect();
                available
            }
            None => select_scan_tasks(
                &self.snapshot.catalog,
                kind,
                self.snapshot.is_admin(),
                self.snapshot.settings.quick_scan_tasks.as_deref(),
            ),
        };
        if task_ids.is_empty() {
            return DispatchOutcome::Rejected(RejectReason::Invalid {
                detail: "no runnable diagnostic tasks were selected".to_string(),
            });
        }

        let targeted_rerun = kind == ScanKind::Targeted
            && task_ids.len() == 1
            && self.scan.snapshot().session_id.is_some();
        let policy = ScanPolicy {
            auto_save: auto_save_allowed(self.snapshot.settings.auto_save, targeted_rerun),
            max_concurrent_tasks: scan_concurrency(self.snapshot.settings.max_concurrent_tasks),
            history_tag: scan_kind_history_tag(kind).to_string(),
        };
        let invalidates = self.scan.begin(kind, task_ids.clone(), policy);
        let invalidation = Invalidation::on_scan_start(!invalidates);
        self.snapshot.derived_invalidated = invalidation;
        // A replacement scan invalidates every derived projection the moment
        // its transaction opens; a targeted rerun defers until it commits, so
        // every failure path leaves the previous evidence and everything
        // derived from it intact.
        self.apply_derived_invalidation(invalidation);

        let total = task_ids.len();
        let sender = self.internal_tx.clone();
        let queue = Arc::clone(&self.queue);
        self.spawn(async move {
            let message = match runtime.start_session(task_ids, kind).await {
                Ok(session_id) => Internal::ScanStarted {
                    session_id,
                    kind,
                    total,
                },
                Err(error) => Internal::ScanStartFailed {
                    error: error.to_string(),
                },
            };
            let _ = sender.send(message);
            queue.wake();
        });
        DispatchOutcome::accepted()
    }

    fn launch_scan_run(&mut self, session_id: String) {
        let Ok(runtime) = self.diagnostics() else {
            self.scan.start_failed();
            self.queue.push(AppEvent::Scan(ScanEvent::StartFailed {
                error: "native diagnostics stopped before the scan began".to_string(),
            }));
            return;
        };
        let max_concurrent = self
            .scan
            .policy()
            .map_or(5, |policy| policy.max_concurrent_tasks);
        let sender = self.internal_tx.clone();
        let queue = Arc::clone(&self.queue);
        let fallback_session = session_id.clone();
        self.spawn(async move {
            let message = match runtime.run_session(session_id, max_concurrent).await {
                Ok(result) if result.cancelled => Internal::ScanFinished {
                    session_id: result.session_id,
                    cancelled: true,
                    evidence: Err("scan was cancelled".to_string()),
                },
                Ok(result) => Internal::ScanFinished {
                    session_id: result.session_id,
                    cancelled: false,
                    evidence: Ok(result.evidence),
                },
                Err(error) => Internal::ScanFinished {
                    session_id: fallback_session,
                    cancelled: false,
                    evidence: Err(error.to_string()),
                },
            };
            let _ = sender.send(message);
            queue.wake();
        });
    }

    fn cancel_scan(&mut self) -> DispatchOutcome {
        if !self.scan.is_busy() {
            return DispatchOutcome::Ignored {
                detail: "no scan is running",
            };
        }
        if !self.scan.request_cancel() {
            // Starting or already cancelling: the intent is recorded and acted
            // on as soon as the session exists.
            return DispatchOutcome::accepted();
        }
        let Some(session_id) = self.scan.snapshot().session_id.clone() else {
            return DispatchOutcome::Rejected(RejectReason::Invalid {
                detail: "the active diagnostic session could not be identified".to_string(),
            });
        };
        let Ok(runtime) = self.diagnostics() else {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Diagnostics,
                detail: "native diagnostics are unavailable".to_string(),
            });
        };
        let sender = self.internal_tx.clone();
        let queue = Arc::clone(&self.queue);
        self.spawn(async move {
            let error = runtime
                .cancel_session(&session_id)
                .await
                .err()
                .map(|error| error.to_string());
            let _ = sender.send(Internal::ScanCancelFinished { session_id, error });
            queue.wake();
        });
        DispatchOutcome::accepted()
    }

    fn commit_scan_evidence(&mut self, session_id: String, evidence: SharedScanEvidence) {
        self.snapshot.derived_invalidated = Invalidation::on_targeted_commit();
        self.apply_derived_invalidation(Invalidation::on_targeted_commit());
        self.advance_evidence_generation();
        if self.issues.commit_evidence(session_id, evidence).is_err() {
            self.stop_issue_delivery("native issue evidence identity was exhausted");
            return;
        }
        self.enqueue_issue_detection();
    }

    fn enqueue_issue_detection(&mut self) {
        let Some(runtime) = self.workers.issues.as_ref() else {
            let detail = self
                .snapshot
                .worker_error(WorkerKind::Issues)
                .unwrap_or("native issue detection is unavailable")
                .to_string();
            self.snapshot.issue_error = Some(detail.clone());
            self.queue
                .push(AppEvent::Issues(IssuesEvent::Failed { error: detail }));
            return;
        };
        let now = self.ports.environment.now();
        let temp_file_count = self.ports.environment.temp_file_count();
        let request = match self.issues.prepare(now, temp_file_count) {
            Ok(request) => request,
            Err(refusal) => {
                self.snapshot.issue_error = Some(format!("{refusal:?}"));
                return;
            }
        };
        if let Err(error) = runtime.enqueue(request) {
            self.stop_issue_delivery(format!("native issue detection stopped · {error}"));
            return;
        }
        self.issue_outstanding = true;
        self.snapshot.issue_error = None;
    }

    fn stop_issue_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.issues.abandon();
        self.issue_outstanding = false;
        self.workers.issues = None;
        self.workers.issue_replies = None;
        self.snapshot.issue_error = Some(reason.clone());
        self.snapshot
            .record_worker_error(WorkerKind::Issues, reason.clone());
        self.queue
            .push(AppEvent::Issues(IssuesEvent::Failed { error: reason }));
    }

    fn refresh_issues(&mut self) -> DispatchOutcome {
        if self.issues.committed_session_id().is_none() {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "no scan evidence has been committed yet".to_string(),
            });
        }
        if self.workers.issues.is_none() {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Issues,
                detail: "native issue detection is unavailable".to_string(),
            });
        }
        self.enqueue_issue_detection();
        DispatchOutcome::accepted()
    }

    fn begin_history_auto_save(&mut self, session_id: String) {
        let Some(history) = self.workers.history.as_ref().map(Arc::clone) else {
            self.finish_scan(
                session_id,
                Some(Err("native scan history is unavailable".to_string())),
            );
            return;
        };
        let Some(tag) = self.scan.policy().map(|policy| policy.history_tag.clone()) else {
            self.finish_scan(
                session_id,
                Some(Err(
                    "the scan-start history policy was unavailable".to_string()
                )),
            );
            return;
        };
        let identity =
            self.snapshot
                .system_info
                .clone()
                .unwrap_or(wfdiag_native_system::SystemInfo {
                    computer_name: "Unknown".to_string(),
                    os_version: "Unknown".to_string(),
                    is_admin: false,
                });
        let record = build_scan_record(
            session_id.clone(),
            &identity,
            &self.scan.snapshot().results,
            self.scan.snapshot().duration_ms,
            tag,
            self.ports.environment.now(),
        );
        match history.request_save(record) {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    self.finish_scan(
                        session_id,
                        Some(Err("request identity exhausted".to_string())),
                    );
                    return;
                };
                let saved_session = session_id;
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::HistoryAutoSave {
                            session_id: saved_session,
                            result: match result {
                                Ok(inner) => inner,
                                Err(failure) => Err(failure_text(failure)),
                            },
                        }
                    });
            }
            Err(error) => self.finish_scan(session_id, Some(Err(error.to_string()))),
        }
    }

    fn finish_scan(&mut self, session_id: String, history: Option<Result<(), String>>) {
        if self.scan.finish_finalization(&session_id) {
            self.queue.push(AppEvent::Scan(ScanEvent::Finalized {
                session_id,
                history,
            }));
        }
    }

    // ---- history -------------------------------------------------------

    fn history(&self) -> Result<Arc<wfdiag_native_history::NativeHistoryRuntime>, RejectReason> {
        self.workers
            .history
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: self
                    .snapshot
                    .worker_error(WorkerKind::History)
                    .unwrap_or("native history is unavailable")
                    .to_string(),
            })
    }

    fn next_history_request(&mut self, kind: HistoryRequest) -> Option<RequestId> {
        let request = self.requests.issue()?;
        self.history_latest.insert(kind, request);
        Some(request)
    }

    fn history_request(&mut self, kind: HistoryRequest) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(kind) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let reply = match kind {
            HistoryRequest::List => history.request_list(),
            _ => {
                return DispatchOutcome::Rejected(RejectReason::Invalid {
                    detail: "unsupported history request".to_string(),
                });
            }
        };
        match reply {
            Ok(reply) => {
                self.snapshot.history.loading = true;
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind,
                            scan_id: None,
                            payload: HistoryPayload::Summaries(flatten(result)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_load(&mut self, scan_id: String) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Load) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match history.request_load(scan_id) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::Load,
                            scan_id: None,
                            payload: HistoryPayload::Record(flatten(result).map(Box::new)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_compare(
        &mut self,
        current_id: String,
        previous_id: String,
        summary_only: bool,
    ) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Compare) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        self.snapshot.history.loading = true;
        let queued = if summary_only {
            history
                .request_compare_summary(current_id, previous_id)
                .map(|reply| {
                    self.replies
                        .register(WorkerKind::History, request, reply, move |result| {
                            Internal::History {
                                request,
                                kind: HistoryRequest::Compare,
                                scan_id: None,
                                payload: HistoryPayload::ComparisonSummary(
                                    flatten(result).map(Box::new),
                                ),
                            }
                        });
                })
        } else {
            history
                .request_compare(current_id, previous_id)
                .map(|reply| {
                    self.replies
                        .register(WorkerKind::History, request, reply, move |result| {
                            Internal::History {
                                request,
                                kind: HistoryRequest::Compare,
                                scan_id: None,
                                payload: HistoryPayload::Comparison(flatten(result).map(Box::new)),
                            }
                        });
                })
        };
        match queued {
            Ok(()) => DispatchOutcome::accepted_request(request),
            Err(error) => {
                self.snapshot.history.loading = false;
                DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                    worker: WorkerKind::History,
                    detail: error.to_string(),
                })
            }
        }
    }

    fn history_compare_to_latest(&mut self) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(session_id) = self.scan.snapshot().effective_session_id() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "no scan has been committed yet".to_string(),
            });
        };
        let identity =
            self.snapshot
                .system_info
                .clone()
                .unwrap_or(wfdiag_native_system::SystemInfo {
                    computer_name: "Unknown".to_string(),
                    os_version: "Unknown".to_string(),
                    is_admin: false,
                });
        let record = build_scan_record(
            session_id,
            &identity,
            &self.scan.snapshot().results,
            self.scan.snapshot().duration_ms,
            self.scan.snapshot().scan_kind.map_or_else(
                || "Scan".to_string(),
                |kind| scan_kind_history_tag(kind).to_string(),
            ),
            self.ports.environment.now(),
        );
        let Some(request) = self.next_history_request(HistoryRequest::CompareToLatest) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match history.request_compare_current_to_latest(Arc::new(record)) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::CompareToLatest,
                            scan_id: None,
                            payload: HistoryPayload::OptionalComparison(
                                flatten(result).map(|value| value.map(Box::new)),
                            ),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_task_diff(
        &mut self,
        current_id: String,
        previous_id: String,
        task_id: String,
    ) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::TaskDiff) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match history.request_task_diff(current_id, previous_id, task_id) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::TaskDiff,
                            scan_id: None,
                            payload: HistoryPayload::TaskDiff(flatten(result).map(Box::new)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_label(&mut self, scan_id: String, label: Option<String>) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Label) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let saved_id = scan_id.clone();
        match history.request_update_label(scan_id, label) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::Label,
                            scan_id: Some(saved_id),
                            payload: HistoryPayload::Unit(flatten(result)),
                        }
                    });
                self.snapshot.history.error = None;
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_tags(&mut self, scan_id: String, tags: Vec<String>) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Tags) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let saved_id = scan_id.clone();
        match history.request_update_tags(scan_id, tags) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::Tags,
                            scan_id: Some(saved_id),
                            payload: HistoryPayload::Unit(flatten(result)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_trends(&mut self, limit: usize) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Trends) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match history.request_trends(limit) {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::Trends,
                            scan_id: None,
                            payload: HistoryPayload::Trends(flatten(result)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    fn history_clear(&mut self) -> DispatchOutcome {
        let history = match self.history() {
            Ok(history) => history,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(request) = self.next_history_request(HistoryRequest::Clear) else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match history.request_clear() {
            Ok(reply) => {
                self.replies
                    .register(WorkerKind::History, request, reply, move |result| {
                        Internal::History {
                            request,
                            kind: HistoryRequest::Clear,
                            scan_id: None,
                            payload: HistoryPayload::Unit(flatten(result)),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::History,
                detail: error.to_string(),
            }),
        }
    }

    // ---- monitoring ----------------------------------------------------

    fn monitor_refresh(&mut self) -> DispatchOutcome {
        match self.workers.monitor.as_ref() {
            Some(handle) if handle.refresh() => DispatchOutcome::accepted(),
            Some(_) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Monitor,
                detail: "the monitor worker stopped".to_string(),
            }),
            None => DispatchOutcome::Rejected(self.monitor_unavailable()),
        }
    }

    fn monitor_unavailable(&self) -> RejectReason {
        RejectReason::WorkerUnavailable {
            worker: WorkerKind::Monitor,
            detail: self
                .snapshot
                .worker_error(WorkerKind::Monitor)
                .unwrap_or("live monitoring is not running")
                .to_string(),
        }
    }

    fn set_monitor_paused(&mut self, paused: bool) -> DispatchOutcome {
        let Some(handle) = self.workers.monitor.as_ref() else {
            return DispatchOutcome::Ignored {
                detail: "live monitoring is not running",
            };
        };
        if self.snapshot.monitor.paused == paused {
            return DispatchOutcome::Ignored {
                detail: "the monitor is already in that state",
            };
        }
        let applied = if paused {
            handle.pause()
        } else {
            handle.resume()
        };
        if !applied {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Monitor,
                detail: "the monitor worker stopped".to_string(),
            });
        }
        self.snapshot.monitor.paused = paused;
        self.queue
            .push(AppEvent::Monitor(MonitorEvent::PausedChanged { paused }));
        DispatchOutcome::accepted()
    }

    fn request_process_page(&mut self, query: ProcessQuery) -> DispatchOutcome {
        let Some(handle) = self.workers.monitor.as_ref() else {
            return DispatchOutcome::Rejected(self.monitor_unavailable());
        };
        match handle.request_processes(query) {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.process_page_request = Some(request);
                self.replies
                    .register(WorkerKind::Monitor, request, reply, move |result| {
                        Internal::ProcessPage {
                            request,
                            outcome: result.map_err(failure_text),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Monitor,
                detail: error,
            }),
        }
    }

    fn request_network_connections(&mut self) -> DispatchOutcome {
        let Some(handle) = self.workers.monitor.as_ref() else {
            return DispatchOutcome::Rejected(self.monitor_unavailable());
        };
        match handle.request_network_connections() {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.replies
                    .register(WorkerKind::Monitor, request, reply, move |result| {
                        Internal::NetworkConnections {
                            request,
                            connections: result.map_err(failure_text),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Monitor,
                detail: error,
            }),
        }
    }

    // ---- providers -----------------------------------------------------

    fn provider_unavailable(&self) -> RejectReason {
        RejectReason::WorkerUnavailable {
            worker: WorkerKind::Provider,
            detail: self
                .snapshot
                .worker_error(WorkerKind::Provider)
                .unwrap_or("native AI provider discovery is unavailable")
                .to_string(),
        }
    }

    fn request_provider_status(&mut self) -> DispatchOutcome {
        let Some(runtime) = self.workers.provider.as_ref() else {
            return DispatchOutcome::Rejected(self.provider_unavailable());
        };
        match runtime.request_status() {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.provider_status_request = Some(request);
                self.snapshot.provider_loading = true;
                self.replies
                    .register(WorkerKind::Provider, request, reply, move |result| {
                        Internal::ProviderStatus {
                            request,
                            status: result.map(Box::new).map_err(failure_text),
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Provider,
                detail: error.to_string(),
            }),
        }
    }

    fn set_provider_preference(&mut self, preference: String) -> DispatchOutcome {
        let gate = PhiPreferenceGate::evaluate(
            self.snapshot.provider_status.as_ref(),
            self.snapshot.provider_loading,
        );
        if let Err(reason) = gate.validate(&preference) {
            self.queue
                .push(AppEvent::Provider(ProviderEvent::PreferenceRejected {
                    reason: reason.clone(),
                }));
            return DispatchOutcome::Rejected(RejectReason::Invalid { detail: reason });
        }
        let Some(runtime) = self.workers.provider.as_ref() else {
            return DispatchOutcome::Rejected(self.provider_unavailable());
        };
        match runtime.request_set_preference_and_status(preference.clone()) {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.provider_status_request = Some(request);
                self.snapshot.provider_loading = true;
                self.replies
                    .register(WorkerKind::Provider, request, reply, move |result| {
                        Internal::ProviderPreference {
                            request,
                            preference,
                            status: match result {
                                Ok(inner) => inner.map(Box::new),
                                Err(failure) => Err(failure_text(failure)),
                            },
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Provider,
                detail: error.to_string(),
            }),
        }
    }

    fn clear_ai_cache(&mut self, session_id: Option<String>) -> DispatchOutcome {
        let Some(runtime) = self.workers.provider.as_ref() else {
            return DispatchOutcome::Rejected(self.provider_unavailable());
        };
        match runtime.request_clear_cache(session_id) {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.replies
                    .register(WorkerKind::Provider, request, reply, move |_result| {
                        Internal::ProviderCacheCleared { request }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Provider,
                detail: error.to_string(),
            }),
        }
    }

    fn list_ollama_models(&mut self) -> DispatchOutcome {
        let Some(runtime) = self.workers.provider.as_ref() else {
            return DispatchOutcome::Rejected(self.provider_unavailable());
        };
        match runtime.request_ollama_models() {
            Ok(reply) => {
                let Some(request) = self.requests.issue() else {
                    return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                };
                self.replies
                    .register(WorkerKind::Provider, request, reply, move |result| {
                        Internal::ProviderOllamaModels {
                            request,
                            models: match result {
                                Ok(inner) => inner,
                                Err(failure) => Err(failure_text(failure)),
                            },
                        }
                    });
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Provider,
                detail: error.to_string(),
            }),
        }
    }

    // ---- settings ------------------------------------------------------

    fn send_settings(
        &mut self,
        kind: SettingsRequestKind,
        build: impl FnOnce(u64) -> SettingsCommand,
    ) -> DispatchOutcome {
        let Some(runtime) = self.workers.settings.as_ref() else {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Settings,
                detail: self
                    .snapshot
                    .worker_error(WorkerKind::Settings)
                    .unwrap_or("the settings worker is unavailable")
                    .to_string(),
            });
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match runtime.send(build(request.get())) {
            Ok(()) => {
                if matches!(kind, SettingsRequestKind::Load) {
                    self.snapshot.settings_loading = true;
                }
                self.settings_requests.insert(request.get(), kind);
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Settings,
                detail: error.to_string(),
            }),
        }
    }

    fn settings_load(&mut self) -> DispatchOutcome {
        self.send_settings(SettingsRequestKind::Load, |request_id| {
            SettingsCommand::Load { request_id }
        })
    }

    fn settings_save(&mut self, settings: Box<AppSettings>) -> DispatchOutcome {
        let echo = settings.clone();
        self.send_settings(SettingsRequestKind::Save(echo), move |request_id| {
            SettingsCommand::Save {
                request_id,
                settings,
            }
        })
    }

    fn settings_update(&mut self, update: SettingsUpdate) -> DispatchOutcome {
        self.send_settings(SettingsRequestKind::Update, move |request_id| {
            SettingsCommand::Update { request_id, update }
        })
    }

    fn settings_credential(&mut self, action: ProviderCredentialCommand) -> DispatchOutcome {
        match action {
            ProviderCredentialCommand::Commit(transaction) => {
                self.send_settings(SettingsRequestKind::Credential, move |request_id| {
                    SettingsCommand::CommitProviderCredentials {
                        request_id,
                        transaction: *transaction,
                    }
                })
            }
            ProviderCredentialCommand::Store { provider, key } => {
                match ProviderKeyId::parse(&provider) {
                    Ok(provider) => {
                        self.send_settings(SettingsRequestKind::Credential, move |request_id| {
                            SettingsCommand::StoreProviderKey {
                                request_id,
                                provider,
                                key,
                            }
                        })
                    }
                    Err(error) => DispatchOutcome::Rejected(RejectReason::Invalid {
                        detail: error.to_string(),
                    }),
                }
            }
            ProviderCredentialCommand::Clear { provider } => {
                match ProviderKeyId::parse(&provider) {
                    Ok(provider) => {
                        self.send_settings(SettingsRequestKind::Credential, move |request_id| {
                            SettingsCommand::ClearProviderKey {
                                request_id,
                                provider,
                            }
                        })
                    }
                    Err(error) => DispatchOutcome::Rejected(RejectReason::Invalid {
                        detail: error.to_string(),
                    }),
                }
            }
        }
    }

    fn apply_loaded_settings(&mut self, settings: AppSettings) {
        self.workers.set_retention(RetentionPolicy::from(&settings));
        self.startup_gate.apply_preference(settings.scan_on_startup);
        self.snapshot.settings = settings;
        self.snapshot.settings_error = None;
    }

    // ---- export --------------------------------------------------------

    fn export_results(&mut self, kind: ExportRequestKind) -> DispatchOutcome {
        let Some(runtime) = self.workers.export.as_ref() else {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Export,
                detail: self
                    .snapshot
                    .worker_error(WorkerKind::Export)
                    .unwrap_or("native report generation is unavailable")
                    .to_string(),
            });
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let results = self.scan.snapshot().evidence();
        match runtime.enqueue(ExportRequest {
            request_id: request.get(),
            kind,
            results,
        }) {
            Ok(()) => {
                self.export_requests.push(request.get());
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::Export,
                detail: error.to_string(),
            }),
        }
    }

    // ---- system and updates -------------------------------------------

    fn request_system_identity(&mut self) {
        let _ = self.request_system(SystemRequestKind::SystemInfo);
        let _ = self.request_system(SystemRequestKind::Architecture);
    }

    fn request_system(&mut self, kind: SystemRequestKind) -> DispatchOutcome {
        let Some(runtime) = self.workers.system.as_ref() else {
            return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::System,
                detail: self
                    .snapshot
                    .worker_error(WorkerKind::System)
                    .unwrap_or("the system worker is unavailable")
                    .to_string(),
            });
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        match runtime.enqueue(SystemRequest {
            request_id: request.get(),
            kind,
        }) {
            Ok(()) => {
                match kind {
                    SystemRequestKind::SystemInfo => self.system_info_request = Some(request),
                    SystemRequestKind::Architecture => self.architecture_request = Some(request),
                }
                DispatchOutcome::accepted_request(request)
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                worker: WorkerKind::System,
                detail: error.to_string(),
            }),
        }
    }

    fn check_for_updates(&mut self, reason: UpdateCheckReason) -> DispatchOutcome {
        let now = self.ports.environment.now_millis();
        let decision = schedule(
            reason,
            self.snapshot.update.in_flight,
            self.ports.update_throttle.should_check(now),
        );
        match decision {
            UpdateSchedule::Throttled => {
                self.queue.push(AppEvent::Update(UpdateEvent::Throttled));
                DispatchOutcome::Ignored {
                    detail: "an update check already ran today",
                }
            }
            UpdateSchedule::AlreadyRunning => DispatchOutcome::Ignored {
                detail: "an update check is already running",
            },
            UpdateSchedule::Check => {
                let Some(runtime) = self.workers.update.as_ref() else {
                    return DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                        worker: WorkerKind::Update,
                        detail: self
                            .snapshot
                            .worker_error(WorkerKind::Update)
                            .unwrap_or("the update worker is unavailable")
                            .to_string(),
                    });
                };
                match runtime.request_check() {
                    Ok(reply) => {
                        let Some(request) = self.requests.issue() else {
                            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
                        };
                        self.snapshot.update.in_flight = true;
                        self.update_request = Some(request);
                        self.replies
                            .register(WorkerKind::Update, request, reply, move |result| {
                                Internal::Update {
                                    request,
                                    outcome: result.map_err(failure_text),
                                }
                            });
                        DispatchOutcome::accepted_request(request)
                    }
                    Err(error) => DispatchOutcome::Rejected(RejectReason::WorkerUnavailable {
                        worker: WorkerKind::Update,
                        detail: error.to_string(),
                    }),
                }
            }
        }
    }

    fn restart_as_admin(&mut self) -> DispatchOutcome {
        let elevation = Arc::clone(&self.ports.elevation);
        let sender = self.internal_tx.clone();
        let queue = Arc::clone(&self.queue);
        let spawned = std::thread::Builder::new()
            .name("wfdiag-app-elevation".to_string())
            .spawn(move || {
                let _ = sender.send(Internal::Elevation(elevation.restart_as_admin()));
                queue.wake();
            });
        match spawned {
            Ok(_) => DispatchOutcome::accepted(),
            Err(error) => DispatchOutcome::Rejected(RejectReason::Invalid {
                detail: error.to_string(),
            }),
        }
    }

    // ---- draining ------------------------------------------------------

    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if let Some(executor) = self.executor.as_ref() {
            executor.spawn(future);
        }
    }

    fn drain_internal(&mut self) {
        let mut messages = Vec::new();
        for _ in 0..CHANNEL_DRAIN_LIMIT {
            match self.internal_rx.try_recv() {
                Ok(message) => messages.push(message),
                Err(_) => break,
            }
        }
        for message in messages {
            self.apply_internal(message);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_internal(&mut self, message: Internal) {
        match message {
            Internal::ScanStarted {
                session_id,
                kind,
                total,
            } => {
                if !self.scan.session_started(session_id.clone(), kind, total) {
                    return;
                }
                self.queue.push(AppEvent::Scan(ScanEvent::Started {
                    session_id: session_id.clone(),
                    kind,
                    total,
                }));
                if self.scan.cancel_requested() {
                    let _ = self.cancel_scan();
                } else {
                    self.launch_scan_run(session_id);
                }
            }
            Internal::ScanStartFailed { error } => {
                self.scan.start_failed();
                self.queue
                    .push(AppEvent::Scan(ScanEvent::StartFailed { error }));
            }
            Internal::ScanFinished {
                session_id,
                cancelled,
                evidence,
            } => self.apply_scan_finished(&session_id, cancelled, evidence),
            Internal::ScanCancelFinished { session_id, error } => {
                if self.scan.snapshot().session_id.as_deref() != Some(session_id.as_str()) {
                    return;
                }
                if let Some(error) = error {
                    self.scan.cancel_failed();
                    self.queue
                        .push(AppEvent::Scan(ScanEvent::CancelFailed { error }));
                } else {
                    self.queue
                        .push(AppEvent::Scan(ScanEvent::CancelAcknowledged));
                }
            }
            Internal::Elevation(result) => {
                let event = match result {
                    Ok(restarted) => SystemEvent::ElevationAttempted { restarted },
                    Err(error) => SystemEvent::Failed { error },
                };
                self.queue.push(AppEvent::System(event));
            }
            Internal::History {
                request,
                kind,
                scan_id,
                payload,
            } => self.apply_history_reply(request, kind, scan_id, payload),
            Internal::HistoryAutoSave { session_id, result } => {
                if result.is_ok() {
                    self.snapshot.history.error = None;
                } else if let Err(error) = result.as_ref() {
                    self.snapshot.history.error = Some(error.clone());
                }
                self.queue.push(AppEvent::History(match result.clone() {
                    Ok(()) => HistoryEvent::ScanSaved {
                        scan_id: session_id.clone(),
                    },
                    Err(error) => HistoryEvent::Failed {
                        request: HistoryRequest::AutoSave,
                        error,
                    },
                }));
                self.finish_scan(session_id, Some(result));
            }
            Internal::Update { request, outcome } => {
                if self.update_request != Some(request) {
                    // A superseded or already-expired check.
                    return;
                }
                self.update_request = None;
                self.snapshot.update.in_flight = false;
                match outcome {
                    Ok(outcome) => {
                        if !matches!(outcome, UpdateOutcome::Silent) {
                            let now = self.ports.environment.now_millis();
                            let _ = self.ports.update_throttle.record(now);
                        }
                        self.snapshot.update.available = outcome.available().cloned();
                        self.snapshot.update.last_outcome = Some(outcome.clone());
                        self.queue
                            .push(AppEvent::Update(UpdateEvent::Checked(Box::new(outcome))));
                    }
                    Err(error) => {
                        self.queue
                            .push(AppEvent::Update(UpdateEvent::Failed { error }));
                    }
                }
            }
            Internal::ProviderStatus { request, status } => {
                if self.provider_status_request != Some(request) {
                    return;
                }
                self.provider_status_request = None;
                self.snapshot.provider_loading = false;
                match status {
                    Ok(status) => {
                        self.snapshot.provider_status = Some((*status).clone());
                        self.queue
                            .push(AppEvent::Provider(ProviderEvent::Status(status)));
                    }
                    Err(error) => {
                        self.queue
                            .push(AppEvent::Provider(ProviderEvent::Failed { error }));
                    }
                }
            }
            Internal::ProviderPreference {
                request,
                preference,
                status,
            } => {
                if self.provider_status_request != Some(request) {
                    return;
                }
                self.provider_status_request = None;
                self.snapshot.provider_loading = false;
                match status {
                    Ok(status) => {
                        self.snapshot.provider_status = Some((*status).clone());
                        self.queue
                            .push(AppEvent::Provider(ProviderEvent::PreferenceApplied {
                                preference,
                                status,
                            }));
                    }
                    Err(error) => {
                        self.queue
                            .push(AppEvent::Provider(ProviderEvent::PreferenceRejected {
                                reason: error,
                            }));
                    }
                }
            }
            Internal::ProviderCacheCleared { request } => {
                let _ = request;
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::CacheCleared));
            }
            Internal::ProviderOllamaModels { request, models } => {
                let _ = request;
                match models {
                    Ok(models) => self
                        .queue
                        .push(AppEvent::Provider(ProviderEvent::OllamaModels(models))),
                    Err(error) => self
                        .queue
                        .push(AppEvent::Provider(ProviderEvent::Failed { error })),
                }
            }
            Internal::ReportBaseline {
                request,
                generation,
                comparison,
            } => self.start_report_generation(request, *generation, comparison.map(|value| *value)),
            Internal::ProcessPage { request, outcome } => {
                if self.process_page_request != Some(request) {
                    return;
                }
                self.process_page_request = None;
                match outcome {
                    Ok(ProcessQueryOutcome::Page(page)) => {
                        self.snapshot.monitor.process_page = Some((*page).clone());
                        self.queue
                            .push(AppEvent::Monitor(MonitorEvent::ProcessPage(page)));
                    }
                    Ok(ProcessQueryOutcome::Superseded) => {
                        self.queue
                            .push(AppEvent::Monitor(MonitorEvent::ProcessPageSuperseded));
                    }
                    Err(error) => {
                        self.snapshot.monitor.error = Some(error.clone());
                        self.queue
                            .push(AppEvent::Monitor(MonitorEvent::Unavailable {
                                reason: error,
                            }));
                    }
                }
            }
            Internal::NetworkConnections {
                request,
                connections,
            } => {
                let _ = request;
                match connections {
                    Ok(connections) => {
                        self.snapshot.monitor.connections = Some(connections.clone());
                        self.queue
                            .push(AppEvent::Monitor(MonitorEvent::NetworkConnections(
                                connections,
                            )));
                    }
                    Err(error) => {
                        self.queue
                            .push(AppEvent::Monitor(MonitorEvent::Unavailable {
                                reason: error,
                            }));
                    }
                }
            }
        }
    }

    fn apply_scan_finished(
        &mut self,
        session_id: &str,
        cancelled: bool,
        evidence: Result<SharedScanEvidence, String>,
    ) {
        // Every event was accepted before `run_session` returned; take them
        // here too so the final counters never depend on thread ordering.
        self.drain_diagnostic_events();
        let catalog = self.snapshot.catalog.clone();
        let outcome = self
            .scan
            .finish_run(session_id, cancelled, evidence, &catalog);
        match outcome {
            RunOutcome::Stale => {}
            RunOutcome::Cancelled => {
                self.queue.push(AppEvent::Scan(ScanEvent::Cancelled));
            }
            RunOutcome::Failed { error, stopped } => {
                self.queue
                    .push(AppEvent::Scan(ScanEvent::Failed { error, stopped }));
            }
            RunOutcome::Incomplete {
                completed,
                expected,
            } => {
                self.queue.push(AppEvent::Scan(ScanEvent::Incomplete {
                    completed,
                    expected,
                }));
            }
            RunOutcome::TargetedFailed { error } => {
                self.queue
                    .push(AppEvent::Scan(ScanEvent::TargetedFailed { error }));
            }
            RunOutcome::TargetedCommitted {
                session_id,
                task_id,
                evidence,
            } => {
                self.queue
                    .push(AppEvent::Scan(ScanEvent::TargetedCommitted {
                        session_id: session_id.clone(),
                        task_id,
                    }));
                self.commit_scan_evidence(session_id, evidence);
            }
            RunOutcome::Committed {
                session_id,
                evidence,
                auto_save,
            } => {
                let snapshot = self.scan.snapshot();
                self.queue.push(AppEvent::Scan(ScanEvent::Committed {
                    session_id: session_id.clone(),
                    completed: snapshot.completed,
                    errors: snapshot.errors,
                    duration_ms: snapshot.duration_ms,
                    auto_save,
                }));
                self.commit_scan_evidence(session_id.clone(), evidence);
                if auto_save {
                    self.begin_history_auto_save(session_id);
                } else {
                    self.queue.push(AppEvent::Scan(ScanEvent::Finalized {
                        session_id,
                        history: None,
                    }));
                }
            }
        }
    }

    fn apply_history_reply(
        &mut self,
        request: RequestId,
        kind: HistoryRequest,
        scan_id: Option<String>,
        payload: HistoryPayload,
    ) {
        if self.history_latest.get(&kind) != Some(&request) {
            // A superseded request of the same kind. Dropping it is what keeps
            // an older list from overwriting the newest one.
            return;
        }
        self.history_latest.remove(&kind);
        if matches!(kind, HistoryRequest::List | HistoryRequest::Compare) {
            self.snapshot.history.loading = false;
        }
        let event = match payload {
            HistoryPayload::Summaries(Ok(scans)) => {
                self.snapshot.history.summaries.clone_from(&scans);
                self.snapshot.history.error = None;
                HistoryEvent::Listed { scans }
            }
            HistoryPayload::Record(Ok(scan)) => HistoryEvent::Loaded { scan },
            HistoryPayload::Comparison(Ok(comparison)) => {
                self.snapshot.history.comparison = Some((*comparison).clone());
                HistoryEvent::Compared { comparison }
            }
            HistoryPayload::ComparisonSummary(Ok(comparison)) => {
                self.snapshot.history.comparison_summary = Some((*comparison).clone());
                HistoryEvent::ComparedSummary { comparison }
            }
            HistoryPayload::OptionalComparison(Ok(comparison)) => {
                HistoryEvent::ComparedToLatest { comparison }
            }
            HistoryPayload::TaskDiff(Ok(diff)) => HistoryEvent::TaskDiff { diff },
            HistoryPayload::Trends(Ok(trends)) => {
                self.snapshot.history.trends.clone_from(&trends);
                HistoryEvent::Trends { trends }
            }
            HistoryPayload::Unit(Ok(())) => match kind {
                HistoryRequest::Label => HistoryEvent::LabelSaved {
                    scan_id: scan_id.unwrap_or_default(),
                },
                HistoryRequest::Tags => HistoryEvent::TagsSaved {
                    scan_id: scan_id.unwrap_or_default(),
                },
                HistoryRequest::Clear => {
                    self.snapshot.history.summaries.clear();
                    self.snapshot.history.comparison = None;
                    self.snapshot.history.comparison_summary = None;
                    self.snapshot.history.trends.clear();
                    HistoryEvent::Cleared
                }
                _ => HistoryEvent::Cleared,
            },
            HistoryPayload::Summaries(Err(error))
            | HistoryPayload::Record(Err(error))
            | HistoryPayload::Comparison(Err(error))
            | HistoryPayload::ComparisonSummary(Err(error))
            | HistoryPayload::OptionalComparison(Err(error))
            | HistoryPayload::TaskDiff(Err(error))
            | HistoryPayload::Trends(Err(error))
            | HistoryPayload::Unit(Err(error)) => {
                self.snapshot.history.error = Some(error.clone());
                HistoryEvent::Failed {
                    request: kind,
                    error,
                }
            }
        };
        self.queue.push(AppEvent::History(event));
    }

    fn drain_diagnostic_events(&mut self) {
        let Some(receiver) = self.workers.diagnostic_events.as_ref() else {
            return;
        };
        let events = receiver.drain();
        let terminated = receiver.is_terminated();
        let catalog = self.snapshot.catalog.clone();
        for event in events {
            match event {
                UiEvent::TaskProgress(progress) => {
                    if self.scan.apply_progress(&progress, &catalog) {
                        let snapshot = self.scan.snapshot();
                        self.queue.push(AppEvent::Scan(ScanEvent::Progress {
                            task_id: progress.task_id,
                            status: progress.status,
                            task_name: progress.task_name,
                            completed: snapshot.completed,
                            total: snapshot.total,
                        }));
                    }
                }
                UiEvent::DiagnosticResult(result) => {
                    let task_id = result.task_id.clone();
                    let success = result.success;
                    if self.scan.apply_result(result, &catalog) {
                        let snapshot = self.scan.snapshot();
                        self.queue.push(AppEvent::Scan(ScanEvent::TaskResult {
                            task_id,
                            success,
                            completed: snapshot.completed,
                            errors: snapshot.errors,
                        }));
                    }
                }
                _ => {}
            }
        }
        if terminated && !self.terminating {
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::Diagnostics,
                panicked: true,
            });
            self.workers.diagnostic_events = None;
        }
    }

    fn drain_monitor_events(&mut self) {
        let Some(receiver) = self.workers.monitor_events.as_ref() else {
            return;
        };
        let events = receiver.drain();
        let terminated = receiver.is_terminated();
        for event in events {
            if let UiEvent::SystemStats(stats) = event {
                self.snapshot.monitor.error = None;
                self.snapshot.monitor.latest = Some(stats.clone());
                self.queue
                    .push(AppEvent::Monitor(MonitorEvent::Stats(Box::new(stats))));
            }
        }
        if terminated && !self.terminating {
            self.snapshot.monitor.available = false;
            self.workers.monitor_events = None;
            self.workers.monitor = None;
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::Monitor,
                panicked: true,
            });
        }
    }

    fn drain_settings_events(&mut self) {
        let mut events = Vec::new();
        let mut stopped = false;
        if let Some(receiver) = self.workers.settings_events.as_ref() {
            for _ in 0..CHANNEL_DRAIN_LIMIT {
                match receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        stopped = true;
                        break;
                    }
                }
            }
        }
        for event in events {
            self.apply_settings_event(event);
        }
        if stopped && !self.terminating {
            self.workers.settings_events = None;
            self.workers.settings = None;
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::Settings,
                panicked: true,
            });
        }
    }

    fn apply_settings_event(&mut self, event: SettingsEvent) {
        let (request_id, fact) = match event {
            SettingsEvent::Loaded { request_id, result } => (
                request_id,
                result.map(|settings| SettingsOutcome::Loaded(Box::new(settings))),
            ),
            SettingsEvent::Saved { request_id, result } => {
                (request_id, result.map(|()| SettingsOutcome::Saved))
            }
            SettingsEvent::Updated { request_id, result } => (
                request_id,
                result.map(|settings| SettingsOutcome::Updated(Box::new(settings))),
            ),
            SettingsEvent::ProviderKeyStored { request_id, result }
            | SettingsEvent::ProviderKeyCleared { request_id, result } => {
                (request_id, result.map(|()| SettingsOutcome::Credential))
            }
            SettingsEvent::ProviderCredentialsCommitted { request_id, result } => (
                request_id,
                result
                    .map(|()| SettingsOutcome::Credential)
                    .map_err(|error| {
                        wfdiag_native_settings::SettingsError::Runtime(error.to_string())
                    }),
            ),
            SettingsEvent::Stopped => {
                if !self.terminating {
                    self.queue.push(AppEvent::WorkerStopped {
                        worker: WorkerKind::Settings,
                        panicked: false,
                    });
                }
                return;
            }
        };
        let Some(kind) = self.settings_requests.remove(&request_id) else {
            // A reply for a request this service never made, or already
            // superseded: dropping it is the guard.
            return;
        };
        if matches!(kind, SettingsRequestKind::Load) {
            self.snapshot.settings_loading = false;
        }
        match fact {
            Ok(SettingsOutcome::Loaded(settings)) => {
                self.apply_loaded_settings((*settings).clone());
                self.queue
                    .push(AppEvent::Settings(SettingsFact::Loaded { settings }));
            }
            Ok(SettingsOutcome::Saved) => {
                if let SettingsRequestKind::Save(settings) = kind {
                    self.apply_loaded_settings((*settings).clone());
                    self.queue
                        .push(AppEvent::Settings(SettingsFact::Saved { settings }));
                }
            }
            Ok(SettingsOutcome::Updated(settings)) => {
                let policy = settings.cloud_fallback_policy;
                self.apply_loaded_settings((*settings).clone());
                self.queue
                    .push(AppEvent::Settings(SettingsFact::Updated { settings }));
                self.resolve_cloud_fallback_write(RequestId::from_raw(request_id), Ok(policy));
            }
            Ok(SettingsOutcome::Credential) => {
                self.queue
                    .push(AppEvent::Settings(SettingsFact::CredentialsCommitted));
            }
            Err(error) => {
                let error = error.to_string();
                self.snapshot.settings_error = Some(error.clone());
                self.queue
                    .push(AppEvent::Settings(SettingsFact::Failed { error }));
                self.resolve_cloud_fallback_write(RequestId::from_raw(request_id), Err(()));
            }
        }
    }

    fn drain_issue_replies(&mut self) {
        let mut completions = Vec::new();
        let mut stopped = false;
        if let Some(receiver) = self.workers.issue_replies.as_ref() {
            for _ in 0..CHANNEL_DRAIN_LIMIT {
                match receiver.try_recv() {
                    Ok(completion) => completions.push(completion),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        stopped = true;
                        break;
                    }
                }
            }
        }
        for completion in completions {
            let Some(pending) = self.issues.accept(&completion) else {
                // The evidence this reply was computed from is no longer
                // committed; a newer pending request stays intact.
                continue;
            };
            self.issue_outstanding = false;
            self.snapshot.issues.clone_from(&completion.issues);
            self.snapshot.issue_error = None;
            self.snapshot.derived_invalidated = Invalidation::on_issue_projection();
            self.apply_derived_invalidation(Invalidation::on_issue_projection());
            self.advance_evidence_generation();
            self.reconcile_staged_reviews();
            self.queue.push(AppEvent::Issues(IssuesEvent::Updated {
                session_id: pending.session_id,
                issues: completion.issues,
            }));
        }
        if stopped && !self.terminating {
            self.workers.issue_replies = None;
            self.workers.issues = None;
            self.issue_outstanding = false;
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::Issues,
                panicked: true,
            });
        }
    }

    fn drain_export_replies(&mut self) {
        let mut completions = Vec::new();
        let mut stopped = false;
        if let Some(receiver) = self.workers.export_replies.as_ref() {
            for _ in 0..CHANNEL_DRAIN_LIMIT {
                match receiver.try_recv() {
                    Ok(completion) => completions.push(completion),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        stopped = true;
                        break;
                    }
                }
            }
        }
        for completion in completions {
            let Some(index) = self
                .export_requests
                .iter()
                .position(|request| *request == completion.request_id)
            else {
                continue;
            };
            self.export_requests.remove(index);
            let event = match completion.result {
                Ok(payload) => ExportEvent::Completed {
                    payload: Box::new(payload),
                },
                Err(error) => ExportEvent::Failed {
                    error: error.to_string(),
                },
            };
            self.queue.push(AppEvent::Export(event));
        }
        if stopped && !self.terminating {
            self.workers.export_replies = None;
            self.workers.export = None;
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::Export,
                panicked: true,
            });
        }
    }

    fn drain_system_replies(&mut self) {
        let mut completions = Vec::new();
        let mut stopped = false;
        if let Some(receiver) = self.workers.system_replies.as_ref() {
            for _ in 0..CHANNEL_DRAIN_LIMIT {
                match receiver.try_recv() {
                    Ok(completion) => completions.push(completion),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        stopped = true;
                        break;
                    }
                }
            }
        }
        for completion in completions {
            let request = RequestId::from_raw(completion.request_id);
            let is_info = self.system_info_request == Some(request);
            let is_architecture = self.architecture_request == Some(request);
            if !is_info && !is_architecture {
                // A superseded probe: the newest request owns the snapshot.
                continue;
            }
            if is_info {
                self.system_info_request = None;
            }
            if is_architecture {
                self.architecture_request = None;
            }
            match completion.result {
                Ok(SystemPayload::SystemInfo(info)) => {
                    self.snapshot.system_info = Some(info.clone());
                    self.snapshot.system_error = None;
                    self.queue
                        .push(AppEvent::System(SystemEvent::Info(Box::new(info))));
                }
                Ok(SystemPayload::Architecture(architecture)) => {
                    self.snapshot.architecture = Some(architecture.clone());
                    self.queue
                        .push(AppEvent::System(SystemEvent::Architecture(Box::new(
                            architecture,
                        ))));
                }
                Err(error) => {
                    let error = error.to_string();
                    self.snapshot.system_error = Some(error.clone());
                    self.queue
                        .push(AppEvent::System(SystemEvent::Failed { error }));
                }
            }
        }
        if stopped && !self.terminating {
            self.workers.system_replies = None;
            self.workers.system = None;
            self.queue.push(AppEvent::WorkerStopped {
                worker: WorkerKind::System,
                panicked: true,
            });
        }
    }

    fn poll_replies(&mut self) {
        let batch = self.replies.poll(Instant::now());
        for timeout in batch.timeouts {
            self.queue.push(AppEvent::ReplyTimedOut {
                worker: timeout.worker,
                request: timeout.request,
            });
        }
        for message in batch.messages {
            self.apply_internal(message);
        }
    }

    fn maybe_start_delayed_update_check(&mut self) {
        let Some(due) = self.update_startup_due else {
            return;
        };
        if Instant::now() < due {
            return;
        }
        self.update_startup_due = None;
        let _ = self.check_for_updates(UpdateCheckReason::Startup);
    }

    fn maybe_start_startup_scan(&mut self) {
        let readiness = StartupReadiness {
            allowed: self.startup_scan_allowed && !self.terminating,
            settings_loading: self.snapshot.settings_loading,
            system_info_pending: self.system_info_request.is_some(),
            architecture_pending: self.architecture_request.is_some(),
        };
        if self.startup_gate.take_when_ready(readiness) {
            let _ = self.start_scan(ScanKind::Quick, None);
        }
    }

    fn publish_pending_work(&self) {
        let outstanding = self.ai_outstanding()
            + self.replies.len()
            + usize::from(self.issue_outstanding)
            + self.export_requests.len()
            + usize::from(self.system_info_request.is_some())
            + usize::from(self.architecture_request.is_some())
            + usize::from(self.update_startup_due.is_some())
            + usize::from(!matches!(self.scan.phase(), ScanPhase::Idle));
        self.watcher.signal().set_pending(outstanding);
    }

    /// The settings service, for hosts that need one synchronous read.
    #[must_use]
    pub fn settings_service(&self) -> SettingsService {
        self.settings_service.clone()
    }

    /// The configuration this service was started with.
    #[must_use]
    pub const fn config(&self) -> &AppConfig {
        &self.config
    }
}

enum SettingsOutcome {
    Loaded(Box<AppSettings>),
    Saved,
    Updated(Box<AppSettings>),
    Credential,
}

fn flatten<T>(result: Result<Result<T, String>, ReplyFailure>) -> Result<T, String> {
    match result {
        Ok(inner) => inner,
        Err(failure) => Err(failure_text(failure)),
    }
}
