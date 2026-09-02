use crate::credential_store::{CredentialStore, CredentialStoreProbe, HostCredentialStoreProbe};
use crate::subscription_spec::{StatusVerdict, parse_status_output, subscription_cli_spec};
use crate::{
    BackendFuture, CliObstacle, CliProbeSnapshot, FoundryEndpointSource, SubscriptionCli,
    SubscriptionCliStatusSource, normalize_base_url,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const PROBE_TTL: Duration = Duration::from_secs(30);
const CLI_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const FOUNDRY_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const FOUNDRY_HEALTH_TIMEOUT: Duration = Duration::from_secs(4);

/// Environment credentials that override a CLI's subscription login.
///
/// Every concrete CLI probe removes this closed list before starting a child,
/// preserving the shipping rule that status reflects the CLI-owned account
/// rather than an API key inherited from `WFDiag`'s environment.
pub const SUBSCRIPTION_OVERRIDE_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
];

/// Extract `scheme://host:port` from the first HTTP(S) URL selected by the
/// shipping 2.5.8 Foundry status parser.
#[must_use]
pub fn extract_http_base(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url = &text[start..];
    let end = url
        .find(|character: char| character.is_whitespace() || character == '"' || character == '\'')
        .unwrap_or(url.len());
    let url = &url[..end];
    let scheme_end = url.find("://")? + 3;
    if url.len() <= scheme_end {
        return None;
    }
    let base_end = url[scheme_end..]
        .find('/')
        .map_or(url.len(), |index| scheme_end + index);
    Some(url[..base_end].to_string())
}

/// A healthy Foundry Local status response contains at least one string in
/// either the documented `Endpoints` field or its observed lowercase form.
#[must_use]
pub fn valid_foundry_status_body(body: &Value) -> bool {
    body.get("Endpoints")
        .or_else(|| body.get("endpoints"))
        .and_then(Value::as_array)
        .is_some_and(|endpoints| endpoints.iter().any(Value::is_string))
}

#[derive(Debug, Clone)]
struct ProcessRequest {
    program: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    what: String,
}

#[derive(Debug, Clone)]
struct ProcessOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait ProcessRunner: Send + Sync + 'static {
    fn run(&self, request: ProcessRequest) -> BackendFuture<'_, Result<ProcessOutput, String>>;
}

#[derive(Debug, Default)]
struct TokioProcessRunner;

impl ProcessRunner for TokioProcessRunner {
    fn run(&self, request: ProcessRequest) -> BackendFuture<'_, Result<ProcessOutput, String>> {
        Box::pin(async move { run_headless(request).await })
    }
}

async fn run_headless(request: ProcessRequest) -> Result<ProcessOutput, String> {
    let mut command = tokio::process::Command::new(&request.program);
    command.args(&request.args);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        if let Ok(directory) = bridge_workdir().await {
            command.current_dir(directory);
        }
    }
    for variable in SUBSCRIPTION_OVERRIDE_ENV_VARS {
        command.env_remove(variable);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let timeout = request.timeout;
    let what = request.what;
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("{what} did not answer within {} seconds", timeout.as_secs()))?
        .map_err(|error| format!("Could not start {what}: {error}"))?;
    Ok(ProcessOutput {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

#[cfg(windows)]
const WORKDIR_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(windows)]
fn workdir_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(base) = dirs::config_dir() {
        candidates.push(
            base.join("com.windowsforum.diagnostics")
                .join("cli-bridge-cwd"),
        );
    }
    candidates.push(
        std::env::temp_dir()
            .join("com.windowsforum.diagnostics")
            .join("cli-bridge-cwd"),
    );
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".wfdiag").join("cli-bridge-cwd"));
    }
    candidates
}

