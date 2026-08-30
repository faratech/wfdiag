use super::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

fn defaults() -> ProviderModelDefaults {
    ProviderModelDefaults {
        foundry: "phi-4-mini".to_string(),
        openai: "gpt-default".to_string(),
        anthropic: "claude-default".to_string(),
        gemini: "gemini-default".to_string(),
        deepseek: "deepseek-default".to_string(),
    }
}

fn status_input() -> ProviderStatusInput {
    ProviderStatusInput {
        preference: AIProviderPreference::Auto,
        settings: ProviderSettingsSnapshot::default(),
        probes: ProviderProbeSnapshot {
            openai_available: true,
            phi_silica_message: Some("Store package required".to_string()),
            codex: CliProbeSnapshot {
                installed: true,
                path: Some("C:\\Tools\\codex.exe".to_string()),
                ..Default::default()
            },
            ..Default::default()
        },
        defaults: defaults(),
    }
}

#[test]
fn provider_and_preference_wire_contracts_are_pinned() {
    for (provider, wire) in [
        (AIProvider::None, "\"none\""),
        (AIProvider::OpenAI, "\"openai\""),
        (AIProvider::PhiSilica, "\"phi_silica\""),
        (AIProvider::FoundryLocal, "\"foundry_local\""),
        (AIProvider::Ollama, "\"ollama\""),
        (AIProvider::CustomOpenAI, "\"custom_openai\""),
        (AIProvider::CodexCli, "\"codex_cli\""),
        (AIProvider::ClaudeCode, "\"claude_code\""),
        (AIProvider::Anthropic, "\"anthropic\""),
        (AIProvider::Gemini, "\"gemini\""),
        (AIProvider::DeepSeek, "\"deepseek\""),
    ] {
        assert_eq!(serde_json::to_string(&provider).unwrap(), wire);
        assert_eq!(serde_json::from_str::<AIProvider>(wire).unwrap(), provider);
    }
    assert_eq!(
        serde_json::from_str::<AIProvider>("\"open_a_i\"").unwrap(),
        AIProvider::OpenAI
    );
    assert_eq!(
        serde_json::to_string(&AIProviderPreference::OpenAI).unwrap(),
        "\"openai\""
    );
}

#[test]
fn preference_aliases_and_store_gate_match_shipping_behavior() {
    assert_eq!(
        parse_provider_preference(" PhiSilica "),
        AIProviderPreference::PhiSilica
    );
    assert_eq!(
        parse_provider_preference("codex"),
        AIProviderPreference::CodexCli
    );
    assert_eq!(
        parse_provider_preference("future-provider"),
        AIProviderPreference::Auto
    );
    assert_eq!(
        validate_provider_preference(AIProviderPreference::PhiSilica, false).unwrap_err(),
        PHI_SILICA_STORE_REQUIRED
    );
    assert_eq!(
        provider_preference_for_runtime("phi_silica", false),
        AIProviderPreference::Auto
    );
}

#[test]
fn routing_preserves_complete_auto_priority_and_explicit_no_fallback() {
    let mut availability = ProviderAvailability {
        phi: true,
        foundry: true,
        ollama: true,
        custom: true,
        codex: true,
        claude: true,
        openai: true,
        anthropic: true,
        gemini: true,
        deepseek: true,
    };
    for expected in [
        AIProvider::PhiSilica,
        AIProvider::FoundryLocal,
        AIProvider::Ollama,
        AIProvider::CustomOpenAI,
        AIProvider::CodexCli,
        AIProvider::ClaudeCode,
        AIProvider::OpenAI,
        AIProvider::Anthropic,
        AIProvider::Gemini,
        AIProvider::DeepSeek,
    ] {
        assert_eq!(
            route_provider(AIProviderPreference::Auto, availability),
            expected
        );
        match expected {
            AIProvider::PhiSilica => availability.phi = false,
            AIProvider::FoundryLocal => availability.foundry = false,
            AIProvider::Ollama => availability.ollama = false,
            AIProvider::CustomOpenAI => availability.custom = false,
            AIProvider::CodexCli => availability.codex = false,
            AIProvider::ClaudeCode => availability.claude = false,
            AIProvider::OpenAI => availability.openai = false,
            AIProvider::Anthropic => availability.anthropic = false,
            AIProvider::Gemini => availability.gemini = false,
            AIProvider::DeepSeek => availability.deepseek = false,
            AIProvider::None => unreachable!(),
        }
    }
    assert_eq!(
        route_provider(AIProviderPreference::Auto, availability),
        AIProvider::None
    );
    assert_eq!(
        route_provider(
            AIProviderPreference::Gemini,
            ProviderAvailability {
                openai: true,
                ..Default::default()
            }
        ),
        AIProvider::None
    );
}

