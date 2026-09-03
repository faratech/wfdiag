//! Subscription-CLI accounts and installation.
//!
//! The two invariants under test are the interlock (one operation at a time
//! across both machines) and the two-confirmation ladder: winget needs one
//! approval, the vendor's PowerShell bootstrap needs a second.

mod support;

use std::time::Duration;
use support::{Harness, boot_ai, boot_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{
    AppCommand, AppEvent, DispatchOutcome, ProviderEvent, RejectReason, SignInRequiredReason,
    SubscriptionEvent, SubscriptionOperation,
};
use wfdiag_native_ai_chat::{
    CliObstacle, SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionAuthState,
    SubscriptionInstallMethod,
};
use wfdiag_native_ai_provider::{CliProbeSnapshot, ProviderProbeSnapshot};

/// An installed CLI with no login cache, as the probe reports it.
fn installed_no_login(path: &str) -> CliProbeSnapshot {
    CliProbeSnapshot {
        usable: false,
        installed: true,
        path: Some(path.to_string()),
        obstacle: Some(CliObstacle::NoStoredLogin),
    }
}

fn refresh_status(harness: &mut Harness) -> Vec<AppEvent> {
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProviderStatus)
            .is_accepted()
    );
    harness.pump_for("the provider status", |event| {
        matches!(event, AppEvent::Provider(ProviderEvent::Status(_)))
    })
}

fn sign_in_required(
    events: &[AppEvent],
) -> Vec<(SubscriptionAuthProvider, CliObstacle, SignInRequiredReason)> {
    subscription_events(events)
        .into_iter()
        .filter_map(|event| match event {
            SubscriptionEvent::SignInRequired {
                provider,
                obstacle,
                reason,
            } => Some((*provider, *obstacle, *reason)),
            _ => None,
        })
        .collect()
}

const CODEX: &str = "codex_cli";

fn subscription_events(events: &[AppEvent]) -> Vec<&SubscriptionEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Provider(ProviderEvent::Subscription(event)) => Some(event.as_ref()),
            _ => None,
        })
        .collect()
}

/// Wait for the codex account row to reach `state`: the projection lands in
/// a drain batch after the operation's own Completed event, so a one-shot
/// snapshot read races it under load (2026-09-03 audit #316).
fn wait_for_account_state(harness: &mut Harness, state: SubscriptionAuthState, what: &str) {
    harness.pump_until(
        |harness, _| {
            harness
                .service
                .snapshot()
                .provider_setup
                .accounts
                .get(CODEX)
                .and_then(|account| account.status.as_ref())
                .map(|status| status.state)
                == Some(state)
        },
        what,
    );
}

