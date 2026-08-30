# wfdiag-native-system

UI-framework-neutral architecture and host system information for WFDiag.

The crate owns the canonical `ProcessorArchitecture`, `ArchitectureInfo`,
shipping `ArchitectureSnapshot`, and `SystemInfo` contracts. Its Windows
collector preserves the existing `IsWow64Process2` behavior (including native
`UNKNOWN`/`TARGET_HOST` handling), the `GetSystemInfo` fallback, Windows-version
registry projection, and token-elevation check used by Store 2.5.8.

`SystemRuntime` provides an unbounded request queue and typed reply channel so a
WinUI dispatcher never performs registry or token queries itself. The provider
boundary is injectable for deterministic tests.

This surface is deliberately read-only. It does not restart or elevate the
process, register/query package identity, initialize Windows App SDK, or touch
Phi Silica. Those operations remain outside this crate and require separate
policy/action boundaries.
