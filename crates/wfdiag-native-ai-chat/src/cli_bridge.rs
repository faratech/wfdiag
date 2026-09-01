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
use tokio::io::AsyncWriteExt;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
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

#[allow(clippy::unused_async)] // Windows validates candidates with an awaited child process.
pub async fn bridge_workdir() -> Result<PathBuf, String> {
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

pub async fn run_headless(
    mut command: tokio::process::Command,
    stdin_payload: Option<&str>,
    timeout: Duration,
    what: &str,
) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    {
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .to_ascii_lowercase();
        if program.ends_with(".cmd") || program.ends_with(".bat") {
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

    tokio::time::timeout(timeout, async {
        let write = async {
            if let Some(payload) = stdin_payload
                && let Some(mut input) = stdin.take()
            {
                input.write_all(payload.as_bytes()).await
            } else {
                Ok(())
            }
        };
        let (write_result, output) = tokio::join!(write, child.wait_with_output());
        write_result.map_err(|error| format!("Could not send input to {what}: {error}"))?;
        output.map_err(|error| format!("{what} failed to run: {error}"))
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

    #[test]
    fn every_workdir_candidate_is_dedicated_and_empty_by_contract() {
        assert!(workdir_candidates().iter().all(|candidate| {
            candidate.file_name().and_then(|name| name.to_str()) == Some("cli-bridge-cwd")
        }));
    }
}
