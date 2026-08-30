# wfdiag-native-history

UI-framework-neutral scan-history persistence and command runtime for WFDiag.

## What is shared

- The shipping `%APPDATA%\wfdiag-tauri\scans` location.
- The existing `.enc` filename convention and v2 envelope.
- Current-user Windows DPAPI encryption with UI disabled.
- Atomic temp-file + flush + replace writes.
- Summary-index rebuild/recovery behavior.
- Save, list, load, compare, summary compare, task diff, tags, label,
  failure trends, retention cleanup, and clear-history semantics.
- Conservative storage-ID validation.

`src-tauri/src/results_storage.rs` remains the single comparison/storage
semantic source during migration and is compiled by this crate as well as the
shipping backend. `src-tauri/src/encrypted_storage.rs` is now only a re-export
of this crate's DPAPI store, so Tauri and native WinUI cannot drift into two
persistence formats.

## Native UI usage

Create `HistoryRuntimeConfig` with the same settings-backed retention callback
and the live diagnostic executor's task catalog, then call
`NativeHistoryRuntime::start`. Every `request_*` method is nonblocking: it
enqueues work and returns a Tokio oneshot receiver. The dedicated history
thread performs JSON, DPAPI, and filesystem work away from the WinUI thread.

```rust,no_run
use std::path::PathBuf;
use wfdiag_native_history::{HistoryRuntimeConfig, NativeHistoryRuntime};

let config = HistoryRuntimeConfig::new(
    PathBuf::from(r"C:\Users\me\AppData\Roaming\wfdiag-tauri\scans"),
    || (true, 30),
    Vec::new,
);
let history = NativeHistoryRuntime::start(config)?;
let list_reply = history.request_list()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`list_reply` should be awaited on an application worker and the result
marshalled to WinUI through `DispatcherQueue`; do not synchronously block the
UI thread waiting for it.

## Current migration limitations

- The crate and Tauri currently define structurally identical `TaskResult` and
  diagnostic-metadata types because those domain contracts still live inside
  the Tauri source tree. Serialization is identical, so existing scan files
  remain compatible, but converting a completed native diagnostic session to
  a history record still maps these four fields. Moving the contracts into a
  small shared domain crate later would make that conversion zero-copy.
- `HistoryRuntimeConfig::shipping_defaults` uses the shipping path and the
  historical default of 30 scans, but it cannot observe Tauri settings.
  Production Reactor wiring should inject the native settings service's live
  retention callback with `HistoryRuntimeConfig::new`.
- DPAPI-protected files are intentionally readable only by the same Windows
  user. Portable non-Windows tests use plaintext payload bytes inside the same
  v2 envelope; that fallback is test support, not a shipping mode.
- The worker serializes history operations. This intentionally prevents index
  races and is appropriate for user-driven history traffic; bulk import is not
  currently an API.
