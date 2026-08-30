# wfdiag-native-monitor

Framework-neutral native Windows telemetry for WFDiag.

The crate owns the monitoring runtime and publishes typed `SystemStats`
through `wfdiag-ui-core`'s coalescing event bus. It has no Tauri, WebView2, or
Windows Reactor dependency, so both UI shells can consume the same collector.
`NativeMonitorRuntime::request_processes` queues full process filtering,
sorting, and pagination on the same worker and returns an awaitable one-shot
result, keeping enumeration off the WinUI thread and out of the one-second
telemetry payload.

During the migration, the proven collector implementation is included from
`src-tauri/src`; the Cargo boundary ensures that UI framework dependencies do
not leak back into it.
