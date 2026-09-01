# WFDiag native WinUI 3 shell (`wfdiag.exe`)

This crate is **the shipping WFDiag application** as of the owner's 2026-09-01 cutover
decision (`docs/REACTOR_MIGRATION.md#cutover-decision-2026-09-01`). It is a pure-native
WinUI 3 shell built on `windows-reactor`: its chrome and all six screens are assembled
manually from native controls. It does not host WebView2, load the React application,
execute JavaScript, or use a web/native UI bridge. The Tauri + React shell (`src-tauri`,
`src/`) is kept buildable as a rollback until a later cleanup release.

`scripts/check-reactor-readiness.py` enforces that boundary by scanning this crate's direct
Cargo dependencies and `src` tree. A WebView/browser-host dependency, a WebView API marker,
or a web frontend asset is a blocker, not an alternate route to parity.

## What lives here (and what does not)

This crate contains **no engine logic**. Everything testable without a window lives in the
workspace crates under `crates/`; the shell depends on those, on `windows`, and on
`windows-reactor` — nothing else. `main.rs` is a thin entry point: panic hook, version
probe, single-instance decision, `App::run_component::<WfdiagShell>`.

| Module | Contents |
| --- | --- |
| `app/` | the root Reactor `Component`: `mod.rs` (state + `Component` impl), `state.rs`, `message.rs`, `consts.rs`, `policy.rs` (pure decisions), `tasks.rs` (background helpers), `orchestration/` (`actions`, `analysis`, `chat`, `export`, `history`, `issues`, `lifecycle`, `providers`, `report`, `scan`, `settings`, `subscriptions`, `update`) |
| `screens/` | `diagnostics`, `monitor`, `processes`, `ai`, `issues`, `history` — each a `view.rs` |
| `dialogs/` | `about`, `action_review`, `palette`, `settings` |
| `widgets/` | `badges`, `cards`, `chrome`, `icons`, `markdown_render`, `palette_colors`, `table` |
| `platform/` | every Win32/WinRT/WinUI edge: `window` (subclass, revisioned lifecycle snapshots, keyboard hook), `instance`, `notifications`, `save_picker`, `external`, `focus`, `winui_focus_bindings`, `ui_wake`, `crash` |
| `ai/` | `chat_tools` (this shell's tool backend) and `report` (Phi-aware resolvers) over the shared AI runtimes |
| `fixtures/` | deterministic visual fixtures and the environment knobs (`knobs.rs`), gated behind the `validation` feature. Nothing here runs in a production build |

Backend events enter through `Message::Backend`, so `wfdiag_ui_core::UiEvent` is the typed
shell boundary. State and callbacks are owned by the Reactor `Component`; there is no DOM and
no IPC bridge between the controls and the component.

The shell embeds the current WFDiag badge and AI avatar from `public/wf-ds`. WinUI/Reactor
does not expose the CSS-style per-element backdrop blur used by the previous UI at the pinned
revision, so `assets/bg-24H4-*-native-blurred.webp` are deterministic, pre-blurred derivatives
of the two canonical WF wallpapers, combined with native image, tint, border, and Acrylic
layers.

## Cargo features

| Feature | Purpose |
| --- | --- |
| *(default)* | **Framework-dependent.** `build.rs` stages only the matching `Microsoft.WindowsAppRuntime.Bootstrap.dll` beside the executable; the machine must have Windows App Runtime 2.4 installed. This is what the Store package ships. |
| `self-contained` | Stages the complete Windows App Runtime beside the executable for direct-installer (MSI/NSIS/portable) validation. **Native Windows Cargo only** — see below. |
| `settings-test-path` | Enables the exact-path settings store used by integration validation. Never enable it for a Store or direct-installer production artifact. |
| `validation` | Superset of `settings-test-path`. Also compiles in `src/fixtures/knobs.rs` — every environment knob (`WFDIAG_REACTOR_*`, `WFDIAG_NO_*`) and the `--wfdiag-version-probe` entry point. Without it the shell performs **no** environment reads at all and every knob is a compile-time production default (#186, #212). Never enable it for a production artifact. |

## Dependency pin

`windows-reactor`, `windows-reactor-setup`, and `windows-core` are pinned to Microsoft
`windows-rs` revision `1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8` (Reactor 0.100.0 source),
because crates.io still publishes only the placeholder `0.0.0` for both Reactor crates. The
pin is a **revision, never a branch**. Moving it means updating both Cargo dependencies,
`reactor-baselines/manifest.json` (`reactor_pin`), and `scripts/build-reactor-msix-probe.py`
together in one reviewed change. `scripts/check-external-gates.py` watches crates.io so the
eventual move to an official release happens as an ordinary dependency update.

## Check from WSL

Use separate target directories for the two deployment modes. Reactor stages runtime files
beside the executable, so sharing one target directory could leave stale self-contained DLLs
in a later framework-dependent build.

Framework-dependent (default):

```bash
PATH=/usr/lib/llvm-20/bin:$PATH \
  CARGO_TARGET_DIR=target/framework-dependent \
  cargo xwin check --target aarch64-pc-windows-msvc

PATH=/usr/lib/llvm-20/bin:$PATH \
  CARGO_TARGET_DIR=target/framework-dependent \
  cargo xwin check --target x86_64-pc-windows-msvc
```

Workspace-wide lint from the same host:

```bash
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin clippy --workspace \
  --target x86_64-pc-windows-msvc -- -D warnings
```

Do not select `self-contained` from WSL. At the pinned Reactor revision, the setup helper
downloads and extracts packages through `%SystemRoot%\System32\curl.exe` and `tar.exe`,
which are available only when the build script runs under Windows. `build.rs` rejects that
cross-host combination so a bare `.exe` cannot be mistaken for a distributable self-contained
candidate.

## Run on Windows

```powershell
cargo run -p wfdiag --target aarch64-pc-windows-msvc
```

For direct-installer validation, enable `self-contained`. Reactor stages the matching Windows
App Runtime and its complete projection/runtime file set beside the executable, then embeds
its self-contained manifest. The installer must carry that upstream-staged set, not just the
`.exe`.

The pinned setup helper otherwise reuses one extraction directory for every target
architecture. `build.rs` scopes that cache by `CARGO_CFG_TARGET_ARCH` and checks the PE
machine type of the staged Windows App Runtime and WinUI DLLs, so a stale x64 extraction can
no longer be copied into an ARM64 artifact (or the reverse); the build fails before an invalid
package can reach startup.

```powershell
$env:CARGO_TARGET_DIR = "target/self-contained"
cargo build --release -p wfdiag --target aarch64-pc-windows-msvc --features self-contained
```

Run that packaging build with native Windows Cargo. `cargo xwin` remains valid for Linux-side
compile and lint checks, but a cross-compiled `.exe` without the adjacent runtime is an
incomplete candidate, and the startup gate below intentionally rejects it before launch.

Validate the complete `target/self-contained/aarch64-pc-windows-msvc/release` directory. It
must contain the executable, `Microsoft.WindowsAppRuntime.dll`, `Microsoft.UI.Xaml.dll`, and
the other Reactor-staged runtime files before it is handed to an MSI/NSIS packaging step.

At the pinned setup revision, the helper stages `Microsoft.Web.WebView2.Core.dll` even for
projects that do not create a WebView. `build.rs` removes that exact unused projection after
staging, leaving 37 files in the tested candidate. The crate has no WebView2 dependency,
control, browser UI, JavaScript bundle, or web/native bridge.

Keep the `self-contained` path separate from the Store/MSIX workflow. It validates loose
direct distribution and does not supply the registered package identity or `systemAIModels`
capability required by the Store-only on-device AI path.

## Validation

The orchestrator runs every Windows suite and writes reports under `validation-reports/`:

```powershell
.\scripts\validate-reactor.ps1 -Suite all
# or one lane:
.\scripts\validate-reactor.ps1 -Suite startup|live-system|about|flows|visual|x64|readiness|gates
```

The startup/Settings crash gate, run against an exact candidate directory:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File scripts/test-reactor-startup.ps1 `
  -Executable target/self-contained/aarch64-pc-windows-msvc/release/wfdiag.exe `
  -Iterations 3
```

It verifies normal and direct-to-Settings startup, repeated Settings open/close through UI
Automation, PE-machine alignment for the executable and critical staged runtime DLLs, local
XAML runtime loading, and the absence of new Application Error / Windows Error Reporting
events. Every suite runs the executable's machine-readable version probe *before* any WinUI
window is created and rejects a binary whose reported version does not match the pinned
baseline, so a stale build cannot produce current-version evidence.

`.github/workflows/reactor-validation.yml` runs the hermetic lanes on an x64 + ARM64 matrix.
The deterministic AI lane is release-gating and has no `continue-on-error`: a no-provider
result is a failure. The live-provider and live-system suites remain supplemental until
hosted-runner UIA and device coverage are stable.

### Validated ARM64 candidate (2.5.8)

The Store-2.5.8 parity candidate was rebuilt natively for ARM64 with executable SHA-256
`E748CDA9E9E89F9DFCDDE9BB3DEFB1387D1B70237EA9DD845CF7B019839B38EA`. Its pre-WinUI probe
reported `2.5.8`, and three normal starts, three direct-to-Settings starts, and three UI
Automation Settings open/close cycles passed with complete ARM64 runtime alignment, local XAML
loading, no WebView projection or module, and zero new Application Error or WER events. A
separate fixture-free run at the Store's 1440x1000 logical / 2160x1500 physical viewport
projected the live `ANDROMEDA`, Windows 11 Professional (25H2), Standard user, and native ARM64
identity through UI Automation, captured the native frame, closed gracefully, and produced zero
crash events. This is developer-machine validation; it does not satisfy the signed
MSI/NSIS/portable clean-machine gate.

## Shortcuts and window lifecycle

Ctrl+R and Ctrl+Numpad1..6 use Reactor accelerators. The isolated Win32 subclass in
`platform/window.rs` supplies Ctrl+K, main-row Ctrl+1..6, Ctrl+/, Ctrl+Shift+Q, and
Ctrl+Shift+F through the UI-thread lifecycle poll, with editable-control, overlay, and
active-scan guards; the same subclass publishes coherent revisioned visibility/minimize/focus
snapshots that drive monitor pause/resume and close-to-tray. Both were accepted as the
implementation for this release by the 2026-09-01 decision; an official Reactor API for either
remains a tracked follow-up, not a blocker.

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

- **Diagnostics and Issues** — Quick/Full progress, task-granular cancellation, stale-session
  rejection, selectable result details, custom Quick Scan selection, optional history auto-save,
  targeted-rerun overlay/rollback, and export/share entry points are live. Each finalized
  authoritative or merged result map drives the shared
  issue detector with request/session/epoch guards; returned issues and maintenance actions render
  natively and refresh after every successful remediation, including when completion occurs off-page.
- **Export and share** — owner-validated `IFileSaveDialog` save picker, `SavedReport` rendering
  (text metadata decoration, `include_raw`), background write, and dialog-cancel as a silent no-op.
  Actual picker and path-policy errors retain their detail and surface in the native status bar.
  WindowsForum sharing renders on the worker, copies through the Windows clipboard via the WinRT
  DataTransfer API, and opens only the allowlisted new-thread target. The shared email payload now
  builds a recipient-free, percent-encoded draft URI that excludes the full report, and Reactor's
  visible Email Report command queues the shared renderer, copies the full body to the Windows
  clipboard, and asks the typed shell adapter to open only that unsent draft. Render, clipboard,
  and mail-client launch failures remain user-visible without exposing report contents in errors.
- **AI chat** — the worker starts by default and resolves each turn from live settings/DPAPI keys.
  OpenAI, Anthropic, Gemini, and DeepSeek APIs; Foundry Local, Ollama, and custom OpenAI-compatible
  endpoints; signed-in Codex and Claude Code subscriptions; and package-bound Phi route through the
  shared transports. Streaming, immediate cancellation, conversation context, stale-request
  rejection, exactly-one terminal delivery, backend-owned New Conversation, and the canonical
  exact-ten bounded tool catalog are wired. Auto clean-failure retry honors the persisted
  Ask/Allow/Never cloud decision, explicit providers never fall back, and fallback responses disclose
  the local-to-cloud transition and retain `ProviderUse` attribution. The tools cover targeted
  diagnostics, live Windows grounding, scan scope/issues/history/comparison/live stats, remediation
  listing, Full Scan consent, and remediation staging. The closed parser rejects unknown
  operations/IDs/properties/unbounded text; Full Scan and remediation calls emit typed UI requests
  only and never execute either action.
- **AI scan report** — the default worker uses the same full provider routing through the shared
  `ReportService`; Generate/Cancel/streaming/provider attribution and force-refresh Regenerate are
  wired with exactly-one terminal delivery. The service cache fast path and complete inline
  cached-body restoration are pinned by focused tests. Reactor obtains the newest different stored
  scan off-thread for changed-since-last-scan evidence; compact evidence and Auto Phi-to-next-local
  rerouting are shared and tested.
- **Issues & remediation** — maintenance and per-issue Run buttons execute through the shared
  `wfdiag-native-remediation` engine only after an opaque, expiring, one-use action proposal is
  reviewed and revalidated against current issue/catalog fingerprints. Repair requires a second
  explicit confirmation and the engine's own tier gate re-checks. `stage_remediation` enters this
  same broker and never executes directly. Ask AI immediately sends current issue evidence through
  native chat. Prioritize / Propose fix plan uses a cancellable native worker plus the shared strict
  catalog-ID parser; stale plans are discarded, provider/fallback attribution is visible, and each
  selected or batch action must still pass the normal fingerprinted review/confirmation broker.
- **Elevation** — restart-as-administrator via the shared `runas` relaunch (UAC dismissal is a
  no-op, not an error).
- **Desktop integration** — scan-completion toast (AUMID-bound; silent no-op unpackaged),
  single-instance mutex + activation-event focus handoff, and a cached exact main-window HWND that
  remains valid while hidden. The tray icon provides Show/Hide, Quick Scan, and Exit; restore handles
  hidden and minimized windows, and close-to-tray honors the settings toggle. The isolated comctl32
  subclass publishes coherent revisioned visibility/minimize/focus snapshots, which pause monitoring
  while the window is unusable and resume plus refresh only a lifecycle-owned pause. The same
  isolated subclass publishes the full shipping Ctrl+K, main-row Ctrl+1..6, Ctrl+/,
  Ctrl+Shift+Q/F shortcut set for the component's UI-thread policy checks; the official Reactor API
  gate remains open.
- **Settings** — provider API key entry/clear per provider (DPAPI-backed, never settings.json)
  plus provider endpoint/model fields, cancellable model catalogs with stale/manual fallback, and
  Quick Scan task customization persisted through the normal Save path. Saved close-to-tray updates
  the live hook and saved notification state gates scan-completion toasts. Persisted scan-on-startup
  waits for settings and system initialization, consumes its gate before starting exactly one Quick
  Scan, and is suppressed in visual mode. Codex/Claude account status/auth workers and their Check,
  Sign in, Sign out, and Cancel controls are connected to component state. Configured-provider setup
  seeding, Phi readiness selection/Save gating, the `claude-sonnet-5` Anthropic default, and the
  confirmed process-tree-contained winget/vendor installer are wired; live
  startup/catalog/CLI/browser/account/install evidence remains open.
- **History** — independent label and tag editors, tag-aware filtering/fallback, all-category
  comparison rows, stale-safe lazy side-by-side task details, bounded structured JSON leaf diffs,
  recurring-failure trend badges, and destructive clear behind explicit confirmation, plus automatic
  Store-compatible post-scan persistence and baseline refresh when the newest scan changes.

The deterministic AI lane is release-gating. The shared chat crate's integration and bounded-tool
unit tests exercise the streaming client, tool loop, cancellation, all ten schemas, and strict
argument validation against a local Rust mock. On Windows, `scripts/test-reactor-ai-flows.ps1` starts
an isolated Python provider and settings file and proves custom-provider readiness, rendered streaming
chat, mid-stream Stop, the exact-ten tool-name contract, a real `list_remediations` result containing
`open_disk_cleanup`, streamed report content, and a fresh deterministic Regenerate through UI
Automation. Shared report tests separately cover the cache fast path and inline cached-body
restoration. All of these gates run in
`.github/workflows/reactor-validation.yml` without `continue-on-error`; a no-provider result fails.
The separate live-provider/system suites remain supplemental until hosted-runner UIA and device
coverage are stable.

## Known gates

Updated 2026-09-01. Three gates that were open when this file called the crate a prototype
were **closed by the owner's cutover decision** and are recorded as `passed` in
`reactor-baselines/manifest.json`:

- ~~Reactor must publish an official non-placeholder release before production adoption.~~
  **Closed.** The release ships on the reviewed exact revision; crates.io still publishes only
  `0.0.0`, and migrating to an official release is a follow-up dependency update.
- ~~The public accelerator enum cannot represent WFDiag's full shortcut set.~~ **Closed for
  this release.** Ctrl+R/Ctrl+Numpad1..6 keep native Reactor accelerators; the isolated Win32
  keyboard hook in `platform/window.rs` supplies Ctrl+K, main-row Ctrl+1..6, Ctrl+/,
  Ctrl+Shift+Q and Ctrl+Shift+F with pure policy tests. An official Reactor accelerator API
  remains a tracked follow-up.
- ~~Window show/hide, close interception, tray restoration, and `AppWindow` lifecycle APIs are
  not exposed by the pinned Reactor surface.~~ **Closed for this release.** The isolated Win32
  interop in `platform/window.rs` plus the cached exact HWND in `platform/instance.rs` (was
  `window_support.rs` / `instance_support.rs` before the module decomposition) is the accepted
  implementation. An official Reactor window-lifecycle API remains a tracked follow-up.

Still open — all of these need real x64/ARM64 hardware evidence, not more code:

- Remaining interaction/error/persistence depth — provider-specific live AI and packaged
  Phi/Aion evidence, the model-catalog/subscription-account and cloud-fallback
  provider/error/accessibility matrices, live issue-to-chat/fix-plan evidence, external
  email-client/clipboard validation, and monitor/network error/accessibility/soak validation —
  is tracked in `reactor-baselines/manifest.json`'s `backend_parity` matrix (all 19 surfaces
  `partial`, `on_device_ai_and_package_identity` `blocked`).
- Visual parity: paired native reviews for light/system/high-contrast and DPI coverage,
  accessibility validation, and x64 visual evidence (`current_baseline_capture`,
  `native_control_parity`). This Linux host can launch and capture the Windows ARM64 executable
  through WSL interop, which is useful evidence but not a substitute for the device matrix.
- Store/MSIX and self-contained MSI/NSIS/portable deployment still require clean-machine x64
  and ARM64 validation (`store_packaging_validation`, `direct_distribution_validation`,
  `aion_store_validation`). The Windows App Runtime strategy itself is decided:
  `Microsoft.WindowsAppRuntime.2`, MinVersion 2.4.0.0.

Run `python3 scripts/check-reactor-readiness.py` for the authoritative current state; it exits
1 while any gate is blocked, and that is expected.
