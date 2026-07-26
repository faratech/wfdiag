//! Subscription CLI bridge: run AI requests through a vendor's own installed
//! CLI, which owns authentication entirely.
//!
//! This app implements NO OAuth, stores NO tokens and never touches the CLI's
//! credential files. It only: detects the installed CLI, reports its sign-in
//! state, offers a Sign in action that spawns the CLI's own login command
//! (the CLI opens the browser itself), and pipes prompts through headless
//! execution. Usage bills to the user's subscription, not an API key.
//!
//! Two bridges exist: the OpenAI Codex CLI (`codex.rs` — OpenAI explicitly
//! endorses third-party harnesses on ChatGPT subscriptions) and the Anthropic
//! Claude Code CLI (`claude_cli.rs` — same drive-the-genuine-CLI pattern that
//! Microsoft's Intelligent Terminal and Zed ship via ACP; what Anthropic's
//! Feb 2026 terms ban is extracting subscription OAuth tokens, which this
//! design never touches).
//!
//! Safety: nothing user-controlled ever goes on a command line. Prompts are
//! delivered via stdin (also sidestepping the ~32k char command-line cap) and
//! model names are allowlist-sanitized.

use crate::ai_service::AIProvider;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

/// How long a cached availability probe stays fresh.
const PROBE_TTL: Duration = Duration::from_secs(30);
/// Executable resolution / status probes are quick CLI calls — but a
/// cold Node-based CLI can take several seconds to boot on Windows.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The login command waits for the user to finish a browser flow.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
const SIGN_OUT_TIMEOUT: Duration = Duration::from_secs(15);
/// Model discovery must remain a lightweight Settings operation.
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MODEL_LIST_MAX_PAGES: usize = 50;
/// Bound stdout/stderr retained from a local helper process.
const MODEL_LIST_STDOUT_LIMIT: u64 = 2 * 1024 * 1024;
const MODEL_LIST_STDERR_LIMIT: u64 = 32 * 1024;

/// Environment credentials that would override a CLI's subscription login.
/// Keep this closed list shared by every bridge invocation, including probes.
pub(super) const SUBSCRIPTION_OVERRIDE_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
];

/// Static description of one bridged CLI.
struct BridgeSpec {
    /// Bare executable name looked up on PATH (`where.exe` / `which`)
    binary: &'static str,
    login_args: &'static [&'static str],
    logout_args: &'static [&'static str],
    /// Command whose zero exit code means "signed in"
    status_args: &'static [&'static str],
    /// Lowercase output substrings that mean "signed out" even when the
    /// status command exits 0 — some CLIs report status without a failing
    /// exit code.
    signed_out_markers: &'static [&'static str],
}

const CODEX_SPEC: BridgeSpec = BridgeSpec {
    binary: "codex",
    login_args: &["login"],
    logout_args: &["logout"],
    status_args: &["login", "status"],
    signed_out_markers: &["not logged in"],
};

const CLAUDE_SPEC: BridgeSpec = BridgeSpec {
    binary: "claude",
    login_args: &["auth", "login"],
    logout_args: &["auth", "logout"],
    status_args: &["auth", "status"],
    signed_out_markers: &["not logged in", "please run /login"],
};

fn spec_for(provider: AIProvider) -> Option<&'static BridgeSpec> {
    match provider {
        AIProvider::CodexCli => Some(&CODEX_SPEC),
        AIProvider::ClaudeCode => Some(&CLAUDE_SPEC),
        _ => None,
    }
}

/// Settings override for the CLI path, if the user configured one.
fn configured_cli_path(provider: AIProvider) -> Option<String> {
    let settings = crate::commands::settings::read_settings_from_disk()?;
    match provider {
        AIProvider::CodexCli => settings.codex_cli_path,
        AIProvider::ClaudeCode => settings.claude_cli_path,
        _ => None,
    }
    .map(|p| p.trim().to_string())
    .filter(|p| !p.is_empty())
}

/// Result of probing a bridged CLI: where it is (None = not installed) and
/// whether its own credentials say we're signed in.
#[derive(Debug, Clone, Default)]
pub struct BridgeProbe {
    pub path: Option<PathBuf>,
    pub authed: bool,
}

