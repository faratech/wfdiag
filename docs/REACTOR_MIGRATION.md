# Windows Reactor migration runbook

## Status

WFDiag's shipping UI remains React/Tauri. The `reactor-spike` package and
`reactor-baselines` contract are non-shipping feasibility work: they must not
replace, modify, or become a prerequisite for the current Store, MSI, NSIS, or
portable builds.

The selected direction is a parallel, pure-native WinUI 3 rewrite. The
Reactor application will manually compose native controls and will not host
WebView2, load the existing React bundle, execute JavaScript, or maintain a
web/native UI bridge. The six populated prototype screens establish that this
direction is feasible, but production cutover is currently blocked. The
authoritative machine-readable state is
[`reactor-baselines/manifest.json`](../reactor-baselines/manifest.json), and
the read-only checker is:

```bash
python3 scripts/check-reactor-readiness.py
python3 scripts/check-reactor-readiness.py --json
```

The command deliberately returns exit code 1 while any blocker remains. A
green result is a cutover prerequisite, not permission by itself.

The manifest also carries an explicit `backend_parity` matrix covering every
shipping service surface: diagnostics, monitoring, processes, Issues and
remediation, History, export/share, Settings and credentials, AI provider
management/chat/reporting, on-device AI, elevation, desktop integrations,
updates, tray lifecycle, single-instance behavior, and global commands. A
fixture-rendered screen cannot satisfy that gate. Each surface stays
`blocked` or `partial` until the native shell has direct integration evidence;
only evidence-backed `passed` entries allow the checker to go green.

## Native UI architecture

`reactor-spike/src/main.rs` is a hand-built Reactor `Component` with a native
title area, navigation rail, status bar, settings `ContentDialog`, and six
native pages: Diagnostics, Live Monitor, Processes, AI Analysis, Issues, and
History. Reactor builders create the real WinUI controls directly. Component
messages own navigation, theme, responsive state, process filtering, monitor
pause/resume, fixture chat, settings, and backend event projection.

There is intentionally no compatibility host around the React UI. Migrating a
screen means rebuilding its semantics and layout with Reactor controls, then
connecting it to the shared Rust application boundary. Any abandoned WebView2
host or JavaScript bridge design is out of scope for this migration.

Schema 2 of the readiness manifest makes that choice enforceable. It fixes
the implementation kind to `native_winui3_reactor`, fixes the inspected
source root to `reactor-spike/src`, and sets `webview_ui_allowed` to `false`.
The checker independently blocks direct Tauri, Wry, WebView/WebView2, CEF, or
similar browser-host dependencies, WebView API markers in Rust, and web
frontend assets under the native source root. Changing the manifest to permit
a WebView is itself an error.

The source interface uses wallpaper blur and CSS backdrop filtering that are
not exposed as a per-element brush by the pinned Reactor surface. The native
prototype therefore embeds deterministic pre-blurred derivatives of the
canonical light and OLED wallpapers and combines them with native tint,
border, and Acrylic layers. This keeps the artwork native and local while
approximating the source depth without a browser renderer.

## Pinned prototype

The feasibility spike pins both `windows-reactor` and
`windows-reactor-setup` to the reviewed `windows-rs` commit
`1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8`. The source identifies the
Reactor API as 0.100.0, but the usable release was not on crates.io when the
spike was created. A branch, tag, or floating Git dependency is prohibited.

The pin is prototype-only. Production requires an official, non-placeholder
release that contains the APIs WFDiag validated. Update the expected version,
revision, both Cargo dependencies, and the readiness contract together in one
reviewed change when that release exists.

On a Windows developer machine with the MSVC Rust target installed:

```powershell
cargo build --manifest-path reactor-spike/Cargo.toml --target x86_64-pc-windows-msvc
cargo run --manifest-path reactor-spike/Cargo.toml --target x86_64-pc-windows-msvc
```

Repeat with `aarch64-pc-windows-msvc` on or for an ARM64 test machine. The
spike's framework-dependent setup requires the matching Windows App Runtime;
it does not alter the shipping application's runtime or package manifest.

This Linux/WSL host can cross-check both Windows architectures:

