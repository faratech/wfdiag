//! The AI, remediation, and provider-setup seams.
//!
//! Four of the engine runtimes this facade drives expose a provider port of
//! their own, so the service can own the real runtime and a test can script the
//! provider underneath it:
//!
//! * agentic chat resolves each turn's transport through [`ChatResolverPort`];
//! * the scan report resolves through [`ReportResolverPort`].
//!
//! The other four do not, so this module declares the port instead and the
//! service talks to the runtime only through it:
//!
//! * [`ActionPort`] — `NativeActionRuntime::start` hard-wires
//!   `RealCatalogExecutor`, so a headless test cannot substitute a command
//!   recorder without one;
//! * [`AnalysisPort`] and [`FixPlanPort`] — the one-shot provider call inside
//!   those runtimes is not injectable, only the endpoint discovery around it
//!   is;
//! * [`ModelCatalogPort`] and [`SubscriptionPort`] — their runtimes'
//!   backend-injecting constructors are private to their crate.
//!
//! Everything here is cross-platform: the engine crates build on Linux, and
//! on-device Phi Silica resolves through `wfdiag_native_phi`, which is a stub
//! off Windows.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_analysis::{
    AnalysisWorkerEvent, DiagnosticAnalysisGeneration, FixPlanGeneration, FixPlanWorkerEvent,
    IssuePrioritizationGeneration, NativeAnalysisRuntime, NativeFixPlanRuntime,
};
use wfdiag_native_ai_chat::workers::provider_setup::{
    ProviderSetupRuntime, ProviderSetupWorkerEvent,
};
use wfdiag_native_ai_chat::workers::subscription_auth::{
    SubscriptionAuthRuntime, SubscriptionAuthWorkerEvent,
};
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallRuntime, SubscriptionInstallWorkerEvent,
};
use wfdiag_native_ai_chat::{
    ChatProvider, ChatResolveFuture, CompatChatProvider, ProviderResolver, ResolvedChatProvider,
    SubscriptionAuthProvider,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, CompatConfigPorts, FoundryEndpointSource,
    ModelCatalogRequest, OllamaSource, ProcessSubscriptionCliStatusSource, ProviderAvailability,
    SettingsProviderKeySource, SharedAiCache, SubscriptionCliStatusSource, SubscriptionConfigPorts,
    next_auto_local_route, parse_provider_preference, provider_config_fingerprint,
    resolve_compat_config, resolve_subscription_config,
};
use wfdiag_native_ai_report::{
    NativeReportRuntime, ReportFuture, ReportProviderResolver, ReportResolverFactory,
    ReportWorkerEvent, ResolvedReportProvider,
};
use wfdiag_native_phi::PhiChatProvider;
use wfdiag_native_remediation::broker::{
    ActionApproval, ActionPrepareInput, ActionProposal, ActionSnapshot,
};
use wfdiag_native_remediation::runtime::{
    ActionRunEvent, ActionRunSummary, ActionWorkerEvent, NativeActionRuntime,
};
use wfdiag_native_settings::SettingsService;

/// The wake callback every AI worker is started with.
pub type AiWake = Arc<dyn Fn() + Send + Sync>;

// ---------------------------------------------------------------- chat ----

/// Resolves one chat turn's concrete transport.
///
/// Secrets stay inside the returned provider; only the non-secret
/// `config_fingerprint` participates in cache identity.
pub trait ChatResolverPort: Send + Sync + 'static {
    /// Resolve `provider`. `cancel` lets a slow probe abandon early and lets an
    /// on-device provider stop generating.
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_>;
}

/// Adapts a shared [`ChatResolverPort`] to the chat runtime's own resolver
/// trait, which is `Send` but not `Sync` and is moved into the worker.
pub(crate) struct SharedChatResolver(pub(crate) Arc<dyn ChatResolverPort>);

impl ProviderResolver for SharedChatResolver {
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_> {
        self.0.resolve(provider, cancel)
    }
}

// -------------------------------------------------------------- report ----

/// Builds one report generation's provider resolver.
pub trait ReportResolverPort: Send + Sync + 'static {
    /// The resolver for a generation already routed to `provider`.
    fn resolver(
        &self,
        provider: AIProvider,
        availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver>;
}

pub(crate) struct SharedReportResolvers(pub(crate) Arc<dyn ReportResolverPort>);

