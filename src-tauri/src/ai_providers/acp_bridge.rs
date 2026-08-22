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
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::cli_bridge;

/// First `npx -y` run downloads the adapter package; cached runs are fast.
const INIT_TIMEOUT: Duration = Duration::from_secs(60);
/// The adapter spawns the agent CLI underneath during session/new.
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
/// One prompt turn. Must stay STRICTLY below ai_chat's whole-turn deadline
/// (TURN_TIMEOUT_SECS = 180 s, which wraps everything including this) or the
/// outer limit kills the turn first and its specific "prompt timed out"
/// error — and the adapter cleanup it triggers — becomes unreachable.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(170);
/// Outer safety net around model discovery: must exceed the sum of the
/// per-step budgets it wraps (INIT 60 + SESSION 30 + margin), otherwise the
/// inner timeouts are unreachable and a first-run `npx` package download
/// always dies at the outer limit.
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(100);
const MODEL_LIST_STDOUT_LIMIT: u64 = 2 * 1024 * 1024;
const MODEL_LIST_STDERR_LIMIT: u64 = 32 * 1024;

/// Adapter package for Claude Code (renamed from the deprecated
/// `@zed-industries/claude-code-acp` — same registry entry Intelligent
/// Terminal launches). PINNED: the project ships breaking changes in minor
/// bumps pre-1.0, and an unpinned `npx -y` would take every release
/// overnight with no code change on our side. Bump deliberately after
/// re-verifying the handshake.
const CLAUDE_ADAPTER_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@0.70.0";

/// The one way an adapter process is launched, shared by discovery and
/// prompts: `npx -y <package>` in the neutral bridge cwd, hidden, with the
/// same scrubs Intelligent Terminal applies (CLAUDECODE is the adapter's
/// recursion guard; the key vars would override the CLI's stored login and
/// turn subscription runs into API billing) plus the documented
/// `CLAUDE_CODE_EXECUTABLE` override so the adapter drives the exact CLI we
/// resolved/configured instead of re-resolving its own. --prefer-offline
/// skips the per-run npm registry round-trip (measured ~12 s of every
/// discovery) when the package is already in the npx cache — which the pin
/// above guarantees after the first run.
fn adapter_command(npx: &Path, workdir: &Path, claude_path: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(npx);
    cmd.args(["-y", "--prefer-offline", CLAUDE_ADAPTER_PACKAGE]);
    cmd.current_dir(workdir);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.env_remove("CLAUDECODE");
    for var in cli_bridge::SUBSCRIPTION_OVERRIDE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.env("CLAUDE_CODE_EXECUTABLE", claude_path);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Bounded stderr reader shared by both entry points, joined (with a short
/// timeout) after the child is gone so error messages can carry its tail.
fn spawn_stderr_reader(
    stderr: Option<tokio::process::ChildStderr>,
) -> Option<tokio::task::JoinHandle<String>> {
    stderr.map(|stderr| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr
                .take(MODEL_LIST_STDERR_LIMIT)
                .read_to_end(&mut bytes)
                .await;
            String::from_utf8_lossy(&bytes).into_owned()
        })
    })
}

