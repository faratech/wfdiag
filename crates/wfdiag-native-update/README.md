# wfdiag-native-update

> **2026-09-23:** the Tauri rollback shell (`src-tauri`) and its React frontend
> were deleted. Passages below that compare against or mention the Tauri shell
> are historical context from the migration era; the native WinUI 3 shell
> (`apps/wfdiag`) is the only host of this crate.

UI-framework-neutral update checking for WFDiag's direct GitHub distribution
channel.

The service preserves the shipping policy:

- debug builds never check;
- genuine Microsoft Store installs never check;
- package-signature API failures fail closed as Store installs;
- the GitHub `releases/latest` request has a ten-second total timeout and the
  existing User-Agent/Accept headers;
- drafts, prereleases, malformed versions, same/older versions, transport
  failures, non-success responses, and malformed JSON are all silent;
- release notes are limited to the first 300 Unicode scalar values.

`NativeUpdateRuntime` owns a dedicated background worker. Reactor can call
`request_check()` on the WinUI thread and await the typed one-shot reply
without running Windows package APIs or network I/O on that thread. The crate
has no filesystem, Tauri, WebView2, or Windows Reactor dependency. HTTP,
package signature, and current version providers are injected so policy tests
never use the live network or host package state.

Reactor's shipping constructor is:

```rust
let service = wfdiag_native_update::UpdateService::shipping_from_str(
    env!("WFDIAG_APP_VERSION"),
    cfg!(debug_assertions),
)?;
let runtime = wfdiag_native_update::NativeUpdateRuntime::start(service)?;
let reply = runtime.request_check()?;
```

`WindowsPackageSignatureProvider` is also public for callers that need the
shared Store-channel decision independently.
