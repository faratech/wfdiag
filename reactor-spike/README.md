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

All six screens support deterministic fixture modes for visual comparison; the normal candidate
also wires live framework-neutral backend workers:

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
usable crates.io release. As checked with `cargo search` on 2026-08-31, both `windows-reactor` and
`windows-reactor-setup` still publish only the placeholder `0.0.0` version even though the reviewed
source declares `0.100.0`; the official-release cutover gate therefore remains blocked.

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
layouts, theme switching, pane collapse, process filtering, lifecycle-aware monitor pause/resume,
live AI chat/report interaction, a native settings surface built from WinUI controls, accessibility
metadata, and the shipping shortcut set. Ctrl+R and Ctrl+Numpad1..6 use Reactor accelerators; the
isolated Win32 window subclass supplies Ctrl+K, main-row Ctrl+1..6, Ctrl+/, Ctrl+Shift+Q, and
Ctrl+Shift+F through the UI-thread lifecycle poll with editable-control, overlay, and active-scan
guards. This implements current behavior without pretending that the pinned Reactor accelerator
API can express those chords.
State and callbacks are owned by the Reactor `Component`; there is no DOM or IPC bridge between the
controls and the component.

Backend events intentionally enter through `Message::Backend`, so
`wfdiag-ui-core::UiEvent` is the typed shell boundary. The Reactor shell now owns a
`NativeMonitorRuntime`, drains its receiver without Tauri, and renders live CPU, memory, storage,
network, GPU, and NPU samples. `NativeDiagnosticRuntime` runs Quick and Full scans through the
existing Windows collectors with native progress/results, task-granular cancellation, and stale
session protection. Targeted reruns use an overlay transaction over the committed scan: the prior
rows, session, scan kind, and task set survive until the one authoritative target result can be
replaced or appended; issue detection then receives the complete merged evidence. Reruns do not
create one-row auto-save records, and every failed/cancelled delivery path restores the base snapshot.
The live Processes page consumes the monitor runtime's nonblocking full-process
queries with debounced filtering, sortable columns, native virtualization, 100-row paging, periodic
refresh, stale-request rejection, selectable details, and responsive full-width rows. An ARM64
expanded-desktop run passed pause/resume, refresh, filtering, PID sorting, selection, scrolling and
page 2 with native XAML loaded and zero WebView/WER evidence; compact/collapsed and x64 runtime
coverage remain open. The component also owns the native History worker and renders
the existing encrypted Store-compatible scan list, tag-aware filtering and fallback labels,
refresh, selection, all-category comparison rows, independent label/tag editing, and lazy
side-by-side task details without Tauri IPC. JSON task outputs additionally receive bounded
leaf-level Added/Removed/Changed rows with overflow disclosure, while non-JSON retains the raw
side-by-side view; comparison rows include recurring-failure trend badges. The native Settings
dialog now owns the shared settings runtime and Store-compatible persistence path: it loads
off-thread, edits every visible non-secret 2.5.8 field, restores the persisted snapshot and preview
theme on Cancel, and commits only a matching successful Save response. ARM64 automation passed
cancel/reopen and save/restart against an isolated Store-file copy without touching the installed
app's settings. Deterministic rows remain isolated to visual QA. Provider-specific model-catalog
and subscription account-flow evidence, live startup-scan/picker/email-client evidence, and the
remaining error/accessibility/device matrices are still production integration gates. The
repository also contains UI-neutral History, Settings, Issues, and Export workers that preserve the shipping
DPAPI/history, credential, detection, and report-rendering behavior. Their implemented paths are
live; the manifest entries remain partial until those explicit gaps and live matrices close. A
Tauri-free AI provider service now preserves the exact status/preference wire contracts,
capabilities, routing, Store-identity validation, shared response cache, Ollama/custom network
probes, and typed worker used by Tauri. Reactor instantiates the same explicit probe bundle
with concrete Phi, Foundry Local, Ollama, custom-endpoint, and Codex/Claude CLI sources and renders
the resolved live provider state. Auto clean-failure retries now honor persisted Ask/Allow/Never
cloud consent; explicit provider choices never fall back, and local-to-cloud transitions disclose
their `ProviderUse` fallback attribution. Settings now drives cancellable live model-catalog
requests from its current draft configuration, renders selectable results and explicit
blocked/loading/error/stale state, and retains manual model entry. The typed Codex/Claude account
worker and component handlers cover status and cancellable sign-in/sign-out operations; Settings
renders Check, Sign in, Sign out, and Cancel with operation status, detail, and errors. The setup
browser seeds from configured provider fields, follows explicit Active AI choices, and remains
independently browsable under Auto. Phi activation and Save are guarded by live availability/readiness
and its backend reason while the setup pane remains reachable. The shared Anthropic default is
`claude-sonnet-5`.

The explicit Install CLI action is now wired for Codex and Claude Code. The allowlisted winget
package requires confirmation; unavailable or failed winget returns a structured vendor PowerShell
fallback that requires a second confirmation and never runs automatically. The installer executes
off-UI inside a Windows Job Object so cancellation, timeout, drop, or worker shutdown terminates its
entire process tree. Provider-specific live catalog/account/browser/install,
credential/fallback-error, accessibility, and packaged Phi/Aion validation remain open. The
UI-neutral System and Update workers also preserve the canonical Windows host,
architecture/emulation, Store-signature, and GitHub release policies. Reactor now renders live
machine/OS/elevation state, exposes architecture and emulation to UI Automation, delays scans until
the elevation probe completes, and owns the
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

## Known Phase 0 gates

- Reactor must publish an official non-placeholder release before production adoption.
- The current public accelerator enum cannot represent WFDiag's full shortcut set (no main-row
  digits, `K`, `/`, or Shift; Control-only modifiers). Ctrl+R/Ctrl+Numpad1..6 retain native Reactor
  accelerators and the isolated Win32 subclass supplies the missing shipping chords, but this
  reviewed fallback does not close the official-Reactor-API or live keyboard/focus gate.
- Window show/hide, close interception, tray restoration, and full `AppWindow` lifecycle APIs are
  not exposed by the pinned Reactor surface; the owner-approved interim path is isolated Win32
  interop in `window_support.rs` plus the cached HWND in `instance_support.rs`. Its lifecycle
  snapshots and monitor visibility wiring are implemented, but do not close the official API gate.
- Remaining interaction/error/persistence depth — provider-specific live AI and packaged Phi/Aion
  evidence, the model-catalog/subscription-account and cloud-fallback provider/error/accessibility
  matrices, live issue-to-chat/fix-plan evidence, external email-client/clipboard validation, and
  monitor/network error/accessibility/soak validation — is tracked in `reactor-baselines/manifest.json`'s
  `backend_parity` matrix.
- This host can launch and capture the Windows ARM64 executable through WSL interop. Cutover remains
  blocked on paired native reviews for the remaining 2.5.8 states,
  light/system/high-contrast and DPI coverage, accessibility validation, and x64 visual evidence.
- Store/MSIX and self-contained MSI/NSIS/portable deployment still require clean-machine x64 and
  ARM64 validation with one approved Windows App Runtime strategy.