impl BridgeProbe {
    pub fn usable(&self) -> bool {
        self.path.is_some() && self.authed
    }
}

fn probe_cache() -> &'static Mutex<HashMap<&'static str, (Instant, BridgeProbe)>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, (Instant, BridgeProbe)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Probe a bridged CLI with a short-lived cache so status refreshes and Auto
/// routing don't spawn processes on every call.
pub async fn probe(provider: AIProvider) -> BridgeProbe {
    let Some(spec) = spec_for(provider) else {
        return BridgeProbe::default();
    };
    if let Ok(cache) = probe_cache().lock()
        && let Some((at, probe)) = cache.get(spec.binary)
        && at.elapsed() < PROBE_TTL
    {
        return probe.clone();
    }
    let fresh = probe_uncached(provider, spec).await;
    if let Ok(mut cache) = probe_cache().lock() {
        cache.insert(spec.binary, (Instant::now(), fresh.clone()));
    }
    fresh
}

/// Drop the cached probe (after sign-in/out, or an explicit refresh).
pub fn invalidate(provider: AIProvider) {
    if let Some(spec) = spec_for(provider)
        && let Ok(mut cache) = probe_cache().lock()
    {
        cache.remove(spec.binary);
    }
}

async fn probe_uncached(provider: AIProvider, spec: &'static BridgeSpec) -> BridgeProbe {
    let path = match resolve_cli(spec.binary, configured_cli_path(provider).as_deref()).await {
        Ok(path) => path,
        Err(_) => return BridgeProbe::default(),
    };
    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(spec.status_args);
    let authed = match run_headless(cmd, None, PROBE_TIMEOUT, spec.binary).await {
        Ok(output) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            is_signed_in(spec, output.status.success(), &text)
        }
        Err(_) => false,
    };
    BridgeProbe {
        path: Some(path),
        authed,
    }
}

/// Signed in = the status command succeeded AND its output doesn't say
/// otherwise (pure for testability).
fn is_signed_in(spec: &BridgeSpec, exit_ok: bool, output: &str) -> bool {
    let text = output.to_lowercase();
    exit_ok
        && !spec
            .signed_out_markers
            .iter()
            .any(|marker| text.contains(marker))
}

// ============================================================================
// Executable resolution
// ============================================================================

/// Resolve a bridged CLI to an absolute path. A configured override wins and
/// must be an absolute path to an existing file; otherwise the executable is
/// looked up on PATH via `where.exe` (Windows) or `which`. npm installs ship
/// `.cmd` shims that `Command::new("name")` cannot spawn by bare name, so the
/// lookup happens here and the returned absolute path is what gets spawned.
pub async fn resolve_cli(
    binary: &'static str,
    override_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(configured) = override_path {
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

    let mut cmd = lookup_command();
    cmd.arg(binary);
    let output = run_headless(cmd, None, PROBE_TIMEOUT, "executable lookup").await?;
    if !output.status.success() {
        return Err(format!("'{binary}' was not found on PATH"));
    }
    pick_lookup_candidate(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("'{binary}' was not found on PATH"))
}

/// PATH lookup command: `where.exe` by absolute path (it lives in System32;
/// an absolute path avoids any PATH games), `which` elsewhere.
fn lookup_command() -> tokio::process::Command {
    #[cfg(windows)]
    {
        let system32 = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32"))
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32"));
        tokio::process::Command::new(system32.join("where.exe"))
    }
    #[cfg(not(windows))]
    {
        tokio::process::Command::new("which")
    }
}

/// Pick the best line from `where`/`which` output: a real `.exe` beats an npm
/// `.cmd`/`.bat` shim (fewer layers, no cmd.exe involvement), else the first
/// match. Returns None when the output has no usable line.
fn pick_lookup_candidate(stdout: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let lower = |l: &&str| l.to_ascii_lowercase();
    lines
        .iter()
        .find(|l| lower(l).ends_with(".exe"))
        .or_else(|| {
            lines
                .iter()
                .find(|l| lower(l).ends_with(".cmd") || lower(l).ends_with(".bat"))
        })
        .or_else(|| lines.first())
        .map(PathBuf::from)
}

// ============================================================================
// Shared runner
// ============================================================================

/// Model names go on the command line, so they are allowlist-sanitized:
/// letters, digits and `. _ : / - [ ]` only. Brackets are required for
/// provider-returned Claude selectors such as `opus[1m]`; no shell
/// metacharacters or whitespace are accepted.
pub fn sanitize_model(model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Model name is empty".to_string());
    }
    if model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-' | '[' | ']'))
    {
        Ok(model.to_string())
    } else {
        Err(format!(
            "Invalid model name '{model}': only letters, digits and . _ : / - [ ] are allowed"
        ))
    }
}

