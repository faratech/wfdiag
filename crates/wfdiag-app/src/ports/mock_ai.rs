//! Scripted doubles for every AI, remediation, and provider-setup seam.
//!
//! Chat and reports run the **real** engine runtimes — real worker threads,
//! real tool loop, real streaming, real cancellation — with only the provider
//! transport scripted. The four seams whose runtimes cannot take an injected
//! backend are replaced wholesale, but the remediation double still drives the
//! genuine [`ActionBroker`], so the Repair gate under test is the shipping one.

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_analysis::{
    AnalysisCacheIdentity, AnalysisWorkerEvent, DiagnosticAnalysisGeneration, FixPlanGeneration,
    FixPlanWorkerEvent, IssuePrioritizationGeneration, ValidatedFixPlan,
};
use wfdiag_native_ai_chat::workers::provider_setup::ProviderSetupWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallWorkerEvent;
use wfdiag_native_ai_chat::{
    ChatProvider, ChatRequest, ChatResolveFuture, ChatTurn, FinishReason, ProviderUse,
    ResolvedChatProvider, SubscriptionAuthOperation, SubscriptionAuthProvider,
    SubscriptionAuthState, SubscriptionAuthStatus, SubscriptionInstallFallbackReason,
    SubscriptionInstallMethod, SubscriptionInstallProgress, SubscriptionInstallStage,
    SubscriptionInstallStatus, ToolCall,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, ModelCatalog, ModelCatalogRequest, ProviderAvailability,
    SharedAiCache,
};
use wfdiag_native_ai_report::{ReportFuture, ReportProviderResolver, ResolvedReportProvider};
use wfdiag_native_remediation::broker::{
    ActionApproval, ActionBroker, ActionGrant, ActionPrepareInput, ActionProposal, ActionSnapshot,
    AuthorizationError, AuthorizedAction, AuthorizedActionExecutor, ExecutionFuture, opaque_id,
};
use wfdiag_native_remediation::remediation::{FixCompletionStatus, FixResult};
use wfdiag_native_remediation::runtime::{
    ActionExecution, ActionExecutionItem, ActionItemStatus, ActionRunEvent, ActionRunSummary,
    ActionWorkerEvent, completed_run_status, initial_run_summary, item_status_for,
};
use wfdiag_native_settings::SettingsService;

use super::ai::{
    ActionHandle, ActionPort, ActionSession, AiPorts, AiWake, AnalysisHandle, AnalysisPort,
    AnalysisSession, ChatResolverPort, FixPlanHandle, FixPlanPort, FixPlanSession,
    ModelCatalogHandle, ModelCatalogPort, ModelCatalogSession, ReportResolverPort,
    SubscriptionAuthHandle, SubscriptionInstallHandle, SubscriptionPort, SubscriptionSession,
};

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------- chat ----

/// One scripted provider response.
#[derive(Clone, Debug, Default)]
pub struct ScriptedTurn {
    /// Text fragments streamed before the turn completes.
    pub deltas: Vec<String>,
    /// Tool calls the model asks for.
    pub tool_calls: Vec<ToolCall>,
    /// How the turn ended.
    pub finished: Option<FinishReason>,
    /// A transport failure instead of a completion.
    pub error: Option<String>,
}

impl ScriptedTurn {
    /// A turn that streams `text` and stops.
    #[must_use]
    pub fn text(text: &str) -> Self {
        Self {
            deltas: text
                .split_inclusive(' ')
                .map(std::string::ToString::to_string)
                .collect(),
            finished: Some(FinishReason::Stop),
            ..Self::default()
        }
    }

    /// A turn that requests one tool call and nothing else.
    #[must_use]
    pub fn tool(call_id: &str, name: &str, arguments: serde_json::Value) -> Self {
        Self {
            tool_calls: vec![ToolCall {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments,
            }],
            finished: Some(FinishReason::ToolUse),
            ..Self::default()
        }
    }

    /// A clean transport failure: nothing streamed, nothing emitted.
    #[must_use]
    pub fn failure(message: &str) -> Self {
        Self {
            error: Some(message.to_string()),
            ..Self::default()
        }
    }

    /// A provider refusal, which is a completion and never a fallback.
    #[must_use]
    pub fn refusal(text: &str) -> Self {
        Self {
            deltas: vec![text.to_string()],
            finished: Some(FinishReason::Refusal),
            ..Self::default()
        }
    }
}

#[derive(Debug, Default)]
struct ChatScriptState {
    turns: HashMap<String, Vec<ScriptedTurn>>,
    resolve_errors: HashMap<String, String>,
    resolved: Vec<AIProvider>,
    prompts: Vec<String>,
    hold: Option<CancellationToken>,
}

/// A scripted chat transport, keyed by provider.
///
/// The same object is both the [`ChatResolverPort`] and, once resolved, the
/// [`ChatProvider`], so a test scripts one value and inspects it afterwards.
#[derive(Clone, Debug, Default)]
pub struct ScriptedChat {
    state: Arc<Mutex<ChatScriptState>>,
}

impl ScriptedChat {
    /// A transport with no script; every provider fails as unconfigured.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the turns `provider` will answer with, in order.
    pub fn script(&self, provider: AIProvider, turns: Vec<ScriptedTurn>) {
        lock(&self.state).turns.insert(provider.to_string(), turns);
    }

