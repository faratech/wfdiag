//! In-memory implementations of every port, for headless tests.
//!
//! These are deliberately public: the crate's integration tests drive the real
//! [`crate::AppService`] through its real command/event API, and the only
//! thing they replace is the environment. Nothing here touches the network,
//! the registry, `WinRT`, or the user's disk.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::oneshot;
use wfdiag_native_ai_provider::{
    AIProviderPreference, ANTHROPIC_DEFAULT_MODEL, BackendFuture, DEEPSEEK_DEFAULT_MODEL,
    FOUNDRY_DEFAULT_MODEL, GEMINI_DEFAULT_MODEL, OPENAI_DEFAULT_MODEL, ProviderManagementBackend,
    ProviderModelDefaults, ProviderProbeSnapshot, ProviderSettingsSnapshot, ProviderStatusInput,
};
use wfdiag_native_diagnostics::{
    DiagnosticExecutor, DiagnosticFuture, DiagnosticOutput, DiagnosticTask,
};
use wfdiag_native_issues::Timestamp;
use wfdiag_native_settings::{
    AllowAllSettings, CredentialStorage, ProviderKeyId, SettingsError, SettingsStorage,
};
use wfdiag_native_system::{ArchitectureSnapshot, SystemError, SystemInfo, SystemProvider};
use wfdiag_native_update::{
    CurrentVersionProvider, PackageSignature, ReleaseHttp, ReleaseRequest, ReleaseResponse,
    SignatureProvider, StaticCurrentVersion,
};
use wfdiag_ui_core::{UiEventPublisher, ui_event_bus};

use super::monitor::{
    MonitorHandle, MonitorPort, MonitorProfileKind, MonitorSession, NetworkConnection,
    NetworkConnectionsReply, ProcessPage, ProcessPageReply, ProcessQuery, ProcessQueryOutcome,
};
use super::{AppPorts, ElevationPort, EnvironmentPort, UpdateThrottlePort};

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What one scripted task produces.
#[derive(Clone, Debug)]
pub struct TaskScript {
    /// Whether the task reports success.
    pub success: bool,
    /// The task's JSON (or plain) output.
    pub output: String,
    /// An optional error message.
    pub error: Option<String>,
    /// The reported duration.
    pub duration_ms: u64,
}

impl TaskScript {
    /// A successful task whose output is `output`.
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
            duration_ms: 1,
        }
    }

    /// A failed task carrying `error`.
    #[must_use]
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
            duration_ms: 1,
        }
    }
}

#[derive(Debug)]
struct ExecutorState {
    tasks: Vec<DiagnosticTask>,
    scripts: HashMap<String, TaskScript>,
    executed: Vec<String>,
}

/// A [`DiagnosticExecutor`] whose catalog and per-task output are scripted.
#[derive(Clone)]
pub struct ScriptedExecutor {
    state: Arc<Mutex<ExecutorState>>,
    hold: Arc<Mutex<Option<String>>>,
    started: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    release_rx: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    release_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl std::fmt::Debug for ScriptedExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScriptedExecutor")
            .finish_non_exhaustive()
    }
}

impl Default for ScriptedExecutor {
    fn default() -> Self {
        Self::with_tasks(&["os_info", "processor", "logical_disk"])
    }
}