impl ReportResolverFactory for SharedReportResolvers {
    fn resolver(
        &self,
        provider: AIProvider,
        availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver> {
        self.0.resolver(provider, availability)
    }
}

/// The report runtime plus its event stream.
pub struct ReportSession {
    /// The runtime handle.
    pub runtime: NativeReportRuntime,
    /// Its typed events.
    pub events: Receiver<ReportWorkerEvent>,
}

/// Start the report runtime over a resolver port. Used by the service; kept
/// here so the port and its one consumer stay together.
///
/// # Errors
///
/// Returns the OS diagnostic when the worker thread cannot start.
pub(crate) fn start_report_runtime(
    resolvers: Arc<dyn ReportResolverPort>,
    cache: SharedAiCache,
    wake: AiWake,
) -> Result<ReportSession, String> {
    NativeReportRuntime::start(Box::new(SharedReportResolvers(resolvers)), cache, wake)
        .map(|(runtime, events)| ReportSession { runtime, events })
        .map_err(|error| format!("Native AI report generation could not start: {error}"))
}

// ------------------------------------------------------------ analysis ----

/// One-shot analysis and issue prioritisation.
pub trait AnalysisHandle: Send + Sync {
    /// Queue a diagnostic interpretation.
    fn generate(&self, request_id: u64, generation: DiagnosticAnalysisGeneration) -> bool;
    /// Queue an issue prioritisation.
    fn prioritize(&self, request_id: u64, generation: IssuePrioritizationGeneration) -> bool;
    /// Cancel the request holding the worker's single slot.
    fn cancel(&self, request_id: u64) -> bool;
}

/// A started analysis runtime.
pub struct AnalysisSession {
    /// The handle.
    pub handle: Box<dyn AnalysisHandle>,
    /// Its typed events.
    pub events: Receiver<AnalysisWorkerEvent>,
}

/// Starts the one-shot analysis runtime.
pub trait AnalysisPort: Send + Sync {
    /// Start the worker.
    ///
    /// # Errors
    ///
    /// Returns a user-facing diagnostic when the worker cannot start.
    fn start(
        &self,
        settings: SettingsService,
        cache: SharedAiCache,
        wake: AiWake,
    ) -> Result<AnalysisSession, String>;
}

/// Fix-plan generation.
pub trait FixPlanHandle: Send + Sync {
    /// Queue a plan.
    fn generate(&self, request_id: u64, generation: FixPlanGeneration) -> bool;
    /// Cancel the request holding the worker's single slot.
    fn cancel(&self, request_id: u64) -> bool;
}

/// A started fix-plan runtime.
pub struct FixPlanSession {
    /// The handle.
    pub handle: Box<dyn FixPlanHandle>,
    /// Its typed events.
    pub events: Receiver<FixPlanWorkerEvent>,
}

/// Starts the fix-plan runtime.
pub trait FixPlanPort: Send + Sync {
    /// Start the worker.
    ///
    /// # Errors
    ///
    /// Returns a user-facing diagnostic when the worker cannot start.
    fn start(&self, settings: SettingsService, wake: AiWake) -> Result<FixPlanSession, String>;
}

// ------------------------------------------------------------- actions ----

/// Staged-proposal and run control.
///
/// Nothing here can authorise anything: `approve` hands the proposal id and a
/// freshly captured snapshot to the broker, which owns the Repair gate.
pub trait ActionHandle: Send + Sync {
    /// Stage catalog ids into a reviewable preview.
    fn prepare(&self, request_id: u64, input: ActionPrepareInput, snapshot: ActionSnapshot)
    -> bool;
    /// Approve a staged preview against a fresh snapshot.
    fn approve(
        &self,
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    ) -> bool;
    /// Drop an unused preview.
    fn discard(&self, proposal_id: String) -> bool;
    /// Ask a run to stop at its next safe boundary.
    ///
    /// # Errors
    ///
    /// Returns the runtime's diagnostic when the run is unknown or its current
    /// action cannot be stopped safely.
    fn cancel(&self, run_id: &str) -> Result<ActionRunSummary, String>;
    /// The newest-first bounded run history.
    fn history(&self) -> Vec<ActionRunSummary>;
    /// Unconsumed, unexpired previews.
    fn pending_proposals(&self) -> Vec<ActionProposal>;
}

/// A started action runtime, with both of its streams.
pub struct ActionSession {
    /// The handle.
    pub handle: Box<dyn ActionHandle>,
    /// Prepare/approve replies.
    pub events: Receiver<ActionWorkerEvent>,
    /// Live run transitions.
    pub runs: Receiver<ActionRunEvent>,
    /// Previews that survived a previous process lifetime.
    pub pending_proposals: Vec<ActionProposal>,
    /// The run history captured atomically with the subscription.
    pub history: Vec<ActionRunSummary>,
    /// The run still executing, if any.
    pub active_run: Option<ActionRunSummary>,
}

/// Starts the remediation runtime.
pub trait ActionPort: Send + Sync {
    /// Start the workers and subscribe to run events atomically.
    ///
    /// # Errors
    ///
    /// Returns a user-facing diagnostic when the workers cannot start.
    fn start(&self, wake: AiWake) -> Result<ActionSession, String>;
}

// ------------------------------------------------------- model catalog ----

/// Live model discovery.
pub trait ModelCatalogHandle: Send + Sync {
    /// Queue one catalog request.
    fn list_models(&self, request_id: u64, request: ModelCatalogRequest) -> bool;
    /// Cancel the request holding the worker's single slot.
    fn cancel(&self, request_id: u64) -> bool;
}

/// A started model-catalog runtime.
pub struct ModelCatalogSession {
    /// The handle.
    pub handle: Box<dyn ModelCatalogHandle>,
    /// Its typed events.
    pub events: Receiver<ProviderSetupWorkerEvent>,
}

/// Starts the model-catalog runtime.
pub trait ModelCatalogPort: Send + Sync {
    /// Start the worker.
    ///
    /// # Errors
    ///
    /// Returns a user-facing diagnostic when the worker cannot start.
    fn start(&self, settings: SettingsService, wake: AiWake)
    -> Result<ModelCatalogSession, String>;
}

// -------------------------------------------------------- subscriptions ---

/// Subscription-CLI account control.
pub trait SubscriptionAuthHandle: Send + Sync {
    /// Probe the account state.
    fn request_status(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool;
    /// Begin an interactive sign-in.
    fn sign_in(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool;
    /// Begin a sign-out.
    fn sign_out(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool;
    /// Cancel the operation holding the worker's single slot.
    fn cancel(&self, operation_id: u64) -> bool;
}

/// Subscription-CLI installation.
pub trait SubscriptionInstallHandle: Send + Sync {
    /// Install through Windows Package Manager. `confirmed` is the user's
    /// first, explicit approval.
    fn install_with_winget(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
    ) -> bool;
    /// Run the vendor bootstrap. This needs **both** approvals.
    fn install_with_vendor_fallback(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> bool;
    /// Cancel the install, killing its process tree.
    fn cancel(&self, request_id: u64) -> bool;
}

/// A started subscription runtime pair.
pub struct SubscriptionSession {
    /// Account control.
    pub auth: Box<dyn SubscriptionAuthHandle>,
    /// Account events.
    pub auth_events: Receiver<SubscriptionAuthWorkerEvent>,
    /// Installation control.
    pub install: Box<dyn SubscriptionInstallHandle>,
    /// Installation events.
    pub install_events: Receiver<SubscriptionInstallWorkerEvent>,
}

/// Starts the subscription runtimes.
pub trait SubscriptionPort: Send + Sync {
    /// Start both workers.
    ///
    /// # Errors
    ///
    /// Returns a user-facing diagnostic when either worker cannot start.
    fn start(&self, settings: SettingsService, wake: AiWake)
    -> Result<SubscriptionSession, String>;
}

// ------------------------------------------------------------- bundle -----

/// Every AI-domain seam, in one bundle.
#[derive(Clone)]
pub struct AiPorts {
    /// Resolves each chat turn's transport.
    pub chat_resolver: Arc<dyn ChatResolverPort>,
    /// Builds each report generation's resolver.
    pub report_resolvers: Arc<dyn ReportResolverPort>,
    /// Starts one-shot analysis and prioritisation.
    pub analysis: Arc<dyn AnalysisPort>,
    /// Starts fix-plan generation.
    pub fix_plan: Arc<dyn FixPlanPort>,
    /// Starts remediation staging and execution.
    pub actions: Arc<dyn ActionPort>,
    /// Starts live model discovery.
    pub model_catalog: Arc<dyn ModelCatalogPort>,
    /// Starts subscription account and installation control.
    pub subscriptions: Arc<dyn SubscriptionPort>,
    /// The response cache shared by analysis and reports.
    pub cache: SharedAiCache,
}

impl std::fmt::Debug for AiPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AiPorts").finish_non_exhaustive()
    }
}

impl AiPorts {
    /// The shipping bundle: real providers, real workers, real catalog
    /// execution.
    ///
    /// `foundry` and `ollama` are the local-endpoint discovery ports, which
    /// stay injectable because their probes touch the network and spawn a CLI.
    #[must_use]
    pub fn shipping(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
        cache: SharedAiCache,
    ) -> Self {
        let source = Arc::new(ProviderConfigSource::new(settings, foundry, ollama));
        Self {
            chat_resolver: Arc::new(ShippingChatResolver {
                source: Arc::clone(&source),
            }),
            report_resolvers: Arc::new(ShippingReportResolvers {
                source: Arc::clone(&source),
            }),
            analysis: Arc::new(ShippingAnalysis {
                source: Arc::clone(&source),
            }),
            fix_plan: Arc::new(ShippingFixPlan {
                source: Arc::clone(&source),
            }),
            actions: Arc::new(ShippingActions),
            model_catalog: Arc::new(ShippingModelCatalog {
                source: Arc::clone(&source),
            }),
            subscriptions: Arc::new(ShippingSubscriptions),
            cache,
        }
    }
}

// --------------------------------------------------- shipping adapters ----

/// Credential and endpoint ports, rebuilt per resolution so a settings edit or
/// a newly saved key applies to the very next request.
pub(crate) struct ProviderConfigSource {
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    subscription: Arc<dyn SubscriptionCliStatusSource>,
}

impl ProviderConfigSource {
    pub(crate) fn new(
        settings: SettingsService,
        foundry: Arc<dyn FoundryEndpointSource>,
        ollama: Arc<dyn OllamaSource>,
    ) -> Self {
        Self {
            settings,
            foundry,
            ollama,
            subscription: Arc::new(ProcessSubscriptionCliStatusSource::new()),
        }
    }

    fn compat_ports(&self) -> CompatConfigPorts {
        CompatConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            keys: Arc::new(SettingsProviderKeySource(self.settings.clone())),
            foundry: Arc::clone(&self.foundry),
            ollama: Arc::clone(&self.ollama),
        }
    }

    fn subscription_ports(&self) -> SubscriptionConfigPorts {
        SubscriptionConfigPorts {
            settings: self.settings.load().unwrap_or_default(),
            status: Arc::clone(&self.subscription),
        }
    }

    async fn resolve(&self, provider: AIProvider) -> Result<ResolvedChatProvider, String> {
        if provider == AIProvider::None {
            return Err("No AI provider is available".to_string());
        }
        let cfg = match provider {
            AIProvider::CodexCli | AIProvider::ClaudeCode => {
                resolve_subscription_config(provider, &self.subscription_ports()).await?
            }
            _ => resolve_compat_config(provider, &self.compat_ports()).await?,
        };
        let requested_model = cfg.model.clone();
        let config_fingerprint = provider_config_fingerprint(provider, &cfg);
        let chat: Arc<dyn ChatProvider> = Arc::new(CompatChatProvider { provider, cfg });
        Ok(ResolvedChatProvider {
            chat,
            config_fingerprint,
            requested_model,
        })
    }
}

fn phi_provider(cancel: Option<CancellationToken>) -> ResolvedChatProvider {
    let chat: Arc<dyn ChatProvider> = cancel.map_or_else(
        || Arc::new(PhiChatProvider::default()) as Arc<dyn ChatProvider>,
        |cancel| Arc::new(PhiChatProvider::new(move || cancel.is_cancelled())),
    );
    ResolvedChatProvider {
        chat,
        config_fingerprint: "provider=phi_silica;runtime=windows_ai".to_string(),
        requested_model: None,
    }
}

struct ShippingChatResolver {
    source: Arc<ProviderConfigSource>,
}

impl ChatResolverPort for ShippingChatResolver {
    fn resolve(&self, provider: AIProvider, cancel: CancellationToken) -> ChatResolveFuture<'_> {
        Box::pin(async move {
            if provider == AIProvider::PhiSilica {
                return Ok(phi_provider(Some(cancel)));
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => Err("AI request cancelled".to_string()),
                resolved = self.source.resolve(provider) => resolved,
            }
        })
    }
}

/// One report generation's resolver. The active provider and the availability
/// snapshot come from the same status reply, so the report core's Phi-to-local
/// policy cannot race a second, independently ordered probe.
struct ShippingReportResolver {
    source: Arc<ProviderConfigSource>,
    provider: AIProvider,
    availability: ProviderAvailability,
}

impl ReportProviderResolver for ShippingReportResolver {
    fn preference(&self) -> AIProviderPreference {
        parse_provider_preference(
            &self
                .source
                .settings
                .load_nonsecret_settings()
                .unwrap_or_default()
                .preferred_ai_provider,
        )
    }