    /// Make resolution itself fail for `provider`.
    pub fn fail_resolution(&self, provider: AIProvider, message: &str) {
        lock(&self.state)
            .resolve_errors
            .insert(provider.to_string(), message.to_string());
    }

    /// Block every streamed turn until the returned token is cancelled, so a
    /// test can observe a mid-stream cancel deterministically.
    #[must_use]
    pub fn hold(&self) -> CancellationToken {
        let token = CancellationToken::new();
        lock(&self.state).hold = Some(token.clone());
        token
    }

    /// Every provider a turn resolved, in order.
    #[must_use]
    pub fn resolved(&self) -> Vec<AIProvider> {
        lock(&self.state).resolved.clone()
    }

    /// Every user prompt the transport saw.
    #[must_use]
    pub fn prompts(&self) -> Vec<String> {
        lock(&self.state).prompts.clone()
    }

    fn next_turn(&self, provider: AIProvider) -> ScriptedTurn {
        let mut state = lock(&self.state);
        let queue = state.turns.entry(provider.to_string()).or_default();
        if queue.is_empty() {
            ScriptedTurn::failure(&format!("{provider} has no scripted response"))
        } else {
            queue.remove(0)
        }
    }
}

struct ScriptedChatProvider {
    script: ScriptedChat,
    provider: AIProvider,
}

impl ChatProvider for ScriptedChatProvider {
    fn stream<'a>(
        &'a self,
        request: &'a ChatRequest,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ChatTurn, String>> + Send + 'a>>
    {
        Box::pin(async move {
            if let Some(last) = request.messages.last() {
                lock(&self.script.state).prompts.push(last.content.clone());
            }
            let hold = lock(&self.script.state).hold.clone();
            let turn = self.script.next_turn(self.provider);
            if let Some(error) = turn.error {
                return Err(error);
            }
            let mut text = String::new();
            for delta in turn.deltas {
                text.push_str(&delta);
                if tx.send(delta).await.is_err() {
                    break;
                }
            }
            // Deltas stream first, so a held turn is genuinely mid-stream when
            // the test cancels it.
            if let Some(hold) = hold {
                hold.cancelled().await;
            }
            Ok(ChatTurn {
                text,
                tool_calls: turn.tool_calls,
                finished: turn.finished.unwrap_or(FinishReason::Stop),
                actual_models: vec![format!("{}-scripted", self.provider)],
                provider_replay: None,
            })
        })
    }
}

impl ChatResolverPort for ScriptedChat {
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err("AI request cancelled".to_string());
            }
            lock(&self.state).resolved.push(provider);
            if let Some(error) = lock(&self.state).resolve_errors.get(&provider.to_string()) {
                return Err(error.clone());
            }
            Ok(ResolvedChatProvider {
                chat: Arc::new(ScriptedChatProvider {
                    script: self.clone(),
                    provider,
                }),
                config_fingerprint: format!("provider={provider};scripted"),
                requested_model: Some(format!("{provider}-scripted")),
            })
        })
    }
}

// -------------------------------------------------------------- report ----

/// A scripted report transport. It reuses [`ScriptedChat`] so one test can
/// script both surfaces the same way.
#[derive(Clone, Debug, Default)]
pub struct ScriptedReport {
    /// The transport the resolver hands to the report core.
    pub chat: ScriptedChat,
}

struct ScriptedReportResolver {
    chat: ScriptedChat,
    provider: AIProvider,
}

impl ReportProviderResolver for ScriptedReportResolver {
    fn preference(&self) -> AIProviderPreference {
        AIProviderPreference::Auto
    }

    fn determine_active(&self, _preference: AIProviderPreference) -> ReportFuture<'_, AIProvider> {
        Box::pin(async move { self.provider })
    }

    fn next_auto_local(
        &self,
        _preference: AIProviderPreference,
        _tried: &[AIProvider],
    ) -> ReportFuture<'_, Option<AIProvider>> {
        Box::pin(async move { None })
    }

    fn resolve(
        &self,
        provider: AIProvider,
    ) -> ReportFuture<'_, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            ChatResolverPort::resolve(&self.chat, provider, CancellationToken::new()).await
        })
    }
}

impl ReportResolverPort for ScriptedReport {
    fn resolver(
        &self,
        provider: AIProvider,
        _availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver> {
        Arc::new(ScriptedReportResolver {
            chat: self.chat.clone(),
            provider,
        })
    }
}

// ------------------------------------------------------------ analysis ----

/// What a scripted one-shot analysis answers with.
#[derive(Clone, Debug)]
pub enum ScriptedAnalysisOutcome {
    /// An interpretation.
    Completed {
        /// The text.
        text: String,
        /// Whether it came from the cache.
        cached: bool,
    },
    /// A failure.
    Failed {
        /// The diagnostic.
        message: String,
        /// Whether a retry could succeed.
        retryable: bool,
    },
}

impl ScriptedAnalysisOutcome {
    /// A successful, uncached interpretation.
    #[must_use]
    pub fn completed(text: &str) -> Self {
        Self::Completed {
            text: text.to_string(),
            cached: false,
        }
    }
}

#[derive(Debug, Default)]
struct AnalysisScriptState {
    analyses: HashMap<String, ScriptedAnalysisOutcome>,
    prioritization: Option<ScriptedAnalysisOutcome>,
    generated: Vec<String>,
    grounding: Vec<bool>,
    prioritized: usize,
    hold: bool,
    cancelled: Vec<u64>,
}

/// A scripted one-shot analysis and prioritisation runtime.
#[derive(Clone, Debug, Default)]
pub struct ScriptedAnalysis {
    state: Arc<Mutex<AnalysisScriptState>>,
}

impl ScriptedAnalysis {
    /// A runtime with no script; every request fails.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script one task's interpretation.
    pub fn script_analysis(&self, task_id: &str, outcome: ScriptedAnalysisOutcome) {
        lock(&self.state)
            .analyses
            .insert(task_id.to_string(), outcome);
    }

