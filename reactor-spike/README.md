# WFDiag native Windows Reactor prototype

This directory is a non-shipping, pure-native WinUI 3 rewrite prototype. Its shell and all six
screens are assembled manually from Windows Reactor controls; it does not host WebView2, load the
React application, execute JavaScript, or use a web/native UI bridge. It deliberately does not
replace or become a dependency of the shipping Tauri UI. The existing Rust backend now depends on
the separate framework-neutral `wfdiag-ui-core` event contract and exposes a native monitor sink so
both shells can share backend events during migration.

`scripts/check-reactor-readiness.py` enforces that boundary by scanning this crate's direct Cargo
dependencies and `src` tree. A WebView/browser-host dependency, WebView API marker, or web frontend
asset is a cutover blocker rather than an alternate route to parity.

The six populated screens are fixture-driven:

- Diagnostics
- Live Monitor
- Processes
- AI Analysis
- Issues
- History

The prototype embeds the current WFDiag badge and AI avatar from `public/wf-ds`. WinUI/Reactor does
not expose the CSS-style per-element backdrop blur used by the shipping UI at the pinned revision,
so `assets/bg-24H4-*-native-blurred.webp` are deterministic, pre-blurred derivatives of the two
canonical WF wallpapers. They retain the source artwork while approximating the existing acrylic
depth with native image, tint, border, and Acrylic window layers.

The dependency is pinned to Microsoft `windows-rs` revision
`1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8` because the documented Reactor 0.100 API is not yet a
usable crates.io release.

## Check from WSL

Use separate target directories for the two deployment modes. Reactor stages runtime files beside
the executable, so sharing one target directory could leave stale self-contained DLLs in a later
framework-dependent build.

Framework-dependent (default):

```bash
PATH=/usr/lib/llvm-20/bin:$PATH \
  CARGO_TARGET_DIR=target/framework-dependent \
  cargo xwin check --target aarch64-pc-windows-msvc

PATH=/usr/lib/llvm-20/bin:$PATH \
  CARGO_TARGET_DIR=target/framework-dependent \
  cargo xwin check --target x86_64-pc-windows-msvc
```

Do not select `self-contained` from WSL. At the pinned Reactor revision, the setup helper downloads
and extracts packages through `%SystemRoot%\System32\curl.exe` and `tar.exe`, which are available
only when the build script runs under Windows. `build.rs` rejects that cross-host combination so a
bare `.exe` cannot be mistaken for a distributable self-contained candidate. Use native Windows
Cargo for the direct-installer artifact check below.

## Run on Windows

By default, the build script selects Reactor's framework-dependent deployment and stages the
matching Windows App Runtime bootstrap DLL next to the binary. The current Reactor revision expects
Windows App Runtime 2.4 to be installed.

```powershell
cargo run --target aarch64-pc-windows-msvc
```

For direct-installer validation, enable `self-contained`. Reactor stages the matching Windows App
Runtime and its complete projection/runtime file set beside the executable, then embeds its
self-contained manifest. The installer must carry that upstream-staged set, not just the `.exe`.

The pinned setup helper otherwise reuses one extraction directory for every target architecture.
`build.rs` scopes that cache by `CARGO_CFG_TARGET_ARCH` and checks the PE machine type of the staged
Windows App Runtime and WinUI DLLs. A stale x64 extraction can therefore no longer be copied into an
ARM64 artifact (or the reverse); the build fails before an invalid package can reach startup.

```powershell
$env:CARGO_TARGET_DIR = "target/self-contained"
cargo build --release --target aarch64-pc-windows-msvc --features self-contained
```

Run that packaging build with native Windows Cargo. `cargo xwin` remains valid for Linux-side
compile and lint checks, but Reactor's pinned setup helper uses Windows `curl.exe`/`tar.exe` while
staging the self-contained runtime. A cross-compiled `.exe` without the adjacent runtime is an
incomplete candidate, and the startup gate below intentionally rejects it before launch.

Validate the complete `target/self-contained/aarch64-pc-windows-msvc/release` directory. It must
contain the executable, `Microsoft.WindowsAppRuntime.dll`, `Microsoft.UI.Xaml.dll`, and the other
Reactor-staged runtime files before it is handed to an MSI/NSIS packaging step.

Run the repeatable startup/Settings crash gate against that exact candidate directory:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File scripts/test-reactor-startup.ps1 `
  -Executable target/self-contained/aarch64-pc-windows-msvc/release/wfdiag-reactor-spike.exe `
  -Iterations 3
