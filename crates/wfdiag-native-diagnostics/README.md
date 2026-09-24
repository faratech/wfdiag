# wfdiag-native-diagnostics

> **2026-09-23:** the Tauri rollback shell (`src-tauri`) and its React frontend
> were deleted. Passages below that compare against or mention the Tauri shell
> are historical context from the migration era; the native WinUI 3 shell
> (`apps/wfdiag`) is the only host of this crate.

Framework-neutral diagnostic orchestration for WFDiag.

The crate owns scan-session replacement, bounded parallel task execution,
task-granular cancellation, result retention, and typed `wfdiag-ui-core`
event delivery. Its executor is injectable, so the state machine is covered by
portable unit tests. On Windows, `NativeDiagnosticExecutor` reuses the proven
collectors in `src-tauri/src` without linking Tauri, Wry, WebView2, Reactor, or
a JavaScript IPC bridge.

`NativeDiagnosticRuntime::start` returns a runtime plus the UI-thread
`UiEventReceiver`. A native shell can start a session, launch a batch on its
worker runtime, cancel it, and read the same complete task evidence carried by
`UiEvent::DiagnosticResult`.

During migration the Windows collectors remain included from their existing
source files. This keeps one implementation while the shipping Tauri commands
and the native shell are transitioned onto the shared coordinator.
