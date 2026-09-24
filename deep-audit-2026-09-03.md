# WFDiag deep audit — 2026-09-03

Full-workspace review for bugs, performance issues, and security vulnerabilities at
`wfdiag-2.6` (`097c655`). Method: baseline tool signals (clippy `-D warnings` clean,
`cargo test -p wfdiag-app` 161/161 green, `cargo audit` allowed-warnings only), then six
area-partitioned review agents over all 19 engine crates, the native shell, the rollback
shell, the frontend, and `scripts/`, each instructed to attack the CLAUDE.md security-model
invariants as claims. Every reported finding was re-verified in the main thread against the
source (cited lines and callers re-read); three sub-claims were rejected during that pass
(listed at the end). **51 findings: 5 high, 19 medium, 27 low.** Issue IDs below match the
GitHub issues filed from this audit (`Reactor audit 2026-09-03 <id>`).

Fix plan markers: **fix** = fixed in this audit's fix series; **filed** = issue filed, fix
needs a decision (architectural / gate-adjacent / rollback-shell deletion).

## High

**H1 — Automation verification with ≥2 source tasks wipes the committed scan and reports false "resolved". [bug · fix]**
`crates/wfdiag-app/src/service/mod.rs:729` gates the targeted-overlay path on
`task_ids.len() == 1`; with 2+ tasks, `begin_verification` (`service/ai.rs:1688`) opens a
*replacement* scan transaction. On commit, `domain/scan.rs:684`
(`self.snapshot.results = authoritative_rows(...)`) replaces the whole committed scan with
the verification rows, `Invalidation::on_scan_start(false)` clears the AI report/analyses/
fix plan, and auto-save can write a 2-task "Manual Diagnostic" history record.
`domain/actions.rs:160-170` then counts every issue whose source task wasn't re-collected as
*resolved*, and `ActionEvent::Verified` + an audit `verified` line assert fixes worked that
were never re-checked. No headless test covers the ≥2-task shape.
Fix: open the overlay for the actual requested task set (or re-collect without swapping the
visible snapshot), and make `verification_result` treat "source task absent from new
evidence" as *unknown*, not resolved.

**H2 — Streamed AI report text is doubled and stale text resurrected. [bug · fix]**
The facade appends each chunk to `snapshot.ai.report.text` *and* queues
`ReportEvent::Delta` (`crates/wfdiag-app/src/service/ai.rs:2283-2292`). The shell copies the
snapshot body into `report_text` first (`apps/wfdiag/src/app/orchestration/events.rs:41`,
`:187`), then `screens/ai/update.rs:492-496` pushes the same chunk again — every chunk after
the first is doubled; the view renders exactly this field.
Fix: one owner for the body — drop the shell-side `push_str` (snapshot copy is
authoritative), or stop mirroring deltas into the snapshot until `Done`.

**H3 — Readiness gate never checks the Windows App Runtime floor. [security · fix]**
`scripts/check-reactor-readiness.py:1047-1051` reads only `PackageDependency@Name`;
`MinVersion` is never compared to `reactor_pin.windows_app_runtime_min_version` (no hit in
the script), and the project's own test fixture builds the dependency with
`MinVersion="1.0.0.0"` and asserts `report.ready`. A manifest lowered to `0.0.0.0` reports
READY.
Fix: parse `MinVersion`, compare against the baseline, add a mismatch test.

**H4 — Production env override silently re-points the grounding endpoint. [security · fix]**
`crates/wfdiag-native-ai-chat/src/grounding.rs:506-508` reads
`WFDIAG_WINDOWSFORUM_MCP_URL` ungated (not behind `validation`, not in `fixtures/knobs.rs`,
in no doc), with no scheme/host validation before every MCP POST — violating rule 4 ("no
env-var behaviour in production"; `knobs.rs`' own header exempts only the three phi vars by
name). Any same-user process can redirect sanitized scan-derived queries to an arbitrary,
possibly cleartext host and feed attacker text back into analysis/reports. Two reviewers
independently flagged it.
Fix: delete the read (compile-time constant) or move it behind `#[cfg(feature =
"validation")]`.

