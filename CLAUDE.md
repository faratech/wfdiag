# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
- **Run development server**: `npm run tauri dev` - Starts both Vite dev server and Tauri in development mode
- **Build for production**: `npm run tauri build` - Creates optimized production build with exe, MSI, and NSIS installers
- **Build sideloadable Store/Phi test MSIX**: `python3 scripts/build-cross.py build-all --build-msix --sign` - Creates a locally signed Store-manifest bundle with `systemAIModels`; production Store uploads omit `--sign` and Microsoft signs the distributed package
- **Build basic Tauri MSIX**: `npm run tauri build -- --bundles msix` - Creates a basic MSIX using `src-tauri/tauri.msix.conf.json`; not the Phi Silica Store package
- **Build MSI installer only**: `npm run tauri build -- --bundles msi` - Creates MSI for direct distribution
- **Build NSIS installer only**: `npm run tauri build -- --bundles nsis` - Creates NSIS installer
- **Build exe only**: `cd src-tauri && cargo build --release` - Builds just the exe without installers
- **Frontend only dev**: `npm run dev` - Runs Vite dev server without Tauri (limited functionality)
- **Type checking**: `npx tsc --noEmit` - Run TypeScript compiler to check for type errors

### ARM64 Windows Support
This application supports both x64 and ARM64 Windows architectures with native builds.

**Prerequisites for ARM64 builds:**
1. Install ARM64 build tools from Visual Studio Installer:
   - Open Visual Studio Installer
   - Select "Individual Components"
   - Install "MSVC v143 - VS 2022 C++ ARM64 build tools"
2. Add Rust ARM64 target: `rustup target add aarch64-pc-windows-msvc`

**ARM64 Build Commands:**
- **Build ARM64 exe**: `cd src-tauri && cargo build --release --target aarch64-pc-windows-msvc`
- **Build ARM64 installer**: `npm run tauri build -- --target aarch64-pc-windows-msvc`
- **Check ARM64 compilation**: `cd src-tauri && cargo check --target aarch64-pc-windows-msvc`

**Architecture Detection:**
- The app automatically detects both process and native architecture at runtime
- Uses `IsWow64Process2` API to detect x64 emulation on ARM64 hardware
- Reports architecture info via `get_architecture_info()` Tauri command
- Architecture module in `src-tauri/src/architecture.rs` provides helper functions

### Version Management
Automated script to bump version numbers across all project files:

- **Bump version**: `python3 scripts/bump-version.py 2.3.0` - Updates version in all 10 project files
- **Dry run**: `python3 scripts/bump-version.py 2.3.0 --dry-run` - Preview changes without modifying files

**Files automatically updated:**
- version.json, package.json, package-lock.json (NPM)
- src-tauri/Cargo.toml, src-tauri/tauri.conf.json, src-tauri/tauri.msix.conf.json (Rust/Tauri)
- AppxManifest.xml (Windows Store - gets .0 suffix)
- src/App.tsx, src/components/AboutDialog.tsx, README.md (Frontend/docs)

### CI/CD Releases
The main release workflow is `build-and-publish-store.yml`:
- **Trigger**: Push any version tag (e.g., `git tag v2.1.7 && git push origin v2.1.7`)
- **Actions**: Builds x64/ARM64, creates MSIX bundle, attestations, GitHub Release, and publishes to Microsoft Store
- **Store publishing**: Uses `microsoft/setup-msstore-cli` with `msstore publish` command
- **Manual dispatch**: Available for testing without creating tags

### Package Signing (Windows)
- **Build MSIX package**: `.\build-msix.ps1` - PowerShell script to build MSIX for Store
- **Sign MSIX package**: `.\sign-msix.ps1` - Sign MSIX with certificate
- **Sign MSI installer**: `.\sign-msi.ps1` - Sign MSI installer package
- **Sign exe**: `.\sign-exe.ps1` - Sign the executable with certificate
- **Create certificate**: `.\create-cert.ps1` - Create self-signed certificate for testing