```bash
(
  cd reactor-spike
  PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin check --target x86_64-pc-windows-msvc
  PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin check --target aarch64-pc-windows-msvc
)
```

On this configured ARM64 host, WSL interop can also launch a copied Windows
artifact and `scripts/capture-window.ps1` can capture it with `PrintWindow`.
That is useful Phase 0 evidence, but it is not a substitute for the clean x64
and ARM64 device matrix.

## UI-neutral backend seam

`crates/wfdiag-ui-core` is the first reusable migration boundary. It has no
Tauri, Reactor, WinUI, or Tokio runtime dependency and defines serializable
`UiEvent` contracts for diagnostics progress, live monitoring, chat/report
streaming, remediation status, and quick scans.

Its bounded event bus preserves chat, terminal, report, remediation, and
quick-scan events in FIFO order with backpressure. Replaceable system samples
use a single latest-value slot, while nonterminal task progress coalesces by
`(session_id, task_id)`. Lossless and progress lanes have independent additive
capacity wakeups so one lane cannot strand publishers in another.

`src-tauri/src/ui_event_adapter.rs` projects native monitor samples into this
contract. A closed native UI receiver stops the monitor loop before another
expensive sample is collected, including a generation check that prevents an
old loop from clearing a newly started loop. The current Tauri emitter remains
the production path.

The prototype accepts this contract through `Message::Backend`, owns a
`NativeMonitorRuntime`, and repeatedly drains the typed receiver into the
native Monitor cards without Tauri IPC. It also owns a
`NativeDiagnosticRuntime`: Quick and Full scans run the existing Windows
collectors on the Tokio worker, publish native progress/results, support
task-granular cancellation, reject stale-session writes, and preserve the last
usable snapshot when a replacement scan cannot start. The monitor runtime has
a separate nonblocking process-query command so full inventory work never
runs on the WinUI thread or bloats telemetry snapshots. The live Processes
screen now consumes that command with debounced filtering, sortable columns,
selection/details, 100-row paging, native virtualization, periodic refresh,
and request IDs that discard stale completions. Post-fix ARM64 desktop testing
passed pause/resume, refresh, filtering, PID sorting, selection, scrolling and
page 2 with no WebView modules, child processes, Application Error, or WER
reports. A responsive explicit row width compensates for WinUI
`ItemsRepeater`'s desired-width arrangement so every realized row aligns with
the fixed table header.

Deterministic fixture mode remains isolated for visual QA. Diagnostics are
still marked partial because task selection, result-detail interaction,
issue projection, export/share, and every production error path are not
complete. Processes remains partial until compact/collapsed
and x64 runtime coverage, accessibility, long-run refresh, and its complete
error-state matrix pass. Backend integration is complete only when every command,
stream, cancellation path, confirmation, persistence, and error state operates
through the native component without Tauri IPC or a web bridge.

Two additional framework-neutral workers now preserve production storage
semantics. `wfdiag-native-history` owns the sole DPAPI v2 encrypted-store
implementation and exposes nonblocking save/list/load, comparison, diff, tag,
label, trend, retention, and clear requests. A live ARM64 compatibility probe
decrypted the host's existing Store scan history. The Reactor component now
owns this runtime and renders the saved-scan list, filter, refresh, selection,
and comparison summary without Tauri IPC. Latest-scan selection cancels stale
comparison work and renders an explicit baseline state rather than an indefinite
loading placeholder. Completed scans now commit from the diagnostic runtime's
authoritative session snapshot, retain the Store's cancellable 500 ms auto-save
window, convert explicitly into the history contract, and use live settings for
auto-save, concurrency, custom Quick Scan selection, retention, and retention
limit. Auto-save failures remain scoped and nonfatal, while deterministic visual
modes never open history storage. Drill-down, tags/labels, trends, destructive
clear confirmation, and the full error/accessibility matrix remain open.
`wfdiag-native-settings` owns the canonical settings schema and typed provider
credential operations while Tauri adapters preserve the existing JSON path,
atomic writer, DPAPI/keyring identifiers, validation order, and secret
scrubbing. The live Reactor component now owns that runtime and renders every
visible non-secret Store 2.5.8 setting. Loading and saving happen off the WinUI
thread with request-ID stale-result rejection; Cancel restores the persisted
snapshot and preview theme, while Save commits only the exact payload acknowledged
by a matching successful worker response. ARM64 self-contained automation passed
cancel/reopen and save/restart against an isolated byte-for-byte copy of the Store
settings file, with the real Store file unchanged and no WebView or crash evidence.
Settings remains `partial` until provider credential entry/clearing, live model
discovery and subscription setup, settings-driven tray/startup/notification side
effects, x64 runtime, accessibility, and the complete validation/error matrix pass.

