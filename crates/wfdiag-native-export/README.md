# wfdiag-native-export

Deterministic, UI-framework-neutral export generation for WFDiag.

The crate renders the same raw/redacted JSON, text, and HTML content as the
shipping `export_results` command. It also owns the current WindowsForum post,
Copy Report `[CODE]`, and email preparation templates. Locale-sensitive date
strings and system metadata are explicit inputs; completed diagnostics use the
canonical shared `TaskResult` type.

`ExportRuntime` owns a dedicated worker thread. A WinUI dispatcher can enqueue
an owned request and poll the typed reply receiver without doing JSON parsing,
HTML escaping, or report assembly on the UI thread.

The crate deliberately does not:

- display save dialogs or choose paths;
- validate or write filesystem destinations;
- access the clipboard;
- URL-encode or launch WindowsForum/mail links;
- read the clock or system identity.

Those delivery concerns remain shell-owned. Tauri retains its established
path-validation and atomic delivery behavior, while Reactor can connect these
payloads to native pickers, clipboard APIs, and the future action/event layer.