/// Neutral, empty working directory for bridged CLI runs so an agentic CLI
/// with a read-only sandbox still has nothing interesting to read.
pub fn bridge_workdir() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "Could not find config directory".to_string())?;
    let dir = base
        .join("com.windowsforum.diagnostics")
        .join("cli-bridge-cwd");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {}", dir.display(), e))?;
    Ok(dir)
}

/// Provider-neutral model metadata reported by a subscription CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModel {
    pub id: String,
    pub label: Option<String>,
    pub description: Option<String>,
}

/// A live CLI model catalog. `default_model` is the CLI's current default,
/// not an application-maintained guess.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeModelCatalog {
    pub models: Vec<BridgeModel>,
    pub default_model: Option<String>,
}

/// Ask the installed Codex CLI's app-server for its live, account-aware model
/// catalog. The protocol is JSON-RPC over JSONL:
/// `initialize` followed by every page of `model/list`.
pub async fn list_codex_models(override_path: Option<&str>) -> Result<BridgeModelCatalog, String> {
    let path = resolve_cli(CODEX_SPEC.binary, override_path).await?;
    tokio::time::timeout(MODEL_LIST_TIMEOUT, list_codex_models_inner(path))
        .await
        .map_err(|_| {
            format!(
                "Codex model discovery did not finish within {} seconds",
                MODEL_LIST_TIMEOUT.as_secs()
            )
        })?
}

async fn list_codex_models_inner(path: PathBuf) -> Result<BridgeModelCatalog, String> {
    let workdir = bridge_workdir()?;
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(["app-server", "--stdio"]);
    cmd.current_dir(workdir);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    for var in SUBSCRIPTION_OVERRIDE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|error| format!("Could not start Codex app-server: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdout".to_string())?
        .take(MODEL_LIST_STDOUT_LIMIT);
    let mut stdout = tokio::io::BufReader::new(stdout);
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr
                .take(MODEL_LIST_STDERR_LIMIT)
                .read_to_end(&mut bytes)
                .await;
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });

    let result = async {
        send_json_line(
            &mut stdin,
            &json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":{
                    "clientInfo":{
                        "name":"wfdiag",
                        "version":env!("CARGO_PKG_VERSION")
                    },
                    "capabilities":{"experimentalApi":true}
                }
            }),
        )
        .await?;
        let _ = read_jsonrpc_result(&mut stdout, 1).await?;

        let mut catalog = BridgeModelCatalog::default();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for page_index in 0..MODEL_LIST_MAX_PAGES {
            let request_id = page_index as u64 + 2;
            send_json_line(
                &mut stdin,
                &json!({
                    "jsonrpc":"2.0",
                    "id":request_id,
                    "method":"model/list",
                    "params":{
                        "cursor":cursor,
                        "limit":100,
                        "includeHidden":false
                    }
                }),
            )
            .await?;
            let response = read_jsonrpc_result(&mut stdout, request_id).await?;
            let page = parse_codex_model_page(&response)?;
            if catalog.default_model.is_none() {
                catalog.default_model = page.default_model;
            }
            catalog.models.extend(page.models);

            let Some(next) = page.next_cursor else {
                catalog.models = stable_dedupe_bridge_models(catalog.models);
                return Ok(catalog);
            };
            if !seen_cursors.insert(next.clone()) {
                return Err("Codex app-server repeated a model-list cursor".to_string());
            }
            cursor = Some(next);
        }

        Err("Codex app-server model list exceeded the pagination limit".to_string())
    }
    .await;

    // The app-server exists only for this discovery call. It has no shutdown
    // RPC, so terminate it after receiving the final page.
    drop(stdin);
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    let stderr = match stderr_task {
        Some(task) => tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        None => String::new(),
    };

    result.map_err(|error| {
        let detail = tail(stderr.trim(), 300);
        if detail.is_empty() {
            error
        } else {
            format!("{error} — {detail}")
        }
    })
}

