//! Live model discovery: the debounce, cancellation, and the rule that a
//! failed refresh never blanks a list the user was reading.

mod support;

use std::time::Duration;
use support::boot_ai;
use wfdiag_app::{AppCommand, AppEvent, ModelCatalogEvent, ProviderEvent};
use wfdiag_native_ai_provider::{AIProvider, ModelCatalog, ModelCatalogEntry};

fn catalog(id: &str) -> ModelCatalog {
    ModelCatalog {
        models: vec![ModelCatalogEntry::from_id(id)],
        default_model: Some(id.to_string()),
    }
}

fn refresh(provider: &str, forced: bool) -> AppCommand {
    AppCommand::RefreshModelCatalog {
        provider: provider.to_string(),
        draft_api_key: None,
        draft_endpoint: None,
        draft_cli_path: None,
        forced,
    }
}

#[test]
fn a_refresh_loads_the_catalog_and_the_next_keystroke_is_debounced() {
    let mut harness = boot_ai("catalog_debounce");
    harness
        .mocks
        .ai
        .model_catalog
        .script(AIProvider::Ollama, Ok(catalog("llama3")));

    assert!(
        harness
            .service
            .dispatch(refresh("ollama", true))
            .is_accepted()
    );
    let events = harness.pump_for("the catalog", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Loaded { .. }
            ))
        )
    });
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Provider(ProviderEvent::ModelCatalog(
            ModelCatalogEvent::Started { .. }
        ))
    )));
    let state = harness
        .service
        .snapshot()
        .provider_setup
        .catalogs
        .get("ollama")
        .cloned()
        .expect("the catalog is in the read model");
    assert!(!state.loading && !state.stale && state.error.is_none());
    assert_eq!(
        state.catalog.expect("a catalog loaded").models[0].id,
        "llama3"
    );

    // A second, unforced request inside the debounce window is refused
    // without touching the provider again.
    let outcome = harness.service.dispatch(refresh("ollama", false));
    assert!(!outcome.is_accepted(), "typing is coalesced, not re-probed");
    let events = harness.pump_briefly();
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Provider(ProviderEvent::ModelCatalog(
            ModelCatalogEvent::Throttled { .. }
        ))
    )));
    assert_eq!(
        harness.mocks.ai.model_catalog.requests(),
        [AIProvider::Ollama],
        "the debounced request never reached discovery"
    );
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_failed_refresh_keeps_the_last_list_on_screen_and_flags_it_stale() {
    let mut harness = boot_ai("catalog_stale");
    harness
        .mocks
        .ai
        .model_catalog
        .script(AIProvider::Ollama, Ok(catalog("llama3")));
    let _ = harness.service.dispatch(refresh("ollama", true));
    harness.pump_for("the first catalog", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Loaded { .. }
            ))
        )
    });

    harness
        .mocks
        .ai
        .model_catalog
        .script(AIProvider::Ollama, Err("the server is offline".to_string()));
    let _ = harness.service.dispatch(refresh("ollama", true));
    let events = harness.pump_for("the failure", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Failed { .. }
            ))
        )
    });
    let last = events
        .iter()
        .find_map(|event| match event {
            AppEvent::Provider(ProviderEvent::ModelCatalog(ModelCatalogEvent::Failed {
                last,
                ..
            })) => Some(last.clone()),
            _ => None,
        })
        .expect("the failure carries the list still worth showing");
    assert_eq!(
        last.expect("the previous catalog survives").models[0].id,
        "llama3"
    );

    let state = harness
        .service
        .snapshot()
        .provider_setup
        .catalogs
        .get("ollama")
        .cloned()
        .expect("the read model keeps the provider");
    assert!(state.stale, "the surviving list is marked stale");
    assert!(state.catalog.is_some(), "the list is not blanked");
    assert_eq!(state.error.as_deref(), Some("the server is offline"));
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn cancelling_a_refresh_stops_it_without_replacing_the_catalog() {
    let mut harness = boot_ai("catalog_cancel");
    harness
        .mocks
        .ai
        .model_catalog
        .script(AIProvider::Ollama, Ok(catalog("llama3")));
    harness.mocks.ai.model_catalog.hold();

    let _ = harness.service.dispatch(refresh("ollama", true));
    harness.pump_for("the refresh to start", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Started { .. }
            ))
        )
    });
    assert!(
        harness
            .service
            .dispatch(AppCommand::CancelModelCatalog)
            .is_accepted()
    );
    let events = harness.pump_for("the cancellation", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Cancelled { .. }
            ))
        )
    });
    assert!(!events.iter().any(|event| matches!(
        event,
        AppEvent::Provider(ProviderEvent::ModelCatalog(
            ModelCatalogEvent::Loaded { .. }
        ))
    )));
    let state = harness
        .service
        .snapshot()
        .provider_setup
        .catalogs
        .get("ollama")
        .cloned()
        .expect("the provider is tracked");
    assert!(!state.loading);
    assert!(state.catalog.is_none());
    harness.shutdown(Duration::from_secs(2));
}

#[test]
fn a_provider_whose_probe_has_side_effects_is_never_discovered_by_typing() {
    let mut harness = boot_ai("catalog_manual_only");
    harness
        .mocks
        .ai
        .model_catalog
        .script(AIProvider::ClaudeCode, Ok(catalog("claude")));

    assert!(
        matches!(
            harness.service.dispatch(refresh("claude_code", false)),
            wfdiag_app::DispatchOutcome::Ignored { .. }
        ),
        "the Claude Code probe downloads an adapter package: it needs an explicit Refresh"
    );
    assert!(harness.mocks.ai.model_catalog.requests().is_empty());

    assert!(
        harness
            .service
            .dispatch(refresh("claude_code", true))
            .is_accepted()
    );
    harness.pump_for("the explicit refresh", |event| {
        matches!(
            event,
            AppEvent::Provider(ProviderEvent::ModelCatalog(
                ModelCatalogEvent::Loaded { .. }
            ))
        )
    });
    assert_eq!(
        harness.mocks.ai.model_catalog.requests(),
        [AIProvider::ClaudeCode]
    );
    harness.shutdown(Duration::from_secs(2));
}
