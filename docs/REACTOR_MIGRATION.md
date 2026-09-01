# Windows Reactor migration runbook

## Status

**Cutover decided (2026-09-01).** The native WinUI 3 shell `apps/wfdiag` (package and binary
`wfdiag` / `wfdiag.exe`) is the shipping product. The Tauri + React shell (`src-tauri`,
`src/`) is kept buildable as a rollback and is packaged only through the Store workflow's
`shell: tauri` input; removing it is a later release. The full decision record, including the
three gates that were waiting on Microsoft rather than on this repository, is at the bottom
of this document: [Cutover decision (2026-09-01)](#cutover-decision-2026-09-01).

The direction is unchanged from the original plan: a pure-native WinUI 3 application that
composes native controls directly. It does not host WebView2, load the React bundle, execute
JavaScript, or maintain a web/native UI bridge. That is enforced, not merely intended —
`reactor-baselines/manifest.json` schema 2 fixes the implementation kind to
`native_winui3_reactor`, fixes the inspected source root to `apps/wfdiag/src`, and sets
`webview_ui_allowed` to `false`.

### What is done

- The engine is a Rust workspace of framework-neutral crates under `crates/`; both shells are
  thin hosts over them. No engine crate `#[path]`-includes `src-tauri` any more; `src-tauri`
  keeps one-line `pub use` shims.
- `wfdiag-app` is the application-service facade (`AppService { start, snapshot, dispatch,
  drain, shutdown }`) with mockable ports and headless integration suites that drive the real
  service on Linux.
- The native shell is decomposed: a thin `main.rs` over `app/`, `screens/`, `dialogs/`,
  `widgets/`, `platform/`, `ai/`, and `fixtures/`.
- Release plumbing is native-first: `AppxManifest.xml` launches `wfdiag.exe` and depends on
  `Microsoft.WindowsAppRuntime.2` (MinVersion `2.4.0.0`), single-sourced from
  `reactor-baselines/manifest.json` → `reactor_pin`; the Store workflow builds the native
  shell per architecture and packages it through
  `scripts/build-reactor-msix-probe.py {stage,pack,bundle,validate-msix}`;
  `.github/workflows/reactor-validation.yml` is an x64 + ARM64 matrix.
- `cutover.official_reactor_release`, `upstream.window_lifecycle`, and
  `upstream.global_accelerators` are closed by the owner decision.

### What remains

Everything still open is **hardware evidence**, not design. The authoritative
machine-readable state is
[`reactor-baselines/manifest.json`](../reactor-baselines/manifest.json), and the read-only
checker is:

```bash
python3 scripts/check-reactor-readiness.py
python3 scripts/check-reactor-readiness.py --json
```

It deliberately returns exit code 1 while any blocker remains, and will keep reporting NOT
READY until the five remaining cutover gates (`current_baseline_capture`,
`native_control_parity`, `aion_store_validation`, `store_packaging_validation`,
`direct_distribution_validation`) and every `backend_parity` surface have real x64/ARM64
evidence. Do not weaken or bypass a gate to make the command green; produce the evidence
through `scripts/validate-reactor.ps1 -Suite all` and
[`docs/validation/clean-machine-protocol.md`](validation/clean-machine-protocol.md), and
record it in the manifest.

The `backend_parity` matrix covers every shipping service surface: diagnostics, monitoring,
processes, Issues and remediation, History, export/share, Settings and credentials, AI
provider management/chat/reporting, on-device AI, elevation, desktop integrations, updates,
tray lifecycle, single-instance behavior, and global commands. A fixture-rendered screen
cannot satisfy that gate. Each surface stays `blocked` or `partial` until the native shell
has direct integration evidence; only evidence-backed `passed` entries allow the checker to
go green.

## Native UI architecture

The shell's root `Component` (`apps/wfdiag/src/app/mod.rs`; `main.rs` is now only a thin
entry point) is hand-built with a native
title area, navigation rail, status bar, settings `ContentDialog`, and six
native pages: Diagnostics, Live Monitor, Processes, AI Analysis, Issues, and
History. Reactor builders create the real WinUI controls directly. Component
messages own navigation, theme, responsive state, process filtering,
lifecycle-aware monitor pause/resume, live AI chat/report flows, deterministic
visual fixtures, settings, and backend event projection.

