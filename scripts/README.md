# WFDiag scripts

Build, version, packaging, and validation tooling. Everything here is run from the repository
root unless stated otherwise.

## Version management

The version is defined once in `/version.json`:

```json
{
  "version": "2.5.8",
  "name": "WF Diagnostics",
  "description": "WindowsForum Diagnostic Tool"
}
```

`bump-version.py` is the implementation; `update-version.js` and `update-version.ps1` are
thin entry points that delegate to it.

```bash
python3 scripts/bump-version.py 2.5.9            # apply
python3 scripts/bump-version.py 2.5.9 --dry-run  # preview
node scripts/update-version.js 2.5.9             # equivalent
```

### Files updated (11)

| # | File | What changes |
| --- | --- | --- |
| 1 | `version.json` | the source of truth |
| 2 | `package.json` | npm package version |
| 3 | `package-lock.json` | root `version` and `packages[""].version` |
| 4 | `apps/wfdiag/Cargo.toml` | **native shell** `[package].version` |
| 5 | `src-tauri/Cargo.toml` | Tauri rollback shell `[package].version` |
| 6 | `src-tauri/tauri.conf.json` | Tauri config version |
| 7 | `AppxManifest.xml` | MSIX `Identity` version only (gets the `.0` suffix) |
| 8 | `src/components/AboutDialog.tsx` | version display (Tauri UI) |
| 9 | `src/App.tsx` | `APP_VERSION` constant (Tauri UI) |
| 10 | `src-tauri/tauri.msix.conf.json` | nested `msixVersion` (X.Y.Z.0) |
| 11 | `README.md` | version badges and headings |

Items 4 and 5 share one lock refresh: the script runs
`cargo update --offline -p wfdiag-tauri -p wfdiag` so the root `Cargo.lock` stops the next
`--locked` build from failing. If `cargo` is not on `PATH` the script warns and you must run
that command yourself before committing.

`bump-version.py` also reads `reactor-baselines/manifest.json` → `reactor_pin` for the
Windows App Runtime framework name and minimum version, so the Store manifest cannot drift
from the pinned Reactor runtime. Never hand-edit those values in `AppxManifest.xml`.

`check-version-sync.py` verifies every one of those files plus **both** `Cargo.lock` package
entries (`wfdiag` and `wfdiag-tauri`) and the native shell's version source
(`apps/wfdiag/build.rs` → `main.rs`). CI and release builds run it before packaging, and Rust
build commands use `--locked` so a release cannot silently resolve a different dependency
graph.

## Packaging

### Native Store package (the product)

`build-reactor-msix-probe.py` renders the Store manifest and stages/packs/bundles the native
shell. It is the *same* renderer used by `.github/workflows/build-and-publish-store.yml`, so
the shipped package and the local probe cannot diverge.

| Subcommand | Purpose |
| --- | --- |
| `stage --target {x64,arm64} --executable <exe> --bootstrap <dll> --output <dir>` | build the per-arch layout from a prebuilt `wfdiag.exe` and its `Microsoft.WindowsAppRuntime.Bootstrap.dll` |
| `pack --target <arch> --layout <dir> --package <file.msix>` | pack one staged layout into an unsigned MSIX |
| `bundle --packages-dir <dir> --bundle <file.msixbundle>` | bundle the packed per-arch MSIX files |
| `validate-layout --target <arch> <layout>` | check a staged layout |
| `validate-msix --target <arch> <package>` | check an unsigned MSIX |
| *(no subcommand)* | build a standalone framework-dependent probe end-to-end into `--output` (default `/mnt/c/code/wfdiag-reactor-store-probe`) |

Each rendered manifest derives from the canonical `AppxManifest.xml` and changes only the
executable, architecture, and Windows App Runtime dependency
(`Microsoft.WindowsAppRuntime.2`, minimum `2.4.0.0`). Only Reactor's pinned 2.4 bootstrap DLL
is staged: app-local Windows App Runtime/WinUI DLLs and the stale `src-tauri/resources/ai-sdk`
AI DLLs are rejected before and after packing. The script has no sign, install, registration,
upload, or publishing operation. See `docs/REACTOR_STORE_PROBE.md`.

`python3 -m unittest scripts/test_build_reactor_msix_probe.py` runs in CI (`store-identity`).

### Tauri rollback package

`python3 scripts/build-cross.py build-all --build-msix` builds the legacy Tauri Store/Phi
Silica MSIX. Microsoft signs the package distributed through the Store; `--sign` is only for
locally sideloadable test bundles. `src-tauri/tauri.msix.conf.json` is kept in version sync
only for basic Tauri MSIX experiments — it does not represent the Store package manifest.