impl ScriptedExecutor {
    /// An executor whose catalog is `task_ids`, every task succeeding.
    #[must_use]
    pub fn with_tasks(task_ids: &[&str]) -> Self {
        let tasks = task_ids
            .iter()
            .map(|id| DiagnosticTask {
                id: (*id).to_string(),
                name: format!("Task {id}"),
                description: format!("Scripted task {id}"),
                category: "Test".to_string(),
                admin_required: false,
            })
            .collect();
        let scripts = task_ids
            .iter()
            .map(|id| {
                (
                    (*id).to_string(),
                    TaskScript::ok(format!("{{\"task\":\"{id}\"}}")),
                )
            })
            .collect();
        Self {
            state: Arc::new(Mutex::new(ExecutorState {
                tasks,
                scripts,
                executed: Vec::new(),
            })),
            hold: Arc::new(Mutex::new(None)),
            started: Arc::new(Mutex::new(None)),
            release_rx: Arc::new(Mutex::new(None)),
            release_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Remove one task from the catalog, so the runtime refuses to run it.
    /// This is how a test reaches the "session could not start" rollback path
    /// without breaking the executor for every other task.
    pub fn remove_task(&self, task_id: &str) {
        let mut state = lock(&self.state);
        state.tasks.retain(|task| task.id != task_id);
        state.scripts.remove(task_id);
    }

    /// Replace one task's scripted outcome.
    pub fn script(&self, task_id: &str, script: TaskScript) {
        lock(&self.state)
            .scripts
            .insert(task_id.to_string(), script);
    }

    /// The task ids executed so far, in completion order.
    #[must_use]
    pub fn executed(&self) -> Vec<String> {
        lock(&self.state).executed.clone()
    }

    /// Block `task_id` inside the executor until [`Self::release`] is called.
    ///
    /// The returned receiver yields once the task has actually started, so a
    /// test can observe the running phase deterministically. The executor
    /// blocks its worker thread while held; that is acceptable for a test
    /// double and keeps the API usable from a synchronous test.
    #[must_use]
    pub fn hold(&self, task_id: &str) -> mpsc::Receiver<()> {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *lock(&self.hold) = Some(task_id.to_string());
        *lock(&self.started) = Some(started_tx);
        *lock(&self.release_rx) = Some(release_rx);
        *lock(&self.release_tx) = Some(release_tx);
        started_rx
    }

    /// Let a held task finish.
    pub fn release(&self) {
        if let Some(sender) = lock(&self.release_tx).take() {
            let _ = sender.send(());
        }
    }
}

impl DiagnosticExecutor for ScriptedExecutor {
    fn available_tasks(&self) -> Vec<DiagnosticTask> {
        lock(&self.state).tasks.clone()
    }

    fn execute(&self, task_id: String) -> DiagnosticFuture<'_> {
        Box::pin(async move {
            let held = lock(&self.hold).as_deref() == Some(task_id.as_str());
            if held {
                if let Some(started) = lock(&self.started).as_ref() {
                    let _ = started.send(());
                }
                let released = lock(&self.release_rx).as_ref().map(mpsc::Receiver::recv);
                let _ = released;
            }
            let script = {
                let mut state = lock(&self.state);
                state.executed.push(task_id.clone());
                state.scripts.get(&task_id).cloned()
            };
            let script = script.unwrap_or_else(|| TaskScript::ok("{}"));
            DiagnosticOutput {
                success: script.success,
                output: script.output,
                error: script.error,
                duration_ms: script.duration_ms,
            }
        })
    }
}

/// A [`SystemProvider`] that returns fixed identity values.
#[derive(Clone, Debug)]
pub struct MockSystemProvider {
    info: Arc<Mutex<Result<SystemInfo, String>>>,
}

impl Default for MockSystemProvider {
    fn default() -> Self {
        Self {
            info: Arc::new(Mutex::new(Ok(SystemInfo {
                computer_name: "TEST-PC".to_string(),
                os_version: "Windows 11".to_string(),
                is_admin: true,
            }))),
        }
    }
}

impl MockSystemProvider {
    /// Replace the identity this provider reports.
    pub fn set_info(&self, info: Result<SystemInfo, String>) {
        *lock(&self.info) = info;
    }
}

impl SystemProvider for MockSystemProvider {
    fn architecture(&self) -> Result<ArchitectureSnapshot, SystemError> {
        Ok(ArchitectureSnapshot {
            process_architecture: 9,
            process_architecture_name: "x64".to_string(),
            native_architecture: 9,
            native_architecture_name: "x64".to_string(),
            is_emulated: false,
            page_size: 4096,
            processor_count: 8,
            emulation_status: "Native x64 execution".to_string(),
        })
    }

