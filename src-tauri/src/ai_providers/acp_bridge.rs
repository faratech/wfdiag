//! Agent Client Protocol (ACP) transport for the subscription CLI bridges.
//!
//! This mirrors Microsoft's Intelligent Terminal integration exactly: the
//! agent CLI is driven through the ACP-project-maintained adapter launched
//! via `npx -y @agentclientprotocol/claude-agent-acp`, speaking JSON-RPC
//! over the child's stdio with the same `agent-client-protocol` crate
//! Intelligent Terminal uses. The adapter wraps the locally installed
//! Claude Code, which owns authentication — we never see a token.
//!
//! Flow per request: spawn adapter → `initialize` → `session/new` (cwd =
//! the empty bridge workdir) → `session/prompt`, collecting streamed
//! `agent_message_chunk` updates until the turn's stop reason arrives.
//! Permission requests are rejected (this bridge is Q&A only — the agent
//! must answer from the prompt, not run tools), and fs/terminal capability
//! is never advertised.

use agent_client_protocol as acp;
use agent_client_protocol::schema::v1;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::cli_bridge;

/// First `npx -y` run downloads the adapter package; cached runs are fast.
const INIT_TIMEOUT: Duration = Duration::from_secs(60);
/// The adapter spawns the agent CLI underneath during session/new.
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(180);

/// Adapter package for Claude Code (renamed from the deprecated
/// `@zed-industries/claude-code-acp` — same registry entry Intelligent
/// Terminal launches).
const CLAUDE_ADAPTER_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";

/// Result of attempting a prompt through the ACP adapter.
pub enum AdapterOutcome {
    /// The turn completed; chunks were forwarded to `delta_tx` when given.
    Answer(String),
    /// The adapter ran but the turn failed.
    Failed(String),
    /// npx isn't available (e.g. native CLI install without Node) — the
    /// caller should fall back to the CLI's own headless mode.
    NoNpx,
}

