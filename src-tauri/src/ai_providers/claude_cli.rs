//! Anthropic Claude Code CLI bridge: Claude subscription without API keys.
//!
//! Requests run through the user's installed Claude Code CLI (`claude -p`),
//! so usage bills to their Claude Pro/Max plan and authentication stays
//! entirely inside Anthropic's own tooling — see `cli_bridge.rs` for the
//! trust model. This is the same drive-the-genuine-CLI pattern Microsoft's
//! Intelligent Terminal and Zed ship (over ACP); this app never touches
//! OAuth tokens, which is what Anthropic's terms actually prohibit.
//!
//! Primary transport is the ACP adapter (`acp_bridge.rs`) — the exact
//! integration Intelligent Terminal ships, with streamed message chunks.
//! When npx isn't available (native CLI install without Node), requests
//! fall back to the CLI's own headless mode: `-p --output-format json`
//! (one JSON result object on stdout) with the payload piped via stdin, a
//! neutral empty working directory and `--max-turns 2` so the CLI answers
//! instead of wandering into its agent loop. No tool-calling either way
//! (the CLI runs its own loop, not ours).

use super::acp_bridge::{self, AdapterOutcome};
use super::{ChatRequest, ChatTurn, FinishReason, ResolvedProviderConfig, cli_bridge};
use crate::ai_service::AIProvider;
use std::time::Duration;
use tokio::sync::mpsc;

/// Claude Code may spend a turn thinking before answering; match Codex.
const EXEC_TIMEOUT: Duration = Duration::from_secs(180);

/// Fixed instruction argument: `-p` requires a prompt argument and treats
/// piped stdin as attached content, so the real payload always goes via
/// stdin (never argv — length caps and quoting).
const STDIN_INSTRUCTION: &str = "Answer the request in the piped input.";

/// One-shot analysis. The system text is prepended to the payload (same
/// pattern as the Codex and Foundry one-shots).
pub async fn one_shot(
    cfg: &ResolvedProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    exec(cfg, format!("{system}\n\n{prompt}"), None).await
}

/// Chat turn: the ACP path streams chunk deltas through `tx` as they
/// arrive; the print-mode fallback sends one full-text delta.
pub async fn chat_single_shot(
    cfg: &ResolvedProviderConfig,
    req: &ChatRequest,
    tx: mpsc::Sender<String>,
) -> Result<ChatTurn, String> {
    let text = exec(cfg, super::phi::flatten_request(req), Some(tx)).await?;
    Ok(ChatTurn {
        text,
        tool_calls: Vec::new(),
        finished: FinishReason::Stop,
    })
}

/// Run one prompt: ACP adapter first (the Intelligent Terminal path),
/// `claude -p` as the no-Node fallback. When `delta_tx` is given the caller
/// gets streaming deltas either way (chunked over ACP, one shot otherwise).
async fn exec(
    cfg: &ResolvedProviderConfig,
    payload: String,
    delta_tx: Option<mpsc::Sender<String>>,
) -> Result<String, String> {
    match acp_bridge::claude_prompt(&payload, cfg.model.as_deref(), delta_tx.clone()).await {
        AdapterOutcome::Answer(text) => return Ok(text),
        AdapterOutcome::Failed(error) => return Err(error),
        AdapterOutcome::NoNpx => {}
    }
    let text = exec_print_mode(cfg, payload).await?;
    if let Some(tx) = delta_tx {
        let _ = tx.send(text.clone()).await;
    }
    Ok(text)
}

async fn exec_print_mode(cfg: &ResolvedProviderConfig, payload: String) -> Result<String, String> {
    let cli = cfg.endpoint_or_err(AIProvider::ClaudeCode)?;
    let workdir = cli_bridge::bridge_workdir()?;
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args([
        "-p",
        STDIN_INSTRUCTION,
        "--output-format",
        "json",
        "--max-turns",
        "2",
    ]);
    if let Some(model) = cfg.model.as_deref() {
        cmd.arg("--model").arg(cli_bridge::sanitize_model(model)?);
    }
    // Empty cwd: headless mode denies permissioned tools, and there is
    // nothing here for the read-only ones to read
    cmd.current_dir(&workdir);

    let output = cli_bridge::run_headless(cmd, Some(&payload), EXEC_TIMEOUT, "Claude Code").await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = cli_bridge::tail(stderr.trim(), 400);
        return Err(format!(
            "Claude Code failed (exit {}): {}",
            output
                .status
                .code()
                .map_or_else(|| "?".to_string(), |c| c.to_string()),
            if detail.is_empty() {
                "no error output. If you recently signed out, sign in again in Settings."
            } else {
                &detail
            }
        ));
    }
    parse_result_json(&String::from_utf8_lossy(&output.stdout))
}

/// Extract the answer from `claude -p --output-format json`: one JSON object
/// of shape `{"type":"result","subtype":"success","is_error":false,
/// "result":"…"}`. Non-JSON noise around it is tolerated by scanning lines.
fn parse_result_json(stdout: &str) -> Result<String, String> {
    let candidates = std::iter::once(stdout.trim()).chain(
        stdout
            .lines()
            .rev()
            .map(str::trim)
            .filter(|l| l.starts_with('{')),
    );
    for candidate in candidates {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("result") {
            continue;
        }
        let result_text = value.get("result").and_then(|r| r.as_str());
        if value.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
            return Err(format!(
                "Claude Code reported an error: {}",
                result_text.unwrap_or("unknown error")
            ));
        }
        return result_text.map(str::to_string).ok_or_else(|| {
            format!(
                "Claude Code returned no answer ({})",
                value
                    .get("subtype")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unexpected output format")
            )
        });
    }
    Err("Claude Code returned no answer (unexpected output format)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_successful_result() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"the answer","duration_ms":1200}"#;
        assert_eq!(parse_result_json(out).unwrap(), "the answer");
    }

    #[test]
    fn error_results_and_missing_answers_fail() {
        let err =
            r#"{"type":"result","subtype":"success","is_error":true,"result":"credit exhausted"}"#;
        assert!(
            parse_result_json(err)
                .unwrap_err()
                .contains("credit exhausted")
        );
        let no_answer = r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#;
        assert!(
            parse_result_json(no_answer)
                .unwrap_err()
                .contains("error_max_turns")
        );
        assert!(parse_result_json("").is_err());
        assert!(parse_result_json("plain text").is_err());
    }

    #[test]
    fn noise_around_the_json_object_is_tolerated() {
        let noisy = concat!(
            "some banner line\n",
            r#"{"type":"other"}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#,
            "\n",
        );
        assert_eq!(parse_result_json(noisy).unwrap(), "ok");
    }
}
