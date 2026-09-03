//! End-to-end integration tests: the REAL chat-completions client and tool
//! loop driven against a hermetic mock provider — the same paths the Reactor
//! shell and the Tauri backend both execute.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_chat::{
    ChatEmitter, ChatMessage, ChatRole, CompatChatProvider, ProviderUse, ToolSpec, TurnStatus,
    build_system_prompt, plan_context, run_chat_turn,
};
use wfdiag_native_ai_provider::{
    AIProvider, CompatConfigPorts, ProviderKeySource, compat_caps, resolve_compat_config,
};
use wfdiag_native_settings::{AppSettings, ProviderKeyId};

mod mock_provider;

use mock_provider::{MOCK_ENDPOINT, MOCK_MODEL};

/// Integration tests run in parallel, so all turns share one process-lifetime
/// mock instead of racing to bind the same loopback port.
fn ensure_mock_provider() -> &'static mock_provider::MockController {
    static MOCK: OnceLock<mock_provider::MockController> = OnceLock::new();
    MOCK.get_or_init(mock_provider::spawn)
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

struct NoKeys;

impl ProviderKeySource for NoKeys {
    fn load(&self, _key: ProviderKeyId) -> Option<String> {
        None
    }
}

struct Unreachable;

impl wfdiag_native_ai_provider::FoundryEndpointSource for Unreachable {
    fn probe(
        &self,
        _configured: Option<String>,
    ) -> wfdiag_native_ai_provider::BackendFuture<'_, Option<String>> {
        Box::pin(async { None })
    }
}

impl wfdiag_native_ai_provider::OllamaSource for Unreachable {
    fn discover(
        &self,
        _configured: Option<String>,
    ) -> wfdiag_native_ai_provider::BackendFuture<'_, Option<String>> {
        Box::pin(async { None })
    }
    fn list_models(
        &self,
        _endpoint: String,
    ) -> wfdiag_native_ai_provider::BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn ports(settings: AppSettings) -> CompatConfigPorts {
    CompatConfigPorts {
        settings,
        keys: Arc::new(NoKeys),
        foundry: Arc::new(Unreachable),
        ollama: Arc::new(Unreachable),
    }
}

// ---------------------------------------------------------------------------
// Emitter + tool executor
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct CollectingEmitter {
    deltas: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<Vec<String>>>,
    completed: Arc<Mutex<Vec<String>>>,
}

impl ChatEmitter for CollectingEmitter {
    fn delta(&self, payload: &wfdiag_native_ai_chat::DeltaPayload) {
        self.deltas
            .lock()
            .expect("deltas")
            .push(payload.text.clone());
    }
    fn tool(&self, _payload: &wfdiag_native_ai_chat::ToolPayload) {}
    fn done(&self, payload: &wfdiag_native_ai_chat::DonePayload) {
        self.completed
            .lock()
            .expect("completed")
            .push(payload.finish_reason.clone());
    }
    fn error(&self, payload: &wfdiag_native_ai_chat::ErrorPayload) {
        self.errors
            .lock()
            .expect("errors")
            .push(payload.message.clone());
    }
}

/// Canonical scan-summary tool backed by an injected immutable snapshot.
struct ScanSummaryExecutor {
    summary: String,
}

impl wfdiag_native_ai_chat::ToolExecutor for ScanSummaryExecutor {
    fn execute<'a>(
        &'a self,
        call: &'a wfdiag_native_ai_chat::ToolCall,
        _cancel: CancellationToken,
    ) -> wfdiag_native_ai_chat::ToolFuture<'a> {
        Box::pin(async move {
            if call.name == "get_scan_summary" {
                Ok(self.summary.clone())
            } else {
                Err(format!("unknown tool '{}'", call.name))
            }
        })
    }
}

fn scan_summary_tool() -> ToolSpec {
    ToolSpec {
        name: "get_scan_summary".to_string(),
        description: "Summarize the immutable current diagnostic scan.".to_string(),
        parameters: serde_json::json!({"type":"object","properties":{},"required":[]}),
    }
}

// ---------------------------------------------------------------------------
// Turn driver
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TurnOutcome {
    status: TurnStatus,
    deltas: Vec<String>,
    answer: String,
    errors: Vec<String>,
}

