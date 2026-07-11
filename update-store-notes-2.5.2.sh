#!/usr/bin/env bash
# Fallback / manual trigger: push the 2.5.2 "What's new" text to the Store
# listing. Run AFTER 2.5.2 is live (the workflow fails cleanly if a
# submission is still in certification).
set -euo pipefail

gh workflow run update-store-release-notes.yml -f release_notes="NEW: Use your existing AI subscription — no API key needed.

• Sign in with ChatGPT: run the AI assistant on your ChatGPT Plus/Pro plan through OpenAI's Codex CLI.
• Sign in with Claude: use your Claude Pro/Max plan through Anthropic's Claude Code CLI, with streaming responses.

Your sign-in stays with the vendor's own tools — this app never stores tokens or API keys for these providers, and requests run in a locked-down, read-only context.

Also new:
• Model pickers for every AI provider, with live model lists fetched from OpenAI, Anthropic, Gemini, DeepSeek, custom endpoints, Foundry Local, and Ollama.
• Model overrides for OpenAI and Foundry Local.
• Smarter Auto routing: local AI first, then your subscription, then metered API keys.
• Clearer error messages and sign-in detection for the new providers."

echo "Dispatched. Watch it with: gh run list --workflow=update-store-release-notes.yml"
