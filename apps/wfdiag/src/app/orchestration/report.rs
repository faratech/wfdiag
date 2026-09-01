//! AI scan report orchestration.

#![deny(unsafe_code)]

use crate::ai::report::start_report_runtime;
use crate::app::WfdiagShell;
use crate::app::policy::{AiWorkerKind, build_history_scan_record, scan_kind_history_tag};
use crate::app::state::{AiMode, Page, PendingAiIntent};
use crate::app::tasks::spawn_report_generation_preparation;
use std::sync::{Arc, Mutex};
use wfdiag_native_ai_provider::{AIProvider, FoundryCliEndpointSource, ReqwestOllamaSource};
use wfdiag_native_ai_report::{ReportGeneration, ReportScan};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_issues::projection::advance_nonzero_generation;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn ensure_report_runtime(&mut self) -> Result<(), String> {
        if self.report_runtime.is_some() {
            return Ok(());
        }
        let settings = self.ai_worker_startup_settings(AiWorkerKind::Report)?;
        let (runtime, receiver) = start_report_runtime(
            settings,
            Arc::new(FoundryCliEndpointSource::new()),
            Arc::new(ReqwestOllamaSource),
            self.ai_worker_cache.clone(),
        )
        .map_err(|error| format!("Native AI report generation could not start: {error}"))?;
        self.report_receiver = Some(Arc::new(Mutex::new(receiver)));
        self.report_runtime = Some(runtime);
        Ok(())
    }

    pub(crate) fn resume_report_wait(&mut self, _context: &ComponentContext<Self>) {
        self.report_wait = None;
    }

    pub(crate) fn begin_report_generation(
        &mut self,
        force_refresh: bool,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · report generation is disabled".to_string();
            return;
        }
        if self.report_pending.is_some() {
            self.status = "A report is already being generated…".to_string();
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.status = "Enable AI insights in Settings before generating a report".to_string();
            return;
        }
        if self.ai_status_loading {
            self.status = "Waiting for AI provider discovery before generating…".to_string();
            return;
        }
        let Some(provider_status) = self
            .ai_provider_status
            .as_ref()
            .filter(|status| status.active_provider != AIProvider::None)
            .cloned()
        else {
            self.status = "Set up an available AI provider before generating".to_string();
            return;
        };
        if let Err(error) = self.ensure_report_runtime() {
            self.report_error = Some(error.clone());
            self.status = error;
            return;
        }
        self.report_error = None;
        if self.diagnostic_results.is_empty() {
            self.transition_to_page(Page::Ai);
            self.ai_mode = AiMode::ScanReport;
            self.pending_ai_intent = Some(PendingAiIntent::Report { force_refresh });
            self.pending_ai_preparation_error = None;
            if self.diagnostics_busy() {
                self.status =
                    "Waiting for the active scan before generating the AI report…".to_string();
            } else {
                self.status = "Running a Quick Scan before generating the AI report…".to_string();
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            return;
        }
        let Some(session_id) = self
            .diagnostic_results
            .first()
            .map(|result| result.session_id.clone())
        else {
            self.status = "Run a scan before generating a report".to_string();
            return;
        };
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before generating a report".to_string();
            return;
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.report_request_id) else {
            self.status = "Native report request identity was exhausted".to_string();
            return;
        };
        self.pending_ai_intent = None;
        self.report_text = None;
        self.report_provider = None;
        self.report_provider_use = None;
        self.report_error = None;
        self.report_source_session_id = Some(session_id.clone());
        self.report_pending = Some(request_id);
        let generation = ReportGeneration {
            scan: ReportScan {
                session_id,
                results,
            },
            provider: provider_status.active_provider,
            availability: provider_status.availability(),
            comparison: None,
            force_refresh,
        };
        self.status = "Preparing AI report…".to_string();

        // Match the shipping automatic-baseline rule atomically on the
        // history worker: compare the live scan with the newest persisted
        // scan from a different session. History is optional evidence.
        let history_tag = self
            .diagnostic_scan_policy
            .as_ref()
            .map(|policy| policy.history_tag.clone())
            .or_else(|| {
                self.diagnostic_scan_kind
                    .map(scan_kind_history_tag)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Diagnostic Scan".to_string());
        let current_record = build_history_scan_record(
            generation.scan.session_id.clone(),
            &self.system_info,
            &self.diagnostic_results,
            self.diagnostic_duration_ms,
            history_tag,
        );
        if let Some(history) = self.history_runtime.as_ref()
            && let Ok(reply) =
                history.request_compare_current_to_latest(std::sync::Arc::new(current_record))
        {
            self.report_prepare_task = Some(spawn_report_generation_preparation(
                context, request_id, generation, reply,
            ));
            return;
        }

        if self
            .report_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.generate(request_id, generation))
        {
            self.resume_report_wait(context);
        } else {
            self.report_pending = None;
            self.report_source_session_id = None;
            self.report_error = Some("The native report queue is unavailable".to_string());
            self.status = "The native report queue is unavailable".to_string();
        }
    }

    pub(crate) fn invalidate_report_for_new_scan(&mut self) {
        if let Some(task) = self.report_prepare_task.take() {
            task.cancel();
        } else if let Some(request_id) = self.report_pending {
            // Invalidate locally before the best-effort worker cancellation.
            // A slow or failed cancellation must not let an old report block
            // generation for the newly committed scan.
            let _ = self
                .report_runtime
                .as_ref()
                .is_some_and(|runtime| runtime.cancel(request_id));
        }
        self.report_pending = None;
        self.report_text = None;
        self.report_provider = None;
        self.report_provider_use = None;
        self.report_source_session_id = None;
        self.report_error = None;
    }
}
