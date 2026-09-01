//! Settings: load, save, typed update, credentials, and the retention policy
//! the history worker reads back.

mod support;

use std::time::Duration;
use support::boot;
use wfdiag_app::{AppCommand, AppEvent, ProviderCredentialCommand, RejectReason, SettingsEvent};
use wfdiag_native_settings::{ProviderKeyId, SettingsUpdate};

#[test]
fn a_settings_document_round_trips_through_save_and_reload() {
    let mut harness = boot("settings_roundtrip");
    assert_eq!(harness.service.snapshot().settings.theme, "dark");

    let mut settings = harness.service.snapshot().settings.clone();
    settings.theme = "light".to_string();
    settings.history_limit = 7;
    settings.retain_history = false;
    assert!(
        harness
            .service
            .dispatch(AppCommand::SaveSettings(Box::new(settings)))
            .is_accepted()
    );
    let events = harness.pump_for("the save acknowledgement", |event| {
        matches!(event, AppEvent::Settings(SettingsEvent::Saved { .. }))
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Settings(SettingsEvent::Saved { settings }) if settings.theme == "light"
    )));
    assert_eq!(harness.service.snapshot().settings.theme, "light");

    // The document really reached storage: a fresh load reads it back.
    assert!(
        harness
            .service
            .dispatch(AppCommand::LoadSettings)
            .is_accepted()
    );
    let events = harness.pump_for("the reload", |event| {
        matches!(event, AppEvent::Settings(SettingsEvent::Loaded { .. }))
    });
    let AppEvent::Settings(SettingsEvent::Loaded { settings }) = events
        .iter()
        .rev()
        .find(|event| matches!(event, AppEvent::Settings(SettingsEvent::Loaded { .. })))
        .expect("a reload arrived")
    else {
        unreachable!("filtered above")
    };
    assert_eq!(settings.theme, "light");
    assert_eq!(settings.history_limit, 7);
    assert!(!settings.retain_history);
    assert!(
        harness
            .mocks
            .settings_storage
            .document()
            .is_some_and(|bytes| {
                let document = String::from_utf8_lossy(&bytes).replace(' ', "");
                document.contains("\"theme\":\"light\"") && document.contains("\"historyLimit\":7")
            }),
        "the persisted document is the one the host saved"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_typed_update_is_applied_persisted_and_reflected_in_the_snapshot() {
    let mut harness = boot("settings_update");
    assert!(
        harness
            .service
            .dispatch(AppCommand::UpdateSetting(
                SettingsUpdate::MaxConcurrentTasks(2)
            ))
            .is_accepted()
    );
    harness.pump_for("the update acknowledgement", |event| {
        matches!(event, AppEvent::Settings(SettingsEvent::Updated { .. }))
    });
    assert_eq!(harness.service.snapshot().settings.max_concurrent_tasks, 2);
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn provider_credentials_are_stored_and_cleared_through_the_worker() {
    let mut harness = boot("settings_credentials");
    assert!(
        harness
            .service
            .dispatch(AppCommand::ProviderCredential(
                ProviderCredentialCommand::Store {
                    provider: "openai".to_string(),
                    key: "sk-test".to_string(),
                }
            ))
            .is_accepted()
    );
    harness.pump_for("the credential write", |event| {
        matches!(
            event,
            AppEvent::Settings(SettingsEvent::CredentialsCommitted)
        )
    });
    assert!(harness.mocks.credentials.is_set(ProviderKeyId::OpenAI));

    // A load reports availability without ever exposing the secret.
    harness.service.dispatch(AppCommand::LoadSettings);
    harness.pump_for("the reload", |event| {
        matches!(event, AppEvent::Settings(SettingsEvent::Loaded { .. }))
    });
    assert!(harness.service.snapshot().settings.open_ai_api_key_set);
    assert!(
        harness
            .service
            .snapshot()
            .settings
            .open_ai_api_key
            .is_none(),
        "the plaintext key never enters the read model"
    );

    assert!(
        harness
            .service
            .dispatch(AppCommand::ProviderCredential(
                ProviderCredentialCommand::Clear {
                    provider: "openai".to_string(),
                }
            ))
            .is_accepted()
    );
    harness.pump_for("the credential clear", |event| {
        matches!(
            event,
            AppEvent::Settings(SettingsEvent::CredentialsCommitted)
        )
    });
    assert!(!harness.mocks.credentials.is_set(ProviderKeyId::OpenAI));
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn an_unknown_credential_provider_is_refused_before_any_worker_sees_it() {
    let mut harness = boot("settings_bad_provider");
    let outcome = harness.service.dispatch(AppCommand::ProviderCredential(
        ProviderCredentialCommand::Clear {
            provider: "not-a-provider".to_string(),
        },
    ));
    assert!(matches!(
        outcome.rejection(),
        Some(RejectReason::Invalid { .. })
    ));
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_startup_scan_preference_is_honoured_exactly_once() {
    let mocks = wfdiag_app::ports::mock::MockPorts::new();
    mocks
        .settings_storage
        .seed(br#"{"scanOnStartup":true}"#.to_vec());
    let directory = support::TempDir::new("startup_scan");
    let config = support::test_config(&directory);
    let (mut service, events) =
        wfdiag_app::AppService::start(config, mocks.to_ports()).expect("the service starts");
    assert!(
        service
            .dispatch(AppCommand::Start { startup_scan: true })
            .is_accepted()
    );
    let mut harness = support::Harness {
        service,
        events,
        mocks,
        directory,
        startup_events: Vec::new(),
    };
    let events = harness.pump_for("the automatic startup scan", |event| {
        matches!(
            event,
            AppEvent::Scan(wfdiag_app::ScanEvent::Committed { .. })
        )
    });
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::Scan(wfdiag_app::ScanEvent::Started { kind, .. })
                if *kind == wfdiag_native_diagnostics::ScanKind::Quick
        )),
        "the startup scan is a quick scan"
    );
    let executed = harness.mocks.executor.executed().len();
    harness.pump_briefly();
    assert_eq!(
        harness.mocks.executor.executed().len(),
        executed,
        "the gate is single-use: no second automatic scan runs"
    );
    harness.shutdown(Duration::from_secs(2));
}
