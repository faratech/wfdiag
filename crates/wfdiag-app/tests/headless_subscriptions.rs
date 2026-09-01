//! Subscription-CLI accounts and installation.
//!
//! The two invariants under test are the interlock (one operation at a time
//! across both machines) and the two-confirmation ladder: winget needs one
//! approval, the vendor's PowerShell bootstrap needs a second.

mod support;

use std::time::Duration;
use support::boot_ai;
use wfdiag_app::{AppCommand, AppEvent, ProviderEvent, SubscriptionEvent, SubscriptionOperation};
use wfdiag_native_ai_chat::{
    SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionAuthState,
    SubscriptionInstallMethod,
};

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
    assert_eq!(
        harness
            .service
            .snapshot()
            .provider_setup
            .accounts
            .get(CODEX)
            .and_then(|account| account.status.as_ref())
            .map(|status| status.state),
        Some(SubscriptionAuthState::SignedIn)
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