**H5 — Duplicate readiness gate id silently flips BLOCKED → PASSED. [security · fix]**
`scripts/check-reactor-readiness.py:1100-1104` folds gates into an id-keyed dict; a later
duplicate overwrites the earlier entry, `missing = required - keys` can't see it, and only
the survivor is evaluated — appending `{"id": "store_packaging_validation", "status":
"passed", …}` greens a hardware-evidence gate. `backend_parity` detects duplicate surface
ids (`:1177`); the gate path doesn't.
Fix: reject duplicate gate ids as an error, like the surfaces path does.

## Medium

**M1 — Verification verdict is not bound to the re-collected evidence. [bug · fix]**
`service/mod.rs:2493-2508` completes `self.verification.take()` on *any* accepted
projection; a host `RefreshIssues` racing the post-fix rerun produces the pre-fix verdict
(`unresolved: [all]`), audited, and the real projection later finds `None`. The code comment
directly above claims the opposite. (Spot-check correction: the `begin_verification` failure
fallback does *not* arm a verification — only the race path is real.)
Fix: carry the targeted session id in `PendingVerification` and complete only from a
projection committed from that session.

**M2 — A failed/cancelled verification scan strands the automation session. [bug · fix]**
`automation.await_projection` is cleared only in the issue-projection branch
(`service/mod.rs:2511-2514`). If the verification rerun fails to start, fails, is
cancelled, or the issues worker stops, no projection ever arrives: no
`SafeFixesFinished`, no closing audit line, and every later `RunSafeFixes` is refused
`Busy`. Also: an empty plan writes `safe_fixes_planned` with no matching `finished`.
Fix: watch the rerun's non-committing outcomes (`ScanStartFailed`, `Failed`,
`Incomplete`, `Cancelled`, worker stop) and finish the session on any of them.

**M3 — Task deadline is escaped by the post-failure command fallback (DISM runs twice). [bug · fix]**
`crates/wfdiag-native-diagnostics/src/catalog.rs:483-521` bounds only the collector join;
on `Err`, the fallback `run_command("dism", …)` runs outside the deadline (executor-capped
at 300 s). `dism_scan_health`'s "native" implementation *is* the same DISM command
(`native_diagnostics.rs:1021-1023`), so a slow non-zero exit re-runs a second full
`/scanhealth`; `CancelScan` cannot interrupt the in-flight collector.
Fix: run the fallback inside the remaining deadline, and don't re-run the command that just
failed.

**M4 — `get_disk_fragmentation` runs one 300 s-capped `defrag` per drive against a 240 s deadline. [perf · fix]**
`native_diagnostics.rs:1140-1151` loops drives sequentially; after the task is correctly
reported timed out, the abandoned blocking thread keeps spawning `defrag /A` per remaining
drive (minutes of background disk work, a pinned blocking-pool thread).
Fix: one shared deadline across the loop.

**M5 — `Timestamp::from_iso_string` panics (release = abort) on non-ASCII input. [bug · fix]**
`crates/wfdiag-native-core/src/timestamp.rs:134-155` byte-length-checks then byte-slices
(`s[0..4]`, …); one multi-byte char in the first 19 bytes panics `byte index not a char
boundary` despite the `Result` contract. `impl Deserialize for Timestamp`
(`:215-223`) routes every persisted timestamp through it (history `ScanRecord`, audit
`at`), and the release profile sets `panic = "abort"` — a malformed document kills the
process instead of a typed error.
Fix: parse bytes (`as_bytes()`/`get()`), so every failure is `Err`.

**M6 — Per-step timeout lets multi-step AutoSafe actions run N× their budget. [bug · fix]**
`crates/wfdiag-native-remediation/src/remediation.rs:706-717` hands the same
`timeout_secs` to every step: `start_critical_services` (5 × 120 s) is catalogued
`AutoSafe`, `long_running: false` — a worst-case ~10-minute unattended batch holding the
single action slot (`runtime.rs:329-331`).
Fix: one shared deadline across the sequence, or reflect the true worst case in
`long_running` (automation then treats it as long-running).

**M7 — MCP SSE assembly corrupts multi-byte UTF-8 at chunk boundaries and grows unbounded. [bug · fix]**
`crates/wfdiag-native-ai-chat/src/grounding.rs:955-962` converts each network chunk with
`from_utf8_lossy` separately (a straddling em-dash/CJK char becomes U+FFFD, the JSON frame
fails to parse → intermittent "did not include a result" grounding failures) and
accumulates the body with no byte cap (only the transport timeout bounds it — the #204
class).
Fix: accumulate `Vec<u8>`/`Bytes`, decode once, cap total bytes.

