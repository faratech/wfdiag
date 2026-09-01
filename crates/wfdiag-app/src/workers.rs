//! Booting every engine runtime behind [`crate::AppPorts`].
//!
//! The order and the capacities mirror the native shell's `Component::create`,
//! which is the shipping behaviour this facade has to preserve. A worker that
//! refuses to start is not fatal: it is recorded as unavailable, its commands
//! are rejected with that reason, and everything else keeps working.

use crate::command::WorkerKind;
use crate::config::AppConfig;
use crate::domain::history::RetentionPolicy;
use crate::event::EventQueue;
use crate::ports::AppPorts;
use crate::ports::monitor::MonitorHandle;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use wfdiag_native_ai_provider::NativeAiProviderRuntime;
use wfdiag_native_diagnostics::{DiagnosticRuntime, DiagnosticTask};
use wfdiag_native_export::{ExportCompleted, ExportRuntime, ExportTask};
use wfdiag_native_history::{
    DiagnosticTask as HistoryDiagnosticTask, HistoryRuntimeConfig, NativeHistoryRuntime,
};
use wfdiag_native_issues::{
    IssueDetectionCompleted, IssueRuntime, RemediationSummary, remediation_summaries,
};
use wfdiag_native_settings::{SettingsEvent, SettingsRuntime, SettingsService};
use wfdiag_native_system::{SystemCompleted, SystemRuntime};
use wfdiag_native_update::{NativeUpdateRuntime, UpdateService};
use wfdiag_ui_core::{UiEventReceiver, UiWakeHandler};

/// One worker that could not start.
#[derive(Clone, Debug)]
pub(crate) struct WorkerStartFailure {
    pub(crate) worker: WorkerKind,
    pub(crate) detail: String,
}

/// Every runtime the service owns.
pub(crate) struct AppWorkers {
    pub(crate) diagnostics: Option<DiagnosticRuntime>,
    pub(crate) diagnostic_events: Option<UiEventReceiver>,
    pub(crate) issues: Option<IssueRuntime>,
    pub(crate) issue_replies: Option<mpsc::Receiver<IssueDetectionCompleted>>,
    pub(crate) history: Option<Arc<NativeHistoryRuntime>>,
    pub(crate) export: Option<ExportRuntime>,
    pub(crate) export_replies: Option<mpsc::Receiver<ExportCompleted>>,
    pub(crate) system: Option<SystemRuntime>,
    pub(crate) system_replies: Option<mpsc::Receiver<SystemCompleted>>,
    pub(crate) settings: Option<SettingsRuntime>,
    pub(crate) settings_events: Option<mpsc::Receiver<SettingsEvent>>,
    pub(crate) provider: Option<NativeAiProviderRuntime>,
    pub(crate) update: Option<NativeUpdateRuntime>,
    pub(crate) monitor: Option<Box<dyn MonitorHandle>>,
    pub(crate) monitor_events: Option<UiEventReceiver>,
    pub(crate) retention: Arc<RwLock<RetentionPolicy>>,
    pub(crate) catalog: Vec<DiagnosticTask>,
    pub(crate) remediations: Vec<RemediationSummary>,
}

impl std::fmt::Debug for AppWorkers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppWorkers")
            .field("catalog", &self.catalog.len())
            .field("monitor", &self.monitor.is_some())
            .finish_non_exhaustive()
    }
}