    fn system_info(&self) -> Result<SystemInfo, SystemError> {
        lock(&self.info).clone().map_err(SystemError::Collection)
    }
}

/// An in-memory settings document.
#[derive(Clone, Debug, Default)]
pub struct MemorySettingsStorage {
    document: Arc<Mutex<Option<Vec<u8>>>>,
}

impl MemorySettingsStorage {
    /// The bytes currently persisted, if any.
    #[must_use]
    pub fn document(&self) -> Option<Vec<u8>> {
        lock(&self.document).clone()
    }

    /// Seed the document a load will read.
    pub fn seed(&self, bytes: Vec<u8>) {
        *lock(&self.document) = Some(bytes);
    }
}

impl SettingsStorage for MemorySettingsStorage {
    fn load(&self) -> Result<Option<Vec<u8>>, SettingsError> {
        Ok(lock(&self.document).clone())
    }

    fn save(&self, serialized: &[u8]) -> Result<(), SettingsError> {
        *lock(&self.document) = Some(serialized.to_vec());
        Ok(())
    }
}

/// In-memory provider API keys.
#[derive(Clone, Debug, Default)]
pub struct MemoryCredentialStorage {
    keys: Arc<Mutex<HashMap<ProviderKeyId, String>>>,
}

impl MemoryCredentialStorage {
    /// Whether a key is stored for `provider`.
    #[must_use]
    pub fn is_set(&self, provider: ProviderKeyId) -> bool {
        lock(&self.keys).contains_key(&provider)
    }
}

impl CredentialStorage for MemoryCredentialStorage {
    fn store(&self, provider: ProviderKeyId, key: &str) -> Result<(), SettingsError> {
        lock(&self.keys).insert(provider, key.to_string());
        Ok(())
    }

    fn load(&self, provider: ProviderKeyId) -> Result<Option<String>, SettingsError> {
        Ok(lock(&self.keys).get(&provider).cloned())
    }

