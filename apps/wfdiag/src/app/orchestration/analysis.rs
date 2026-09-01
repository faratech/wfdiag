//! AI analysis orchestration: diagnostics, prioritization, and fix plans.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{AiWorkerKind, issue_prioritization_payload, provider_display_name};
use crate::app::state::{
    DiagnosticAnalysisAttempt, FixPlanAttempt, IssuePrioritizationAttempt,
    IssuePrioritizationDisplay, PendingDiagnosticAnalysis, PendingFixPlan,
    PendingIssuePrioritization,
};
use crate::platform::ui_wake;
use std::sync::{Arc, Mutex};
use wfdiag_native_ai_analysis::{
    AnalysisRoute, AnalysisWorkerEvent, DiagnosticAnalysisGeneration, FixPlanGeneration,
    IssuePrioritizationGeneration, NativeAnalysisRuntime, NativeFixPlanRuntime,
    initial_fix_plan_route,
};
use wfdiag_native_ai_chat::ProviderUse;
use wfdiag_native_ai_provider::{
    AIProvider, FoundryCliEndpointSource, ReqwestOllamaSource, next_auto_local_route,
    next_fallback_candidate, parse_provider_preference,
};
use wfdiag_native_issues::projection::advance_nonzero_generation;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn ensure_analysis_runtime(&mut self) -> Result<(), String> {
        if self.analysis_runtime.is_some() {
            return Ok(());
        }
        let settings = self.ai_worker_startup_settings(AiWorkerKind::Analysis)?;
        let (runtime, receiver) = NativeAnalysisRuntime::start(
            settings,
            Arc::new(FoundryCliEndpointSource::new()),
            Arc::new(ReqwestOllamaSource),
            self.ai_worker_cache.clone(),
            Arc::new(ui_wake::notify),
        )
        .map_err(|error| format!("Native diagnostic AI could not start: {error}"))?;
        self.analysis_receiver = Some(Arc::new(Mutex::new(receiver)));
        self.analysis_runtime = Some(runtime);
        Ok(())
    }

    pub(crate) fn ensure_fix_plan_runtime(&mut self) -> Result<(), String> {
        if self.fix_plan_runtime.is_some() {
            return Ok(());
        }
        let settings = self.ai_worker_startup_settings(AiWorkerKind::FixPlan)?;
        let (runtime, receiver) = NativeFixPlanRuntime::start(
            settings,
            Arc::new(FoundryCliEndpointSource::new()),
            Arc::new(ReqwestOllamaSource),
            Arc::new(ui_wake::notify),
        )
        .map_err(|error| format!("Native AI fix planning could not start: {error}"))?;
        self.fix_plan_receiver = Some(Arc::new(Mutex::new(receiver)));
        self.fix_plan_runtime = Some(runtime);
        Ok(())
    }

    pub(crate) fn resume_analysis_wait(&mut self, _context: &ComponentContext<Self>) {
        self.analysis_wait = None;
    }

    pub(crate) fn resume_fix_plan_wait(&mut self, _context: &ComponentContext<Self>) {
        self.fix_plan_wait = None;
    }

    pub(crate) fn invalidate_fix_plan(&mut self) {
        if let Some(pending) = self.fix_plan_pending.take()
            && let Some(runtime) = self.fix_plan_runtime.as_ref()
        {
            let _ = runtime.cancel(pending.request_id);
        }
        self.fix_plan = None;
        self.fix_plan_error = None;
    }

    pub(crate) fn invalidate_issue_prioritization(&mut self) {
        if let Some(pending) = self.issue_prioritization_pending.take()
            && let Some(runtime) = self.analysis_runtime.as_ref()
        {
            let _ = runtime.cancel(pending.request_id);
        }
        self.issue_prioritization = IssuePrioritizationDisplay::default();
    }

    pub(crate) fn begin_issue_prioritization(
        &mut self,
        force_refresh: bool,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · issue prioritization is disabled".to_string();
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.status = "Enable AI insights in Settings before prioritizing issues".to_string();
            return;
        }
        if self.analysis_pending.is_some() || self.issue_prioritization_pending.is_some() {
            self.status = "Another one-shot AI analysis is already running…".to_string();
            return;
        }
        let Some(session_id) = self.issue_source_session_id.clone() else {
            self.status = "Run a diagnostic scan before prioritizing issues".to_string();
            return;
        };
        let issues_data = match issue_prioritization_payload(&self.issues) {
            Ok(payload) if payload != "[]" => payload,
            Ok(_) => {
                self.invalidate_issue_prioritization();
                self.status = "No detected issues need prioritization".to_string();
                return;
            }
            Err(error) => {
                self.issue_prioritization.error = Some(error.clone());
                self.status = format!("Could not prepare issue prioritization · {error}");
                return;
            }
        };
        if !force_refresh
            && self.issue_prioritization.committed_epoch == self.issue_committed_epoch
            && self.issue_prioritization.text.is_some()
        {
            self.status = "Issue prioritization is already available".to_string();
            return;
        }
        let Some(status) = self.ai_provider_status.as_ref() else {
            self.status = "Set up an available AI provider before prioritizing issues".to_string();
            return;
        };
        let preference = parse_provider_preference(&self.settings_snapshot.preferred_ai_provider);
        let availability = status.availability();
        let Some(candidate) = next_fallback_candidate(preference, None, &[], availability) else {
            self.status = "Set up an available AI provider before prioritizing issues".to_string();
            return;
        };
        let attempt = IssuePrioritizationAttempt {
            generation: IssuePrioritizationGeneration {
                session_id,
                issues_data,
                route: AnalysisRoute {
                    preference,
                    provider: candidate.provider,
                    availability,
                    fallback_from: None,
                },
                network_grounding_enabled: self.settings_snapshot.network_grounding_enabled,
                force_refresh,
            },
            tried: vec![candidate.provider],
            initial_provider: candidate.provider,
            committed_epoch: self.issue_committed_epoch,
        };
        let _ = self.queue_issue_prioritization(attempt, context);
    }

    pub(crate) fn queue_issue_prioritization(
        &mut self,
        attempt: IssuePrioritizationAttempt,
        context: &ComponentContext<Self>,
    ) -> bool {
        if let Err(error) = self.ensure_analysis_runtime() {
            self.issue_prioritization.error = Some(error.clone());
            self.issue_prioritization.busy = false;
            self.status = error;
            return false;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.analysis_request_id) else {
            self.issue_prioritization.error =
                Some("AI issue-prioritization request identity was exhausted".to_string());
            self.status = "Native AI issue prioritization is unavailable".to_string();
            return false;
        };
        let Some(runtime) = self.analysis_runtime.as_ref() else {
            self.issue_prioritization.error =
                Some("Native one-shot AI worker is unavailable".to_string());
            self.status = "Native AI issue prioritization is unavailable".to_string();
            return false;
        };
        let provider = attempt.generation.route.provider;
        let queued = if attempt.generation.force_refresh {
            runtime.reprioritize_issues(request_id, attempt.generation.clone())
        } else {
            runtime.prioritize_issues(request_id, attempt.generation.clone())
        };
        if !queued {
            self.issue_prioritization.error =
                Some("The native one-shot AI queue is unavailable".to_string());
            self.status = "The native AI issue-prioritization queue is unavailable".to_string();
            return false;
        }
        self.issue_prioritization = IssuePrioritizationDisplay {
            text: None,
            provider_use: Some(ProviderUse::for_provider(
                provider,
                attempt.generation.route.fallback_from,
            )),
            grounding: None,
            cached: false,
            error: None,
            busy: true,
            committed_epoch: attempt.committed_epoch,
        };
        self.issue_prioritization_pending = Some(PendingIssuePrioritization {
            request_id,
            attempt,
        });
        self.status = format!(
            "Prioritizing detected issues · {}…",
            provider_display_name(provider)
        );
        self.resume_analysis_wait(context);
        true
    }

    pub(crate) fn apply_issue_prioritization_event(
        &mut self,
        event: AnalysisWorkerEvent,
        context: &ComponentContext<Self>,
    ) {
        match event {
            AnalysisWorkerEvent::Ack {
                provider_use,
                grounding,
                cached,
                ..
            } => {
                self.issue_prioritization.provider_use = Some(provider_use);
                self.issue_prioritization.grounding = grounding;
                self.issue_prioritization.cached = cached;
                self.issue_prioritization.busy = true;
                self.status = if cached {
                    "Loading cached issue prioritization…".to_string()
                } else {
                    "The AI provider is prioritizing detected issues…".to_string()
                };
                self.resume_analysis_wait(context);
            }
            AnalysisWorkerEvent::Done {
                interpretation,
                provider_use,
                grounding,
                cached,
                ..
            } => {
                let Some(pending) = self.issue_prioritization_pending.take() else {
                    return;
                };
                if pending.attempt.committed_epoch != self.issue_committed_epoch
                    || pending.attempt.generation.session_id
                        != self.issue_source_session_id.as_deref().unwrap_or_default()
                {
                    self.issue_prioritization = IssuePrioritizationDisplay::default();
                    self.status =
                        "Ignored stale issue prioritization after evidence changed".to_string();
                    return;
                }
                let provider = provider_use.provider_id.clone();
                self.issue_prioritization = IssuePrioritizationDisplay {
                    text: Some(interpretation),
                    provider_use: Some(provider_use),
                    grounding,
                    cached,
                    error: None,
                    busy: false,
                    committed_epoch: pending.attempt.committed_epoch,
                };
                self.status = if cached {
                    format!("Issue prioritization ready · {provider} · cached")
                } else {
                    format!("Issue prioritization ready · {provider}")
                };
            }
            AnalysisWorkerEvent::Failed {
                route,
                provider_use,
                grounding,
                message,
                retryable,
                ..
            } => {
                let Some(mut pending) = self.issue_prioritization_pending.take() else {
                    self.resume_analysis_wait(context);
                    return;
                };
                let next_local = retryable.then(|| {
                    next_auto_local_route(
                        route.preference,
                        &pending.attempt.tried,
                        route.availability,
                    )
                });
                if let Some(Some(provider)) = next_local {
                    pending.attempt.tried.push(provider);
                    pending.attempt.generation.route.provider = provider;
                    pending.attempt.generation.route.fallback_from =
                        Some(pending.attempt.initial_provider);
                    if self.queue_issue_prioritization(pending.attempt, context) {
                        return;
                    }
                }
                self.issue_prioritization.provider_use = Some(provider_use);
                self.issue_prioritization.grounding = grounding;
                self.issue_prioritization.error = Some(message.clone());
                self.issue_prioritization.busy = false;
                self.status = format!("Issue prioritization failed · {message}");
            }
            AnalysisWorkerEvent::Cancelled {
                provider_use,
                grounding,
                ..
            } => {
                self.issue_prioritization_pending = None;
                self.issue_prioritization.provider_use = Some(provider_use);
                self.issue_prioritization.grounding = grounding;
                self.issue_prioritization.busy = false;
                self.issue_prioritization.error = None;
                self.status = "Issue prioritization cancelled".to_string();
            }
        }
    }

    pub(crate) fn begin_fix_plan(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · AI fix planning is disabled".to_string();
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.status = "Enable AI insights in Settings before proposing a fix plan".to_string();
            return;
        }
        if self.fix_plan_pending.is_some() {
            self.status = "A fix plan is already being generated…".to_string();
            return;
        }
        let detected_issues = self
            .issues
            .iter()
            .filter(|issue| {
                issue.detected && issue.status == wfdiag_native_issues::IssueStatus::Detected
            })
            .cloned()
            .collect::<Vec<_>>();
        if detected_issues.is_empty() {
            self.fix_plan = None;
            self.fix_plan_error = None;
            self.status = "No detected issues need a fix plan".to_string();
            return;
        }
        let Some(provider_status) = self.ai_provider_status.as_ref() else {
            self.status = "Set up an available AI provider before proposing a fix plan".to_string();
            return;
        };
        let preference = parse_provider_preference(&self.settings_snapshot.preferred_ai_provider);
        let availability = provider_status.availability();
        let route =
            initial_fix_plan_route(preference, provider_status.active_provider, availability);
        if route.provider == AIProvider::None || !availability.contains(route.provider) {
            self.status = "Set up an available AI provider before proposing a fix plan".to_string();
            return;
        }
        let snapshot = self.action_snapshot();
        let mut tried = route.fallback_from.into_iter().collect::<Vec<_>>();
        if !tried.contains(&route.provider) {
            tried.push(route.provider);
        }
        let initial_provider = tried.first().copied().unwrap_or(route.provider);
        let attempt = FixPlanAttempt {
            generation: FixPlanGeneration {
                detected_issues,
                route,
                scan_fingerprint: snapshot.scan_fingerprint,
                catalog_fingerprint: snapshot.catalog_fingerprint,
            },
            tried,
            initial_provider,
        };
        self.fix_plan = None;
        self.fix_plan_error = None;
        let _ = self.queue_fix_plan(attempt, context);
    }

    pub(crate) fn queue_fix_plan(
        &mut self,
        attempt: FixPlanAttempt,
        context: &ComponentContext<Self>,
    ) -> bool {
        if let Err(error) = self.ensure_fix_plan_runtime() {
            self.fix_plan_error = Some(error.clone());
            self.status = error;
            return false;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.fix_plan_request_id) else {
            self.fix_plan_error = Some("AI fix-plan request identity was exhausted".to_string());
            self.status = "Native AI fix planning is unavailable".to_string();
            return false;
        };
        let Some(runtime) = self.fix_plan_runtime.as_ref() else {
            self.fix_plan_error = Some("Native AI fix-plan worker is unavailable".to_string());
            self.status = "Native AI fix planning is unavailable".to_string();
            return false;
        };
        let provider = attempt.generation.route.provider;
        if !runtime.generate(request_id, attempt.generation.clone()) {
            self.fix_plan_error = Some("The native AI fix-plan queue is unavailable".to_string());
            self.status = "The native AI fix-plan queue is unavailable".to_string();
            return false;
        }
        self.fix_plan_pending = Some(PendingFixPlan {
            request_id,
            attempt,
        });
        self.fix_plan_error = None;
        self.status = format!("Preparing a vetted fix plan with {provider}…");
        self.resume_fix_plan_wait(context);
        true
    }

    pub(crate) fn begin_selected_diagnostic_analysis(
        &mut self,
        force_refresh: bool,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · diagnostic AI is disabled".to_string();
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.status = "Enable AI insights in Settings before interpreting".to_string();
            return;
        }
        if self.analysis_pending.is_some() || self.issue_prioritization_pending.is_some() {
            self.status = "Another one-shot AI analysis is already running…".to_string();
            return;
        }
        let Some(result) = self
            .selected_result_task_id
            .as_deref()
            .and_then(|task_id| {
                self.diagnostic_results
                    .iter()
                    .find(|result| result.task_id == task_id)
            })
            .or_else(|| self.diagnostic_results.first())
            .cloned()
        else {
            self.status = "Run a diagnostic scan before asking for an interpretation".to_string();
            return;
        };
        if !force_refresh
            && self
                .diagnostic_analyses
                .get(&result.task_id)
                .and_then(|display| display.interpretation.as_ref())
                .is_some()
        {
            self.status = "Diagnostic interpretation is already available".to_string();
            return;
        }
        let Some(status) = self.ai_provider_status.as_ref() else {
            self.status = "Set up an available AI provider before interpreting".to_string();
            return;
        };
        let preference = parse_provider_preference(&self.settings_snapshot.preferred_ai_provider);
        let availability = status.availability();
        let Some(candidate) = next_fallback_candidate(preference, None, &[], availability) else {
            self.status = "Set up an available AI provider before interpreting".to_string();
            return;
        };
        let task_name = self
            .diagnostic_catalog
            .iter()
            .find(|task| task.id == result.task_id)
            .map_or_else(|| result.task_id.clone(), |task| task.name.clone());
        let generation = DiagnosticAnalysisGeneration {
            session_id: result.session_id.clone(),
            task_id: result.task_id.clone(),
            task_name,
            diagnostic_result: Arc::clone(&result.result),
            route: AnalysisRoute {
                preference,
                provider: candidate.provider,
                availability,
                fallback_from: None,
            },
            network_grounding_enabled: self.settings_snapshot.network_grounding_enabled,
            force_refresh,
        };
        let _ = self.queue_diagnostic_analysis(
            DiagnosticAnalysisAttempt {
                generation,
                tried: vec![candidate.provider],
                initial_provider: candidate.provider,
            },
            context,
        );
    }

    pub(crate) fn queue_diagnostic_analysis(
        &mut self,
        attempt: DiagnosticAnalysisAttempt,
        context: &ComponentContext<Self>,
    ) -> bool {
        let task_id = attempt.generation.task_id.clone();
        if let Err(error) = self.ensure_analysis_runtime() {
            let display = self.diagnostic_analyses.entry(task_id).or_default();
            display.busy = false;
            display.error = Some(error.clone());
            self.status = error;
            return false;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.analysis_request_id) else {
            self.status = "Diagnostic AI request identity was exhausted".to_string();
            return false;
        };
        let provider = attempt.generation.route.provider;
        let Some(runtime) = self.analysis_runtime.as_ref() else {
            self.status = "Native diagnostic AI is unavailable".to_string();
            return false;
        };
        let queued = if attempt.generation.force_refresh {
            runtime.retry(request_id, attempt.generation.clone())
        } else {
            runtime.generate(request_id, attempt.generation.clone())
        };
        if !queued {
            self.status = "The native diagnostic AI queue is unavailable".to_string();
            return false;
        }
        let display = self.diagnostic_analyses.entry(task_id).or_default();
        display.busy = true;
        display.error = None;
        display.cached = false;
        display.provider_use = Some(ProviderUse::for_provider(
            provider,
            attempt.generation.route.fallback_from,
        ));
        self.analysis_pending = Some(PendingDiagnosticAnalysis {
            request_id,
            attempt,
        });
        self.status = format!(
            "Interpreting the selected diagnostic · {}…",
            provider_display_name(provider)
        );
        self.resume_analysis_wait(context);
        true
    }
}
