//! Desktop-framework-neutral process runner for subscription CLI transports.
//!
//! Authentication remains entirely inside the vendor CLI. Every request is
//! isolated in an empty working directory, scrubs API-key overrides, carries
//! prompts over stdin, and is bounded so dropping the provider future kills
//! the child process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Byte caps on what a bridge child's pipes may add to the heap (#204).
///
/// `wait_with_output` grew an unbounded `Vec` per pipe, so a vendor CLI stuck
/// in a print loop could exhaust memory before the timeout fired. A completed
/// Codex/Claude answer is far below the stdout cap, and diagnostics only ever
/// use the tail of stderr, so the caps are invisible in normal operation.
pub const HEADLESS_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
pub const HEADLESS_STDERR_LIMIT: usize = 256 * 1024;
#[cfg(windows)]
const WORKDIR_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const SUBSCRIPTION_OVERRIDE_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
];

static VALIDATED_WORKDIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModel {
    pub id: String,
    pub label: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeModelCatalog {
    pub models: Vec<BridgeModel>,
    pub default_model: Option<String>,
}

pub fn sanitize_model(model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Model name is empty".to_string());
    }
    if model.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | ':' | '/' | '-' | '[' | ']')
    }) {
        Ok(model.to_string())
    } else {
        Err(format!(
            "Invalid model name '{model}': only letters, digits and . _ : / - [ ] are allowed"
        ))
    }
}

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
    let system32 = std::env::var_os("SystemRoot").map_or_else(
        || PathBuf::from(r"C:\Windows\System32"),
        |root| PathBuf::from(root).join("System32"),
    );
    let mut command = tokio::process::Command::new(system32.join("cmd.exe"));
    command.args(["/d", "/c", "cd"]);
    command.current_dir(directory);
    command.creation_flags(0x0800_0000);
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

pub async fn bridge_workdir() -> Result<PathBuf, String> {
    if let Some(directory) = VALIDATED_WORKDIR.get() {
        // Best-effort recreate (the user may have deleted the directory after
        // validation); a real failure surfaces from the child spawn itself.
        // This is the cache-hit fast path (every bridged call once warm), so
        // it must not block the async runtime thread.
        let _ = tokio::fs::create_dir_all(directory).await;
        return Ok(directory.clone());
    }

    let mut failures = Vec::new();
    for candidate in workdir_candidates() {
        if let Err(error) = tokio::fs::create_dir_all(&candidate).await {
            failures.push(format!("{}: {error}", candidate.display()));
            continue;
        }
        #[cfg(windows)]
        if !windows_cwd_is_spawnable(&candidate).await {
            failures.push(format!(
                "{}: child processes cannot start there",
                candidate.display()
            ));
            continue;
        }
        // Racing callers walk the same deterministic candidate list, so
        // whichever `set` wins resolves to the same directory.
        let _ = VALIDATED_WORKDIR.set(candidate.clone());
        return Ok(candidate);
    }
    Err(format!(
        "No usable working directory for CLI bridge runs ({}) — this can happen with virtualized Store installs",
        failures.join("; ")
    ))
}

pub async fn resolve_cli(
    binary: &'static str,
    override_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(configured) = override_path.map(str::trim).filter(|path| !path.is_empty()) {
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

    let mut command = lookup_command();
    command.arg(binary);
    let output = run_headless(command, None, PROBE_TIMEOUT, "executable lookup").await?;
    if !output.status.success() {
        return Err(format!("'{binary}' was not found on PATH"));
    }
    pick_lookup_candidate(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("'{binary}' was not found on PATH"))
}

fn lookup_command() -> tokio::process::Command {
    #[cfg(windows)]
    {
        let system32 = std::env::var_os("SystemRoot").map_or_else(
            || PathBuf::from(r"C:\Windows\System32"),
            |root| PathBuf::from(root).join("System32"),
        );
        tokio::process::Command::new(system32.join("where.exe"))
    }
    #[cfg(not(windows))]
    {
        tokio::process::Command::new("which")
    }
}

pub(super) fn pick_lookup_candidate(stdout: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let has_extension = |line: &&str, expected: &str| {
        Path::new(line)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
    };
    lines
        .iter()
        .find(|line| has_extension(line, "exe"))
        .or_else(|| {
            lines
                .iter()
                .find(|line| has_extension(line, "cmd") || has_extension(line, "bat"))
        })
        .or_else(|| lines.first())
        .map(PathBuf::from)
}

/// Read `reader` to end-of-stream, retaining at most `limit` bytes.
///
/// Draining always continues to EOF so the child never blocks on a full pipe;
/// only the retained buffer is bounded. This mirrors (and is shared with) the
/// installer runner in `subscription_install.rs` (#204).
pub(super) async fn drain_bounded<R: AsyncRead + Unpin>(
    reader: Option<R>,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    // Heap buffer, not a stack array: this future is held across `.await` by
    // every bridge call, and an inline 8 KiB array would bloat all of them.
    let mut buffer = vec![0_u8; 8 * 1024];
    let Some(mut reader) = reader else {
        return Ok(retained);
    };
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(retained);
        }
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
}

pub async fn run_headless(
    command: tokio::process::Command,
    stdin_payload: Option<&str>,
    timeout: Duration,
    what: &str,
) -> Result<std::process::Output, String> {
    run_headless_bounded(
        command,
        stdin_payload,
        timeout,
        what,
        HEADLESS_STDOUT_LIMIT,
        HEADLESS_STDERR_LIMIT,
    )
    .await
}

