# WF Diagnostics v2.5.8 - WindowsForum Diagnostic Tool

[![License: CC BY-NC-ND 4.0](https://img.shields.io/badge/License-CC_BY--NC--ND_4.0-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-nd/4.0/)
[![Version](https://img.shields.io/badge/version-2.5.8-blue.svg)](https://github.com/faratech/wfdiag/releases)
[![Platform](https://img.shields.io/badge/platform-Windows-lightblue.svg)](https://github.com/faratech/wfdiag)
[![Security](https://img.shields.io/badge/security-hardened-green.svg)](https://github.com/faratech/wfdiag)
[![AI](https://img.shields.io/badge/AI-Hybrid-purple.svg)](https://github.com/faratech/wfdiag)

A Windows diagnostics application written in Rust. It combines native system checks, live monitoring, issue detection and guided remediation, encrypted scan history, process inspection, and an optional multi-provider AI assistant.

The shipping UI is a **native WinUI 3 shell** (`apps/wfdiag`, binary `wfdiag.exe`, built on `windows-reactor`); the original Tauri v2 + React shell (`src-tauri`, `src/`) is kept buildable as a rollback and will be removed in a later release. Both shells are thin hosts over the same framework-neutral engine crates in `crates/`, so behaviour cannot drift between them. See `docs/REACTOR_MIGRATION.md` for the migration record and the 2026-09-01 cutover decision.

## 🚀 Key Features

### **Core Capabilities**
- **45 Diagnostic Tasks** across system, hardware, storage, network, security, software, logs, graphics, drivers, performance, and debugging
- **Real-time System Monitoring** with live CPU, memory, disk, and network visualization (Non-blocking)
- **Encrypted Scan History** protected for the current Windows user with DPAPI
- **28 Deterministic Issue Rules** with explicit verified, detected, and unknown outcomes
- **Tiered Remediation** with backend-enforced confirmation for repair operations
- **Scan History & Comparison** with intelligent change detection
- **Process Explorer** with backend filtering, sorting, pagination, pause, and detail lookup
- **Multiple Export Formats** (JSON, text, HTML, and forum-friendly text)

### **Optional AI Assistant** 🧠
- **Local/self-hosted options:** Phi Silica in the Microsoft Store build, Foundry Local, Ollama, and custom OpenAI-compatible servers
- **Subscription bridges:** Codex CLI and Claude Code CLI use their own installed sign-in; wfdiag does not extract or store subscription tokens
- **Cloud APIs:** OpenAI, Anthropic, Gemini, and DeepSeek with per-provider keys stored outside `settings.json`
- **Privacy controls:** WindowsForum network grounding is opt-in, and local-to-cloud fallback follows the saved Ask/Allow/Never policy
- **Two focused views:** Assistant conversation and Scan Report; provider configuration stays in Settings

### **Security Architecture** 🔒
- Diagnostic subprocesses use a closed command allowlist, validated arguments, hidden windows, and timeouts
- AI chat tools are read-only; repairs always execute through the separate remediation catalog
- Scan history uses current-user DPAPI protection and atomic file replacement
- API keys use one DPAPI entry per provider, are stripped from `settings.json`, and are never returned to the UI
- Export destinations go through a shared path policy; external links resolve from a closed enum, never a URL string crossing the UI boundary
- Repairs execute only through `wfdiag-native-remediation`'s action broker, which enforces the Repair-tier confirmation itself

## 📋 Diagnostic Categories

The registry currently contains 45 checks. `crates/wfdiag-native-diagnostics/src/catalog.rs` is the authoritative task catalog; both shells derive their category counts from that registry instead of maintaining a second list.

## 🛠️ Architecture

One Cargo workspace: framework-neutral engine crates plus two shells that only drive them.

```
apps/wfdiag  (SHIPPING)                  src-tauri + src/  (ROLLBACK)
native WinUI 3 / windows-reactor         Tauri v2 + React
        │                                        │
        └──────────────┬─────────────────────────┘
                       │
              crates/wfdiag-app
   AppService { start, snapshot, dispatch, drain, shutdown }
        ports + domain state machines + headless tests
                       │
 ┌─────────────────────┴──────────────────────────────────────┐
 │ crates/wfdiag-native-*                                     │
 │  core        error, timestamps, atomic writes, WMI         │
 │  diagnostics task catalog + Windows collectors             │
 │  monitor     live CPU/mem/disk/net/GPU/NPU (cfg(windows))  │
 │  issues      catalog, detectors, projection, fix plans     │
 │  remediation action broker = the only execution path       │
 │  history     DPAPI-encrypted scan store + comparison       │
 │  export      renderers + path policy + URL policy          │
 │  settings    settings doc + per-provider credential store  │
 │  system / update / projection / ui-core                    │
 │  ai-provider / ai-chat / ai-report / ai-analysis / phi     │
 └────────────────────────────────────────────────────────────┘
```

Engine crates build and test on Linux with no Windows and no GUI (CI job `rust-portable`);
`wfdiag-native-monitor` is the one `#![cfg(windows)]` crate. See `CLAUDE.md` for the full map.

## 📦 Installation & Usage

### **Prerequisites**
- **Windows 10/11** on x64 or ARM64
- **Rust MSVC toolchain** (pinned in `rust-toolchain.toml`) and Visual Studio C++ build tools
- **Windows App Runtime 2.4+** to run the native shell (the Store package declares it as a framework dependency)
- **Node.js** only for the Tauri rollback shell: `winget install OpenJS.NodeJS`
- **Optional Phi Silica:** Copilot+ PC, Windows 11 24H2+, and the Microsoft Store package identity

### **Development Setup**
```bash
git clone https://github.com/faratech/wfdiag.git
cd wfdiag

# Native shell (the product) — Windows, x64 or ARM64
cargo run   -p wfdiag --target x86_64-pc-windows-msvc
cargo build -p wfdiag --release --target aarch64-pc-windows-msvc

# Engine crates: build and test anywhere, no Windows required
cargo test --workspace --exclude wfdiag --exclude wfdiag-tauri

# Tauri rollback shell (needs Node)
npm install
npm run tauri dev
npm run tauri build
```

The Microsoft Store package is produced by `.github/workflows/build-and-publish-store.yml` on a
version tag: it builds `wfdiag.exe` per architecture and packages it with
`python3 scripts/build-reactor-msix-probe.py stage|pack|bundle|validate-msix`. Manual dispatch
accepts a `shell` input (`reactor`, the default, or `tauri` for the rollback). The workflow
uploads an unsigned bundle; Microsoft signs the package delivered through the Store.

`AppxManifest.xml` launches `wfdiag.exe` and depends on `Microsoft.WindowsAppRuntime.2`
(MinVersion 2.4.0.0), single-sourced from `reactor-baselines/manifest.json`. The legacy
`src-tauri/tauri.msix.conf.json` is a basic Tauri MSIX experiment, not the Store package.

## 🎯 Application API

The native shell drives the engine through one facade — `AppService::dispatch(AppCommand)` in
`crates/wfdiag-app` — and reads `AppEvent`s back from `drain()`. The Tauri rollback shell
exposes the equivalent surface as IPC commands, listed below for reference.

### **Core Diagnostics**
- `start_diagnostics(task_ids)` - Begin diagnostic session
- `run_diagnostics_parallel(task_ids)` - Batch execution (5 concurrent)
- `detect_issues()` - Return detected and unverified checks
- `action_prepare(...)` / `action_approve(...)` - Stage a remediation proposal, then execute it
  through the action broker (Repair tier requires explicit confirmation, enforced in the broker)

### **AI & Analysis**
- `ai_get_status()` - Probe configured/local AI providers
- `ai_chat_send(...)` / `ai_chat_cancel(...)` - Start or cancel a streamed assistant turn
- `ai_chat_get_history(...)` - Rehydrate a conversation after navigation
- `ai_generate_report(...)` - Generate the dedicated scan report

### **Data Management**
- `save_current_scan(results)` - Store encrypted scan results
- `compare_scans_summary(...)` / `get_scan_task_diff(...)` - Lazy scan comparison
- `list_processes(query)` - Paginated and sortable process data
- `export_results(format)` - Export as JSON/Text/HTML

## 🧠 AI Capabilities

Auto routing is local-first: Phi Silica → Foundry Local → Ollama → custom OpenAI-compatible → Codex CLI → Claude Code → OpenAI → Anthropic → Gemini → DeepSeek. An explicit provider selection never silently routes elsewhere. `wfdiag_native_ai_provider::capabilities()` is the authoritative source for provider capabilities and context budgets; the routing decision itself is the pure, unit-tested `route_provider()`.

## 🔄 Version History

### **v2.5.8 (Current) - Live AI Model Discovery**
- ✅ **Always-current model catalogs**: Loads available models directly from provider APIs and the Codex/Claude CLI metadata instead of shipping static lists.
- ✅ **Claude model clarity**: Shows Opus, Sonnet, Haiku, and Fable versions, exact IDs, and provider descriptions in an accessible searchable picker.
- ✅ **Gemini freshness**: Ranks live compatible models semantically and dynamically selects the newest stable general-purpose model when no override is saved.
- ✅ **GPT-5.6 compatibility**: Supports Sol, Terra, Luna, and future provider-reported OpenAI models, including the required tool-calling reasoning configuration.
- ✅ **Provider attribution**: Displays the requested and provider-reported model for AI responses when available.
- ✅ **Simpler settings**: Consolidates redundant provider controls while preserving Auto multi-provider setup.

### **v2.5.5 - Performance & Accuracy Improvements**
- ✅ **Non-blocking System Monitor**: Decoupled slow polling operations (Disk, NPU) for instant UI responsiveness.
- ✅ **Network Rate Fix**: Corrected transfer rate calculation to eliminate spikes during startup.
- ✅ ✅ **Accurate Swap Metrics**: Switched to native PDH counters for true paging file utilization.
- ✅ **Settings Persistence**: Resolved issue with AI provider preference resetting to "Auto".
- ✅ **UI Polishing**: Enhanced network charts with simultaneous Upload/Download tooltips.

### **v2.1.7 - Hybrid AI & Remediation**
- ✅ **Hybrid AI Engine**: Integration of local Phi Silica models alongside OpenAI.
- ✅ **Issue Detector**: Automated identification of common system problems.
- ✅ **Auto-Fixer**: One-click remediation for supported issues.
- ✅ **Health Model**: Visual health scoring system.
- ✅ **UI Updates**: New Issues tab and improved navigation rail.

### **Previous Versions**
- **v2.1.0** - Security Hardening & Encryption
- **v2.0.0** - Rewrite in Tauri v2 + React

## 📄 License

This project is licensed under the **Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International License**.

- ✅ **Free for personal, educational, and research use**
- ❌ **No commercial use** without permission
- ❌ **No derivative works**

### **Commercial Licensing**
Contact: * [contact](https://windowsforum.com/misc/contact)

---
**Copyright © 2025 Fara Technologies LLC. All rights reserved.**
**Developed for WindowsForum.com**
