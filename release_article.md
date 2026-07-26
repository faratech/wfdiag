# WindowsForum Diagnostics 2.5.0: AI That Can Inspect Your PC, Not Just Explain It

WindowsForum Diagnostics 2.5.0 is the biggest intelligence update yet for the WindowsForum system diagnostic app. The headline is not simply that the app now supports more AI providers. The important change is that the assistant can work from real diagnostic evidence: it can inspect the current scan, run safe read-only checks when the active model supports tools, compare recent scans, surface detected issues, and stream its reasoning back while it works.

That changes the role of the app. Earlier versions were already useful for collecting Windows system facts, monitoring resources, exporting reports, and identifying common problems. Version 2.5.0 moves closer to an interactive troubleshooting workspace: scan the PC, ask a question, watch the assistant gather data, then decide what to do from a safer issue and remediation catalog.

The update also broadens model choice. Depending on your hardware and preferences, WindowsForum Diagnostics can use on-device Phi Silica, Microsoft Foundry Local, Ollama, custom OpenAI-compatible endpoints, OpenAI, Anthropic Claude, Google Gemini, or DeepSeek. Auto mode remains local-first, while explicit provider choices stay explicit.

---

## The Short Version

WindowsForum Diagnostics 2.5.0 adds four big pieces:

- A streaming AI assistant that can run read-only diagnostic tools, show live activity, and answer from current PC data.
- A broader provider system covering on-device, local-server, custom endpoint, and cloud AI options.
- A one-click AI scan report that turns a completed scan into a short health summary with top issues and next steps.
- A rebuilt issue and remediation system with a fixed catalog of detected problems, vetted actions, maintenance tools, and backend-enforced confirmation for repair commands.

The practical result is simple: the app is better at moving from "here is a wall of diagnostic output" to "here is what matters, here is what changed, and here are the safe next actions."

---

## From Static Scan Results To Interactive Troubleshooting

Most Windows diagnostic tools stop at collection. They tell you the OS version, services, drivers, event logs, disks, updates, startup entries, performance counters, and hardware state, then leave interpretation to the user. That is useful for power users and support volunteers, but it still creates a gap: the data is there, yet the next question is usually "what does this mean?"

WindowsForum Diagnostics 2.5.0 narrows that gap. The AI screen now includes a persistent assistant conversation, streaming responses, a Stop button, suggested prompts, visible provider status, and tool activity chips. When the selected provider supports tool use, the assistant can call read-only diagnostics itself. It can run a specific task, summarize the current scan, list detected issues, compare with the previous stored scan, read live monitor stats, list available remediations, and inspect recent scan history.

That matters because troubleshooting is rarely linear. A user might ask, "Why is this PC slow after the last update?" The useful answer may require performance data, startup entries, driver age, Windows Update state, event logs, and scan drift. A conventional report can contain all of that, but it does not choose the next check. The 2.5.0 assistant can.

The interface also makes the work visible. Tool activity chips show what the assistant ran and whether each check completed or failed. That is a better model for trust than a chatbot that simply produces a confident paragraph from hidden context. If the answer says disk space is low or a service is stopped, the user can see that the assistant actually consulted the diagnostic surface behind that claim.

---

## The Assistant Has Guardrails

The most important design detail in 2.5.0 is what the assistant cannot do.

The chat tool registry is read-only. It does not expose `fix_issue`, `run_remediation`, restart commands, tag changes, or any other mutation. Chat-triggered diagnostics are also kept out of the saved scan session so a conversational check does not silently rewrite scan provenance. In other words, the assistant can help inspect and explain, but it cannot repair the machine behind the user's back.

The backend also bounds the tool loop. It limits tool iterations, caps tool calls per turn, applies per-tool timeouts, limits concurrency, trims context to provider budgets, and forces a final answer when tool budgets are exhausted. These are not glamorous release-note items, but they matter. Diagnostics can involve WMI, event logs, device queries, and other slow Windows surfaces. A useful AI assistant inside a diagnostic app needs to be able to stop, stream, and fail predictably.