    /// Script the prioritisation answer.
    pub fn script_prioritization(&self, outcome: ScriptedAnalysisOutcome) {
        lock(&self.state).prioritization = Some(outcome);
    }

    /// Withhold every answer until [`Self::release`], so a test can cancel.
    pub fn hold(&self) {
        lock(&self.state).hold = true;
    }

    /// Stop withholding answers.
    pub fn release(&self) {
        lock(&self.state).hold = false;
    }

    /// Every task analysed, in order.
    #[must_use]
    pub fn generated(&self) -> Vec<String> {
        lock(&self.state).generated.clone()
    }

    /// Whether live network grounding was enabled for each request, in order.
    #[must_use]
    pub fn grounding_flags(&self) -> Vec<bool> {
        lock(&self.state).grounding.clone()
    }

    /// How many prioritisations ran.
    #[must_use]
    pub fn prioritized(&self) -> usize {
        lock(&self.state).prioritized
    }
}

fn analysis_identity(key: &str) -> AnalysisCacheIdentity {
    AnalysisCacheIdentity {
        output_hash: format!("hash:{key}"),
        cache_key: format!("cache:{key}"),
        config_fingerprint: "scripted".to_string(),
    }
}

struct ScriptedAnalysisHandle {
    state: Arc<Mutex<AnalysisScriptState>>,
    events: Sender<AnalysisWorkerEvent>,
    wake: AiWake,
}

impl ScriptedAnalysisHandle {
    fn emit(&self, event: AnalysisWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn answer(
        &self,
        request_id: u64,
        key: &str,
        provider_use: ProviderUse,
        outcome: Option<ScriptedAnalysisOutcome>,
        route: wfdiag_native_ai_analysis::AnalysisRoute,
    ) {
        let identity = analysis_identity(key);
        let cached = matches!(
            outcome,
            Some(ScriptedAnalysisOutcome::Completed { cached: true, .. })
        );
        self.emit(AnalysisWorkerEvent::Ack {
            request_id,
            identity: identity.clone(),
            provider_use: provider_use.clone(),
            grounding: None,
            cached,
        });
        if lock(&self.state).hold {
            return;
        }
        match outcome {
            Some(ScriptedAnalysisOutcome::Completed { text, cached }) => {
                self.emit(AnalysisWorkerEvent::Done {
                    request_id,
                    identity,
                    interpretation: text,
                    provider_use,
                    grounding: None,
                    cached,
                });
            }
            Some(ScriptedAnalysisOutcome::Failed { message, retryable }) => {
                self.emit(AnalysisWorkerEvent::Failed {
                    request_id,
                    route,
                    identity: Some(identity),
                    provider_use,
                    grounding: None,
                    message,
                    retryable,
                });
            }
            None => {
                self.emit(AnalysisWorkerEvent::Failed {
                    request_id,
                    route,
                    identity: Some(identity),
                    provider_use,
                    grounding: None,
                    message: "no scripted analysis".to_string(),
                    retryable: false,
                });
            }
        }
    }
}

impl AnalysisHandle for ScriptedAnalysisHandle {
    fn generate(&self, request_id: u64, generation: DiagnosticAnalysisGeneration) -> bool {
        let outcome = {
            let mut state = lock(&self.state);
            state.generated.push(generation.task_id.clone());
            state.grounding.push(generation.network_grounding_enabled);
            state.analyses.get(&generation.task_id).cloned()
        };
        let provider_use =
            ProviderUse::for_provider(generation.route.provider, generation.route.fallback_from);
        self.answer(
            request_id,
            &generation.task_id,
            provider_use,
            outcome,
            generation.route,
        );
        true
    }

    fn prioritize(&self, request_id: u64, generation: IssuePrioritizationGeneration) -> bool {
        let outcome = {
            let mut state = lock(&self.state);
            state.prioritized += 1;
            state.grounding.push(generation.network_grounding_enabled);
            state.prioritization.clone()
        };
        let provider_use =
            ProviderUse::for_provider(generation.route.provider, generation.route.fallback_from);
        self.answer(
            request_id,
            "prioritization",
            provider_use,
            outcome,
            generation.route,
        );
        true
    }