    fn clear(&self, provider: ProviderKeyId) -> Result<(), SettingsError> {
        lock(&self.keys).remove(&provider);
        Ok(())
    }
}

/// A scripted GitHub release transport.
#[derive(Clone, Debug)]
pub struct ScriptedReleaseHttp {
    response: Arc<Mutex<Result<ReleaseResponse, String>>>,
    requests: Arc<Mutex<Vec<ReleaseRequest>>>,
}

impl Default for ScriptedReleaseHttp {
    fn default() -> Self {
        Self {
            response: Arc::new(Mutex::new(Ok(ReleaseResponse {
                status: 200,
                body: br#"{"tag_name":"v0.0.1","html_url":"https://example.invalid","draft":false,"prerelease":false}"#
                    .to_vec(),
            }))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ScriptedReleaseHttp {
    /// Answer the next fetch with this response (or transport failure).
    pub fn set_response(&self, response: Result<ReleaseResponse, String>) {
        *lock(&self.response) = response;
    }

    /// Answer with a release whose tag is `tag`.
    pub fn set_release(&self, tag: &str, html_url: &str) {
        let body = format!(
            r#"{{"tag_name":"{tag}","html_url":"{html_url}","draft":false,"prerelease":false}}"#
        );
        self.set_response(Ok(ReleaseResponse {
            status: 200,
            body: body.into_bytes(),
        }));
    }

    /// Every request the service issued.
    #[must_use]
    pub fn requests(&self) -> Vec<ReleaseRequest> {
        lock(&self.requests).clone()
    }
}

impl ReleaseHttp for ScriptedReleaseHttp {
    fn fetch_latest(&self, request: &ReleaseRequest) -> Result<ReleaseResponse, String> {
        lock(&self.requests).push(request.clone());
        lock(&self.response).clone()
    }
}

/// A package-signature provider with settable answers.
#[derive(Clone, Debug)]
pub struct MockSignatureProvider {
    identity: Arc<Mutex<bool>>,
    signature: Arc<Mutex<Result<PackageSignature, String>>>,
}

impl Default for MockSignatureProvider {
    fn default() -> Self {
        Self {
            identity: Arc::new(Mutex::new(false)),
            signature: Arc::new(Mutex::new(Ok(PackageSignature::Other))),
        }
    }
}

impl MockSignatureProvider {
    /// Make this look like an installed Store package, which silences the
    /// GitHub update channel entirely.
    pub fn set_store_install(&self, store: bool) {
        *lock(&self.identity) = store;
        *lock(&self.signature) = Ok(if store {
            PackageSignature::Store
        } else {
            PackageSignature::Other
        });
    }
}

impl SignatureProvider for MockSignatureProvider {
    fn has_package_identity(&self) -> bool {
        *lock(&self.identity)
    }

    fn signature(&self) -> Result<PackageSignature, String> {
        lock(&self.signature).clone()
    }
}

/// A provider-management backend with a scripted probe snapshot.
#[derive(Clone, Debug)]
pub struct MockProviderBackend {
    probes: Arc<Mutex<ProviderProbeSnapshot>>,
    settings: Arc<Mutex<ProviderSettingsSnapshot>>,
    preference: Arc<Mutex<AIProviderPreference>>,
    identity: Arc<Mutex<bool>>,
    cleared_caches: Arc<Mutex<Vec<Option<String>>>>,
    ollama_models: Arc<Mutex<Result<Vec<String>, String>>>,
}

impl Default for MockProviderBackend {
    fn default() -> Self {
        Self {
            probes: Arc::new(Mutex::new(ProviderProbeSnapshot::default())),
            settings: Arc::new(Mutex::new(ProviderSettingsSnapshot::default())),
            preference: Arc::new(Mutex::new(AIProviderPreference::default())),
            identity: Arc::new(Mutex::new(false)),
            cleared_caches: Arc::new(Mutex::new(Vec::new())),
            ollama_models: Arc::new(Mutex::new(Ok(Vec::new()))),
        }
    }
}

impl MockProviderBackend {
    /// Replace the probe results a status refresh projects from.
    pub fn set_probes(&self, probes: ProviderProbeSnapshot) {
        *lock(&self.probes) = probes;
    }

    /// Replace the non-secret settings a status refresh projects from.
    pub fn set_settings(&self, settings: ProviderSettingsSnapshot) {
        *lock(&self.settings) = settings;
    }

    /// Declare whether this process has package identity, which gates the
    /// Phi Silica preference.
    pub fn set_package_identity(&self, identity: bool) {
        *lock(&self.identity) = identity;
    }

    /// The preference most recently applied.
    #[must_use]
    pub fn preference(&self) -> AIProviderPreference {
        *lock(&self.preference)
    }

    /// Every cache-clear request, in order.
    #[must_use]
    pub fn cleared_caches(&self) -> Vec<Option<String>> {
        lock(&self.cleared_caches).clone()
    }

    /// Set the model list the Ollama probe returns.
    pub fn set_ollama_models(&self, models: Result<Vec<String>, String>) {
        *lock(&self.ollama_models) = models;
    }
}

impl ProviderManagementBackend for MockProviderBackend {
    fn status_input(&self) -> BackendFuture<'_, ProviderStatusInput> {
        Box::pin(async move {
            ProviderStatusInput {
                preference: *lock(&self.preference),
                settings: lock(&self.settings).clone(),
                probes: lock(&self.probes).clone(),
                defaults: ProviderModelDefaults {
                    foundry: FOUNDRY_DEFAULT_MODEL.to_string(),
                    openai: OPENAI_DEFAULT_MODEL.to_string(),
                    anthropic: ANTHROPIC_DEFAULT_MODEL.to_string(),
                    gemini: GEMINI_DEFAULT_MODEL.to_string(),
                    deepseek: DEEPSEEK_DEFAULT_MODEL.to_string(),
                },
            }
        })
    }

    fn has_package_identity(&self) -> bool {
        *lock(&self.identity)
    }

    fn set_preference(&self, preference: AIProviderPreference) {
        *lock(&self.preference) = preference;
    }

    fn clear_cache(&self, session_id: Option<&str>) {
        lock(&self.cleared_caches).push(session_id.map(str::to_string));
    }

    fn list_ollama_models(&self) -> BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move { lock(&self.ollama_models).clone() })
    }
}

/// A live-monitoring port whose samples and pages are pushed by the test.
#[derive(Clone, Debug, Default)]
pub struct ScriptedMonitor {
    page: Arc<Mutex<ProcessPage>>,
    connections: Arc<Mutex<Vec<NetworkConnection>>>,
    publisher: Arc<Mutex<Option<UiEventPublisher>>>,
    control: Arc<MonitorControl>,
    stall: Arc<std::sync::atomic::AtomicBool>,
    held: Arc<Mutex<Vec<oneshot::Sender<ProcessQueryOutcome>>>>,
}

/// What a [`ScriptedMonitor`] was asked to do.
#[derive(Debug, Default)]
pub struct MonitorControl {
    paused: std::sync::atomic::AtomicBool,
    refreshes: AtomicU64,
}

impl MonitorControl {
    /// Whether the collector is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// How many immediate samples were requested.
    #[must_use]
    pub fn refreshes(&self) -> u64 {
        self.refreshes.load(Ordering::Relaxed)
    }
}

impl ScriptedMonitor {
    /// The control record, for assertions.
    #[must_use]
    pub fn control(&self) -> Arc<MonitorControl> {
        Arc::clone(&self.control)
    }