#[test]
#[allow(clippy::too_many_lines)] // Full JSON golden pins every legacy wire field and omission.
fn status_projection_matches_legacy_shape_order_and_defaults() {
    let status = project_provider_status(status_input());
    assert_eq!(status.preferred_provider, AIProvider::OpenAI);
    assert_eq!(status.active_provider, AIProvider::OpenAI);
    assert!(status.openai_available);
    assert!(status.openai_api_key_set);
    assert_eq!(status.providers.len(), 10);
    assert_eq!(
        status
            .providers
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>(),
        vec![
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal,
            AIProvider::Ollama,
            AIProvider::CustomOpenAI,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
            AIProvider::OpenAI,
            AIProvider::Anthropic,
            AIProvider::Gemini,
            AIProvider::DeepSeek,
        ]
    );
    let codex = status
        .providers
        .iter()
        .find(|provider| provider.id == AIProvider::CodexCli)
        .unwrap();
    assert!(!codex.available);
    assert!(codex.configured);
    assert_eq!(codex.endpoint.as_deref(), Some("C:\\Tools\\codex.exe"));
    let openai = status
        .providers
        .iter()
        .find(|provider| provider.id == AIProvider::OpenAI)
        .unwrap();
    assert_eq!(openai.model.as_deref(), Some("gpt-default"));
    assert!(openai.supports_tools);
    assert!(openai.supports_streaming);

    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "preferred_provider": "openai",
            "openai_available": true,
            "openai_api_key_set": true,
            "phi_silica_available": false,
            "phi_silica_ready": false,
            "phi_silica_message": "Store package required",
            "foundry_local_available": false,
            "active_provider": "openai",
            "providers": [
                {
                    "id": "phi_silica",
                    "available": false,
                    "configured": false,
                    "supports_tools": false,
                    "supports_streaming": false
                },
                {
                    "id": "foundry_local",
                    "available": false,
                    "configured": false,
                    "model": "phi-4-mini",
                    "supports_tools": false,
                    "supports_streaming": true
                },
                {
                    "id": "ollama",
                    "available": false,
                    "configured": true,
                    "supports_tools": true,
                    "supports_streaming": true
                },
                {
                    "id": "custom_openai",
                    "available": false,
                    "configured": false,
                    "supports_tools": true,
                    "supports_streaming": true
                },
                {
                    "id": "codex_cli",
                    "available": false,
                    "configured": true,
                    "endpoint": "C:\\Tools\\codex.exe",
                    "supports_tools": false,
                    "supports_streaming": false
                },
                {
                    "id": "claude_code",
                    "available": false,
                    "configured": false,
                    "supports_tools": false,
                    "supports_streaming": true
                },
                {
                    "id": "openai",
                    "available": true,
                    "configured": true,
                    "model": "gpt-default",
                    "supports_tools": true,
                    "supports_streaming": true
                },
                {
                    "id": "anthropic",
                    "available": false,
                    "configured": false,
                    "model": "claude-default",
                    "supports_tools": true,
                    "supports_streaming": true
                },
                {
                    "id": "gemini",
                    "available": false,
                    "configured": false,
                    "model": "gemini-default",
                    "supports_tools": true,
                    "supports_streaming": true
                },
                {
                    "id": "deepseek",
                    "available": false,
                    "configured": false,
                    "model": "deepseek-default",
                    "supports_tools": true,
                    "supports_streaming": true
                }
            ]
        })
    );
}

#[test]
fn status_deserializes_the_pre_provider_array_legacy_shape() {
    let status: AIProviderStatus = serde_json::from_value(serde_json::json!({
        "preferred_provider": "openai",
        "openai_available": true,
        "openai_api_key_set": true,
        "phi_silica_available": false,
        "phi_silica_ready": false,
        "phi_silica_message": null,
        "active_provider": "openai"
    }))
    .unwrap();
    assert!(!status.foundry_local_available);
    assert_eq!(status.foundry_local_endpoint, None);
    assert!(status.providers.is_empty());
}

#[test]
fn status_projection_preserves_custom_configuration_and_endpoint_fallback() {
    let mut input = status_input();
    input.settings.custom_endpoint = Some(" https://configured.example/v1 ".to_string());
    input.settings.custom_model = Some("model-a".to_string());
    let status = project_provider_status(input);
    let custom = status
        .providers
        .iter()
        .find(|provider| provider.id == AIProvider::CustomOpenAI)
        .unwrap();
    assert!(custom.configured);
    assert!(!custom.available);
    assert_eq!(
        custom.endpoint.as_deref(),
        Some(" https://configured.example/v1 ")
    );
}