**M8 — The chat tool `search_windows_knowledge` bypasses the single grounding boundary. [security · fix]**
`grounding.rs:267-287` only `compact_text`s the model-authored query before POSTing it to
the MCP endpoint; `SAFE_QUERY_FIELDS`/`safe_value_term` are applied on the analysis path
only. Diagnostic output is attacker-influenced, so prompt injection can push scan-derived
text into the query (bounded at 420 chars, HTTPS). The module header ("nothing reaches the
network unless…") is false for this API.
Fix: run the tool query through the same allowlisted reduction, or narrow the documented
invariant explicitly.

**M9 — Hitting `MAX_STREAM_CHARS` stalls the turn instead of truncating it. [bug · fix]**
`engine.rs:409-414` sets `receiver_open = false` and stops draining; `openai_compat.rs:404`
(`tx.send(content).await`, channel cap 256 at `engine.rs:360`) blocks forever → the turn
burns the 180 s budget and reports a timeout, discarding a fully generated answer (and the
retry stalls again). Same lines: `streamed.chars().count()` per delta is O(n²).
Fix: keep draining and discarding past the cap; track a counter; prefer `try_send` in
`openai_compat`.

**M10 — The settings save echo re-injects plaintext API keys into shell state. [security · fix]**
`apps/wfdiag/src/dialogs/settings/update.rs:364-371` writes the plaintext drafts into the
submitted document; the facade clones it as the echo (`service/mod.rs:1630`) and applies
the echo to the snapshot + `SettingsFact::Saved` (`:2414-2418`); the shell copies that into
`shell.settings` and the draft (`events.rs:60`, `update.rs:795`) — *after* the drafts were
wiped. Keys never reach disk (`for_disk`/`skip_serializing` hold) and no Debug/log sink
exists, but the code comment claims the document carries "only the `*_configured` flags".
Fix: submit `None` key values + flags (the `ProviderCredentialCommand::Commit` already
carries the plaintext), or have the facade echo a `for_disk()`-equivalent.

**M11 — The palette's command catalogue is rebuilt every frame while closed. [perf · fix]**
`apps/wfdiag/src/app/mod.rs:649-651` evaluates `self.palette_command_specs()` eagerly
(~200 allocations + linear scans per render); `PaletteDialog::view` discards it when closed
(`dialogs/palette/overlay.rs:32,146-147`) — contradicting the zero-cost-overlay contract
in the adjacent comment.
Fix: build specs only when `self.palette.open`.

**M12 — `sync_from_snapshot` deep-clones the read model on every message, twice per event batch. [perf · fix]**
`app/mod.rs:472` syncs unconditionally at the end of every `update()` (including pure-UI
messages) and `events.rs:41` syncs again per batch; each sync clones settings, the 49-task
catalog, results, issues, history, provider rows, and large AI strings (O(n) per streamed
report chunk → O(n²) per stream).
Fix: sync once per drained batch; share bulky strings (`Arc<str>`/`Cow`).

**M13 — Rollback shell's vendored `cli_bridge` fork reintroduces the unbounded child-output bug (#204). [security · filed]**
`src-tauri/src/ai_providers/cli_bridge.rs:843` still uses `child.wait_with_output()`
(unbounded per-pipe Vecs) for every bridge call in the rollback shell; the engine copy was
fixed with `drain_bounded` + byte caps (`crates/.../cli_bridge.rs:315-345`). The fork is
also a direct rule-1 violation (shell must be thin shims).
Fix (decision): delete the fork and route through the crate, or port `drain_bounded`.

**M14 — Rollback shell collapses the 3-state CLI verdict to 2 and caches "signed out" from an `Unclear` probe. [security · filed]**
`src-tauri/src/ai_providers/cli_bridge.rs:249-256` (`is_signed_in`), `:231-238`
(`conclusive = true` on the non-zero path) cache a fabricated "signed out" for the 30 s TTL
when a vendor status exits non-zero transiently — the exact failure the engine's
`StatusVerdict::Unclear` ("never reported as signed out") was written to prevent; the
shell's own comment records the consequence (silent fall-through to metered cloud keys).
Fix (decision): consume `subscription_spec::parse_status_output` from the crate.

**M15 — MSIX `pack`/`validate-layout`/`validate-msix` accept *added* capabilities. [security · fix]**
`scripts/build-reactor-msix-probe.py:343-349` enforces the exact capability set only when
`expected_capabilities` is passed (only the render path does); `:556` and `:700` validate
required-subset only — an added `broadFileSystemAccess` ships with exit 0.
Fix: pin the canonical set in the validate paths too.

**M16 — `check-external-gates.py` pin check is a dead loop and `drift` exits 0. [security · fix]**
`scripts/check-external-gates.py:89-94` assigns the constant it initialized (reading
`apps/wfdiag/Cargo.toml` cannot change any outcome; both pins are hardcoded literals, not
read from `reactor-baselines/manifest.json`), and `main` (`:176-177`) returns 1 only for
`actionable` — deleting `AppxManifest.xml` yields `drift` + `incomplete`, exit 0.
Fix: read pins from `reactor_pin`; map drift/missing-input to a failing exit.

**M17 — The MSIX probe hardcodes the framework name *and* floor, then "validates" them against its own constants. [security · fix]**
`build-reactor-msix-probe.py:49-50` literals are injected and checked tautologically
(`:267-268`, `:367-369`); `bump-version.py`'s claim that the floor is single-sourced is
false twice (with M16). Raising the baseline floor changes nothing the probe emits.
Fix: load both values from `reactor-baselines/manifest.json`.

**M18 — Rollback shell re-implements `AUTO_FALLBACK_ORDER` by hand. [bug · filed]**
`src-tauri/src/ai_service.rs:296-355` restates the local-first trust ordering as seven
hand-written branches although the crate exports the ordering table; an added provider or
reorder silently leaves the rollback shell routing local→cloud wrong.
Fix (decision): drive both helpers from `wfdiag_native_ai_provider::fallback`.

**M19 — Unbounded link-target scan makes markdown parsing quadratic on the UI thread. [perf · fix]**
`crates/wfdiag-native-projection/src/markdown.rs:385` bounds the label search to 512 chars
but scans `link_target_end` over the entire remaining document; `[x](`-heavy model output
(100 KB message ≈ 25 000 unbounded scans) is parsed on the UI thread, re-rendered per
streaming delta (`screens/ai/view.rs` per message). The crate already fixed this class for
`**` spans.
Fix: bound the target scan like `bounded_tail`.

## Low

**L1 — `get_disk_info_optimized` trusts the API's returned length over its own 256-char buffer. [bug · fix]**
`crates/wfdiag-native-monitor/src/monitor.rs:1691-1702` — `while i < len as usize` can
index past the array (panic) or, when the buffer is left untouched on
insufficient-buffer, silently report zero disks. Fix: clamp to `buffer.len()`, retry
right-sized.

**L2 — Kernel process-record walk has no bounds check and forms an aligned reference into `Vec<u8>`. [bug · fix]**
`monitor.rs:1899-1936` — no `offset + size_of::<SystemProcessInfo>() <= buffer.len()`
check, `next_entry_offset` trusted, alignment UB by the letter (comment admits it).
Fix: bound the walk; `read_unaligned` or `#[repr(C, align(8))]`.

**L3 — Every settings load DPAPI-decrypts all five provider secrets just to compute booleans. [perf · fix]**
`crates/wfdiag-native-settings/src/lib.rs:615-624`, `:712-719` — `load()` (and hence every
`update()`) materializes five plaintexts to test emptiness. Fix: existence/size-based
`is_set` (the CLI credential-store fast path already does this).

**L4 — Provider credential plaintext lives in non-zeroizing `String`s on the staging path. [security-hygiene · fix]**
`lib.rs:322-365`, `:739-782` — staged mutations and rollback snapshots hold plain
`String`s (no disclosure path exists today; the DPAPI boundary itself is `Zeroizing`).
Fix: `Zeroizing<String>` like the protector boundary.

**L5 — Claude ACP prompt path has no pipe byte cap and no cap on accumulated answer text; child not in a Job Object. [perf · fix]**
`providers/acp_bridge.rs:348-352`, `:396-401` — the model-list path caps stdout
(`:179`) but the prompt path doesn't; `collected` grows unbounded until the 170 s timeout;
`kill_on_drop` covers only the direct child (grandchild orphan on cancel).
Fix: same `take(limit)` discipline; cap `collected`; Job Object like the installer.

**L6 — A forced-final answer that also carries tool calls is discarded after being streamed. [bug · fix]**
`engine.rs:806-811` errors when `final_round && !tool_calls.is_empty()` even with a
non-empty answer; compat servers do this routinely — the user loses a complete answer.
Fix: only error when the answer is empty; drop stray tool calls.

**L7 — The CLI status probe still uses unbounded `command.output()`. [perf · fix]**
`crates/wfdiag-native-ai-provider/src/local_probes.rs:111-118` — the last `#204` holdout
(10 s timeout bounds time, not bytes); also a drifting copy of `bridge_workdir` helpers.
Fix: reuse the bounded runner.

**L8 — The worker-owned chat history is never trimmed on the native path. [perf · fix]**
`crates/wfdiag-native-ai-chat/src/runtime.rs:665,726` — `WorkerState.messages` grows every
turn (only `truncate_failed_turn`/`Reset` shrink it); `trim_completed_session`'s sole
production caller is `finish_session_with_tools`, which only the **rollback shell** calls.
Fix: trim after each turn, keeping the trailing turn.

**L9 — Battery health is computed from the oldest capacity-history row. [bug · fix]**
`native_diagnostics.rs:1389-1391` — `.first()` bound to a variable named `latest`
(powercfg lists periods chronologically ascending) → optimistic health on recently
degraded batteries. Fix: `.last()` + fixture test.

**L10 — `reset_windows_update` reads child pipes only after exit and ignores its cancellation token. [bug · fix]**
`remediation.rs:1113-1146` — a child that fills its pipe blocks until the 60 s kill;
`_cancel` never consulted (catalog marks the action `cancellable: false`, so the runtime
refuses rather than cooperates). Fix: drain on threads with the deadline; honour cancel
between phases.

**L11 — One fresh WMI/COM connection per task, several collectors opening 2-3. [perf · fix]**
`wmi.rs:103-144` + `native_diagnostics.rs:2088,2107` — ~25+ connection setups per scan
where one per namespace would do. Fix: reuse per-namespace connections across the scan.

**L12 — A missing/disabled Windows Update event channel fails the whole task. [bug · fix]**
`native_diagnostics.rs:2308-2324` — failure query `?`, success query
`.unwrap_or_default()`; enterprise-disabled channel → "unknown" instead of an honest zero.
Fix: tolerate both halves symmetrically (+ `channel_available` flag).

**L13 — `get_scheduled_tasks` enumerates the entire tree, then truncates to 200; all-disabled → task failed. [perf/bug · fix]**
`native_diagnostics.rs:2063-2082` — seven COM calls per task for entries then discarded;
`Err("No scheduled tasks found")` on a healthy machine. Fix: budget during enumeration;
empty-but-successful result.

**L14 — `EvtNext` errors are conflated with end-of-results, silently truncating evidence. [bug · fix]**
`native_diagnostics.rs:321-328` — any error (not just `ERROR_NO_MORE_ITEMS`) breaks the
loop as exhaustion, so a transient failure yields a prefix while the collector reports
success. (Spot-check correction: the `250` is the batch size, not a timeout — timeout is 0.)
Fix: distinguish the error code; surface `partial_results`.

**L15 — Chat fallback re-captures the tool evidence per attempt. [bug · fix]**
`crates/wfdiag-app/src/service/ai.rs:660` — `chat_tool_snapshot()` runs on every attempt
including retries, contradicting `domain/consent.rs`'s "captured **once**" invariant (and
paying a deep clone per retry). Fix: capture once in `begin_chat_turn`, thread through.

