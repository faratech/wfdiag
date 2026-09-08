//! The guards themselves: a superseded reply is dropped, and a reply that
//! never lands becomes a typed timeout instead of a hang.

mod support;

use std::time::Duration;
use support::{boot, boot_with, start_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::ports::monitor::ProcessQuery;
use wfdiag_app::{AppCommand, AppEvent, MonitorEvent, SystemEvent, WorkerKind};

#[test]
fn a_superseded_reply_is_dropped_and_only_the_newest_updates_the_snapshot() {
    let mut harness = boot("guards_stale_reply");
    // Two probes are issued without draining in between, so both replies are
    // waiting when the host next drains. Only the newest may be applied.
    harness
        .mocks
        .system
        .set_info(Ok(wfdiag_native_system::SystemInfo {
            computer_name: "STALE-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: false,
        }));
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestSystemInfo)
            .is_accepted()
    );
    harness
        .mocks
        .system
        .set_info(Ok(wfdiag_native_system::SystemInfo {
            computer_name: "CURRENT-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: true,
        }));
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestSystemInfo)
            .is_accepted()
    );

    let events = harness.pump_for("the identity probes", |event| {
        matches!(event, AppEvent::System(SystemEvent::Info(_)))
    });
    let delivered: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::System(SystemEvent::Info(info)) => Some(info.computer_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        delivered,
        ["CURRENT-PC"],
        "the older probe's answer is dropped, not merely overwritten"
    );
    assert_eq!(
        harness
            .service
            .snapshot()
            .system_info
            .as_ref()
            .map(|info| info.computer_name.as_str()),
        Some("CURRENT-PC")
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_superseded_connection_reply_is_dropped_and_only_the_newest_updates_the_snapshot() {
    use wfdiag_app::ports::monitor::NetworkConnection;

    fn connection(local_addr: &str, status: &str) -> NetworkConnection {
        NetworkConnection {
            protocol: "TCP".to_string(),
            local_addr: local_addr.to_string(),
            remote_addr: "127.0.0.1:5001".to_string(),
            status: status.to_string(),
        }
    }

    let mocks = MockPorts::new();
    mocks
        .monitor
        .set_connections(vec![connection("127.0.0.1:5000", "ESTABLISHED")]);
    let mut harness = boot_with("guards_stale_connections", mocks);

    // Two connection requests are issued without draining in between, so both
    // replies are waiting when the host next drains (#198: the monitor port
    // answers each request on its own thread, so an older reply can land
    // after a newer one was issued). Only the newest may be applied.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestNetworkConnections)
            .is_accepted()
    );
    harness.mocks.monitor.set_connections(vec![
        connection("127.0.0.1:5000", "ESTABLISHED"),
        connection("127.0.0.1:5353", "BOUND"),
    ]);
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestNetworkConnections)
            .is_accepted()
    );

    let events = harness.pump_for("the connection lists", |event| {
        matches!(
            event,
            AppEvent::Monitor(MonitorEvent::NetworkConnections(_))
        )
    });
    let delivered: Vec<usize> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Monitor(MonitorEvent::NetworkConnections(connections)) => {
                Some(connections.len())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        delivered,
        [2],
        "the older reply is dropped, not merely overwritten"
    );
    assert_eq!(
        harness
            .service
            .snapshot()
            .monitor
            .connections
            .as_ref()
            .map(Vec::len),
        Some(2)
    );
    // A superseded reply is a routine race, not a failure.
    assert!(harness.service.snapshot().monitor.error.is_none());
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_reply_that_never_lands_becomes_a_typed_timeout() {
    let mocks = MockPorts::new();
    mocks.monitor.stall_process_queries(true);
    let mut harness = start_with("guards_timeout", mocks, |config| {
        config.with_reply_timeout(Duration::from_millis(50))
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::Start {
                startup_scan: false
            })
            .is_accepted()
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProcessPage(ProcessQuery::default()))
            .is_accepted()
    );

    let events = harness.pump_for("the reply deadline", |event| {
        matches!(event, AppEvent::ReplyTimedOut { .. })
    });
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::ReplyTimedOut {
                worker: WorkerKind::Monitor,
                ..
            }
        )),
        "the host learns which worker owed the answer"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Monitor(MonitorEvent::Unavailable { .. }))),
        "the domain also reports the failure in its own terms"
    );
    // The service is still usable afterwards.
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestSystemInfo)
            .is_accepted()
    );
    harness.pump_for("a later request", |event| {
        matches!(event, AppEvent::System(SystemEvent::Info(_)))
    });
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn visibility_and_page_demand_preserve_the_user_pause() {
    let mut harness = boot("monitor_demand");
    // Visibility must not invent demand for a non-live page.
    harness
        .service
        .dispatch(AppCommand::SetMonitorDemand { active: false });
    harness
        .service
        .dispatch(AppCommand::WindowVisibility { visible: false });
    harness
        .service
        .dispatch(AppCommand::WindowVisibility { visible: true });
    assert!(harness.mocks.monitor.control().is_paused());
    harness
        .service
        .dispatch(AppCommand::SetMonitorDemand { active: true });
    assert!(!harness.mocks.monitor.control().is_paused());

    // Neither navigation nor a minimize/restore may clear a user pause.
    harness
        .service
        .dispatch(AppCommand::SetMonitorPaused { paused: true });
    harness
        .service
        .dispatch(AppCommand::SetMonitorDemand { active: false });
    harness
        .service
        .dispatch(AppCommand::WindowVisibility { visible: false });
    harness
        .service
        .dispatch(AppCommand::SetMonitorDemand { active: true });
    harness
        .service
        .dispatch(AppCommand::WindowVisibility { visible: true });
    assert!(harness.mocks.monitor.control().is_paused());
    harness
        .service
        .dispatch(AppCommand::SetMonitorPaused { paused: false });
    assert!(!harness.mocks.monitor.control().is_paused());
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn live_monitoring_streams_samples_pages_and_connections() {
    use wfdiag_app::ports::monitor::{NetworkConnection, ProcessPage, ProcessRow};

    let mocks = MockPorts::new();
    mocks.monitor.set_page(ProcessPage {
        captured_at: 1_700_000_000,
        total: 1,
        offset: 0,
        limit: 100,
        items: vec![ProcessRow {
            pid: 42,
            parent_pid: 1,
            name: "wfdiag.exe".to_string(),
            cpu_percent: 3.5,
            memory_percent: 1.0,
            memory_mb: 128.0,
            virtual_memory_mb: 512.0,
            gpu_percent: None,
            gpu_memory_mb: None,
            npu_percent: None,
            npu_memory_mb: None,
            cpu_time_secs: 12,
            start_time: 1_699_999_000,
            status: "Running".to_string(),
            thread_count: 8,
            handle_count: 200,
            priority: 8,
            io_read_bytes: 1024,
            io_write_bytes: 2048,
        }],
    });
    mocks.monitor.set_connections(vec![NetworkConnection {
        protocol: "TCP".to_string(),
        local_addr: "127.0.0.1:5000".to_string(),
        remote_addr: "127.0.0.1:5001".to_string(),
        status: "ESTABLISHED".to_string(),
    }]);
    let mut harness = boot_with("monitor", mocks);

    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProcessPage(ProcessQuery::default()))
            .is_accepted()
    );
    harness.pump_for("the process page", |event| {
        matches!(event, AppEvent::Monitor(MonitorEvent::ProcessPage(_)))
    });
    assert_eq!(
        harness
            .service
            .snapshot()
            .monitor
            .process_page
            .as_ref()
            .map(|page| page.items.len()),
        Some(1)
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestNetworkConnections)
            .is_accepted()
    );
    harness.pump_for("the connection list", |event| {
        matches!(
            event,
            AppEvent::Monitor(MonitorEvent::NetworkConnections(_))
        )
    });
    assert_eq!(
        harness
            .service
            .snapshot()
            .monitor
            .connections
            .as_ref()
            .map(Vec::len),
        Some(1)
    );

    // Hiding the window pauses sampling; showing it resumes.
    assert!(
        harness
            .service
            .dispatch(AppCommand::WindowVisibility { visible: false })
            .is_accepted()
    );
    assert!(harness.mocks.monitor.control().is_paused());
    assert!(harness.service.snapshot().monitor.paused);
    assert!(
        harness
            .service
            .dispatch(AppCommand::WindowVisibility { visible: true })
            .is_accepted()
    );
    assert!(!harness.mocks.monitor.control().is_paused());

    assert!(
        harness
            .service
            .dispatch(AppCommand::MonitorRefresh)
            .is_accepted()
    );
    assert_eq!(harness.mocks.monitor.control().refreshes(), 1);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_superseded_process_detail_reply_is_dropped_and_only_the_newest_updates_the_snapshot() {
    use wfdiag_app::ports::monitor::ProcessDetail;

    let mocks = MockPorts::new();
    mocks.monitor.set_process_detail(ProcessDetail {
        pid: 7,
        image_path: Some(r"C:\\Program Files\\Seven\\seven.exe".to_string()),
        access_denied: false,
    });
    mocks.monitor.set_process_detail(ProcessDetail {
        pid: 8,
        image_path: None,
        access_denied: true,
    });
    let mut harness = boot_with("guards_stale_process_detail", mocks);

    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProcessDetail { pid: 7 })
            .is_accepted()
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProcessDetail { pid: 8 })
            .is_accepted()
    );
    let events = harness.pump_for("the process detail", |event| {
        matches!(event, AppEvent::Monitor(MonitorEvent::ProcessDetail(_)))
    });
    let delivered: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Monitor(MonitorEvent::ProcessDetail(detail)) => Some(detail.pid),
            _ => None,
        })
        .collect();
    assert_eq!(
        delivered,
        [8],
        "the older reply is dropped, not merely overwritten"
    );
    let detail = harness
        .service
        .snapshot()
        .monitor
        .process_detail
        .clone()
        .expect("the newest detail is in the read model");
    assert_eq!(detail.pid, 8);
    assert!(detail.access_denied);
    harness.shutdown(Duration::from_secs(2));
}
