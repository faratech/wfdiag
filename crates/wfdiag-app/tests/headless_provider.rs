//! AI provider status, the Phi Silica preference gate, and the cache control.

mod support;

use std::time::Duration;
use support::{boot, boot_with};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{AppCommand, AppEvent, ProviderEvent, RejectReason};
use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, ProviderProbeSnapshot};

#[test]
fn a_status_refresh_projects_the_backend_probes() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        ollama_endpoint: Some("http://127.0.0.1:11434".to_string()),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("provider_status", mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProviderStatus)
            .is_accepted()
    );
    let events = harness.pump_for("the provider status", |event| {
        matches!(event, AppEvent::Provider(ProviderEvent::Status(_)))
    });
    let AppEvent::Provider(ProviderEvent::Status(status)) = events
        .iter()
        .find(|event| matches!(event, AppEvent::Provider(ProviderEvent::Status(_))))
        .expect("a status arrived")
    else {
        unreachable!("filtered above")
    };
    assert_eq!(status.active_provider, AIProvider::Ollama);
    assert!(
        status
            .providers
            .iter()
            .any(|row| row.id == AIProvider::Ollama && row.available)
    );
    assert!(!harness.service.snapshot().provider_loading);
    assert_eq!(
        harness
            .service
            .snapshot()
            .provider_status
            .as_ref()
            .map(|status| status.active_provider),
        Some(AIProvider::Ollama)
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn phi_silica_cannot_be_selected_before_a_probe_or_when_it_is_not_ready() {
    let mut harness = boot("provider_phi_gate");
    // No status yet: the gate refuses rather than guessing.
    let outcome = harness.service.dispatch(AppCommand::SetProviderPreference {
        preference: "phi_silica".to_string(),
    });
    assert!(matches!(
        outcome.rejection(),
        Some(RejectReason::Invalid { .. })
    ));
    let events = harness.pump_briefly();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::Provider(ProviderEvent::PreferenceRejected { .. })
        )),
        "the host is told why the selection was refused"
    );
    assert_eq!(
        harness.mocks.provider_backend.preference(),
        AIProviderPreference::default(),
        "no preference reached the backend"
    );

    // A probe that reports Phi as unavailable keeps the gate closed.
    harness.service.dispatch(AppCommand::RequestProviderStatus);
    harness.pump_for("the provider status", |event| {
        matches!(event, AppEvent::Provider(ProviderEvent::Status(_)))
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::SetProviderPreference {
                preference: "phi_silica".to_string(),
            })
            .rejection()
            .is_some()
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_valid_preference_is_applied_and_answered_with_the_new_status() {
    let mocks = MockPorts::new();
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        ollama_endpoint: Some("http://127.0.0.1:11434".to_string()),
        ..ProviderProbeSnapshot::default()
    });
    let mut harness = boot_with("provider_preference", mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::SetProviderPreference {
                preference: "ollama".to_string(),
            })
            .is_accepted()
    );
    let events = harness.pump_for("the applied preference", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::PreferenceApplied { .. })
        )
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Provider(ProviderEvent::PreferenceApplied { preference, status })
            if preference == "ollama" && status.preferred_provider == AIProvider::Ollama
    )));
    assert_eq!(
        harness.mocks.provider_backend.preference(),
        AIProviderPreference::Ollama
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn clearing_the_ai_cache_reaches_the_backend() {
    let mut harness = boot("provider_cache");
    assert!(
        harness
            .service
            .dispatch(AppCommand::ClearAiCache {
                session_id: Some("chat-1".to_string()),
            })
            .is_accepted()
    );
    harness.pump_for("the cache acknowledgement", |event| {
        matches!(event, AppEvent::Provider(ProviderEvent::CacheCleared))
    });
    assert_eq!(
        harness.mocks.provider_backend.cleared_caches(),
        [Some("chat-1".to_string())]
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn the_ollama_model_list_is_forwarded_verbatim() {
    let mocks = MockPorts::new();
    mocks
        .provider_backend
        .set_ollama_models(Ok(vec!["llama3.3".to_string(), "qwen3".to_string()]));
    let mut harness = boot_with("provider_models", mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::ListOllamaModels)
            .is_accepted()
    );
    let events = harness.pump_for("the model list", |event| {
        matches!(event, AppEvent::Provider(ProviderEvent::OllamaModels(_)))
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Provider(ProviderEvent::OllamaModels(models)) if models == &["llama3.3", "qwen3"]
    )));
    harness.shutdown(Duration::from_secs(2));
}