**L16 — `temp_file_count()` walks all of `%TEMP%` on the host thread per scan/refresh; audit writes are synchronous. [perf · fix]**
`crates/wfdiag-app/src/ports/mod.rs:55-59` + `ports/audit.rs:131-152` — unbounded
`read_dir().count()` inside `drain` on machines with huge temp dirs; audit entries
open/write/rotate synchronously on the host thread. Fix: move into the worker request;
amortize; keep an open file handle with rotation.

**L17 — `scan_fingerprint` re-hashes every task output at every remediation boundary. [perf · fix]**
`crates/wfdiag-app/src/domain/actions.rs:49-71` — O(total output bytes) per commit/approve/
projection where the value only changes with `issue_generation`. Fix: memoize against the
generation.

**L18 — A stale "history was not saved" error survives scans that never attempt auto-save. [bug · fix]**
`apps/wfdiag/src/app/orchestration/events.rs:311-317` — `Finalized { history: None }`
never clears `history.error`. Fix: treat `None` as "nothing attempted" and clear.

**L19 — Window-less-testable parse/projection policy lives in the shell (rule 2), plus 5× duplicated subscription wire-ids. [hygiene · filed]**
`apps/wfdiag/src/screens/diagnostics/view.rs:301-306,354-378,533-580` (with its own
`#[cfg(test)]` module) belongs in `wfdiag-native-projection`; wire-id literals at
`app/policy.rs:263,268,360-365`, `dialogs/settings/update.rs:520-523,572-575`.
Fix (decision): move the projection into the crate; one shell helper for wire ids.

