//! The one-click AI scan report on the real report runtime.

mod support;

use std::time::Duration;
use support::boot_ai;
use wfdiag_app::ports::mock_ai::ScriptedTurn;
use wfdiag_app::{AppCommand, AppEvent, ReportEvent};
use wfdiag_native_ai_provider::AIProvider;

const BODY: &str = "## Health summary\nThe PC looks healthy.";

fn report_deltas(events: &[AppEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Report(ReportEvent::Delta { text }) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn script(harness: &support::Harness, turns: usize) {
    harness.mocks.ai.report.chat.script(
        AIProvider::Ollama,
        (0..turns).map(|_| ScriptedTurn::text(BODY)).collect(),
    );
}

#[test]
fn a_report_streams_and_finishes_once() {
    let mut harness = boot_ai("report_stream");
    script(&harness, 1);
    harness.commit_scan();

    assert!(
        harness
            .service
            .dispatch(AppCommand::GenerateReport {
                force_refresh: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the report to finish", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Report(ReportEvent::Started { .. }))),
        "the host learns which provider is writing the report"
    );
    assert_eq!(report_deltas(&events), BODY);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AppEvent::Report(ReportEvent::Done { .. })))
            .count(),
        1
    );
    let snapshot = harness.service.snapshot();
    assert_eq!(snapshot.ai.report.text.as_deref(), Some(BODY));
    assert!(!snapshot.ai.report.generating);
    assert!(!snapshot.ai.report.cached);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_second_report_for_the_same_evidence_is_served_inline_from_the_cache() {
    let mut harness = boot_ai("report_cached");
    // Only one scripted turn: a cache hit must not reach the provider again.
    script(&harness, 1);
    harness.commit_scan();

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: false,
    });
    harness.pump_for("the first report", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: false,
    });
    let events = harness.pump_for("the cached report", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });
    let cached = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Report(ReportEvent::Cached { report, .. }) => Some(report.as_str()),
            _ => None,
        })
        .expect("a cache hit returns the body inline");
    assert_eq!(cached, BODY);
    assert!(
        report_deltas(&events).is_empty(),
        "a cache hit streams nothing"
    );
    assert!(harness.service.snapshot().ai.report.cached);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_forced_regeneration_bypasses_the_cache_and_asks_the_provider_again() {
    let mut harness = boot_ai("report_force");
    script(&harness, 2);
    harness.commit_scan();

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: false,
    });
    harness.pump_for("the first report", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: true,
    });
    let events = harness.pump_for("the regenerated report", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });
    assert_eq!(
        report_deltas(&events),
        BODY,
        "a forced report streams instead of replaying the cache"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::Report(ReportEvent::Cached { .. })))
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn cancelling_a_report_ends_it_without_a_body() {
    let mut harness = boot_ai("report_cancel");
    script(&harness, 1);
    let _hold = harness.mocks.ai.report.chat.hold();
    harness.commit_scan();

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: false,
    });
    harness.pump_for("the report to start", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Started { .. }))
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelReport)
            .is_accepted()
    );
    let events = harness.pump_for("the cancellation", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Cancelled))
    });
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::Report(ReportEvent::Done { .. }))),
        "a cancelled report never also completes"
    );
    assert!(!harness.service.snapshot().ai.report.generating);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_new_scan_invalidates_the_report_it_described() {
    let mut harness = boot_ai("report_invalidated");
    script(&harness, 2);
    harness.commit_scan();

    let _ = harness.service.dispatch(AppCommand::GenerateReport {
        force_refresh: false,
    });
    harness.pump_for("the report", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Done { .. }))
    });
    assert!(harness.service.snapshot().ai.report.text.is_some());

    // A replacement scan invalidates the moment its transaction opens.
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartScan {
                kind: wfdiag_native_diagnostics::ScanKind::Quick,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the invalidation", |event| {
        matches!(event, AppEvent::Report(ReportEvent::Invalidated))
    });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Report(ReportEvent::Invalidated)))
    );
    assert!(
        harness.service.snapshot().ai.report.text.is_none(),
        "nothing stale stays on screen"
    );
    harness.shutdown(Duration::from_secs(2));
}
