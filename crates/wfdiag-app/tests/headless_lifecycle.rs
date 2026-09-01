//! Start-up, shutdown, and the command surface's refusal contract.

mod support;

use std::time::Duration;
use support::{TempDir, boot, start_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{
    AppCommand, AppConfig, AppEvent, AppService, DispatchOutcome, RejectReason, WorkerKind,
};

/// Issue #200: a shell must be able to paint its first frame with the user's
/// persisted theme. The settings document is therefore read synchronously
/// during `start`, before any worker exists and before any event is drained.
#[test]
fn the_persisted_settings_are_available_before_the_first_drain() {
    let mocks = MockPorts::new();
    mocks
        .settings_storage
        .seed(br#"{"theme":"light","scanOnStartup":true,"maxConcurrentTasks":3}"#.to_vec());
    let harness = start_with("settings_first_frame", mocks, |config| config);
    assert_eq!(
        harness.service.snapshot().settings.theme,
        "light",
        "the persisted theme is readable before the settings worker answers"
    );
    assert_eq!(harness.service.snapshot().settings.max_concurrent_tasks, 3);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn start_publishes_a_snapshot_and_shutdown_terminates_within_its_budget() {
    let harness = boot("lifecycle");
    assert!(
        harness
            .startup_events
            .iter()
            .any(|event| matches!(event, AppEvent::Started { .. })),
        "the host is told when the service is ready"
    );

    let started = std::time::Instant::now();
    let report = harness.shutdown(Duration::from_secs(2));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "shutdown must be bounded, not best-effort"
    );
    assert!(report.is_clean(), "every worker stopped: {report:#?}");
    let stopped: Vec<WorkerKind> = report.workers.iter().map(|record| record.worker).collect();
    assert_eq!(
        stopped.first(),
        Some(&WorkerKind::Monitor),
        "event producers stop before the workers that answer requests"
    );
    assert_eq!(
        stopped.last(),
        Some(&WorkerKind::Settings),
        "settings stop last so an in-flight save is not cut short"
    );
}

#[test]
fn a_second_start_is_ignored_and_commands_after_shutdown_are_refused() {
    let mut harness = boot("lifecycle_guards");
    assert!(matches!(
        harness.service.dispatch(AppCommand::Start {
            startup_scan: false
        }),
        DispatchOutcome::Ignored { .. }
    ));
    assert!(harness.service.dispatch(AppCommand::Shutdown).is_accepted());
    assert_eq!(
        harness
            .service
            .dispatch(AppCommand::ListHistory)
            .rejection(),
        Some(&RejectReason::Terminating)
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn every_declared_extension_point_is_rejected_as_not_wired() {
    let mut harness = boot("not_wired");
    let unwired = [
        AppCommand::ChatSend {
            prompt: "hello".to_string(),
        },
        AppCommand::ChatCancel,
        AppCommand::ChatReset,
        AppCommand::CloudFallbackDecision { allow: true },
        AppCommand::GenerateReport {
            force_refresh: false,
        },
        AppCommand::CancelReport,
        AppCommand::AnalyzeDiagnostic {
            task_id: "os_info".to_string(),
        },
        AppCommand::CancelAnalysis,
        AppCommand::PrioritizeIssues,
        AppCommand::GenerateFixPlan,
        AppCommand::CancelFixPlan,
        AppCommand::PrepareRemediation {
            remediation_id: "open_disk_cleanup".to_string(),
        },
        AppCommand::ApproveAction {
            proposal_id: "proposal".to_string(),
        },
        AppCommand::DiscardProposal {
            proposal_id: "proposal".to_string(),
        },
        AppCommand::CancelAction {
            run_id: "run".to_string(),
        },
        AppCommand::RefreshModelCatalog {
            provider: "openai".to_string(),
        },
        AppCommand::CancelModelCatalog,
        AppCommand::SubscriptionAuth {
            provider: "codex_cli".to_string(),
            sign_in: true,
        },
        AppCommand::CancelSubscriptionAuth,
        AppCommand::InstallSubscriptionCli {
            provider: "codex_cli".to_string(),
        },
        AppCommand::CancelSubscriptionInstall,
    ];
    for command in unwired {
        assert_eq!(
            harness.service.dispatch(command.clone()).rejection(),
            Some(&RejectReason::NotWired),
            "{command:?} must refuse loudly, never silently no-op"
        );
    }
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_worker_that_never_started_rejects_its_commands_with_the_reason() {
    // No history directory: the history worker never starts.
    let directory = TempDir::new("no_history");
    let mocks = MockPorts::new();
    let config = AppConfig::default().with_debug_build(false);
    let (mut service, _events) =
        AppService::start(config, mocks.to_ports()).expect("the service starts");
    let rejection = service.dispatch(AppCommand::ListHistory);
    match rejection.rejection() {
        Some(RejectReason::WorkerUnavailable { worker, detail }) => {
            assert_eq!(*worker, WorkerKind::History);
            assert!(
                detail.contains("history storage directory"),
                "the rejection carries the startup diagnostic: {detail}"
            );
        }
        other => panic!("expected a history-unavailable rejection, got {other:?}"),
    }
    assert!(
        service
            .snapshot()
            .worker_error(WorkerKind::History)
            .is_some(),
        "the snapshot records why the worker is missing"
    );
    let report = service.shutdown(Duration::from_secs(2));
    assert!(report.is_clean());
    drop(directory);
}