    fn cancel(&self, request_id: u64) -> bool {
        lock(&self.state).cancelled.push(request_id);
        self.emit(AnalysisWorkerEvent::Cancelled {
            request_id,
            identity: None,
            provider_use: ProviderUse::for_provider(AIProvider::None, None),
            grounding: None,
        });
        true
    }
}

impl AnalysisPort for ScriptedAnalysis {
    fn start(
        &self,
        _settings: SettingsService,
        _cache: SharedAiCache,
        wake: AiWake,
    ) -> Result<AnalysisSession, String> {
        let (events, receiver) = channel();
        Ok(AnalysisSession {
            handle: Box::new(ScriptedAnalysisHandle {
                state: Arc::clone(&self.state),
                events,
                wake,
            }),
            events: receiver,
        })
    }
}

// ------------------------------------------------------------ fix plan ----

#[derive(Debug, Default)]
struct FixPlanScriptState {
    plan: Option<ValidatedFixPlan>,
    failure: Option<(String, bool)>,
    requests: usize,
}

/// A scripted fix-plan runtime.
#[derive(Clone, Debug, Default)]
pub struct ScriptedFixPlan {
    state: Arc<Mutex<FixPlanScriptState>>,
}

impl ScriptedFixPlan {
    /// A runtime with no script; every request fails.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the validated plan the worker returns. The fingerprints are
    /// filled in from the request, which is what the real validator does.
    pub fn script(&self, plan: ValidatedFixPlan) {
        lock(&self.state).plan = Some(plan);
    }

    /// Script a failure instead.
    pub fn script_failure(&self, message: &str, retryable: bool) {
        lock(&self.state).failure = Some((message.to_string(), retryable));
    }

    /// How many plans were requested.
    #[must_use]
    pub fn requests(&self) -> usize {
        lock(&self.state).requests
    }
}

struct ScriptedFixPlanHandle {
    state: Arc<Mutex<FixPlanScriptState>>,
    events: Sender<FixPlanWorkerEvent>,
    wake: AiWake,
}

impl ScriptedFixPlanHandle {
    fn emit(&self, event: FixPlanWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

impl FixPlanHandle for ScriptedFixPlanHandle {
    fn generate(&self, request_id: u64, generation: FixPlanGeneration) -> bool {
        let (plan, failure) = {
            let mut state = lock(&self.state);
            state.requests += 1;
            (state.plan.clone(), state.failure.clone())
        };
        let provider_use =
            ProviderUse::for_provider(generation.route.provider, generation.route.fallback_from);
        self.emit(FixPlanWorkerEvent::Ack {
            request_id,
            provider_use: provider_use.clone(),
        });
        if let Some((message, retryable)) = failure {
            self.emit(FixPlanWorkerEvent::Failed {
                request_id,
                route: generation.route,
                provider_use,
                message,
                retryable,
            });
            return true;
        }
        let Some(mut plan) = plan else {
            self.emit(FixPlanWorkerEvent::Failed {
                request_id,
                route: generation.route,
                provider_use,
                message: "no scripted fix plan".to_string(),
                retryable: false,
            });
            return true;
        };
        plan.provider_use = provider_use;
        plan.scan_fingerprint = generation.scan_fingerprint;
        plan.catalog_fingerprint = generation.catalog_fingerprint;
        self.emit(FixPlanWorkerEvent::Done { request_id, plan });
        true
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.emit(FixPlanWorkerEvent::Cancelled {
            request_id,
            provider_use: ProviderUse::for_provider(AIProvider::None, None),
        });
        true
    }
}

impl FixPlanPort for ScriptedFixPlan {
    fn start(&self, _settings: SettingsService, wake: AiWake) -> Result<FixPlanSession, String> {
        let (events, receiver) = channel();
        Ok(FixPlanSession {
            handle: Box::new(ScriptedFixPlanHandle {
                state: Arc::clone(&self.state),
                events,
                wake,
            }),
            events: receiver,
        })
    }
}

// ------------------------------------------------------------- actions ----

/// A [`AuthorizedActionExecutor`] that records catalog ids instead of running
/// anything. It mirrors the remediation crate's own `RecordingRunner`, which is
/// `cfg(test)`-private to that crate.
#[derive(Debug, Default)]
pub struct RecordingExecutor {
    calls: Mutex<Vec<String>>,
    fail: Mutex<Option<String>>,
}

impl RecordingExecutor {
    /// Every remediation id executed, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }

    /// Make one remediation id report failure.
    pub fn fail_on(&self, remediation_id: &str) {
        *lock(&self.fail) = Some(remediation_id.to_string());
    }
}

impl AuthorizedActionExecutor for RecordingExecutor {
    fn execute<'a>(
        &'a self,
        action: AuthorizedAction<'a>,
        cancel: &'a CancellationToken,
    ) -> ExecutionFuture<'a> {
        let id = action.remediation_id().to_string();
        lock(&self.calls).push(id.clone());
        let failed = lock(&self.fail).as_deref() == Some(id.as_str());
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Ok(FixResult {
                    success: false,
                    message: "cancelled".to_string(),
                    actions_taken: Vec::new(),
                    requires_restart: false,
                    completion_status: FixCompletionStatus::Cancelled,
                    steps: Vec::new(),
                });
            }
            Ok(FixResult {
                success: !failed,
                message: if failed {
                    format!("{id} failed")
                } else {
                    format!("{id} completed")
                },
                actions_taken: vec![id.clone()],
                requires_restart: false,
                completion_status: if failed {
                    FixCompletionStatus::Failed
                } else {
                    FixCompletionStatus::Succeeded
                },
                steps: Vec::new(),
            })
        })
    }
}

