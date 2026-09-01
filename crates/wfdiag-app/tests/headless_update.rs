//! The GitHub update channel: every outcome, and the once-a-day throttle.

mod support;

use std::time::Duration;
use support::boot_with;
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{AppCommand, AppEvent, UpdateCheckReason, UpdateEvent};
use wfdiag_native_update::{ReleaseResponse, UpdateFailure, UpdateOutcome};

fn checked_outcome(events: &[AppEvent]) -> Option<UpdateOutcome> {
    events.iter().rev().find_map(|event| match event {
        AppEvent::Update(UpdateEvent::Checked(outcome)) => Some((**outcome).clone()),
        _ => None,
    })
}

fn check(harness: &mut support::Harness) -> UpdateOutcome {
    assert!(
        harness
            .service
            .dispatch(AppCommand::CheckForUpdates {
                reason: UpdateCheckReason::Manual,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the update check", |event| {
        matches!(event, AppEvent::Update(UpdateEvent::Checked(_)))
    });
    checked_outcome(&events).expect("a check completed")
}

#[test]
fn a_store_install_never_contacts_github() {
    let mocks = MockPorts::new();
    mocks.signature.set_store_install(true);
    let mut harness = boot_with("update_store", mocks);
    assert_eq!(check(&mut harness), UpdateOutcome::Silent);
    assert!(
        harness.mocks.release_http.requests().is_empty(),
        "a Store install makes no request at all"
    );
    assert!(
        harness.mocks.update_throttle.last_run().is_none(),
        "a silent channel does not consume the daily throttle"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn an_older_published_release_reports_up_to_date() {
    let mut mocks = MockPorts::new();
    mocks.current_version = "2.5.8".to_string();
    mocks
        .release_http
        .set_release("v2.5.0", "https://example.invalid/tag/v2.5.0");
    let mut harness = boot_with("update_current", mocks);
    assert_eq!(check(&mut harness), UpdateOutcome::UpToDate);
    assert_eq!(harness.mocks.release_http.requests().len(), 1);
    assert!(
        harness.mocks.update_throttle.last_run().is_some(),
        "a completed check records the throttle timestamp"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_newer_published_release_is_offered_to_the_host() {
    let mut mocks = MockPorts::new();
    mocks.current_version = "2.5.8".to_string();
    mocks.release_http.set_release(
        "v9.9.9",
        "https://github.com/faratech/wfdiag/releases/tag/v9.9.9",
    );
    let mut harness = boot_with("update_available", mocks);
    let outcome = check(&mut harness);
    let UpdateOutcome::Available(update) = outcome else {
        panic!("expected an available release, got {outcome:?}")
    };
    assert_eq!(update.version, "9.9.9");
    assert_eq!(
        harness
            .service
            .snapshot()
            .update
            .available
            .as_ref()
            .map(|info| info.version.as_str()),
        Some("9.9.9"),
        "the read model carries the offer"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn transport_and_status_failures_are_distinguishable_from_being_current() {
    let mocks = MockPorts::new();
    mocks
        .release_http
        .set_response(Err("dns lookup failed".to_string()));
    let mut harness = boot_with("update_failed", mocks);
    assert_eq!(
        check(&mut harness),
        UpdateOutcome::Failed(UpdateFailure::Transport("dns lookup failed".to_string())),
        "an offline check is not the same fact as being up to date"
    );

    harness.mocks.release_http.set_response(Ok(ReleaseResponse {
        status: 403,
        body: Vec::new(),
    }));
    assert_eq!(
        check(&mut harness),
        UpdateOutcome::Failed(UpdateFailure::Status(403))
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_startup_check_is_throttled_but_a_manual_check_is_not() {
    let mut mocks = MockPorts::new();
    mocks.current_version = "2.5.8".to_string();
    mocks
        .release_http
        .set_release("v2.5.0", "https://example.invalid/tag/v2.5.0");
    let mut harness = boot_with("update_throttle", mocks);
    // A check ran a minute ago.
    let now = wfdiag_app::EnvironmentPort::now_millis(&harness.mocks.environment);
    harness
        .mocks
        .update_throttle
        .set_last_run(Some(now - 60_000));

    let outcome = harness.service.dispatch(AppCommand::CheckForUpdates {
        reason: UpdateCheckReason::Startup,
    });
    assert!(
        matches!(outcome, wfdiag_app::DispatchOutcome::Ignored { .. }),
        "the passive check honours the daily throttle"
    );
    let events = harness.pump_briefly();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Update(UpdateEvent::Throttled))),
        "the host is told why nothing happened"
    );
    assert!(harness.mocks.release_http.requests().is_empty());

    // The user asking explicitly still checks.
    assert_eq!(check(&mut harness), UpdateOutcome::UpToDate);
    assert_eq!(harness.mocks.release_http.requests().len(), 1);
    harness.shutdown(Duration::from_secs(2));
}