This is the right line for a Windows repair tool. Diagnosis can be conversational. Repair should remain explicit.

---

## A Larger AI Provider Roster

Version 2.5.0 also turns the AI layer into a provider system instead of a single cloud integration with a local fallback. The Settings dialog now separates "Active AI" from "Provider setup," gives each provider its own configuration fields, and stores keys through the operating system secret store rather than writing them into `settings.json`.

The current provider picker covers:

| Provider | Where it runs | Notes |
| --- | --- | --- |
| Phi Silica | On-device NPU through Windows AI APIs | Store build only, for supported Windows AI hardware and package identity scenarios. |
| Foundry Local | Local Microsoft AI service | Free local option. The app discovers the dynamic local endpoint instead of hardcoding a port. |
| Ollama | Local server | Auto-detects the default endpoint and can use the first installed model when no model is configured. |
| Custom OpenAI-compatible | Local or hosted endpoint | Useful for OpenRouter, Groq, proxies, and other `/v1/chat/completions` services. |
| OpenAI | Cloud | Existing cloud option, now part of the shared provider layer. |
| Anthropic Claude | Cloud | Native Messages API client with streaming support. |
| Google Gemini | Cloud | Native `generateContent` client with provider-specific tool handling. |
| DeepSeek | Cloud | OpenAI-compatible cloud option with its own key and model settings. |

Auto mode is local-first. It prefers Phi Silica when available, then Foundry Local, then Ollama, then a configured custom endpoint, then cloud providers. That order keeps the privacy-friendly and no-key options at the front while preserving the cloud route for users who want larger models or already have an API key.

Explicit provider selection is intentionally stricter. If a user chooses Claude, Gemini, Ollama, DeepSeek, or another specific provider and it is unavailable, the app does not silently fall back to something else. That is a sensible choice: when diagnostic data may be sent to a cloud model, "try another provider" should not be a hidden behavior.

---

## Local AI Gets Clearer Boundaries

Phi Silica remains the most privacy-preserving option because generation happens through Windows on supported hardware. But 2.5.0 is careful about availability. The app treats Phi Silica as a Microsoft Store/MSIX path with package identity and the `systemAIModels` capability. Loose portable builds do not try to fake identity or bypass the Windows AI access model.

That may sound like an implementation detail, but it prevents a frustrating user experience. If Phi Silica is not available, the app can say why and point users toward alternatives such as Foundry Local or Ollama instead of presenting an on-device AI button that fails with obscure COM or access-denied errors.

Foundry Local and Ollama fill the gap for users who want local AI on ordinary PCs. Foundry Local is discovered through its service status or a configured endpoint, while Ollama uses its local API and installed model list. Users who want cloud-scale reasoning can still configure OpenAI, Claude, Gemini, DeepSeek, or an OpenAI-compatible provider.

The result is not "one AI feature." It is a routing layer that lets the same diagnostic app fit different privacy, hardware, and budget preferences.

---

## One-Click Scan Reports

The new scan report feature is the release's clearest quality-of-life improvement. After a scan completes, the AI screen can generate a compact health report from the actual scan data. The report is designed to answer the questions users usually ask first:

- What is the overall health verdict?
- Which detected issues matter most?
- What changed compared with the previous scan?
- What should I do first?

The report path is deterministic about the data it assembles. It prioritizes headline scan counts, failures, detected issues, comparison data, and compact high-value diagnostic outputs. It streams through `ai-report://` events when generated live and uses a scan-content cache when the same report has already been produced.

This is especially useful for forum support. A full diagnostic export can be too long for a quick thread. A one-click report gives the user a concise summary they can paste into a post, then keep the deeper export available when someone asks for the underlying evidence.

---

## Issue Detection Is Now A Catalog, Not A Grab Bag

The Issues screen also received a major backend rewrite. Instead of scattered fix logic, 2.5.0 introduces a central issue catalog with 28 issue specifications. Each issue defines its category, severity, display text, recommendation, source diagnostic tasks, optional remediation mapping, and deterministic detector function.