    fn determine_active(&self, _preference: AIProviderPreference) -> ReportFuture<'_, AIProvider> {
        Box::pin(async move { self.provider })
    }

    fn next_auto_local(
        &self,
        preference: AIProviderPreference,
        tried: &[AIProvider],
    ) -> ReportFuture<'_, Option<AIProvider>> {
        let next = next_auto_local_route(preference, tried, self.availability);
        Box::pin(async move { next })
    }

    fn resolve(
        &self,
        provider: AIProvider,
    ) -> ReportFuture<'_, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            if provider == AIProvider::PhiSilica {
                return Ok(phi_provider(None));
            }
            self.source.resolve(provider).await
        })
    }
}

struct ShippingReportResolvers {
    source: Arc<ProviderConfigSource>,
}

impl ReportResolverPort for ShippingReportResolvers {
    fn resolver(
        &self,
        provider: AIProvider,
        availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver> {
        Arc::new(ShippingReportResolver {
            source: Arc::clone(&self.source),
            provider,
            availability,
        })
    }
}

struct ShippingAnalysis {
    source: Arc<ProviderConfigSource>,
}

struct NativeAnalysisHandle(NativeAnalysisRuntime);

impl AnalysisHandle for NativeAnalysisHandle {
    fn generate(&self, request_id: u64, generation: DiagnosticAnalysisGeneration) -> bool {
        if generation.force_refresh {
            self.0.retry(request_id, generation)
        } else {
            self.0.generate(request_id, generation)
        }
    }

