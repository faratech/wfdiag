# WindowsForum Diagnostics 2.5.2: Use Your ChatGPT or Claude Subscription — No API Key Required

WindowsForum Diagnostics 2.5.2 is now rolling out on the Microsoft Store. It is a focused update with one headline feature: the AI assistant can now run on the AI subscription you already pay for. If you have ChatGPT Plus/Pro or Claude Pro/Max, you can sign in once and use the assistant without creating an API account, buying credits, or pasting a key.

---

## The Short Version

- **Sign in with ChatGPT.** A new provider runs requests through OpenAI's Codex CLI on your machine. Usage bills to your existing ChatGPT plan — no OpenAI API key needed.
- **Sign in with Claude.** A second new provider does the same for Anthropic, driving the installed Claude Code CLI over the Agent Client Protocol — the same integration approach Microsoft's own Intelligent Terminal uses. Responses stream in live.
- **Model pickers everywhere.** Every provider in Settings now has a model dropdown. Cloud providers fetch their live model list from the service the moment a key is present; local servers report what's installed; the subscription providers offer a curated catalog.
- **Smarter Auto routing.** Auto mode stays local-first, but a signed-in subscription now takes priority over metered API keys — so you never burn pay-per-token credits while a flat-rate plan is sitting there.

---

## How the Subscription Providers Work — and Why You Can Trust Them

The design principle is simple: **this app never touches your account credentials.**

WindowsForum Diagnostics does not implement OAuth, does not store tokens, and never reads the vendors' credential files. Instead, it detects the official CLI you already installed — OpenAI's Codex CLI or Anthropic's Claude Code — and lets that CLI own the entire sign-in. Click **Sign in with ChatGPT** or **Sign in with Claude** in Settings and the vendor's own login opens in your browser, exactly as if you had run the CLI in a terminal. The app only ever sees "signed in" or "not signed in."

Requests follow the same rule. Prompts are handed to the vendor's tooling over standard input — never through a shell command line — and the runs are locked down:

- Codex executes in a read-only sandbox inside an empty working directory. It cannot read your files or change anything.
- Claude Code runs through the ACP adapter with all tool permissions rejected — it answers from the diagnostic context the app provides, nothing more.
- API-key environment variables are stripped from these runs, so a stray `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` on your system can never silently flip your usage from subscription billing to per-token API billing.

On the compliance side: OpenAI has publicly endorsed third-party use of ChatGPT subscriptions through Codex, and the Claude integration mirrors Microsoft Intelligent Terminal's shipped approach — the genuine CLI does the work under your own login. Nothing is extracted, spoofed, or proxied.

### Getting started

| Provider | You need | Setup |
| --- | --- | --- |
| ChatGPT via Codex CLI | ChatGPT Plus/Pro/Team + `npm install -g @openai/codex` | Settings → Provider setup → ChatGPT via Codex CLI → Sign in with ChatGPT |
| Claude via Claude Code | Claude Pro/Max + `npm install -g @anthropic-ai/claude-code` | Settings → Provider setup → Claude via Claude Code CLI → Sign in with Claude |

The app auto-detects both CLIs (npm and native installs), shows install hints when they're missing, and offers a path override if you keep tools somewhere unusual. Claude responses stream token-by-token; the first Claude request fetches a small adapter package, so give it an extra moment once.

---

## Model Selection, Everywhere

Until now most providers ran on a fixed or hand-typed model name. In 2.5.2, every provider pane in Settings has a model dropdown:

- **OpenAI, Anthropic, Gemini, DeepSeek** — the live model list is fetched from your account the moment a key is entered (even before you press Save), filtered to chat-capable models.
- **Custom endpoints** (OpenRouter, Groq, proxies) — the endpoint's `/v1/models` list populates the picker.
- **Foundry Local and Ollama** — the dropdown shows exactly the models your local server reports.
- **Codex and Claude Code** — a curated catalog of current models, with "CLI default" as the sensible empty choice.

OpenAI and Foundry Local also gain model overrides for the first time — you're no longer pinned to the app defaults.

---

## Availability

Version 2.5.2 is submitted to the Microsoft Store and will appear as an automatic update once certification completes. The Store build remains the recommended install — it's signed by Microsoft and is the only build with on-device Phi Silica support on Copilot+ PCs.

- **Microsoft Store:** https://apps.microsoft.com/detail/9nj59rh053pv
- **Portable builds and release notes:** https://github.com/faratech/wfdiag/releases

As always: the assistant remains read-only by design. It can inspect, explain, and recommend — repairs still require your explicit confirmation, and that isn't changing.

Questions, feedback, and bug reports are welcome in this thread.
