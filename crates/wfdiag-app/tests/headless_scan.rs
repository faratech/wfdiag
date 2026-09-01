//! The scan transaction end to end: quick scan, targeted rerun overlay,
//! rollback, cancellation, and the issue commit that rides on it.

mod support;

use std::time::Duration;
use support::{boot, boot_with};
use wfdiag_app::ports::mock::{MockPorts, TaskScript};
use wfdiag_app::{AppCommand, AppEvent, IssuesEvent, ScanEvent};
use wfdiag_native_diagnostics::ScanKind;

fn committed(events: &[AppEvent]) -> Option<(&String, usize, usize, bool)> {
    events.iter().find_map(|event| match event {
        AppEvent::Scan(ScanEvent::Committed {
            session_id,
            completed,
            errors,
            auto_save,
            ..
        }) => Some((session_id, *completed, *errors, *auto_save)),
        _ => None,
    })
}

#[test]
fn a_quick_scan_streams_progress_and_commits_catalog_ordered_evidence() {
    let mut harness = boot("quick_scan");
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartScan {
                kind: ScanKind::Quick
            })
            .is_accepted()
    );
    let events = harness.pump_for("the scan to commit", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Committed { .. }))
    });

    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::Scan(ScanEvent::Started {
                kind: ScanKind::Quick,
                ..
            })
        )),
        "the host learns the session started"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Scan(ScanEvent::TaskResult { .. }))),
        "per-task results stream while the scan runs"
    );
    let (session_id, completed, errors, auto_save) =
        committed(&events).expect("the scan committed");
    assert_eq!(completed, 3);
    assert_eq!(errors, 0);
    assert!(auto_save, "auto-save is on by default");

    let snapshot = harness.service.snapshot();
    assert_eq!(snapshot.scan.session_id.as_ref(), Some(session_id));
    let ids: Vec<&str> = snapshot
        .scan
        .results
        .iter()
        .map(|result| result.task_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["os_info", "processor", "logical_disk"],
        "results are ordered by the executor's catalog, not completion order"
    );
    assert_eq!(harness.mocks.executor.executed().len(), 3);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn issue_detection_commits_against_the_scanned_evidence() {
    let mut harness = boot("issue_commit");
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    let events = harness.pump_for("issue detection to answer", |event| {
        matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))
    });

    let (session_id, _, _, _) = committed(&events).expect("the scan committed first");
    let AppEvent::Issues(IssuesEvent::Updated {
        session_id: issue_session,
        issues,
    }) = events
        .iter()
        .find(|event| matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. })))
        .expect("issues were projected")
    else {
        unreachable!("filtered above")
    };
    assert_eq!(
        issue_session, session_id,
        "issues are tied to the committed scan, not to a later one"
    );
    assert!(!issues.is_empty(), "the catalog projects every issue row");
    assert_eq!(&harness.service.snapshot().issues, issues);
    assert!(harness.service.snapshot().issue_error.is_none());

    // Re-running detection against the same evidence is allowed and keeps the
    // same evidence identity.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RefreshIssues)
            .is_accepted()
    );
    let refreshed = harness.pump_for("the refreshed projection", |event| {
        matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))
    });
    assert!(refreshed.iter().any(|event| matches!(
        event,
        AppEvent::Issues(IssuesEvent::Updated { session_id, .. }) if session_id == issue_session
    )));
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_targeted_rerun_replaces_exactly_one_row_in_the_committed_scan() {
    let mut harness = boot("targeted_rerun");
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    let first = harness.pump_for("the base scan", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))
    });
    let (base_session, _, _, _) = committed(&first).expect("base scan committed");
    let base_session = base_session.clone();
    let base_outputs: Vec<String> = harness
        .service
        .snapshot()
        .scan
        .results
        .iter()
        .map(|result| result.output.clone())
        .collect();

    // Re-run one row, and make it fail this time.
    harness
        .mocks
        .executor
        .script("processor", TaskScript::failed("processor probe failed"));
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartTargetedScan {
                task_ids: vec!["processor".to_string()],
            })
            .is_accepted()
    );
    let events = harness.pump_for("the rerun to merge", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::TargetedCommitted { .. }))
    });
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::Scan(ScanEvent::TargetedCommitted { session_id, task_id })
                if session_id == &base_session && task_id == "processor"
        )),
        "the merged snapshot keeps the base session identity"
    );

    let snapshot = harness.service.snapshot();
    assert_eq!(snapshot.scan.results.len(), 3, "no row was added or lost");
    let replaced = snapshot
        .scan
        .results
        .iter()
        .find(|result| result.task_id == "processor")
        .expect("the target row");
    assert!(!replaced.success, "exactly the reran row changed");
    for result in &snapshot.scan.results {
        assert_eq!(result.session_id, base_session);
        if result.task_id != "processor" {
            assert!(
                base_outputs.contains(&result.output),
                "untouched rows keep their original output"
            );
        }
    }
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_failed_targeted_rerun_rolls_the_committed_scan_back_untouched() {
    let mut harness = boot("targeted_rollback");
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    harness.pump_for("the base scan", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))
    });
    let before = harness.service.snapshot().scan.clone();
    assert_eq!(before.results.len(), 3);

    // The task disappears from the executor between the scan and the rerun, so
    // `start_session` refuses it. That is the failure path the overlay must
    // roll back from: the committed scan has to survive it untouched.
    harness.mocks.executor.remove_task("processor");
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartTargetedScan {
                task_ids: vec!["processor".to_string()],
            })
            .is_accepted(),
        "the engine accepts the rerun; the runtime is what refuses it"
    );
    let events = harness.pump_for("the rerun to fail", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::StartFailed { .. }))
    });
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::Issues(IssuesEvent::Updated { .. }))),
        "a failed rerun never re-projects issues"
    );

    let after = &harness.service.snapshot().scan;
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.results.len(), before.results.len());
    assert_eq!(after.completed, before.completed);
    assert_eq!(after.errors, before.errors);
    for (left, right) in after.results.iter().zip(before.results.iter()) {
        assert_eq!(left.task_id, right.task_id);
        assert_eq!(left.output, right.output);
    }
    assert!(!harness.service.snapshot().scan_busy());

    // A rerun of a task that still exists works afterwards: the machine is
    // not wedged by the failed transaction.
    assert!(
        harness
            .service
            .dispatch(AppCommand::StartTargetedScan {
                task_ids: vec!["os_info".to_string()],
            })
            .is_accepted()
    );
    harness.pump_for("the recovery rerun", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::TargetedCommitted { .. }))
    });
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn stopping_a_scan_restores_the_previous_evidence() {
    let mut mocks = MockPorts::new();
    mocks.executor = wfdiag_app::ports::mock::ScriptedExecutor::with_tasks(&[
        "os_info",
        "processor",
        "logical_disk",
    ]);
    let mut harness = boot_with("scan_cancel", mocks);
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    harness.pump_for("the base scan", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))
    });
    let before = harness.service.snapshot().scan.clone();

    let started = harness.mocks.executor.hold("processor");
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Full,
    });
    // The run only launches once the host drains the session-started message,
    // so pump before waiting on the executor.
    harness.pump_for("the replacement scan to start", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Started { .. }))
    });
    started
        .recv_timeout(Duration::from_secs(5))
        .expect("the held task started");
    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelScan)
            .is_accepted()
    );
    // Wait for the runtime to acknowledge the stop before releasing the held
    // task, so the run really observes cancellation instead of racing it.
    harness.pump_for("the cancellation acknowledgement", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::CancelAcknowledged))
    });
    harness.mocks.executor.release();

    harness.pump_for("the stopped scan to settle", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Cancelled))
    });
    let after = &harness.service.snapshot().scan;
    assert_eq!(
        after.session_id, before.session_id,
        "the previous committed scan is visible again"
    );
    assert_eq!(after.results.len(), before.results.len());
    harness.shutdown(Duration::from_secs(2));
}