    fn prioritize(&self, request_id: u64, generation: IssuePrioritizationGeneration) -> bool {
        if generation.force_refresh {
            self.0.reprioritize_issues(request_id, generation)
        } else {
            self.0.prioritize_issues(request_id, generation)
        }
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.0.cancel(request_id)
    }
}

impl AnalysisPort for ShippingAnalysis {
    fn start(
        &self,
        settings: SettingsService,
        cache: SharedAiCache,
        wake: AiWake,
    ) -> Result<AnalysisSession, String> {
        NativeAnalysisRuntime::start(
            settings,
            Arc::clone(&self.source.foundry),
            Arc::clone(&self.source.ollama),
            cache,
            wake,
        )
        .map(|(runtime, events)| AnalysisSession {
            handle: Box::new(NativeAnalysisHandle(runtime)),
            events,
        })
        .map_err(|error| format!("Native diagnostic AI could not start: {error}"))
    }
}

struct ShippingFixPlan {
    source: Arc<ProviderConfigSource>,
}

struct NativeFixPlanHandle(NativeFixPlanRuntime);

impl FixPlanHandle for NativeFixPlanHandle {
    fn generate(&self, request_id: u64, generation: FixPlanGeneration) -> bool {
        self.0.generate(request_id, generation)
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.0.cancel(request_id)
    }
}

impl FixPlanPort for ShippingFixPlan {
    fn start(&self, settings: SettingsService, wake: AiWake) -> Result<FixPlanSession, String> {
        NativeFixPlanRuntime::start(
            settings,
            Arc::clone(&self.source.foundry),
            Arc::clone(&self.source.ollama),
            wake,
        )
        .map(|(runtime, events)| FixPlanSession {
            handle: Box::new(NativeFixPlanHandle(runtime)),
            events,
        })
        .map_err(|error| format!("Native AI fix planning could not start: {error}"))
    }
}

struct ShippingActions;

struct NativeActionHandle(NativeActionRuntime);

impl ActionHandle for NativeActionHandle {
    fn prepare(
        &self,
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    ) -> bool {
        self.0.prepare(request_id, input, snapshot)
    }