/// Run one prompt through the Claude Code ACP adapter. `model` must already
/// be sanitized; it rides the documented `ANTHROPIC_MODEL` env var (the
/// adapter has no model flag). Text chunks stream into `delta_tx` as they
/// arrive when a sender is provided.
pub async fn claude_prompt(
    payload: &str,
    model: Option<&str>,
    delta_tx: Option<mpsc::Sender<String>>,
) -> AdapterOutcome {
    let Ok(npx) = cli_bridge::resolve_cli("npx", None).await else {
        return AdapterOutcome::NoNpx;
    };

    let workdir = match cli_bridge::bridge_workdir() {
        Ok(dir) => dir,
        Err(e) => return AdapterOutcome::Failed(e),
    };

    let mut cmd = tokio::process::Command::new(&npx);
    cmd.args(["-y", CLAUDE_ADAPTER_PACKAGE]);
    cmd.current_dir(&workdir);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Same scrubs as Intelligent Terminal plus our subscription-only rule:
    // CLAUDECODE is the adapter's recursion guard (it refuses to start when
    // set); the key vars would override the CLI's stored login and turn
    // subscription runs into API billing.
    for var in [
        "CLAUDECODE",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_API_KEY",
    ] {
        cmd.env_remove(var);
    }
    if let Some(model) = model {
        cmd.env("ANTHROPIC_MODEL", model);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return AdapterOutcome::Failed(format!("Could not start the ACP adapter: {e}")),
    };
    let outgoing = child.stdin.take().expect("stdin piped").compat_write();
    let incoming = child.stdout.take().expect("stdout piped").compat();
    // Capture stderr for error messages (npx banners, adapter panics) and
    // keep the pipe drained so the adapter can't block on it.
    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut buf) = buf.lock() {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        });
    }

    let collected: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let builder = acp::Client
        .builder()
        .name("wfdiag")
        .on_receive_request(
            move |req: v1::AgentRequest,
                  responder: acp::Responder<serde_json::Value>,
                  _cx| async move {
                match req {
                    // Q&A bridge: never grant tool permissions — the agent
                    // answers from the prompt text alone. (Enum-level
                    // handlers respond with the serialized variant, same as
                    // Intelligent Terminal's respond_enum.)
                    v1::AgentRequest::RequestPermissionRequest(request) => {
                        let response = v1::ClientResponse::RequestPermissionResponse(
                            v1::RequestPermissionResponse::new(reject_outcome(&request.options)),
                        );
                        match serde_json::to_value(response) {
                            Ok(value) => responder.respond(value),
                            Err(e) => responder
                                .respond_with_error(acp::Error::into_internal_error(e)),
                        }
                    }
                    // fs/terminal capability is never advertised, so anything
                    // else is out of contract
                    _ => responder.respond_with_error(acp::Error::method_not_found()),
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let collected = collected.clone();
                move |notif: v1::AgentNotification, _cx| {
                    let collected = collected.clone();
                    let delta_tx = delta_tx.clone();
                    async move {
                        if let v1::AgentNotification::SessionNotification(n) = notif
                            && let v1::SessionUpdate::AgentMessageChunk(chunk) = n.update
                            && let v1::ContentBlock::Text(text) = chunk.content
                        {
                            if let Ok(mut collected) = collected.lock() {
                                collected.push_str(&text.text);
                            }
                            if let Some(tx) = delta_tx {
                                let _ = tx.send(text.text).await;
                            }
                        }
                        Ok(())
                    }
                }
            },
            acp::on_receive_notification!(),
        );

    let workdir_for_session = workdir.clone();
    let turn = builder
        .connect_with(acp::ByteStreams::new(outgoing, incoming), async move |cx| {
            tokio::time::timeout(
                INIT_TIMEOUT,
                cx.send_request(
                    v1::InitializeRequest::new(acp::schema::ProtocolVersion::V1)
                        .client_info(v1::Implementation::new("wfdiag", env!("CARGO_PKG_VERSION"))),
                )
                .block_task(),
            )
            .await
            .map_err(|_| acp::Error::internal_error().data("initialize timed out"))??;

            let session = tokio::time::timeout(
                SESSION_TIMEOUT,
                cx.send_request(v1::NewSessionRequest::new(workdir_for_session))
                    .block_task(),
            )
            .await
            .map_err(|_| acp::Error::internal_error().data("session/new timed out"))??;

            let response = tokio::time::timeout(
                PROMPT_TIMEOUT,
                cx.send_request(v1::PromptRequest::new(
                    session.session_id,
                    vec![v1::ContentBlock::Text(v1::TextContent::new(payload))],
                ))
                .block_task(),
            )
            .await
            .map_err(|_| acp::Error::internal_error().data("prompt timed out"))??;

            Ok(response.stop_reason)
        })
        .await;

    let text = collected.lock().map(|t| t.clone()).unwrap_or_default();
    match turn {
        Ok(v1::StopReason::Refusal) => {
            AdapterOutcome::Failed("The model declined to answer this request".to_string())
        }
        Ok(_) if !text.trim().is_empty() => AdapterOutcome::Answer(text),
        Ok(stop) => AdapterOutcome::Failed(format!(
            "Claude Code returned no text (turn ended with {stop:?})"
        )),
        Err(e) => {
            let stderr = stderr_buf.lock().map(|b| b.clone()).unwrap_or_default();
            let stderr_tail = cli_bridge::tail(stderr.trim(), 300);
            AdapterOutcome::Failed(if stderr_tail.is_empty() {
                format!("Claude Code (ACP) failed: {e}")
            } else {
                format!("Claude Code (ACP) failed: {e} — {stderr_tail}")
            })
        }
    }
}

/// Pick the reject option the agent offered (prefer "reject once"), falling
/// back to the protocol's cancelled outcome when none exists.
fn reject_outcome(options: &[v1::PermissionOption]) -> v1::RequestPermissionOutcome {
    let pick = |kind: v1::PermissionOptionKind| {
        options
            .iter()
            .find(|o| o.kind == kind)
            .map(|o| o.option_id.clone())
    };
    match pick(v1::PermissionOptionKind::RejectOnce)
        .or_else(|| pick(v1::PermissionOptionKind::RejectAlways))
    {
        Some(option_id) => {
            v1::RequestPermissionOutcome::Selected(v1::SelectedPermissionOutcome::new(option_id))
        }
        None => v1::RequestPermissionOutcome::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, kind: v1::PermissionOptionKind) -> v1::PermissionOption {
        v1::PermissionOption::new(id.to_string(), id.to_string(), kind)
    }

    #[test]
    fn rejects_via_the_offered_option_and_cancels_without_one() {
        let options = vec![
            option("allow", v1::PermissionOptionKind::AllowOnce),
            option("deny", v1::PermissionOptionKind::RejectOnce),
        ];
        match reject_outcome(&options) {
            v1::RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(sel.option_id.to_string(), "deny");
            }
            other => panic!("expected Selected, got {other:?}"),
        }
        assert!(matches!(
            reject_outcome(&[option("allow", v1::PermissionOptionKind::AllowOnce)]),
            v1::RequestPermissionOutcome::Cancelled
        ));
    }
}