There is intentionally no compatibility host around the React UI. Migrating a
screen means rebuilding its semantics and layout with Reactor controls, then
connecting it to the shared Rust application boundary. Any abandoned WebView2
host or JavaScript bridge design is out of scope for this migration.

Schema 2 of the readiness manifest makes that choice enforceable. It fixes
the implementation kind to `native_winui3_reactor`, fixes the inspected
source root to `apps/wfdiag/src`, and sets `webview_ui_allowed` to `false`.
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

> **Superseded in part (2026-09-01).** The pin itself is unchanged and still exact, but it is
> no longer "prototype-only": the owner decided to ship on this reviewed revision. See
> [Cutover decision](#cutover-decision-2026-09-01). Moving the pin still requires updating both
> Cargo dependencies, `reactor-baselines/manifest.json` (`reactor_pin`), and
> `scripts/build-reactor-msix-probe.py` in one reviewed change. Historical text follows.

The feasibility spike pins both `windows-reactor` and
`windows-reactor-setup` to the reviewed `windows-rs` commit
`1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8`. The source identifies the
Reactor API as 0.100.0. A fresh `cargo search` on 2026-08-31 still reports
only the placeholder `0.0.0` releases for both crates, so the usable API is
not available from crates.io. A branch, tag, or floating Git dependency is
prohibited.

The pin is prototype-only. Production requires an official, non-placeholder
release that contains the APIs WFDiag validated. Update the expected version,
revision, both Cargo dependencies, and the readiness contract together in one
reviewed change when that release exists.

On a Windows developer machine with the MSVC Rust target installed:

```powershell
cargo build --manifest-path apps/wfdiag/Cargo.toml --target x86_64-pc-windows-msvc
cargo run --manifest-path apps/wfdiag/Cargo.toml --target x86_64-pc-windows-msvc
```

Repeat with `aarch64-pc-windows-msvc` on or for an ARM64 test machine. The
spike's framework-dependent setup requires the matching Windows App Runtime;
it does not alter the shipping application's runtime or package manifest.

This Linux/WSL host can cross-check both Windows architectures:

```bash
(
  cd apps/wfdiag
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
usable snapshot when a replacement scan cannot start. A targeted rerun against
a committed scan now uses an overlay transaction: progress is staged without
mutating the committed rows, the one authoritative completed result replaces
or appends only its task, the prior session/scan kind/task IDs are retained,
and a previously absent target is appended. Issue detection receives the
complete merged evidence. Targeted reruns never
create a one-row auto-save record, and start/run/cancel/delivery failures restore
the prior snapshot unchanged. The monitor runtime has a separate nonblocking
process-query command so full inventory work never runs on the WinUI thread or
bloats telemetry snapshots. The live Processes
screen now consumes that command with debounced filtering, sortable columns,
selection/details, 100-row paging, native virtualization, periodic refresh,
and request IDs that discard stale completions. Post-fix ARM64 desktop testing
passed pause/resume, refresh, filtering, PID sorting, selection, scrolling and
page 2 with no WebView modules, child processes, Application Error, or WER
reports. A responsive explicit row width compensates for WinUI
`ItemsRepeater`'s desired-width arrangement so every realized row aligns with
the fixed table header.

Deterministic fixture mode remains isolated for visual QA. Diagnostics also
wire Settings-backed custom Quick Scan selection, selectable result details,
authoritative post-scan issue projection, Store-compatible optional history
auto-save, and native export/share entry points. Diagnostics remain partial
until the complete admin/non-admin, cancellation-race, collector-error,
x64/ARM64, interaction, and accessibility matrices pass. Processes remains partial until compact/collapsed
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
modes never open history storage. The live History page now edits tags, loads
failure trends, edits a user label independently from metadata tags, and clears
all history behind explicit destructive confirmation. Label and filter fallback
matches the Store flow (`label` -> first tag -> `Scan`). Regressed, recovered,
and output-changed tasks all render as expandable rows; expanding one lazily
requests only that task's stored outputs, rejects stale request generations,
and shows explicit loading/error state plus side-by-side previous/current text.
When both outputs are JSON, the detail also renders bounded leaf-level Added,
Removed, and Changed rows (with an overflow count) above the raw documents;
non-JSON output retains the side-by-side fallback. Comparison rows now carry the
same recurring-failure trend badge policy as the shipping screen. A refreshed
list also recomputes an existing comparison when its latest-scan baseline
changes. The live loading/error/x64/ARM64/accessibility interaction matrix
remains open.

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
Saved close-to-tray state now updates the live window hook and saved notification
state gates scan-completion toasts. A one-use startup gate reads the persisted
`scan_on_startup` value, waits for both settings and system initialization to
finish, then starts exactly one Quick Scan; deterministic visual mode always
suppresses it. The provider setup browser seeds from the actually configured
draft/provider fields when Settings opens and follows an explicit Active AI
selection, while Auto still permits browsing another setup pane without
changing activation. Phi selection and Save are guarded by the live
availability/readiness state and backend reason; its setup pane remains
accessible even when Phi cannot be activated. Settings remains `partial` until
live startup-scan evidence, provider-specific model-catalog and subscription
account/install-flow validation, side-effect interaction testing, x64 runtime,
accessibility, and the complete validation/error matrix pass.

`wfdiag-native-ai-provider` owns the exact provider/preference/status wire
contracts, capability table, local-first routing policy, Store-identity
preference gate, pure status projection, shared response cache, shipping
Ollama/custom endpoint probes, and a nonblocking typed worker for status,
selection, cache clearing, and model discovery. `ProviderManagementService`
is now constructed by Tauri from the same reusable settings adapter and an
explicit `ProviderProbeBundle`. Reactor now instantiates the same service with
the shared settings adapter and concrete Phi, Foundry Local, Ollama, custom
endpoint, and Codex/Claude CLI probes, then renders the resolved provider state
in the native AI workspace. `wfdiag-native-phi` is the sole owner of the
reviewed Windows AI projection, package-identity gate, LAF/token policy,
activation fallback, model cache, readiness, prompt-fit, and generation path.
Its `WindowsPhiStatusSource` runs the real shipping probe off the async worker
and returns the established Store-required result before any WinRT/DLL/LAF
work in an unpackaged process. Tauri retains only command adapters over this
crate. Reactor also wires the cloud-fallback consent policy: an Auto-selected
local provider may retry a clean failure only after the persisted Allow choice
or an Ask → Allow decision; Never is persisted, explicit provider selections
never fall back, and every local-to-cloud transition is disclosed with
`ProviderUse` fallback attribution. The native Settings dialog now drives a
cancellable off-UI model-catalog worker from the current draft endpoint, CLI
path, and available credential state. It renders Refresh/Cancel, selectable
catalog results, blocked/loading/error state, and a stale-last-successful-list
fallback while retaining manual model entry. Codex/Claude account status and
cancellable sign-in/sign-out operations have a typed worker and component
handlers. Settings renders Check, Sign in, Sign out, and Cancel controls with
operation status, detail, and errors; account checks also participate in catalog
refresh. Anthropic's empty-model default remains exactly
`claude-sonnet-5` in the shared catalog service and the Reactor Settings hint.
Reactor now also exposes the shared subscription CLI installer only through an
explicit Settings action. The allowlisted Codex/Claude winget package requires
confirmation; missing or failed winget produces a structured vendor-fallback
offer that requires a second, method-specific confirmation and is never invoked
silently. Installation runs off the UI thread in a Windows Job Object, so
cancellation, timeout, runtime drop, or worker teardown terminates the complete
winget/PowerShell process tree rather than only its direct child. This surface
remains `partial` until provider-specific live catalog/account/browser/install/
error evidence, accessibility, and the Phi-to-Aion transition pass their live
matrices.

`wfdiag-native-issues` now owns the sole issue catalog and deterministic
detectors and exposes a nonblocking, request-ID-based detection worker with a
read-only remediation metadata snapshot. Reactor commits every finalized
authoritative scan into that worker, guards completions by request/session/
epoch, renders the returned issues and maintenance actions, and refreshes them
after every successful remediation even if the user navigated away while it
was running. `wfdiag-native-export` preserves
the shipping JSON, text, HTML, WindowsForum, clipboard, and email renderers
behind a UI-neutral worker. Reactor wires its native save picker, path/format
validation, background file write, clipboard copy, and allowlisted
WindowsForum launch. Picker cancellation remains a silent no-op, while actual
dialog/path failures retain their detail and appear in the native status UI.
The shared email payload now also produces a recipient-free, fully
percent-encoded `mailto:` draft URI that never contains the full diagnostic
report, and Reactor owns a typed `ShellExecuteW` adapter that can open only
that unsent draft after an explicit user action. The visible Email Report
command now queues the shared email renderer; successful completion copies the
full report body to the Windows clipboard and opens only the recipient-free
unsent draft, while render, clipboard, and shell-launch failures retain a typed
user-visible status.
Issues stays `partial` until its populated/empty/error/
remediation-refresh and device, interaction, and accessibility matrices pass.
Export stays `partial` until the picker/error/write/clipboard/browser/mail,
external-client, device, and accessibility matrices pass.

**Progress note (updated 2026-08-31, AI/lifecycle integration pass).** The
extraction and wiring frontier moved substantially across the reviewed series:

- `wfdiag-native-ai-report` is authoritative: src-tauri's report command now
  delegates to it (wire-identical events, pinned by tests), and the Reactor
  shell starts its report worker by default and generates through the same
  service. Streaming, cancellation, provider attribution, cache identity, and
  exactly-one terminal delivery plus cached-body rehydration are wired for the
  full provider set. Reactor resolves the newest different stored scan on the
  history worker for changed-since-last-scan evidence, and the shared compact
  evidence plus Auto Phi-to-next-local reroute policies are pinned by tests.
- The shared chat-completions client moved into `wfdiag-native-ai-chat`
  (re-exported by src-tauri), with `resolve_compat_config` +
  `CompatChatProvider` + `provider_config_fingerprint` in the compat layer.
  Reactor starts the chat worker by default and resolves each turn from live
  DPAPI-backed settings. OpenAI, Anthropic, Gemini, and DeepSeek API providers;
  Foundry Local, Ollama, and custom OpenAI-compatible endpoints; signed-in
  Codex and Claude Code subscriptions; and package-bound Phi all route through
  their shared transports. Auto clean-failure retry honors persisted
  Ask/Allow/Never cloud consent, explicit provider selection never falls back,
  and the UI discloses the local-to-cloud transition with fallback attribution.
  Immediate in-flight cancellation, exactly-one terminal projection,
  backend-owned conversation context/New Conversation, and the canonical
  exact-ten bounded tool catalog are wired. The catalog is
  `run_diagnostic`, `search_windows_knowledge`, `get_scan_summary`,
  `request_full_scan`, `get_detected_issues`, `compare_with_previous_scan`,
  `get_live_stats`, `list_remediations`, `list_scan_history`, and
  `stage_remediation`. Its closed parser rejects unknown operations, IDs,
  properties, and unbounded text. Full Scan and remediation tools create only
  typed consent/proposal events; neither can start a scan or execute a fix.
- The save picker is wired end to end (format → owner-validated dialog →
  `SavedReport` render → background write). Cancellation and typed picker
  failure are distinct; only cancellation is silent, while failures surface in
  the status UI.
- Remediation execution extracted to `wfdiag-native-remediation` (with the
  tier confirm gate and injectable runner). Maintenance, per-issue Run, and AI
  staging now enter the same opaque action broker: catalog-only preview,
  expiring one-use proposal, current-state fingerprint revalidation, explicit
  approval, and a second Repair confirmation before the shared engine runs.
- Issue assistance is native end to end. Ask AI immediately sends structured
  current-issue evidence into chat. Prioritize / Propose fix plan runs on a
  cancellable off-UI worker, shares Tauri's bounded catalog-ID-only prompt and
  strict parser, displays provider/fallback attribution, discards stale scan or
  catalog results, and can pass only revalidated issue/remediation pairs into
  the same tiered action broker. The model has no execution capability.
- Elevation moved into the crate (`elevation::relaunch_self_elevated`) and
  powers restart-as-administrator; single instance (mutex + activation
  event) and scan-completion toasts (AUMID-bound, silent-unpackaged) are
  live; tray + close-to-tray + Show/Hide/Quick Scan/Exit run through the
  isolated Win32 subclass in `apps/wfdiag/src/platform/window.rs` per the
  owner-approved interop interpretation of the lifecycle gate. The exact
  Reactor HWND is cached independently of visibility, restore/show targets
  that handle, and an atomic revisioned lifecycle snapshot now feeds window
  visibility/focus/minimize changes back to the component. Monitoring pauses
  while the window is unusable and resumes with a refresh only when the pause
  was lifecycle-owned.
- Command palette and shortcut help render as native dialogs. Ctrl+R and
  Ctrl+Numpad1..6 retain Reactor accelerators; the isolated Win32 window
  subclass supplies the otherwise-unrepresentable Ctrl+K, main-row Ctrl+1..6,
  Ctrl+/, Ctrl+Shift+Q, and Ctrl+Shift+F chords through the UI-thread lifecycle
  poll. Component policy blocks commands behind overlays, suppresses non-palette
  shortcuts in editable controls, and blocks scan shortcuts while a scan is
  active. The pinned `AcceleratorKey` enum still lacks main-row digits, `K`,
  `/`, and Shift, so this shipping-behavior fallback does not close
  `upstream.global_accelerators`.
- Settings gained DPAPI-backed provider key entry/clear, live cancellable model
  catalogs with manual fallback, configured-provider setup seeding, Phi
  readiness selection/Save gates, and Quick Scan task customization. Typed
  subscription status/sign-in/sign-out/cancel operations and the confirmed,
  process-tree-contained winget/vendor installer are integrated; live CLI,
  browser, account, and installer evidence remains open. Anthropic's default is
  pinned to `claude-sonnet-5`. History gained independent label and tag editing,
  all-category comparison rows, lazy side-by-side task details, bounded
  structured JSON leaf diffs, recurring-failure trend badges, and destructive
  clear behind an explicit confirmation. Targeted diagnostic reruns now commit
  through the full-scan overlay transaction described above.

**Live validation (2026-08-30, ARM64 self-contained, debug).** A native
host build of the full current candidate passed
`scripts/test-reactor-live-system.ps1` end to end with every new
subsystem constructed (chat, report, action, instance, tray hook):
native ARM64 execution, Store-matching machine card and footer UIA,
logical/physical/UIA evidence under
`apps/wfdiag/captures-2.5.8/live-system-validation-claude/`, zero
Application Error/WER events, and a clean graceful close. The run
initially exposed a graceful-close hang — root-caused to the worker
runtimes joining their OS threads while the command sender was still
alive (recv never disconnected); fixed by releasing the sender before
the join in all three workers.

`wfdiag-native-system` now owns the canonical machine, Windows-version, elevation, and
architecture/emulation projections that both shells read, so the parity suites compare
one implementation against itself rather than two independent ones.
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
nine primary states under `apps/wfdiag/captures-2.5.8`: Diagnostics empty and
populated, Monitor populated, AI empty, Issues empty and populated, History
comparison, and Settings top use the `-reactor-current.png` / `-comparison-current.png`
pairs. The corrected Processes milestone uses
`live-process-validation/deterministic-processes-populated-1440x900.png` and
`live-process-validation/processes-populated-store-left-reactor-right.png`.

The final ARM64 candidate also has pairs for all 18 manifest states under
`apps/wfdiag/captures-2.5.8/final`. These current-version reviews drive
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
   `wfdiag` while the prototype is active.
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

> **Superseded in part (2026-09-01).** `AppxManifest.xml` now depends on
> `Microsoft.WindowsAppRuntime.2` (MinVersion `2.4.0.0`), single-sourced from
> `reactor-baselines/manifest.json` → `reactor_pin`. The runtime *alignment* question below is
> therefore decided; the clean-machine and Store-certification *validation* bullets are still
> open (`store_packaging_validation`, `direct_distribution_validation`, `aion_store_validation`).
> The closing instruction not to change `AppxManifest.xml` "based on the spike alone" was
> discharged by the owner decision, not by a spike. Historical text follows.

The Store manifest previously depended on
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
`apps/wfdiag/captures-2.5.8/live-system-validation`, including the Store-left/
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
  preserve the current tray and single-instance lifecycle. The isolated Win32
  implementation now caches the exact HWND, publishes coherent revisioned
  lifecycle snapshots, restores hidden/minimized windows, and drives monitor
  pause/resume, but this does not satisfy the official-Reactor-API gate.
- `global_accelerators`: native application-wide handling for Ctrl+K,
  Ctrl+1 through Ctrl+6, Ctrl+/, Ctrl+Shift+Q, and Ctrl+Shift+F, including
  correct behavior while focus is inside text controls and dialogs. The
  isolated Win32 subclass implements those chords today with pure policy tests,
  but the gate deliberately remains open until an official Reactor API and the
  required live keyboard/focus evidence exist.

A gate may be changed to `passed` only with an official release/revision,
links to the upstream API or change, and automated plus manual WFDiag evidence.

## Validation system (2026-08-31)

The remaining-gate evidence is produced by a dedicated harness (owner goal:
1:1 functionality and rendering parity, measured rather than asserted):

- `scripts/lib/ReactorUia.psm1` — shared UIA/process helpers (hermetic
  launch, unique-button wait+invoke, status-text scanning, crash events,
  graceful close, combined-image sheets, WebView guard).
- `crates/wfdiag-native-ai-chat/tests/ai_flows.rs`, the bounded-tool unit tests,
  and `scripts/test-reactor-ai-flows.ps1` form the mandatory hermetic AI gates.
  The Rust suite exercises the shared streaming client, tool loop,
  cancellation, all ten schemas, and strict parsing against a local mock. The
  Windows UIA suite owns an isolated settings file and OpenAI-compatible mock,
  requires the custom provider to become active, and validates real streaming
  chat, mid-stream cancellation, the exact-ten tool-name contract, a
  `list_remediations` round trip returning the native `open_disk_cleanup`
  catalog ID, streamed report content, and forced report regeneration matching
  the React/Tauri UI contract. Shared report-service tests separately pin the
  cache fast path and inline cached body. The Windows workflow builds with
  `settings-test-path` and runs all of these without `continue-on-error`;
  no-provider is a failure.
- `scripts/test-reactor-chat.ps1` / `-report.ps1` / `-remediation.ps1` —
  supplemental interactive suites over live providers and system state. Chat
  and report now return failure when no provider is executable. These suites
  target the same chat composer/Send/Stop, report Generate/Cancel/Regenerate,
  per-row Run ("Run {label}"), Repair confirmation, and process-refresh
  automation surfaces; hosted-runner execution remains informational until
  its non-hermetic provider/hardware and UIA matrix is stable.
- `scripts/capture-reactor-variants.ps1` +
  `reactor-baselines/variants.json` + `scripts/check-variants.py` — theme
  and reduced-motion variant captures over deterministic fixture states,
  plus the open rendering-defect list (seeded with the owner-reported
  process-list refresh divergence). Reduced-motion capture snapshots and
  verifies restoration of the session-only animation setting; orchestrated
  validation writes its manifest and PNGs under the report directory rather
  than changing the tracked baseline document.
- `scripts/test-reactor-process-refresh-parity.ps1` — the defect target #1:
  Processes-screen triptych (initial / mid-refresh / refreshed) with
  Store-left/Reactor-right combined sheets.
- `scripts/test-reactor-x64.ps1` +
  `.github/workflows/reactor-validation.yml` — x64 evidence locally
  (emulated) and on a clean windows-latest runner.
- `scripts/check-external-gates.py` — crates.io release watch, runtime
  alignment drift, packaging pre-flight (exit 1 when an external change
  becomes actionable).
- `scripts/validate-reactor.ps1 -Suite startup|live-system|about|flows|visual|x64|readiness|gates|all`
  — orchestrator; `all` includes every listed suite and reports land under
  `validation-reports/`.
- `docs/validation/clean-machine-protocol.md` — the manual protocol for
  clean-machine and Store certification gates, with a sign-off table.

`wfdiag-native-system` now owns the canonical machine, Windows-version, elevation, and
architecture/emulation projections that both shells read, so the parity suites compare
one implementation against itself rather than two independent ones.

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
- The native component now wires major production command/event, persistence,
  AI cancellation/tool/report-cache, and desktop-lifecycle paths, but the
  remaining backend surfaces and the complete success/error/accessibility
  matrices have not all passed native Windows evidence review.
- Aion/Store, framework-dependent Store packaging, and self-contained direct
  packaging have not passed the required device matrices.

Unexpected `error` findings—missing contracts, changed checksums, Store
identity drift, unpinned dependencies, or omitted gates—are regressions and
must be corrected before further prototype work is trusted.

## Release gates (historical wording updated 2026-09-01)

This list was originally written as "strict no-cutover gates": Tauri was to remain the only
production frontend until every condition below was satisfied. The 2026-09-01 decision
settled the direction — the native shell ships — so the list below is now the **release
acceptance checklist** for that shell, not a veto on the choice of shell. Items 1, 2, and the
upstream-API portion of item 6 were closed by that decision; the rest still require real
x64/ARM64 hardware evidence recorded in `reactor-baselines/manifest.json`.

1. `check-reactor-readiness.py` exits 0 with evidence-backed gates. *(open — five cutover
   gates and the `backend_parity` matrix)*
2. Shared backend contract tests pass through both the Tauri and native adapters with
   preserved settings, credential, cache, and history formats. *(structurally closed: both
   shells now compile the same engine crates, and CI tests the whole workspace on Windows
   plus the engine crates on Linux)*
3. Every baseline state passes native fidelity review at all required themes, window sizes,
   DPI settings, and accessibility modes. *(open — `current_baseline_capture`)*
4. Scan, monitoring, processes, issues/remediation, history, AI streaming and cancellation,
   settings, exports, elevation, tray, taskbar, notifications, clipboard, updates, and
   single-instance workflows pass on x64 and ARM64. *(open — `backend_parity`)*
5. Narrator/UI Automation, keyboard-only operation, focus restoration, high contrast, text
   scaling, reduced motion, and chart alternatives pass. *(open)*
6. Store certification and clean-machine Store/direct distribution testing pass with one
   validated Windows App Runtime strategy. *(open — `store_packaging_validation`,
   `direct_distribution_validation`; the runtime strategy itself is decided:
   `Microsoft.WindowsAppRuntime.2`, MinVersion 2.4.0.0)*
7. Startup, memory, UI latency, large-list scrolling, streaming throughput, and complete
   installed footprint meet the recorded Tauri baseline or have explicit approved exceptions.
   *(open)*

The native shell may use documented Reactor and `windows` APIs for required Windows
integration, but it may not satisfy a gate by reintroducing WebView2, embedding the Tauri
frontend, or routing UI behavior through JavaScript.

Removal of React, Vite, Tauri, Node-dependent CI, and `dist` staging belongs to a later
cleanup release, after a signed rollback tag has been created. Until then `src-tauri` stays
buildable and CI keeps testing it.

## Cutover decision (2026-09-01)

The owner decided on 2026-09-01 that the next release ships the native WinUI 3
shell (`apps/wfdiag`, binary `wfdiag.exe`) as the production UI, and settled the
three gates that were waiting on Microsoft rather than on this repository:

1. **Reactor release.** `windows-reactor` and `windows-reactor-setup` still publish
   only the placeholder `0.0.0` on crates.io (re-checked 2026-09-01; upstream has
   moved three commits past the pin). The release ships on the reviewed, exact
   windows-rs revision `1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8` (Reactor 0.100.0
   source). The pin is a revision, never a branch; it is recorded in the root
   `Cargo.lock`; `scripts/check-external-gates.py` keeps watching crates.io so the
   move to an official release happens as a normal dependency update later.
2. **Windows App Runtime.** `AppxManifest.xml` now depends on
   `Microsoft.WindowsAppRuntime.2` (MinVersion `2.4.0.0`), the line the pinned
   Reactor stages. The pin is single-sourced from
   `reactor-baselines/manifest.json` (`reactor_pin.windows_app_runtime_framework`
   / `windows_app_runtime_min_version`) and read by `scripts/bump-version.py`,
   `scripts/check-reactor-readiness.py`, and `scripts/check-external-gates.py`.
   On-device AI keeps working because `wfdiag-native-phi` resolves the AI Text
   DLL from whichever `Microsoft.WindowsAppRuntime*` package the app depends on;
   this must still be validated on Copilot+ hardware (`aion_store_validation`).
3. **Window lifecycle and global accelerators.** The isolated Win32 interop in
   `apps/wfdiag/src/platform/window.rs` and `platform/instance.rs` (cached HWND,
   revisioned lifecycle snapshots, close-to-tray, tray menu, the keyboard hook
   delivering Ctrl+K / Ctrl+1..6 / Ctrl+/ / Ctrl+Shift+Q / Ctrl+Shift+F) is the
   accepted implementation for this release. Official Reactor APIs for both
   remain tracked follow-ups, not blockers.

The Tauri shell (`src-tauri`) stays buildable as a rollback until a later
cleanup release removes it. The remaining cutover gates
(`current_baseline_capture`, `native_control_parity`, `aion_store_validation`,
`store_packaging_validation`, `direct_distribution_validation`) and every
`backend_parity` surface still flip only on real x64/ARM64 hardware evidence
(see `docs/validation/clean-machine-protocol.md`).