    /// Accept process queries but never answer them, so a test can observe
    /// the reply deadline instead of a hang.
    pub fn stall_process_queries(&self, stall: bool) {
        self.stall.store(stall, Ordering::Relaxed);
    }

    /// Replace the page every process query answers with.
    pub fn set_page(&self, page: ProcessPage) {
        *lock(&self.page) = page;
    }

    /// Replace the connection list.
    pub fn set_connections(&self, connections: Vec<NetworkConnection>) {
        *lock(&self.connections) = connections;
    }

    /// Publish one telemetry sample to the started session, if any.
    ///
    /// Returns `false` when no session has been started.
    #[must_use]
    pub fn publish(&self, stats: wfdiag_ui_core::SystemStats) -> bool {
        let publisher = lock(&self.publisher).clone();
        publisher.is_some_and(|publisher| {
            publisher
                .try_publish(wfdiag_ui_core::UiEvent::SystemStats(stats))
                .is_ok()
        })
    }
}

struct ScriptedMonitorHandle {
    page: Arc<Mutex<ProcessPage>>,
    connections: Arc<Mutex<Vec<NetworkConnection>>>,
    control: Arc<MonitorControl>,
    stall: Arc<std::sync::atomic::AtomicBool>,
    held: Arc<Mutex<Vec<oneshot::Sender<ProcessQueryOutcome>>>>,
}

impl MonitorHandle for ScriptedMonitorHandle {
    fn pause(&self) -> bool {
        self.control.paused.store(true, Ordering::Relaxed);
        true
    }

    fn resume(&self) -> bool {
        self.control.paused.store(false, Ordering::Relaxed);
        true
    }

