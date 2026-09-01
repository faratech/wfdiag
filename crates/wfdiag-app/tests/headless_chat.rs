//! Agentic chat end to end, on the real chat runtime.
//!
//! Only the provider transport is scripted: the worker thread, the tool loop,
//! the streaming emitter, the cancellation token, and the `Auto` fallback
//! decision are all the shipping code.

mod support;

use std::time::Duration;
use support::{ai_mocks, boot_ai, boot_ai_with};
use wfdiag_app::ports::mock_ai::ScriptedTurn;
use wfdiag_app::{AppCommand, AppEvent, ChatEvent, ScanEvent};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_settings::CloudFallbackPolicy;

fn deltas(events: &[AppEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Chat(ChatEvent::Delta { text }) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn terminals(events: &[AppEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                AppEvent::Chat(
                    ChatEvent::Done { .. } | ChatEvent::Failed { .. } | ChatEvent::Cancelled
                )
            )
        })
        .count()
}

#[test]
fn a_turn_streams_coalesced_deltas_and_ends_exactly_once() {
    let mut harness = boot_ai("chat_stream");
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![ScriptedTurn::text("Your C: drive is nearly full.")],
    );
    harness.commit_scan();

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatSend {
                prompt: "why is my disk full".to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the turn to finish", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Done { .. }))
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Chat(ChatEvent::Started { .. }))),
        "the host learns which provider answered"
    );
    let streamed: String = deltas(&events).concat();
    assert_eq!(streamed, "Your C: drive is nearly full.");
    assert_eq!(
        terminals(&events),
        1,
        "exactly one terminal event ends a turn"
    );
    assert_eq!(
        harness.service.snapshot().ai.chat.text,
        "Your C: drive is nearly full."
    );
    assert!(!harness.service.snapshot().ai.chat.streaming);
    assert_eq!(harness.mocks.ai.chat.resolved(), [AIProvider::Ollama]);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn cancelling_mid_stream_keeps_what_arrived_and_ends_the_turn_once() {
    let mut harness = boot_ai("chat_cancel");
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![ScriptedTurn::text("partial answer")],
    );
    let _hold = harness.mocks.ai.chat.hold();
    harness.commit_scan();

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatSend {
                prompt: "why is my disk full".to_string(),
            })
            .is_accepted()
    );
    let streaming = harness.pump_for("the first delta", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Delta { .. }))
    });
    assert!(!deltas(&streaming).is_empty());

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatCancel)
            .is_accepted()
    );
    let events = harness.pump_for("the cancellation", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Cancelled))
    });
    assert_eq!(terminals(&events), 1);
    assert!(!harness.service.snapshot().ai.chat.streaming);
    assert!(
        harness.service.snapshot().ai.chat.text.contains("partial"),
        "text that already streamed is not thrown away"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_clean_local_failure_asks_before_crossing_into_the_cloud_and_allow_resumes_the_turn() {
    let mut harness = boot_ai("chat_consent_allow");
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![ScriptedTurn::failure("Ollama is not running")],
    );
    harness
        .mocks
        .ai
        .chat
        .script(AIProvider::OpenAI, vec![ScriptedTurn::text("cloud answer")]);
    harness.commit_scan();

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatSend {
                prompt: "why is my disk full".to_string(),
            })
            .is_accepted()
    );
    let asked = harness.pump_for("the consent prompt", |event| {
        matches!(
            event,
            AppEvent::Chat(ChatEvent::CloudFallbackRequired { .. })
        )
    });
    assert_eq!(
        terminals(&asked),
        0,
        "asking is not a terminal event: the turn is still alive"
    );
    let prompt = harness
        .service
        .snapshot()
        .ai
        .chat
        .cloud_fallback
        .clone()
        .expect("the prompt is in the read model");
    assert_eq!(prompt.candidate, "openai");
    assert_eq!(prompt.reason, "Ollama is not running");
    assert!(!prompt.saving);

    assert!(
        harness
            .service
            .dispatch(AppCommand::CloudFallbackDecision { allow: true })
            .is_accepted()
    );
    let events = harness.pump_for("the resumed turn", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Done { .. }))
    });
    assert_eq!(deltas(&events).concat(), "cloud answer");
    assert_eq!(terminals(&events), 1);

    let done = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Chat(ChatEvent::Done { provider_use, .. }) => Some(provider_use),
            _ => None,
        })
        .expect("the turn completed");
    assert_eq!(done.provider_id, "openai");
    assert_eq!(
        done.fallback_from.as_deref(),
        Some("ollama"),
        "attribution stays pinned to the provider the user actually chose"
    );
    assert_eq!(
        harness.service.snapshot().settings.cloud_fallback_policy,
        CloudFallbackPolicy::Allow,
        "Allow is remembered"
    );
    assert_eq!(
        harness.mocks.ai.chat.resolved(),
        [AIProvider::Ollama, AIProvider::OpenAI]
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn never_persists_the_refusal_and_ends_the_turn_without_reaching_the_cloud() {
    let mut harness = boot_ai("chat_consent_never");
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![ScriptedTurn::failure("Ollama is not running")],
    );
    harness
        .mocks
        .ai
        .chat
        .script(AIProvider::OpenAI, vec![ScriptedTurn::text("never sent")]);
    harness.commit_scan();

    let _ = harness.service.dispatch(AppCommand::ChatSend {
        prompt: "why is my disk full".to_string(),
    });
    harness.pump_for("the consent prompt", |event| {
        matches!(
            event,
            AppEvent::Chat(ChatEvent::CloudFallbackRequired { .. })
        )
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::CloudFallbackDecision { allow: false })
            .is_accepted()
    );
    let events = harness.pump_for("the refusal", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Failed { .. }))
    });
    assert_eq!(terminals(&events), 1);
    assert_eq!(
        harness.service.snapshot().settings.cloud_fallback_policy,
        CloudFallbackPolicy::Never,
        "Never is remembered so the next local failure does not ask again"
    );
    assert_eq!(
        harness.mocks.ai.chat.resolved(),
        [AIProvider::Ollama],
        "the cloud provider was never contacted"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_tool_call_reads_the_real_committed_scan_evidence() {
    let mocks = ai_mocks();
    mocks.executor.script(
        "os_info",
        wfdiag_app::ports::mock::TaskScript::ok(r#"{"os":"Windows 11","build":26100}"#),
    );
    let mut harness = boot_ai_with("chat_tools", mocks);
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![
            ScriptedTurn::tool("call-1", "get_scan_summary", serde_json::json!({})),
            ScriptedTurn::text("Your scan looks healthy."),
        ],
    );
    harness.commit_scan();

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatSend {
                prompt: "summarise my scan".to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the tool round trip", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Done { .. }))
    });

    // The first activity announces the call; the last carries its result.
    let activity = events
        .iter()
        .rev()
        .find_map(|event| match event {
            AppEvent::Chat(ChatEvent::ToolActivity { activity, .. }) => Some(activity),
            _ => None,
        })
        .expect("the model's tool call is reported as activity");
    assert_eq!(activity.tool, "get_scan_summary");
    let preview = activity
        .result_preview
        .as_deref()
        .expect("a completed tool carries its result preview");
    assert!(
        preview.contains("SCAN_SCOPE"),
        "the tool answered from the committed scan, not from an empty snapshot: {preview}"
    );
    assert_eq!(deltas(&events).concat(), "Your scan looks healthy.");
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_two_action_shaped_tools_only_ever_ask_and_never_act() {
    let mut harness = boot_ai("chat_tool_requests");
    harness.mocks.ai.chat.script(
        AIProvider::Ollama,
        vec![
            ScriptedTurn::tool(
                "call-1",
                "stage_remediation",
                // A maintenance action is the only kind the model may stage
                // without naming a detected issue.
                serde_json::json!({"remediation_id": "flush_dns"}),
            ),
            ScriptedTurn::tool(
                "call-2",
                "request_full_scan",
                serde_json::json!({"reason": "the quick scan skipped driver evidence"}),
            ),
            ScriptedTurn::text("Shall I go ahead?"),
        ],
    );
    harness.commit_scan();
    let scans_before = harness.mocks.executor.executed().len();

    assert!(
        harness
            .service
            .dispatch(AppCommand::ChatSend {
                prompt: "can you fix my disk".to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the turn to finish", |event| {
        matches!(event, AppEvent::Chat(ChatEvent::Done { .. }))
    });

    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::Chat(ChatEvent::ProposalStaged { remediation_id, .. })
                if remediation_id == "flush_dns"
        )),
        "staging is a request the user still has to approve"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Chat(ChatEvent::FullScanRequested { .. }))),
        "a Full Scan is requested, never started"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::Scan(ScanEvent::Started { .. }))),
        "no scan started behind the user's back"
    );
    assert_eq!(
        harness.mocks.executor.executed().len(),
        scans_before,
        "no diagnostic ran"
    );
    assert!(
        harness.mocks.ai.actions.executed().is_empty(),
        "the tool registry is read-only: nothing reached catalog execution"
    );
    assert_eq!(
        harness.service.snapshot().ai.chat.proposals.len(),
        1,
        "the staged request is visible to the host as a request"
    );
    harness.shutdown(Duration::from_secs(2));
}
