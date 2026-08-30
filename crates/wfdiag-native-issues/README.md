# wfdiag-native-issues

Portable issue detection for WFDiag's native UI shells.

The crate owns a dedicated worker/request API around the existing canonical
issue catalog and pure detector functions. A request carries completed native
diagnostic results plus an injected UTC timestamp and temporary-file count.
The worker never reads the clock, filesystem, registry, or environment, so a
request is deterministic and replayable.

`IssueRuntime::start_canonical` consumes the complete shared read-only catalog
from `wfdiag-remediation-catalog`. The lower-level `IssueRuntime::start` still
accepts an injected snapshot and validates that every remediation referenced by
the issue catalog resolves. Both paths embed the same `Issue` wire contract used
by the shipping Tauri frontend. This crate intentionally exposes no remediation
execution method.

## Native integration

- Reactor can call `IssueRuntime::start_canonical`; no Tauri or action-execution
  dependency is required.
- The native scan coordinator's `DiagnosticOutput` is an alias of this crate's
  `TaskResult`, so its completed result map can be moved directly into a
  request without translating or duplicating diagnostic payloads.
- Reading the temporary directory and current time remains a shell concern;
  capture both once when creating the request.
- Remediation staging, confirmation grants, elevation, execution, progress,
  and cancellation remain exclusively in the action broker and are not part
  of this crate.
