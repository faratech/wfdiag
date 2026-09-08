# Native-shell performance work — 2.5.9

## Implemented

- **Preserved freshness:** one-second system telemetry and the existing
  two-second process refresh/cache interval. The release optimization profile
  and Reactor dependency pin are unchanged.
- **Independent monitor lifecycle facts:** periodic sampling requires host
  visibility, a live-page consumer, and no explicit user pause. Returning from
  another page or restoring the window cannot clear that user pause.
- **Less UI synchronization:** facade-owned snapshot invalidation survives
  event draining and includes synchronous command changes. Telemetry and chat
  events invalidate only their domains; cross-domain workflows conservatively
  invalidate all domains. Composer-only changes consume no snapshot data. The
  enclosing native wake no longer repeats its nested event synchronization.
- **Completion-driven replies:** oneshots register a host waker; the shared
  watcher sleeps until the earliest reply/startup deadline. AI and scan workers
  already publish wakes and no longer cause a separate 50 ms polling loop.
  Legacy issue/export/system channels still use bounded, outstanding-work-only
  polling. Bounded internal/AI drains explicitly schedule another wake when full.
- **Bounded conversations:** worker history retains at most 100 messages and
  512 Ki characters; display history retains at most 200 messages under the
  same character budget, evicting whole turns. Terminal reconciliation precedes
  eviction, including cancellation/failure paths, avoiding stale slice indexes.
  Stream prefixes respect Unicode boundaries and the 256 Ki-character display
  ceiling, including the final channel drain. Anthropic/Gemini raw SSE input and
  OpenAI-compatible accumulated text/refusal/tool data have a 2 MiB budget.
- **Stable Markdown rendering:** an input-compared Reactor component prevents
  unchanged Markdown from being reparsed/rebuilt on unrelated updates. Chat keys
  use turn and role rather than a shifting history index. Fence-language parsing
  stops at the first invalid byte instead of rescanning the remaining document.
- **Bounded process collection:** one active blocking capture plus one latest
  pending query. Superseded requests release their replies immediately. Reply
  projection runs on the collector, eliminating a waiting OS thread per query.
  Cache age starts at capture completion. CPU-only queries bypass GPU locking;
  failed adapter queries cannot force rediscovery every second.
- **Lazy optional AI workers:** report, analysis, fix-plan, model-catalog, and
  subscription workers no longer start with the application. They are retained
  after first use; failed starts keep the existing unavailable diagnostics.
  Auth/install are created together on first subscription operation because the
  existing subscription port exposes one paired factory. Chat teardown and eager
  remediation recovery are unchanged.

The prior process-row identity and monitor-graph parity fixes are retained.
Version sources are synchronized through the repository tooling at 2.5.9.

## Verification and limits

Portable workspace tests and strict Clippy passed, including a 60-turn real
chat-worker regression, lifecycle/demand guards, direct completion wakes,
deadline-only wake behavior, lazy-worker retention, Unicode/history limits, and
adversarial Markdown input. Windows x64 production-shape and ARM64 validation
cross-target Clippy passed. The ARM64 release validation executable built and
the Windows-native monitor runtime tests passed, including 1,000 queued queries
coalescing to one latest pending query.

The release validation build also passed the live Processes refresh regression:
100 baseline rows, 10 observations over three refresh cycles, no collapse,
identity-churn, invalid-geometry or overlap failures, and graceful shutdown.
Evidence on this host:
`C:\Temp\wfdiag-perf-after-cSrARF\process-refresh-rerun\process-refresh-20260907-223205.json`.

`scripts/test-reactor-performance.ps1` records foreground and minimized samples
and rejects runs that lose foreground ownership. Initial before/after trials on
ANDROMEDA were interrupted by other foreground applications. A before-build
Monitor run completed, but there is no matched valid after run; **no CPU/RAM
reduction percentage or Tauri comparison is claimed**. Repeat on an undisturbed
desktop with identical release features and representative long-running work:

```powershell
.\scripts\test-reactor-performance.ps1 -Executable C:\before\wfdiag.exe -OutputDirectory C:\perf\before
.\scripts\test-reactor-performance.ps1 -Executable C:\after\wfdiag.exe -OutputDirectory C:\perf\after
```

The broader `measure-reactor-resources.ps1` remains the process-tree/Quick Scan
benchmark. All-day telemetry/chat soak tests, clean-machine x64/ARM64 resource
comparisons, and Store/hardware readiness evidence remain release validation
work. No readiness gate was weakened, and no commit, push, or release was made.
