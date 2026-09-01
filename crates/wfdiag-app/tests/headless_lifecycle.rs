//! Start-up, shutdown, and the command surface's refusal contract.

mod support;

use std::time::Duration;
use support::{TempDir, boot, start_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{
    AppCommand, AppConfig, AppEvent, AppService, DispatchOutcome, RejectReason,
    SubscriptionOperation, WorkerKind,
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

/// Step 11 wired every declared extension point. The contract that replaced
/// the blanket `NotWired` refusal is that each command now reaches its own
/// domain and answers with that domain's real precondition — never a generic
/// refusal, and never a silent no-op.
#[test]
fn every_ai_command_is_routed_to_its_own_domain() {
    let mut harness = boot("ai_routing");

    // Cancelling something that is not running is a no-op, not a failure.
    for command in [
        AppCommand::ChatCancel,
        AppCommand::CancelReport,
        AppCommand::CancelAnalysis,
        AppCommand::CancelFixPlan,
        AppCommand::CancelModelCatalog,
        AppCommand::CancelSubscriptionAuth,
        AppCommand::CancelSubscriptionInstall,
        AppCommand::CloudFallbackDecision { allow: true },
        AppCommand::ConfirmSubscriptionInstall { accepted: true },
    ] {
        assert!(
            matches!(
                harness.service.dispatch(command.clone()),
                DispatchOutcome::Ignored { .. }
            ),
            "{command:?} must be a no-op, not a refusal"
        );
    }

    // Work that needs evidence is refused with the evidence's reason.
    assert!(matches!(
        harness
            .service
            .dispatch(AppCommand::PrioritizeIssues {
                force_refresh: false
            })
            .rejection(),
        Some(RejectReason::NotReady { .. })
    ));
    assert!(matches!(
        harness
            .service
            .dispatch(AppCommand::AnalyzeDiagnostic {
                task_id: "os_info".to_string(),
                force_refresh: false,
            })
            .rejection(),
        Some(RejectReason::NotReady { .. })
    ));
    assert!(
        matches!(
            harness.service.dispatch(AppCommand::GenerateFixPlan),
            DispatchOutcome::Ignored { .. }
        ),
        "there are no detected issues to plan for"
    );

    // An id the engine does not recognise is invalid, never ignored.
    assert!(matches!(
        harness
            .service
            .dispatch(AppCommand::RefreshModelCatalog {
                provider: "not_a_provider".to_string(),
                draft_api_key: None,
                draft_endpoint: None,
                draft_cli_path: None,
                forced: true,
            })
            .rejection(),
        Some(RejectReason::Invalid { .. })
    ));
    assert!(
        matches!(
            harness
                .service
                .dispatch(AppCommand::SubscriptionAuth {
                    provider: "openai".to_string(),
                    operation: SubscriptionOperation::Status,
                })
                .rejection(),
            Some(RejectReason::Invalid { .. })
        ),
        "OpenAI is not a subscription CLI"
    );
    assert!(
        matches!(
            harness
                .service
                .dispatch(AppCommand::ApproveAction {
                    proposal_id: "never-staged".to_string(),
                    confirm_repair: false,
                })
                .rejection(),
            Some(RejectReason::Invalid { .. })
        ),
        "a preview that was never staged cannot be approved"
    );

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
