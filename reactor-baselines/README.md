# Reactor visual baseline contract

`manifest.json` is the machine-readable visual oracle for the browserless
native UI migration. It maps every currently available audit screenshot to a screen,
state, theme, viewport, and checksum; protects the reusable brand assets; and
records the acceptance dimensions that require human review.

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

Schema 2 also protects the chosen architecture. The candidate must use native
Reactor/WinUI controls from `apps/wfdiag/src`; WebView-hosted parity is not
allowed. The validator rejects direct browser-shell dependencies, WebView API
markers, and HTML/CSS/JavaScript/TypeScript frontend assets in that source
tree. An unused WebView2 projection DLL staged by the upstream self-contained
Windows App Runtime is a packaging artifact and does not satisfy or violate
this UI rule by itself.

Run the read-only validator from the repository root:

```bash
python3 scripts/check-reactor-readiness.py
python3 scripts/check-reactor-readiness.py --json
```

A non-zero result is expected during the prototype. Do not weaken or bypass a
gate to make the command green; resolve the prerequisite and record its
evidence instead.
