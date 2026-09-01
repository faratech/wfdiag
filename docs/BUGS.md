# Bugs

Bugs are tracked in **GitHub Issues**, not in this file.

The 2026-09-01 pre-release audit (engine/GUI separation + Reactor cutover) is tracked by
[**issue #232**](https://github.com/faratech/wfdiag/issues/232) —
*"Reactor audit tracking (2026-09-01): 12 high / 19 medium / 13 low / 5 release-plumbing
findings"* — with 48 child issues, **#184–#231**.

## Convention

| Field | Rule |
| --- | --- |
| Title | `Reactor audit <date> <id>: <one-line summary>` — e.g. `Reactor audit 2026-09-01 H2: …`. The `<id>` is the audit's own identifier (`H`igh / `M`edium / `L`ow / `R`elease-plumbing + number). |
| Labels | `bug` plus exactly one priority label: `priority: high`, `priority: medium`, or `priority: low`. |
| Closing | Close with the fixing **commit hash**, so the issue points at the change that resolved it. Reference it from the commit too (`Closes #NNN`). |
| Body | Location (file + what is wrong) and Fix (what the change should do). Keep it short enough to act on without re-deriving the analysis. |

New findings that are not part of an audit sweep get a plain descriptive title and the same
`bug` + priority labels.

## Historical note

This file previously held a static 2025 findings list covering the Tauri backend and the
React frontend. Every item on it was resolved (mostly in PR #7 and follow-ups) and the file
had not been maintained since; the content was removed on 2026-09-01 rather than left to rot
beside the live tracker. Recover it from git history if you need it:

```bash
git log --follow -p -- docs/BUGS.md
```
