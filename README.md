# WF Diagnostics v2.5.7 - WindowsForum Diagnostic Tool

[![License: CC BY-NC-ND 4.0](https://img.shields.io/badge/License-CC_BY--NC--ND_4.0-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-nd/4.0/)
[![Version](https://img.shields.io/badge/version-2.5.7-blue.svg)](https://github.com/faratech/wfdiag/releases)
[![Platform](https://img.shields.io/badge/platform-Windows-lightblue.svg)](https://github.com/faratech/wfdiag)
[![Security](https://img.shields.io/badge/security-hardened-green.svg)](https://github.com/faratech/wfdiag)
[![AI](https://img.shields.io/badge/AI-Hybrid-purple.svg)](https://github.com/faratech/wfdiag)

A Windows diagnostics application built with Tauri v2, Rust, React, and TypeScript. It combines native system checks, live monitoring, issue detection and guided remediation, encrypted scan history, process inspection, and an optional multi-provider AI assistant.

## 🚀 Key Features

### **Core Capabilities**
- **46 Diagnostic Tasks** across system, hardware, storage, network, security, software, logs, graphics, drivers, performance, and debugging
- **Real-time System Monitoring** with live CPU, memory, disk, and network visualization (Non-blocking)
- **Encrypted Scan History** protected for the current Windows user with DPAPI
- **29 Deterministic Issue Rules** with explicit verified, detected, and unknown outcomes
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
- API keys use one DPAPI/keyring entry per provider and are never returned to the webview
- Tauri capabilities expose only the required core/window operations, notifications, save dialog, and clipboard-write access

## 📋 Diagnostic Categories

The registry currently contains 46 checks. `src-tauri/src/diagnostics.rs` is the authoritative task catalog; the UI derives its category counts from that registry instead of maintaining a second list.

## 🛠️ Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Frontend (React + TypeScript)        │
├─────────────────────────────────────────────────────────┤
│  • screens/ - Diagnostics, Monitor, Processes, AI,     │
│    Issues, and History                                 │
│  • contexts/ + hooks/ - shared state and behavior      │
│  • components/ui/ - reusable accessible primitives     │
├─────────────────────────────────────────────────────────┤
│                    Tauri v2 IPC Bridge                 │
├─────────────────────────────────────────────────────────┤
│                Backend (Pure Rust)                     │
├─────────────────────────────────────────────────────────┤
│  • lib.rs - IPC registration and app lifecycle         │
│  • diagnostics.rs - Core diagnostic task execution     │
│  • native_diagnostics.rs - Windows-specific checks      │
│  • ai_service.rs + ai_providers/ - provider routing    │
│  • issue_catalog.rs + issue_detector.rs - issue engine │
│  • remediation.rs - closed repair catalog              │
│  • results_storage.rs - encrypted scan history          │
│  • native_monitor.rs - Real-time performance counters  │
└─────────────────────────────────────────────────────────┘
```

## 📦 Installation & Usage

### **Prerequisites**
- **Windows 10/11** on x64 or ARM64
- **Rust MSVC toolchain** and Visual Studio C++ build tools for native development
- **Node.js**: `winget install OpenJS.NodeJS`
- **Optional Phi Silica:** Copilot+ PC, Windows 11 24H2+, and the Microsoft Store package identity

### **Development Setup**
```bash
# Clone repository
git clone https://github.com/faratech/wfdiag.git
cd wfdiag

# Install dependencies
npm install

# Run in development mode
npm run tauri dev

# Build for production
npm run tauri build

# Build the unsigned Microsoft Store/Phi Silica MSIX bundle
python3 scripts/build-cross.py build-all --build-msix
```

The Store workflow uploads an unsigned bundle; Microsoft signs the package delivered through the Store. Add `--sign` only when creating a locally sideloadable test bundle. The direct Tauri MSIX config (`src-tauri/tauri.msix.conf.json`) is a basic MSIX experiment and is not the Store/Phi Silica package.

## 🎯 Tauri Commands API

### **Core Diagnostics**
- `start_diagnostics(task_ids)` - Begin diagnostic session
- `run_diagnostics_parallel(task_ids)` - Batch execution (5 concurrent)
- `detect_issues()` - Return detected and unverified checks
- `run_remediation(id, confirmed)` - Execute a cataloged remediation with per-step results

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

Auto routing is local-first: Phi Silica → Foundry Local → Ollama → custom OpenAI-compatible → Codex CLI → Claude Code → OpenAI → Anthropic → Gemini → DeepSeek. An explicit provider selection never silently routes elsewhere. See `ai_providers::capabilities()` for the authoritative provider capabilities and context budgets.

## 🔄 Version History

### **v2.5.7 (Current) - Live AI Model Discovery**
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
