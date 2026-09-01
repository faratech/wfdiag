//! Wiring for agentic chat, the AI report, per-task analysis, issue
//! prioritisation, fix plans, remediation, model catalogs, and subscription
//! CLIs.
//!
//! This is a child module of [`super`] so it can reach the service's private
//! state directly: the whole point of the facade is that one type owns every
//! runtime and one `drain` judges every reply. Splitting the file keeps that
//! true while keeping each half readable.
//!
//! Three rules hold throughout:
//!
//! * **Deltas are coalesced per drain.** However many fragments a provider
//!   streamed, a host sees one `Delta` and exactly one terminal event.
//! * **Nothing here can authorise a remediation.** `approve` hands a freshly
//!   captured snapshot to the broker; the Repair gate lives there.
//! * **Every staleness comparison happens here**, against the newtypes in
//!   [`crate::ids`], never in a host.

use super::{AppService, Internal};
use crate::command::{DispatchOutcome, RejectReason, SubscriptionOperation};
use crate::domain::actions::{
    ReviewSurface, StagedReview, admin_blocked, build_snapshot, proposal_matches, stale_reviews,
};
use crate::domain::ai_intent::{
    IntentAction, IntentReadiness, PendingAiIntent, requires_scan_data,
};
use crate::domain::catalog::{RefreshDecision, auto_discovery_allowed};
use crate::domain::consent::{
    ChatAttempt, ConsentAnswer, FallbackDecision, PendingConsent, PendingPolicyWrite,
    PolicyWriteOutcome, apply_policy_write, decide_fallback,
};
use crate::domain::history::{build_scan_record, scan_kind_history_tag};
use crate::domain::invalidation::Invalidation;
use crate::domain::providers::PendingAiProviderGate;
use crate::domain::subscriptions::{
    AuthAdmission, InstallAdmission, InstallPrompt, admit_auth, admit_install,
    completion_refreshes_models, progress_label, verified_install_path,
};
use crate::event::{
    ActionEvent, AnalysisEvent, AppEvent, ChatEvent, FixPlanEvent, ModelCatalogEvent,
    PrioritizationEvent, ProviderEvent, ReportEvent, SubscriptionEvent,
};
use crate::ids::{Generation, RequestId};
use crate::ports::chat_tools::{ChatScanSnapshot, ChatToolSnapshot};
use crate::snapshot_ai::{CloudFallbackPrompt, FullScanRequest, StagedProposalRequest};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::{Instant, SystemTime};
use wfdiag_native_ai_analysis::{
    AnalysisRoute, AnalysisWorkerEvent, DiagnosticAnalysisGeneration, FixPlanGeneration,
    FixPlanRoute, FixPlanWorkerEvent, IssuePrioritizationGeneration, initial_fix_plan_route,
};
use wfdiag_native_ai_chat::workers::provider_setup::ProviderSetupWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallWorkerEvent;
use wfdiag_native_ai_chat::{
    ChatWorkerEvent, SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionInstallMethod,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderStatus, ModelCatalogRequest, parse_provider_preference,
};
use wfdiag_native_ai_report::{ReportGeneration, ReportScan, ReportWorkerEvent};
use wfdiag_native_history::ComparisonResult;
use wfdiag_native_issues::IssueStatus;
use wfdiag_native_remediation::broker::{
    ActionApproval, ActionPrepareInput, ActionRequest, ActionSnapshot,
};
use wfdiag_native_remediation::runtime::{ActionRunEvent, ActionWorkerEvent};
use wfdiag_native_settings::SettingsUpdate;
use wfdiag_native_system::SystemInfo;

/// How many events one drain accepts from a single AI worker channel.
const AI_DRAIN_LIMIT: usize = 512;

/// One task's in-flight analysis.
#[derive(Clone, Debug)]
pub(super) struct PendingAnalysis {
    pub(super) request: RequestId,
    pub(super) task_id: String,
}

/// One in-flight subscription account operation.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingSubscriptionAuth {
    pub(super) request: RequestId,
    pub(super) provider: SubscriptionAuthProvider,
}

/// One in-flight subscription installation.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingSubscriptionInstall {
    pub(super) request: RequestId,
    pub(super) method: SubscriptionInstallMethod,
}

/// The unsaved provider-setup values a catalog refresh discovers with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogDraft {
    /// An unsaved API key.
    pub api_key: Option<String>,
    /// An unsaved endpoint.
    pub endpoint: Option<String>,
    /// An unsaved CLI path.
    pub cli_path: Option<String>,
}

/// Resolve a provider wire id. The ids are the ones
/// [`AIProvider`]'s `Display` writes, which is also what the settings document
/// and every host payload use.
fn parse_provider(wire: &str) -> Result<AIProvider, RejectReason> {
    const PROVIDERS: [AIProvider; 10] = [
        AIProvider::OpenAI,
        AIProvider::PhiSilica,
        AIProvider::FoundryLocal,
        AIProvider::Ollama,
        AIProvider::CustomOpenAI,
        AIProvider::CodexCli,
        AIProvider::ClaudeCode,
        AIProvider::Anthropic,
        AIProvider::Gemini,
        AIProvider::DeepSeek,
    ];
    PROVIDERS
        .into_iter()
        .find(|provider| provider.to_string() == wire)
        .ok_or_else(|| RejectReason::Invalid {
            detail: format!("'{wire}' is not a known AI provider"),
        })
}

fn parse_subscription_provider(wire: &str) -> Result<SubscriptionAuthProvider, RejectReason> {
    match parse_provider(wire)? {
        AIProvider::CodexCli => Ok(SubscriptionAuthProvider::Codex),
        AIProvider::ClaudeCode => Ok(SubscriptionAuthProvider::ClaudeCode),
        other => Err(RejectReason::Invalid {
            detail: format!("{other} is not a subscription CLI"),
        }),
    }
}

fn unavailable(domain: &'static str, detail: Option<&str>) -> RejectReason {
    RejectReason::WorkerUnavailable {
        worker: crate::command::WorkerKind::Provider,
        detail: detail.map_or_else(
            || format!("the {domain} runtime is unavailable"),
            std::string::ToString::to_string,
        ),
    }
}

impl AppService {
    // ---- start-up bookkeeping -------------------------------------------

    /// Publish every AI runtime that refused to start, so its commands are
    /// refused with the real reason instead of a generic one.
    pub(super) fn record_ai_worker_errors(&mut self) {
        for (domain, detail) in self.workers.ai_errors.clone() {
            if domain == "remediation" {
                self.snapshot.actions.error = Some(detail.clone());
            }
            self.snapshot
                .record_worker_error(crate::command::WorkerKind::Provider, detail);
        }
    }

    /// Adopt the previews and runs the action runtime had before this service
    /// existed. Only unconsumed, unexpired previews survive.
    pub(super) fn adopt_rehydrated_actions(&mut self) {
        self.snapshot.actions.review = self.workers.rehydrated_proposals.first().cloned();
        self.snapshot
            .actions
            .history
            .clone_from(&self.workers.rehydrated_runs);
        self.snapshot
            .actions
            .active_run
            .clone_from(&self.workers.rehydrated_active_run);
    }

    /// How much AI work is outstanding, for the reply watcher's tick.
    pub(super) fn ai_outstanding(&self) -> usize {
        usize::from(self.chat_pending.is_some())
            + usize::from(self.report_pending.is_some())
            + usize::from(self.analysis_pending.is_some())
            + usize::from(self.prioritization_pending.is_some())
            + usize::from(self.fix_plan_pending.is_some())
            + usize::from(self.action_pending.is_some())
            + usize::from(self.catalog_pending.is_some())
            + usize::from(self.subscription_auth_pending.is_some())
            + usize::from(self.subscription_install_pending.is_some())
            + usize::from(self.pending_intent.is_some())
            + usize::from(self.snapshot.actions.active_run.is_some())
    }

    /// Version the committed evidence every derived projection is bound to.
    pub(super) fn advance_evidence_generation(&mut self) {
        if let Some(generation) = self.evidence_generations.issue() {
            self.evidence_generation = generation;
        }
    }

    /// Drop the derived projections a transaction invalidated, cancelling any
    /// generation still in flight for them.
    pub(super) fn apply_derived_invalidation(&mut self, invalidation: Invalidation) {
        if !invalidation.any() {
            return;
        }
        // Each projection reports `Invalidated` only when there was something
        // to drop: a replacement scan invalidates when its transaction opens
        // *and* when it commits, and a host must not be told twice that
        // nothing changed.
        if invalidation.report {
            let had = self.report_pending.is_some() || self.snapshot.ai.report.text.is_some();
            if let Some(request) = self.report_pending.take()
                && let Some(runtime) = self.workers.report.as_ref()
            {
                let _ = runtime.cancel(request.get());
            }
            self.report_pending = None;
            self.snapshot.ai.report = crate::snapshot_ai::ReportSnapshot::default();
            if had {
                self.queue.push(AppEvent::Report(ReportEvent::Invalidated));
            }
        }
        if invalidation.analyses {
            let had = self.analysis_pending.is_some() || !self.snapshot.ai.analyses.is_empty();
            if let Some(pending) = self.analysis_pending.take()
                && let Some(runtime) = self.workers.analysis.as_ref()
            {
                let _ = runtime.cancel(pending.request.get());
            }
            self.snapshot.ai.analyses.clear();
            if had {
                self.queue
                    .push(AppEvent::Analysis(AnalysisEvent::Invalidated));
            }
        }
        if invalidation.prioritization {
            let had = self.prioritization_pending.is_some()
                || self.snapshot.ai.prioritization.text.is_some();
            if let Some((request, _)) = self.prioritization_pending.take()
                && let Some(runtime) = self.workers.analysis.as_ref()
            {
                let _ = runtime.cancel(request.get());
            }
            self.snapshot.ai.prioritization = crate::snapshot_ai::PrioritizationSnapshot::default();
            if had {
                self.queue
                    .push(AppEvent::Prioritization(PrioritizationEvent::Invalidated));
            }
        }
        if invalidation.fix_plan {
            let had = self.fix_plan_pending.is_some() || self.snapshot.ai.fix_plan.plan.is_some();
            if let Some(request) = self.fix_plan_pending.take()
                && let Some(runtime) = self.workers.fix_plan.as_ref()
            {
                let _ = runtime.cancel(request.get());
            }
            self.snapshot.ai.fix_plan = crate::snapshot_ai::FixPlanSnapshot::default();
            if had {
                self.queue
                    .push(AppEvent::FixPlan(FixPlanEvent::Invalidated));
            }
        }
    }