`wfdiag-native-ai-provider` owns the exact provider/preference/status wire
contracts, capability table, local-first routing policy, Store-identity
preference gate, pure status projection, shared response cache, shipping
Ollama/custom endpoint probes, and a nonblocking typed worker for status,
selection, cache clearing, and model discovery. `ProviderManagementService`
is now constructed by Tauri from the same reusable settings adapter and an
explicit `ProviderProbeBundle`. Reactor now instantiates the same service with
the shared settings adapter and concrete Foundry, Ollama, custom-endpoint, and
Codex/Claude CLI probes. `wfdiag-native-phi` is the sole owner of the reviewed
Windows AI projection, package-identity gate, LAF/token policy, activation
fallback, model cache, readiness, prompt-fit, and generation path. Its
`WindowsPhiStatusSource` runs the real shipping probe off the async worker and
returns the established Store-required result before any WinRT/DLL/LAF work in
an unpackaged process. Tauri retains only command adapters over this crate.
This surface remains `partial` until the full provider/model/bridge/loading/
error matrix is rendered. Chat/report streaming, credential values, fallback
consent, and the Phi-to-Aion API transition remain separate gates.

`wfdiag-native-issues` now owns the sole issue catalog and deterministic
detectors and exposes a nonblocking, request-ID-based detection worker with a
read-only remediation metadata snapshot. `wfdiag-native-export` preserves the
shipping JSON, text, HTML, WindowsForum, clipboard, and email renderers behind
a UI-neutral worker. These are also `partial`: Reactor has not yet submitted
completed scans to Issues or delivered exports through native pickers,
clipboard, filesystem, and allowlisted link operations.

`wfdiag-native-system` now owns the canonical machine, Windows-version,
elevation, process/native architecture, emulation, page-size, and processor
count projections behind a nonblocking worker. `wfdiag-native-update` owns the
Store-aware GitHub release policy and Windows package-signature provider,
including debug/Store silence and fail-closed signature handling. Both Tauri
commands delegate to these seams. Reactor now starts the native system worker,
renders its machine/OS/elevation result, exposes native/emulated architecture in
the machine-card automation name, and blocks scan selection until elevation is
known. Reactor also supplies the Store 2.5.8 update
delay/throttle semantics, a five-second informational notice, a native About
dialog, and a typed external-action boundary that reconstructs only an exact
allowlisted WFDiag GitHub release URL. Deterministic visual modes never start
the timer or touch update persistence; throttle path resolution and reads run
off the UI thread, and the notice pauses its exact remaining lifetime on hover
with epoch- and generation-scoped timer delivery. The update gate remains `partial`
pending packaged Store/direct x64 and ARM64 runtime, persistence,
accessibility, and browser-launch validation.

## Visual oracle

The baseline manifest records 18 populated, empty, compact, conversational,
and Settings audit states from the Store-signed WFDiag 2.5.8 package. It pins
each exact viewport/theme, source version, and SHA-256 under
`reactor-baselines/captures/store-2.5.8`; inherited 2.5.4 screenshots are not
accepted as the visual oracle.

The native pass retains same-viewport source-left/native-right evidence for
nine primary states under `reactor-spike/captures-2.5.8`: Diagnostics empty and
populated, Monitor populated, AI empty, Issues empty and populated, History
comparison, and Settings top use the `-reactor-current.png` / `-comparison-current.png`
pairs. The corrected Processes milestone uses
`live-process-validation/deterministic-processes-populated-1440x900.png` and
`live-process-validation/processes-populated-store-left-reactor-right.png`.