async fn send_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("Could not encode CLI model request: {error}"))?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .map_err(|error| format!("Could not send CLI model request: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("Could not flush CLI model request: {error}"))
}

async fn read_jsonrpc_result<R>(reader: &mut R, wanted_id: u64) -> Result<Value, String>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let mut line = String::new();
        let count = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("Could not read CLI model response: {error}"))?;
        if count == 0 {
            return Err(format!(
                "CLI model server closed before response {wanted_id}"
            ));
        }
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|error| format!("CLI model server returned malformed JSON: {error}"))?;
        // Notifications have no id and are deliberately ignored.
        if value.get("id").and_then(Value::as_u64) != Some(wanted_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown JSON-RPC error");
            return Err(format!("CLI model server rejected the request: {detail}"));
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| "CLI model server returned no result".to_string());
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CodexModelPage {
    models: Vec<BridgeModel>,
    default_model: Option<String>,
    next_cursor: Option<String>,
}

fn parse_codex_model_page(result: &Value) -> Result<CodexModelPage, String> {
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex model list returned no data array".to_string())?;
    let mut page = CodexModelPage {
        next_cursor: result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string),
        ..CodexModelPage::default()
    };
    for raw in data {
        let Some(id) = raw
            .get("model")
            .or_else(|| raw.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if raw.get("isDefault").and_then(Value::as_bool) == Some(true)
            && page.default_model.is_none()
        {
            page.default_model = Some(id.clone());
        }
        page.models.push(BridgeModel {
            label: optional_distinct(raw.get("displayName").and_then(Value::as_str), &id),
            description: non_empty_owned(raw.get("description").and_then(Value::as_str)),
            id,
        });
    }
    Ok(page)
}

fn optional_distinct(value: Option<&str>, id: &str) -> Option<String> {
    non_empty_owned(value).filter(|value| value != id)
}

fn non_empty_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn stable_dedupe_bridge_models(models: Vec<BridgeModel>) -> Vec<BridgeModel> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
}

/// Run a prepared command hidden and bounded: no console window, optional
/// stdin payload, captured output, hard timeout, and the child is killed if
/// the future is dropped (chat cancellation drops the provider future).
pub async fn run_headless(
    mut cmd: tokio::process::Command,
    stdin_payload: Option<&str>,
    timeout: Duration,
    what: &str,
) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Bridge runs must bill to the CLI's own signed-in subscription. In
    // headless mode these env vars silently override the stored login ("the
    // key is always used when present"), turning subscription runs into API
    // billing — or failing outright on a stale key.
    for var in SUBSCRIPTION_OVERRIDE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.stdin(if stdin_payload.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {what}: {e}"))?;
    if let Some(payload) = stdin_payload {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("Could not open stdin for {what}"))?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| format!("Could not send input to {what}: {e}"))?;
        // Dropping the handle closes the pipe so the CLI sees end-of-input
        drop(stdin);
    }
    tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{what} did not answer within {} seconds", timeout.as_secs()))?
        .map_err(|e| format!("{what} failed to run: {e}"))
}

// ============================================================================
// Tauri commands (sign-in state is owned by the CLI; we only drive it)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub installed: bool,
    pub signed_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl From<BridgeProbe> for BridgeStatus {
    fn from(probe: BridgeProbe) -> Self {
        Self {
            installed: probe.path.is_some(),
            signed_in: probe.authed,
            path: probe.path.map(|p| p.display().to_string()),
        }
    }
}

fn parse_bridge_provider(provider: &str) -> Result<AIProvider, String> {
    match provider.to_lowercase().as_str() {
        "codex_cli" | "codexcli" | "codex" => Ok(AIProvider::CodexCli),
        "claude_code" | "claudecode" | "claude" => Ok(AIProvider::ClaudeCode),
        other => Err(format!("Unknown bridge provider: {other}")),
    }
}