### Rust Backend
- **Check Rust code**: `cd src-tauri && cargo check` - Quick syntax and type checking
- **Format Rust code**: `cd src-tauri && cargo fmt` - Format code according to Rust standards
- **Run Rust lints**: `cd src-tauri && cargo clippy` - Run Rust linter for code quality
- **Update dependencies**: `cd src-tauri && cargo update` - Update Rust dependencies to latest compatible versions

## Architecture

This is a Tauri v2 application with a clear separation between frontend and backend:

### Frontend (src/)
- **React 19** with TypeScript for UI
- **Custom CSS design system** in `src/styles/colors_and_type.css` (tokens) + `src/styles/app.css` (components) — Windows 11 Fluent/Mica aesthetic, light/dark themes. No component library; Font Awesome for icons.
- **Tauri v2 API** for IPC communication with backend
- `App.tsx` is a thin shell (nav rail, command bar, custom titlebar, status bar); each screen lives in `src/screens/*.tsx` (Diagnostics, Monitor, Processes, AI, Issues, History)
- Shared state in `src/contexts/` (AppContext, AIContext, ThemeContext, ToastContext); behavior in `src/hooks/` (useScanner, useDiagnostics, useMonitoring, useAI, useCommands, useGlobalShortcuts, …)
- Reusable primitives in `src/components/ui/` (Modal, Button, Tooltip, Skeleton, EmptyState, Kbd) plus CommandPalette, ShortcutHelp, Titlebar in `src/components/`
- Uses Vite for fast development and optimized production builds; Vitest + Testing Library for unit tests (`npm test`), ESLint flat config (`npm run lint`)

