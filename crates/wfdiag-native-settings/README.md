# WFDiag native settings seam

`wfdiag-native-settings` owns the UI-framework-neutral `AppSettings` schema,
closed provider-key identifiers, shipping persistence adapters, typed settings
mutations, and the background command/event worker intended for WinUI/Reactor.

The shipping Tauri handlers and a native Windows shell can construct the same
`SettingsService` from the reusable stores. This deliberately preserves both
persisted locations:

- settings: `%APPDATA%/com.windowsforum.diagnostics/settings.json`, written
  through a sibling `.tmp`, file flush, and atomic replace;
- Windows credentials: `%LOCALAPPDATA%/WFDiag/credentials.bin`,
  `credentials_anthropic.bin`, `credentials_gemini.bin`,
  `credentials_deepseek.bin`, and `credentials_custom.bin`, protected with
  current-user DPAPI and the shipping no-additional-entropy contract;
- non-Windows Tauri credentials: the existing `wfdiag-tauri` keyring entries.

Plaintext provider keys are write-only inputs. They are routed directly to the
injected credential store, stripped from serialized settings, and never
returned by `load`; only `*_api_key_set` availability flags are returned.
`load_nonsecret_settings` and `provider_key_is_set` let the shared AI-provider
composition preserve the shipping failure isolation: corrupt settings fall
back independently without hiding otherwise readable credential availability.

For a Windows native shell, construct `ShippingSettingsStorage` and
`WindowsDpapiCredentialStorage` directly, or call
`windows_shipping_settings_service(validator)`, then pass the service to
`SettingsRuntime::start`. No Tauri, Wry, WebView2, or Reactor crate is needed.

## Remaining native-shell integration gaps

- Reactor must bridge `SettingsEvent` delivery into its component message
  queue and explicitly synchronize provider routing, grounding policy, and
  close-to-tray state after successful load/save. Those process-global side
  effects remain shipping-backend responsibilities.
- Store/package-specific provider admission (notably Phi Silica identity) is
  supplied through `SettingsValidator`; a native shell must install the same
  policy rather than use `AllowAllSettings` in production.
- Secure credential reads remain backend-only. A UI must consume availability
  flags and must not add a command that returns key material.