fn sign_in_tokens() -> &'static Mutex<HashMap<&'static str, CancellationToken>> {
    static TOKENS: OnceLock<Mutex<HashMap<&'static str, CancellationToken>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
pub async fn ai_bridge_status(
    provider: String,
    refresh: Option<bool>,
) -> Result<BridgeStatus, String> {
    let provider = parse_bridge_provider(&provider)?;
    if refresh.unwrap_or(false) {
        invalidate(provider);
    }
    Ok(probe(provider).await.into())
}

/// Spawn the CLI's own login command. The CLI opens the browser and stores
/// its credentials itself; we only wait for it to finish. One sign-in per
/// provider at a time; cancellable via `ai_bridge_sign_in_cancel`.
#[tauri::command]
pub async fn ai_bridge_sign_in(provider: String) -> Result<BridgeStatus, String> {
    let provider_id = parse_bridge_provider(&provider)?;
    let spec = spec_for(provider_id).ok_or_else(|| format!("No bridge spec for {provider}"))?;
    let path = resolve_cli(spec.binary, configured_cli_path(provider_id).as_deref()).await?;

    let token = CancellationToken::new();
    {
        let mut tokens = sign_in_tokens()
            .lock()
            .map_err(|_| "Sign-in state unavailable".to_string())?;
        if tokens.contains_key(spec.binary) {
            return Err("A sign-in is already in progress".to_string());
        }
        tokens.insert(spec.binary, token.clone());
    }

    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(spec.login_args);
    let result = tokio::select! {
        _ = token.cancelled() => Err("Sign-in cancelled".to_string()),
        run = run_headless(cmd, None, SIGN_IN_TIMEOUT, "sign-in") => match run {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "Sign-in failed: {}",
                tail(String::from_utf8_lossy(&output.stderr).trim(), 400)
            )),
            Err(e) => Err(e),
        },
    };

    if let Ok(mut tokens) = sign_in_tokens().lock() {
        tokens.remove(spec.binary);
    }
    invalidate(provider_id);
    result?;
    Ok(probe(provider_id).await.into())
}

#[tauri::command]
pub async fn ai_bridge_sign_in_cancel(provider: String) -> Result<(), String> {
    let provider_id = parse_bridge_provider(&provider)?;
    let spec = spec_for(provider_id).ok_or_else(|| format!("No bridge spec for {provider}"))?;
    if let Ok(tokens) = sign_in_tokens().lock()
        && let Some(token) = tokens.get(spec.binary)
    {
        token.cancel();
    }
    Ok(())
}

/// Run the CLI's own logout command (it removes its stored credentials).
#[tauri::command]
pub async fn ai_bridge_sign_out(provider: String) -> Result<BridgeStatus, String> {
    let provider_id = parse_bridge_provider(&provider)?;
    let spec = spec_for(provider_id).ok_or_else(|| format!("No bridge spec for {provider}"))?;
    let path = resolve_cli(spec.binary, configured_cli_path(provider_id).as_deref()).await?;
    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(spec.logout_args);
    // `logout` on an already signed-out CLI may exit non-zero; either way the
    // fresh probe below reports the truth, so the exit code is not an error.
    let _ = run_headless(cmd, None, SIGN_OUT_TIMEOUT, "sign-out").await?;
    invalidate(provider_id);
    Ok(probe(provider_id).await.into())
}