/// [`run_headless`] with explicit output caps, so the bound itself is testable.
pub async fn run_headless_bounded(
    mut command: tokio::process::Command,
    stdin_payload: Option<&str>,
    timeout: Duration,
    what: &str,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    {
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .to_ascii_lowercase();
        // `program` is already ASCII-lowercased above, so the case-sensitive
        // suffix check is exactly what we want here (clippy false positive).
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        let is_batch_shim = program.ends_with(".cmd") || program.ends_with(".bat");
        if is_batch_shim {
            // Rust >= 1.77 refuses to spawn batch files with arguments, so a
            // where.exe fallback that only found an npm shim would otherwise
            // fail with an opaque spawn error.
            return Err(format!(
                "{what} resolved to the batch shim '{program}', which cannot receive arguments. \
Install the native executable (for example via the official installer) and retry."
            ));
        }
    }

    #[cfg(windows)]
    command.creation_flags(0x0800_0000);

    #[cfg(windows)]
    if let Ok(directory) = bridge_workdir().await {
        command.current_dir(directory);
    }
    for variable in SUBSCRIPTION_OVERRIDE_ENV_VARS {
        command.env_remove(variable);
    }
    command
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start {what}: {error}"))?;
    let mut stdin = match stdin_payload {
        Some(_) => Some(
            child
                .stdin
                .take()
                .ok_or_else(|| format!("Could not open stdin for {what}"))?,
        ),
        None => None,
    };

    // #204: drain both pipes concurrently with a byte cap instead of
    // `wait_with_output`, whose per-pipe `Vec` grew without limit. Both pipes
    // are still read to EOF, so a chatty child can never deadlock on a full
    // pipe; the process exit status and the retained text are unchanged for
    // every output that fits, including the "not logged in" markers callers
    // match on.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    tokio::time::timeout(timeout, async {
        let write = async {
            if let Some(payload) = stdin_payload
                && let Some(mut input) = stdin.take()
            {
                let result = input.write_all(payload.as_bytes()).await;
                // Closing stdin is what tells the child no more input is
                // coming; `wait_with_output` used to do this by dropping it.
                drop(input);
                result
            } else {
                Ok(())
            }
        };
        let (write_result, status, stdout, stderr) = tokio::join!(
            write,
            child.wait(),
            drain_bounded(stdout, stdout_limit),
            drain_bounded(stderr, stderr_limit),
        );
        write_result.map_err(|error| format!("Could not send input to {what}: {error}"))?;
        let read_failed = |error: std::io::Error| format!("{what} failed to run: {error}");
        Ok(std::process::Output {
            status: status.map_err(|error| format!("{what} failed to run: {error}"))?,
            stdout: stdout.map_err(read_failed)?,
            stderr: stderr.map_err(read_failed)?,
        })
    })
    .await
    .map_err(|_| format!("{what} did not answer within {} seconds", timeout.as_secs()))?
}

pub fn tail(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    value.chars().skip(count - max_chars).collect()
}

pub fn stable_dedupe_bridge_models(models: Vec<BridgeModel>) -> Vec<BridgeModel> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_sanitizer_rejects_command_line_injection() {
        for model in ["claude-sonnet-5", "opus[1m]", "org/model:tag"] {
            assert_eq!(sanitize_model(model).as_deref(), Ok(model));
        }
        for model in ["", "sonnet 5", "model;whoami", "m$(whoami)", "m&y"] {
            assert!(sanitize_model(model).is_err(), "accepted {model:?}");
        }
    }

    #[test]
    fn lookup_prefers_native_executable_over_script_shim() {
        let output = "C:\\Users\\x\\npm\\codex.cmd\r\nC:\\Tools\\codex.exe\r\n";
        assert_eq!(
            pick_lookup_candidate(output),
            Some(PathBuf::from("C:\\Tools\\codex.exe"))
        );
    }

    /// #204: a runaway child must not be able to grow the app's heap through
    /// its pipes; both streams are still read to EOF so the child can finish.
    #[cfg(unix)]
    #[tokio::test]
    async fn runaway_child_output_is_capped_without_breaking_completion() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("yes abcdefghij | head -c 200000; yes 1234567890 | head -c 40000 >&2; exit 0");
        let output = run_headless_bounded(
            command,
            None,
            Duration::from_secs(30),
            "runaway child",
            4096,
            1024,
        )
        .await
        .expect("the child still runs to completion");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 4096);
        assert_eq!(output.stderr.len(), 1024);
    }

    /// The caps must not disturb ordinary probe output: the signed-out markers
    /// callers match on are only ever a few bytes.
    #[cfg(unix)]
    #[tokio::test]
    async fn short_output_and_stdin_payloads_are_preserved_verbatim() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("cat; echo 'Not logged in'; echo 'please run /login' >&2");
        let output = run_headless_bounded(
            command,
            Some("prompt over stdin"),
            Duration::from_secs(30),
            "status probe",
            HEADLESS_STDOUT_LIMIT,
            HEADLESS_STDERR_LIMIT,
        )
        .await
        .expect("probe runs");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("prompt over stdin"), "{stdout}");
        assert!(stdout.to_lowercase().contains("not logged in"), "{stdout}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("please run /login"),
            "stderr was not preserved"
        );
    }

    #[test]
    fn every_workdir_candidate_is_dedicated_and_empty_by_contract() {
        assert!(workdir_candidates().iter().all(|candidate| {
            candidate.file_name().and_then(|name| name.to_str()) == Some("cli-bridge-cwd")
        }));
    }
}