#[cfg(windows)]
async fn windows_cwd_is_spawnable(directory: &Path) -> bool {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let system32 = std::env::var_os("SystemRoot").map_or_else(
        || PathBuf::from(r"C:\Windows\System32"),
        |root| PathBuf::from(root).join("System32"),
    );
    let mut command = tokio::process::Command::new(system32.join("cmd.exe"));
    command.args(["/d", "/c", "cd"]);
    command.current_dir(directory);
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(WORKDIR_PROBE_TIMEOUT, command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

#[cfg(windows)]
async fn bridge_workdir() -> Result<PathBuf, String> {
    use std::sync::OnceLock;

    static VALIDATED_WORKDIR: OnceLock<PathBuf> = OnceLock::new();
    if let Some(directory) = VALIDATED_WORKDIR.get() {
        // Best-effort recreate (the user may have deleted the directory after
        // validation); a real failure surfaces from the child spawn itself.
        let _ = std::fs::create_dir_all(directory);
        return Ok(directory.clone());
    }

    let mut failures = Vec::new();
    for candidate in workdir_candidates() {
        if let Err(error) = std::fs::create_dir_all(&candidate) {
            failures.push(format!("{}: {error}", candidate.display()));
            continue;
        }
        if !windows_cwd_is_spawnable(&candidate).await {
            failures.push(format!(
                "{}: child processes cannot start there",
                candidate.display()
            ));
            continue;
        }
        let _ = VALIDATED_WORKDIR.set(candidate.clone());
        return Ok(candidate);
    }
    Err(format!(
        "No usable working directory for CLI bridge runs ({}) — this can happen with virtualized Store installs",
        failures.join("; ")
    ))
}

fn normalized_override(path: Option<&str>) -> Option<&str> {
    path.map(str::trim).filter(|path| !path.is_empty())
}

fn lookup_program() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("SystemRoot").map_or_else(
            || PathBuf::from(r"C:\Windows\System32\where.exe"),
            |root| PathBuf::from(root).join("System32").join("where.exe"),
        )
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("which")
    }
}

fn pick_lookup_candidate(stdout: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let has_suffix =
        |line: &&str, suffix: &str| line.to_ascii_lowercase().strip_suffix(suffix).is_some();
    lines
        .iter()
        .find(|line| has_suffix(line, ".exe"))
        .or_else(|| {
            lines
                .iter()
                .find(|line| has_suffix(line, ".cmd") || has_suffix(line, ".bat"))
        })
        .or_else(|| lines.first())
        .map(PathBuf::from)
}

async fn resolve_cli_with(
    runner: &dyn ProcessRunner,
    binary: &str,
    override_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(configured) = normalized_override(override_path) {
        let path = Path::new(configured);
        if !path.is_absolute() {
            return Err(format!(
                "The configured path for '{binary}' must be absolute: {configured}"
            ));
        }
        if !path.is_file() {
            return Err(format!(
                "The configured path for '{binary}' does not exist: {configured}"
            ));
        }
        return Ok(path.to_path_buf());
    }

    let output = runner
        .run(ProcessRequest {
            program: lookup_program(),
            args: vec![binary.to_string()],
            timeout: CLI_PROBE_TIMEOUT,
            what: "executable lookup".to_string(),
        })
        .await?;
    if !output.success {
        return Err(format!("'{binary}' was not found on PATH"));
    }
    pick_lookup_candidate(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("'{binary}' was not found on PATH"))
}

trait FoundryHealthProbe: Send + Sync + 'static {
    fn is_healthy(&self, endpoint: String) -> BackendFuture<'_, bool>;
}

#[derive(Debug, Default)]
struct ReqwestFoundryHealthProbe;

impl FoundryHealthProbe for ReqwestFoundryHealthProbe {
    fn is_healthy(&self, endpoint: String) -> BackendFuture<'_, bool> {
        Box::pin(async move { foundry_service_is_healthy(&endpoint).await })
    }
}

/// Validate a Foundry Local base URL through the current status endpoint.
///
/// Foundry Local 0.10 moved the OpenAI-compatible API from `/openai/*` to
/// `/v1/*` and its health document from `/openai/status` to `/status`.
/// Probe the current route first, then retain the pre-0.10 route as a
/// compatibility fallback. Both responses use the same `endpoints` schema.
pub async fn foundry_service_is_healthy(base: &str) -> bool {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + FOUNDRY_HEALTH_TIMEOUT;
    for path in ["/status", "/openai/status"] {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            break;
        }
        // Note (2026-08-31 audit): both routes are deliberately probed on any
        // non-success or schema mismatch — pinned by
        // `invalid_current_health_schema_falls_back_to_legacy` and
        // `foundry_health_fails_when_current_and_legacy_routes_are_invalid`.
        // The bounded deadline below is the accepted worst-case cost of the
        // pre-0.10 compatibility fallback.
        let Ok(response) = client
            .get(format!("{base}{path}"))
            .timeout(timeout)
            .send()
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if response
            .json::<Value>()
            .await
            .is_ok_and(|body| valid_foundry_status_body(&body))
        {
            return true;
        }
    }
    false
}