`crates/wfdiag-native-phi/src/windows_ai_bindings.rs` is reviewed, tracked generated source.
Ordinary builds never rewrite it. Regenerate it explicitly with
`python3 scripts/build-cross.py generate-bindings`, review the diff, and commit the result
separately from a release build.

## Readiness and external-gate checkers

```bash
python3 scripts/check-reactor-readiness.py [--json]   # exit 1 while any gate is blocked
python3 scripts/check-external-gates.py [--json]      # exit 1 when an external change is actionable
python3 scripts/check-store-identity.py
python3 scripts/check-variants.py
python3 -m unittest scripts/test_check_reactor_readiness.py
python3 -m unittest scripts/test_check_variants.py
python3 -m unittest scripts/test_validation_harness.py
```

`check-reactor-readiness.py` is read-only and reports NOT READY on purpose while the five
hardware-gated cutover gates and the `backend_parity` matrix lack device evidence. Do not
weaken or bypass a gate to make it green. `check-external-gates.py` watches crates.io for a
real `windows-reactor` release, checks Store-manifest vs Reactor-runtime drift, and runs the
packaging pre-flight for `docs/validation/clean-machine-protocol.md`.

## Validation lanes (Windows only)

`validate-reactor.ps1` is the orchestrator; reports land under `validation-reports/`.

```powershell
.\scripts\validate-reactor.ps1 -Suite all
.\scripts\validate-reactor.ps1 -Suite startup,flows
```

| Lane | Script(s) | Covers |
| --- | --- | --- |
| `startup` | `test-reactor-startup.ps1` | normal + direct-to-Settings startup, Settings open/close via UIA, PE-machine alignment, local XAML, no new Application Error/WER |
| `live-system` | `test-reactor-live-system.ps1` | fixture-free live Diagnostics at 1440x1000; machine/OS/elevation/architecture UIA projection, footer version, browserless module state |
| `about` | `test-reactor-about-parity.ps1` | installed Store 2.5.8 About dialog beside a fresh native candidate at 150% DPI (strict oracle: exact package family/AUMID, ARM64, matching version probe) |
| `flows` | `test-reactor-ai-flows.ps1`, `test-reactor-chat.ps1`, `test-reactor-report.ps1`, `test-reactor-remediation.ps1` | hermetic AI lane (isolated settings + mock provider: streaming chat, mid-stream Stop, the exact-ten tool contract, a real `list_remediations` returning `open_disk_cleanup`, streamed report, forced Regenerate) plus the supplemental live-provider suites |
| `visual` | `capture-reactor-variants.ps1` + `check-variants.py` (`reactor-baselines/variants.json`), `test-reactor-process-refresh-parity.ps1` | theme and reduced-motion variants over deterministic fixtures, the open rendering-defect list, and the Processes refresh triptych |
| `x64` | `test-reactor-x64.ps1` | x64 evidence locally (emulated) and on a clean `windows-latest` runner |
| `readiness` | `check-reactor-readiness.py` | the manifest gates |
| `gates` | `check-external-gates.py` | crates.io / runtime drift / packaging pre-flight |

Every lane runs against a **validation candidate** — `cargo build --release -p wfdiag
--features self-contained,validation` — because the deterministic knobs and the
`--wfdiag-version-probe` entry point only exist under `validation`. A production-shaped build
(no features) compiles the probe out entirely, so it never answers and the harness rejects it
before any window is created — a release artifact cannot be mistaken for a validation
candidate.

Supporting pieces: `scripts/lib/ReactorUia.psm1` (hermetic launch, unique-button wait+invoke,
status-text scanning, crash events, graceful close, combined-image sheets, WebView guard),
`scripts/lib/mock-provider.py` (the isolated OpenAI-compatible mock),
`measure-reactor-resources.ps1` (startup/memory/footprint), and
`docs/validation/clean-machine-protocol.md` (the manual clean-machine and Store-certification
protocol with a sign-off table).

The deterministic AI lane and the hermetic Rust suites
(`cargo test -p wfdiag-native-ai-chat`, `-p wfdiag-native-ai-report`) run in
`.github/workflows/reactor-validation.yml` on an x64 + ARM64 matrix **without**
`continue-on-error`: a no-provider result is a failure.

## Native Reactor visual capture