/// Kill the adapter and collect what it wrote to stderr.
async fn reap_adapter(
    child: &mut tokio::process::Child,
    stderr_task: Option<tokio::task::JoinHandle<String>>,
) -> String {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    match stderr_task {
        Some(task) => tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Describe a child that died before the protocol produced a result.
fn adapter_exit_error(status: std::io::Result<std::process::ExitStatus>) -> acp::Error {
    let exit = status
        .ok()
        .and_then(|s| s.code())
        .map_or_else(|| "?".to_string(), |c| c.to_string());
    acp::Error::internal_error().data(format!(
        "the Claude ACP adapter exited before answering (exit {exit})"
    ))
}

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

/// Start an ACP session without sending a prompt and read the model selector
/// that the current Claude Agent SDK reports. This is live/account-aware and
/// does not spend tokens: `session/new` initializes metadata only.
pub async fn list_claude_models(
    claude_path: &Path,
) -> Result<cli_bridge::BridgeModelCatalog, String> {
    tokio::time::timeout(MODEL_LIST_TIMEOUT, list_claude_models_inner(claude_path))
        .await
        .map_err(|_| {
            format!(
                "Claude Code model discovery did not finish within {} seconds",
                MODEL_LIST_TIMEOUT.as_secs()
            )
        })?
}

async fn list_claude_models_inner(
    claude_path: &Path,
) -> Result<cli_bridge::BridgeModelCatalog, String> {
    let npx = cli_bridge::resolve_cli("npx", None).await.map_err(|_| {
        "Claude Code model discovery requires Node.js/npm (npx was not found).".to_string()
    })?;
    let workdir = cli_bridge::bridge_workdir().await?;

    // The path override makes an unsaved Settings draft effective for
    // discovery too (see `adapter_command`).
    let mut cmd = adapter_command(&npx, &workdir, claude_path);
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("Could not start the Claude ACP adapter: {error}"))?;
    let outgoing = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open the Claude ACP adapter stdin".to_string())?
        .compat_write();
    let incoming = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open the Claude ACP adapter stdout".to_string())?
        .take(MODEL_LIST_STDOUT_LIMIT)
        .compat();
    let stderr_task = spawn_stderr_reader(child.stderr.take());

    let builder = acp::Client
        .builder()
        .name("wfdiag-model-discovery")
        .on_receive_request(
            move |req: v1::AgentRequest,
                  responder: acp::Responder<serde_json::Value>,
                  _cx| async move {
                match req {
                    v1::AgentRequest::RequestPermissionRequest(request) => {
                        let response = v1::ClientResponse::RequestPermissionResponse(
                            v1::RequestPermissionResponse::new(reject_outcome(&request.options)),
                        );
                        match serde_json::to_value(response) {
                            Ok(value) => responder.respond(value),
                            Err(error) => responder
                                .respond_with_error(acp::Error::into_internal_error(error)),
                        }
                    }
                    _ => responder.respond_with_error(acp::Error::method_not_found()),
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_notification(
            |_notification: v1::AgentNotification, _cx| async move { Ok(()) },
            acp::on_receive_notification!(),
        );

    let workdir_for_session = workdir.clone();
    let connect = builder.connect_with(acp::ByteStreams::new(outgoing, incoming), async move |cx| {
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

        tokio::time::timeout(
            SESSION_TIMEOUT,
            cx.send_request(v1::NewSessionRequest::new(workdir_for_session))
                .block_task(),
        )
        .await
        .map_err(|_| acp::Error::internal_error().data("session/new timed out"))?
    });
    // Race the handshake against adapter death so a child that dies on
    // startup (bad npx, unusable cwd, missing CLI) fails in about a second
    // with its stderr instead of burning the full initialize timeout.
    tokio::pin!(connect);
    let response = tokio::select! {
        biased; // prefer a completed protocol result over a simultaneous exit
        res = &mut connect => res,
        status = child.wait() => Err(adapter_exit_error(status)),
    };

    let stderr = reap_adapter(&mut child, stderr_task).await;

    match response {
        Ok(session) => catalog_from_config_options(session.config_options.as_deref()),
        Err(error) => {
            let detail = cli_bridge::tail(stderr.trim(), 300);
            Err(if detail.is_empty() {
                format!("Claude Code model discovery failed: {error}")
            } else {
                format!("Claude Code model discovery failed: {error} — {detail}")
            })
        }
    }
}

fn catalog_from_config_options(
    options: Option<&[v1::SessionConfigOption]>,
) -> Result<cli_bridge::BridgeModelCatalog, String> {
    let option = options
        .unwrap_or_default()
        .iter()
        .find(|option| {
            matches!(
                option.category,
                Some(v1::SessionConfigOptionCategory::Model)
            ) || option.id.0.as_ref() == "model"
        })
        .ok_or_else(|| "Claude Code did not report a model selector.".to_string())?;
    let v1::SessionConfigKind::Select(select) = &option.kind else {
        return Err("Claude Code reported an unsupported model selector.".to_string());
    };

    let source: Vec<&v1::SessionConfigSelectOption> = match &select.options {
        v1::SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
        v1::SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .collect(),
        _ => Vec::new(),
    };
    let models = source
        .into_iter()
        .filter_map(|option| {
            let id = option.value.0.trim().to_string();
            if id.is_empty() {
                return None;
            }
            Some(cli_bridge::BridgeModel {
                label: non_empty_distinct(&option.name, &id),
                description: option
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                id,
            })
        })
        .collect();
    let models = cli_bridge::stable_dedupe_bridge_models(models);
    if models.is_empty() {
        return Err("Claude Code reported an empty model selector.".to_string());
    }
    let current = select.current_value.0.trim().to_string();
    Ok(cli_bridge::BridgeModelCatalog {
        models,
        default_model: (!current.is_empty()).then_some(current),
    })
}

fn non_empty_distinct(value: &str, id: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value != id).then(|| value.to_string())
}

/// Run one prompt through the Claude Code ACP adapter, driving the CLI at
/// `claude_path` (resolved/configured by the caller). `model` must already
/// be sanitized; it rides the documented `ANTHROPIC_MODEL` env var (the
/// adapter has no model flag). Text chunks stream into `delta_tx` as they
/// arrive when a sender is provided.
pub async fn claude_prompt(
    claude_path: &Path,
    payload: &str,
    model: Option<&str>,
    delta_tx: Option<mpsc::Sender<String>>,
) -> AdapterOutcome {
    let Ok(npx) = cli_bridge::resolve_cli("npx", None).await else {
        return AdapterOutcome::NoNpx;
    };

    let workdir = match cli_bridge::bridge_workdir().await {
        Ok(dir) => dir,
        Err(e) => return AdapterOutcome::Failed(e),
    };

    let mut cmd = adapter_command(&npx, &workdir, claude_path);
    if let Some(model) = model {
        cmd.env("ANTHROPIC_MODEL", model);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return AdapterOutcome::Failed(format!("Could not start the ACP adapter: {e}")),
    };
    let outgoing = child.stdin.take().expect("stdin piped").compat_write();
    let incoming = child.stdout.take().expect("stdout piped").compat();
    // Capture stderr for error messages (npx banners, adapter panics) and
    // keep the pipe drained so the adapter can't block on it.
    let stderr_task = spawn_stderr_reader(child.stderr.take());

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
    let connect = builder.connect_with(acp::ByteStreams::new(outgoing, incoming), async move |cx| {
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
    });
    // Race the turn against adapter death — a child that dies on startup
    // (bad npx, unusable cwd, missing CLI) fails in about a second with its
    // stderr instead of burning the full initialize timeout.
    tokio::pin!(connect);
    let turn = tokio::select! {
        biased; // prefer a completed protocol result over a simultaneous exit
        res = &mut connect => res,
        status = child.wait() => Err(adapter_exit_error(status)),
    };

    let stderr = reap_adapter(&mut child, stderr_task).await;

    let text = collected.lock().map(|t| t.clone()).unwrap_or_default();
    match turn {
        Ok(stop) => finish_adapter_turn(stop, text),
        Err(e) => {
            let stderr_tail = cli_bridge::tail(stderr.trim(), 300);
            AdapterOutcome::Failed(if stderr_tail.is_empty() {
                format!("Claude Code (ACP) failed: {e}")
            } else {
                format!("Claude Code (ACP) failed: {e} — {stderr_tail}")
            })
        }
    }
}

fn finish_adapter_turn(stop: v1::StopReason, text: String) -> AdapterOutcome {
    match stop {
        v1::StopReason::EndTurn if !text.trim().is_empty() => AdapterOutcome::Answer(text),
        v1::StopReason::EndTurn => {
            AdapterOutcome::Failed("Claude Code completed without an answer".to_string())
        }
        v1::StopReason::MaxTokens => AdapterOutcome::Failed(
            "Claude Code reached its token limit before completing the answer".to_string(),
        ),
        v1::StopReason::MaxTurnRequests => AdapterOutcome::Failed(
            "Claude Code reached its turn limit before completing the answer".to_string(),
        ),
        v1::StopReason::Refusal => {
            AdapterOutcome::Failed("The model declined to answer this request".to_string())
        }
        v1::StopReason::Cancelled => {
            AdapterOutcome::Failed("Claude Code cancelled the request".to_string())
        }
        _ => AdapterOutcome::Failed(format!(
            "Claude Code ended with an unsupported stop reason ({stop:?})"
        )),
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

    #[test]
    fn prompt_timeout_stays_inside_the_chat_turn_deadline() {
        // ai_chat.rs wraps the WHOLE turn (including this prompt) in
        // TURN_TIMEOUT_SECS = 180 s. The inner budget must stay strictly
        // below it or the outer limit fires first and this module's
        // "prompt timed out" path — with its adapter cleanup — is
        // unreachable. Same invariant family as model_catalog's
        // BRIDGE_CATALOG_TIMEOUT test.
        assert!(PROMPT_TIMEOUT < Duration::from_secs(180));
        assert!(PROMPT_TIMEOUT > INIT_TIMEOUT + SESSION_TIMEOUT);
    }

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

    #[test]
    fn only_end_turn_is_a_successful_prompt_completion() {
        assert!(matches!(
            finish_adapter_turn(v1::StopReason::EndTurn, "answer".into()),
            AdapterOutcome::Answer(text) if text == "answer"
        ));
        for stop in [
            v1::StopReason::MaxTokens,
            v1::StopReason::MaxTurnRequests,
            v1::StopReason::Refusal,
            v1::StopReason::Cancelled,
        ] {
            assert!(matches!(
                finish_adapter_turn(stop, "partial".into()),
                AdapterOutcome::Failed(_)
            ));
        }
        assert!(matches!(
            finish_adapter_turn(v1::StopReason::EndTurn, "  ".into()),
            AdapterOutcome::Failed(_)
        ));
    }

    #[test]
    fn extracts_the_current_claude_acp_catalog_without_losing_versions() {
        // Exact model config reported by claude-agent-acp 0.62.0 with
        // Claude Code 2.1.220. The option values are selectors (some aliases),
        // while the live descriptions carry their resolved marketing versions.
        let options: Vec<v1::SessionConfigOption> =
            serde_json::from_value(serde_json::json!([{
                "id": "model",
                "name": "Model",
                "description": "AI model to use",
                "category": "model",
                "type": "select",
                "currentValue": "opus",
                "options": [
                    {
                        "value": "default",
                        "name": "Default (recommended)",
                        "description": "Opus 5 with 1M context · Best for everyday, complex tasks"
                    },
                    {
                        "value": "opus[1m]",
                        "name": "Opus (1M context)",
                        "description": "Opus 5 with 1M context · Best for everyday, complex tasks"
                    },
                    {
                        "value": "claude-fable-5[1m]",
                        "name": "Fable",
                        "description": "Fable 5 · Most capable for your hardest and longest-running tasks"
                    },
                    {
                        "value": "sonnet",
                        "name": "Sonnet",
                        "description": "Sonnet 5 · Efficient for routine tasks"
                    },
                    {
                        "value": "haiku",
                        "name": "Haiku",
                        "description": "Haiku 4.5 · Fastest for quick answers"
                    },
                    {
                        "value": "opus",
                        "name": "Opus",
                        "description": "Opus 5 · Best for everyday, complex tasks"
                    }
                ]
            }]))
            .unwrap();
        let catalog = catalog_from_config_options(Some(&options)).unwrap();
        assert_eq!(catalog.default_model.as_deref(), Some("opus"));
        assert_eq!(
            catalog.models,
            vec![
                cli_bridge::BridgeModel {
                    id: "default".into(),
                    label: Some("Default (recommended)".into()),
                    description: Some(
                        "Opus 5 with 1M context · Best for everyday, complex tasks".into()
                    ),
                },
                cli_bridge::BridgeModel {
                    id: "opus[1m]".into(),
                    label: Some("Opus (1M context)".into()),
                    description: Some(
                        "Opus 5 with 1M context · Best for everyday, complex tasks".into()
                    ),
                },
                cli_bridge::BridgeModel {
                    id: "claude-fable-5[1m]".into(),
                    label: Some("Fable".into()),
                    description: Some(
                        "Fable 5 · Most capable for your hardest and longest-running tasks".into()
                    ),
                },
                cli_bridge::BridgeModel {
                    id: "sonnet".into(),
                    label: Some("Sonnet".into()),
                    description: Some("Sonnet 5 · Efficient for routine tasks".into()),
                },
                cli_bridge::BridgeModel {
                    id: "haiku".into(),
                    label: Some("Haiku".into()),
                    description: Some("Haiku 4.5 · Fastest for quick answers".into()),
                },
                cli_bridge::BridgeModel {
                    id: "opus".into(),
                    label: Some("Opus".into()),
                    description: Some("Opus 5 · Best for everyday, complex tasks".into()),
                },
            ]
        );
    }

    #[test]
    fn model_option_id_is_a_backward_compatible_category_fallback() {
        let options = vec![v1::SessionConfigOption::select(
            "model",
            "Model",
            "sonnet",
            vec![v1::SessionConfigSelectOption::new("sonnet", "Sonnet")],
        )];
        let catalog = catalog_from_config_options(Some(&options)).unwrap();
        assert_eq!(catalog.models[0].id, "sonnet");
        assert_eq!(catalog.default_model.as_deref(), Some("sonnet"));
        assert!(catalog_from_config_options(None).is_err());
    }
}