**L20 — The toast AUMID/PFN is checked by no identity script. [security · fix]**
`apps/wfdiag/src/platform/notifications.rs:180` — the PFN exists in
`reactor-baselines/manifest.json` but `check-store-identity.py` opens five files, no `.rs`;
a rebrand that passes all five checks drops every toast silently. Fix: add the AUMID to the
script, sourced from `reactor_pin`.

**L21 — The URL-scheme allowlist exists in three places that already disagree. [hygiene · filed]**
`src-tauri/src/lib.rs:968-975` (http/https/mailto), `src/screens/DiagnosticDetail.tsx:16-22`
(http/https only, file-private), `crates/wfdiag-native-projection/src/markdown.rs:112-127`
(full policy). No exploit today (call sites re-validate). Fix (decision): one decision
function exported from the projection crate.

**L22 — A superseded window's tray icon is never removed after a remount. [bug · fix]**
`apps/wfdiag/src/platform/window.rs:942-949` — `WM_NCDESTROY` cleanup (incl.
`remove_tray_icon`) is gated on `is_registered_main_window`, so a replaced HWND leaves a
dead icon until Explorer restarts. Fix: track the HWND that owns the live icon.

**L23 — The "no WebView dependency" gate reads only the shell manifest, not the 16 linked crates. [bug · fix]**
`scripts/check-reactor-readiness.py:444-447` — workspace members resolve as
`{ workspace = true }` and are never opened; a `wry` dependency in an engine crate would
pass. Fix: walk `workspace.members`.