#[derive(Default)]
struct ActionMockState {
    broker: ActionBroker,
    runs: Vec<ActionRunSummary>,
}

/// A remediation double built on the **genuine** [`ActionBroker`].
///
/// Staging, fingerprint revalidation, the issue reconciliation, and the Repair
/// confirmation gate are the shipping code paths; only catalog execution is
/// replaced by [`RecordingExecutor`].
#[derive(Clone, Debug, Default)]
pub struct MockActions {
    state: Arc<Mutex<ActionMockState>>,
    /// The recorder every approved action runs through.
    pub executor: Arc<RecordingExecutor>,
    clock: Arc<Mutex<u64>>,
}

impl std::fmt::Debug for ActionMockState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionMockState")
            .field("runs", &self.runs.len())
            .finish_non_exhaustive()
    }
}

impl MockActions {
    /// A broker with an empty proposal store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ActionMockState::default())),
            executor: Arc::new(RecordingExecutor::default()),
            clock: Arc::new(Mutex::new(1_000)),
        }
    }

    /// Every remediation id that actually executed.
    #[must_use]
    pub fn executed(&self) -> Vec<String> {
        self.executor.calls()
    }

    fn now(&self) -> u64 {
        *lock(&self.clock)
    }
}

struct MockActionHandle {
    mock: MockActions,
    events: Sender<ActionWorkerEvent>,
    runs: Sender<ActionRunEvent>,
    wake: AiWake,
}

impl MockActionHandle {
    fn emit(&self, event: ActionWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn publish(&self, request_id: u64, summary: &ActionRunSummary) {
        if self
            .runs
            .send(ActionRunEvent {
                request_id,
                summary: summary.clone(),
            })
            .is_ok()
        {
            (self.wake)();
        }
    }

    fn run_grant(&self, request_id: u64, grant: &ActionGrant) {
        let run_id = opaque_id("run");
        let mut summary = initial_run_summary(run_id, grant, self.mock.now());
        self.publish(request_id, &summary);

        let cancel = CancellationToken::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let mut items = Vec::new();
        for (index, action) in grant.actions().enumerate() {
            let remediation = action.preview().remediation.clone();
            let issue_id = action.preview().issue_id.clone();
            let result = runtime.as_ref().map_or_else(
                |_| Err("the scripted executor could not start".to_string()),
                |runtime| runtime.block_on(self.mock.executor.execute(action, &cancel)),
            );
            if let Some(item) = summary.actions.get_mut(index) {
                match &result {
                    Ok(fix) => {
                        item.status = item_status_for(fix.completion_status);
                        item.result = Some(fix.clone());
                    }
                    Err(error) => {
                        item.status = ActionItemStatus::Failed;
                        item.error = Some(error.clone());
                    }
                }
            }
            items.push(ActionExecutionItem {
                remediation,
                issue_id,
                result,
            });
        }
        summary.current_index = None;
        summary.completed_at_ms = Some(self.mock.now());
        summary.status = completed_run_status(&summary.actions, false);
        lock(&self.mock.state).runs.insert(0, summary.clone());
        self.publish(request_id, &summary);
        self.emit(ActionWorkerEvent::Done {
            request_id,
            execution: ActionExecution {
                run_id: summary.run_id.clone(),
                proposal_id: summary.proposal_id.clone(),
                items,
                summary,
            },
        });
    }
}

impl ActionHandle for MockActionHandle {
    fn prepare(
        &self,
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    ) -> bool {
        let now = self.mock.now();
        let prepared = lock(&self.mock.state).broker.prepare(input, &snapshot, now);
        match prepared {
            Ok(proposal) => self.emit(ActionWorkerEvent::Prepared {
                request_id,
                proposal,
            }),
            Err(message) => self.emit(ActionWorkerEvent::Failed {
                request_id,
                message,
            }),
        }
        true
    }

    fn approve(
        &self,
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    ) -> bool {
        let now = self.mock.now();
        let authorized =
            lock(&self.mock.state)
                .broker
                .authorize(&proposal_id, &snapshot, approval, now);
        match authorized {
            Ok(grant) => self.run_grant(request_id, &grant),
            Err(AuthorizationError::RepairConfirmationRequired(proposal)) => {
                self.emit(ActionWorkerEvent::NeedsRepairConfirmation {
                    request_id,
                    proposal,
                });
            }
            Err(AuthorizationError::Rejected(message)) => {
                self.emit(ActionWorkerEvent::Failed {
                    request_id,
                    message,
                });
            }
        }
        true
    }

    fn discard(&self, proposal_id: String) -> bool {
        lock(&self.mock.state).broker.discard(&proposal_id).is_ok()
    }

    fn cancel(&self, run_id: &str) -> Result<ActionRunSummary, String> {
        lock(&self.mock.state)
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .cloned()
            .ok_or_else(|| "The remediation run is no longer active".to_string())
    }

    fn history(&self) -> Vec<ActionRunSummary> {
        lock(&self.mock.state).runs.clone()
    }

