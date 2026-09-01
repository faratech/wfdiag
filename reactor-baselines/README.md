# Reactor visual baseline contract

`manifest.json` is the machine-readable visual oracle for the browserless
native UI **and the single source of truth for the release pins**. It maps every
currently available audit screenshot to a screen, state, theme, viewport, and
checksum; protects the reusable brand assets; records the acceptance dimensions
that require human review; and carries the Reactor revision and Windows App
Runtime floor. It is read by `scripts/check-reactor-readiness.py`,
`scripts/check-external-gates.py`, and `scripts/bump-version.py`.

The baseline contains 18 durable dark-theme captures from the installed,
Store-signed WFDiag 2.5.8 application in
`reactor-baselines/captures/store-2.5.8`. This complete core state set covers
empty, populated, comparison, settings, issue handoff, and desktop/compact AI
states without relying on the older ignored `.playwright-mcp` artifacts.

Every screenshot records its own `source_application_version`. The validator
compares that field with `version.json` and reports a blocker if even one
state came from an older application version. Replacing a capture therefore
requires updating its path, dimensions, checksum, and source version. The
top-level `baseline.application_version` must also match the shipping version;
it does not override or hide stale per-screenshot provenance.

The `current_baseline_capture` gate remains blocked because production parity
sign-off still requires the planned light, high-contrast, DPI, and
reduced-motion coverage plus attached review evidence. Completing the 18
core dark-theme captures is necessary evidence, but is not by itself the
entire gate.

Schema 2 also protects the chosen architecture. `ui_architecture` fixes the kind to
`native_winui3_reactor`, the inspected `source_root` to `apps/wfdiag/src` (the
native shell package `wfdiag` / `wfdiag.exe`; it was `reactor-spike/src` before
the 2026-09-01 rename), and `webview_ui_allowed` to `false`. The candidate must
use native Reactor/WinUI controls from that source root; WebView-hosted parity is
not allowed. The validator rejects direct browser-shell dependencies, WebView API
markers, and HTML/CSS/JavaScript/TypeScript frontend assets in that source
tree. An unused WebView2 projection DLL staged by the upstream self-contained
Windows App Runtime is a packaging artifact and does not satisfy or violate
this UI rule by itself.

## Pins carried here

`reactor_pin` is authoritative for three values that must never be hand-edited
anywhere else:

- `revision` — the reviewed `microsoft/windows-rs` commit the shell builds
  against (a **revision**, never a branch or tag), mirrored in
  `apps/wfdiag/Cargo.toml` and the root `Cargo.lock`.
- `windows_app_runtime_framework` / `windows_app_runtime_min_version` — the
  `PackageDependency` written into `AppxManifest.xml`
  (`Microsoft.WindowsAppRuntime.2`, minimum `2.4.0.0`).
- `decision_record` — the anchor for the owner decision that closed the
  official-release gate: `docs/REACTOR_MIGRATION.md#cutover-decision-2026-09-01`.

Moving any of them means updating the manifest, both Cargo dependencies, and
`scripts/build-reactor-msix-probe.py` together in one reviewed change.

## Running the validator

Read-only, from the repository root:

```bash
python3 scripts/check-reactor-readiness.py
python3 scripts/check-reactor-readiness.py --json
```

A non-zero exit is expected: the cutover to the native shell is decided, but the
five hardware gates (`current_baseline_capture`, `native_control_parity`,
`aion_store_validation`, `store_packaging_validation`,
`direct_distribution_validation`) and the 19-surface `backend_parity` matrix
still need real x64/ARM64 device evidence. Do not weaken or bypass a gate to
make the command green; resolve the prerequisite and record its evidence
instead.
