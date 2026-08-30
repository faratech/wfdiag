# wfdiag-native-ai-provider

UI-framework-neutral provider-management boundary used by the shipping Tauri
backend and prepared for the native Reactor shell.

It owns the exact provider/preference/status wire types, capability table,
local-first routing policy, Store-identity preference gate, pure status
projection, shared response cache, and a nonblocking typed worker for status
refresh, provider selection, cache clearing, and Ollama model discovery. It
also owns the shipping Foundry CLI/health, Codex/Claude subscription-status,
Ollama HTTP, and custom-endpoint TCP probes. Chat, analysis/report streaming,
credential values, and Phi/Aion activation are deliberately out of scope.

The shipping Tauri commands compose `ProviderManagementService` and are thin
adapters over the same worker. No Tauri, Wry, WebView2, or Reactor dependency
exists in this crate.

## Exact Reactor-side composition

On Windows, a native shell can assemble the common service without importing
any `src-tauri` module. The package-specific Phi probe remains an explicit
typed input:

- `PhiStatusSource`: calls the existing package-identity/LAF-aware Phi status
  implementation; activation is intentionally not duplicated here.

`PackageIdentitySource` is also injected so Store admission and development
identity use one process-level policy. Given those two process-specific
implementations, the native setup is:

```rust,ignore
use std::sync::Arc;
use wfdiag_native_ai_provider::*;
use wfdiag_native_settings::windows_shipping_settings_service;

let identity: Arc<dyn PackageIdentitySource> = Arc::new(ReactorPackageIdentity);
let validator = Arc::new(ProviderPreferenceSettingsValidator::new(identity.clone()));
let settings = windows_shipping_settings_service(validator);
let configuration = Arc::new(SettingsServiceProviderConfigurationSource::new(settings));

let selection = ProviderSelectionState::default();
let persisted = configuration.snapshot();
selection.sync_persisted(&persisted.preferred_provider, identity.as_ref());

let probes = ProviderProbeBundle::shipping_networks(
    configuration,
    identity,
    Arc::new(ReactorPhiStatus),
    Arc::new(FoundryCliEndpointSource::new()),
    Arc::new(ProcessSubscriptionCliStatusSource::new()),
);
let cache = SharedAiCache::new(100);
let service = ProviderManagementService::new(
    probes,
    selection,
    Arc::new(cache),
    ProviderModelDefaults {
        foundry: "phi-4-mini".into(),
        openai: OPENAI_MODEL.into(),
        anthropic: ANTHROPIC_DEFAULT_MODEL.into(),
        gemini: GEMINI_DEFAULT_MODEL.into(),
        deepseek: DEEPSEEK_DEFAULT_MODEL.into(),
    },
);
let runtime = NativeAiProviderRuntime::start(Arc::new(service))?;
```

`request_status`, `request_set_preference`, `request_clear_cache`, and
`request_ollama_models` immediately return typed oneshot receivers, so no
network, DPAPI, CLI, or WinRT work executes on the WinUI thread.

## Concrete boundary still outside this crate

Package identity and Phi/Aion activation remain process/package composition
because they depend on the Store identity and Windows AI runtime. Foundry
endpoint discovery and Codex/Claude subscription status are concrete native
adapters in this crate and do not import Tauri, Reactor, Wry, or WebView2.
