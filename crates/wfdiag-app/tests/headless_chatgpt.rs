//! Native account commands use the real facade, cancellation and snapshot guards.
mod support;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::ports::mock_ai::ScriptedTurn;
use wfdiag_app::{AppCommand, AppConfig, AppEvent, AppService, ChatEvent, RejectReason};
use wfdiag_native_ai_provider::{AIProvider, ProviderProbeSnapshot};
use wfdiag_native_chatgpt::{
    AccountCommand, AuthorizationUrl, BrowserLauncher, ChatGptPorts, ChatGptService,
};
use wfdiag_native_settings::{
    AppSettings, MemoryProtectedDocumentStorage, ProtectedDocumentStorage,
};

fn pump(service: &mut AppService, ready: impl Fn(&AppService) -> bool) -> Vec<AppEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    loop {
        events.extend(service.drain());
        if ready(service) {
            return events;
        }
        assert!(Instant::now() < deadline, "account action timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn seeded() -> (AppService, MockPorts, Arc<ChatGptService>) {
    let mocks = MockPorts::new();
    mocks.settings_storage.seed(
        serde_json::to_vec(&AppSettings {
            preferred_ai_provider: "ollama".into(),
            ai_enabled: true,
            ..AppSettings::default()
        })
        .unwrap(),
    );
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        ollama_endpoint: Some("http://127.0.0.1:11434".into()),
        ..ProviderProbeSnapshot::default()
    });
    let store = Arc::new(MemoryProtectedDocumentStorage::default());
    store.try_lock().unwrap().unwrap().save(&json!({"version":1,"host_id":"urn:uuid:419c1ae5-9389-4f2c-85de-8d908c37c4a6","active":"account","accounts":[{"id":"account","client_id":"issued","subject":"subject","email":"fixture@example.test","label":"Test account","tokens":{"access":"fixture-access","refresh":"fixture-refresh","identity":null,"scopes":["chatgpt.tokens.use.direct"],"expires_at":u64::MAX,"earliest_refresh_at":0},"model":"first","models":[{"slug":"first","display_name":"First"},{"slug":"second","display_name":"Second"}],"failure":null,"welcome_seen":true,"revision":1}]}).to_string()).unwrap();
    let mut account_ports = ChatGptPorts::mock();
    account_ports.storage = store;
    let account = Arc::new(ChatGptService::new(account_ports));
    let mut ports = mocks.to_ports();
    ports.ai.chatgpt = account.clone();
    let (mut service, _) = AppService::start(AppConfig::default(), ports).unwrap();
    service.dispatch(AppCommand::Start {
        startup_scan: false,
    });
    pump(&mut service, |s| {
        s.snapshot().system_info.is_some()
            && !s.snapshot().settings_loading
            && s.snapshot().provider_setup.chatgpt.active.is_some()
    });
    (service, mocks, account)
}
#[test]
fn account_model_change_cancels_chat_clears_cache_and_keeps_provider_preference() {
    let (mut service, mocks, account) = seeded();
    mocks
        .ai
        .chat
        .script(AIProvider::Ollama, vec![ScriptedTurn::text("old context")]);
    let hold = mocks.ai.chat.hold();
    assert!(
        service
            .dispatch(AppCommand::ChatSend {
                prompt: "Explain my PC".into()
            })
            .is_accepted()
    );
    pump(&mut service, |s| s.snapshot().ai.chat.streaming);
    mocks
        .ai
        .cache
        .insert("account-test".into(), "old cache".into());
    assert!(
        service
            .dispatch(AppCommand::ChatGptAccount(AccountCommand::SelectModel(
                "second".into()
            )))
            .is_accepted()
    );
    let events = pump(&mut service, |s| {
        s.snapshot()
            .provider_setup
            .chatgpt
            .selected()
            .unwrap()
            .model
            .as_deref()
            == Some("second")
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AppEvent::Chat(ChatEvent::SessionReset)))
    );
    assert!(!service.snapshot().ai.chat.streaming);
    assert!(mocks.ai.cache.get("account-test").is_none());
    assert_eq!(service.snapshot().settings.preferred_ai_provider, "ollama");
    assert_eq!(account.snapshot().selected().unwrap().generation, 2);
    hold.cancel();
    let _ = service.shutdown(Duration::from_secs(2));
}
#[derive(Default)]
struct HeldBrowser(Mutex<Option<String>>);
impl BrowserLauncher for HeldBrowser {
    fn open(&self, target: &AuthorizationUrl) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(target.as_str().into());
        Ok(())
    }
}
#[test]
fn sign_in_is_nonblocking_cancellable_and_shutdown_closes_the_listener() {
    let mocks = MockPorts::new();
    let browser = Arc::new(HeldBrowser::default());
    let mut account_ports = ChatGptPorts::mock();
    account_ports.browser = browser.clone();
    let account = Arc::new(ChatGptService::new(account_ports));
    let mut ports = mocks.to_ports();
    ports.ai.chatgpt = account;
    let (mut service, _) = AppService::start(AppConfig::default(), ports).unwrap();
    // Wait for the startup storage probe's completion before an account action.
    pump(&mut service, |_| browser.0.lock().unwrap().is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = service.dispatch(AppCommand::ChatGptAccount(AccountCommand::Connect {
            account: None,
            consent: false,
        }));
        if result.is_accepted() {
            break;
        }
        assert!(matches!(
            result.rejection(),
            Some(RejectReason::Busy { .. })
        ));
        assert!(Instant::now() < deadline);
        service.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    pump(&mut service, |_| browser.0.lock().unwrap().is_some());
    assert!(service.snapshot().provider_setup.chatgpt.busy);
    assert!(
        service
            .dispatch(AppCommand::ChatGptAccount(AccountCommand::Cancel))
            .is_accepted()
    );
    pump(&mut service, |s| {
        !s.snapshot().provider_setup.chatgpt.busy
            && s.snapshot().provider_setup.chatgpt.error.is_some()
    });
    assert_eq!(
        service.snapshot().provider_setup.chatgpt.accounts,
        [] as [wfdiag_native_chatgpt::AccountView; 0]
    );
    let _ = service.shutdown(Duration::from_secs(2));
}

#[test]
fn chatgpt_failures_stop_auto_before_another_billing_provider_across_requests() {
    let mocks = support::ai_mocks();
    mocks.settings_storage.seed(
        serde_json::to_vec(&AppSettings {
            cloud_fallback_policy: wfdiag_native_settings::CloudFallbackPolicy::Allow,
            ..AppSettings::default()
        })
        .unwrap(),
    );
    mocks.provider_backend.set_probes(ProviderProbeSnapshot {
        chatgpt_ready: true,
        chatgpt_connected: true,
        chatgpt_model: Some("account-model".into()),
        openai_available: true,
        ..ProviderProbeSnapshot::default()
    });
    mocks.ai.chat.script(
        AIProvider::ChatGPT,
        vec![
            ScriptedTurn::failure("ChatGPT quota reached"),
            ScriptedTurn::failure("ChatGPT quota reached"),
        ],
    );
    mocks.ai.chat.script(
        AIProvider::OpenAI,
        vec![ScriptedTurn::text("must not use an API key")],
    );
    let mut harness = support::boot_ai_with("chatgpt_cloud_boundary", mocks);
    for _ in 0..2 {
        assert!(
            harness
                .service
                .dispatch(AppCommand::ChatSend {
                    prompt: "Hello".into()
                })
                .is_accepted()
        );
        harness.pump_for("ChatGPT failure", |event| {
            matches!(event, AppEvent::Chat(ChatEvent::Failed { .. }))
        });
    }
    assert_eq!(
        harness.mocks.ai.chat.resolved(),
        vec![AIProvider::ChatGPT, AIProvider::ChatGPT]
    );
    harness.shutdown(Duration::from_secs(2));
}