    /// Discard staged previews that no longer describe the committed evidence.
    ///
    /// Both surfaces are judged against **one** snapshot: capturing two would
    /// let evidence land between them.
    pub(super) fn reconcile_staged_reviews(&mut self) {
        let snapshot = self.action_snapshot();
        let stale = stale_reviews(
            self.snapshot.actions.review.as_ref(),
            self.snapshot.actions.repair_confirmation.as_ref(),
            &snapshot,
        );
        if !stale.any() {
            return;
        }
        let mut dropped = Vec::new();
        if stale.review
            && let Some(proposal) = self.snapshot.actions.review.take()
        {
            dropped.push(proposal.proposal_id);
        }
        if stale.repair
            && let Some(proposal) = self.snapshot.actions.repair_confirmation.take()
        {
            dropped.push(proposal.proposal_id);
        }
        for proposal_id in dropped {
            if let Some(runtime) = self.workers.actions.as_ref() {
                let _ = runtime.discard(proposal_id.clone());
            }
            self.queue
                .push(AppEvent::Action(ActionEvent::Discarded { proposal_id }));
        }
    }

    // ---- the pending-intent gate ----------------------------------------

    fn readiness(&self) -> IntentReadiness<'_> {
        IntentReadiness {
            ai_enabled: self.snapshot.settings.ai_enabled,
            provider_loading: self.snapshot.provider_loading,
            provider_status: self.snapshot.provider_status.as_ref(),
            scan_busy: self.snapshot.scan_busy(),
            chat_busy: self.chat_pending.is_some(),
            report_busy: self.report_pending.is_some(),
        }
    }

    /// Re-evaluate the parked AI request after every prerequisite settles.
    pub(super) fn resume_pending_intent(&mut self) {
        let Some(intent) = self.pending_intent.clone() else {
            self.snapshot.ai.pending_intent = None;
            return;
        };
        if self.terminating {
            return;
        }
        match crate::domain::ai_intent::evaluate(&intent, self.readiness()) {
            IntentAction::Run => {
                self.pending_intent = None;
                self.snapshot.ai.pending_intent = None;
                self.snapshot.ai.preparation_error = None;
                match intent {
                    PendingAiIntent::Chat { prompt } => {
                        let _ = self.begin_chat_turn(prompt);
                    }
                    PendingAiIntent::Report { force_refresh } => {
                        let _ = self.begin_report(force_refresh);
                    }
                }
            }
            IntentAction::Wait { .. } => {}
            IntentAction::RefreshProviders => {
                self.snapshot.ai.preparation_error = None;
                // A probe that cannot even be queued would otherwise be
                // retried on every drain forever, so it ends the parked
                // request instead of spinning.
                if !self.request_provider_status().is_accepted() {
                    let reason = "Native AI provider discovery is unavailable".to_string();
                    self.pending_intent = None;
                    self.snapshot.ai.pending_intent = None;
                    self.snapshot.ai.preparation_error = Some(reason.clone());
                    match intent {
                        PendingAiIntent::Chat { .. } => {
                            self.queue
                                .push(AppEvent::Chat(ChatEvent::Failed { message: reason }));
                        }
                        PendingAiIntent::Report { .. } => {
                            self.queue
                                .push(AppEvent::Report(ReportEvent::Failed { message: reason }));
                        }
                    }
                }
            }
            IntentAction::Fail { reason } => {
                self.pending_intent = None;
                self.snapshot.ai.pending_intent = None;
                self.snapshot.ai.preparation_error = Some(reason.clone());
                match intent {
                    PendingAiIntent::Chat { .. } => {
                        self.queue
                            .push(AppEvent::Chat(ChatEvent::Failed { message: reason }));
                    }
                    PendingAiIntent::Report { .. } => {
                        self.queue
                            .push(AppEvent::Report(ReportEvent::Failed { message: reason }));
                    }
                }
            }
        }
    }

    fn park_intent(&mut self, intent: &PendingAiIntent, reason: &str) -> DispatchOutcome {
        self.snapshot.ai.pending_intent = Some(intent.clone());
        self.snapshot.ai.preparation_error = None;
        self.pending_intent = Some(intent.clone());
        match *intent {
            PendingAiIntent::Chat { .. } => self.queue.push(AppEvent::Chat(ChatEvent::Deferred {
                reason: reason.to_string(),
            })),
            PendingAiIntent::Report { .. } => {
                self.queue.push(AppEvent::Report(ReportEvent::Deferred {
                    reason: reason.to_string(),
                }));
            }
        }
        DispatchOutcome::accepted()
    }

    fn provider_status(&self) -> Option<&AIProviderStatus> {
        self.snapshot
            .provider_status
            .as_ref()
            .filter(|status| status.active_provider != AIProvider::None)
    }

    // ---- chat -------------------------------------------------------------

    pub(super) fn chat_send(&mut self, prompt: &str) -> DispatchOutcome {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return DispatchOutcome::Ignored {
                detail: "an empty prompt is not a question",
            };
        }
        if self.chat_consent.is_some() || self.chat_policy_write.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "answer the cloud-fallback prompt before sending another message"
                    .to_string(),
            });
        }
        if self.chat_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a chat turn is already streaming".to_string(),
            });
        }
        if self.workers.chat.is_none() {
            return DispatchOutcome::Rejected(unavailable("chat", self.workers.ai_error("chat")));
        }
        match PendingAiProviderGate::evaluate(
            self.snapshot.settings.ai_enabled,
            self.snapshot.provider_loading,
            self.snapshot.provider_status.as_ref(),
        ) {
            PendingAiProviderGate::Ready => {}
            PendingAiProviderGate::Waiting => {
                return self.park_intent(
                    &PendingAiIntent::Chat { prompt },
                    "Checking AI providers before continuing…",
                );
            }
            PendingAiProviderGate::Refresh => {
                let _ = self.request_provider_status();
                return self.park_intent(
                    &PendingAiIntent::Chat { prompt },
                    "Checking AI providers before continuing…",
                );
            }
            PendingAiProviderGate::Disabled => {
                return DispatchOutcome::Rejected(RejectReason::NotReady {
                    detail: "Enable AI insights in Settings before sending".to_string(),
                });
            }
            PendingAiProviderGate::Unavailable => {
                return DispatchOutcome::Rejected(RejectReason::NotReady {
                    detail: "Set up an available AI provider before sending".to_string(),
                });
            }
        }
        // A question about *this* PC needs evidence. A general Windows
        // question does not justify running diagnostics on the user's machine.
        if requires_scan_data(&prompt) && self.scan.snapshot().results.is_empty() {
            let scan_busy = self.snapshot.scan_busy();
            let outcome = self.park_intent(
                &PendingAiIntent::Chat {
                    prompt: prompt.clone(),
                },
                if scan_busy {
                    "Waiting for the active scan before asking the AI assistant…"
                } else {
                    "Running a Quick Scan before asking the AI assistant…"
                },
            );
            if !scan_busy {
                let _ = self.start_scan(wfdiag_native_diagnostics::ScanKind::Quick, None);
            }
            return outcome;
        }
        self.begin_chat_turn(prompt)
    }

    fn begin_chat_turn(&mut self, prompt: String) -> DispatchOutcome {
        let Some(status) = self.provider_status().cloned() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Set up an available AI provider before sending".to_string(),
            });
        };
        let preference = parse_provider_preference(&self.snapshot.settings.preferred_ai_provider);
        let Some(turn) = self.chat_turns.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let Some(attempt) =
            ChatAttempt::plan(turn.get(), prompt, preference, status.availability())
        else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Set up an available AI provider before sending".to_string(),
            });
        };
        self.chat_turn = Some(turn);
        self.snapshot.ai.chat = crate::snapshot_ai::ChatSnapshot::default();
        if self.dispatch_chat_attempt(None, attempt) {
            DispatchOutcome::accepted()
        } else {
            DispatchOutcome::Rejected(unavailable("chat", self.workers.ai_error("chat")))
        }
    }

    fn dispatch_chat_attempt(&mut self, previous: Option<RequestId>, attempt: ChatAttempt) -> bool {
        let Some(runtime) = self.workers.chat.as_ref() else {
            return false;
        };
        let Some(request) = self.requests.issue() else {
            return false;
        };
        let provider = attempt.current_provider;
        let fallback_from = attempt.fallback_from();
        let allow_fallback = attempt.allows_fallback();
        let evidence = self.chat_tool_snapshot();
        let queued = previous.map_or_else(
            || {
                runtime.send_attempt(
                    request.get(),
                    attempt.prompt.clone(),
                    provider,
                    fallback_from,
                    allow_fallback,
                    evidence.clone(),
                )
            },
            |previous| {
                runtime.retry_attempt(
                    previous.get(),
                    request.get(),
                    attempt.prompt.clone(),
                    provider,
                    fallback_from,
                    allow_fallback,
                    evidence.clone(),
                )
            },
        );
        if !queued {
            return false;
        }
        self.chat_pending = Some(request);
        self.chat_attempt = Some(attempt);
        self.chat_consent = None;
        self.snapshot.ai.chat.streaming = true;
        self.snapshot.ai.chat.provider = Some(provider.to_string());
        self.snapshot.ai.chat.error = None;
        self.snapshot.ai.chat.cloud_fallback = None;
        self.queue.push(AppEvent::Chat(ChatEvent::Started {
            provider: provider.to_string(),
        }));
        true
    }

    pub(super) fn chat_cancel(&mut self) -> DispatchOutcome {
        if let Some(write) = self.chat_policy_write.take() {
            self.finish_chat_cancelled();
            drop(write);
            return DispatchOutcome::accepted();
        }
        if self.chat_consent.take().is_some() {
            self.finish_chat_cancelled();
            return DispatchOutcome::accepted();
        }
        if self.chat_pending.is_none() {
            return DispatchOutcome::Ignored {
                detail: "no chat turn is streaming",
            };
        }
        let cancelled = self
            .workers
            .chat
            .as_ref()
            .is_some_and(wfdiag_native_ai_chat::NativeChatRuntime::cancel);
        if cancelled {
            DispatchOutcome::accepted()
        } else {
            self.finish_chat_cancelled();
            DispatchOutcome::accepted()
        }
    }

    pub(super) fn chat_reset(&mut self) -> DispatchOutcome {
        let Some(runtime) = self.workers.chat.as_ref() else {
            return DispatchOutcome::Rejected(unavailable("chat", self.workers.ai_error("chat")));
        };
        if !runtime.new_session() {
            return DispatchOutcome::Rejected(unavailable("chat", self.workers.ai_error("chat")));
        }
        self.chat_pending = None;
        self.chat_attempt = None;
        self.chat_consent = None;
        self.chat_policy_write = None;
        self.snapshot.ai.chat = crate::snapshot_ai::ChatSnapshot::default();
        self.queue.push(AppEvent::Chat(ChatEvent::SessionReset));
        DispatchOutcome::accepted()
    }

    pub(super) fn cloud_fallback_decision(&mut self, allow: bool) -> DispatchOutcome {
        let Some(consent) = self.chat_consent.take() else {
            return DispatchOutcome::Ignored {
                detail: "no cloud-fallback prompt is open",
            };
        };
        let answer = ConsentAnswer::from_allow(allow);
        let policy = answer.policy();
        // The answer is applied to the turn only once the preference write
        // lands: a failed write must re-arm the prompt, not silently lose the
        // message the user is waiting for.
        let outcome = self.settings_update(SettingsUpdate::CloudFallbackPolicy(policy));
        match outcome {
            DispatchOutcome::Accepted {
                request: Some(request),
            } => {
                self.snapshot.ai.chat.cloud_fallback = Some(CloudFallbackPrompt {
                    candidate: consent.candidate.to_string(),
                    reason: consent.reason.clone(),
                    saving: true,
                });
                self.chat_policy_write = Some((request, PendingPolicyWrite { policy, consent }));
                DispatchOutcome::accepted_request(request)
            }
            other => {
                self.chat_consent = Some(consent);
                other
            }
        }
    }

    /// Apply a completed cloud-fallback preference write.
    pub(super) fn resolve_cloud_fallback_write(
        &mut self,
        request: RequestId,
        persisted: Result<wfdiag_native_settings::CloudFallbackPolicy, ()>,
    ) {
        let Some((pending_request, pending)) = self.chat_policy_write.take() else {
            return;
        };
        if pending_request != request {
            self.chat_policy_write = Some((pending_request, pending));
            return;
        }
        match apply_policy_write(pending, persisted) {
            PolicyWriteOutcome::Continue(consent) => {
                let PendingConsent {
                    mut attempt,
                    candidate,
                    ..
                } = *consent;
                attempt.advance_to(candidate);
                let previous = self.chat_pending;
                self.snapshot.ai.chat.cloud_fallback = None;
                if !self.dispatch_chat_attempt(previous, attempt) {
                    self.finish_chat_failure("The native AI chat queue is unavailable".to_string());
                }
            }
            PolicyWriteOutcome::Refuse { message, .. } => {
                self.snapshot.ai.chat.cloud_fallback = None;
                self.finish_chat_failure(message);
            }
            PolicyWriteOutcome::Reprompt(consent) => {
                let consent = *consent;
                self.snapshot.ai.chat.cloud_fallback = Some(CloudFallbackPrompt {
                    candidate: consent.candidate.to_string(),
                    reason: consent.reason.clone(),
                    saving: false,
                });
                self.queue
                    .push(AppEvent::Chat(ChatEvent::CloudFallbackRequired {
                        candidate: consent.candidate.to_string(),
                        reason: consent.reason.clone(),
                    }));
                self.chat_consent = Some(consent);
            }
        }
    }

    fn finish_chat_failure(&mut self, message: String) {
        self.chat_pending = None;
        self.chat_attempt = None;
        self.chat_consent = None;
        self.snapshot.ai.chat.streaming = false;
        self.snapshot.ai.chat.error = Some(message.clone());
        self.snapshot.ai.chat.finish_reason = Some("error".to_string());
        self.queue
            .push(AppEvent::Chat(ChatEvent::Failed { message }));
    }

    fn finish_chat_cancelled(&mut self) {
        self.chat_pending = None;
        self.chat_attempt = None;
        self.chat_consent = None;
        self.snapshot.ai.chat.streaming = false;
        self.snapshot.ai.chat.cloud_fallback = None;
        self.snapshot.ai.chat.finish_reason = Some("cancelled".to_string());
        self.queue.push(AppEvent::Chat(ChatEvent::Cancelled));
    }

    /// Capture everything one turn may read, once.
    fn chat_tool_snapshot(&self) -> ChatToolSnapshot {
        let architecture = self.snapshot.architecture.as_ref().map_or_else(
            || "Unknown".to_string(),
            |arch| arch.emulation_status.clone(),
        );
        let identity = self.host_identity();
        let system_overview = Some(format!(
            "Computer name: {}\nOperating system: {}\nArchitecture: {architecture}\nElevated: {}",
            identity.computer_name,
            identity.os_version,
            if identity.is_admin { "yes" } else { "no" }
        ));
        let scan = self.scan.snapshot();
        let session_id = scan.effective_session_id();
        let selected_tasks = if scan.task_ids.is_empty() {
            scan.results
                .iter()
                .map(|result| result.task_id.clone())
                .collect()
        } else {
            scan.task_ids.clone()
        };
        let running = self.snapshot.scan_busy();
        let chat_scan = session_id.as_ref().map(|session_id| ChatScanSnapshot {
            session_id: session_id.clone(),
            started_at: SystemTime::now(),
            scan_kind: scan
                .scan_kind
                .unwrap_or(wfdiag_native_diagnostics::ScanKind::Targeted),
            selected_tasks,
            results: scan.results.clone(),
            running,
        });
        let current_history_scan = session_id.as_ref().and_then(|session_id| {
            (!scan.results.is_empty() && !running).then(|| {
                build_scan_record(
                    session_id.clone(),
                    &identity,
                    &scan.results,
                    scan.duration_ms,
                    scan.scan_kind
                        .map_or("Diagnostic Scan", scan_kind_history_tag)
                        .to_string(),
                    self.ports.environment.now(),
                )
            })
        });
        ChatToolSnapshot {
            system_overview,
            scan: chat_scan,
            current_history_scan,
            issues: self.snapshot.issues.clone(),
            history: self.snapshot.history.summaries.clone(),
            live_stats: self.snapshot.monitor.latest.clone(),
            remediations: self.snapshot.remediations.clone(),
            network_grounding_enabled: self.snapshot.settings.network_grounding_enabled,
        }
    }

    fn host_identity(&self) -> SystemInfo {
        self.snapshot
            .system_info
            .clone()
            .unwrap_or_else(|| SystemInfo {
                computer_name: "Unknown".to_string(),
                os_version: "Unknown".to_string(),
                is_admin: false,
            })
    }

    // ---- report ----------------------------------------------------------

    pub(super) fn generate_report(&mut self, force_refresh: bool) -> DispatchOutcome {
        if self.report_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a report is already being generated".to_string(),
            });
        }
        if self.workers.report.is_none() {
            return DispatchOutcome::Rejected(unavailable(
                "report",
                self.workers.ai_error("report"),
            ));
        }
        match PendingAiProviderGate::evaluate(
            self.snapshot.settings.ai_enabled,
            self.snapshot.provider_loading,
            self.snapshot.provider_status.as_ref(),
        ) {
            PendingAiProviderGate::Ready => {}
            PendingAiProviderGate::Waiting => {
                return self.park_intent(
                    &PendingAiIntent::Report { force_refresh },
                    "Checking AI providers before continuing…",
                );
            }
            PendingAiProviderGate::Refresh => {
                let _ = self.request_provider_status();
                return self.park_intent(
                    &PendingAiIntent::Report { force_refresh },
                    "Checking AI providers before continuing…",
                );
            }
            PendingAiProviderGate::Disabled => {
                return DispatchOutcome::Rejected(RejectReason::NotReady {
                    detail: "Enable AI insights in Settings before generating a report".to_string(),
                });
            }
            PendingAiProviderGate::Unavailable => {
                return DispatchOutcome::Rejected(RejectReason::NotReady {
                    detail: "Set up an available AI provider before generating".to_string(),
                });
            }
        }
        if self.scan.snapshot().results.is_empty() {
            let scan_busy = self.snapshot.scan_busy();
            let outcome = self.park_intent(
                &PendingAiIntent::Report { force_refresh },
                if scan_busy {
                    "Waiting for the active scan before generating the AI report…"
                } else {
                    "Running a Quick Scan before generating the AI report…"
                },
            );
            if !scan_busy {
                let _ = self.start_scan(wfdiag_native_diagnostics::ScanKind::Quick, None);
            }
            return outcome;
        }
        self.begin_report(force_refresh)
    }

    fn begin_report(&mut self, force_refresh: bool) -> DispatchOutcome {
        let Some(status) = self.provider_status().cloned() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Set up an available AI provider before generating".to_string(),
            });
        };
        let Some(session_id) = self.scan.snapshot().effective_session_id() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Run a scan before generating a report".to_string(),
            });
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let generation = ReportGeneration {
            scan: ReportScan {
                session_id: session_id.clone(),
                results: self.scan.snapshot().evidence(),
            },
            provider: status.active_provider,
            availability: status.availability(),
            comparison: None,
            force_refresh,
        };
        self.report_pending = Some(request);
        self.snapshot.ai.report = crate::snapshot_ai::ReportSnapshot {
            generating: true,
            source_session_id: Some(session_id),
            ..crate::snapshot_ai::ReportSnapshot::default()
        };
        // The "Changed since last scan" section needs the newest stored scan.
        // Resolving it here keeps the report worker free of history I/O, and a
        // baseline that cannot be resolved is a soft failure: the report is
        // still worth generating without it.
        if !self.request_report_baseline(request, &generation) {
            self.start_report_generation(request, generation, None);
        }
        DispatchOutcome::accepted_request(request)
    }

    fn request_report_baseline(
        &mut self,
        request: RequestId,
        generation: &ReportGeneration,
    ) -> bool {
        let Some(history) = self.workers.history.as_ref().map(Arc::clone) else {
            return false;
        };
        let identity = self.host_identity();
        let scan = self.scan.snapshot();
        let record = build_scan_record(
            generation.scan.session_id.clone(),
            &identity,
            &scan.results,
            scan.duration_ms,
            scan.scan_kind
                .map_or("Diagnostic Scan", scan_kind_history_tag)
                .to_string(),
            self.ports.environment.now(),
        );
        let Ok(reply) = history.request_compare_current_to_latest(Arc::new(record)) else {
            return false;
        };
        let generation = generation.clone();
        self.replies.register(
            crate::command::WorkerKind::History,
            request,
            reply,
            move |result| Internal::ReportBaseline {
                request,
                generation: Box::new(generation),
                comparison: match result {
                    Ok(Ok(comparison)) => comparison.map(Box::new),
                    _ => None,
                },
            },
        );
        true
    }

    pub(super) fn start_report_generation(
        &mut self,
        request: RequestId,
        mut generation: ReportGeneration,
        comparison: Option<ComparisonResult>,
    ) {
        if self.report_pending != Some(request) {
            return;
        }
        generation.comparison = comparison;
        let queued = self
            .workers
            .report
            .as_ref()
            .is_some_and(|runtime| runtime.generate(request.get(), generation));
        if !queued {
            self.report_pending = None;
            self.snapshot.ai.report.generating = false;
            let message = "The native report queue is unavailable".to_string();
            self.snapshot.ai.report.error = Some(message.clone());
            self.queue
                .push(AppEvent::Report(ReportEvent::Failed { message }));
        }
    }

    pub(super) fn cancel_report(&mut self) -> DispatchOutcome {
        let Some(request) = self.report_pending else {
            return DispatchOutcome::Ignored {
                detail: "no report is being generated",
            };
        };
        let cancelled = self
            .workers
            .report
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(request.get()));
        if !cancelled {
            self.report_pending = None;
            self.snapshot.ai.report.generating = false;
            self.queue.push(AppEvent::Report(ReportEvent::Cancelled));
        }
        DispatchOutcome::accepted()
    }

    // ---- analysis, prioritisation, fix plan ------------------------------

    fn analysis_route(&self) -> Result<AnalysisRoute, RejectReason> {
        let status = self.provider_status().ok_or(RejectReason::NotReady {
            detail: "Set up an available AI provider before interpreting".to_string(),
        })?;
        let preference = parse_provider_preference(&self.snapshot.settings.preferred_ai_provider);
        Ok(AnalysisRoute {
            preference,
            provider: status.active_provider,
            availability: status.availability(),
            fallback_from: None,
        })
    }

    pub(super) fn analyze_diagnostic(
        &mut self,
        task_id: String,
        force_refresh: bool,
    ) -> DispatchOutcome {
        if !self.snapshot.settings.ai_enabled {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Enable AI insights in Settings before interpreting".to_string(),
            });
        }
        if self.analysis_pending.is_some() || self.prioritization_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "another one-shot AI analysis is already running".to_string(),
            });
        }
        let scan = self.scan.snapshot();
        let Some(result) = scan
            .results
            .iter()
            .find(|result| result.task_id == task_id)
            .cloned()
        else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Run a diagnostic scan before asking for an interpretation".to_string(),
            });
        };
        let route = match self.analysis_route() {
            Ok(route) => route,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(runtime) = self.workers.analysis.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "analysis",
                self.workers.ai_error("analysis"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let task_name = self
            .snapshot
            .catalog
            .iter()
            .find(|task| task.id == task_id)
            .map_or_else(|| task_id.clone(), |task| task.name.clone());
        let generation = DiagnosticAnalysisGeneration {
            session_id: result.session_id.clone(),
            task_id: task_id.clone(),
            task_name,
            diagnostic_result: Arc::clone(&result.result),
            route,
            network_grounding_enabled: self.snapshot.settings.network_grounding_enabled,
            force_refresh,
        };
        if !runtime.generate(request.get(), generation) {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native diagnostic AI queue is unavailable".to_string(),
            });
        }
        self.analysis_pending = Some(PendingAnalysis {
            request,
            task_id: task_id.clone(),
        });
        let entry = self.snapshot.ai.analyses.entry(task_id).or_default();
        entry.busy = true;
        entry.error = None;
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn cancel_analysis(&mut self) -> DispatchOutcome {
        let Some(pending) = self.analysis_pending.as_ref() else {
            return DispatchOutcome::Ignored {
                detail: "no analysis is running",
            };
        };
        let request = pending.request;
        let cancelled = self
            .workers
            .analysis
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(request.get()));
        if !cancelled
            && let Some(pending) = self.analysis_pending.take()
            && let Some(entry) = self.snapshot.ai.analyses.get_mut(&pending.task_id)
        {
            entry.busy = false;
        }
        DispatchOutcome::accepted()
    }

    pub(super) fn prioritize_issues(&mut self, force_refresh: bool) -> DispatchOutcome {
        if !self.snapshot.settings.ai_enabled {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Enable AI insights in Settings before prioritizing issues".to_string(),
            });
        }
        if self.analysis_pending.is_some() || self.prioritization_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "another one-shot AI analysis is already running".to_string(),
            });
        }
        let Some(session_id) = self.issues.committed_session_id().map(str::to_string) else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Run a diagnostic scan before prioritizing issues".to_string(),
            });
        };
        let detected: Vec<_> = self
            .snapshot
            .issues
            .iter()
            .filter(|issue| issue.status == IssueStatus::Detected)
            .collect();
        if detected.is_empty() {
            return DispatchOutcome::Ignored {
                detail: "no detected issues need prioritization",
            };
        }
        let issues_data = serde_json::to_string(&detected).unwrap_or_else(|_| "[]".to_string());
        let route = match self.analysis_route() {
            Ok(route) => route,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        let Some(runtime) = self.workers.analysis.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "analysis",
                self.workers.ai_error("analysis"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let generation = IssuePrioritizationGeneration {
            session_id,
            issues_data,
            route,
            network_grounding_enabled: self.snapshot.settings.network_grounding_enabled,
            force_refresh,
        };
        if !runtime.prioritize(request.get(), generation) {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native AI issue-prioritization queue is unavailable".to_string(),
            });
        }
        self.prioritization_pending = Some((request, self.evidence_generation));
        self.snapshot.ai.prioritization = crate::snapshot_ai::PrioritizationSnapshot {
            busy: true,
            ..crate::snapshot_ai::PrioritizationSnapshot::default()
        };
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn generate_fix_plan(&mut self) -> DispatchOutcome {
        if !self.snapshot.settings.ai_enabled {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Enable AI insights in Settings before proposing a fix plan".to_string(),
            });
        }
        if self.fix_plan_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a fix plan is already being generated".to_string(),
            });
        }
        let detected: Vec<_> = self
            .snapshot
            .issues
            .iter()
            .filter(|issue| issue.detected && issue.status == IssueStatus::Detected)
            .cloned()
            .collect();
        if detected.is_empty() {
            return DispatchOutcome::Ignored {
                detail: "no detected issues need a fix plan",
            };
        }
        let Some(status) = self.provider_status().cloned() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Set up an available AI provider before proposing a fix plan".to_string(),
            });
        };
        let preference = parse_provider_preference(&self.snapshot.settings.preferred_ai_provider);
        let route: FixPlanRoute =
            initial_fix_plan_route(preference, status.active_provider, status.availability());
        if route.provider == AIProvider::None || !route.availability.contains(route.provider) {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "Set up an available AI provider before proposing a fix plan".to_string(),
            });
        }
        let snapshot = self.action_snapshot();
        let Some(runtime) = self.workers.fix_plan.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "fix plan",
                self.workers.ai_error("fix plan"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let generation = FixPlanGeneration {
            detected_issues: detected,
            route,
            scan_fingerprint: snapshot.scan_fingerprint,
            catalog_fingerprint: snapshot.catalog_fingerprint,
        };
        if !runtime.generate(request.get(), generation) {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native AI fix-plan queue is unavailable".to_string(),
            });
        }
        self.fix_plan_pending = Some(request);
        self.snapshot.ai.fix_plan = crate::snapshot_ai::FixPlanSnapshot {
            busy: true,
            ..crate::snapshot_ai::FixPlanSnapshot::default()
        };
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn cancel_fix_plan(&mut self) -> DispatchOutcome {
        let Some(request) = self.fix_plan_pending else {
            return DispatchOutcome::Ignored {
                detail: "no fix plan is being generated",
            };
        };
        let cancelled = self
            .workers
            .fix_plan
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(request.get()));
        if !cancelled {
            self.fix_plan_pending = None;
            self.snapshot.ai.fix_plan.busy = false;
            self.queue.push(AppEvent::FixPlan(FixPlanEvent::Cancelled));
        }
        DispatchOutcome::accepted()
    }

    // ---- remediation ------------------------------------------------------

    /// The authoritative snapshot every prepare and approve is validated
    /// against. Captured fresh at each boundary, never cached.
    pub(super) fn action_snapshot(&self) -> ActionSnapshot {
        build_snapshot(
            self.issues.committed_session_id(),
            self.evidence_generation.get(),
            &self.scan.snapshot().results,
            &self.snapshot.issues,
            self.snapshot.is_admin(),
        )
    }

    pub(super) fn prepare_remediation(
        &mut self,
        remediation_id: String,
        issue_id: Option<String>,
    ) -> DispatchOutcome {
        if self.action_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a remediation request is already active".to_string(),
            });
        }
        let snapshot = self.action_snapshot();
        let Some(runtime) = self.workers.actions.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "remediation",
                self.workers.ai_error("remediation"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let input = ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id,
                issue_id,
            }],
            expected_scan_fingerprint: Some(snapshot.scan_fingerprint.clone()),
            expected_catalog_fingerprint: Some(snapshot.catalog_fingerprint.clone()),
        };
        if !runtime.prepare(request.get(), input, snapshot) {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native remediation preview queue is unavailable".to_string(),
            });
        }
        self.action_pending = Some(request);
        self.action_pending_review = None;
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn approve_action(
        &mut self,
        proposal_id: &str,
        confirm_repair: bool,
    ) -> DispatchOutcome {
        if self.action_pending.is_some() {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "a remediation request is already active".to_string(),
            });
        }
        let (proposal, surface) = if self
            .snapshot
            .actions
            .repair_confirmation
            .as_ref()
            .is_some_and(|proposal| proposal.proposal_id == proposal_id)
        {
            (
                self.snapshot.actions.repair_confirmation.take(),
                ReviewSurface::RepairConfirmation,
            )
        } else if self
            .snapshot
            .actions
            .review
            .as_ref()
            .is_some_and(|proposal| proposal.proposal_id == proposal_id)
        {
            (self.snapshot.actions.review.take(), ReviewSurface::Review)
        } else {
            (None, ReviewSurface::Review)
        };
        let Some(proposal) = proposal else {
            return DispatchOutcome::Rejected(RejectReason::Invalid {
                detail: "that action preview is not staged for review".to_string(),
            });
        };
        let snapshot = self.action_snapshot();
        if admin_blocked(&proposal, snapshot.is_admin) {
            let review = StagedReview { proposal, surface };
            self.restore_review(review);
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "This action requires administrator rights".to_string(),
            });
        }
        let approval = if confirm_repair {
            ActionApproval::RepairConfirmed
        } else {
            ActionApproval::Reviewed
        };
        let Some(runtime) = self.workers.actions.as_ref() else {
            let review = StagedReview { proposal, surface };
            self.restore_review(review);
            return DispatchOutcome::Rejected(unavailable(
                "remediation",
                self.workers.ai_error("remediation"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            let review = StagedReview { proposal, surface };
            self.restore_review(review);
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        if !runtime.approve(
            request.get(),
            proposal.proposal_id.clone(),
            snapshot,
            approval,
        ) {
            let review = StagedReview { proposal, surface };
            self.restore_review(review);
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native remediation approval queue is unavailable".to_string(),
            });
        }
        self.action_pending = Some(request);
        self.action_pending_review = Some(StagedReview { proposal, surface });
        DispatchOutcome::accepted_request(request)
    }

    fn restore_review(&mut self, review: StagedReview) {
        match review.surface {
            ReviewSurface::Review => self.snapshot.actions.review = Some(review.proposal),
            ReviewSurface::RepairConfirmation => {
                self.snapshot.actions.repair_confirmation = Some(review.proposal);
            }
        }
    }

    /// Put a bounced preview back only if it still describes the committed
    /// evidence and the broker still holds it.
    fn restore_review_if_current(&mut self, review: StagedReview) -> bool {
        let snapshot = self.action_snapshot();
        let still_staged = self.workers.actions.as_ref().is_some_and(|runtime| {
            runtime
                .pending_proposals()
                .iter()
                .any(|proposal| proposal.proposal_id == review.proposal.proposal_id)
        });
        if still_staged && proposal_matches(&review.proposal, &snapshot) {
            self.restore_review(review);
            return true;
        }
        if still_staged && let Some(runtime) = self.workers.actions.as_ref() {
            let _ = runtime.discard(review.proposal.proposal_id);
        }
        false
    }

    pub(super) fn discard_proposal(&mut self, proposal_id: &str) -> DispatchOutcome {
        let matches = |proposal: &wfdiag_native_remediation::broker::ActionProposal| {
            proposal.proposal_id == proposal_id
        };
        if self.snapshot.actions.review.as_ref().is_some_and(matches) {
            self.snapshot.actions.review = None;
        } else if self
            .snapshot
            .actions
            .repair_confirmation
            .as_ref()
            .is_some_and(matches)
        {
            self.snapshot.actions.repair_confirmation = None;
        }
        let Some(runtime) = self.workers.actions.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "remediation",
                self.workers.ai_error("remediation"),
            ));
        };
        if runtime.discard(proposal_id.to_string()) {
            self.queue.push(AppEvent::Action(ActionEvent::Discarded {
                proposal_id: proposal_id.to_string(),
            }));
            DispatchOutcome::accepted()
        } else {
            DispatchOutcome::Rejected(RejectReason::Invalid {
                detail: "an approved action preview cannot be dismissed".to_string(),
            })
        }
    }

    pub(super) fn cancel_action(&mut self, run_id: &str) -> DispatchOutcome {
        let Some(runtime) = self.workers.actions.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "remediation",
                self.workers.ai_error("remediation"),
            ));
        };
        match runtime.cancel(run_id) {
            Ok(summary) => {
                self.apply_run_summary(summary);
                DispatchOutcome::accepted()
            }
            Err(error) => DispatchOutcome::Rejected(RejectReason::Invalid { detail: error }),
        }
    }

    fn apply_run_summary(&mut self, summary: wfdiag_native_remediation::runtime::ActionRunSummary) {
        let terminal = summary.status.terminal();
        if terminal {
            self.snapshot
                .actions
                .history
                .retain(|run| run.run_id != summary.run_id);
            self.snapshot.actions.history.insert(0, summary.clone());
            self.snapshot.actions.history.truncate(50);
            if self
                .snapshot
                .actions
                .active_run
                .as_ref()
                .is_some_and(|run| run.run_id == summary.run_id)
            {
                self.snapshot.actions.active_run = None;
            }
            self.queue.push(AppEvent::Action(ActionEvent::Summary {
                summary: Box::new(summary),
            }));
        } else {
            self.snapshot.actions.active_run = Some(summary.clone());
            self.queue.push(AppEvent::Action(ActionEvent::Run {
                summary: Box::new(summary),
            }));
        }
    }

    // ---- model catalog ----------------------------------------------------

    pub(super) fn refresh_model_catalog(
        &mut self,
        provider: &str,
        draft: CatalogDraft,
        forced: bool,
    ) -> DispatchOutcome {
        let provider = match parse_provider(provider) {
            Ok(provider) => provider,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        if !forced && !auto_discovery_allowed(provider) {
            return DispatchOutcome::Ignored {
                detail: "this provider is only discovered on an explicit refresh",
            };
        }
        match self.catalog_throttle.decide(Instant::now(), forced) {
            RefreshDecision::Refresh => {}
            RefreshDecision::Throttled => {
                let last = self
                    .snapshot
                    .provider_setup
                    .catalogs
                    .get(&provider.to_string())
                    .and_then(|state| state.catalog.clone());
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::ModelCatalog(
                        ModelCatalogEvent::Throttled {
                            provider: provider.to_string(),
                            last: last.map(Box::new),
                        },
                    )));
                return DispatchOutcome::Ignored {
                    detail: "a catalog refresh already ran inside the debounce window",
                };
            }
            RefreshDecision::CancelAndRetry => {
                self.catalog_throttle.request_retry();
                self.catalog_retry = Some((provider.to_string(), draft));
                if let Some((request, _)) = self.catalog_pending.as_ref()
                    && let Some(runtime) = self.workers.model_catalog.as_ref()
                {
                    let _ = runtime.cancel(request.get());
                }
                return DispatchOutcome::accepted();
            }
        }
        self.start_catalog_refresh(provider, draft)
    }

    fn start_catalog_refresh(
        &mut self,
        provider: AIProvider,
        draft: CatalogDraft,
    ) -> DispatchOutcome {
        let Some(runtime) = self.workers.model_catalog.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "model discovery",
                self.workers.ai_error("model discovery"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let queued = runtime.list_models(
            request.get(),
            ModelCatalogRequest {
                provider,
                draft_api_key: draft.api_key,
                draft_endpoint: draft.endpoint,
                draft_cli_path: draft.cli_path,
            },
        );
        if !queued {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the native model discovery queue is busy".to_string(),
            });
        }
        self.catalog_throttle.started(Instant::now());
        self.catalog_pending = Some((request, provider.to_string()));
        let state = self
            .snapshot
            .provider_setup
            .catalogs
            .entry(provider.to_string())
            .or_default();
        state.loading = true;
        state.error = None;
        state.blocked = None;
        self.queue
            .push(AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Started {
                    provider: provider.to_string(),
                },
            )));
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn cancel_model_catalog(&mut self) -> DispatchOutcome {
        self.catalog_throttle.clear_retry();
        self.catalog_retry = None;
        let Some((request, _)) = self.catalog_pending.as_ref() else {
            return DispatchOutcome::Ignored {
                detail: "no model catalog refresh is running",
            };
        };
        let request = *request;
        let cancelled = self
            .workers
            .model_catalog
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(request.get()));
        if cancelled {
            DispatchOutcome::accepted()
        } else {
            self.catalog_pending = None;
            let _ = self.catalog_throttle.finished();
            DispatchOutcome::accepted()
        }
    }

    // ---- subscription CLIs ------------------------------------------------

    pub(super) fn subscription_auth(
        &mut self,
        provider: &str,
        operation: SubscriptionOperation,
    ) -> DispatchOutcome {
        let provider = match parse_subscription_provider(provider) {
            Ok(provider) => provider,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        match admit_auth(
            self.subscription_auth_pending.is_some(),
            self.subscription_install_pending.is_some(),
            self.workers.ai_error("subscription CLI"),
        ) {
            AuthAdmission::Start => {}
            AuthAdmission::Refuse { reason } => {
                return DispatchOutcome::Rejected(RejectReason::Busy { detail: reason });
            }
        }
        let operation = match operation {
            SubscriptionOperation::Status => SubscriptionAuthOperation::Status,
            SubscriptionOperation::SignIn => SubscriptionAuthOperation::SignIn,
            SubscriptionOperation::SignOut => SubscriptionAuthOperation::SignOut,
        };
        let cli_path = match provider {
            SubscriptionAuthProvider::Codex => self.snapshot.settings.codex_cli_path.clone(),
            SubscriptionAuthProvider::ClaudeCode => self.snapshot.settings.claude_cli_path.clone(),
        };
        let Some(runtime) = self.workers.subscription_auth.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "subscription CLI",
                self.workers.ai_error("subscription CLI"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        let queued = match operation {
            SubscriptionAuthOperation::Status => {
                runtime.request_status(request.get(), provider, cli_path)
            }
            SubscriptionAuthOperation::SignIn => runtime.sign_in(request.get(), provider, cli_path),
            SubscriptionAuthOperation::SignOut => {
                runtime.sign_out(request.get(), provider, cli_path)
            }
        };
        if !queued {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the subscription account queue is busy".to_string(),
            });
        }
        self.subscription_auth_pending = Some(PendingSubscriptionAuth { request, provider });
        let account = self
            .snapshot
            .provider_setup
            .accounts
            .entry(subscription_key(provider))
            .or_default();
        account.operation = Some(operation);
        account.error = None;
        self.queue
            .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                SubscriptionEvent::Started {
                    provider,
                    operation,
                },
            ))));
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn cancel_subscription_auth(&mut self) -> DispatchOutcome {
        let Some(pending) = self.subscription_auth_pending else {
            return DispatchOutcome::Ignored {
                detail: "no subscription account action is running",
            };
        };
        let cancelled = self
            .workers
            .subscription_auth
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(pending.request.get()));
        if !cancelled {
            self.subscription_auth_pending = None;
        }
        DispatchOutcome::accepted()
    }

    pub(super) fn install_subscription_cli(&mut self, provider: &str) -> DispatchOutcome {
        let provider = match parse_subscription_provider(provider) {
            Ok(provider) => provider,
            Err(reason) => return DispatchOutcome::Rejected(reason),
        };
        match admit_install(
            provider,
            self.subscription_install_pending.is_some(),
            self.subscription_auth_pending.is_some(),
            self.snapshot.provider_setup.install_prompt.is_some(),
            self.workers.ai_error("subscription CLI"),
        ) {
            InstallAdmission::Confirm { prompt } => {
                // Nothing is installed until the confirmation is accepted.
                self.snapshot.provider_setup.install_prompt = Some(prompt);
                self.snapshot.provider_setup.install_error = None;
                DispatchOutcome::accepted()
            }
            InstallAdmission::Refuse { reason } => {
                DispatchOutcome::Rejected(RejectReason::Busy { detail: reason })
            }
        }
    }

    pub(super) fn confirm_subscription_install(&mut self, accepted: bool) -> DispatchOutcome {
        let Some(prompt) = self.snapshot.provider_setup.install_prompt.take() else {
            return DispatchOutcome::Ignored {
                detail: "no installation confirmation is open",
            };
        };
        if !accepted {
            return DispatchOutcome::accepted();
        }
        let provider = prompt.provider();
        let method = prompt.method();
        let Some(runtime) = self.workers.subscription_install.as_ref() else {
            return DispatchOutcome::Rejected(unavailable(
                "subscription CLI",
                self.workers.ai_error("subscription CLI"),
            ));
        };
        let Some(request) = self.requests.issue() else {
            return DispatchOutcome::Rejected(RejectReason::IdentityExhausted);
        };
        // The confirmation booleans are the user's answers, carried verbatim:
        // the vendor bootstrap runs only when both were given.
        let queued = match prompt {
            InstallPrompt::Winget { .. } => {
                runtime.install_with_winget(request.get(), provider, true)
            }
            InstallPrompt::VendorFallback { .. } => {
                runtime.install_with_vendor_fallback(request.get(), provider, true, true)
            }
        };
        if !queued {
            return DispatchOutcome::Rejected(RejectReason::Busy {
                detail: "the subscription installer is already busy".to_string(),
            });
        }
        self.subscription_install_pending = Some(PendingSubscriptionInstall { request, method });
        self.snapshot.provider_setup.install_progress = None;
        self.snapshot.provider_setup.install_error = None;
        self.queue
            .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                SubscriptionEvent::InstallStarted { provider, method },
            ))));
        DispatchOutcome::accepted_request(request)
    }

    pub(super) fn cancel_subscription_install(&mut self) -> DispatchOutcome {
        self.snapshot.provider_setup.install_prompt = None;
        let Some(pending) = self.subscription_install_pending else {
            return DispatchOutcome::Ignored {
                detail: "no subscription installation is running",
            };
        };
        let cancelled = self
            .workers
            .subscription_install
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(pending.request.get()));
        if !cancelled {
            self.subscription_install_pending = None;
        }
        DispatchOutcome::accepted()
    }

    // ---- draining ---------------------------------------------------------

    /// Read every AI worker, guard every reply, and publish one coalesced
    /// delta per streaming domain.
    pub(super) fn drain_ai_events(&mut self) {
        let mut chat_delta = String::new();
        let mut report_delta = String::new();
        for event in take_events(&mut self.workers.chat_events) {
            self.apply_chat_event(event, &mut chat_delta);
        }
        if !chat_delta.is_empty() {
            self.snapshot.ai.chat.text.push_str(&chat_delta);
            self.queue
                .push(AppEvent::Chat(ChatEvent::Delta { text: chat_delta }));
        }
        for event in take_events(&mut self.workers.report_events) {
            self.apply_report_event(event, &mut report_delta);
        }
        if !report_delta.is_empty() {
            self.snapshot
                .ai
                .report
                .text
                .get_or_insert_with(String::new)
                .push_str(&report_delta);
            self.queue
                .push(AppEvent::Report(ReportEvent::Delta { text: report_delta }));
        }
        for event in take_events(&mut self.workers.analysis_events) {
            self.apply_analysis_event(event);
        }
        for event in take_events(&mut self.workers.fix_plan_events) {
            self.apply_fix_plan_event(event);
        }
        // Run transitions before command replies: a run's terminal summary
        // arrives on both streams, and applying the reply first would let the
        // earlier `Running` transition resurrect a finished run.
        for event in take_events(&mut self.workers.action_runs) {
            self.apply_action_run(event);
        }
        for event in take_events(&mut self.workers.action_events) {
            self.apply_action_event(event);
        }
        for event in take_events(&mut self.workers.model_catalog_events) {
            self.apply_catalog_event(event);
        }
        for event in take_events(&mut self.workers.subscription_auth_events) {
            self.apply_subscription_auth_event(event);
        }
        for event in take_events(&mut self.workers.subscription_install_events) {
            self.apply_subscription_install_event(event);
        }
    }

    fn apply_chat_event(&mut self, event: ChatWorkerEvent, delta: &mut String) {
        let Some(pending) = self.chat_pending else {
            return;
        };
        if pending.get() != event.request_id() {
            return;
        }
        match event {
            ChatWorkerEvent::Delta { text, .. } => delta.push_str(&text),
            ChatWorkerEvent::ToolActivity {
                activity, history, ..
            } => {
                self.snapshot.ai.chat.tools = history.clone();
                self.queue.push(AppEvent::Chat(ChatEvent::ToolActivity {
                    activity: Box::new(activity),
                    history: Box::new(history),
                }));
            }
            ChatWorkerEvent::Proposal {
                remediation_id,
                issue_id,
                ..
            } => {
                // The model may name a catalog id; nothing is staged with the
                // broker and nothing runs until the user prepares it.
                self.snapshot.ai.chat.proposals.push(StagedProposalRequest {
                    remediation_id: remediation_id.clone(),
                    issue_id: issue_id.clone(),
                });
                self.queue.push(AppEvent::Chat(ChatEvent::ProposalStaged {
                    remediation_id,
                    issue_id,
                }));
            }
            ChatWorkerEvent::FullScanRequested {
                source_scan_id,
                reason,
                ..
            } => {
                // A request, never a start: the user confirms a Full Scan.
                self.snapshot.ai.chat.full_scan_request = Some(FullScanRequest {
                    source_scan_id: source_scan_id.clone(),
                    reason: reason.clone(),
                });
                self.queue
                    .push(AppEvent::Chat(ChatEvent::FullScanRequested {
                        source_scan_id,
                        reason,
                    }));
            }
            ChatWorkerEvent::Done {
                provider,
                provider_use,
                finish_reason,
                tool_history,
                ..
            } => {
                self.chat_pending = None;
                self.chat_attempt = None;
                self.snapshot.ai.chat.streaming = false;
                self.snapshot.ai.chat.provider = Some(provider.clone());
                self.snapshot.ai.chat.provider_use = Some(provider_use.clone());
                self.snapshot.ai.chat.finish_reason = Some(finish_reason.clone());
                self.snapshot.ai.chat.tools = tool_history.clone();
                if !delta.is_empty() {
                    self.snapshot.ai.chat.text.push_str(delta);
                    self.queue.push(AppEvent::Chat(ChatEvent::Delta {
                        text: std::mem::take(delta),
                    }));
                }
                self.queue.push(AppEvent::Chat(ChatEvent::Done {
                    provider,
                    provider_use: Box::new(provider_use),
                    finish_reason,
                    tool_history: Box::new(tool_history),
                }));
            }
            ChatWorkerEvent::RetryableFailure { message, .. } => {
                self.apply_chat_retryable_failure(message, delta);
            }
            ChatWorkerEvent::Failed { message, .. } => {
                if !delta.is_empty() {
                    self.snapshot.ai.chat.text.push_str(delta);
                    self.queue.push(AppEvent::Chat(ChatEvent::Delta {
                        text: std::mem::take(delta),
                    }));
                }
                self.finish_chat_failure(message);
            }
            ChatWorkerEvent::Cancelled { .. } => {
                if !delta.is_empty() {
                    self.snapshot.ai.chat.text.push_str(delta);
                    self.queue.push(AppEvent::Chat(ChatEvent::Delta {
                        text: std::mem::take(delta),
                    }));
                }
                self.finish_chat_cancelled();
            }
        }
    }

    fn apply_chat_retryable_failure(&mut self, message: String, delta: &mut String) {
        let Some(mut attempt) = self.chat_attempt.take() else {
            self.finish_chat_failure(message);
            return;
        };
        attempt.record_failure(message);
        match decide_fallback(&attempt, self.snapshot.settings.cloud_fallback_policy) {
            FallbackDecision::Continue { provider } => {
                let previous = self.chat_pending;
                attempt.advance_to(provider);
                // The retry is invisible: no terminal event was emitted, the
                // conversation is untouched, and this is the same logical turn.
                if !self.dispatch_chat_attempt(previous, attempt) {
                    self.finish_chat_failure("The native AI chat queue is unavailable".to_string());
                }
            }
            FallbackDecision::Prompt { provider, reason } => {
                self.chat_pending = None;
                self.snapshot.ai.chat.streaming = false;
                self.snapshot.ai.chat.cloud_fallback = Some(CloudFallbackPrompt {
                    candidate: provider.to_string(),
                    reason: reason.clone(),
                    saving: false,
                });
                self.chat_consent = Some(PendingConsent {
                    attempt,
                    candidate: provider,
                    reason: reason.clone(),
                });
                if !delta.is_empty() {
                    self.snapshot.ai.chat.text.push_str(delta);
                    self.queue.push(AppEvent::Chat(ChatEvent::Delta {
                        text: std::mem::take(delta),
                    }));
                }
                self.queue
                    .push(AppEvent::Chat(ChatEvent::CloudFallbackRequired {
                        candidate: provider.to_string(),
                        reason,
                    }));
            }
            FallbackDecision::Refuse { message } | FallbackDecision::Exhausted { message } => {
                self.finish_chat_failure(message);
            }
        }
    }

    fn apply_report_event(&mut self, event: ReportWorkerEvent, delta: &mut String) {
        let Some(pending) = self.report_pending else {
            return;
        };
        if pending.get() != event.request_id() {
            return;
        }
        match event {
            ReportWorkerEvent::Ack {
                provider,
                provider_use,
                ..
            } => {
                self.snapshot.ai.report.provider = Some(provider.clone());
                self.snapshot.ai.report.provider_use = Some(provider_use);
                self.queue
                    .push(AppEvent::Report(ReportEvent::Started { provider }));
            }
            ReportWorkerEvent::Delta { text, .. } => delta.push_str(&text),
            ReportWorkerEvent::Done {
                provider,
                cached,
                finish_reason,
                provider_use,
                report,
                ..
            } => {
                self.report_pending = None;
                self.snapshot.ai.report.generating = false;
                self.snapshot.ai.report.provider = Some(provider.clone());
                self.snapshot.ai.report.provider_use = Some(provider_use.clone());
                self.snapshot.ai.report.cached = cached;
                if let Some(report) = report {
                    // A cached report is returned inline: the core streams no
                    // deltas for a cache hit.
                    delta.clear();
                    self.snapshot.ai.report.text = Some(report.clone());
                    self.queue.push(AppEvent::Report(ReportEvent::Cached {
                        provider: provider.clone(),
                        report,
                    }));
                } else if !delta.is_empty() {
                    self.snapshot
                        .ai
                        .report
                        .text
                        .get_or_insert_with(String::new)
                        .push_str(delta);
                    self.queue.push(AppEvent::Report(ReportEvent::Delta {
                        text: std::mem::take(delta),
                    }));
                }
                self.queue.push(AppEvent::Report(ReportEvent::Done {
                    provider,
                    finish_reason,
                    provider_use: Box::new(provider_use),
                }));
            }
            ReportWorkerEvent::Failed { message, .. } => {
                self.report_pending = None;
                self.snapshot.ai.report.generating = false;
                self.snapshot.ai.report.error = Some(message.clone());
                self.queue
                    .push(AppEvent::Report(ReportEvent::Failed { message }));
            }
            ReportWorkerEvent::Cancelled { .. } => {
                self.report_pending = None;
                self.snapshot.ai.report.generating = false;
                self.queue.push(AppEvent::Report(ReportEvent::Cancelled));
            }
        }
    }

    #[allow(clippy::too_many_lines)] // One event surface, two guarded domains.
    fn apply_analysis_event(&mut self, event: AnalysisWorkerEvent) {
        let request = event.request_id();
        if let Some((pending, generation)) = self.prioritization_pending
            && pending.get() == request
        {
            self.apply_prioritization_event(event, generation);
            return;
        }
        let Some(pending) = self.analysis_pending.clone() else {
            return;
        };
        if pending.request.get() != request {
            return;
        }
        let task_id = pending.task_id;
        match event {
            AnalysisWorkerEvent::Ack {
                provider_use,
                cached,
                ..
            } => {
                let entry = self
                    .snapshot
                    .ai
                    .analyses
                    .entry(task_id.clone())
                    .or_default();
                entry.busy = true;
                entry.cached = cached;
                entry.provider_use = Some(provider_use.clone());
                self.queue.push(AppEvent::Analysis(AnalysisEvent::Started {
                    task_id,
                    provider: provider_use.provider_id,
                    cached,
                }));
            }
            AnalysisWorkerEvent::Done {
                interpretation,
                provider_use,
                cached,
                ..
            } => {
                self.analysis_pending = None;
                let entry = self
                    .snapshot
                    .ai
                    .analyses
                    .entry(task_id.clone())
                    .or_default();
                entry.busy = false;
                entry.cached = cached;
                entry.error = None;
                entry.interpretation = Some(interpretation.clone());
                entry.provider_use = Some(provider_use.clone());
                self.queue
                    .push(AppEvent::Analysis(AnalysisEvent::Completed {
                        task_id,
                        interpretation,
                        cached,
                        provider_use: Box::new(provider_use),
                    }));
            }
            AnalysisWorkerEvent::Failed {
                message, retryable, ..
            } => {
                self.analysis_pending = None;
                let entry = self
                    .snapshot
                    .ai
                    .analyses
                    .entry(task_id.clone())
                    .or_default();
                entry.busy = false;
                entry.error = Some(message.clone());
                self.queue.push(AppEvent::Analysis(AnalysisEvent::Failed {
                    task_id,
                    message,
                    retryable,
                }));
            }
            AnalysisWorkerEvent::Cancelled { .. } => {
                self.analysis_pending = None;
                if let Some(entry) = self.snapshot.ai.analyses.get_mut(&task_id) {
                    entry.busy = false;
                }
                self.queue
                    .push(AppEvent::Analysis(AnalysisEvent::Cancelled { task_id }));
            }
        }
    }

    fn apply_prioritization_event(&mut self, event: AnalysisWorkerEvent, generation: Generation) {
        match event {
            AnalysisWorkerEvent::Ack { provider_use, .. } => {
                self.snapshot.ai.prioritization.busy = true;
                self.queue
                    .push(AppEvent::Prioritization(PrioritizationEvent::Started {
                        provider: provider_use.provider_id,
                    }));
            }
            AnalysisWorkerEvent::Done {
                interpretation,
                cached,
                ..
            } => {
                self.prioritization_pending = None;
                if generation != self.evidence_generation {
                    // The issues this ranking describes are gone; showing it
                    // over newer evidence would be a lie.
                    self.snapshot.ai.prioritization =
                        crate::snapshot_ai::PrioritizationSnapshot::default();
                    self.queue
                        .push(AppEvent::Prioritization(PrioritizationEvent::Invalidated));
                    return;
                }
                self.snapshot.ai.prioritization = crate::snapshot_ai::PrioritizationSnapshot {
                    text: Some(interpretation.clone()),
                    cached,
                    busy: false,
                    error: None,
                };
                self.queue
                    .push(AppEvent::Prioritization(PrioritizationEvent::Completed {
                        ranking: interpretation,
                        cached,
                    }));
            }
            AnalysisWorkerEvent::Failed {
                message, retryable, ..
            } => {
                self.prioritization_pending = None;
                self.snapshot.ai.prioritization.busy = false;
                self.snapshot.ai.prioritization.error = Some(message.clone());
                self.queue
                    .push(AppEvent::Prioritization(PrioritizationEvent::Failed {
                        message,
                        retryable,
                    }));
            }
            AnalysisWorkerEvent::Cancelled { .. } => {
                self.prioritization_pending = None;
                self.snapshot.ai.prioritization.busy = false;
                self.queue
                    .push(AppEvent::Prioritization(PrioritizationEvent::Cancelled));
            }
        }
    }

    fn apply_fix_plan_event(&mut self, event: FixPlanWorkerEvent) {
        let Some(pending) = self.fix_plan_pending else {
            return;
        };
        if pending.get() != event.request_id() {
            return;
        }
        match event {
            FixPlanWorkerEvent::Ack { provider_use, .. } => {
                self.queue.push(AppEvent::FixPlan(FixPlanEvent::Started {
                    provider: provider_use.provider_id,
                }));
            }
            FixPlanWorkerEvent::Done { plan, .. } => {
                self.fix_plan_pending = None;
                self.snapshot.ai.fix_plan.busy = false;
                let current = self.action_snapshot();
                if plan.scan_fingerprint != current.scan_fingerprint
                    || plan.catalog_fingerprint != current.catalog_fingerprint
                {
                    self.snapshot.ai.fix_plan.plan = None;
                    let message = "The scan or remediation catalog changed while the plan was being generated. Generate a fresh plan.".to_string();
                    self.snapshot.ai.fix_plan.error = Some(message.clone());
                    self.queue.push(AppEvent::FixPlan(FixPlanEvent::Failed {
                        message,
                        retryable: true,
                    }));
                    return;
                }
                self.snapshot.ai.fix_plan.plan = Some(plan.clone());
                self.snapshot.ai.fix_plan.error = None;
                self.queue.push(AppEvent::FixPlan(FixPlanEvent::Completed {
                    plan: Box::new(plan),
                }));
            }
            FixPlanWorkerEvent::Failed {
                message, retryable, ..
            } => {
                self.fix_plan_pending = None;
                self.snapshot.ai.fix_plan.busy = false;
                self.snapshot.ai.fix_plan.error = Some(message.clone());
                self.queue.push(AppEvent::FixPlan(FixPlanEvent::Failed {
                    message,
                    retryable,
                }));
            }
            FixPlanWorkerEvent::Cancelled { .. } => {
                self.fix_plan_pending = None;
                self.snapshot.ai.fix_plan.busy = false;
                self.queue.push(AppEvent::FixPlan(FixPlanEvent::Cancelled));
            }
        }
    }

    fn apply_action_event(&mut self, event: ActionWorkerEvent) {
        let Some(pending) = self.action_pending else {
            return;
        };
        if pending.get() != event.request_id() {
            return;
        }
        match event {
            ActionWorkerEvent::Prepared { proposal, .. } => {
                self.action_pending = None;
                self.action_pending_review = None;
                if proposal_matches(&proposal, &self.action_snapshot()) {
                    self.snapshot.actions.review = Some(proposal.clone());
                    self.snapshot.actions.error = None;
                    self.queue.push(AppEvent::Action(ActionEvent::Proposal {
                        proposal: Box::new(proposal),
                    }));
                } else {
                    if let Some(runtime) = self.workers.actions.as_ref() {
                        let _ = runtime.discard(proposal.proposal_id);
                    }
                    let message =
                        "Discarded a stale remediation preview; review the current issue again"
                            .to_string();
                    self.snapshot.actions.error = Some(message.clone());
                    self.queue
                        .push(AppEvent::Action(ActionEvent::Rejected { message }));
                }
            }
            ActionWorkerEvent::NeedsRepairConfirmation { proposal, .. } => {
                self.action_pending = None;
                self.action_pending_review = None;
                // The broker did NOT consume the preview: it is still
                // reviewable, and nothing ran.
                let review = StagedReview {
                    proposal: proposal.clone(),
                    surface: ReviewSurface::RepairConfirmation,
                };
                if self.restore_review_if_current(review) {
                    self.queue
                        .push(AppEvent::Action(ActionEvent::RepairConfirmationRequired {
                            proposal: Box::new(proposal),
                        }));
                } else {
                    self.queue.push(AppEvent::Action(ActionEvent::Rejected {
                        message: "The remediation preview changed before repair confirmation"
                            .to_string(),
                    }));
                }
            }
            ActionWorkerEvent::Done { execution, .. } => {
                self.action_pending = None;
                self.action_pending_review = None;
                let succeeded = execution
                    .summary
                    .actions
                    .iter()
                    .any(|item| item.result.as_ref().is_some_and(|result| result.success));
                let already_final = self
                    .snapshot
                    .actions
                    .history
                    .iter()
                    .any(|run| run.run_id == execution.summary.run_id);
                if !already_final {
                    self.apply_run_summary(execution.summary);
                }
                if succeeded {
                    // Returning to the Issues view must never show a known
                    // stale, pre-repair projection.
                    let _ = self.refresh_issues();
                }
            }
            ActionWorkerEvent::Failed { message, .. } => {
                self.action_pending = None;
                let restored = self
                    .action_pending_review
                    .take()
                    .is_some_and(|review| self.restore_review_if_current(review));
                let message = if restored {
                    format!("{message} · the staged action is still available for review")
                } else {
                    message
                };
                self.snapshot.actions.error = Some(message.clone());
                self.queue
                    .push(AppEvent::Action(ActionEvent::Rejected { message }));
            }
        }
    }

    fn apply_action_run(&mut self, event: ActionRunEvent) {
        let finished = self
            .snapshot
            .actions
            .history
            .iter()
            .any(|run| run.run_id == event.summary.run_id);
        if finished && !event.summary.status.terminal() {
            // A late non-terminal transition for a run that already ended.
            return;
        }
        let first = !finished
            && self
                .snapshot
                .actions
                .active_run
                .as_ref()
                .is_none_or(|run| run.run_id != event.summary.run_id);
        if first && !event.summary.status.terminal() {
            self.queue.push(AppEvent::Action(ActionEvent::Approved {
                run_id: event.summary.run_id.clone(),
                summary: Box::new(event.summary.clone()),
            }));
            self.snapshot.actions.active_run = Some(event.summary);
            return;
        }
        self.apply_run_summary(event.summary);
    }

    fn apply_catalog_event(&mut self, event: ProviderSetupWorkerEvent) {
        let Some((pending, provider_id)) = self.catalog_pending.clone() else {
            return;
        };
        if pending.get() != event.request_id() {
            return;
        }
        match event {
            ProviderSetupWorkerEvent::Ack { .. } => return,
            ProviderSetupWorkerEvent::ModelsLoaded { catalog, .. } => {
                self.snapshot
                    .provider_setup
                    .catalogs
                    .entry(provider_id.clone())
                    .or_default()
                    .loaded(catalog.clone());
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::ModelCatalog(
                        ModelCatalogEvent::Loaded {
                            provider: provider_id,
                            catalog: Box::new(catalog),
                        },
                    )));
            }
            ProviderSetupWorkerEvent::Failed { message, .. } => {
                let state = self
                    .snapshot
                    .provider_setup
                    .catalogs
                    .entry(provider_id.clone())
                    .or_default();
                state.failed(message.clone());
                // The last good list stays on screen, flagged stale.
                let last = state.catalog.clone();
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::ModelCatalog(
                        ModelCatalogEvent::Failed {
                            provider: provider_id,
                            error: message,
                            last: last.map(Box::new),
                        },
                    )));
            }
            ProviderSetupWorkerEvent::Cancelled { .. } => {
                if let Some(state) = self.snapshot.provider_setup.catalogs.get_mut(&provider_id) {
                    state.loading = false;
                }
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::ModelCatalog(
                        ModelCatalogEvent::Cancelled {
                            provider: provider_id,
                        },
                    )));
            }
        }
        self.catalog_pending = None;
        let retry = self.catalog_throttle.finished();
        if retry
            && let Some((provider, draft)) = self.catalog_retry.take()
            && let Ok(provider) = parse_provider(&provider)
        {
            let _ = self.start_catalog_refresh(provider, draft);
        }
    }

    fn apply_subscription_auth_event(&mut self, event: SubscriptionAuthWorkerEvent) {
        let Some(pending) = self.subscription_auth_pending else {
            return;
        };
        if pending.request.get() != event.operation_id() {
            return;
        }
        match event {
            SubscriptionAuthWorkerEvent::Ack { .. } => {}
            SubscriptionAuthWorkerEvent::StatusLoaded { status, .. } => {
                self.subscription_auth_pending = None;
                self.record_account(status.provider, Some(status.clone()), None);
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::Status {
                            status: Box::new(status),
                        },
                    ))));
            }
            SubscriptionAuthWorkerEvent::Completed {
                operation, status, ..
            } => {
                self.subscription_auth_pending = None;
                self.record_account(status.provider, Some(status.clone()), None);
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::Completed {
                            operation,
                            status: Box::new(status),
                        },
                    ))));
                let _ = self.request_provider_status();
                if completion_refreshes_models(operation) {
                    let _ = self.refresh_model_catalog(
                        &subscription_key(pending.provider),
                        CatalogDraft::default(),
                        true,
                    );
                }
            }
            SubscriptionAuthWorkerEvent::Failed {
                provider,
                operation,
                message,
                ..
            } => {
                self.subscription_auth_pending = None;
                self.record_account(provider, None, Some(message.clone()));
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::Failed {
                            provider,
                            operation,
                            error: message,
                        },
                    ))));
            }
            SubscriptionAuthWorkerEvent::Cancelled {
                provider,
                operation,
                ..
            } => {
                self.subscription_auth_pending = None;
                if let Some(account) = self
                    .snapshot
                    .provider_setup
                    .accounts
                    .get_mut(&subscription_key(provider))
                {
                    account.operation = None;
                }
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::Cancelled {
                            provider,
                            operation,
                        },
                    ))));
            }
        }
    }

    fn record_account(
        &mut self,
        provider: SubscriptionAuthProvider,
        status: Option<wfdiag_native_ai_chat::SubscriptionAuthStatus>,
        error: Option<String>,
    ) {
        let account = self
            .snapshot
            .provider_setup
            .accounts
            .entry(subscription_key(provider))
            .or_default();
        account.operation = None;
        if let Some(status) = status {
            account.status = Some(status);
            account.error = None;
        }
        if error.is_some() {
            account.error = error;
        }
    }

    fn apply_subscription_install_event(&mut self, event: SubscriptionInstallWorkerEvent) {
        let Some(pending) = self.subscription_install_pending else {
            return;
        };
        if pending.request.get() != event.request_id() {
            return;
        }
        match event {
            SubscriptionInstallWorkerEvent::Ack { .. } => {}
            SubscriptionInstallWorkerEvent::Progress { progress, .. } => {
                self.snapshot.provider_setup.install_progress = Some(progress);
                let _ = progress_label(&progress);
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::InstallProgress {
                            progress: Box::new(progress),
                        },
                    ))));
            }
            SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                provider,
                reason,
                ..
            } => {
                // A separate, second approval: accepting the first never
                // implies running the vendor's script.
                self.subscription_install_pending = None;
                self.snapshot.provider_setup.install_progress = None;
                self.snapshot.provider_setup.install_prompt =
                    Some(InstallPrompt::VendorFallback { provider, reason });
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::InstallFallbackRequired { provider, reason },
                    ))));
            }
            SubscriptionInstallWorkerEvent::Installed { status, .. } => {
                self.subscription_install_pending = None;
                self.snapshot.provider_setup.install_progress = None;
                if verified_install_path(&status.path) {
                    self.record_account(status.provider, Some(status.auth_status()), None);
                    self.snapshot.provider_setup.install_error = None;
                    self.queue
                        .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                            SubscriptionEvent::Installed {
                                status: Box::new(status),
                            },
                        ))));
                    let _ = self.request_provider_status();
                } else {
                    let error =
                        "The installer did not return a verified absolute CLI path".to_string();
                    self.snapshot.provider_setup.install_error = Some(error.clone());
                    self.queue
                        .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                            SubscriptionEvent::InstallFailed {
                                provider: status.provider,
                                method: pending.method,
                                error,
                            },
                        ))));
                }
            }
            SubscriptionInstallWorkerEvent::Failed {
                provider,
                method,
                message,
                ..
            } => {
                self.subscription_install_pending = None;
                self.snapshot.provider_setup.install_progress = None;
                self.snapshot.provider_setup.install_error = Some(message.clone());
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::InstallFailed {
                            provider,
                            method,
                            error: message,
                        },
                    ))));
            }
            SubscriptionInstallWorkerEvent::Cancelled {
                provider, method, ..
            } => {
                self.subscription_install_pending = None;
                self.snapshot.provider_setup.install_progress = None;
                self.queue
                    .push(AppEvent::Provider(ProviderEvent::Subscription(Box::new(
                        SubscriptionEvent::InstallCancelled { provider, method },
                    ))));
            }
        }
    }
}

fn subscription_key(provider: SubscriptionAuthProvider) -> String {
    match provider {
        SubscriptionAuthProvider::Codex => AIProvider::CodexCli.to_string(),
        SubscriptionAuthProvider::ClaudeCode => AIProvider::ClaudeCode.to_string(),
    }
}

/// Take up to [`AI_DRAIN_LIMIT`] events, dropping the receiver when the worker
/// has stopped so a dead channel is not polled forever.
fn take_events<T>(receiver: &mut Option<std::sync::mpsc::Receiver<T>>) -> Vec<T> {
    let mut events = Vec::new();
    let mut disconnected = false;
    if let Some(channel) = receiver.as_ref() {
        for _ in 0..AI_DRAIN_LIMIT {
            match channel.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    }
    if disconnected {
        *receiver = None;
    }
    events
}