The final ARM64 candidate also has pairs for all 18 manifest states under
`reactor-spike/captures-2.5.8/final`. These current-version reviews drive
implementation but are not cutover approvals; themes, DPI values,
accessibility modes, and x64 visual evidence still require matched review.

Before implementing or approving a screen:

1. Launch the current signed Tauri application on Windows with deterministic,
   non-sensitive fixture data.
2. Put it into the state named by the baseline manifest and set the specified
   window dimensions, theme, scale factor, high-contrast setting, and
   reduced-motion setting.
3. Capture the application window with the repository helper. For example:

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass `
     -File scripts/capture-window.ps1 `
     -ProcessName wfdiag-tauri `
     -OutputPath reactor-baselines/captures/diagnostics-populated-dark-1440x900.png `
     -LogicalWidth 1440 `
     -LogicalHeight 900
   ```

4. Capture the equivalent Reactor state using process name
   `wfdiag-reactor-spike` while the prototype is active.
5. Record both paths, dimensions, hashes, state metadata, and review evidence
   in `reactor-baselines/manifest.json`. Never use screenshots containing API
   keys, usernames, machine-identifying paths, or real diagnostic data.
6. Review structure, content, brand, responsive behavior, interaction states,
   accessibility, and data visualization against every acceptance category in
   the manifest. Native WinUI control metrics, caption buttons, focus visuals,
   and Fluent symbols may differ; missing information, changed behavior, or
   lost brand hierarchy may not. The deterministic pre-blurred wallpaper is an
   explicit native approximation and must be judged in the same combined
   image, not treated as literal CSS backdrop blur.

Final evidence must include at least:

- Default 1200x800, wide 1440x900, compact 900x800, and minimum 720x540
  windows.
- Light, dark, system, and high-contrast themes.
- 100%, 150%, and 200% display scaling.
- Reduced-motion behavior and keyboard-only focus states.
- Empty, loading, populated, error, cancellation, confirmation, and long or
  streaming-content states for every applicable screen.

## Runtime and packaging gate

The Store manifest currently depends on
`Microsoft.WindowsAppRuntime.1.8`. The pinned Reactor setup stages Windows App
SDK 2.4 (`Microsoft.WindowsAppRuntime.2`). This is an intentional blocking
finding, not a version string to edit until validation is available.

A runtime-alignment decision requires all of the following on clean x64 and
ARM64 systems:

- Store package identity, publisher, PFN, `runFullTrust`, and
  `systemAIModels` read back unchanged after install.
- Aion and any retained Phi fallback behavior pass on real Copilot+ devices.
- The framework-dependent Store bundle installs, launches, updates, and
  uninstalls correctly without carrying duplicate 1.8/2.4 stacks.
- Self-contained MSI, NSIS, and portable-directory distributions carry the
  complete matching runtime and pass install, upgrade, uninstall, launch, and
  update checks.
- x64 and native ARM64 packages are signed and measured for installed size,
  startup, idle memory, and runtime file duplication.

The explicit `self-contained` spike feature completed native ARM64 staging and
startup validation. Packaging builds must run through native Windows Cargo:
Linux-side `cargo xwin` remains the compile/lint path, but the pinned setup
helper invokes Windows `curl.exe`/`tar.exe` for runtime staging, so its bare
cross-compiled executable is not a deployable self-contained candidate. The
startup gate detects the missing adjacent runtime and refuses to launch it.

The pinned setup helper initially emitted 38 files totaling
64,839,702 bytes, including an unused 1,203,040-byte
`Microsoft.Web.WebView2.Core.dll`. A copied candidate with that exact file
removed passed normal startup, direct-to-Settings startup, Settings open/close,
local `Microsoft.UI.Xaml.dll` loading, and Application Error/WER inspection.
`build.rs` now removes the unused projection after staging, leaving 37 files in
the tested candidate. The current native ARM64 build has executable SHA-256
`E748CDA9E9E89F9DFCDDE9BB3DEFB1387D1B70237EA9DD845CF7B019839B38EA`; its
pre-WinUI version probe returned `2.5.8`, and three normal starts, three
direct-to-Settings starts, and three UI Automation Settings open/close cycles
passed with complete PE alignment, local XAML, no WebView projection/module,
and zero new Application Error/WER events. The application has no WebView2
dependency, control, web UI, or bridge. This developer-machine validation
still does not pass the signed MSI/NSIS/portable clean-machine gate.