    fn refresh(&self) -> bool {
        self.control.refreshes.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn request_processes(&self, query: ProcessQuery) -> Result<ProcessPageReply, String> {
        let (sender, receiver) = oneshot::channel();
        if self.stall.load(Ordering::Relaxed) {
            // Keep the sender alive so the reply never lands and never closes.
            lock(&self.held).push(sender);
            return Ok(receiver);
        }
        let mut page = lock(&self.page).clone();
        page.offset = query.offset;
        page.limit = query.limit;
        let _ = sender.send(ProcessQueryOutcome::Page(Box::new(page)));
        Ok(receiver)
    }

    fn request_network_connections(&self) -> Result<NetworkConnectionsReply, String> {
        let (sender, receiver) = oneshot::channel();
        let _ = sender.send(lock(&self.connections).clone());
        Ok(receiver)
    }
}

impl MonitorPort for ScriptedMonitor {
    fn start(&self, _profile: MonitorProfileKind) -> Result<Option<MonitorSession>, String> {
        let capacity = std::num::NonZeroUsize::new(32).expect("32 is non-zero");
        let (publisher, events) = ui_event_bus(capacity);
        *lock(&self.publisher) = Some(publisher);
        Ok(Some(MonitorSession {
            handle: Box::new(ScriptedMonitorHandle {
                page: Arc::clone(&self.page),
                connections: Arc::clone(&self.connections),
                control: Arc::clone(&self.control),
                stall: Arc::clone(&self.stall),
                held: Arc::clone(&self.held),
            }),
            events,
        }))
    }
}

/// A clock a test can set.
#[derive(Clone, Debug)]
pub struct MockEnvironment {
    now_millis: Arc<AtomicU64>,
    temp_file_count: Arc<Mutex<Option<usize>>>,
}

impl Default for MockEnvironment {
    fn default() -> Self {
        Self {
            now_millis: Arc::new(AtomicU64::new(1_750_000_000_000)),
            temp_file_count: Arc::new(Mutex::new(Some(0))),
        }
    }
}

impl MockEnvironment {
    /// Move the clock forward.
    pub fn advance(&self, millis: u64) {
        self.now_millis.fetch_add(millis, Ordering::Relaxed);
    }

    /// Set the temporary-file count issue detection reads.
    pub fn set_temp_file_count(&self, count: Option<usize>) {
        *lock(&self.temp_file_count) = count;
    }
}

impl EnvironmentPort for MockEnvironment {
    fn now(&self) -> Timestamp {
        let millis = self.now_millis.load(Ordering::Relaxed);
        Timestamp::from_secs(i64::try_from(millis / 1000).unwrap_or(i64::MAX))
    }

    fn now_millis(&self) -> u64 {
        self.now_millis.load(Ordering::Relaxed)
    }

    fn temp_file_count(&self) -> Option<usize> {
        *lock(&self.temp_file_count)
    }
}

/// An in-memory update throttle.
#[derive(Clone, Debug, Default)]
pub struct MemoryUpdateThrottle {
    last_run: Arc<Mutex<Option<u64>>>,
}

impl MemoryUpdateThrottle {
    /// The recorded timestamp of the last completed check.
    #[must_use]
    pub fn last_run(&self) -> Option<u64> {
        *lock(&self.last_run)
    }

    /// Pretend a check ran at `millis`.
    pub fn set_last_run(&self, millis: Option<u64>) {
        *lock(&self.last_run) = millis;
    }
}

impl UpdateThrottlePort for MemoryUpdateThrottle {
    fn should_check(&self, now_millis: u64) -> bool {
        let last = lock(&self.last_run).map(|value| value.to_string());
        wfdiag_native_update::policy::should_check(last.as_deref(), now_millis)
    }