/// Last `max` characters of a string (char-safe), for error-message tails.
pub(super) fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        text.chars().skip(count - max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_prefers_exe_over_cmd_shim() {
        let out = "C:\\Users\\x\\AppData\\Roaming\\npm\\codex.cmd\r\nC:\\Tools\\codex.exe\r\n";
        assert_eq!(
            pick_lookup_candidate(out),
            Some(PathBuf::from("C:\\Tools\\codex.exe"))
        );
    }

    #[test]
    fn lookup_falls_back_to_cmd_then_first_line() {
        let cmd_only = "C:\\Users\\x\\AppData\\Roaming\\npm\\codex.cmd\r\n";
        assert_eq!(
            pick_lookup_candidate(cmd_only),
            Some(PathBuf::from(
                "C:\\Users\\x\\AppData\\Roaming\\npm\\codex.cmd"
            ))
        );
        let plain = "/usr/local/bin/codex\n";
        assert_eq!(
            pick_lookup_candidate(plain),
            Some(PathBuf::from("/usr/local/bin/codex"))
        );
        assert_eq!(pick_lookup_candidate("  \n"), None);
    }

    #[test]
    fn model_sanitizer_accepts_published_ids_and_rejects_injection() {
        for ok in [
            "gpt-5.5-codex",
            "o4-mini",
            "org/model:tag",
            "a_b.c-d",
            "opus[1m]",
            "claude-fable-5[1m]",
        ] {
            assert_eq!(sanitize_model(ok).unwrap(), ok);
        }
        for bad in [
            "",
            "  ",
            "model name",
            "opus[1m];rm",
            "claude-fable-5[1m] && whoami",
            "m\"x",
            "m&y",
            "m|z",
            "m$(whoami)",
        ] {
            assert!(sanitize_model(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn signed_out_markers_override_a_zero_exit_code() {
        assert!(is_signed_in(
            &CLAUDE_SPEC,
            true,
            "Logged in as: mike@example.com"
        ));
        assert!(!is_signed_in(
            &CLAUDE_SPEC,
            true,
            "Not logged in · Please run /login"
        ));
        assert!(!is_signed_in(
            &CLAUDE_SPEC,
            false,
            "Logged in as: mike@example.com"
        ));
        assert!(!is_signed_in(&CODEX_SPEC, true, "Not logged in"));
    }

    #[test]
    fn tail_is_char_safe() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
        // Multi-byte chars at the cut point must not panic
        assert_eq!(tail("héllo wörld", 5), "wörld");
    }

    #[test]
    fn subscription_override_keys_are_all_scrubbed() {
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CODEX_API_KEY",
            "OPENAI_API_KEY",
        ] {
            assert!(
                SUBSCRIPTION_OVERRIDE_ENV_VARS.contains(&key),
                "bridge forgot to scrub {key}"
            );
        }
    }

    #[test]
    fn codex_model_page_uses_wire_model_and_provider_metadata() {
        let page = parse_codex_model_page(&json!({
            "data":[
                {
                    "id":"picker-sol",
                    "model":"gpt-5.6-sol",
                    "displayName":"GPT-5.6-Sol",
                    "description":"Latest frontier agentic coding model.",
                    "isDefault":true
                },
                {
                    "id":"picker-terra",
                    "model":"gpt-5.6-terra",
                    "displayName":"GPT-5.6-Terra",
                    "description":"Balanced model.",
                    "isDefault":false
                }
            ],
            "nextCursor":"page-2"
        }))
        .unwrap();
        assert_eq!(page.default_model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
        assert_eq!(
            page.models[0],
            BridgeModel {
                id: "gpt-5.6-sol".into(),
                label: Some("GPT-5.6-Sol".into()),
                description: Some("Latest frontier agentic coding model.".into()),
            }
        );
        assert_eq!(page.models[1].id, "gpt-5.6-terra");
    }

    #[test]
    fn codex_model_page_tolerates_new_fields_and_skips_invalid_rows() {
        let page = parse_codex_model_page(&json!({
            "data":[
                {"id":"gpt-future","displayName":"GPT Future","isDefault":false,"newField":42},
                {"displayName":"Missing id"}
            ],
            "nextCursor":null,
            "futureTopLevelField":true
        }))
        .unwrap();
        assert_eq!(page.models.len(), 1);
        assert_eq!(page.models[0].id, "gpt-future");
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn bridge_model_dedupe_preserves_first_seen_order_and_metadata() {
        let models = vec![
            BridgeModel {
                id: "gpt-5.6-sol".into(),
                label: Some("First".into()),
                description: None,
            },
            BridgeModel {
                id: "gpt-5.6-terra".into(),
                label: None,
                description: None,
            },
            BridgeModel {
                id: "gpt-5.6-sol".into(),
                label: Some("Duplicate".into()),
                description: Some("Later".into()),
            },
        ];
        let deduped = stable_dedupe_bridge_models(models);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].id, "gpt-5.6-sol");
        assert_eq!(deduped[0].label.as_deref(), Some("First"));
        assert_eq!(deduped[1].id, "gpt-5.6-terra");
    }
}