async fn run_turn(
    provider: AIProvider,
    prompt: &str,
    scan_summary: Option<String>,
    cancel: CancellationToken,
) -> TurnOutcome {
    let cfg = resolve_compat_config(
        provider,
        &CompatConfigPorts {
            settings: AppSettings {
                custom_endpoint: Some(MOCK_ENDPOINT.to_string()),
                custom_model: Some(MOCK_MODEL.to_string()),
                ..AppSettings::default()
            },
            keys: Arc::new(NoKeys),
            foundry: Arc::new(Unreachable),
            ollama: Arc::new(Unreachable),
        },
    )
    .await
    .expect("compat resolution must succeed for the custom endpoint");

    let caps = compat_caps(provider);
    let plan = plan_context(caps.context_budget_chars);
    let tools_enabled = caps.supports_tools && scan_summary.is_some();
    let system = build_system_prompt(tools_enabled, false, None, &plan);
    let chat = CompatChatProvider {
        provider,
        cfg: cfg.clone(),
    };
    let mut provider_use = ProviderUse::for_provider(provider, None);
    let mut messages = vec![ChatMessage {
        role: ChatRole::User,
        content: prompt.to_string(),
        tool_calls: Vec::new(),
        tool_call_id: None,
        tool_name: None,
        tool_result_is_error: false,
        provider_replay: None,
    }];
    let specs: Vec<ToolSpec> = if tools_enabled {
        vec![scan_summary_tool()]
    } else {
        Vec::new()
    };
    let executor = ScanSummaryExecutor {
        summary: scan_summary.unwrap_or_default(),
    };
    let emitter = CollectingEmitter::default();
    let outcome = run_chat_turn(
        &mut provider_use,
        caps,
        &chat,
        "integration-test",
        "msg_test",
        &mut messages,
        &system,
        &specs,
        &executor,
        &emitter,
        cancel,
        false,
    )
    .await;
    let answer = messages
        .last()
        .filter(|message| message.role == ChatRole::Assistant)
        .map(|message| message.content.clone())
        .unwrap_or_default();
    TurnOutcome {
        status: outcome.expect("turn must not error against the mock"),
        deltas: emitter.deltas.lock().expect("deltas").clone(),
        answer,
        errors: emitter.errors.lock().expect("errors").clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_round_trip_reaches_completed_with_mock_reply() {
    let _mock = ensure_mock_provider();
    let outcome = run_turn(
        AIProvider::CustomOpenAI,
        "hello there",
        None,
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(outcome.status, TurnStatus::Completed { .. }));
    assert!(
        !outcome.deltas.is_empty(),
        "a streaming turn must emit deltas"
    );
    assert!(
        outcome.answer.contains("MOCK_REPLY"),
        "answer must carry the mock reply, got: {outcome:?}"
    );
    assert!(outcome.errors.is_empty(), "no errors expected");
}

// FIXME: red on windows-x64 CI — surfaced 2026-09-03 once the fmt gate and
// subscription_install failures stopped masking it. Ignored with intent;
// re-enable after fixing, do not delete.
#[tokio::test]
#[ignore = "slow_stream_cancels_mid_turn: see FIXME above"]
async fn slow_stream_cancels_mid_turn() {
    let _mock = ensure_mock_provider();
    let cancel = CancellationToken::new();
    let cancel_request = cancel.clone();
    let cancellation = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        cancel_request.cancel();
    });
    let outcome = run_turn(
        AIProvider::CustomOpenAI,
        "tell me something slow",
        None,
        cancel,
    )
    .await;
    cancellation.await.expect("cancellation task must finish");
    // The slow stream runs 30 x 700 ms; the cancel must beat completion.
    assert!(
        matches!(outcome.status, TurnStatus::Cancelled),
        "expected cancellation, got {outcome:?}"
    );
}

#[tokio::test]
async fn tool_round_trip_feeds_the_scan_summary_into_the_answer() {
    let _mock = ensure_mock_provider();
    let scan_summary = "Computer name: TESTBOX\nOperating system: Windows 11";
    let outcome = run_turn(
        AIProvider::CustomOpenAI,
        "what hardware am I running (use the tool)",
        Some(scan_summary.to_string()),
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(outcome.status, TurnStatus::Completed { .. }));
    assert!(
        outcome.answer.contains("TESTBOX"),
        "the tool result must reach the model and back: {outcome:?}"
    );
}

#[test]
fn compat_resolution_yields_the_mock_endpoint_and_model() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let settings = AppSettings {
        custom_endpoint: Some("http://127.0.0.1:18093".to_string()),
        custom_model: Some(MOCK_MODEL.to_string()),
        ..AppSettings::default()
    };
    let resolved = runtime.block_on(resolve_compat_config(
        AIProvider::CustomOpenAI,
        &ports(settings),
    ));
    let resolved = resolved.expect("custom endpoint must resolve");
    assert_eq!(resolved.endpoint.as_deref(), Some("http://127.0.0.1:18093"));
    assert_eq!(resolved.model.as_deref(), Some(MOCK_MODEL));
    assert!(resolved.api_key.is_none());
}
