# CLAUDE.md

Guidance for Claude Code (claude.ai/code) and other agents working in this repository.
`AGENTS.md` and `GEMINI.md` are symlinks to this file — edit this one.

## What this is

**WindowsForum Diagnostics** (WFDiag) — a Windows diagnostics desktop app: native system
checks, live monitoring, process inspection, deterministic issue detection with tiered
remediation, encrypted scan history, exports, and an optional multi-provider AI assistant.
Ships through the Microsoft Store (package `32827MikeFara.WindowsForumDiagnostics`).

### Two shells, one engine

The product is a Rust **workspace**: framework-neutral engine crates under `crates/`, and
two shells that only *drive* them.

| | `apps/wfdiag` (**the product**) | `src-tauri` (rollback) |
| --- | --- | --- |
| Package / binary | `wfdiag` / `wfdiag.exe` | `wfdiag-tauri` / `wfdiag_tauri` |
| UI | native WinUI 3 via `windows-reactor` | Tauri v2 + React (`src/`) |
| Status | shipping as of the 2026-09-01 cutover decision | kept buildable as a rollback; deletion is a later release |
| Packaged by | Store workflow default (`shell: reactor`) | Store workflow `shell: tauri` input only |

The cutover decision is recorded in
[`docs/REACTOR_MIGRATION.md#cutover-decision-2026-09-01`](docs/REACTOR_MIGRATION.md).
Neither shell contains engine logic: both call the same crates, so behaviour cannot drift.

## Workspace map

Root `Cargo.toml`: `members = ["crates/*", "apps/wfdiag", "src-tauri"]`,
`default-members` = engine crates + the native shell (so an unqualified cargo command skips
the Tauri shell; on Linux, exclude both shells — see Commands).
Centralized `[workspace.dependencies]` (one version per dep), `[workspace.lints]`
(`unsafe_code = "deny"`, clippy `all` + `pedantic` = warn) which each engine crate opts into
with `[lints] workspace = true`, release/dev profiles at the root, one `Cargo.lock`, one
`target/`.

"Portable" = builds and tests on Linux with no Windows and no GUI (CI job `rust-portable`).

