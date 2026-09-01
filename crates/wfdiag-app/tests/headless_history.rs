//! Scan history: the auto-save that rides on a completed scan, listing,
//! comparison, labels, tags, trends, and clearing.

mod support;

use std::time::Duration;
use support::boot;
use wfdiag_app::{AppCommand, AppEvent, HistoryEvent, HistoryRequest, ScanEvent};
use wfdiag_native_diagnostics::ScanKind;

/// Run one quick scan to completion, returning its session id and every event
/// the run produced. The auto-save acknowledgement lands in the same batch as
/// the finalization, so callers assert on this list rather than pumping again.
fn run_scan(harness: &mut support::Harness) -> (String, Vec<AppEvent>) {
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    let events = harness.pump_for("a completed scan", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))
    });
    let session_id = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Scan(ScanEvent::Finalized { session_id, .. }) => Some(session_id.clone()),
            _ => None,
        })
        .expect("the scan finalized");
    (session_id, events)
}

#[test]
fn a_completed_scan_is_auto_saved_and_then_listed() {
    let mut harness = boot("history_autosave");
    let (session_id, saved) = run_scan(&mut harness);

    assert!(saved.iter().any(|event| matches!(
        event,
        AppEvent::History(HistoryEvent::ScanSaved { scan_id }) if scan_id == &session_id
    )));
    assert!(saved.iter().any(|event| matches!(
        event,
        AppEvent::Scan(ScanEvent::Finalized {
            history: Some(Ok(())),
            ..
        })
    )));

    assert!(
        harness
            .service
            .dispatch(AppCommand::ListHistory)
            .is_accepted()
    );
    harness.pump_for("the history list", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Listed { .. }))
    });
    let summaries = &harness.service.snapshot().history.summaries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, session_id);
    assert_eq!(summaries[0].task_count, 3);
    assert_eq!(
        summaries[0].tags,
        ["Quick Scan"],
        "the scan kind is recorded as the history tag"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn two_saved_scans_can_be_compared_and_labelled() {
    let mut harness = boot("history_compare");
    let (first, first_events) = run_scan(&mut harness);
    assert!(
        first_events
            .iter()
            .any(|event| matches!(event, AppEvent::History(HistoryEvent::ScanSaved { .. })))
    );

    // Make the second scan differ so the comparison has something to report.
    harness.mocks.executor.script(
        "processor",
        wfdiag_app::ports::mock::TaskScript::failed("processor probe failed"),
    );
    let (second, second_events) = run_scan(&mut harness);
    assert!(second_events.iter().any(|event| matches!(
        event,
        AppEvent::History(HistoryEvent::ScanSaved { scan_id }) if scan_id == &second
    )));

    assert!(
        harness
            .service
            .dispatch(AppCommand::CompareHistory {
                current_id: second.clone(),
                previous_id: first.clone(),
                summary_only: false,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the comparison", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Compared { .. }))
    });
    let AppEvent::History(HistoryEvent::Compared { comparison }) = events
        .iter()
        .find(|event| matches!(event, AppEvent::History(HistoryEvent::Compared { .. })))
        .expect("a comparison arrived")
    else {
        unreachable!("filtered above")
    };
    assert_eq!(comparison.current_scan.id, second);
    assert_eq!(comparison.previous_scan.id, first);
    assert_eq!(
        comparison.new_failures.len(),
        1,
        "the reran task regressed between the two scans"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::SaveHistoryLabel {
                scan_id: second.clone(),
                label: Some("Baseline".to_string()),
            })
            .is_accepted()
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::SaveHistoryTags {
                scan_id: second.clone(),
                tags: vec!["stable".to_string()],
            })
            .is_accepted()
    );
    let events = harness.pump_until(
        |_, events| {
            events
                .iter()
                .any(|event| matches!(event, AppEvent::History(HistoryEvent::LabelSaved { .. })))
                && events
                    .iter()
                    .any(|event| matches!(event, AppEvent::History(HistoryEvent::TagsSaved { .. })))
        },
        "the label and tag writes",
    );
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::History(HistoryEvent::LabelSaved { scan_id }) if scan_id == &second
    )));

    harness.service.dispatch(AppCommand::ListHistory);
    harness.pump_for("the refreshed list", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Listed { .. }))
    });
    let stored = harness
        .service
        .snapshot()
        .history
        .summaries
        .iter()
        .find(|summary| summary.id == second)
        .expect("the labelled scan");
    assert_eq!(stored.label.as_deref(), Some("Baseline"));
    assert_eq!(stored.tags, ["stable"]);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn trends_and_clearing_operate_on_the_stored_scans() {
    let mut harness = boot("history_clear");
    // A trend only exists for a task that has failed at least once.
    harness.mocks.executor.script(
        "processor",
        wfdiag_app::ports::mock::TaskScript::failed("processor probe failed"),
    );
    let (_session_id, events) = run_scan(&mut harness);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::History(HistoryEvent::ScanSaved { .. })))
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::HistoryTrends { limit: 10 })
            .is_accepted()
    );
    harness.pump_for("the trends", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Trends { .. }))
    });
    let trends = &harness.service.snapshot().history.trends;
    assert!(
        trends
            .iter()
            .any(|trend| trend.task_id == "processor" && trend.failed == 1),
        "the failing task shows up in the trend window: {trends:?}"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::ClearHistory)
            .is_accepted()
    );
    harness.pump_for("the clear acknowledgement", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Cleared))
    });
    assert!(harness.service.snapshot().history.summaries.is_empty());
    assert!(harness.service.snapshot().history.trends.is_empty());
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_missing_scan_reports_a_typed_history_failure() {
    let mut harness = boot("history_missing");
    assert!(
        harness
            .service
            .dispatch(AppCommand::LoadHistoryScan {
                scan_id: "scan_does_not_exist".to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the failure", |event| {
        matches!(event, AppEvent::History(HistoryEvent::Failed { .. }))
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::History(HistoryEvent::Failed {
            request: HistoryRequest::Load,
            ..
        })
    )));
    assert!(harness.service.snapshot().history.error.is_some());
    harness.shutdown(Duration::from_secs(2));
}