`capture-reactor-baselines.ps1` launches the supplied Reactor executable once
for each of the 18 Store 2.5.8 states recorded in
`reactor-baselines/manifest.json`. It applies the exact viewport and
deterministic state variables, captures the launched PID rather than an
arbitrary process-name match, validates every PNG's dimensions, and restores
the caller's environment afterward. Before any WinUI window is created, the
script runs the executable's machine-readable version probe and rejects a
binary whose reported application version does not exactly match the pinned
baseline manifest. Unsupported older binaries time out and are rejected, so a
stale 2.5.4 development build cannot silently produce 2.5.8 evidence.

```powershell
.\scripts\capture-reactor-baselines.ps1 `
  -Executable C:\path\to\wfdiag.exe
```

For one-off interactive captures, `capture-window.ps1` accepts either
`-ProcessId` or `-ProcessName`. Automated evidence should use `-ProcessId`;
name-based capture rejects ambiguous visible instances.

`test-reactor-live-system.ps1` validates a built candidate without deterministic
fixtures. It launches live Diagnostics at the Store 2.5.8 1440x1000 viewport,
checks the native machine/OS/elevation/architecture UI Automation projection,
footer version, local XAML and browserless module state, writes logical,
physical and UIA evidence, closes the exact PID, and checks Application
Error/WER. Build and run the startup gate first; this script does not build.

```powershell
.\scripts\test-reactor-live-system.ps1 `
  -Executable C:\path\to\wfdiag.exe `
  -OutputDirectory C:\path\to\live-system-evidence `
  -HoldSeconds 2
```

`test-reactor-about-parity.ps1` captures the installed Microsoft Store 2.5.8
About dialog beside a freshly built native Reactor candidate. The oracle is
deliberately strict: it requires the exact Store-signed ARM64 2.5.8 package,
the exact package family/AUMID, a 150% (144 DPI) display, and no already-running
Store or candidate process. The candidate must be ARM64 and its version probe
must report 2.5.8. The script opens both dialogs through UI Automation, requires
the six exact control names plus the complete description, captures foreground
DWM-visible frames at 1440x1000, rejects blank/dim-overlay-only captures, and
writes source/native/combined PNGs and JSON evidence. It also verifies local
WinUI XAML, no loaded or staged WebView2 files, graceful exits, and Application
Error/WER. It does not build the candidate and refuses to overwrite a non-empty
output directory.

Run it only after the new ARM64 self-contained candidate has passed the startup
and live-system gates:

```powershell
.\scripts\test-reactor-about-parity.ps1 `
  -Executable C:\path\to\new-build\aarch64-pc-windows-msvc\release\wfdiag.exe `
  -OutputDirectory C:\Temp\wfdiag-reactor-about-2.5.8-new
```

`test-reactor-action-regressions.ps1` is the focused live UIA gate for the
partial-remediation disclosure, export-format fallback, Device Manager, and
administrator-relaunch regressions. It requires a validation candidate built
with `self-contained,validation`; the version probe rejects production
builds before launching a window. Empty/unsupported export values are loaded
from GUID-scoped temporary settings files, the native Text save picker is
cancelled, and Device Manager cleanup targets only the newly observed HWND.
No user setting or report is written.

The UAC phase is opt-in because automation must not drive the secure desktop.
With `-IncludeAdminRelaunch`, approve the Windows prompt manually; the script
then verifies old-process exit, one new elevated candidate, and its visible
window. If Windows integrity isolation prevents the non-elevated harness from
closing that child, the evidence identifies the exact PID for manual cleanup.

```powershell
.\scripts\test-reactor-action-regressions.ps1 `
  -Executable C:\path\to\validation-build\wfdiag.exe `
  -OutputDirectory C:\Temp\wfdiag-reactor-action-regressions

# Interactive UAC handoff coverage:
.\scripts\test-reactor-action-regressions.ps1 `
  -Executable C:\path\to\validation-build\wfdiag.exe `
  -IncludeAdminRelaunch
```

## Release workflow

1. Bump the version: `python3 scripts/bump-version.py X.Y.Z` (see the table above), then
   `python3 scripts/check-version-sync.py` to confirm all 11 files and both `Cargo.lock`
   entries agree.
2. Review and commit with an explicit pathspec — never `git add -A`:
   `git diff` → `git commit -- version.json package.json package-lock.json Cargo.lock ...`
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`. That triggers
   `.github/workflows/build-and-publish-store.yml`, which builds `wfdiag.exe` for x64 and
   ARM64 and packages them through `build-reactor-msix-probe.py`. `workflow_dispatch` accepts
   a `shell` input (`reactor` — the default and the product — or `tauri` for the rollback).
4. The workflow uploads an **unsigned** bundle; Microsoft signs the package delivered through
   the Store. `--sign` in `build-cross.py` is only for locally sideloadable test bundles.
