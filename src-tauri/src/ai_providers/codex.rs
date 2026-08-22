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

/// The invariant part of every Codex invocation. User configuration and
/// repository rules are deliberately disabled because a bridge run must be
/// isolated from machine-local instructions as well as workspace files.
const CODEX_EXEC_ARGS: &[&str] = &[
    "exec",
    "--json",
    "--ephemeral",
    "--skip-git-repo-check",
    "--ignore-user-config",
    "--ignore-rules",
    "--color",
    "never",
    "--sandbox",
    "read-only",
];

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
        actual_models: Vec::new(),
        provider_replay: None,
    })
}

async fn exec(cfg: &ResolvedProviderConfig, payload: String) -> Result<String, String> {
    let cli = cfg.endpoint_or_err(AIProvider::CodexCli)?;
    let workdir = cli_bridge::bridge_workdir().await?;
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args(CODEX_EXEC_ARGS);
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
        let detail = with_upstream_auth_hint(&detail);
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

/// Codex CLI 0.149.0 shipped an auth regression (upstream issues #39883 /
/// #40138): `login status` still reports success but every ChatGPT-login
/// request fails with 401/unauthorized. When that exact pattern shows up,
/// point the user at the upstream fix instead of our config — nothing here
/// is misconfigured.
fn with_upstream_auth_hint(detail: &str) -> String {
    let text = detail.to_lowercase();
    let auth_failure = (text.contains("401") || text.contains("unauthorized"))
        && (text.contains("login")
            || text.contains("auth")
            || text.contains("token")
            || text.contains("sign in"));
    if auth_failure {
        format!(
            "{detail}\n\nNote: Codex CLI 0.149.0 has a known sign-in bug that produces \
             exactly this error. Downgrade with `npm install -g @openai/codex@0.148.0` \
             and try again."
        )
    } else {
        detail.to_string()
    }
}

/// Extract the answer from `codex exec --json` output: JSONL thread events,
/// where the text of the last completed `agent_message` item is the reply.
/// An `error` event only fails the call when no answer arrived at all (the
/// CLI retries transient errors itself).
fn parse_exec_jsonl(stdout: &str) -> Result<String, String> {
    let mut answer: Option<String> = None;
    let mut error: Option<String> = None;
    let mut completed = false;
    let mut failed: Option<String> = None;
    let mut malformed_event = false;
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            malformed_event = true;
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
            Some("turn.completed") => completed = true,
            Some("turn.failed") => {
                failed = Some(
                    event
                        .pointer("/error/message")
                        .or_else(|| event.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown turn failure")
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    if let Some(message) = failed {
        return Err(format!("Codex CLI turn failed: {message}"));
    }
    if malformed_event {
        return Err("Codex CLI returned malformed JSONL output".to_string());
    }
    if !completed {
        return Err(match error {
            Some(message) => format!("Codex CLI did not complete: {message}"),
            None => "Codex CLI ended before turn.completed".to_string(),
        });
    }
    match answer.filter(|text| !text.trim().is_empty()) {
        Some(text) => Ok(text),
        None => Err("Codex CLI completed without an answer".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_auth_hint_fires_only_on_401_patterns() {
        let hit = with_upstream_auth_hint(
            "stream error: HTTP 401 Unauthorized while using ChatGPT login",
        );
        assert!(hit.contains("0.148.0"), "hint missing: {hit}");

        let miss = with_upstream_auth_hint("model 'nope' is not available");
        assert!(!miss.contains("0.148.0"), "hint must not fire here: {miss}");

        let miss2 = with_upstream_auth_hint("sandbox denied access to C:\\Temp");
        assert!(
            !miss2.contains("0.148.0"),
            "hint must not fire here: {miss2}"
        );
    }

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
    fn error_event_fails_without_a_completed_turn() {
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
            "\n",
            r#"{"type":"turn.completed"}"#,
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
            "\n",
            r#"{"type":"turn.completed"}"#,
        );
        assert_eq!(parse_exec_jsonl(noisy).unwrap(), "hi");
        assert!(parse_exec_jsonl("").is_err());
        assert!(parse_exec_jsonl("plain text only").is_err());
    }

    #[test]
    fn partial_answer_does_not_hide_turn_failure() {
        let out = concat!(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}"#,
            "\n",
            r#"{"type":"turn.failed","error":{"message":"model overloaded"}}"#,
        );
        assert!(
            parse_exec_jsonl(out)
                .unwrap_err()
                .contains("model overloaded")
        );
    }

    #[test]
    fn answer_without_terminal_event_is_rejected() {
        let out = r#"{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}"#;
        assert!(
            parse_exec_jsonl(out)
                .unwrap_err()
                .contains("turn.completed")
        );
    }

    #[test]
    fn invocation_is_ephemeral_read_only_and_ignores_local_instructions() {
        for flag in [
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--sandbox",
            "read-only",
        ] {
            assert!(
                CODEX_EXEC_ARGS.contains(&flag),
                "missing isolation arg {flag}"
            );
        }
    }
}