### Backend (src-tauri/)
- **Tauri v2** with lib.rs/main_tauri.rs structure
- **Pure Rust** implementation for system diagnostics
- **lib.rs**: Main application logic and Tauri command handlers
- **main_tauri.rs**: Entry point that calls the run function from lib.rs
- **commands/**: Command modules (export.rs, settings.rs)
- **diagnostics.rs**: Core diagnostic task definitions and execution logic
- **native_diagnostics.rs**: Windows-specific diagnostic implementations
- **wmi_native.rs**: Native WMI wrapper using Windows COM APIs (IWbemLocator, IWbemServices)
- **windows_native.rs**: Direct Windows API bindings and wrappers
- **architecture.rs**: CPU architecture detection (x64, ARM64) and emulation detection
- **native_monitor.rs**: Real-time system monitoring with CPU, memory, disk, network and NPU stats
- **ai_service.rs / ai_cache.rs / ai_prompts.rs**: Unified AI layer (provider routing, response cache, budget-aware prompts)
- **ai_providers/**: One client module per provider (openai, anthropic, gemini, openai_compat, ollama, foundry, phi) + `capabilities()` table, `resolve_config()`, shared discovery/SSE helpers; **phi_silica.rs**: on-device Phi Silica WinRT
- **ai_chat.rs / ai_tools.rs**: Agentic chat (backend session store, streaming tool loop) and its READ-ONLY tool registry
- **ai_report.rs**: One-click AI scan health report (deterministic context assembly, no tool loop)
- **issue_catalog.rs / issue_detector.rs**: Issue engine — `issue_catalog.rs` is the single
  registry (~28 `IssueSpec`s: metadata + remediation mapping; invariants enforced by tests);
  `issue_detector.rs` holds the pure detect fns (deterministic: clock + temp-file count are
  injected via `DetectCtx`, never read inside detectors)
- **remediation.rs**: Tiered remediation catalog (OpenTool | AutoSafe | Repair). Every command
  is a compile-time constant. The Repair tier REQUIRES `confirmed: true` — the gate lives in
  `remediation::execute`, not the UI. Commands run through the injectable `CommandRunner`
  (tests use a recorder; `RealRunner` adds CREATE_NO_WINDOW + timeouts). `maintenance: true`
  entries appear in the Issues screen's always-available Maintenance list
- **ai_fix_plan.rs**: AI-proposed fix plans — the model only ever emits catalog IDs, which
  `parse_fix_plan` validates against the remediation catalog and detected issues; execution is
  always user-initiated through the normal confirm flow
- **results_storage.rs**: Scan results storage, comparison, tags, failure trends
- **state.rs**: AppState (current session, monitor, scan storage, cancelled sessions)
- **tray.rs**: System tray (Show/Hide, Quick Scan, Exit) and close-to-tray handling
- **update_check.rs**: GitHub-release update check (silent for Store installs via SignatureKind)
- **sparse_identity.rs**: `has_package_identity()` — Phi Silica availability gate
- **windows_ai_bindings.rs**: GENERATED by windows-bindgen — never hand-edit

### Key Dependencies
- Tauri: v2.11 (Tauri framework; plugins: dialog, clipboard-manager, notification, single-instance)
- windows: v0.62 (Windows API bindings with native WMI support via Win32::System::Wmi)
- winreg: v0.55 (Windows Registry access)
- async-openai: v0.41 (features: native-tls, responses, chat-completion — the chat-completion
  feature is REQUIRED for the generic OpenAI-compatible providers; OpenRouter/Ollama/etc. do
  not serve /v1/responses)
- reqwest v0.13 + eventsource-stream (native Anthropic/Gemini clients with SSE streaming)
- tokio: v1.52 (Async runtime); tokio-util (CancellationToken for stoppable chat turns)

### Key Architectural Decisions
1. **Tauri v2 Migration**: Uses the Tauri v2 plugin architecture for the native save dialog, clipboard writes, notifications, and single-instance handling
2. **IPC Bridge**: All system access goes through Tauri commands - frontend never directly accesses system
3. **Async Operations**: All diagnostic tasks run asynchronously using Tokio
4. **Task Batching**: Diagnostics run in batches of 5 to prevent overwhelming the system
5. **Error Handling**: Comprehensive error handling with fallbacks for each diagnostic
6. **State Management**: Frontend uses React hooks, backend uses Rust's ownership model
7. **Real-time Monitoring**: Uses Tauri events to stream system stats from backend to frontend

## Tauri v2 Specific Configuration

### Capabilities (src-tauri/capabilities/default.json)
Defines permissions for:
- Core window operations
- Native save dialog
- Clipboard writes
- Desktop notifications

### Plugin Usage
The app uses these Tauri v2 plugins:
- `tauri-plugin-dialog`: Native dialogs
- `tauri-plugin-clipboard-manager`: Clipboard access
- `tauri-plugin-notification`: Scan-completion notifications
- `tauri-plugin-single-instance`: Focus the existing window on a second launch

## Diagnostic Task Categories

The application organizes diagnostics into these categories:
- **System**: OS, BIOS, boot configuration, environment variables
- **Hardware**: CPU, RAM, motherboard, TPM, devices
- **Storage**: Disks, partitions, volumes, free space analysis
- **Network**: Adapters, IP configuration, routing tables
- **Drivers**: System drivers, versions, digital signatures
- **Software**: Installed programs, Windows services, features
- **Logs**: Event logs, Windows Update history, reliability
- **Debug**: BSOD minidumps, crash analysis, system files
- **Performance**: Uptime, performance counters, resource usage

## Important Implementation Details

### Tauri Commands
All backend functionality is exposed through Tauri commands registered in `lib.rs` (see the
`invoke_handler` list there for the authoritative set). Key ones:
- `get_system_info` / `get_architecture_info`: System information, admin status, CPU architecture
- `get_available_tasks`: List of all diagnostic tasks
- `start_diagnostics`: Begin a new diagnostic session
- `cancel_diagnostics`: Cancel a running session (task-granular: in-flight tasks finish, queued tasks skip)
- `run_diagnostic_task` / `run_diagnostics_parallel`: Execute diagnostics (parallel is the main path)
- `get_session_results`: Retrieve results from current session
- `commands::export::export_results` / `save_results_to_file`: Export in JSON/Text/HTML, path-validated saves
- `get_uptime`, `restart_as_admin`, `fix_issue`, `get_fixable_issue_ids`
- `start_monitoring` / `stop_monitoring` / `get_current_stats` / `get_network_connections`
- `save_current_scan`, `list_scan_history`, `load_scan`, `compare_scans`, `clear_scan_history`,
  `update_scan_tags`, `get_task_trends`
- `check_for_update`: GitHub-release update check (no-op for Store installs and debug builds)
- `commands::settings::*`: load/save settings; per-provider API key storage
  (`store_provider_api_key` / `clear_provider_api_key` with provider ∈ openai, anthropic,
  gemini, custom_openai; legacy `store_api_key`/`load_api_key`/`clear_api_key` = OpenAI)
- AI one-shot: `ai_get_status` (incl. the per-provider `providers` array),
  `ai_analyze_diagnostic`, `ai_analyze_section`, `ai_explain_health`, `ai_set_preference`,
  `ai_clear_cache`, `ai_list_ollama_models`, plus `phi_silica::*`
- AI chat (streaming): `ai_chat_send` returns an ack immediately, then events
  `ai-chat://delta` (coalesced text), `ai-chat://tool` (activity), `ai-chat://done`,
  `ai-chat://error` (camelCase payloads pinned by serde tests); `ai_chat_cancel`,
  `ai_chat_new_session`, `ai_chat_get_history` (render projection for rehydration)
- AI report: `ai_generate_report` (cached → full text in the ack; else streams via
  `ai-report://delta|done|error`)

### Windows API Integration
The backend makes extensive use of Windows APIs through the `windows` crate:
- WMI queries for system information
- Registry access for software and configuration
- Native APIs for hardware enumeration
- Performance counters for system metrics
- Network adapter statistics
- Process and service enumeration

### Security Considerations
- Application requests admin privileges for full diagnostic access
- All file system access is controlled through Tauri v2 capabilities
- No arbitrary code execution from frontend
- Secure IPC bridge prevents injection attacks
- API keys for OpenAI integration are never stored in code

## Development Tips

1. **Frontend Changes**: Screens live in `src/screens/`, shared state in `src/contexts/`, behavior in `src/hooks/`, primitives in `src/components/ui/`. `App.tsx` is only the shell.
2. **Adding Diagnostics**: New diagnostics go in `diagnostics.rs` with implementation in `native_diagnostics.rs`
3. **Windows APIs**: Use existing patterns in `windows_native.rs` for new API calls
4. **Error Handling**: Always provide fallbacks - diagnostics should never crash the app
5. **Testing**: `npm test` (Vitest) for frontend units; `cargo test` runs on Windows only (CI windows-latest job). On a Linux dev box verify Rust with `PATH=/usr/lib/llvm-20/bin:$PATH cargo xwin check|clippy --target x86_64-pc-windows-msvc`. System-specific diagnostics still need manual testing on Windows.
6. **Performance**: Keep diagnostic batches small (5 tasks) to maintain UI responsiveness
7. **Tauri v2 Imports**: Use `@tauri-apps/api/core` for invoke, plugin packages for specific features
8. **Real-time Updates**: Use Tauri events (`emit` from backend, `listen` in frontend) for streaming data
9. **Monitoring Cleanup**: System monitoring automatically stops when switching tabs or when app is hidden (visibility change API)
10. **Generated Code**: `src-tauri/src/windows_ai_bindings.rs` is generated by `windows-bindgen` — never hand-edit it (changes are lost on regeneration). The `try_into().unwrap()` at ~line 2090 is upstream codegen and unreachable in practice.

## Phi Silica (On-Device AI) Integration

### Overview
Phi Silica is Microsoft's on-device AI model available on Copilot+ PCs (Windows 11 24H2+, build 26100+). It uses the `Microsoft.Windows.AI.Text.LanguageModel` WinRT API.

### Current Status: ✅ WORKING — Microsoft Store build ONLY (decided June 2026)

**Phi Silica requires registered package identity. There is no bypass.** This
was settled empirically: a bare unpackaged exe gets `0x80070005`
(E_ACCESSDENIED) from BOTH activation paths — standard WinRT activation AND
`DllGetActivationFactory` on the bundled DLLs. The DLL-bundling trick only
bypasses the activation-*factory* lookup; the API itself checks identity.
Do not re-litigate this; the failed experiment was commit 96ca754.

Consequences (the shipped architecture):
- **Store/MSIX build**: Phi Silica works (identity + `systemAIModels` + LAF token).
- **Loose/portable exe**: never attempts Phi Silica. `phi_silica.rs`
  short-circuits with "requires the Microsoft Store version" when
  `has_package_identity()` is false, and the AI service routes to
  Foundry Local → OpenAI instead.
- **Sparse identity packages** are dev-only tooling for testing the Store
  path on a loose exe (see below) — not a shipping mechanism.

### Activation (inside the Store build)

`create_language_model()` tries `DllGetActivationFactory` on the resolved AI
Text DLL first (the configuration the MSIX build has always shipped —
`RoGetActivationFactory` has historically returned E_ACCESSDENIED for
third-party apps even with identity), then falls back to standard WinRT
activation. DLL search order is framework package dirs first, then bundled
copies next to the exe (`dll_search_dirs()`).

The Microsoft-issued LAF token is bound to the full Store Package Family Name
`32827MikeFara.WindowsForumDiagnostics_t6j5qexy2jpp2` (name + publisher hash,
not just publisher). Token resolution: `WFDIAG_LAF_TOKEN` env var →
`phiSilicaLafToken` setting → built-in fallback.

### Files Involved
- **`src-tauri/src/phi_silica.rs`**: Main implementation (identity gate, dual-path activation, LAF unlock)
- **`src-tauri/src/sparse_identity.rs`**: `has_package_identity()` (slim — self-registration was removed with the Store-only decision)
- **`src-tauri/src/update_check.rs`**: `is_store_install()` — identity + `SignatureKind == Store` (identity alone is true for sparse-registered dev exes too)
- **`src-tauri/src/windows_ai_bindings.rs`**: Auto-generated WinRT bindings via `windows-bindgen` 0.66
- **`scripts/build-cross.py`**: Build script (MSIX bundling; `BUNDLE_AI_DLLS = False` for loose exes)

### Bundled DLLs (per architecture)
The MSIX package includes these DLLs for both x64 and ARM64:
- `Microsoft.WindowsAppRuntime.dll` (~2.3-2.7 MB)
- `Microsoft.Windows.AI.Text.dll` (~630-670 KB)
- `Microsoft.Windows.AI.Text.Projection.dll` (~240-260 KB)
- `Microsoft.WindowsAppRuntime.Bootstrap.dll` (~390 KB)
- `WinRT.Runtime.dll` (~1.4-1.6 MB)

### Technical Implementation

#### Manifest Configuration
```xml
<Package xmlns:systemai="http://schemas.microsoft.com/appx/manifest/systemai/windows10"
         IgnorableNamespaces="uap rescap systemai">
  <Dependencies>
    <!-- Both Universal and Desktop required for systemAIModels capability -->
    <TargetDeviceFamily Name="Windows.Universal" MinVersion="10.0.26100.0" MaxVersionTested="10.0.26226.0" />
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26226.0" />
    <!-- Framework dependency used by the Store MSIX bundle workflow -->
    <PackageDependency Name="Microsoft.WindowsAppRuntime.1.8"
                       MinVersion="8000.675.1142.0"
                       Publisher="CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US" />
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <systemai:Capability Name="systemAIModels"/>
  </Capabilities>
</Package>
```

### Requirements
1. **Windows 11 24H2** (build 26100+)
2. **Copilot+ PC** with NPU (40+ TOPS) - ARM64 or x64
3. **Registered package identity** (Microsoft Store build) with the `systemAIModels` capability — non-negotiable
4. **LAF token** bound to the Store PFN for generation on the stable framework

### Error Codes Reference
| Code | Name | Meaning |
|------|------|---------|
| `0x80040154` | CLASS_E_CLASSNOTREGISTERED | WinRT class not found - framework/bundled DLLs missing |
| `0x80070005` | E_ACCESSDENIED | No package identity (any activation path), or LAF not unlocked at generation |
| `0x80070032` | ERROR_NOT_SUPPORTED | Bootstrap API not supported for packaged apps; also `Add-AppxPackage` on a sparse MSIX without `-ExternalLocation` |

### Historical Approaches (What Didn't Work)

1. **RoGetActivationFactory with PackageDependency** → `0x80070005` (blocked for third-party)
2. **Bundled-DLL `DllGetActivationFactory` from an UNPACKAGED exe** → `0x80070005` — the bypass only skips the factory lookup; the API gates on identity (commit 96ca754, June 2026)
3. **LAF token unlock under sparse Developer identity with a different package name** → "Unavailable" (token binds to the full Store PFN, name included)
4. **Self-registering sparse package at startup for shipped loose exes** → worked mechanically, but requires trusting the self-signed cert and conflicts with an installed Store app (same identity) — abandoned in favor of Store-only
5. **Bootstrapper initialization** → `0x80070032` (not supported for packaged apps)

### Build Commands
```bash
# Full build with MSIX and signing (includes DLL bundling)
python3 scripts/build-cross.py build-all --build-msix --sign

# Just rebuild MSIX (without recompiling)
python3 scripts/build-cross.py build-msix --sign

# DEV ONLY: sparse identity packages to test the Store path on a loose exe
python3 scripts/build-cross.py build-sparse --sign
```

### Sparse Packaging (DEV TOOLING ONLY)
`build-sparse` creates per-arch "package with external location" identity
packages so a developer can test the Store-identity Phi Silica path without a
Store install: the exe stays loose on disk and a tiny signed MSIX (manifest +
logos only, `AllowExternalContent=true`) grants it the Store identity when
registered via `Install-SparseIdentity.ps1` (which uses
`Add-AppxPackage -ExternalLocation`; without that flag registration fails
with `0x80070032`). Prerequisites: the self-signed cert trusted once
(Trusted Root, admin) and the real Store app uninstalled first (one
registration per identity). The exe's embedded application manifest
(`src-tauri/windows-app.manifest`) carries the matching `<msix>` element —
without it Windows cannot attach identity to a directly-launched process.
Gotchas encoded in the manifests: concrete per-arch `ProcessorArchitecture`
(neutral breaks x64-on-ARM64 WinAppSDK resolution) and `MinVersion
10.0.26100.0` (lower values silently drop the systemai capability).
There is no in-app self-registration: shipped loose exes do not attempt to
gain identity (Store-only decision).

### AI Providers

| Provider (wire id) | Runs | Auth | Tools | Streaming | Budget (chars) |
|---|---|---|---|---|---|
| `phi_silica` | on-device NPU (Store build only) | package identity | no | no | 2,500 |
| `foundry_local` | local server | none | no (unverified) | yes | 12,000 |
| `ollama` | local server | none | yes | yes | 12,000 |
| `custom_openai` | any /v1/chat/completions server | optional key | yes | yes | 24,000 |
| `codex_cli` | cloud via installed Codex CLI | ChatGPT sign-in (CLI-owned) | no | no | 24,000 |
| `claude_code` | cloud via installed Claude Code CLI | Claude sign-in (CLI-owned) | no | no | 24,000 |
| `openai` | cloud | API key | yes | yes | 48,000 |
| `anthropic` | cloud (native Messages API) | API key | yes | yes | 48,000 |
| `gemini` | cloud (native generateContent) | API key | yes | yes | 48,000 |
| `deepseek` | cloud (OpenAI-compatible) | API key | yes | yes | 48,000 |

`ai_providers::capabilities()` is the single source of truth for this table.
Auto routing is local-first: Phi → Foundry → Ollama → custom → Codex CLI →
Claude Code → OpenAI → Anthropic → Gemini; the pure decision lives in `route_provider()`
(unit-tested, takes a `ProviderAvailability` struct); probing stays lazy in
`determine_active_provider_with_key()`. An explicit (non-Auto) preference
never falls back to another provider. In **Auto** chat, if the chosen
provider fails before any text streams (round 0, nothing emitted — e.g. a
flaky CLI bridge), `ai_chat_send` retries the same message on the next
available Auto provider via `next_auto_provider()`; `run_chat_turn`'s
`allow_fallback` returns `Err` without emitting a terminal event so the
retry is invisible. Fallback reuses the first provider's caps/system, which
is safe because Auto budgets are non-decreasing down the chain. Wire strings are pinned per-variant
with explicit `#[serde(rename)]` — `rename_all = "snake_case"` would emit
`"open_a_i"` for OpenAI (a real bug fixed in 2.5.0; do not reintroduce).

Provider gotchas encoded in the clients (don't relearn these):
- Anthropic: `max_tokens` REQUIRED; never send `temperature`; branch on
  `stop_reason == "refusal"` BEFORE reading content; default model constant
  `ANTHROPIC_DEFAULT_MODEL` in `ai_providers/anthropic.rs`.
- Gemini: auth via `x-goog-api-key` HEADER (never `?key=` — keys must not
  appear in URLs); assistant role is `"model"`; `functionResponse.response`
  must be a JSON object; no tool-call ids (synthesized as `name#index`).
- Generic/custom + Ollama use `/v1/chat/completions` (they do not serve
  `/v1/responses`); OpenAI/Foundry one-shot keeps the Responses API. No token
  cap goes on the wire (current OpenAI models reject `max_tokens`, compat
  servers don't all know `max_completion_tokens`).
- The Foundry Local port is dynamic by design; it is discovered via
  `foundry service status` or the `localAiEndpoint` setting — never hardcode
  it (resolution lives in `ai_providers/foundry.rs`).
- Ollama has no default model: the `ollamaModel` setting, else the first
  entry from `/api/tags`, else an error telling the user to pull a model.
- Subscription CLI bridges (`ai_providers/cli_bridge.rs` + `codex.rs` +
  `claude_cli.rs` + `acp_bridge.rs`): we implement NO OAuth and store NO
  tokens; the installed CLI owns sign-in (driven by the generic
  `ai_bridge_*` commands) and usage bills to the user's plan. OpenAI
  endorses this for Codex. The Claude transport mirrors Microsoft's
  Intelligent Terminal EXACTLY: spawn `npx -y
  @agentclientprotocol/claude-agent-acp` and speak ACP over stdio using the
  same `agent-client-protocol` crate (initialize → session/new →
  session/prompt; agent_message_chunk streams as deltas; permission
  requests are rejected — Q&A only; scrub `CLAUDECODE` or the adapter
  refuses to start). `claude -p --output-format json --max-turns 2` is only
  the no-Node fallback. What Anthropic's Feb 2026 terms ban is extracting
  subscription OAuth tokens for direct API use — never do that. Prompts go
  via stdin (never argv — npm `.cmd` shims + quoting), exes are resolved
  through `where.exe` because bare `Command::new("codex")` cannot spawn npm
  shims, codex runs use `codex exec --json --ephemeral --sandbox read-only`
  in an empty workdir, probes are TTL-cached 30 s, and every bridge child
  gets `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`/`OPENAI_API_KEY` scrubbed
  (headless CLIs prefer env keys over the stored login — a stale key breaks
  runs AND flips billing to the API; status probes also treat "not logged
  in" text as signed-out because exit codes lie).

API keys: one DPAPI file / keyring entry per provider via the closed
`ProviderKeyId` set (`dpapi.rs`); OpenAI keeps the legacy `credentials.bin`
name so existing installs need no migration. Keys NEVER land in
settings.json — `settings_for_disk()` strips them (tested invariant).

Agentic chat safety: the tool registry in `ai_tools.rs` is strictly
READ-ONLY (no `fix_issue`, no mutations) and chat-triggered diagnostic runs
are never written into the scan session. The loop is bounded: 4 tool
iterations × 8 calls, 45 s per-tool timeout, concurrency 3, then a forced
final answer. When Phi Silica is unavailable the status message says why
("requires the Microsoft Store version") and what to do (`winget install
Microsoft.FoundryLocal`).

### Testing
```powershell
# Remove old version
Get-AppxPackage *WindowsForumDiagnostics* | Remove-AppxPackage

# Install new version
Add-AppxPackage -Path "C:\code\WindowsForum_Diagnostics_2.1.5.msixbundle"

# Phi Silica debug log is OPT-IN: set WFDIAG_AI_LOG=1, then check
# C:\temp\phi-silica-rust.log
```

### Supported Hardware
- ✅ ARM64 Copilot+ PCs (Snapdragon X Elite/Plus) - Tested
- ✅ x64 Copilot+ PCs (Intel Core Ultra, AMD Ryzen AI) - DLLs bundled