The catalog covers the problems Windows users actually bring to support threads:

- Storage: low disk space, fragmentation, SMART failure warnings, unhealthy disks, disk I/O events.
- Security: firewall disabled, antivirus disabled, suspicious hosts-file redirection.
- System health: pending updates, component store corruption, pending reboot, unexpected shutdowns, Kernel-Power events.
- Debug and hardware: recent blue screen dumps, WHEA events, Device Manager problem codes, battery degradation.
- Performance: high CPU, high memory, page file pressure, too many startup programs, excessive temporary files.
- Services, drivers, and network: stopped automatic services, service crash loops, unsigned or stale drivers, missing DNS configuration.

The important part is that detection is deterministic. The code injects time and temporary-file counts into the detection context instead of letting detectors read those values ad hoc. That makes the engine easier to test and harder to surprise.

Quick Scan also now surfaces the newer detectors, so the faster scan path is not blind to the expanded issue model.

---

## Fixes Are Vetted And Tiered

The old `issue_fixer` path has been replaced by a remediation engine. That engine exposes a fixed catalog of actions, and every action is a compile-time constant. User text and AI output do not become command arguments.

Remediations are split into three tiers:

| Tier | What it does | Examples |
| --- | --- | --- |
| OpenTool | Opens a Windows tool for the user to inspect or act | Task Manager, Windows Update, Device Manager, Disk Cleanup, Optimize Drives, Windows Security. |
| AutoSafe | Runs narrow, non-destructive maintenance actions | Flush DNS, clean temp files, rebuild icon cache, empty Recycle Bin, start core services. |
| Repair | Runs system-altering or admin-level repair actions | DISM RestoreHealth, SFC, Windows Update reset, network stack reset, scheduled restart. |

The Repair tier is confirmation-gated in the backend, not merely in the UI. If the frontend asks to run a Repair action without `confirmed: true`, the backend returns a confirmation-required outcome before any command executes. The confirm modal then shows the exact action and risk class.

This is also where the AI fix-plan work fits. The model can propose a plan by referencing catalog IDs for detected issues, but it cannot invent a command and it cannot execute it. The parsed plan is validated against the remediation catalog and the current detected issues. The user still clicks Run, and the normal confirmation flow still applies.

That is the right design for a community diagnostic tool. AI can help prioritize. The app owns the allowed actions. The user owns the decision.

---

## Better Output, Better Settings, Better Streaming

Several smaller changes make 2.5.0 feel more complete in day-to-day use.

AI output now supports GitHub-flavored Markdown tables in the app's safe lightweight renderer, so model responses do not turn structured comparisons into mangled text. The renderer remains deliberately small and avoids arbitrary HTML handling, which is the right tradeoff for diagnostic output that may contain filenames, log fragments, and other untrusted strings.

The Settings dialog has been redesigned around provider-specific setup. OpenAI, Claude, Gemini, DeepSeek, custom endpoints, Ollama, Foundry Local, and Phi Silica each get the fields they need. Ollama can populate a model dropdown when the server is available. Custom endpoints expose endpoint, model, and optional key fields. Cloud providers expose key and model choices.

The AI status surface now refreshes provider availability more reliably, and fixes landed for stream races and provider refresh bugs. That matters because the AI screen is event-driven: the backend sends deltas, tool events, done events, and errors while the frontend batches updates to avoid overwhelming React.

Export handling also received fixes and hardening. Save paths are validated against allowed locations, HTML output is escaped, and diagnostics export issues found during the 2.5.0 bug bash were fixed before the release tag.

---

## Monitoring And Hardware Detail Improve Too

Although the AI work is the headline, 2.5.0 also adds a new Windows adapter monitoring path. The backend now includes D3DKMT-based GPU/NPU adapter sampling, including utilization and memory metrics where Windows exposes them. That gives the live monitor and process views a better foundation for modern Windows PCs, especially systems with discrete GPUs or neural processors.