    fn pending_proposals(&self) -> Vec<ActionProposal> {
        let now = self.mock.now();
        lock(&self.mock.state).broker.pending(now)
    }
}

impl ActionPort for MockActions {
    fn start(&self, wake: AiWake) -> Result<ActionSession, String> {
        let (events, event_rx) = channel();
        let (runs, run_rx) = channel();
        Ok(ActionSession {
            handle: Box::new(MockActionHandle {
                mock: self.clone(),
                events,
                runs,
                wake,
            }),
            events: event_rx,
            runs: run_rx,
            pending_proposals: Vec::new(),
            history: Vec::new(),
            active_run: None,
        })
    }
}

// ------------------------------------------------------- model catalog ----

#[derive(Debug, Default)]
struct CatalogScriptState {
    catalogs: HashMap<String, Result<ModelCatalog, String>>,
    requests: Vec<AIProvider>,
    hold: bool,
}

/// A scripted model-catalog runtime.
#[derive(Clone, Debug, Default)]
pub struct ScriptedModelCatalog {
    state: Arc<Mutex<CatalogScriptState>>,
}

impl ScriptedModelCatalog {
    /// A runtime with no script; every provider fails.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script one provider's answer.
    pub fn script(&self, provider: AIProvider, answer: Result<ModelCatalog, String>) {
        lock(&self.state)
            .catalogs
            .insert(provider.to_string(), answer);
    }

    /// Withhold every answer so a test can cancel one.
    pub fn hold(&self) {
        lock(&self.state).hold = true;
    }

    /// Every provider a refresh asked about, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<AIProvider> {
        lock(&self.state).requests.clone()
    }
}

struct ScriptedCatalogHandle {
    state: Arc<Mutex<CatalogScriptState>>,
    events: Sender<ProviderSetupWorkerEvent>,
    wake: AiWake,
    active: Mutex<Option<(u64, AIProvider)>>,
}