    fn record(&self, now_millis: u64) -> Result<(), String> {
        *lock(&self.last_run) = Some(now_millis);
        Ok(())
    }
}

/// An elevation port that records the request.
#[derive(Clone, Debug, Default)]
pub struct MockElevation {
    outcome: Arc<Mutex<Option<Result<bool, String>>>>,
    requests: Arc<AtomicU64>,
}

impl MockElevation {
    /// How many relaunches were requested.
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Set what the next relaunch reports.
    pub fn set_outcome(&self, outcome: Result<bool, String>) {
        *lock(&self.outcome) = Some(outcome);
    }
}

impl ElevationPort for MockElevation {
    fn restart_as_admin(&self) -> Result<bool, String> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        lock(&self.outcome).clone().unwrap_or(Ok(false))
    }
}

/// Every mock, with the handles a test needs to script and inspect them.
#[derive(Clone, Debug)]
pub struct MockPorts {
    /// The scripted diagnostic executor.
    pub executor: ScriptedExecutor,
    /// The host identity provider.
    pub system: MockSystemProvider,
    /// The settings document.
    pub settings_storage: MemorySettingsStorage,
    /// Provider API keys.
    pub credentials: MemoryCredentialStorage,
    /// The GitHub release transport.
    pub release_http: ScriptedReleaseHttp,
    /// Package identity and signature.
    pub signature: MockSignatureProvider,
    /// AI provider probes and mutations.
    pub provider_backend: MockProviderBackend,
    /// Live monitoring.
    pub monitor: ScriptedMonitor,
    /// The clock and temporary-directory inputs.
    pub environment: MockEnvironment,
    /// The update throttle.
    pub update_throttle: MemoryUpdateThrottle,
    /// Elevation requests.
    pub elevation: MockElevation,
    /// The version the update check compares against.
    pub current_version: String,
}

impl Default for MockPorts {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPorts {
    /// A fresh bundle with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            executor: ScriptedExecutor::default(),
            system: MockSystemProvider::default(),
            settings_storage: MemorySettingsStorage::default(),
            credentials: MemoryCredentialStorage::default(),
            release_http: ScriptedReleaseHttp::default(),
            signature: MockSignatureProvider::default(),
            provider_backend: MockProviderBackend::default(),
            monitor: ScriptedMonitor::default(),
            environment: MockEnvironment::default(),
            update_throttle: MemoryUpdateThrottle::default(),
            elevation: MockElevation::default(),
            current_version: "2.5.8".to_string(),
        }
    }

    /// Build the port bundle the service consumes.
    ///
    /// # Panics
    ///
    /// Panics only if the compiled-in `0.0.0` fallback version ever stops
    /// being valid semantic version syntax.
    #[must_use]
    pub fn to_ports(&self) -> AppPorts {
        AppPorts {
            diagnostics: Arc::new(self.executor.clone()),
            system: Arc::new(self.system.clone()),
            settings_storage: Arc::new(self.settings_storage.clone()),
            credentials: Arc::new(self.credentials.clone()),
            settings_validator: Arc::new(AllowAllSettings),
            release_http: Arc::new(self.release_http.clone()),
            signature: Arc::new(self.signature.clone()),
            current_version: Arc::new(
                StaticCurrentVersion::parse(&self.current_version).unwrap_or_else(|_| {
                    StaticCurrentVersion::parse("0.0.0").expect("0.0.0 is valid semver")
                }),
            ) as Arc<dyn CurrentVersionProvider>,
            provider_backend: Arc::new(self.provider_backend.clone()),
            monitor: Arc::new(self.monitor.clone()),
            elevation: Arc::new(self.elevation.clone()),
            environment: Arc::new(self.environment.clone()),
            update_throttle: Arc::new(self.update_throttle.clone()),
        }
    }

    /// Consume the bundle, keeping only the ports.
    #[must_use]
    pub fn into_ports(self) -> AppPorts {
        self.to_ports()
    }
}

#[cfg(test)]
mod tests {
    use super::{MockPorts, ScriptedExecutor, TaskScript};
    use std::future::Future;
    use wfdiag_native_diagnostics::DiagnosticExecutor;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    #[test]
    fn the_scripted_executor_reports_its_catalog_and_scripted_outputs() {
        let executor = ScriptedExecutor::with_tasks(&["os_info"]);
        executor.script("os_info", TaskScript::failed("nope"));
        assert_eq!(executor.available_tasks().len(), 1);
        let output = block_on(executor.execute("os_info".to_string()));
        assert!(!output.success);
        assert_eq!(output.error.as_deref(), Some("nope"));
        assert_eq!(executor.executed(), ["os_info"]);
    }

    #[test]
    fn the_mock_bundle_builds_a_complete_port_set() {
        let mocks = MockPorts::new();
        let ports = mocks.to_ports();
        assert!(!ports.signature.has_package_identity());
        assert_eq!(ports.environment.temp_file_count(), Some(0));
    }
}