    fn approve(
        &self,
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    ) -> bool {
        self.0.approve(request_id, proposal_id, snapshot, approval)
    }

    fn discard(&self, proposal_id: String) -> bool {
        self.0.discard(proposal_id)
    }

    fn cancel(&self, run_id: &str) -> Result<ActionRunSummary, String> {
        self.0.cancel(run_id)
    }

    fn history(&self) -> Vec<ActionRunSummary> {
        self.0.list_history()
    }

    fn pending_proposals(&self) -> Vec<ActionProposal> {
        self.0.list_pending_proposals()
    }
}

impl ActionPort for ShippingActions {
    fn start(&self, wake: AiWake) -> Result<ActionSession, String> {
        let (runtime, events) = NativeActionRuntime::start(Some(wake))
            .map_err(|error| format!("Native remediation unavailable · {error}"))?;
        let (runs, snapshot) = runtime.subscribe_run_events();
        Ok(ActionSession {
            handle: Box::new(NativeActionHandle(runtime)),
            events,
            runs,
            pending_proposals: snapshot.pending_proposals,
            history: snapshot.history,
            active_run: snapshot.active_run,
        })
    }
}

struct ShippingModelCatalog {
    source: Arc<ProviderConfigSource>,
}

struct NativeModelCatalogHandle(ProviderSetupRuntime);

impl ModelCatalogHandle for NativeModelCatalogHandle {
    fn list_models(&self, request_id: u64, request: ModelCatalogRequest) -> bool {
        self.0.list_models(request_id, request)
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.0.cancel(request_id)
    }
}

impl ModelCatalogPort for ShippingModelCatalog {
    fn start(
        &self,
        settings: SettingsService,
        wake: AiWake,
    ) -> Result<ModelCatalogSession, String> {
        ProviderSetupRuntime::start(
            settings,
            Arc::clone(&self.source.foundry),
            Arc::clone(&self.source.ollama),
            wake,
        )
        .map(|(runtime, events)| ModelCatalogSession {
            handle: Box::new(NativeModelCatalogHandle(runtime)),
            events,
        })
        .map_err(|error| format!("Native model discovery is unavailable: {error}"))
    }
}

struct ShippingSubscriptions;

struct NativeSubscriptionAuthHandle(SubscriptionAuthRuntime);

impl SubscriptionAuthHandle for NativeSubscriptionAuthHandle {
    fn request_status(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool {
        self.0.request_status(operation_id, provider, cli_path)
    }

    fn sign_in(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool {
        self.0.start_sign_in(operation_id, provider, cli_path)
    }

    fn sign_out(
        &self,
        operation_id: u64,
        provider: SubscriptionAuthProvider,
        cli_path: Option<String>,
    ) -> bool {
        self.0.start_sign_out(operation_id, provider, cli_path)
    }

    fn cancel(&self, operation_id: u64) -> bool {
        self.0.cancel(operation_id)
    }
}

struct NativeSubscriptionInstallHandle(SubscriptionInstallRuntime);

impl SubscriptionInstallHandle for NativeSubscriptionInstallHandle {
    fn install_with_winget(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
    ) -> bool {
        self.0.install_with_winget(request_id, provider, confirmed)
    }

    fn install_with_vendor_fallback(
        &self,
        request_id: u64,
        provider: SubscriptionAuthProvider,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> bool {
        self.0
            .install_with_vendor_fallback(request_id, provider, confirmed, fallback_confirmed)
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.0.cancel(request_id)
    }
}

impl SubscriptionPort for ShippingSubscriptions {
    fn start(
        &self,
        settings: SettingsService,
        wake: AiWake,
    ) -> Result<SubscriptionSession, String> {
        let (auth, auth_events) = SubscriptionAuthRuntime::start(settings, Arc::clone(&wake))
            .map_err(|error| format!("Subscription account controls are unavailable: {error}"))?;
        let (install, install_events) = SubscriptionInstallRuntime::start(wake)
            .map_err(|error| format!("Subscription CLI installation is unavailable: {error}"))?;
        Ok(SubscriptionSession {
            auth: Box::new(NativeSubscriptionAuthHandle(auth)),
            auth_events,
            install: Box::new(NativeSubscriptionInstallHandle(install)),
            install_events,
        })
    }
}

/// The per-worker teardown budget every AI runtime is stopped inside.
pub(crate) const AI_STOP_BUDGET: Duration = Duration::from_secs(2);