```

The gate verifies normal and direct-to-Settings startup, repeated Settings open/close through UI
Automation, PE-machine alignment for the executable and critical staged runtime DLLs, local XAML
runtime loading, and the absence of new Application Error/Windows Error Reporting events.

The current Store-2.5.8 parity candidate was rebuilt natively for ARM64 at commit-working-tree state
with SHA-256 `E748CDA9E9E89F9DFCDDE9BB3DEFB1387D1B70237EA9DD845CF7B019839B38EA`.
Its pre-WinUI probe reported `2.5.8`, and three normal starts, three direct-to-Settings starts, and
three UI Automation Settings open/close cycles passed with complete ARM64 runtime alignment, local
XAML loading, no WebView projection/module, and zero new Application Error or WER events. A separate
fixture-free run at the Store's 1440x1000 logical / 2160x1500 physical viewport projected the live
`ANDROMEDA`, Windows 11 Professional (25H2), Standard user, and native ARM64 identity through UI
Automation, captured the native frame, closed gracefully, and also produced zero crash events.

At the pinned setup revision, the helper stages `Microsoft.Web.WebView2.Core.dll` even for projects
that do not create a WebView. WFDiag's build script removes that exact unused projection after
staging. A copied self-contained candidate with the file absent passed normal startup,
direct-to-Settings startup, Settings open/close, local XAML loading, and crash-log validation before
the omission was made part of the build. The WFDiag crate has no WebView2 dependency, control,
browser UI, JavaScript bundle, or web/native bridge.

Keep this opt-in separate from the Store/MSIX workflow. It validates loose direct distribution and
does not supply the registered package identity or `systemAIModels` capability required by the
Store-only on-device AI path.

The prototype demonstrates a hand-built native title area and navigation rail, six responsive page
layouts, theme switching, pane collapse, process filtering, monitor pause/resume, chat fixture
interaction, a native settings surface built from WinUI controls, accessibility metadata, and
`Ctrl+R` refresh.
State and callbacks are owned by the Reactor `Component`; there is no DOM or IPC bridge between the
controls and the component.

Backend events intentionally enter through `Message::Backend`, so
`wfdiag-ui-core::UiEvent` is the typed shell boundary. The Reactor shell now owns a
`NativeMonitorRuntime`, drains its receiver without Tauri, and renders live CPU, memory, storage,
network, GPU, and NPU samples. `NativeDiagnosticRuntime` runs Quick and Full scans through the
existing Windows collectors with native progress/results, task-granular cancellation, and stale
session protection. The live Processes page consumes the monitor runtime's nonblocking full-process
queries with debounced filtering, sortable columns, native virtualization, 100-row paging, periodic
refresh, stale-request rejection, selectable details, and responsive full-width rows. An ARM64
expanded-desktop run passed pause/resume, refresh, filtering, PID sorting, selection, scrolling and
page 2 with native XAML loaded and zero WebView/WER evidence; compact/collapsed and x64 runtime
coverage remain open. The component also owns the native History worker and renders
the existing encrypted Store-compatible scan list, filtering, refresh, selection, and comparison
summary without Tauri IPC. The native Settings dialog now owns the shared settings runtime and
Store-compatible persistence path: it loads off-thread, edits every visible non-secret 2.5.8 field,
restores the persisted snapshot and preview theme on Cancel, and commits only a matching successful
Save response. ARM64 automation passed cancel/reopen and save/restart against an isolated Store-file
copy without touching the installed app's settings. Deterministic rows remain isolated to visual QA.
Result interaction, scan persistence, History drill-down/edit/clear/trends, Issues, AI credential and
model-discovery flows, export, remediation, and the remaining desktop services are still production
integration gates. The repository also contains
UI-neutral History, Settings, Issues, and Export workers that preserve the shipping DPAPI/history,
credential, detection, and report-rendering behavior. They remain partial until every success,
empty, loading, validation, confirmation, persistence, and error state is rendered or delivered by
the native component. A Tauri-free AI provider service now preserves the exact status/preference wire
contracts, capabilities, routing, Store-identity validation, shared response cache, Ollama/custom
network probes, and typed worker used by Tauri. Reactor can assemble the same explicit probe bundle,
but Phi-status, Foundry, and subscription-CLI concrete adapters still need extraction before live
integration; presentation and interaction are also open. The UI-neutral System and Update workers
also preserve the canonical Windows host, architecture/emulation, Store-signature, and GitHub
release policies. Reactor now renders live machine/OS/elevation state, exposes architecture and
emulation to UI Automation, delays scans until the elevation probe completes, and owns the
Store-compatible delayed update check, throttle, notification, native About dialog, and typed
allowlisted external actions. Packaged Store/direct x64 and ARM64 validation, complete accessibility
coverage, and explicit browser-action automation remain open.

## Current visual evidence

The visual oracle is the Store-signed WFDiag 2.5.8 package installed on this host. All 18 named
Store states are captured under `reactor-baselines/captures/store-2.5.8`. The final ARM64 candidate
has same-viewport source-left/native-right evidence for all 18 states under
`captures-2.5.8/final`, with a complete review sheet at
`captures-2.5.8/final/all-18-comparisons-contact.png`. The nine primary states also retain these
earlier evidence paths:

| State | Native capture | Combined review |
| --- | --- | --- |
| Diagnostics empty | `captures-2.5.8/diagnostics-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/diagnostics-empty-desktop-dark-comparison-current.png` |
| Diagnostics live system (ARM64) | `captures-2.5.8/live-system-validation/diagnostics-live-system-20260830-114311.png` | `captures-2.5.8/live-system-validation/diagnostics-empty-desktop-dark-live-system-store-left-reactor-right.png` |
| Diagnostics populated | `captures-2.5.8/diagnostics-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/diagnostics-populated-desktop-dark-comparison-current.png` |
| Live Monitor populated | `captures-2.5.8/monitor-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/monitor-populated-desktop-dark-comparison-current.png` |
| Processes populated | `captures-2.5.8/live-process-validation/deterministic-processes-populated-1440x900.png` | `captures-2.5.8/live-process-validation/processes-populated-store-left-reactor-right.png` |
| AI empty | `captures-2.5.8/ai-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/ai-empty-desktop-dark-comparison-current.png` |
| Issues empty | `captures-2.5.8/issues-empty-desktop-dark-reactor-current.png` | `captures-2.5.8/issues-empty-desktop-dark-comparison-current.png` |
| Issues populated | `captures-2.5.8/issues-populated-desktop-dark-reactor-current.png` | `captures-2.5.8/issues-populated-desktop-dark-comparison-current.png` |
| History comparison | `captures-2.5.8/history-comparison-desktop-dark-reactor-current.png` | `captures-2.5.8/history-comparison-desktop-dark-comparison-current.png` |
| Settings top | `captures-2.5.8/settings-top-desktop-dark-reactor-current.png` | `captures-2.5.8/settings-top-desktop-dark-comparison-current.png` |

These are current-version comparisons rather than inherited 2.5.4 references. They drive the
native geometry and styling but are not production parity approvals: light/system/high-contrast
themes, DPI variants, accessibility modes, and x64 visual evidence still require matched review.
See `design-qa.md` for the detailed assessment.

## Live backend surfaces (wired, no Tauri IPC)

Beyond the original monitor/diagnostics/process inventory, the following now run against the
real backend through the framework-neutral crates:

- **Export to file** — owner-validated `IFileSaveDialog` save picker, `SavedReport` rendering
  (text metadata decoration, `include_raw`), background write, dialog-cancel as a silent no-op.
- **AI chat** — streaming send over the shared chat-completions client (cloud OpenAI, Foundry
  Local, Ollama, custom endpoints), per-turn provider resolution from live settings/DPAPI keys,
  conversation context, stale-request rejection. Cancel button and the read-only tool registry
  are follow-ups; Phi/CLI/Anthropic/Gemini/DeepSeek transports report a clear gap.
- **AI scan report** — Generate/Cancel/streaming/cached/regenerate via the shared `ReportService`,
  with provider attribution; comparison baselines arrive with a later increment.
- **Issues & remediation** — maintenance and per-issue Run buttons execute through the shared
  `wfdiag-native-remediation` engine; Repair requires the explicit confirmation dialog (preview
  built from catalog constants only) and the engine's own tier gate re-checks. Ask AI / Propose
  fix plan prefill the chat input; model output never reaches execution.
- **Elevation** — restart-as-administrator via the shared `runas` relaunch (UAC dismissal is a
  no-op, not an error).
- **Desktop integration** — scan-completion toast (AUMID-bound; silent no-op unpackaged),
  single-instance mutex + activation-event focus handoff, tray icon with Show/Hide, Quick Scan,
  and Exit, close-to-tray honoring the settings toggle (comctl32 subclass, isolated in
  `window_support.rs` for a future Reactor-native swap).
- **Settings** — provider API key entry/clear per provider (DPAPI-backed, never settings.json)
  and Quick Scan task customization persisted through the normal Save path.
- **History** — tags editor and destructive clear behind an explicit confirmation, plus the
  existing list/filter/select/compare.

## Known Phase 0 gates

- Reactor must publish an official non-placeholder release before production adoption.
- The current public accelerator enum cannot represent WFDiag's full shortcut set (no main-row
  digits, `K`, `/`, or Shift; Control-only modifiers). The expressible subset is wired
  (Ctrl+R, Ctrl+Numpad1..6); the palette and shortcut list stay reachable from the titlebar.
- Window show/hide, close interception, tray restoration, and full `AppWindow` lifecycle APIs are
  not exposed by the pinned Reactor surface; the owner-approved interim path is the isolated
  Win32 interop in `window_support.rs` (comctl32 subclass + `Shell_NotifyIconW`).
- Remaining interaction/error/persistence depth — chat tools and cancel button, report comparison
  baselines, Phi/CLI/cloud-native transports in the native chat, history trends and task-diff
  drill-down, monitor lease/visibility parity and network connections view — is tracked in
  `reactor-baselines/manifest.json`'s `backend_parity` matrix.
- This host can launch and capture the Windows ARM64 executable through WSL interop. Cutover remains
  blocked on paired native reviews for the remaining 2.5.8 states,
  light/system/high-contrast and DPI coverage, accessibility validation, and x64 visual evidence.
- Store/MSIX and self-contained MSI/NSIS/portable deployment still require clean-machine x64 and
  ARM64 validation with one approved Windows App Runtime strategy.
