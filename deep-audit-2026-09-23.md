# WFDiag deep audit — 2026-09-23 (round 3)

Full-workspace audit of `main` @ `7b544c6` (release-2.5.9 shape), round 3 of the
audit-remediate-rescan loop. Six reviewer slices (post-audit delta, security invariants,
concurrency/lifecycle, Windows-only code, providers/wire, scripts/CI/frontend/rollback)
under the standing ≤6-reviewer cap; every candidate finding re-verified against the source
on the main thread before acceptance. Baseline: round 2 closed all 84 issues
(#233–#320) at `96cf742` with a green battery; everything in `96cf742..HEAD`
(70 files, +1745/−829: the 6dc31b9 perf pass, 92959d4 bootstrap-DLL removal, 4c37733 tray
fix, CI cost-policy commits, 3348cd8 version bump) had never been reviewed.

## Baseline battery at 7b544c6

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --workspace --all-targets --exclude wfdiag --exclude wfdiag-tauri -- -D warnings` | ✅ |
| `cargo test --workspace --exclude wfdiag --exclude wfdiag-tauri` | ✅ |
| version-sync / store-identity / external-gates / readiness unit tests | ✅ |
| `npx tsc --noEmit` / `npx vitest run` | ✅ / ✅ |
| `npx eslint .` | ❌ M1 |
| xwin clippy x86_64 + aarch64 (after `npm run build`) | ✅ / ✅ |

The only red is the frontend lint gate (M1). `check-reactor-readiness.py` remains at its
designed NOT-READY (hardware evidence outstanding).

## High

### H1 (R6-1) `scripts/build-reactor-msix-probe.py:821` — undefined `BOOTSTRAP_DLL` crashes the standalone probe after the full build

`"staged_dlls": [BOOTSTRAP_DLL]` references a constant deleted by 92959d4; it is assigned
nowhere. Any bare invocation (`python scripts/build-reactor-msix-probe.py`, the documented
"standalone probe") completes both cargo builds, pack, and bundle, then dies with an
uncaught `NameError` — `probe-report.json` / `NON-PUBLISHING-PROBE.txt` are never written.
`test_build_reactor_msix_probe.py` cannot see it because it exercises only subcommand
handlers, never `build_probe`.

## Medium

### M1 `src/screens/DiagnosticDetail.tsx:22` — `eslint` is red on main

`/[\u0000- ]/` trips `no-control-regex`; `npx eslint .` exits 1. Introduced by
8a63e1f (round 2's own fix series) and undetected since, because CI stopped running on
pushes (M2) and the round-2 final battery never included the frontend checks. The check
itself is deliberate and correct — the gate is red, not the policy.

### M2 `ci.yml` — pushes to `main` no longer trigger any CI; CLAUDE.md's CI contract is stale

7b544c6 removed the `push:` trigger. `store-identity`/`frontend`/`rust-portable`/`rust`
run only on `pull_request`/`workflow_dispatch`; the weekly Monday cron runs only
`rust-arm64` + `cargo-audit`. Every recent main commit (all direct pushes since 2026-09-09)
landed with no CI — which is exactly how M1 survived three weeks. CLAUDE.md still documents
"PRs unfiltered + pushes to `main`", and `scripts/check-store-identity.py`'s docstring
still says "Run in CI on every push".

### M3 `ci.yml:5` — `branches: [main]` filter deletes the documented stacked-PR guarantee

The comment 7b544c6 removed said: "No branch filter on pull_request: stacked PRs
(feature → feature) must run the same gates as PRs into main". A PR targeting any branch
other than main now runs no CI at all.

### M4 `ci.yml:102,126` — `rust-arm64` and `cargo-audit` no longer gate PRs

Both are `schedule || workflow_dispatch` only. The only aarch64 check/clippy lane sees a
breaking PR at most a week later, or at release-build time in the Store workflow (which
builds aarch64 — the worst possible moment). The cost rationale is reasonable but
undocumented; CLAUDE.md rule 5 still claims ARM64 clippy as a standing CI gate.

### M5 (R3-1) facade — a timed-out provider-status reply strands `provider_loading = true` and wedges all later AI intents

`crates/wfdiag-app/src/service/mod.rs:1536/1574` set it; only the `Internal::ProviderStatus`
/`ProviderPreference` handlers clear it (`:2041/:2084`). The reply-timeout path
(`poll_replies`, `:2774-2783`) emits `ReplyTimedOut` and drops the pending entry **without
clearing the flag** or the request id, so the worker's late reply is discarded (id guard)
and `PendingAiProviderGate::evaluate` parks every later Chat/Report/Analyze dispatch as
`Waiting` forever; `RefreshProviders` refuses to re-probe while `loading == true`. Recovery
requires the host to spontaneously dispatch a fresh `RequestProviderStatus`. Reachable when
the single-threaded provider worker misses the 30 s facade deadline (slow local-server
probe sets). Pre-existing.

### M6 (R3-2) analysis/fix-plan `one_shot` is unbounded on the OpenAI-compatible transport family

`openai_compat.rs` builds a default reqwest client (no timeout) and `one_shot`
(`:307-333`) awaits bare; `analysis.rs:431` and `fix_plan.rs:335/506` `select!` only on
`cancel.cancelled()`. A server that accepts and never responds hangs the analysis/fix-plan
worker (domain `Busy`) until the user cancels. Chat (180 s turn), report, Anthropic/Gemini
one-shots (120 s), and CLI one-shots (150 s) are all bounded — only the compat family
(openai, deepseek, custom_openai, ollama, foundry_local) is not. Pre-existing.

### M7 (R6-5) five validation scripts still hard-pin 2.5.8 against `version.json` = 2.5.9

`capture-reactor-variants.ps1:98`, `test-reactor-chat.ps1:34`, `test-reactor-report.ps1`,
`test-reactor-remediation.ps1:34`, `test-reactor-ui-regressions.ps1:43` all `throw
"not the pinned 2.5.8 oracle"` for a HEAD-built candidate. 3348cd8 bumped the version and
6dc31b9 unpinned only one of the six (`test-reactor-process-refresh-parity.ps1`), so
`validate-reactor.ps1 -Suite visual|flows|x64` is fail-red for every current build — the
documented `-Suite all` evidence lane is broken, and inconsistently so (one script
unpinned, five not).

### M8 (R6-6) `scripts/lib/ReactorUia.psm1:47-48` — shared version probe has no timeout

`Start-Process --wfdiag-version-probe -Wait -PassThru` blocks forever on a featureless exe
(the probe entry point is compile-time `false` without `wfdiag/validation`, so the exe
ignores the flag and opens its GUI). The inline copies in `capture-reactor-baselines.ps1`
and `test-reactor-live-system.ps1` do `WaitForExit(10000)` + kill + throw; the shared lib
copy used by variants/chat/report/remediation/ai-flows/parity does not.

### M9 (R6-7) `reactor-validation.yml:130-135` — external-gates step is labelled "Informational … not gating" but gates

`python scripts/check-external-gates.py --json` runs with no `continue-on-error`; the
script exits 1 on an actionable finding (a newer crates.io release — the watcher's whole
purpose) and 2 on any network error, failing the x64 validation leg. The repo's own
orchestrator (`validate-reactor.ps1:186-192`) treats the same exit as informational; the
comment and the behaviour disagree, and a crates.io outage or a routine upstream release
turns red into the validation lane's default state.

### M10 (R6-8) adopted-reactor-version copy is hard-coded in `check-external-gates.py`, and CLAUDE.md's pin-move checklist names the wrong script

`EXPECTED_REACTOR_VERSION = "0.100.0"` (`:33`) is never cross-checked against
`reactor-baselines/manifest.json → reactor_pin` (unlike `check-reactor-readiness.py:270-288`,
which fails loudly). CLAUDE.md's move-the-pin checklist lists `build-reactor-msix-probe.py`
(which now reads the manifest via `_reactor_pin()` and needs no edit) and omits
`check-external-gates.py` (which needs a manual edit). A future pin move that follows the
documented checklist leaves the watcher reporting the new release as "newer than adopted"
forever.

### M11 (R6-17) `scripts/test-reactor-performance.ps1` silently depends on `validation`-feature knobs it never verifies

It sets `WFDIAG_REACTOR_PAGE` / `WFDIAG_REACTOR_SETTINGS_TEST_PATH` / `WFDIAG_NO_TRAY` and
runs no version probe: against a featureless exe the knobs are absent, four pages record
identically-mislabeled "passed" runs, and settings land in the developer's **real** store.
`docs/PERFORMANCE_2.5.9.md` never states the feature requirement and "identical
release-feature builds" reads as featureless. The perf evidence behind the 2.5.9
performance doc is only valid for `--features validation` builds.

### M12 (R5-1) subscription env-scrub list is duplicated inside the engine — drift silently converts subscription runs into direct API billing

`crates/wfdiag-native-ai-chat/src/cli_bridge.rs:28` redefines `SUBSCRIPTION_OVERRIDE_ENV_VARS`
byte-identical to the single-sourced `wfdiag-native-ai-provider/src/local_probes.rs:25`,
although ai-chat already depends on ai-provider (the rollback shell's third copy in
`src-tauri/src/ai_providers/cli_bridge.rs:60` is the known, decision-pending fork family
#250/#251/#255 — not re-litigated here). If a vendor override var is added to the probe
list but not the engine's scrub copy, every bridge child and the ACP adapter
(`acp_bridge.rs:89`) inherits the host override key; reverse drift makes probes misreport
account state.

### M13 (R5-2 + R3 corroboration) five safety-relevant AI tests are `#[ignore]`d and the old native-contracts CI lane is gone

`subscription_install.rs:1109/1263/1334/1376` (installer allowlist+budgets per
provider/method; cancel-after-commit cannot relabel success; ordered static progress) and
`ai_flows.rs:258` (`slow_stream_cancels_mid_turn` — the only mid-stream cancellation test)
are skipped on every platform, so neither `rust-portable` nor Windows CI runs them; the
FIXMEs say "ignored with intent, do not delete", but nothing re-enables them. These guard
the highest-consequence automation path (subscription installer + cancellation), and the
old soft-failed "native contracts" CI step that partially covered this area was removed in
the merge, leaving no lane at all.

## Low

### L1 docs-only PRs produce no statuses (`ci.yml:6-9` paths-ignore)

If any CI job is a required check, docs-only PRs hang at "Expected — waiting for status";
`workflow_dispatch` results don't attach to the PR. `validation-reports/**` in the ignore
list doesn't exist at HEAD (dead entry).

### L2 scheduled and dispatched runs share one cancel group (`ci.yml:17-19`)

`ci-${{ github.ref }}` + `cancel-in-progress` means a manual dispatch on main cancels the
only weekly deep scan (and vice versa), with no retry for 7 days.

### L3 CodeQL weekly scan is cancellable and nothing scans in between (`codeql.yml:11-13`)

Same group collision as L2; there is no push/PR trigger, so new code goes unscanned for up
to a week and an in-flight scan can be cancelled by a dispatch.

### L4 `actions-policy.yml` gaps

Reusable workflow pinned to a floating `faratech/ci-standards@main` (the only unpinned
external ref in the tree); `paths:` triggers don't include `.github/ci-policy.yml` itself
(editing the policy never re-runs the check); no concurrency block; and `.github/ci-policy.yml`
(f3f348d) lists 5 of 16 workflows, omitting the most expensive Store/Windows family.
Verified non-issue: the external workflow exists and the weekly runs succeed.

### L5 `claude-code-review.yml` — automatic PR review removed; the kept concurrency group is inert

7b544c6 dropped the `pull_request` trigger (dispatch-only now) but kept
`group: claude-review-${{ github.event.pull_request.number || github.run_id }}` —
`pull_request.number` never exists on dispatch, so the group is unique per run and
`cancel-in-progress` can never dedupe (62a779f's stated purpose). Should key on
`inputs.pr_number`. `.github/ci-policy.yml` still classifies the workflow as `event`.

### L6 `release-signed.yml` is vestigial but kept "live"

The signing job scans a fresh checkout for `*.exe/*.msix` and can only ever throw "No
signable artifacts found" (no build step, none committed); it uses archived
`actions/create-release@v1`; and this round's hardening added concurrency + attestation
permissions to it, with the group keyed `ref+version` so a cross-branch same-version
dispatch could race on one tag.

### L7 `scripts/README.md` documents `update-version.ps1`, which does not exist

Only `update-version.js` is present; scripts/README.md is CLAUDE.md's canonical pointer
for version bumps.

### L8 `check-reactor-readiness.py:29` — `EXPECTED_REACTOR_RUNTIME_MIN_VERSION` is defined but never enforced

`expected_pin` omits `windows_app_runtime_min_version`; a consistent drift of *both*
AppxManifest and manifest.json to a lower floor passes. The constant survives only in the
test fixture.

### L9 `rust-portable` is the only CI job without `timeout-minutes`

fe3f4f2 capped every other job (5/25/60/60/10); a hang burns the 360-minute default on the
longest Linux suite.

### L10 `bump-version.py:435` prints `git add -A && git commit` as its next-step hint

Directly contradicts CLAUDE.md rule 6 ("never `git add -A`") and scripts/README.md.

### L11 `bump-version.py` `update_json_file` never checks that its regex matched

`re.sub(count=1)` on a non-match silently writes unchanged content and reports success.
Latent only — all current targets match (mechanically verified).

### L12 `bump-version.py` Cargo.lock refresh is an uncounted 12th write whose failure doesn't affect the exit code

`refresh_cargo_lock` warns-and-returns on failure; the final exit compares counted writes
only, so the tool can exit 0 with a stale lock the next `--locked` build rejects.

### L13 `reactor-baselines/manifest.json:737` — stale placeholder narrative inside the single-source file

`official_reactor_release.summary` still says "crates.io still publishes only the
placeholder 0.0.0 … (re-checked 2026-09-01)", contradicting the adopted 0.100.0 in the same
file's `reactor_pin`; `check-reactor-readiness.py` renders it verbatim as a PASS message.

### L14 `validate-reactor.ps1:147,172,183` — stale `$LASTEXITCODE` if `python` cannot start

With `$ErrorActionPreference = "Continue"`, a `CommandNotFoundException` leaves the
previous suite's 0 in `$LASTEXITCODE` — false-green in aggregation, false-red standalone.

### L15 `package.json:15` — `npm run version-sync` runs the bumper, not the checker

`update-version.js` with no arg falls back to the current version: a no-op that "succeeds"
while verifying nothing. `scripts/check-version-sync.py` is the real checker.

### L16 PowerShell harness gaps (three, bundled)

`test-reactor-performance.ps1:15` `$Pages` lacks a `ValidateSet` (a typo'd page records a
"passed" run of the wrong page); `test-reactor-startup.ps1:96-102` sets validation knobs
with no feature probe and no `Set-StrictMode`; `capture-reactor-baselines/variants.ps1`
captures are not settings-hermetic (no `WFDIAG_REACTOR_SETTINGS_TEST_PATH` /
`WFDIAG_NO_TRAY`), so baseline maintenance writes to the developer's real settings.

### L17 `RunAsInvoker` never lands in the shipped binary

`__COMPAT_LAYER: RunAsInvoker` is runner env only; no embedded manifest with
`requestedExecutionLevel` exists in `apps/wfdiag`. Harmless today (MSIX full-trust; no
installer-detection heuristics match), but if the intent was to pin the product's elevation
behaviour, it lives only in CI.

### L18 rollback shell registers 15 commands with no frontend caller

`src-tauri/src/lib.rs:1097-1166` (`phi_silica::*`, credential commands,
`copy_minidumps_to_desktop`, …). Dead surface, drift hazard; pre-existing and distinct from
the decision-pending forks #250/#251/#255.

### L19 docs drift bundle

CLAUDE.md's `rust` CI row overstates the no-feature pass ("test … again with no features" —
the no-feature test is `-p wfdiag` only, clippy is whole-workspace); `scripts/README.md`
script set drifts from the tree (orphaned `test-reactor-ui-regressions.ps1` reference set,
undocumented `capture-store-baselines.ps1`, `migrate_store_draft.py`,
`store-release-as-secret.ps1`). (The CI-trigger and push-CI doc drift is M2; rule-1/4 and
#213 prose are L20/L21.)

### L20 (R2-1) #213 prose vs gate reality: the lock carries three windows-core families, not two

`Cargo.lock` has windows-core 0.61.2 (rollback webview stack only), 0.62.2, 0.100.0, and
`check-reactor-readiness.py:74-77` allowlists all three families — while CLAUDE.md says the
workspace "links two distinct windows-rs type systems" and "a third … fails a gate". A
reader believes a family fails closed when it is allowlisted. No exploitable path; the
shell file-level boundary is intact.

### L21 (R2-2) rules 1 and 4 overstate their contracts as written

Rule 1 ("src-tauri keeps only one-line `pub use` shims") vs 2,533 live lines in
`src-tauri/src/ai_providers/` (deliberate rollback, #284); rule 4 ("No env-var behaviour in
production … outside knobs.rs") vs `knobs.rs`' own header, which exempts the engine-crate
Phi vars (`WFDIAG_LAF_TOKEN`, `WFDIAG_AI_LOG`, `WFDIAG_ACTIVATION_ORDER`) by name and
documents `--wfdiag-elevated-relaunch` + `platform/crash.rs` LOCALAPPDATA. The real
contracts live in the knobs.rs header and issue #284; the prose should say so.

### L22 (R4-1) `apps/wfdiag/app-icon.rc:1` couples the product build to the rollback shell's assets

`ICON "../../src-tauri/icons/icon.ico"`, embedded unconditionally by `build.rs:73`
(`embed_resource::compile(...).expect(...)`). When src-tauri is deleted (scheduled "later
release"), `cargo build -p wfdiag` and the Store pipeline fail with an opaque rc error.
Move the icon into `apps/wfdiag/` (or shared assets) before then.

### L23 (R4-2) `window.rs:840-846` — `taskbar_created_message()` lacks the zero fallback its siblings have

On `RegisterWindowMessageW` failure the arm compares against 0 = `WM_NULL`, which the shell
itself posts after every `TrackPopupMenu` — each would run a remove+re-add tray cycle.
Cosmetic worst case; the asymmetry with `tray_callback_message`/`ui_wake_message`
(explicit `WM_APP` fallbacks) is the defect.

### L24 (R4-3) tray icon uses `LoadIconW` — only the 32×32 logical image, blurred at >100% DPI

No regression vs `ExtractIconW`, but 4c37733's goal is only partially realized on scaled
displays; `LoadImageW` with `LR_DEFAULTSIZE` per DPI is the exact fix. Lifetime is sound.

### L25 (R4-4) monitoring pause/resume decision moved out of testable `policy.rs` with no replacement test

6dc31b9 deleted `monitoring_lifecycle_action` (and its test) and inlined the logic in
`orchestration/lifecycle.rs:205-233`. The new logic is correct (the facade now derives
pause from `user_paused || !monitor_demand || !window_visible` — strictly better), but the
shell-side resume/refresh/status branches are exactly the pure decisions repo convention
keeps in `policy.rs`.

### L26 (R5-3) rollback sign-in child is not killed on cancel/timeout

`src-tauri/src/ai_providers/cli_bridge.rs` `run_sign_in_flow` (626ef2d): env scrubbed,
`CREATE_NEW_CONSOLE`, 600 s cap — but no `kill_on_drop`/JobObject, unlike the same file's
three other spawns (`:433/:533/:829`). Cancel or timeout detaches the vendor login console;
sign-in can complete after WFDiag reported failure. Rollback shell only; self-heals at next
probe.

### L27 (R5-4) dead `MAX_QUERY_CHARS = 16_000` in `engine.rs:21` shadows the real 420 bound

Only consumer is a src-tauri UI projection import; the sanitizer's actual bound is the
private `grounding.rs:51` (420). A future edit wiring the engine constant into the tool
path would loosen the untrusted-input query bound 38× unnoticed. Delete or rename the dead
constant.

### L28 (R5-5) non-streaming one-shot and error bodies have no byte cap on native HTTP providers

`anthropic.rs:389/435` (+`:96`), `gemini.rs:337/366`, `deepseek.rs:130` call
`.text()/.json()` uncapped; the new 2 MiB discipline covers SSE and openai_compat
streaming only. A broken/compromised trusted endpoint can force a large one-shot
allocation until the transport timeout. (Related to M6 but a different mechanism and fix
sites.)

### L29 (R3-3) `GenerateReport` spawns the report runtime before the AI-enabled gate

`ai.rs:922` `ensure_optional("report")` runs before `PendingAiProviderGate::evaluate`
(`:927`): a first report request with AI disabled builds a Tokio runtime + worker thread
and then rejects — the runtime stays resident forever, against 6dc31b9's lazy-start goal.
All sibling domains gate first. Introduced by 6dc31b9.

### L30 (R3-4) shutdown report honesty

`workers.rs:526-569` pushes `WorkerStopRecord::stopped(...)` (=`stopped_within_budget:
true`) for Monitor/Diagnostics/History/Provider/Update even when the handle was `None`
(never started); `:588-596` discards chat/report `stop_and_join(budget)` results, so an AI
worker that misses `AI_STOP_BUDGET` is absent from `ShutdownReport` rather than reported
over-budget.

### L31 (R3-5) same stuck-flag family, latent: update checks and subscription-account probes

A timed-out update reply leaves `update_request`/`snapshot.update.in_flight` set
(`mod.rs:1842-1866` vs `:2013-2017`), and `schedule(...)` then answers every future
`CheckForUpdates` with `Ignored "already running"` forever; same shape for
`subscription_accounts_request` (`ai.rs:502-506`). Both inner timeouts sit under the 30 s
facade deadline, so it needs a genuinely hung worker thread — latent, but the M5 fix
should cover this family systematically.

### L32 (R3-6) `MonitorRefresh` while effectively paused reports a misleading `WorkerUnavailable` — REJECTED in verification

`mod.rs:1368-1377` maps a `false` from `handle.refresh()` to "the monitor worker stopped";
`request_refresh` returns false simply because the monitor is deliberately not running
(user pause / hidden window / no demand). Wrong reject reason, no wedge.

**Rejected (#366 closed as invalid):** tracing the full chain, `request_refresh()`
returning false on a paused monitor makes `send_control_wake(required = false)` return
`!commands.is_closed()` — i.e. TRUE for a live worker — so the facade's `refresh()` only
returns false when the command channel is closed and the reject reason is accurate as
written. This is the round's one filed-then-rejected finding.

## Invariant audit summary

Sound after attack (highlights): the remediation single-execution-path with unforgeable
one-use expiring fingerprint-revalidated grants and the Repair gate in the broker (all
re-verified to line level); automation approving with `Reviewed` only behind two
default-off settings with audit trail; exactly ten read-only chat tools with enforced loop
bounds and forced final answer; the grounding sanitizer as the single machine-derived
query boundary (compile-time const endpoint, 4 MiB response cap); export path
canonicalization + closed URL set; API keys `skip_serializing` with closed `ProviderKeyId`
DPAPI set; history envelope version mutual-refusal + atomic writes; the trusted executor's
closed allowlist with exact-shape argument validation; spawn-site confinement (sign-in
console keeps its documented Job Object/KillOnDrop/scrub shape in the native shell); env
reads confined to documented lookups; `unsafe` confinement and the #213 file-level
boundary gate-enforced; reactor pin single-sourcing end-to-end (manifest ↔ Cargo ↔ gate
constants ↔ probe); provider capabilities/routing/consent/attribution tables exact; serde
wire names pinned with round-trip tests; all per-provider gotchas (Anthropic
max_tokens/refusal-first, Gemini header auth, no compat token cap, Foundry dynamic port,
Ollama no-default) verified in code; SSE 2 MiB + char-boundary handling; worker teardown
family clean (sender-before-join, reapers, budgeted joins); timeout nesting verified with
the sole exception M6; control-plane event survival under queue pressure; monitor
single-flight + superseded-reply contracts; wndproc/subclass/hook lifetimes; single-instance
SDDL; save-picker COM discipline; frontend XSS surface clean; version 2.5.9 consistent
across all 11 bump targets.

Violated / drifted this round: the CI contract (M2–M4, M9, L1–L6 — one cost-motivated
commit left main untested and the docs describing the gates stale); packaging/validation
tooling (H1, M7, M8, M11, L14, L16); engine single-sourcing hygiene (M12, L27); facade
liveness on timeout paths (M5, L31); shutdown/report honesty (L30); documentation prose
contracts (L19–L21, L13); and one red frontend gate (M1).

## Rejected / corrected during verification (selected)

1. Rejected (R6): "7b544c6 broke the with/without-features release-shape guarantee" — the
   no-feature pass tests `-p wfdiag`, which is the only package with the feature; coverage
   complete.
2. Corrected (workflow helper → R6/R2): "three windows-core versions = third type system"
   — the lock families are allowlisted deliberately (L20 records the prose drift; not a
   gate failure).
3. Corrected (workflow helper): "`faratech/ci-standards@main` doesn't exist / every trigger
   fails" — the weekly runs succeed (09-14, 09-21); reduced to floating-ref risk (L4).
4. Rejected (R6): "AboutDialog `open_url` is an unvalidated URL crossing the boundary" —
   open_url enforces the engine link policy; the mailto is fully percent-encoded.
5. Rejected (R5): charging bypass via uncharged SSE chunk fields; Gemini promptFeedback
   handling; `client_for` `/v1` double-append; stale `user_message_index` after trim — all
   disproven at line level.
6. Rejected (R3): 12 concurrency candidates including the new `ReplyWatcher` missing
   timeouts, monitor `spawn_blocking` deadlock, `Terminated` loss under pressure — each
   disproven against the current code.
7. Rejected (R2): Phi env vars as a rule-4 violation (exempted by name in knobs.rs);
   `ActionGrant::for_tests` (cfg(test), pub(crate)); `run_diagnostic` executing scans
   (read-only collector, never enters the session).
8. Rejected (R4): bootstrap DLL still expected anywhere (all remaining references are
   negative checks or the legitimate tauri path); WQL interpolation (validated at every
   site); monitor stale-baseline leak.

Adjudicated, not re-litigated: src-tauri forks (#250/#251/#255/#275/#277), WMI COM-affinity
design (#267), Ollama one-click setup (#30), Phi env vars (documented in PHI_SILICA.md),
`phi_silica_laf_token` in settings.json (capability token, prior-round decision).

## Test gaps (appendix)

- No test covers the reply-timeout path clearing request-side in-flight state (M5, L31) —
  the flag family has no headless coverage at all.
- No test pins `one_shot` transport timeouts on the compat family (M6) or byte caps on
  native-provider one-shot/error bodies (L28).
- Five `#[ignore]`d tests guard the subscription installer and mid-stream cancellation
  (M13) with no re-enable lane.
- No test exercises `build_probe` (the standalone-probe path) — H1 was invisible to the
  unittest because only subcommand handlers run under test.
- The monitoring pause/resume shell decision lost its policy test in 6dc31b9 (L25).
- No gate asserts the validation scripts' version pins move with `version.json` (M7) —
  the bump missed five of six.

---

# Addendum: post-audit delta regression review (R1) — completed

The dedicated delta pass (redone after the first reviewer was lost to rate limits) found
**no H/M defects in `96cf742..7b544c6`**. New findings, both verified and filed:

- **L33 (#367)** — a resumed report intent's Quick Scan mutates scan state after drain's
  mask-gated snapshot copy and pushes no event, so the host could render one cycle with the
  old phase. **Fixed** (drain re-checks the scan phase after `resume_pending_intent`).
- **L34 (#368)** — CLAUDE.md's facade contract omitted `SetMonitorDemand` and
  `take_snapshot_changes`. **Fixed** (contract updated).
- R1-1 (the five `#[ignore]`d installer/cancel tests should be Windows-scoped, since all
  pass on Linux) was folded into #334 and **fixed** with it.

Verified sound by the delta pass (highlights): the `fenced_segments` rewrite is
branch-equivalent and kills the O(n²) scan; `monitor_time_labels` matches the exact
floor-formula; the single-sample graph fix renders the true level; `ensure_optional`
builds its wake from the shared queue and all five domains gate before lazy start; every
observable scan mutation is mask-covered except the L33 hole; the new headless tests pin
real behaviour; the shell display eviction single-sources the policy through the engine's
pure `completed_history_cut`; PERFORMANCE_2.5.9.md's claims match the code; the rollback
shell is unaffected by demand-gated sampling (its defaults preserve legacy behaviour).

# Fix outcomes (same session)

All findings are closed: **44 fixed in 20 commits** (`ffac29a..f35ecc7` on `main`), **4
closed by owner decision** with "Decision needed" comments (#340 release-signed vestigial,
#351 RunAsInvoker embedding, #352 rollback dead commands, #358 tray DPI — the last two
belong in the hardware evidence lane), and **1 rejected during main-thread verification**
(#366 — the monitor-refresh reject reason is accurate; `request_refresh`'s paused-false
does not propagate to a false return for a live worker). Issues #321–#368 plus the two
delta findings #367/#368; one summary comment on tracking #182.

## Regression pass over the fix series, and its fixes

The adversarial pass over `ffac29a..f35ecc7` found **no high-severity regressions** and
two low findings, both fixed, filed, and closed (#369, #370 — `3f1c9b2`, `78e6f20`):

- **REG-1 (#369)** — `build.rs`'s `rerun-if-changed` still watched the old `src-tauri`
  icon path after `158c58f` moved the file; icon-only edits would not rebuild the
  resource. Fixed.
- **REG-2 (#370)** — the reply-timeout release covered only 3 of the 8 single-slot request
  ids (system info, architecture, process page/detail, network still pinned their domains'
  slots on a missed deadline). Fixed, with the unit test extended to all eight.
- The regression fix itself regressed (**#371**, medium): the release originally ran
  *before* the expired entries' `Internal` failure messages, so the request-id guards
  dropped the domains' own failure reports (`MonitorEvent::Unavailable` — the headless
  guard test `a_reply_that_never_lands_becomes_a_typed_timeout` failed 3/3). The release
  is now the mop-up after `apply_internal` (`8b63c7a`); all 17 facade suites and the full
  workspace battery pass.

Commit map (partial, theme → commit): reply-timeout flag release `ffac29a` (#326/#365);
one-shot transport timeout + body caps `fb48229` (#327/#362); scrub-list re-export,
input-cap rename, report gate order `4fa87f5` (#333/#361/#363); shutdown honesty `4077d85`
(#364); tray fallback + icon move + rollback kill_on_drop `278e03a`/`158c58f`/`531c2fe`
(#357/#356/#360); probe constant + AST lint `579fcc6` (#321); version pins `5700821`
(#328); probe timeout `b336169` (#329); pin single-sourcing `bafb924` (#331); gates
workflow + claude-review group `2770543`/`e93403d` (#330/#339); bump-script honesty
`fbfeb83` (#344/#345/#346); floor enforcement + narrative `5f26207`/`6051c32` (#342/#347);
version-sync script + portable timeout `8dd1570`/`344dcf8` (#349/#343); CI contract
`aac8836` (#323/#324/#325/#335/#336); cost-policy pinning `e380a40` (#337/#338); frontend
gate `887b5d5` (#322); Windows-scoped test ignores `1b2cb29` (#334); CLAUDE.md
reconciliation `33de049` (#354/#355/#368); policy re-extraction `f98990b` (#359); drain
freshness `873d90d` (#367); scripts/README `982a885` (#341/#353); validate-reactor python
resolution `140742e` (#348); perf/startup/capture harness `8223e1f`/`f35ecc7` (#332/#350).

## Terminal iteration (final re-scan)

The final-round re-scan of `3f1c9b2..a29edfd` found one medium (**M14, #372**): the
timeout-release fix was *dead code* for the system probe slots — system requests share one
completion channel and never register with the reply deadline tracker, so the real wedge
lived on the disconnect path, where a worker death left `system_info_request` held and
`start_scan` rejected every future scan forever ("administrator access is still being
detected"). Fixed in `eeea73d`, together with **L38 (#373)** — the process-detail failure
arm was the one silent monitor failure arm (page and network emit `MonitorEvent::Unavailable`).
One low follow-up (#374: the Store pipeline still sources icons from `src-tauri`) was
closed by owner decision, folded into the src-tauri deletion pass alongside #352.

The terminal re-scan of `eeea73d` returned **zero blocking defects** — all four of the
commit's claims verified correct, every attack angle (terminating guard, event
survival/order, re-entrancy, cross-talk) traced and either rejected or reduced to three
low observations: **L40 (#375)** — the specific failure reason was clobbered by the
generic stopped text within the same batch; fixed in `eb221bf` (WorkerStopped first, the
specific `SystemEvent::Failed` last). **L41 (#376)** — `MonitorEvent::Unavailable` is
query-kind-blind (pre-existing shape, widened to detail); closed by owner decision
(typed per-kind outcome events). **L42 (#377)** — no headless harness can drive the
system-worker disconnect release; closed by owner decision (review-only coverage for the
2-line path).

Convergence: findings per pass shrank M → L → cosmetic observations, and the terminal
pass found nothing blocking. A closing review of `eb221bf` itself — the last commit, so
that every commit in the series had an adversarial pass — returned **CLEAN**: the reorder
fixes the #375 clobber for the only order-sensitive consumers (the native shell's status
line, last-writer-wins), introduces no queue-pressure regression (identical victim sets
at capacity; the durable `snapshot.system_error` → chrome banner path is order- and
eviction-independent), preserves the slot-clearing guarantees, and breaks no consumer in
either shell. Total for round 3: **57 findings (#321–#377), all filed and all closed** —
49 fixed in 35 local commits (`ffac29a..eb221bf` on main, unpushed), 7 closed by owner
decision (#340, #351, #352, #358, #374, #376, #377), and 1 rejected during main-thread
verification (#366). Full battery green at `eb221bf`.

# Final battery after the fix series

`cargo fmt --check`, portable clippy `-D warnings`, portable workspace tests (run twice),
`cargo test -p wfdiag-app`, version-sync / store-identity / external-gates / both gate
unittests, xwin clippy x64 + aarch64, `tsc --noEmit`, `eslint .`, `vitest run` (208) —
**all green**. `check-reactor-readiness.py` remains at its designed NOT-READY. One flake:
`a_long_conversation_survives_worker_history_eviction` failed once in the first
full-workspace parallel run and passed in isolation (4/4), the full suite (3/3), and the
second full run — added to the flaky watch list next to
`closing_a_status_reply_cancels_active_probe_and_unblocks_shutdown`.

Housekeeping note: commit `e93403d` normalized `claude-code-review.yml`'s mixed line
endings to LF while changing the concurrency group; content change is otherwise exactly
the two-line group fix.