type FoundryCliCache = Arc<Mutex<Option<(Instant, Option<PathBuf>)>>>;

fn shared_foundry_cli_cache() -> FoundryCliCache {
    static CACHE: OnceLock<FoundryCliCache> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(Mutex::new(None))).clone()
}

/// Shipping Foundry Local endpoint probe with no UI-framework dependency.
///
/// A healthy configured endpoint wins. Otherwise the adapter resolves the
/// `foundry` CLI, asks the current CLI for `status --output json`, extracts its
/// dynamic base URL, and validates `/status`. The pre-0.10
/// `service status` spelling remains a fallback so existing installations do
/// not regress during the CLI transition, as does the legacy
/// `/openai/status` health route. CLI resolution (including a miss) is cached
/// for 30 seconds, as in 2.5.8; endpoint health is checked on every probe.
#[derive(Clone)]
pub struct FoundryCliEndpointSource {
    runner: Arc<dyn ProcessRunner>,
    health: Arc<dyn FoundryHealthProbe>,
    cli_cache: FoundryCliCache,
}

impl fmt::Debug for FoundryCliEndpointSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FoundryCliEndpointSource")
            .finish_non_exhaustive()
    }
}

impl Default for FoundryCliEndpointSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FoundryCliEndpointSource {
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: Arc::new(TokioProcessRunner),
            health: Arc::new(ReqwestFoundryHealthProbe),
            cli_cache: shared_foundry_cli_cache(),
        }
    }

    #[cfg(test)]
    fn with_dependencies(
        runner: Arc<dyn ProcessRunner>,
        health: Arc<dyn FoundryHealthProbe>,
    ) -> Self {
        Self {
            runner,
            health,
            cli_cache: Arc::new(Mutex::new(None)),
        }
    }

    async fn resolved_cli(&self) -> Option<PathBuf> {
        let cached = self.cli_cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(at, _)| at.elapsed() < PROBE_TTL)
                .map(|(_, path)| path.clone())
        });
        if let Some(path) = cached {
            return path;
        }

        let resolved = match resolve_cli_with(self.runner.as_ref(), "foundry", None).await {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("Foundry CLI resolution failed: {error}");
                None
            }
        };
        if let Ok(mut cache) = self.cli_cache.lock() {
            *cache = Some((Instant::now(), resolved.clone()));
        }
        resolved
    }

    async fn discover_endpoint(&self) -> Option<String> {
        let path = self.resolved_cli().await?;
        for args in [
            vec![
                "status".to_string(),
                "--output".to_string(),
                "json".to_string(),
            ],
            vec!["service".to_string(), "status".to_string()],
        ] {
            let Ok(output) = self
                .runner
                .run(ProcessRequest {
                    program: path.clone(),
                    args,
                    timeout: FOUNDRY_STATUS_TIMEOUT,
                    what: "Foundry Local CLI".to_string(),
                })
                .await
            else {
                continue;
            };
            if let Some(endpoint) = extract_http_base(&String::from_utf8_lossy(&output.stdout))
                .or_else(|| extract_http_base(&String::from_utf8_lossy(&output.stderr)))
            {
                return Some(endpoint);
            }
        }
        None
    }

    async fn probe_inner(&self, configured: Option<String>) -> Option<String> {
        if let Some(endpoint) = configured.as_deref().and_then(normalize_base_url)
            && self.health.is_healthy(endpoint.clone()).await
        {
            return Some(endpoint);
        }
        let endpoint = self.discover_endpoint().await?;
        self.health
            .is_healthy(endpoint.clone())
            .await
            .then_some(endpoint)
    }
}

impl FoundryEndpointSource for FoundryCliEndpointSource {
    fn probe(&self, configured: Option<String>) -> BackendFuture<'_, Option<String>> {
        Box::pin(async move { self.probe_inner(configured).await })
    }
}