| Crate | Responsibility | Portable |
| --- | --- | --- |
| `wfdiag-native-core` | error type, timestamps, atomic file writes, trusted-program command executor (`src/security.rs`), native WMI wrapper | yes |
| `wfdiag-remediation-catalog` | read-only remediation metadata (`REMEDIATION_COUNT = 17`, tiers `OpenTool`/`AutoSafe`/`Repair`, 8 `maintenance` entries) | yes |
| `wfdiag-native-issues` | issue catalog (28 `IssueSpec`s), pure detectors, UI projection, fix-plan validation, worker runtime | yes |
| `wfdiag-native-remediation` | remediation engine + `broker::ActionBroker` — the **only** execution path | yes |
| `wfdiag-native-diagnostics` | task catalog (45 tasks), Windows collectors, scan orchestration runtime | yes |
| `wfdiag-native-monitor` | live CPU/memory/disk/network/GPU/NPU telemetry, process inventory. `#![cfg(windows)]` — empty library elsewhere | Windows only |
| `wfdiag-native-history` | encrypted scan history (DPAPI envelope `VERSION = 2`), comparison, tags, trends | yes |
| `wfdiag-native-export` | report renderers, `src/path_policy.rs` (save-destination policy), `src/external.rs` (closed URL set) | yes |
| `wfdiag-native-settings` | settings document + per-provider credential store (DPAPI) | yes |
| `wfdiag-native-system` | host identity, Windows version, elevation, CPU architecture/emulation | yes |
| `wfdiag-native-update` | GitHub-release update policy + `UpdateOutcome` | yes |
| `wfdiag-native-ai-provider` | provider enum/wire names, `capabilities()`, `route_provider()`, model catalogs, local probes, `SettingsProviderKeySource` | yes |
| `wfdiag-native-ai-chat` | chat state, bounded tool loop, `providers/` transports, `src/grounding.rs` (the single sanitizer), `runtime`, `tools`, `workers/` | yes |
| `wfdiag-native-ai-report` | one-click scan report: `src/evidence.rs` assembly + `src/runtime.rs` | yes |
| `wfdiag-native-ai-analysis` | per-task analysis, issue prioritization, fix-plan workers, prompts/budgets | yes |
| `wfdiag-native-phi` | on-device Phi Silica runtime + generated WinRT bindings | yes (Windows paths cfg'd) |
| `wfdiag-native-projection` | pure projections: JSON diff, markdown-lite parser + link policy, process identity, monitor graph geometry | yes |
| `wfdiag-ui-core` | `UiEvent` contract + delivery bus (no Tauri/Reactor/Tokio) | yes |
| `wfdiag-app` | **the application service facade** (below) | yes |

## The facade: `wfdiag-app`

`AppService` owns every engine runtime and exposes one command-in / event-out API, so any
GUI — or a headless test — drives the same engine.

```rust
AppService::start(config: AppConfig, ports: AppPorts) -> Result<(Self, AppEventReceiver), AppStartError>
        .snapshot() -> &AppSnapshot          // the read model
        .dispatch(AppCommand) -> DispatchOutcome   // never blocks
        .drain() -> Vec<AppEvent>            // only reader of workers, only writer of the snapshot
        .shutdown(budget: Duration) -> ShutdownReport
```

* **Single-threaded and host-owned**: `dispatch`/`drain` take `&mut self`. Workers wake the
  host through the callback installed with `AppEventReceiver::set_wake_handler`.
* **All staleness lives in `drain`**, compared against the newtypes in `ids` (`Epoch`,
  `Generation`, `RequestId`). A host never holds or compares a request id.
* `DispatchOutcome` = `Accepted` | `Ignored` | `Rejected(RejectReason)`; reject reasons are
  `Terminating`, `WorkerUnavailable`, `Busy`, `Invalid`, `NotReady`, `IdentityExhausted`.

`AppCommand` variants (`crates/wfdiag-app/src/command.rs`) — *lifecycle*: `Start`,
`WindowVisibility`, `Shutdown`; *scan/issues*: `StartScan`, `StartTargetedScan`, `CancelScan`,
`RefreshIssues`; *history*: `ListHistory`, `LoadHistoryScan`, `CompareHistory`,
`CompareCurrentToLatest`, `HistoryTaskDiff`, `SaveHistoryLabel`, `SaveHistoryTags`,
`HistoryTrends`, `ClearHistory`; *monitor*: `MonitorRefresh`, `SetMonitorPaused`,
`RequestProcessPage`, `RequestNetworkConnections`; *provider*: `RequestProviderStatus`,
`SetProviderPreference`, `ClearAiCache`, `ListOllamaModels`, `RefreshModelCatalog`,
`CancelModelCatalog`; *settings*: `LoadSettings`, `SaveSettings`, `UpdateSetting`,
`ProviderCredential`; *host*: `ExportResults`, `CheckForUpdates`, `RequestSystemInfo`,
`RequestArchitecture`, `RestartAsAdmin`; *AI*: `ChatSend`, `ChatCancel`, `ChatReset`,
`CloudFallbackDecision`, `GenerateReport`, `CancelReport`, `AnalyzeDiagnostic`,
`CancelAnalysis`, `PrioritizeIssues`, `GenerateFixPlan`, `CancelFixPlan`; *remediation*:
`PrepareRemediation`, `ApproveAction`, `DiscardProposal`, `CancelAction`; *subscriptions*:
`SubscriptionAuth`, `CancelSubscriptionAuth`, `InstallSubscriptionCli`,
`ConfirmSubscriptionInstall`, `CancelSubscriptionInstall`.

`AppEvent` (`event.rs`) is `Started` + one variant per domain — `Scan`, `Issues`, `History`,
`Monitor`, `Provider`, `Settings`, `Export`, `Update`, `System`, `Chat`, `Report`, `Analysis`,
`FixPlan`, `Prioritization`, `Action` — plus `WorkerStopped`, `ReplyTimedOut`, `Terminated`.
Each carries its own sub-enum (`ScanEvent::Progress`, `ActionEvent::RepairConfirmationRequired`).

Everything environmental is a port (`AppPorts`: diagnostics, system, settings storage,
credentials, settings validator, release HTTP, signature, current version, provider backend,
monitor, elevation, environment, update throttle, and the `ai::AiPorts` bundle).
`AppPorts::mock()` builds a complete in-memory bundle, so the `headless_*` suites in
`crates/wfdiag-app/tests/` drive the *real* service — real workers, real threads, real
guards — on Linux with no Windows and no GUI.

## Native shell layout (`apps/wfdiag/src`)

`main.rs` is a thin entry point only: install the panic hook, answer the version probe
(compile-time `false` without the `validation` feature), take the single-instance decision,
then `App::run_component::<WfdiagShell>`. Everything else lives in the modules below.

* `app/` — the root Reactor `Component`: `mod.rs` (state fields + `Component` impl),
  `state.rs`, `message.rs`, `consts.rs`, `policy.rs` (pure decisions), `tasks.rs`
  (background helpers), `orchestration/` (one module per concern: `actions`, `analysis`,
  `chat`, `export`, `history`, `issues`, `lifecycle`, `providers`, `report`, `scan`,
  `settings`, `subscriptions`, `update`).
* `screens/{diagnostics,monitor,processes,ai,issues,history}/view.rs` — one module per page.
* `dialogs/` — `about`, `action_review`, `palette`, `settings`.
* `widgets/` — `badges`, `cards`, `chrome`, `icons`, `markdown_render`, `palette_colors`, `table`.
* `platform/` — every Win32/WinRT/WinUI edge: `window` (subclass, lifecycle snapshots,
  keyboard hook), `instance`, `notifications`, `save_picker`, `external`, `focus`,
  `winui_focus_bindings`, `ui_wake`, `crash`.
* `ai/` — `chat_tools` (the shell's tool backend) and `report` (Phi-aware resolvers).
* `fixtures/` — `knobs` (env-var knobs), `visual`, `issues`. Nothing here runs in production.

Cargo features (release artifacts enable **none** of them):

| Feature | Effect |
| --- | --- |
| *(default)* | framework-dependent — `build.rs` stages only the matching `Microsoft.WindowsAppRuntime.Bootstrap.dll` |
| `self-contained` | stages the full Windows App Runtime beside the exe for direct-installer validation; **must** be built with native Windows Cargo |
| `settings-test-path` | the exact-path settings store used by integration validation |
| `validation` | superset of `settings-test-path`: also compiles in `fixtures/knobs.rs` — every env knob (`WFDIAG_REACTOR_*`, `WFDIAG_NO_*`) and the `--wfdiag-version-probe` entry point. Without it the shell performs **no** environment reads at all and every knob is a compile-time production default (#186, #212) |

## Commands

From the repository root unless stated.

```bash
# Native (Linux) — engine crates only; the two shells are Windows-only
cargo check  --workspace --all-targets --exclude wfdiag --exclude wfdiag-tauri
cargo clippy --workspace --all-targets --exclude wfdiag --exclude wfdiag-tauri -- -D warnings
cargo test   --workspace --exclude wfdiag --exclude wfdiag-tauri
cargo test -p wfdiag-app                 # the headless integration suites
cargo fmt --all

# Cross-check the Windows shells from this Linux/WSL box (the /usr/bin/clang*
# symlinks are broken llvm-21 — the PATH prefix is required)
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin check  --workspace --target x86_64-pc-windows-msvc
PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin clippy --workspace --target aarch64-pc-windows-msvc

# Frontend (Tauri rollback shell only). `npm run build` must precede any cargo command
# that includes wfdiag-tauri: tauri-build embeds the untracked ../dist.
npm ci && npx tsc --noEmit && npx eslint . && npx vitest run && npm run build

# Script checks
python3 scripts/check-version-sync.py
python3 scripts/check-store-identity.py
python3 scripts/check-reactor-readiness.py [--json]   # exit 1 while a gate is blocked
python3 scripts/check-external-gates.py               # exit 1 when an external change is actionable
python3 -m unittest scripts/test_check_reactor_readiness.py

# Version bump — 11 files (see scripts/README.md)
python3 scripts/bump-version.py 2.5.9 [--dry-run]
```

Windows-only (PowerShell, real hardware):
`scripts/validate-reactor.ps1 -Suite startup|live-system|about|flows|visual|x64|readiness|gates|all`
(reports land in `validation-reports/`), `capture-reactor-baselines.ps1`,
`capture-reactor-variants.ps1`, and the focused `test-reactor-*.ps1` suites.

`src-tauri/.cargo/config.toml` is an **untracked, WSL-only** cross-compile config
(`.gitignore:152`). Never commit it.

## CI

`.github/workflows/ci.yml` (PRs unfiltered + pushes to `main`):

| Job | Runner | What |
| --- | --- | --- |
| `store-identity` | ubuntu | `check-version-sync.py`, `check-store-identity.py`, probe unit tests |
| `frontend` | ubuntu | `npm ci`, `tsc --noEmit`, `eslint`, `vitest run`, `npm audit --omit=dev` |
| `rust-portable` | ubuntu | check / clippy `-D warnings` / **test** the workspace minus the two shells — the headless guarantee |
| `rust` | windows | `cargo fmt --check`, then clippy + `cargo test` over the **whole** workspace with `--features wfdiag/validation`, **and again with no `wfdiag` features** — the release shape, and the only configuration in which the shell's `production_defaults` knob test compiles (#186, #212) |
| `rust-arm64` | windows | check + clippy `-D warnings` for `aarch64-pc-windows-msvc`, `--features wfdiag/validation` |
| `cargo-audit` | ubuntu | advisory (`continue-on-error`) |

Every cargo invocation in CI uses `--locked`.
`.github/workflows/reactor-validation.yml` is the x64 (`windows-latest`) + ARM64
(`windows-11-arm`, best-effort) matrix: hermetic AI engine/report tests, a self-contained
validation release build, version probe, then the AI-flow, live-system, chat, report, and
remediation UIA suites.

## Release

Tag push (`v2.5.9`) → `.github/workflows/build-and-publish-store.yml`. `workflow_dispatch`
takes a `shell` input: **`reactor`** (default, the product) or `tauri` (rollback). Both keep
the same Store identity and version.

1. Per-arch build: `cargo build --locked --release -p wfdiag --target {x86_64,aarch64}-pc-windows-msvc`.
2. Package through the probe script, which is the single manifest renderer for both the
   shipped package and the alignment probe:
   `python scripts/build-reactor-msix-probe.py stage|pack|bundle|validate-msix`
   (plus `validate-layout`; bare invocation builds a standalone probe).
3. Unsigned bundle uploaded — Microsoft signs the Store-delivered package. Attestations +
   GitHub Release follow.

`AppxManifest.xml` launches `wfdiag.exe` and depends on `Microsoft.WindowsAppRuntime.2`
(MinVersion `2.4.0.0`) with `runFullTrust` + `systemAIModels`. That framework name and floor
are **single-sourced** from `reactor-baselines/manifest.json` → `reactor_pin` and read by
`scripts/bump-version.py`, `check-reactor-readiness.py`, and `check-external-gates.py`.

### Reactor pin policy

`windows-reactor`, `windows-reactor-setup`, and `windows-core` are pinned to the reviewed
`microsoft/windows-rs` **revision** `1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8` (Reactor
0.100.0 source). crates.io still publishes only the placeholder `0.0.0` for both Reactor
crates. A branch, tag, or floating git dependency is prohibited. Moving the pin means
updating, in one reviewed change: both Cargo dependencies, `reactor-baselines/manifest.json`
(`reactor_pin`), and `scripts/build-reactor-msix-probe.py`. `check-external-gates.py` watches
crates.io so the eventual move to an official release happens as a normal dependency update.

## Readiness and validation gates

`python3 scripts/check-reactor-readiness.py` reports **NOT READY** (exit 1) on purpose. The
cutover is *decided*; what remains is hardware evidence:

* Closed by the 2026-09-01 decision: `cutover.official_reactor_release`,
  `upstream.window_lifecycle`, `upstream.global_accelerators`.
* Still blocked, each needing real x64/ARM64 evidence: `current_baseline_capture`
  (light/high-contrast/DPI/reduced-motion variants + parity review),
  `native_control_parity`, `aion_store_validation` (Copilot+ devices),
  `store_packaging_validation`, `direct_distribution_validation`.
* All 19 `backend_parity` surfaces are `partial` (`on_device_ai_and_package_identity` is
  `blocked`) until each has live integration evidence.

Evidence comes from `scripts/validate-reactor.ps1 -Suite all` and the manual
`docs/validation/clean-machine-protocol.md`. **Never weaken or bypass a gate to go green.**

## Security model

* **Remediation has exactly one execution path.** `remediation::execute_authorized` is
  `pub(crate)`. Every caller outside `wfdiag-native-remediation` goes through
  `broker::RealCatalogExecutor`, which only accepts an `AuthorizedAction` borrowed from an
  `ActionGrant`, and `broker::ActionBroker::authorize` refuses to mint a grant over a
  `RemediationTier::Repair` preview without `ActionApproval::RepairConfirmed`. The gate is in
  the broker, **not in the UI**. Grants are opaque, expiring, one-use, and revalidated
  against current issue/catalog fingerprints. Every command is a compile-time constant run
  through an injectable `CommandRunner`.
* **AI chat tools are strictly read-only.** Exactly ten: `run_diagnostic`,
  `search_windows_knowledge`, `get_scan_summary`, `request_full_scan`, `get_detected_issues`,
  `compare_with_previous_scan`, `get_live_stats`, `list_remediations`, `list_scan_history`,
  `stage_remediation`. `request_full_scan` and `stage_remediation` only emit typed UI
  requests — they never execute. The loop is bounded: `MAX_TOOL_ITERATIONS = 4`,
  `MAX_TOOL_CALLS_PER_TURN = 8`, `TOOL_TIMEOUT_SECS = 45`, `TOOL_CONCURRENCY = 3`,
  `TURN_TIMEOUT_SECS = 180`, then a forced final answer. Chat-triggered scans are never
  written into the scan session.
* **One grounding sanitizer.** `crates/wfdiag-native-ai-chat/src/grounding.rs` is the single
  untrusted-input → search-query boundary: nothing reaches the WindowsForum MCP endpoint
  unless it comes from `SAFE_QUERY_FIELDS` and survives `safe_value_term`. No shell keeps a
  private copy.
* **Export destinations are closed.** `crates/wfdiag-native-export/src/path_policy.rs` decides the suggested filename,
  which directories may be saved into, and whether a filename belongs to this app (with
  canonicalization against junction/symlink replacement). `crates/wfdiag-native-export/src/external.rs` resolves a
  typed `ExportExternalAction` to one trusted HTTPS URL — a URL string never crosses the UI
  boundary.
* **API keys never land in `settings.json`.** Every key field in `AppSettings` is
  `#[serde(default, skip_serializing)]`; only `*_api_key_set` booleans persist. Storage is
  one current-user DPAPI entry per provider via the closed `ProviderKeyId` set
  (`crates/wfdiag-native-settings/src/persistence.rs`).
* **Scan history** uses a versioned envelope: `VERSION_DPAPI = 2` (current-user DPAPI
  ciphertext) on Windows, with a *distinctly versioned* plaintext envelope on non-Windows dev
  hosts so the two can never be confused. Writes are atomic (`crates/wfdiag-native-core/src/fs_atomic.rs`).
* **Subprocesses** run through the trusted-program executor in `crates/wfdiag-native-core/src/security.rs`
  (closed allowlist, validated arguments, hidden window, timeouts).
* **No env-var behaviour in production.** Every knob (`WFDIAG_REACTOR_*`, `WFDIAG_NO_*`, the
  version probe) belongs in `apps/wfdiag/src/fixtures/knobs.rs` behind the `validation`
  feature, which release artifacts never enable.
* **`unsafe` is confined.** `[workspace.lints]` sets `unsafe_code = "deny"`, and every engine
  crate opts in with `[lints] workspace = true` (the exception is `wfdiag-native-monitor`,
  which is pure Windows API code); crates that need FFI carry a narrow
  `#![allow(unsafe_code)]` at the module that needs it. The shells do **not** inherit the
  workspace lints — the native shell instead applies `#![deny(unsafe_code)]` per module, so
  `unsafe` exists only in `apps/wfdiag/src/platform/` (and the generated
  `winui_focus_bindings.rs`). Keep it that way: a new `unsafe` block anywhere else is a
  review failure.
* Logs: opt-in Phi debug log at `%LOCALAPPDATA%\WFDiag\logs\phi-silica.log` (set
  `WFDIAG_AI_LOG=1`); crash records at `%LOCALAPPDATA%\WFDiag\logs\crash-*.log`
  (`create_new`, so never written through a planted link).

## AI providers

`wfdiag_native_ai_provider::capabilities()` is the single source of truth for this table;
transports live in `crates/wfdiag-native-ai-chat/src/providers/` (plus the shared
`openai_compat` client, `cli_bridge.rs`, and `compat_provider.rs` in that crate's root).

| Provider (wire id) | Runs | Auth | Tools | Streaming | Budget (chars) |
| --- | --- | --- | --- | --- | --- |
| `phi_silica` | on-device NPU (Store build only) | package identity | no | no | 2,500 |
| `foundry_local` | local server | none | no | yes | 12,000 |
| `ollama` | local server | none | yes | yes | 12,000 |
| `custom_openai` | any `/v1/chat/completions` server | optional key | yes | yes | 24,000 |
| `codex_cli` | cloud via installed Codex CLI | CLI-owned sign-in | no | no | 24,000 |
| `claude_code` | cloud via installed Claude Code CLI | CLI-owned sign-in | no | yes | 24,000 |
| `openai` | cloud | API key | yes | yes | 48,000 |
| `anthropic` | cloud (native Messages API) | API key | yes | yes | 48,000 |
| `gemini` | cloud (native generateContent) | API key | yes | yes | 48,000 |
| `deepseek` | cloud (OpenAI-compatible) | API key | yes | yes | 48,000 |

Auto routing is local-first — `AUTO_FALLBACK_ORDER` in `crates/wfdiag-native-ai-provider/src/fallback.rs`:
Phi → Foundry → Ollama → custom → Codex CLI → Claude Code → OpenAI → Anthropic → Gemini →
DeepSeek. The pure decision is `route_provider(preference, ProviderAvailability)`; probing
stays lazy. **An explicit (non-Auto) preference never falls back.** Auto clean-failure
retries honour the persisted Ask/Allow/Never cloud-consent policy and disclose the
local-to-cloud transition via `ProviderUse` attribution. Wire strings are pinned per variant
with explicit `#[serde(rename)]` (plus `alias = "open_a_i"` for old settings files) —
`rename_all = "snake_case"` would emit `"open_a_i"` for OpenAI, a real bug fixed in 2.5.0.

Gotchas encoded in the clients — do not relearn these:

* **Anthropic**: `max_tokens` is REQUIRED; never send `temperature`; branch on
  `stop_reason == "refusal"` *before* reading content. Default model constant
  `ANTHROPIC_DEFAULT_MODEL = "claude-sonnet-5"` (defined in
  `crates/wfdiag-native-ai-provider/src/catalog_service.rs`, re-exported by
  `providers/anthropic.rs`).
* **Gemini**: auth via the `x-goog-api-key` **header**, never `?key=` (keys must not appear
  in URLs); assistant role is `"model"`; `functionResponse.response` must be a JSON object;
  no tool-call ids (synthesized as `name#index`).
* **Generic/custom + Ollama** use `/v1/chat/completions` (they do not serve `/v1/responses`).
  No token cap goes on the wire: current OpenAI models reject `max_tokens` and compat servers
  do not all know `max_completion_tokens`.
* **Foundry Local**'s port is dynamic by design — discovered via `foundry status --output json`
  (legacy `service status` spelling kept as a fallback) or the `localAiEndpoint` setting.
  Never hardcode it.
* **Ollama** has no default model: the `ollamaModel` setting, else the first entry from
  `/api/tags`, else an error telling the user to pull one.
* **Subscription CLI bridges** (`crates/wfdiag-native-ai-chat/src/cli_bridge.rs`,
  `providers/{codex,claude_cli,acp_bridge}.rs`): we implement NO OAuth and store NO tokens —
  the installed CLI owns sign-in and usage bills to the user's plan. Never extract
  subscription OAuth tokens for direct API use.
  * Claude speaks ACP over stdio to `npx -y @agentclientprotocol/claude-agent-acp` (pinned
    adapter version; the `agent-client-protocol` crate is pinned at 1.x — 2.0 rewrites the
    transport layer). Permission requests are rejected (Q&A only) and `CLAUDECODE` is
    scrubbed or the adapter refuses to start. `claude -p --output-format json --max-turns 2`
    is the fallback both when npx is missing and when ACP fails before any text streamed.
  * `bridge_workdir()` is async and *validated* by actually spawning `cmd.exe /d /c cd` in
    each candidate: MSIX AppData virtualization can hide a fresh path from the host of every
    npm `.cmd` shim.
  * Timeout budgets are strictly nested and must stay that way — `BRIDGE_CATALOG_TIMEOUT =
    120s` for bridge providers vs 15 s for HTTP ones. An outer timeout smaller than the inner
    ones silently breaks first-run model discovery.
  * Prompts go via stdin (never argv); exes resolve through `where.exe` (bare
    `Command::new` cannot spawn npm shims); codex runs use
    `codex exec --json --ephemeral --sandbox read-only`; probes are TTL-cached; and every
    bridge child gets `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` / `OPENAI_API_KEY` scrubbed.

## Phi Silica (on-device AI)

Short version: Phi Silica requires **registered package identity** — it works in the Store
build only, and the loose/portable exe short-circuits to Foundry Local → cloud. The runtime
lives in `crates/wfdiag-native-phi`; `windows_ai_bindings.rs` in that crate is **generated by
`windows-bindgen` — never hand-edit it**. Microsoft is replacing Phi Silica with Aion
Instruct (retail ~November 2026), which drops the LAF token requirement, so the LAF apparatus
has a firm expiry date.

The full engineering record — activation ordering, LAF token history, the 2026-08 `Unavailable`
investigation, error codes, bundled DLLs, sparse-identity dev tooling — is
**[`docs/PHI_SILICA.md`](docs/PHI_SILICA.md)**. Read it before touching anything Phi-related;
several conclusions there are explicitly marked "do not re-litigate".

## Rules for contributors

1. **No `#[path]` includes.** Engine code compiles once, in its crate. `src-tauri` keeps only
   one-line `pub use` shims over the crates.
2. **No engine logic in `apps/`.** If a rule, parse, policy, or projection can be tested
   without a window, it belongs in a crate. `apps/wfdiag` may depend on the crates, `windows`,
   and `windows-reactor`; nothing else.
3. **No `windows-reactor` outside `apps/wfdiag`.** No crate under `crates/` may depend on it
   (nor on Tauri, Wry, or any WebView host); mentioning it in a doc comment is fine.
   `check-reactor-readiness.py` enforces the WebView ban over `apps/wfdiag/src` — a browser
   dependency is a blocker, not an alternate route to parity.
4. **No environment knobs outside `apps/wfdiag/src/fixtures/knobs.rs`**, and everything there
   is behind the `validation` feature, so a release build performs no environment reads for
   behaviour. A plain OS path lookup (`%LOCALAPPDATA%` in `platform/crash.rs`) is not a knob;
   anything that *changes behaviour* is.
5. **Lints are the contract**: a new crate gets `[lints] workspace = true`; CI runs clippy
   `-D warnings` on Windows x64, Windows ARM64, and Linux, plus `cargo fmt --all --check`.
6. **Commit per step, with pathspecs.** One reviewable change per commit
   (`git commit -- <paths>`), never `git add -A`. Do not commit or push unless asked.
7. **Bug convention**: title `Reactor audit <date> <id>: …`, labels `bug` plus one of
   `priority: high` / `medium` / `low`, closed with the fixing commit hash. See `docs/BUGS.md`.
8. **Testing reality**: `cargo test` for the shells only runs on Windows CI. On this Linux box
   verify with `cargo xwin check|clippy`, and put anything you want covered into a crate so
   `rust-portable` and `cargo test -p wfdiag-app` can reach it.
9. **Never hand-edit a single-sourced value**: the Reactor pin, the Windows App Runtime
   framework/floor, and the version live in one place each (`reactor-baselines/manifest.json`,
   `version.json`) and are propagated by scripts.

## Docs map

| File | Contents |
| --- | --- |
| `docs/REACTOR_MIGRATION.md` | migration runbook, architecture rationale, gate definitions, the cutover decision record |
| `docs/PHI_SILICA.md` | the complete Phi Silica / LAF / Aion engineering record |
| `docs/REACTOR_STORE_PROBE.md` | the non-publishing Store/MSIX alignment probe |
| `docs/validation/clean-machine-protocol.md` | manual clean-machine + Store certification protocol with sign-off table |
| `docs/BUGS.md` | pointer to the GitHub audit tracking issue and the bug convention |
| `docs/STORE-PUBLISHING.md` | release plumbing (Store delivery; Microsoft signs the shipped package) |
| `scripts/README.md` | version-bump file list, MSIX/probe subcommands, capture and validation lanes |
| `reactor-baselines/README.md` | the visual-oracle + pin contract, and what may not be weakened |
| `apps/wfdiag/README.md` | the native shell: layout, build/run, evidence, open gates |