#[derive(Default)]
struct FakeBackend {
    identity: bool,
    preference: Mutex<Option<AIProviderPreference>>,
    cleared: Mutex<Vec<Option<String>>>,
    status_calls: AtomicUsize,
    model_calls: AtomicUsize,
    status_polled: AtomicBool,
}

impl ProviderManagementBackend for FakeBackend {
    fn status_input(&self) -> BackendFuture<'_, ProviderStatusInput> {
        Box::pin(async move {
            self.status_calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.status_polled.store(true, Ordering::Release);
            status_input()
        })
    }

    fn has_package_identity(&self) -> bool {
        self.identity
    }

    fn set_preference(&self, preference: AIProviderPreference) {
        *self.preference.lock().unwrap() = Some(preference);
    }

    fn clear_cache(&self, session_id: Option<&str>) {
        self.cleared
            .lock()
            .unwrap()
            .push(session_id.map(str::to_string));
    }

    fn list_ollama_models(&self) -> BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move {
            self.model_calls.fetch_add(1, Ordering::Relaxed);
            Ok(vec!["llama3.2:latest".to_string()])
        })
    }
}

#[tokio::test]
async fn native_worker_is_nonblocking_typed_and_applies_all_commands() {
    let backend = Arc::new(FakeBackend::default());
    let runtime = NativeAiProviderRuntime::start(backend.clone()).unwrap();

    let status_reply = runtime.request_status().unwrap();
    assert!(!backend.status_polled.load(Ordering::Acquire));
    let status = status_reply.await.unwrap();
    assert_eq!(status.active_provider, AIProvider::OpenAI);

    runtime
        .request_set_preference("ollama".to_string())
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *backend.preference.lock().unwrap(),
        Some(AIProviderPreference::Ollama)
    );

    let refreshed = runtime
        .request_set_preference_and_status("openai".to_string())
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.active_provider, AIProvider::OpenAI);
    assert_eq!(
        *backend.preference.lock().unwrap(),
        Some(AIProviderPreference::OpenAI)
    );

    runtime
        .request_clear_cache(Some("session-1".to_string()))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(
        backend.cleared.lock().unwrap().as_slice(),
        &[Some("session-1".to_string())]
    );

    let models = runtime
        .request_ollama_models()
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(models, vec!["llama3.2:latest"]);
    assert_eq!(backend.status_calls.load(Ordering::Relaxed), 2);
    assert_eq!(backend.model_calls.load(Ordering::Relaxed), 1);
}

#[derive(Default)]
struct CancellableBackend {
    started: AtomicBool,
    cancelled: AtomicBool,
}

struct CancellationGuard<'a>(&'a AtomicBool);

impl Drop for CancellationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl ProviderManagementBackend for CancellableBackend {
    fn status_input(&self) -> BackendFuture<'_, ProviderStatusInput> {
        Box::pin(async move {
            let _guard = CancellationGuard(&self.cancelled);
            self.started.store(true, Ordering::Release);
            std::future::pending::<()>().await;
            unreachable!("the cancellation test never completes provider status")
        })
    }

    fn has_package_identity(&self) -> bool {
        false
    }

    fn set_preference(&self, _preference: AIProviderPreference) {}

    fn clear_cache(&self, _session_id: Option<&str>) {}

    fn list_ollama_models(&self) -> BackendFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[test]
fn closing_a_status_reply_cancels_active_probe_and_unblocks_shutdown() {
    let backend = Arc::new(CancellableBackend::default());
    let runtime = NativeAiProviderRuntime::start(backend.clone()).unwrap();
    let reply = runtime.request_status().unwrap();
    for _ in 0..100 {
        if backend.started.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(backend.started.load(Ordering::Acquire));

    drop(reply);
    for _ in 0..100 {
        if backend.cancelled.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(backend.cancelled.load(Ordering::Acquire));
    drop(runtime);
}

#[tokio::test]
async fn native_worker_enforces_phi_identity_gate_before_mutating_backend() {
    let backend = Arc::new(FakeBackend::default());
    let runtime = NativeAiProviderRuntime::start(backend.clone()).unwrap();
    let error = runtime
        .request_set_preference("phi_silica".to_string())
        .unwrap()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error, PHI_SILICA_STORE_REQUIRED);
    assert_eq!(*backend.preference.lock().unwrap(), None);
}