/// An npm script shim (`.cmd`/`.bat`) resolves on PATH but cannot receive
/// arguments from a Rust child process; only the native executable works.
#[must_use]
pub fn is_batch_shim(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

/// Shipping Codex/Claude subscription status probe without Tauri or Reactor.
///
/// Configured paths must be absolute existing files; otherwise `where.exe`
/// (Windows) or `which` resolves the executable. A shim-only resolution and a
/// missing credential cache are decided without spawning the CLI; otherwise
/// the CLI is usable only when its status command exits successfully and
/// emits no signed-out marker. Definitive results are cached for 30 seconds,
/// while spawn failures and timeouts are not.
#[derive(Clone)]
pub struct ProcessSubscriptionCliStatusSource {
    runner: Arc<dyn ProcessRunner>,
    credentials: Arc<dyn CredentialStoreProbe>,
    cache: SubscriptionProbeCache,
    /// Per-key async mutexes: concurrent cache misses share one probe instead
    /// of spawning one child process per caller. Entries live for the
    /// process lifetime but are bounded by the key space (2 providers × the
    /// few configured paths a user can set).
    in_flight: InFlightProbes,
}

/// Per-`(cli, path)` single-flight guards for concurrent cache misses.
type InFlightProbes = Arc<Mutex<HashMap<(SubscriptionCli, String), Arc<tokio::sync::Mutex<()>>>>>;

type SubscriptionProbeCache =
    Arc<Mutex<HashMap<(SubscriptionCli, String), (Instant, CliProbeSnapshot)>>>;

fn shared_subscription_probe_cache() -> SubscriptionProbeCache {
    static CACHE: OnceLock<SubscriptionProbeCache> = OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

impl fmt::Debug for ProcessSubscriptionCliStatusSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessSubscriptionCliStatusSource")
            .finish_non_exhaustive()
    }
}

impl Default for ProcessSubscriptionCliStatusSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSubscriptionCliStatusSource {
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: Arc::new(TokioProcessRunner),
            credentials: Arc::new(HostCredentialStoreProbe),
            cache: shared_subscription_probe_cache(),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn with_dependencies(
        runner: Arc<dyn ProcessRunner>,
        credentials: Arc<dyn CredentialStoreProbe>,
    ) -> Self {
        Self {
            runner,
            credentials,
            cache: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Drop one provider's cached status after an external sign-in, sign-out,
    /// installation, or explicit refresh action.
    pub fn invalidate(&self, provider: SubscriptionCli) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|(cached_provider, _), _| *cached_provider != provider);
        }
    }

    async fn probe_uncached(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> (CliProbeSnapshot, bool) {
        let spec = subscription_cli_spec(provider);
        let Ok(path) = resolve_cli_with(
            self.runner.as_ref(),
            spec.binary,
            configured_path.as_deref(),
        )
        .await
        else {
            return (CliProbeSnapshot::default(), true);
        };
        let installed = |usable: bool, obstacle: Option<CliObstacle>| CliProbeSnapshot {
            usable,
            installed: true,
            path: Some(path.display().to_string()),
            obstacle,
        };
        if is_batch_shim(&path) {
            return (installed(false, Some(CliObstacle::BatchShimOnly)), true);
        }
        // No credential cache means never signed in here: decided without
        // spawning. Present or unknown caches still ask the CLI.
        if self.credentials.detect(spec.credential_store) == CredentialStore::Absent {
            return (installed(false, Some(CliObstacle::NoStoredLogin)), true);
        }
        let output = self
            .runner
            .run(ProcessRequest {
                program: path.clone(),
                args: spec.status_args.iter().map(ToString::to_string).collect(),
                timeout: CLI_PROBE_TIMEOUT,
                what: spec.binary.to_string(),
            })
            .await;
        match output {
            Ok(output) => {
                let verdict = parse_status_output(
                    spec,
                    output.success,
                    &String::from_utf8_lossy(&output.stdout),
                    &String::from_utf8_lossy(&output.stderr),
                );
                let snapshot = match verdict {
                    StatusVerdict::SignedIn => installed(true, None),
                    StatusVerdict::SignedOut => installed(false, Some(CliObstacle::SignedOut)),
                    StatusVerdict::Unclear => installed(false, Some(CliObstacle::StatusUnclear)),
                };
                (snapshot, true)
            }
            Err(error) => {
                eprintln!("Bridge probe for {} inconclusive: {error}", spec.binary);
                (installed(false, Some(CliObstacle::StatusUnclear)), false)
            }
        }
    }

    async fn probe_inner(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> CliProbeSnapshot {
        let cache_key = (
            provider,
            configured_path
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_string(),
        );
        if let Some(probe) = self.cached_snapshot(&cache_key) {
            return probe;
        }

        // Single-flight: serialize concurrent misses per key, then re-check
        // the cache so followers adopt the leader's fresh result instead of
        // spawning their own child process.
        let guard = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            in_flight.entry(cache_key.clone()).or_default().clone()
        };
        let _permit = guard.lock().await;
        if let Some(probe) = self.cached_snapshot(&cache_key) {
            return probe;
        }

        let (fresh, conclusive) = self.probe_uncached(provider, configured_path).await;
        if conclusive && let Ok(mut cache) = self.cache.lock() {
            cache.insert(cache_key, (Instant::now(), fresh.clone()));
        }
        fresh
    }

    fn cached_snapshot(&self, cache_key: &(SubscriptionCli, String)) -> Option<CliProbeSnapshot> {
        self.cache.lock().ok().and_then(|cache| {
            cache
                .get(cache_key)
                .filter(|(at, _)| at.elapsed() < PROBE_TTL)
                .map(|(_, probe)| probe.clone())
        })
    }
}

