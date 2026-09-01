//! Export rendering and host identity.

mod support;

use std::time::Duration;
use support::{boot, boot_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{AppCommand, AppEvent, ExportEvent, ScanEvent, SystemEvent};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_export::{ExportPayload, ExportRequestKind, ReportFormat};

#[test]
fn a_committed_scan_renders_a_json_report() {
    let mut harness = boot("export_report");
    harness.service.dispatch(AppCommand::StartScan {
        kind: ScanKind::Quick,
    });
    harness.pump_for("the scan", |event| {
        matches!(event, AppEvent::Scan(ScanEvent::Finalized { .. }))
    });

    assert!(
        harness
            .service
            .dispatch(AppCommand::ExportResults {
                kind: Box::new(ExportRequestKind::Report {
                    format: ReportFormat::Json,
                    include_raw: true,
                }),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the rendered report", |event| {
        matches!(event, AppEvent::Export(ExportEvent::Completed { .. }))
    });
    let AppEvent::Export(ExportEvent::Completed { payload }) = events
        .iter()
        .find(|event| matches!(event, AppEvent::Export(ExportEvent::Completed { .. })))
        .expect("a payload arrived")
    else {
        unreachable!("filtered above")
    };
    let ExportPayload::Report(json) = payload.as_ref() else {
        panic!("expected a report payload, got {payload:?}")
    };
    assert!(
        json.contains("os_info"),
        "the report renders the committed evidence"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn host_identity_and_architecture_are_probed_at_start() {
    let harness = boot("system_identity");
    let snapshot = harness.service.snapshot();
    assert_eq!(
        snapshot
            .system_info
            .as_ref()
            .map(|info| info.computer_name.as_str()),
        Some("TEST-PC")
    );
    assert!(snapshot.is_admin());
    assert_eq!(
        snapshot
            .architecture
            .as_ref()
            .map(|arch| arch.process_architecture_name.as_str()),
        Some("x64")
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_failing_identity_probe_is_reported_without_stopping_the_service() {
    let mocks = MockPorts::new();
    mocks
        .system
        .set_info(Err("the registry is unavailable".to_string()));
    let mut harness = boot_with_failing_identity("system_failure", mocks);
    assert!(harness.service.snapshot().system_error.is_some());
    // Architecture still answered, and the service still accepts commands.
    assert!(harness.service.snapshot().architecture.is_some());
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestArchitecture)
            .is_accepted()
    );
    harness.pump_for("the architecture probe", |event| {
        matches!(event, AppEvent::System(SystemEvent::Architecture(_)))
    });
    harness.shutdown(Duration::from_secs(2));
}

/// `boot` waits for host identity, which never arrives when the probe fails,
/// so this variant waits for the failure instead.
fn boot_with_failing_identity(label: &str, mocks: MockPorts) -> support::Harness {
    let mut harness = support::start_with(label, mocks, |config| config);
    assert!(
        harness
            .service
            .dispatch(AppCommand::Start {
                startup_scan: false
            })
            .is_accepted()
    );
    harness.startup_events = harness.pump_for("the identity failure", |event| {
        matches!(event, AppEvent::System(SystemEvent::Failed { .. }))
    });
    harness
}

#[test]
fn an_elevation_request_is_forwarded_to_the_host_port() {
    let mocks = MockPorts::new();
    mocks.elevation.set_outcome(Ok(true));
    let mut harness = boot_with("elevation", mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::RestartAsAdmin)
            .is_accepted()
    );
    let events = harness.pump_for("the elevation result", |event| {
        matches!(
            event,
            AppEvent::System(SystemEvent::ElevationAttempted { .. })
        )
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::System(SystemEvent::ElevationAttempted { restarted: true })
    )));
    assert_eq!(harness.mocks.elevation.requests(), 1);
    harness.shutdown(Duration::from_secs(2));
}