impl ScriptedCatalogHandle {
    fn emit(&self, event: ProviderSetupWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

impl ModelCatalogHandle for ScriptedCatalogHandle {
    fn list_models(&self, request_id: u64, request: ModelCatalogRequest) -> bool {
        if lock(&self.active).is_some() {
            return false;
        }
        let provider = request.provider;
        let (answer, hold) = {
            let mut state = lock(&self.state);
            state.requests.push(provider);
            (
                state.catalogs.get(&provider.to_string()).cloned(),
                state.hold,
            )
        };
        *lock(&self.active) = Some((request_id, provider));
        self.emit(ProviderSetupWorkerEvent::Ack {
            request_id,
            provider,
        });
        if hold {
            return true;
        }
        *lock(&self.active) = None;
        match answer {
            Some(Ok(catalog)) => self.emit(ProviderSetupWorkerEvent::ModelsLoaded {
                request_id,
                provider,
                catalog,
            }),
            Some(Err(message)) => self.emit(ProviderSetupWorkerEvent::Failed {
                request_id,
                provider,
                message,
            }),
            None => self.emit(ProviderSetupWorkerEvent::Failed {
                request_id,
                provider,
                message: format!("{provider} has no scripted catalog"),
            }),
        }
        true
    }

    fn cancel(&self, request_id: u64) -> bool {
        let active = lock(&self.active).take();
        let Some((active_id, provider)) = active else {
            return false;
        };
        if active_id != request_id {
            *lock(&self.active) = Some((active_id, provider));
            return false;
        }
        self.emit(ProviderSetupWorkerEvent::Cancelled {
            request_id,
            provider,
        });
        true
    }
}

impl ModelCatalogPort for ScriptedModelCatalog {
    fn start(
        &self,
        _settings: SettingsService,
        wake: AiWake,
    ) -> Result<ModelCatalogSession, String> {
        let (events, receiver) = channel();
        Ok(ModelCatalogSession {
            handle: Box::new(ScriptedCatalogHandle {
                state: Arc::clone(&self.state),
                events,
                wake,
                active: Mutex::new(None),
            }),
            events: receiver,
        })
    }
}

// -------------------------------------------------------- subscriptions ---

#[derive(Debug, Default)]
struct SubscriptionScriptState {
    states: HashMap<SubscriptionAuthProvider, SubscriptionAuthState>,
    auth_failure: Option<String>,
    install_path: Option<std::path::PathBuf>,
    install_failure: Option<String>,
    require_vendor_fallback: bool,
    operations: Vec<(SubscriptionAuthProvider, SubscriptionAuthOperation)>,
    installs: Vec<(SubscriptionAuthProvider, SubscriptionInstallMethod)>,
    killed: Vec<u64>,
    hold_auth: bool,
    hold_install: bool,
}

/// A scripted subscription-CLI double.
#[derive(Clone, Debug, Default)]
pub struct ScriptedSubscriptions {
    state: Arc<Mutex<SubscriptionScriptState>>,
}

impl ScriptedSubscriptions {
    /// A double where both CLIs are absent.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the account state a status probe reports.
    pub fn set_state(&self, provider: SubscriptionAuthProvider, state: SubscriptionAuthState) {
        lock(&self.state).states.insert(provider, state);
    }

    /// Make every account operation fail.
    pub fn fail_auth(&self, message: &str) {
        lock(&self.state).auth_failure = Some(message.to_string());
    }

    /// The absolute path a successful installation reports.
    pub fn set_install_path(&self, path: impl Into<std::path::PathBuf>) {
        lock(&self.state).install_path = Some(path.into());
    }

    /// Make winget fail so the vendor fallback is required.
    pub fn require_vendor_fallback(&self) {
        lock(&self.state).require_vendor_fallback = true;
    }

    /// Make every installation fail.
    pub fn fail_install(&self, message: &str) {
        lock(&self.state).install_failure = Some(message.to_string());
    }

    /// Withhold account answers so a test can cancel one.
    pub fn hold_auth(&self) {
        lock(&self.state).hold_auth = true;
    }

    /// Withhold install answers so a test can cancel one.
    pub fn hold_install(&self) {
        lock(&self.state).hold_install = true;
    }

    /// Every account operation, in order.
    #[must_use]
    pub fn operations(&self) -> Vec<(SubscriptionAuthProvider, SubscriptionAuthOperation)> {
        lock(&self.state).operations.clone()
    }

    /// Every installation attempt, in order.
    #[must_use]
    pub fn installs(&self) -> Vec<(SubscriptionAuthProvider, SubscriptionInstallMethod)> {
        lock(&self.state).installs.clone()
    }

    /// The requests whose process tree the double killed.
    #[must_use]
    pub fn killed(&self) -> Vec<u64> {
        lock(&self.state).killed.clone()
    }
}

struct ScriptedAuthHandle {
    state: Arc<Mutex<SubscriptionScriptState>>,
    events: Sender<SubscriptionAuthWorkerEvent>,
    wake: AiWake,
    active: Mutex<Option<(u64, SubscriptionAuthProvider, SubscriptionAuthOperation)>>,
}

impl ScriptedAuthHandle {
    fn emit(&self, event: SubscriptionAuthWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn run(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
    ) -> bool {
        if lock(&self.active).is_some() {
            return false;
        }
        *lock(&self.active) = Some((operation_id, provider, operation));
        let (failure, hold) = {
            let mut state = lock(&self.state);
            state.operations.push((provider, operation));
            if operation == SubscriptionAuthOperation::SignIn {
                state
                    .states
                    .insert(provider, SubscriptionAuthState::SignedIn);
            } else if operation == SubscriptionAuthOperation::SignOut {
                state
                    .states
                    .insert(provider, SubscriptionAuthState::SignedOut);
            }
            (state.auth_failure.clone(), state.hold_auth)
        };
        self.emit(SubscriptionAuthWorkerEvent::Ack {
            operation_id,
            provider,
            operation,
        });
        if hold {
            return true;
        }
        *lock(&self.active) = None;
        if let Some(message) = failure {
            self.emit(SubscriptionAuthWorkerEvent::Failed {
                operation_id,
                provider,
                operation,
                message,
            });
            return true;
        }
        // One guard: two `lock` calls in one expression would deadlock on a
        // non-reentrant mutex, because the first temporary outlives the second.
        let status = {
            let state = lock(&self.state);
            SubscriptionAuthStatus {
                provider,
                state: state
                    .states
                    .get(&provider)
                    .copied()
                    .unwrap_or(SubscriptionAuthState::NotInstalled),
                path: state.install_path.clone(),
            }
        };
        if operation == SubscriptionAuthOperation::Status {
            self.emit(SubscriptionAuthWorkerEvent::StatusLoaded {
                operation_id,
                status,
            });
        } else {
            self.emit(SubscriptionAuthWorkerEvent::Completed {
                operation_id,
                operation,
                status,
            });
        }
        true
    }
}

impl SubscriptionAuthHandle for ScriptedAuthHandle {
    fn request_status(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        _cli_path: Option<String>,
    ) -> bool {
        self.run(operation_id, provider, SubscriptionAuthOperation::Status)
    }

    fn sign_in(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        _cli_path: Option<String>,
    ) -> bool {
        self.run(operation_id, provider, SubscriptionAuthOperation::SignIn)
    }

    fn sign_out(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        _cli_path: Option<String>,
    ) -> bool {
        self.run(operation_id, provider, SubscriptionAuthOperation::SignOut)
    }

    fn cancel(&self, operation_id: u64) -> bool {
        let Some((active_id, provider, operation)) = lock(&self.active).take() else {
            return false;
        };
        if active_id != operation_id {
            *lock(&self.active) = Some((active_id, provider, operation));
            return false;
        }
        self.emit(SubscriptionAuthWorkerEvent::Cancelled {
            operation_id,
            provider,
            operation,
        });
        true
    }
}

struct ScriptedInstallHandle {
    state: Arc<Mutex<SubscriptionScriptState>>,
    events: Sender<SubscriptionInstallWorkerEvent>,
    wake: AiWake,
    active: Mutex<Option<(u64, SubscriptionAuthProvider, SubscriptionInstallMethod)>>,
}

impl ScriptedInstallHandle {
    fn emit(&self, event: SubscriptionInstallWorkerEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn run(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> bool {
        if !confirmed || lock(&self.active).is_some() {
            return false;
        }
        *lock(&self.active) = Some((request_id, provider, method));
        let (require_fallback, failure, path, hold) = {
            let mut state = lock(&self.state);
            state.installs.push((provider, method));
            (
                state.require_vendor_fallback,
                state.install_failure.clone(),
                state.install_path.clone(),
                state.hold_install,
            )
        };
        self.emit(SubscriptionInstallWorkerEvent::Ack {
            request_id,
            provider,
            method,
        });
        self.emit(SubscriptionInstallWorkerEvent::Progress {
            request_id,
            progress: SubscriptionInstallProgress {
                provider,
                method,
                stage: SubscriptionInstallStage::ResolvingInstaller,
            },
        });
        if hold {
            return true;
        }
        *lock(&self.active) = None;
        if method == SubscriptionInstallMethod::Winget && require_fallback {
            self.emit(
                SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                    request_id,
                    provider,
                    reason: SubscriptionInstallFallbackReason::WingetFailed,
                },
            );
            return true;
        }
        if method == SubscriptionInstallMethod::VendorPowerShell && !fallback_confirmed {
            self.emit(
                SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                    request_id,
                    provider,
                    reason: SubscriptionInstallFallbackReason::ExplicitApprovalMissing,
                },
            );
            return true;
        }
        if let Some(message) = failure {
            self.emit(SubscriptionInstallWorkerEvent::Failed {
                request_id,
                provider,
                method,
                message,
            });
            return true;
        }
        let path = path.unwrap_or_else(|| {
            if cfg!(windows) {
                std::path::PathBuf::from(r"C:\scripted\cli.cmd")
            } else {
                std::path::PathBuf::from("/scripted/cli")
            }
        });
        lock(&self.state)
            .states
            .insert(provider, SubscriptionAuthState::SignedOut);
        self.emit(SubscriptionInstallWorkerEvent::Installed {
            request_id,
            status: SubscriptionInstallStatus {
                provider,
                path,
                state: SubscriptionAuthState::SignedOut,
            },
        });
        true
    }
}

impl SubscriptionInstallHandle for ScriptedInstallHandle {
    fn install_with_winget(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
    ) -> bool {
        self.run(
            request_id,
            provider,
            SubscriptionInstallMethod::Winget,
            confirmed,
            false,
        )
    }

    fn install_with_vendor_fallback(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> bool {
        self.run(
            request_id,
            provider,
            SubscriptionInstallMethod::VendorPowerShell,
            confirmed,
            fallback_confirmed,
        )
    }

    fn cancel(&self, request_id: u64) -> bool {
        let Some((active_id, provider, method)) = lock(&self.active).take() else {
            return false;
        };
        if active_id != request_id {
            *lock(&self.active) = Some((active_id, provider, method));
            return false;
        }
        lock(&self.state).killed.push(request_id);
        self.emit(SubscriptionInstallWorkerEvent::Cancelled {
            request_id,
            provider,
            method,
        });
        true
    }
}

impl SubscriptionPort for ScriptedSubscriptions {
    fn start(
        &self,
        _settings: SettingsService,
        wake: AiWake,
    ) -> Result<SubscriptionSession, String> {
        let (auth_events, auth_rx) = channel();
        let (install_events, install_rx) = channel();
        Ok(SubscriptionSession {
            auth: Box::new(ScriptedAuthHandle {
                state: Arc::clone(&self.state),
                events: auth_events,
                wake: Arc::clone(&wake),
                active: Mutex::new(None),
            }),
            auth_events: auth_rx,
            install: Box::new(ScriptedInstallHandle {
                state: Arc::clone(&self.state),
                events: install_events,
                wake,
                active: Mutex::new(None),
            }),
            install_events: install_rx,
        })
    }
}

// ------------------------------------------------------------- bundle -----

/// Every AI double, with the handles a test scripts and inspects.
#[derive(Clone, Debug)]
pub struct MockAiPorts {
    /// The scripted chat transport.
    pub chat: ScriptedChat,
    /// The scripted report transport.
    pub report: ScriptedReport,
    /// The scripted analysis runtime.
    pub analysis: ScriptedAnalysis,
    /// The scripted fix-plan runtime.
    pub fix_plan: ScriptedFixPlan,
    /// The broker-backed remediation double.
    pub actions: MockActions,
    /// The scripted model-catalog runtime.
    pub model_catalog: ScriptedModelCatalog,
    /// The scripted subscription CLIs.
    pub subscriptions: ScriptedSubscriptions,
    /// The response cache chat and reports share.
    pub cache: SharedAiCache,
}

impl Default for MockAiPorts {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAiPorts {
    /// A fresh bundle with nothing scripted.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chat: ScriptedChat::new(),
            report: ScriptedReport::default(),
            analysis: ScriptedAnalysis::new(),
            fix_plan: ScriptedFixPlan::new(),
            actions: MockActions::new(),
            model_catalog: ScriptedModelCatalog::new(),
            subscriptions: ScriptedSubscriptions::new(),
            cache: SharedAiCache::new(16),
        }
    }

    /// Project the doubles into the port bundle the service consumes.
    #[must_use]
    pub fn to_ports(&self) -> AiPorts {
        AiPorts {
            chat_resolver: Arc::new(self.chat.clone()),
            report_resolvers: Arc::new(self.report.clone()),
            analysis: Arc::new(self.analysis.clone()),
            fix_plan: Arc::new(self.fix_plan.clone()),
            actions: Arc::new(self.actions.clone()),
            model_catalog: Arc::new(self.model_catalog.clone()),
            subscriptions: Arc::new(self.subscriptions.clone()),
            cache: self.cache.clone(),
        }
    }
}