The same candidate also passed the fixture-free live-system gate at the Store
Diagnostics viewport: 1440x1000 logical at 144 DPI (2160x1500 physical). UI
Automation independently matched `ANDROMEDA`, Windows 11 Professional (25H2),
Standard user, and `Native ARM64 execution`; the local XAML/no-WebView checks,
graceful close, and Application Error/WER checks passed. Evidence is under
`reactor-spike/captures-2.5.8/live-system-validation`, including the Store-left/
Reactor-right combined review.

Do not change `AppxManifest.xml`, Store workflow manifests, bundled AI DLLs,
or the direct installer runtime based on the spike alone.

## Upstream API gates

Production must use official Reactor APIs; WFDiag will not maintain a private
Reactor fork or mutate Reactor-owned raw XAML objects out of band.

Two upstream gates are mandatory and cannot be removed from the readiness
manifest:

- `window_lifecycle`: close interception, close-to-tray, show/hide,
  minimize/maximize/restore, activation, and focus behavior sufficient to
  preserve the current tray and single-instance lifecycle.
- `global_accelerators`: native application-wide handling for Ctrl+K,
  Ctrl+1 through Ctrl+6, Ctrl+/, Ctrl+Shift+Q, and Ctrl+Shift+F, including
  correct behavior while focus is inside text controls and dialogs.

A gate may be changed to `passed` only with an official release/revision,
links to the upstream API or change, and automated plus manual WFDiag evidence.

## Validation commands

The checker tests use only temporary fixtures and verify that evaluation does
not change the inspected tree:

```bash
python3 -m unittest scripts/test_check_reactor_readiness.py
```

Run the repository check after the tests:

```bash
python3 scripts/check-reactor-readiness.py --json
```

The current expected blockers are:

- The Store 2.5.8 oracle and all 18 dark-theme ARM64 pairs are complete, but
  other theme/DPI/accessibility variants and x64 visual evidence are pending.
- Windows App Runtime 1.8 and Reactor's 2.4 target are not aligned.
- An official usable Reactor release is not selected.
- Window lifecycle and global accelerator APIs are not proven upstream.
- The native component is still fixture-driven and is not wired to the full
  production backend command/event, persistence, cancellation, confirmation,
  or desktop-service surface.
- Aion/Store, framework-dependent Store packaging, and self-contained direct
  packaging have not passed the required device matrices.

Unexpected `error` findings—missing contracts, changed checksums, Store
identity drift, unpinned dependencies, or omitted gates—are regressions and
must be corrected before further prototype work is trusted.

## Strict no-cutover gates

Tauri remains the only production frontend until every condition below is
satisfied in the same candidate revision:

1. `check-reactor-readiness.py` exits 0 with evidence-backed gates.
2. Shared backend contract tests pass through both the Tauri and Reactor
   adapters with preserved settings, credential, cache, and history formats.
3. Every baseline state passes native fidelity review at all required themes,
   window sizes, DPI settings, and accessibility modes.
4. Scan, monitoring, processes, issues/remediation, history, AI streaming and
   cancellation, settings, exports, elevation, tray, taskbar, notifications,
   clipboard, updates, and single-instance workflows pass on x64 and ARM64.
5. Narrator/UI Automation, keyboard-only operation, focus restoration, high
   contrast, text scaling, reduced motion, and chart alternatives pass.
6. Store certification and clean-machine Store/direct distribution testing
   pass with one validated Windows App Runtime strategy.
7. Startup, memory, UI latency, large-list scrolling, streaming throughput,
   and complete installed footprint meet the recorded Tauri baseline or have
   explicit approved exceptions.

The native rewrite may use documented Reactor and `windows` APIs for required
Windows integration, but it may not satisfy a gate by reintroducing WebView2,
embedding the Tauri frontend, or routing UI behavior through JavaScript.

Only after these gates pass should a separate cutover change make Reactor the
shipping entry point. Removal of React, Vite, Tauri, Node-dependent CI, and
`dist` staging belongs to a later cleanup change after a signed rollback tag
has been created.