/// Project the diagnostic catalog into the history worker's metadata shape.
fn history_task_catalog(catalog: &[DiagnosticTask]) -> Vec<HistoryDiagnosticTask> {
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

/// Project the diagnostic catalog into the export renderer's metadata shape.
fn export_task_catalog(catalog: &[DiagnosticTask]) -> Vec<ExportTask> {
    catalog
        .iter()
        .map(|task| ExportTask::new(&task.id, &task.name, &task.category))
        .collect()
}

impl AppWorkers {
    /// Start every runtime, in the shipping shell's order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn start(
        config: &AppConfig,
        ports: &AppPorts,
        settings_service: &SettingsService,
        retention: RetentionPolicy,
        queue: &Arc<EventQueue>,
    ) -> (Self, Vec<WorkerStartFailure>) {
        let mut failures = Vec::new();
        let retention = Arc::new(RwLock::new(retention));

        // 1. Live monitoring, so telemetry starts flowing while the rest boots.
        let (monitor, monitor_events) = if config.start_monitor {
            match ports.monitor.start(config.monitor_profile) {
                Ok(Some(session)) => {
                    let wake_queue = Arc::clone(queue);
                    session
                        .events
                        .set_wake_handler(UiWakeHandler::new(move || wake_queue.wake()));
                    (Some(session.handle), Some(session.events))
                }
                Ok(None) => {
                    failures.push(WorkerStartFailure {
                        worker: WorkerKind::Monitor,
                        detail: "live monitoring is not available on this host".to_string(),
                    });
                    (None, None)
                }
                Err(error) => {
                    failures.push(WorkerStartFailure {
                        worker: WorkerKind::Monitor,
                        detail: format!("Native monitoring could not start: {error}"),
                    });
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // 2. Host identity. The initial requests are issued by the service so
        //    their ids come from the one request-id counter.
        let (system, system_replies) =
            match SystemRuntime::start_with_provider(Arc::clone(&ports.system)) {
                Ok((runtime, replies)) => (Some(runtime), Some(replies)),
                Err(error) => {
                    failures.push(WorkerStartFailure {
                        worker: WorkerKind::System,
                        detail: error.to_string(),
                    });
                    (None, None)
                }
            };

        // 3. Diagnostics, which also supplies the catalog every later worker
        //    is configured from.
        let (diagnostics, diagnostic_events) = DiagnosticRuntime::with_event_bus(
            Arc::clone(&ports.diagnostics),
            config.diagnostic_event_capacity,
        );
        let catalog = diagnostics.available_tasks();
        let wake_queue = Arc::clone(queue);
        diagnostic_events.set_wake_handler(UiWakeHandler::new(move || wake_queue.wake()));

        // 4. Export rendering.
        let (export, export_replies) = match ExportRuntime::start(export_task_catalog(&catalog)) {
            Ok((runtime, replies)) => (Some(runtime), Some(replies)),
            Err(error) => {
                failures.push(WorkerStartFailure {
                    worker: WorkerKind::Export,
                    detail: format!("Native report generation is unavailable: {error}"),
                });
                (None, None)
            }
        };

        // 5. Issue detection, validated against the canonical remediation
        //    catalog before its worker starts.
        let remediations = remediation_summaries();
        let (issues, issue_replies) = match IssueRuntime::start(remediations.clone()) {
            Ok((runtime, replies)) => (Some(runtime), Some(replies)),
            Err(error) => {
                failures.push(WorkerStartFailure {
                    worker: WorkerKind::Issues,
                    detail: format!("Native issue detection is unavailable: {error}"),
                });
                (None, None)
            }
        };

        // 6. Scan history, reading the live retention policy on every save.
        let history = if let Some(directory) = config.history_storage_dir.clone() {
            let retention_provider = Arc::clone(&retention);
            let task_catalog = history_task_catalog(&catalog);
            match NativeHistoryRuntime::start(HistoryRuntimeConfig::new(
                directory,
                move || {
                    retention_provider.read().map_or_else(
                        |_| RetentionPolicy::default().as_tuple(),
                        |policy| policy.as_tuple(),
                    )
                },
                move || task_catalog.clone(),
            )) {
                Ok(runtime) => Some(Arc::new(runtime)),
                Err(error) => {
                    failures.push(WorkerStartFailure {
                        worker: WorkerKind::History,
                        detail: format!("Native history is unavailable: {error}"),
                    });
                    None
                }
            }
        } else {
            failures.push(WorkerStartFailure {
                worker: WorkerKind::History,
                detail: "no history storage directory was configured".to_string(),
            });
            None
        };

        // 7. Settings, with an event-driven wake so no task polls it.
        let wake_queue = Arc::clone(queue);
        let (settings, settings_events) = match SettingsRuntime::start_with_wake(
            settings_service.clone(),
            Arc::new(move || wake_queue.wake()),
        ) {
            Ok((runtime, events)) => (Some(runtime), Some(events)),
            Err(error) => {
                failures.push(WorkerStartFailure {
                    worker: WorkerKind::Settings,
                    detail: error.to_string(),
                });
                (None, None)
            }
        };

        // 8. AI provider management.
        let provider = match NativeAiProviderRuntime::start(Arc::clone(&ports.provider_backend)) {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                failures.push(WorkerStartFailure {
                    worker: WorkerKind::Provider,
                    detail: format!("Native AI provider discovery is unavailable: {error}"),
                });
                None
            }
        };

        // 9. The GitHub update channel.
        let update = match NativeUpdateRuntime::start(UpdateService::new(
            Arc::clone(&ports.release_http),
            Arc::clone(&ports.signature),
            Arc::clone(&ports.current_version),
            config.debug_build,
        )) {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                failures.push(WorkerStartFailure {
                    worker: WorkerKind::Update,
                    detail: error.to_string(),
                });
                None
            }
        };

        (
            Self {
                diagnostics: Some(diagnostics),
                diagnostic_events: Some(diagnostic_events),
                issues,
                issue_replies,
                history,
                export,
                export_replies,
                system,
                system_replies,
                settings,
                settings_events,
                provider,
                update,
                monitor,
                monitor_events,
                retention,
                catalog,
                remediations,
            },
            failures,
        )
    }

    /// Publish a new retention policy for the next history save.
    pub(crate) fn set_retention(&self, policy: RetentionPolicy) {
        if let Ok(mut current) = self.retention.write() {
            *current = policy;
        }
    }

    /// Stop every worker in dependency order, honouring `budget` per worker.
    ///
    /// Producers stop before consumers: the scan coordinator and monitor first
    /// (they publish events), then the workers that answer requests, then
    /// settings last so an in-flight save is not cut short.
    pub(crate) fn stop(&mut self, budget: Duration) -> Vec<WorkerStopRecord> {
        let mut records = Vec::new();

        // Live producers first: their event buses are closed so no further
        // events can be published into a queue nobody will drain.
        if let Some(events) = self.monitor_events.take() {
            events.clear_wake_handler();
            events.close();
        }
        drop(self.monitor.take());
        records.push(WorkerStopRecord::stopped(WorkerKind::Monitor));

        if let Some(events) = self.diagnostic_events.take() {
            events.clear_wake_handler();
            events.close();
        }
        drop(self.diagnostics.take());
        records.push(WorkerStopRecord::stopped(WorkerKind::Diagnostics));

        if let Some(mut issues) = self.issues.take() {
            records.push(WorkerStopRecord::joined(
                WorkerKind::Issues,
                issues.stop_and_join(budget),
            ));
        }
        self.issue_replies = None;

        if let Some(mut export) = self.export.take() {
            records.push(WorkerStopRecord::joined(
                WorkerKind::Export,
                export.stop_and_join(budget),
            ));
        }
        self.export_replies = None;

        if let Some(mut system) = self.system.take() {
            records.push(WorkerStopRecord::joined(
                WorkerKind::System,
                system.stop_and_join(budget),
            ));
        }
        self.system_replies = None;

        drop(self.history.take());
        records.push(WorkerStopRecord::stopped(WorkerKind::History));
        drop(self.provider.take());
        records.push(WorkerStopRecord::stopped(WorkerKind::Provider));
        drop(self.update.take());
        records.push(WorkerStopRecord::stopped(WorkerKind::Update));

        if let Some(mut settings) = self.settings.take() {
            records.push(WorkerStopRecord::joined(
                WorkerKind::Settings,
                settings.stop_and_join(budget),
            ));
        }
        self.settings_events = None;

        records
    }
}

/// How one worker responded to teardown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerStopRecord {
    /// Which worker.
    pub worker: WorkerKind,
    /// Whether it exited inside the budget. Workers whose crates expose only
    /// a detached-reaper `Drop` always report `true`: they are asked to stop
    /// and joined off the caller's thread, so the caller never blocks.
    pub stopped_within_budget: bool,
}

impl WorkerStopRecord {
    const fn stopped(worker: WorkerKind) -> Self {
        Self {
            worker,
            stopped_within_budget: true,
        }
    }

    const fn joined(worker: WorkerKind, joined: bool) -> Self {
        Self {
            worker,
            stopped_within_budget: joined,
        }
    }
}
