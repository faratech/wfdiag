//! OpenAI Codex CLI bridge: "Sign in with ChatGPT" without API keys.
//!
//! Requests run through the user's installed Codex CLI (`codex exec`), so
//! usage bills to their ChatGPT plan and authentication stays entirely inside
//! OpenAI's own tooling — see `cli_bridge.rs` for the trust model. Headless
//! runs use `--json` (JSONL thread events on stdout) with the prompt piped
//! via stdin, a neutral empty working directory and the default read-only
//! sandbox, so the CLI can neither see the user's files nor change anything.
//!
//! No tool-calling (the CLI runs its own agent loop, not ours) and no
//! streaming (`--json` emits whole items): chat flattens to a single shot,
//! like Phi Silica.

use super::{ChatRequest, ChatTurn, FinishReason, ResolvedProviderConfig, cli_bridge};
use crate::ai_service::AIProvider;
use std::time::Duration;
use tokio::sync::mpsc;

/// Codex plans multi-step agent turns; give it more room than an API call.
const EXEC_TIMEOUT: Duration = Duration::from_secs(180);

/// One-shot analysis. Codex exec has no separate system-prompt flag, so the
/// system text is prepended (same pattern as Foundry's one-shot).
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    exec(cfg, format!("{system}\n\n{prompt}")).await
}

/// Chat via a single flattened completion: one full-text delta, then done.
pub async fn chat_single_shot(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let text = exec(cfg, super::phi::flatten_request(req)).await?;
    let _ = tx.send(text.clone()).await;
    Ok(ChatTurn {
        text,
        tool_calls: Vec::new(),
        finished: FinishReason::Stop,
    })
}

async fn exec(cfg: &ResolvedProviderConfig, payload: String) -> Result<String, String> {
    let cli = cfg.endpoint_or_err(AIProvider::CodexCli)?;
    let workdir = cli_bridge::bridge_workdir()?;
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args([
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "--color",
        "never",
        "--sandbox",
        "read-only",
    ]);
    cmd.arg("-C").arg(&workdir);
    if let Some(model) = cfg.model.as_deref() {
        cmd.arg("-m").arg(cli_bridge::sanitize_model(model)?);
    }
    // `-` = read the prompt from stdin (no command-line length or quoting issues)
    cmd.arg("-");
    cmd.current_dir(&workdir);

    let output = cli_bridge::run_headless(cmd, Some(&payload), EXEC_TIMEOUT, "Codex CLI").await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        // codex writes its real failure to stdout as a JSONL `error` event;
        // stderr is often empty. Check stdout first, then stderr, then a
        // generic hint — never swallow the actual reason (see issue #99).
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = extract_error(&stdout)
            .or_else(|| {
                let tail = cli_bridge::tail(stderr.trim(), 400);
                (!tail.is_empty()).then_some(tail)
            })
            .or_else(|| {
                let tail = cli_bridge::tail(stdout.trim(), 400);
                (!tail.is_empty()).then_some(tail)
            })
            .unwrap_or_else(|| {
                "no error output. If you recently signed out, sign in again in Settings."
                    .to_string()
            });
        return Err(format!(
            "Codex CLI failed (exit {}): {}",
            output
                .status
                .code()
                .map_or_else(|| "?".to_string(), |c| c.to_string()),
            detail
        ));
    }
    parse_exec_jsonl(&stdout)
}

/// The message from the last `error` event in `codex exec --json` output, if
/// any. Used to surface the real reason behind a non-zero exit.
fn extract_error(stdout: &str) -> Option<String> {
    let mut message = None;
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(line)
            && event.get("type").and_then(|t| t.as_str()) == Some("error")
        {
            message = event
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string);
        }
    }
    message
}

/// Extract the answer from `codex exec --json` output: JSONL thread events,
/// where the text of the last completed `agent_message` item is the reply.
/// An `error` event only fails the call when no answer arrived at all (the
/// CLI retries transient errors itself).
fn parse_exec_jsonl(stdout: &str) -> Result<String, String> {
    let mut answer: Option<String> = None;
    let mut error: Option<String> = None;
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match event.get("type").and_then(|t| t.as_str()) {
            Some("item.completed") => {
                if event.pointer("/item/type").and_then(|t| t.as_str()) == Some("agent_message")
                    && let Some(text) = event.pointer("/item/text").and_then(|t| t.as_str())
                {
                    answer = Some(text.to_string());
                }
            }
            Some("error") => {
                error = Some(
                    event
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    match (answer, error) {
        (Some(text), _) => Ok(text),
        (None, Some(message)) => Err(format!("Codex CLI reported an error: {message}")),
        (None, None) => Err("Codex CLI returned no answer (unexpected output format)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_agent_message() {
        let out = concat!(
            r#"{"type":"thread.started","thread_id":"t1"}"#,
            "\n",
            r#"{"type":"turn.started"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"thinking"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"first"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"final answer"}}"#,
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":10}}"#,
            "\n",
        );
        assert_eq!(parse_exec_jsonl(out).unwrap(), "final answer");
    }

    #[test]
    fn error_event_fails_only_without_an_answer() {
        let failed = r#"{"type":"error","message":"stream disconnected"}"#;
        assert!(
            parse_exec_jsonl(failed)
                .unwrap_err()
                .contains("stream disconnected")
        );
        // A transient error followed by a successful answer is a success
        let recovered = concat!(
            r#"{"type":"error","message":"retrying"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"ok"}}"#,
        );
        assert_eq!(parse_exec_jsonl(recovered).unwrap(), "ok");
    }

    #[test]
    fn extract_error_finds_the_last_error_event() {
        let out = concat!(
            r#"{"type":"error","message":"stream disconnected, retrying"}"#,
            "\n",
            r#"{"type":"error","message":"401 Unauthorized: please run codex login"}"#,
            "\n",
        );
        assert_eq!(
            extract_error(out).unwrap(),
            "401 Unauthorized: please run codex login"
        );
        // No error event -> None (so the caller can fall back to stderr/stdout)
        assert_eq!(extract_error(r#"{"type":"turn.completed"}"#), None);
        assert_eq!(extract_error("plain text"), None);
    }

    #[test]
    fn non_json_noise_is_skipped_and_empty_output_errors() {
        let noisy = concat!(
            "Reading prompt from stdin...\n",
            "not json at all\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}"#,
        );
        assert_eq!(parse_exec_jsonl(noisy).unwrap(), "hi");
        assert!(parse_exec_jsonl("").is_err());
        assert!(parse_exec_jsonl("plain text only").is_err());
    }
}