The broader monitoring code also received race and responsiveness fixes across the release window. For a diagnostic app, that matters: live charts should help explain system behavior, not become another source of lag.

---

## Under The Hood: A More Maintainable App

The 2.5.0 release is large because it is not just a feature patch. It reorganizes major backend and frontend surfaces:

- The old OpenAI-specific integration was folded into a provider layer.
- Native clients were added for Anthropic and Gemini.
- OpenAI-compatible chat completion support now covers providers that do not implement OpenAI's Responses API.
- Per-provider context budgets keep local and cloud prompts within practical limits.
- Backend chat sessions keep provider-shaped tool messages away from the UI.
- `Cargo.lock` is now committed for reproducible Rust builds.
- The app moved forward to newer Tauri, Vite, TypeScript, ESLint, Vitest, and dependency versions.
- CI now runs checks on pull requests to any base branch.
- Dependency updates addressed Vite/esbuild and Undici security alerts.
- ESLint warnings were removed through real code fixes rather than suppression.

Those details are not as visible as the new assistant, but they make the app easier to ship and easier to trust.

---

## What This Means For WindowsForum Users

For everyday users, the biggest change is that WindowsForum Diagnostics can now help interpret a machine without requiring the user to already know which subsystem matters. You can run a scan, ask what failed, ask whether there are security concerns, ask why the system feels slow, or generate a short report for a support thread.

For enthusiasts and support volunteers, the value is evidence. The assistant can summarize, but the app still exposes the scan data, issue list, comparison view, exports, and remediation details. A helper can ask for the report first, then drill into raw output only when needed.

For administrators and power users, the provider model is the key feature. Local-first routing allows a privacy-preserving default. Cloud providers remain available when stronger reasoning or larger context is worth the tradeoff. Custom endpoints make it possible to route through an internal model gateway or preferred hosted provider.

For Copilot+ PC owners, the Store build remains the route for Phi Silica. For everyone else, Foundry Local and Ollama are the practical local options.

---

## Requirements And Availability

WindowsForum Diagnostics remains a Windows desktop app built with Tauri and React, with a Rust backend for native diagnostics.

For the best experience:

- Use Windows 11 for full functionality.
- Use the Microsoft Store/MSIX build on a supported Copilot+ PC for Phi Silica on-device AI.
- Install Foundry Local or Ollama for local AI on non-Copilot+ systems.
- Add an API key in Settings for OpenAI, Anthropic Claude, Gemini, DeepSeek, or a custom cloud endpoint.
- Run as administrator when you want the fullest diagnostic coverage or when a repair-tier remediation requires elevation.

The app continues to support the core diagnostic workflow without AI. AI adds interpretation, tool-guided investigation, and reports; it is not required to collect diagnostics.

---

## Bottom Line

WindowsForum Diagnostics 2.5.0 is a substantial release because it treats AI as part of the diagnostic workflow instead of a decorative summary box. The assistant can inspect, stream, compare, and explain. The provider layer respects local-first privacy choices. The issue engine is broader and more deterministic. The remediation system is safer because allowed actions are cataloged, tiered, and confirmation-gated in the backend.

That combination is exactly where a Windows troubleshooting app should be heading: collect real evidence, explain it clearly, make safe actions obvious, and keep the final repair decision in the user's hands.

---

## References

1. [WindowsForum News section](https://windowsforum.com/forums/windows-news.4/)
2. [WindowsForum Diagnostics GitHub releases](https://github.com/faratech/wfdiag/releases)
3. [Microsoft Learn: Build a chat app with Phi Silica and WinUI 3](https://learn.microsoft.com/en-us/windows/ai/apis/phi-silica-winui-tutorial)
4. [Microsoft Learn: Foundry Local CLI reference](https://learn.microsoft.com/en-us/azure/foundry-local/reference/reference-cli)
5. [Microsoft Learn: Get started building an app with Windows AI APIs](https://learn.microsoft.com/en-us/windows/ai/apis/get-started)