impl SubscriptionCliStatusSource for ProcessSubscriptionCliStatusSource {
    fn probe(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> BackendFuture<'_, CliProbeSnapshot> {
        Box::pin(async move { self.probe_inner(provider, configured_path).await })
    }

    fn invalidate(&self, provider: SubscriptionCli) {
        Self::invalidate(self, provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<Result<ProcessOutput, String>>>,
        requests: Mutex<Vec<ProcessRequest>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<Result<ProcessOutput, String>>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<ProcessRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl ProcessRunner for FakeRunner {
        fn run(&self, request: ProcessRequest) -> BackendFuture<'_, Result<ProcessOutput, String>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(request);
                self.outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fake runner output exhausted")
            })
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            success,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn foundry_http_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for (status, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(2);
                let Some(mut stream) = (loop {
                    match listener.accept() {
                        Ok((stream, _)) => break Some(stream),
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break None,
                    }
                }) else {
                    return;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                if let Some(path) = request_line.split_whitespace().nth(1) {
                    observed.lock().unwrap().push(path.to_string());
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or_default() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let reason = if status == 200 { "OK" } else { "Not Found" };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });
        (base, requests, server)
    }

    struct FakeHealth {
        answers: Mutex<VecDeque<bool>>,
        endpoints: Mutex<Vec<String>>,
        calls: AtomicUsize,
    }

    impl FakeHealth {
        fn new(answers: impl IntoIterator<Item = bool>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                endpoints: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl FoundryHealthProbe for FakeHealth {
        fn is_healthy(&self, endpoint: String) -> BackendFuture<'_, bool> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::Relaxed);
                self.endpoints.lock().unwrap().push(endpoint);
                self.answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fake health answer exhausted")
            })
        }
    }

    #[test]
    fn foundry_url_extraction_matches_shipping_edge_cases() {
        assert_eq!(
            extract_http_base(
                "service is running at http://127.0.0.1:55769/openai/status trailing"
            )
            .as_deref(),
            Some("http://127.0.0.1:55769")
        );
        assert_eq!(
            extract_http_base("endpoint: https://localhost:5273\" ignored").as_deref(),
            Some("https://localhost:5273")
        );
        assert_eq!(extract_http_base("service is not running"), None);
        assert_eq!(extract_http_base("http://"), None);
        // 2.5.8 checks the HTTP pattern first, even when HTTPS occurs earlier.
        assert_eq!(
            extract_http_base("https://first.test/path then http://second.test/path").as_deref(),
            Some("http://second.test")
        );
    }

