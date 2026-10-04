# ChatGPT plan access

WindowsForum Diagnostics supports native **Continue with ChatGPT** alongside the
Codex CLI bridge and OpenAI API-key provider. In **AI Assistant**, choose
**Continue with ChatGPT**, or open **Settings → Set up provider → ChatGPT plan**.
Your default browser handles authorization. No CLI installation or API key is needed.

After connecting, choose the account and model in Settings. **Add account** keeps
another registration separate, including workspaces sharing the same email.
**Enable ChatGPT plan usage** requests consent when identity sign-in succeeded
without plan permission. Connecting keeps your preferred provider unchanged;
choose **ChatGPT plan** as the preferred provider to use it explicitly.

Auto tries Aion/Phi, Foundry Local and Ollama before ChatGPT, then other cloud
providers. A connected ChatGPT account remains the cloud boundary when paused,
restricted or awaiting reconnect. A failed ChatGPT request never automatically
switches to an API key, Codex CLI or another billing provider. Select another
provider explicitly or sign out to change that choice. Manage allowance and app
limits through **Manage usage**. WFDiag does not infer a reset time from a quota error.

## Implementation

`wfdiag-native-chatgpt` owns authorization, account storage, discovery and renewal.
The shared instance reaches provider status, model discovery, chat, reports,
analysis, prioritization and fix plans through `wfdiag-app` ports. Its UI projection
contains account labels, models and sanitized recovery metadata, never credentials.
Windows stores the versioned document in current-user DPAPI at
`%LOCALAPPDATA%\WFDiag\chatgpt-accounts.dpapi`, separately from `settings.json` and
CLI credential caches. A sibling file lock serializes renewal across processes;
rotated tokens are saved atomically before inference. Generation cancellation does
not interrupt a renewal transaction already in progress. Headless tests inject
memory storage; production has no plaintext disk fallback.

Authorization uses a loopback listener bound to `127.0.0.1`, PKCE S256, random state
and nonce, and signature/issuer/audience/expiry/nonce verification of the ID token.
The host ID is persisted before browser launch. Issued client IDs survive interrupted
exchange, reconnect and sign-out. Unvalidated registration attempts stay separate
from the active account. Sign-out clears local tokens and attempts bounded remote
revocation; an unconfirmed revocation is reported to the user.

The native transport uses stateless streamed `/v1/responses` requests. Only a
completed response may enter a cache or run an advertised diagnostic tool. Private
continuation items preserve encrypted reasoning and assistant phase. Account or
model changes cancel generations, clear conversation and derived projections, and
invalidate caches. Existing remediation authorization remains in the action broker.

## Verification

Portable fixtures cover loopback callbacks, PKCE, signed identity validation,
missing consent, multiple registrations, model ordering, rotating refresh races,
failed saves, cancellation, quota after streaming starts and incomplete responses.
Facade tests cover account commands and context invalidation. The committed test
RSA key is synthetic and used only to sign fixture identity tokens.

Before release, validate browser authorization, account switching, DPAPI persistence,
refresh, sign-out and all AI workflows on packaged Windows x64 and ARM64 installations.
Cross-compilation cannot establish browser/Store/DPAPI runtime behavior. The existing
hardware readiness gates remain unchanged.

Protocol references: [Registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in),
[Accounts and sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions),
[Models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference),
[Errors and recovery](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery).