**L24 — `bump-version.py` reports success on partial pattern matches and accepts a trailing newline. [bug · fix]**
`scripts/bump-version.py:298-309` (any-one-pattern counts as updated), `:329`
(`re.match` with `$` accepts `"2.6.0\n"`; `check-version-sync.py:64` correctly uses
`fullmatch`). Fix: require every pattern; `re.fullmatch`.

**L25 — `ResolvedProviderConfig` derives `Debug` including `api_key`. [security-hygiene · fix]**
`crates/wfdiag-native-ai-provider/src/provider_config.rs:10-16` — no reachable sink today,
but the redacting sibling (`ModelCatalogRequest`) shows the intended pattern. Fix: manual
`Debug` that redacts the key.

**L26 — CLAUDE.md's native-shell layout is stale (docs). [docs · fix]**
`apps/wfdiag/src/ai/` does not exist (the tool backend lives in
`crates/wfdiag-app/src/ports/chat_tools.rs`), and `app/orchestration/` holds only
`commands.rs, events.rs, lifecycle.rs, mod.rs, route.rs` — not the 13 concern modules
listed. Fix CLAUDE.md's "Native shell layout" section.

## Invariant audit summary

Sound after attack (highlights): single remediation execution path with unforgeable
one-use expiring fingerprint-revalidated grants and the Repair gate in the broker; argv
compile-time-constant with a closed System32 allowlist; `fs_atomic` staging discipline;
automation approvals (`Reviewed` only) and planner (`AutoSafe`, no-restart) gating;
default-off automation settings; read-only ten-tool set with bounded loop; no OAuth/token
storage; credential-store presence-only probing; sign-in Job Object/KillOnDrop/10-min cap;
env scrubbing, stdin-not-argv, nested bridge timeouts; provider wire gotchas (Anthropic
`max_tokens`/refusal-first, Gemini header auth, no compat token cap); explicit-preference
never falls back; keys never serialized to settings.json; history DPAPI envelope
version confusion; export destination canonicalization and closed URL set; hostile update
JSON handling; markdown link policy at render; fix-plan validation against detected issues
and catalog; prompt/data budgets; `UiEvent` bus bounded/lossless; monitor pagination and
counter guards; wndproc/subclass/hook lifetime soundness; single-instance security
descriptor; save-picker COM discipline; clipboard privacy; frontend XSS surface; Tauri
capability surface.