    #[test]
    fn foundry_health_body_requires_a_string_endpoint() {
        assert!(valid_foundry_status_body(&serde_json::json!({
            "Endpoints": ["http://localhost:5272"],
            "ModelDirPath": "C:/models"
        })));
        assert!(valid_foundry_status_body(&serde_json::json!({
            "endpoints": [42, "http://localhost:5272"]
        })));
        assert!(!valid_foundry_status_body(&serde_json::json!({})));
        assert!(!valid_foundry_status_body(&serde_json::json!({
            "Endpoints": []
        })));
        assert!(!valid_foundry_status_body(&serde_json::json!({
            "Endpoints": [42]
        })));
    }

    #[tokio::test]
    async fn current_foundry_health_route_wins_without_a_legacy_request() {
        let (base, requests, server) =
            foundry_http_server(vec![(200, r#"{"endpoints":["http://127.0.0.1:5272"]}"#)]);

        assert!(foundry_service_is_healthy(&base).await);
        server.join().unwrap();
        assert_eq!(requests.lock().unwrap().as_slice(), &["/status"]);
    }

    #[tokio::test]
    async fn legacy_foundry_health_route_is_used_after_current_404() {
        let (base, requests, server) = foundry_http_server(vec![
            (404, r#"{"error":"missing"}"#),
            (200, r#"{"Endpoints":["http://127.0.0.1:5272/openai"]}"#),
        ]);

        assert!(foundry_service_is_healthy(&base).await);
        server.join().unwrap();
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            &["/status", "/openai/status"]
        );
    }

    #[tokio::test]
    async fn invalid_current_health_schema_falls_back_to_legacy() {
        let (base, requests, server) = foundry_http_server(vec![
            (200, r#"{"state":"ready"}"#),
            (200, r#"{"endpoints":["http://127.0.0.1:5272/openai"]}"#),
        ]);

        assert!(foundry_service_is_healthy(&base).await);
        server.join().unwrap();
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            &["/status", "/openai/status"]
        );
    }

    #[tokio::test]
    async fn foundry_health_fails_when_current_and_legacy_routes_are_invalid() {
        let (base, requests, server) = foundry_http_server(vec![
            (200, r#"{"endpoints":[]}"#),
            (200, r#"{"Endpoints":[42]}"#),
        ]);

        assert!(!foundry_service_is_healthy(&base).await);
        server.join().unwrap();
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            &["/status", "/openai/status"]
        );
    }

    #[tokio::test]
    async fn healthy_configured_foundry_endpoint_wins_without_cli_work() {
        let runner = Arc::new(FakeRunner::default());
        let health = Arc::new(FakeHealth::new([true]));
        let source = FoundryCliEndpointSource::with_dependencies(runner.clone(), health.clone());

        let endpoint = source
            .probe(Some(" https://configured.test/v1/ ".to_string()))
            .await;

        assert_eq!(endpoint.as_deref(), Some("https://configured.test"));
        assert!(runner.requests().is_empty());
        assert_eq!(
            health.endpoints.lock().unwrap().as_slice(),
            &["https://configured.test"]
        );
    }

    #[tokio::test]
    async fn unhealthy_configured_foundry_endpoint_falls_back_to_cli_and_health_check() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            Ok(output(true, "/opt/foundry\n", "")),
            Ok(output(
                false,
                "running at http://127.0.0.1:55769/openai/status",
                "",
            )),
        ]));
        let health = Arc::new(FakeHealth::new([false, true]));
        let source = FoundryCliEndpointSource::with_dependencies(runner.clone(), health.clone());

        let endpoint = source
            .probe(Some("http://configured.test/v1".to_string()))
            .await;

        assert_eq!(endpoint.as_deref(), Some("http://127.0.0.1:55769"));
        assert_eq!(
            health.endpoints.lock().unwrap().as_slice(),
            &["http://configured.test", "http://127.0.0.1:55769"]
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].args, ["foundry"]);
        assert_eq!(requests[0].timeout, CLI_PROBE_TIMEOUT);
        assert_eq!(requests[1].program, PathBuf::from("/opt/foundry"));
        assert_eq!(requests[1].args, ["status", "--output", "json"]);
        assert_eq!(requests[1].timeout, FOUNDRY_STATUS_TIMEOUT);
        // Discovery extracts a URL even when the CLI exits non-zero; some
        // historical Foundry builds did that while the service was starting.
        assert!(!runner.requests()[1].what.is_empty());
    }

    #[tokio::test]
    async fn current_foundry_status_falls_back_to_legacy_service_status() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            Ok(output(true, "/opt/foundry\n", "")),
            Ok(output(false, "", "unknown command: status")),
            Ok(output(
                true,
                "running at http://127.0.0.1:5273/openai/status",
                "",
            )),
        ]));
        let health = Arc::new(FakeHealth::new([true]));
        let source = FoundryCliEndpointSource::with_dependencies(runner.clone(), health.clone());

        let endpoint = source.probe(None).await;

        assert_eq!(endpoint.as_deref(), Some("http://127.0.0.1:5273"));
        assert_eq!(
            health.endpoints.lock().unwrap().as_slice(),
            &["http://127.0.0.1:5273"]
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].args, ["status", "--output", "json"]);
        assert_eq!(requests[2].args, ["service", "status"]);
    }

    #[test]
    fn lookup_prefers_a_native_executable_then_a_script_shim() {
        assert_eq!(
            pick_lookup_candidate("C:\\npm\\codex.cmd\r\nC:\\bin\\codex.EXE\r\n"),
            Some(PathBuf::from("C:\\bin\\codex.EXE"))
        );
        assert_eq!(
            pick_lookup_candidate("C:\\npm\\claude.bat\r\nC:\\npm\\claude.cmd\r\n"),
            Some(PathBuf::from("C:\\npm\\claude.bat"))
        );
        assert_eq!(pick_lookup_candidate(" \n\r\n"), None);
    }

    #[test]
    fn configured_cli_path_normalization_matches_shipping() {
        assert_eq!(normalized_override(None), None);
        assert_eq!(normalized_override(Some("  ")), None);
        assert_eq!(
            normalized_override(Some("  /opt/bin/codex \r\n")),
            Some("/opt/bin/codex")
        );
    }

    struct FakeCredentials(CredentialStore);

    impl CredentialStoreProbe for FakeCredentials {
        fn detect(&self, _kind: crate::credential_store::CredentialStoreKind) -> CredentialStore {
            self.0
        }
    }

    fn source_with(
        runner: Arc<FakeRunner>,
        store: CredentialStore,
    ) -> ProcessSubscriptionCliStatusSource {
        ProcessSubscriptionCliStatusSource::with_dependencies(
            runner,
            Arc::new(FakeCredentials(store)),
        )
    }

    #[tokio::test]
    async fn absent_credential_store_reports_no_stored_login_without_running_the_status_command() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![Ok(output(
            true,
            "/opt/codex\n",
            "",
        ))]));
        let source = source_with(runner.clone(), CredentialStore::Absent);

        let first = source.probe(SubscriptionCli::Codex, None).await;
        let cached = source.probe(SubscriptionCli::Codex, None).await;

        assert_eq!(
            first,
            CliProbeSnapshot {
                usable: false,
                installed: true,
                path: Some("/opt/codex".to_string()),
                obstacle: Some(CliObstacle::NoStoredLogin),
            }
        );
        assert_eq!(cached, first, "decisive, so cached");
        let requests = runner.requests();
        assert_eq!(requests.len(), 1, "only the executable lookup ran");
        assert_eq!(requests[0].args, ["codex"]);
    }

    #[tokio::test]
    async fn present_credential_store_still_runs_the_status_command() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            Ok(output(true, "/opt/codex\n", "")),
            Ok(output(true, "Not logged in", "")),
        ]));
        let source = source_with(runner.clone(), CredentialStore::Present);

        let probe = source.probe(SubscriptionCli::Codex, None).await;

        assert!(probe.installed && !probe.usable);
        assert_eq!(probe.obstacle, Some(CliObstacle::SignedOut));
        assert_eq!(runner.requests().len(), 2);
        assert_eq!(runner.requests()[1].args, ["login", "status"]);
    }

    #[tokio::test]
    async fn batch_shim_only_resolution_is_installed_but_unusable_without_a_spawn() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![Ok(output(
            true,
            "C:\\Users\\mike\\AppData\\Roaming\\npm\\codex.cmd\n",
            "",
        ))]));
        let source = source_with(runner.clone(), CredentialStore::Present);

        let probe = source.probe(SubscriptionCli::Codex, None).await;

        assert!(probe.installed && !probe.usable);
        assert_eq!(probe.obstacle, Some(CliObstacle::BatchShimOnly));
        assert_eq!(runner.requests().len(), 1);
        assert!(is_batch_shim(Path::new("x.CMD")) && is_batch_shim(Path::new("x.bat")));
        assert!(!is_batch_shim(Path::new("codex.exe")) && !is_batch_shim(Path::new("codex")));
    }

    #[tokio::test]
    async fn subscription_probe_uses_shipping_command_and_caches_definitive_status() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            Ok(output(true, "/opt/codex\n", "")),
            Ok(output(true, "Logged in using ChatGPT", "")),
        ]));
        let source = source_with(runner.clone(), CredentialStore::Unknown);

        let first = source.probe(SubscriptionCli::Codex, None).await;
        let cached = source.probe(SubscriptionCli::Codex, None).await;

        assert_eq!(first, cached);
        assert_eq!(
            first,
            CliProbeSnapshot {
                usable: true,
                installed: true,
                path: Some("/opt/codex".to_string()),
                obstacle: None,
            }
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].args, ["codex"]);
        assert_eq!(requests[1].program, PathBuf::from("/opt/codex"));
        assert_eq!(requests[1].args, ["login", "status"]);
        assert_eq!(requests[1].timeout, CLI_PROBE_TIMEOUT);
    }

    #[tokio::test]
    async fn configured_absolute_cli_path_bypasses_path_lookup() {
        let executable = std::env::current_exe().unwrap();
        let runner = Arc::new(FakeRunner::with_outputs(vec![Ok(output(
            true,
            "Logged in using ChatGPT",
            "",
        ))]));
        let source = source_with(runner.clone(), CredentialStore::Unknown);

        let probe = source
            .probe(
                SubscriptionCli::Codex,
                Some(format!("  {}  ", executable.display())),
            )
            .await;

        assert!(probe.installed);
        assert!(probe.usable);
        assert_eq!(
            probe.path.as_deref(),
            Some(executable.to_string_lossy().as_ref())
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program, executable);
        assert_eq!(requests[0].args, ["login", "status"]);
    }

    #[tokio::test]
    async fn inconclusive_subscription_status_is_not_cached_as_signed_out() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            Ok(output(true, "/opt/claude\n", "")),
            Err("claude did not answer within 10 seconds".to_string()),
            Ok(output(true, "/opt/claude\n", "")),
            Ok(output(true, "Logged in", "")),
        ]));
        let source = source_with(runner.clone(), CredentialStore::Unknown);

        let inconclusive = source.probe(SubscriptionCli::ClaudeCode, None).await;
        let recovered = source.probe(SubscriptionCli::ClaudeCode, None).await;

        assert_eq!(inconclusive.path.as_deref(), Some("/opt/claude"));
        assert!(inconclusive.installed);
        assert!(!inconclusive.usable);
        assert_eq!(inconclusive.obstacle, Some(CliObstacle::StatusUnclear));
        assert!(recovered.usable);
        assert_eq!(runner.requests().len(), 4);
        assert_eq!(runner.requests()[1].args, ["auth", "status"]);
    }

    #[tokio::test]
    async fn definitive_missing_cli_result_is_cached() {
        let runner = Arc::new(FakeRunner::with_outputs(vec![Ok(output(false, "", ""))]));
        let source = source_with(runner.clone(), CredentialStore::Unknown);

        let first = source.probe(SubscriptionCli::Codex, None).await;
        let cached = source.probe(SubscriptionCli::Codex, None).await;

        assert_eq!(first, CliProbeSnapshot::default());
        assert_eq!(cached, first);
        assert_eq!(runner.requests().len(), 1);
    }

    #[test]
    fn subscription_override_keys_match_the_shipping_closed_list() {
        assert_eq!(
            SUBSCRIPTION_OVERRIDE_ENV_VARS,
            [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CODEX_API_KEY",
                "OPENAI_API_KEY",
            ]
        );
    }
}