#[test]
fn status_then_sign_in_then_sign_out_walk_the_account_state_machine() {
    let mut harness = boot_ai("subscription_auth");
    harness.mocks.ai.subscriptions.set_state(
        SubscriptionAuthProvider::Codex,
        SubscriptionAuthState::SignedOut,
    );
    harness
        .mocks
        .ai
        .subscriptions
        .set_install_path(if cfg!(windows) {
            r"C:\scripted\codex.cmd"
        } else {
            "/scripted/codex"
        });

    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::Status,
            })
            .is_accepted()
    );
    let events = harness.pump_for("the account status", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Status { .. })
        )
    });
    let status = subscription_events(&events)
        .into_iter()
        .find_map(|event| match event {
            SubscriptionEvent::Status { status } => Some(status.clone()),
            _ => None,
        })
        .expect("a status arrived");
    assert_eq!(status.state, SubscriptionAuthState::SignedOut);
    assert!(status.installed());

    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::SignIn,
            })
            .is_accepted()
    );
    harness.pump_for("the sign-in", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Completed { operation: SubscriptionAuthOperation::SignIn, .. })
        )
    });
    // The account row flips when the post-operation status projection is
    // drained, which can be a later batch than Completed itself.
    wait_for_account_state(
        &mut harness,
        SubscriptionAuthState::SignedIn,
        "the signed-in projection",
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::SignOut,
            })
            .is_accepted()
    );
    harness.pump_for("the sign-out", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Completed { operation: SubscriptionAuthOperation::SignOut, .. })
        )
    });
    wait_for_account_state(
        &mut harness,
        SubscriptionAuthState::SignedOut,
        "the signed-out projection",
    );
    assert_eq!(
        harness.mocks.ai.subscriptions.operations(),
        [
            (
                SubscriptionAuthProvider::Codex,
                SubscriptionAuthOperation::Status
            ),
            (
                SubscriptionAuthProvider::Codex,
                SubscriptionAuthOperation::SignIn
            ),
            (
                SubscriptionAuthProvider::Codex,
                SubscriptionAuthOperation::SignOut
            ),
        ]
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn an_account_operation_can_be_cancelled_and_blocks_a_second_one_while_it_runs() {
    let mut harness = boot_ai("subscription_auth_cancel");
    harness.mocks.ai.subscriptions.hold_auth();

    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::SignIn,
            })
            .is_accepted()
    );
    harness.pump_briefly();
    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::Status,
            })
            .rejection()
            .is_some(),
        "one account operation at a time"
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::InstallSubscriptionCli {
                provider: CODEX.to_string(),
            })
            .rejection()
            .is_some(),
        "an installer must not race the account it is about to change"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelSubscriptionAuth)
            .is_accepted()
    );
    harness.pump_for("the cancellation", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Cancelled { .. })
        )
    });
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn installation_needs_a_confirmation_and_declining_installs_nothing() {
    let mut harness = boot_ai("subscription_install_confirm");

    assert!(
        harness
            .service
            .dispatch(AppCommand::InstallSubscriptionCli {
                provider: CODEX.to_string(),
            })
            .is_accepted()
    );
    assert!(
        harness
            .service
            .snapshot()
            .provider_setup
            .install_prompt
            .is_some(),
        "asking is all that happened"
    );
    assert!(
        harness.mocks.ai.subscriptions.installs().is_empty(),
        "no installer ran before the user answered"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::ConfirmSubscriptionInstall { accepted: false })
            .is_accepted()
    );
    harness.pump_briefly();
    assert!(harness.mocks.ai.subscriptions.installs().is_empty());
    assert!(
        harness
            .service
            .snapshot()
            .provider_setup
            .install_prompt
            .is_none()
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_vendor_bootstrap_needs_its_own_second_confirmation() {
    let mut harness = boot_ai("subscription_install_fallback");
    harness.mocks.ai.subscriptions.require_vendor_fallback();

    let _ = harness
        .service
        .dispatch(AppCommand::InstallSubscriptionCli {
            provider: CODEX.to_string(),
        });
    assert!(
        harness
            .service
            .dispatch(AppCommand::ConfirmSubscriptionInstall { accepted: true })
            .is_accepted()
    );
    harness.pump_for("the fallback request", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::InstallFallbackRequired { .. })
        )
    });
    assert_eq!(
        harness.mocks.ai.subscriptions.installs(),
        [(
            SubscriptionAuthProvider::Codex,
            SubscriptionInstallMethod::Winget
        )],
        "accepting the first confirmation never runs the vendor script"
    );
    assert!(
        harness
            .service
            .snapshot()
            .provider_setup
            .install_prompt
            .is_some(),
        "a second, separate confirmation is now open"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::ConfirmSubscriptionInstall { accepted: true })
            .is_accepted()
    );
    harness.pump_for("the installation", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Installed { .. })
        )
    });
    assert_eq!(
        harness.mocks.ai.subscriptions.installs(),
        [
            (
                SubscriptionAuthProvider::Codex,
                SubscriptionInstallMethod::Winget
            ),
            (
                SubscriptionAuthProvider::Codex,
                SubscriptionInstallMethod::VendorPowerShell
            ),
        ]
    );
    assert_eq!(
        harness
            .service
            .snapshot()
            .provider_setup
            .accounts
            .get(CODEX)
            .and_then(|account| account.status.as_ref())
            .map(|status| status.state),
        Some(SubscriptionAuthState::SignedOut),
        "a fresh install is never signed in automatically"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn cancelling_an_installation_kills_its_process_tree() {
    let mut harness = boot_ai("subscription_install_cancel");
    harness.mocks.ai.subscriptions.hold_install();

    let _ = harness
        .service
        .dispatch(AppCommand::InstallSubscriptionCli {
            provider: CODEX.to_string(),
        });
    let _ = harness
        .service
        .dispatch(AppCommand::ConfirmSubscriptionInstall { accepted: true });
    harness.pump_for("the installer to start", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::InstallProgress { .. })
        )
    });

    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelSubscriptionInstall)
            .is_accepted()
    );
    harness.pump_for("the cancellation", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::InstallCancelled { .. })
        )
    });
    assert_eq!(
        harness.mocks.ai.subscriptions.killed().len(),
        1,
        "cancelling closes the installer's process tree"
    );
    assert!(
        harness
            .service
            .snapshot()
            .provider_setup
            .install_progress
            .is_none()
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_status_refresh_projects_both_cli_accounts_so_settings_never_says_not_checked() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        codex: installed_no_login("C:/tools/codex.exe"),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("subscription_projection", mocks);
    let events = refresh_status(&mut harness);

    let accounts = &harness.service.snapshot().provider_setup.accounts;
    let codex = accounts["codex_cli"].status.as_ref().expect("projected");
    assert_eq!(codex.state, SubscriptionAuthState::SignedOut);
    assert_eq!(codex.obstacle, Some(CliObstacle::NoStoredLogin));
    assert!(codex.installed());
    let claude = accounts["claude_code"].status.as_ref().expect("projected");
    assert_eq!(claude.state, SubscriptionAuthState::NotInstalled);
    assert!(
        !subscription_events(&events)
            .iter()
            .any(|event| matches!(event, SubscriptionEvent::Status { .. })),
        "a projection is not an account operation"
    );
    // Nothing else is usable, so the installed CLI is the way in.
    assert_eq!(
        sign_in_required(&events),
        [(
            SubscriptionAuthProvider::Codex,
            CliObstacle::NoStoredLogin,
            SignInRequiredReason::OnlyCandidate
        )]
    );
    assert_eq!(
        harness
            .service
            .snapshot()
            .provider_setup
            .sign_in_required
            .map(|requirement| requirement.reason),
        Some(SignInRequiredReason::OnlyCandidate)
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn an_explicit_signed_out_cli_preference_raises_sign_in_required_instead_of_a_bare_refusal() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        openai_available: true,
        codex: installed_no_login("C:/tools/codex.exe"),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("subscription_explicit", mocks);
    let events = refresh_status(&mut harness);
    assert!(
        sign_in_required(&events).is_empty(),
        "Auto routes to OpenAI, so nothing is required yet"
    );
    // The shell persists the preference and applies it to the backend; both
    // halves happen here so the refusal below reads the persisted choice.
    assert!(
        harness
            .service
            .dispatch(AppCommand::UpdateSetting(
                wfdiag_native_settings::SettingsUpdate::PreferredAiProvider(CODEX.to_string())
            ))
            .is_accepted()
    );
    harness.pump_for("the setting", |event| {
        matches!(
            event,
            AppEvent::Settings(wfdiag_app::SettingsEvent::Updated { .. })
        )
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::SetProviderPreference {
                preference: CODEX.to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the preference", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::PreferenceApplied { .. })
        )
    });
    assert_eq!(
        sign_in_required(&events)
            .iter()
            .map(|(_, _, reason)| *reason)
            .collect::<Vec<_>>(),
        [SignInRequiredReason::ExplicitPreference]
    );

    let outcome = harness.service.dispatch(AppCommand::ChatSend {
        prompt: "why is my PC slow?".to_string(),
    });
    let DispatchOutcome::Rejected(RejectReason::NotReady { detail }) = outcome else {
        panic!("a signed-out explicit CLI must refuse with NotReady, got {outcome:?}");
    };
    assert!(
        detail.starts_with("Sign in to ChatGPT before sending: Codex CLI has no stored login"),
        "{detail}"
    );
    let events = harness.pump_briefly();
    assert_eq!(
        sign_in_required(&events).len(),
        1,
        "a refused user action raises the prompt again"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn auto_with_only_a_signed_out_cli_raises_sign_in_required_once_per_change() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        claude: installed_no_login("C:/tools/claude.exe"),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("subscription_once", mocks);
    let first = refresh_status(&mut harness);
    let second = refresh_status(&mut harness);
    assert_eq!(sign_in_required(&first).len(), 1);
    assert!(
        sign_in_required(&second).is_empty(),
        "an unchanged requirement is not re-raised by a refresh"
    );
    harness
        .mocks
        .provider_backend
        .set_probes(ProviderProbeSnapshot {
            claude: CliProbeSnapshot {
                obstacle: Some(CliObstacle::SignedOut),
                ..installed_no_login("C:/tools/claude.exe")
            },
            ..ProviderProbeSnapshot::default()
        });
    let third = refresh_status(&mut harness);
    assert_eq!(
        sign_in_required(&third),
        [(
            SubscriptionAuthProvider::ClaudeCode,
            CliObstacle::SignedOut,
            SignInRequiredReason::OnlyCandidate
        )]
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_completed_install_offers_sign_in_without_starting_it() {
    let mut harness = boot_ai("subscription_install_offer");
    harness
        .mocks
        .ai
        .subscriptions
        .set_install_path("/opt/codex/codex");
    let _ = harness
        .service
        .dispatch(AppCommand::InstallSubscriptionCli {
            provider: CODEX.to_string(),
        });
    assert!(
        harness
            .service
            .dispatch(AppCommand::ConfirmSubscriptionInstall { accepted: true })
            .is_accepted()
    );
    let events = harness.pump_for("the offer", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::SignInOffered { .. })
        )
    });
    let subscription = subscription_events(&events);
    let installed_at = subscription
        .iter()
        .position(|event| matches!(event, SubscriptionEvent::Installed { .. }))
        .expect("installed first");
    let offered_at = subscription
        .iter()
        .position(|event| {
            matches!(
                event,
                SubscriptionEvent::SignInOffered {
                    provider: SubscriptionAuthProvider::Codex
                }
            )
        })
        .expect("then offered");
    assert!(installed_at < offered_at);
    assert!(
        !harness
            .mocks
            .ai
            .subscriptions
            .operations()
            .iter()
            .any(|(_, operation)| *operation == SubscriptionAuthOperation::SignIn),
        "offered, never started"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_pending_account_operation_is_not_overwritten_by_a_status_projection() {
    let mut harness = boot_ai("subscription_pending");
    harness.mocks.ai.subscriptions.hold_auth();
    assert!(
        harness
            .service
            .dispatch(AppCommand::SubscriptionAuth {
                provider: CODEX.to_string(),
                operation: SubscriptionOperation::SignIn,
            })
            .is_accepted()
    );
    harness.pump_for("the sign-in to start", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::Subscription(event))
                if matches!(**event, SubscriptionEvent::Started { .. })
        )
    });
    harness
        .mocks
        .provider_backend
        .set_probes(ProviderProbeSnapshot {
            codex: CliProbeSnapshot {
                usable: true,
                installed: true,
                path: Some("C:/tools/codex.exe".to_string()),
                obstacle: None,
            },
            ..ProviderProbeSnapshot::default()
        });
    refresh_status(&mut harness);
    let account = &harness.service.snapshot().provider_setup.accounts["codex_cli"];
    assert_eq!(account.operation, Some(SubscriptionAuthOperation::SignIn));
    assert_ne!(
        account.status.as_ref().map(|status| status.state),
        Some(SubscriptionAuthState::SignedIn),
        "the running sign-in reports the truth when it is done, not the projection"
    );
    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelSubscriptionAuth)
            .is_accepted()
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn request_subscription_accounts_probes_only_the_two_clis() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        codex: installed_no_login("C:/tools/codex.exe"),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("subscription_startup_check", mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestSubscriptionAccounts)
            .is_accepted()
    );
    harness.pump_until(
        |harness, _| {
            harness
                .service
                .snapshot()
                .provider_setup
                .accounts
                .get("codex_cli")
                .is_some_and(|account| account.status.is_some())
        },
        "the account rows",
    );
    let accounts = &harness.service.snapshot().provider_setup.accounts;
    assert_eq!(
        accounts["codex_cli"].status.as_ref().unwrap().state,
        SubscriptionAuthState::SignedOut
    );
    assert_eq!(
        accounts["claude_code"].status.as_ref().unwrap().state,
        SubscriptionAuthState::NotInstalled
    );
    assert!(
        harness.service.snapshot().provider_status.is_none(),
        "no full provider refresh happened"
    );
    harness.shutdown(Duration::from_secs(2));
}