Violated: grounding boundary (H4 endpoint control, M8 tool-path bypass; the "no env-var
behaviour in production" rule); "scans cannot stall on one collector" (M3, M4); the
documented verification/audit invariant (H1, M1, M2); the single-sanitizer claim (M8);
shell thinness (M13, M14, M18, L19, L21); single-sourced floor (M16, M17); chat-tools
evidence-capture contract (L15); palette zero-cost contract (M11); sync-per-drain contract
(M12); rule 2 (L19).

## Rejected / corrected during verification

1. Rejected: "the `begin_verification` fallback arms a verification over stale evidence"
   — the fallback calls `refresh_issues()` without setting `self.verification` (only the
   `RefreshIssues` race, M1, is real).
2. Corrected: "`EvtNext` uses a 250 ms timeout" — 250 is the batch size; the timeout
   argument is 0 (L14 reworded, substance unchanged).
3. Corrected: "`trim_completed_session` has no non-test callers" — it has one, inside
   `finish_session_with_tools`, which only the rollback shell calls (L8 reworded,
   substance unchanged for the native product).

## Test gaps (appendix)

- No headless test covers a verification rerun with ≥2 source tasks (H1), a verification
  race against `RefreshIssues` (M1), or any non-committing verification-scan outcome (M2).
- No test pins `MinVersion` in the readiness contract (H3) or rejects duplicate gate ids
  (H5); the existing fixture *bakes in* `MinVersion="1.0.0.0"`.
- No test covers a multi-byte char straddling an SSE chunk boundary (M7) or the
  `MAX_STREAM_CHARS` cap on a non-`try_send` transport (M9).
- No test covers `Timestamp::from_iso_string` with non-ASCII bytes (M5) — a panic test
  would have caught it.
- No test covers a verification batch spanning multiple drives/per-step timeouts (M4, M6),
  the ≥2-task chat session trim (L8), or `powercfg` battery row ordering (L9).
- Gate scripts: no test asserts the validate paths pin the capability *set exactly* (M15),
  that drift/missing inputs fail the exit code (M16), or that the probe reads the floor
  from the baseline (M17).


---

# Iteration 2 (same day): second pass + regression review — all findings fixed

A second sweep (six reviewers: one hunk-by-hunk regression review of the fix series
itself, five covering the areas iteration 1 under-read) produced **34 more findings,
filed as #283–#316**. A later provider/event pass added **#317–#320**. Every one is now
**fixed and closed** (44 first-pass fixes + 34 second-pass fixes + 6 = 84 closed issues,
commits `d3b3fed..96cf742` on `wfdiag-2.6`), with three follow-up commits for
Windows-target compile fallout (20efa8b, 85646c2, plus fmt). Highlights of iteration 2:

- The regression review caught **two real regressions in iteration 1's own fixes**:
  the temp-count cache made post-fix verification re-detect cleared issues (#283 —
  reverted), and the workspace-wide WebView scan permanently blocked `ui.native` on the
  deliberate rollback shell (#284 — src-tauri excluded, documented). This is exactly why
  the fix series gets its own adversarial pass.
- A flaky headless test was traced to a **real production bug**: after a sign-in/out the
  facade's own status refresh re-projected the account row from the TTL-cached
  pre-operation probe (#316) — fixed by invalidating the process-global probe cache on
  auth completion, mirroring the auth truth in the mock, and bounding the test's waits.
- Windows-reactor: **adopted the official crates.io 0.100.0 release** (superseding the
  git-revision pin, per the pin policy's anticipated move), applying its API differences
  (setup crate's implicit framework-dependent default; windows-core 0.100's private
  Type/TypeKind now under `imp::`).

Also verified sound and recorded: the task_output move (56cde1c) is behaviour-identical;
evidence prompt-injection defences hold; cache-key identity is sound; no XSS sink in the
frontend; all Tauri commands registered; the rollback broker still gates Repair.

**Final battery at 96cf742:** fmt clean; Linux clippy `-D warnings` clean; 823 workspace
tests + 164 facade tests green; xwin clippy clean on x64 and aarch64; version sync,
store identity, external gates clear; readiness at its designed NOT-READY (hardware
evidence outstanding).
