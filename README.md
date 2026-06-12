# WF Diagnostics v2.3.0 - WindowsForum Diagnostic Tool

[![License: CC BY-NC-ND 4.0](https://img.shields.io/badge/License-CC_BY--NC--ND_4.0-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-nd/4.0/)
[![Version](https://img.shields.io/badge/version-2.3.0-blue.svg)](https://github.com/faratech/wfdiag/releases)
[![Platform](https://img.shields.io/badge/platform-Windows-lightblue.svg)](https://github.com/faratech/wfdiag)
[![Security](https://img.shields.io/badge/security-hardened-green.svg)](https://github.com/faratech/wfdiag)
[![AI](https://img.shields.io/badge/AI-Hybrid-purple.svg)](https://github.com/faratech/wfdiag)

A **security-hardened**, modern diagnostic tool built with **Tauri v2** and **React** for Windows systems. Developed by **Fara Technologies LLC** for **WindowsForum.com**. This comprehensive utility features advanced security architecture, encrypted data storage, and a unique **Hybrid AI** analysis engine that leverages both OpenAI and local **Phi Silica** models on Copilot+ PCs.

## 🚀 Key Features

### **Core Capabilities**
- **38+ Diagnostic Tasks** across 8 categories (System, Hardware, Storage, Network, etc.)
- **Real-time System Monitoring** with live CPU, memory, disk, and network visualization (Non-blocking)
- **Encrypted Data Storage** using AES-256-GCM with machine-specific keys
- **Automated Issue Detection** identifies critical problems (low disk space, firewall disabled, etc.)
- **One-Click Fixes** for common system issues
- **Scan History & Comparison** with intelligent change detection
- **Multiple Export Formats** (JSON, Text, HTML, Forum-formatted)

### **Hybrid AI Architecture** 🧠
- **Unified AI Service**: Seamlessly switches between Cloud (OpenAI) and Edge (Phi Silica) models.
- **Local AI (Phi Silica)**: Runs entirely on-device using the NPU on Copilot+ PCs (Windows 11 24H2+).
- **Cloud AI (OpenAI)**: Fallback integration for standard PCs with function calling support.
- **Context-Aware Analysis**: Intelligent interpretation of diagnostic results and health scores.

### **Security Architecture** 🔒
- **Command Injection Prevention** with strict command whitelisting (12 allowed commands)
- **PowerShell Script Filtering** blocks dangerous operations, allows only diagnostic cmdlets
- **Filesystem Access Restrictions** limited to specific safe directories
- **Encrypted Local Storage** for sensitive scan data and API keys
- **Secure OAuth2 Implementation** with PKCE for WindowsForum.com integration
- **Input Validation & Sanitization** on all user inputs and system commands

## 📋 Diagnostic Categories

| Category | Tasks | Description |
|----------|-------|-------------|
| **System** | 7 | OS info, BIOS, boot config, environment variables, updates |
| **Hardware** | 6 | CPU, RAM, motherboard, TPM, device enumeration |
| **Storage** | 5 | Disks, partitions, volumes, fragmentation, SMART health |
| **Network** | 4 | Adapters, IP config, routing, connectivity, DNS |
| **Drivers** | 3 | System drivers, versions, digital signatures |
| **Software** | 6 | Programs, services, Windows features, startup apps |
| **Logs** | 4 | Event logs, Windows Update history, reliability |
| **Debug** | 3 | BSOD analysis, crash dumps, system files |

## 🛠️ Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Frontend (React + TypeScript)        │
├─────────────────────────────────────────────────────────┤
│  • App.tsx - Main application layout & routing         │
│  • SystemMonitoring.tsx - Real-time stats dashboard    │
│  • IssuesTab.tsx - Issue detection & remediation UI    │
│  • AIInterpretationPanel.tsx - AI analysis display     │
│  • HealthModel.tsx - System health scoring visualization│
├─────────────────────────────────────────────────────────┤
│                    Tauri v2 IPC Bridge                 │
├─────────────────────────────────────────────────────────┤
│                Backend (Pure Rust)                     │
├─────────────────────────────────────────────────────────┤
│  • lib.rs - Command exposure & app state management    │
│  • diagnostics.rs - Core diagnostic task execution     │
│  • ai_service.rs - Unified AI provider abstraction     │
│  • phi_silica.rs - Local AI (Windows App SDK/WinRT)    │
│  • issue_detector.rs - Automated problem identification│
│  • security.rs - Command validation & input filtering  │
│  • encrypted_storage.rs - AES-256-GCM data encryption  │
│  • native_monitor.rs - Real-time performance counters  │
└─────────────────────────────────────────────────────────┘
```

## 🔐 Security Features

### **Command Execution Hardening**
- **Whitelist-Only Execution**: Only 12 pre-approved system commands
- **Argument Validation**: Strict parameter checking for each command
- **PowerShell Protection**: Blocks dangerous cmdlets (Invoke-Expression, Start-Process, etc.)

### **Data Protection**
- **AES-256-GCM Encryption**: All scan data encrypted at rest
- **Machine-Specific Keys**: Derived from Windows GUID + user context
- **DPAPI Integration**: Secure storage for API keys and tokens

## 📦 Installation & Usage

### **Prerequisites**
- **Windows 10/11** (x64)
- **Rust**: `winget install Rustlang.Rust.GNU`
- **Node.js**: `winget install OpenJS.NodeJS`
- **(Optional) Copilot+ PC**: Required for local Phi Silica AI features (Windows 11 24H2+)

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
```

## 🎯 Tauri Commands API

### **Core Diagnostics**
- `start_diagnostics(task_ids)` - Begin diagnostic session
- `run_diagnostics_parallel(task_ids)` - Batch execution (5 concurrent)
- `detect_issues()` - Analyze results for specific problems
- `fix_issue(issue_id)` - Attempt to automatically resolve an issue

### **AI & Analysis**
- `ai_get_status()` - Check availability of OpenAI and Phi Silica
- `ai_analyze_diagnostic(...)` - Interpret specific task results
- `analyze_with_phi_silica(prompt)` - Direct interaction with local model
- `ensure_phi_silica()` - Initialize/Download local model

### **Data Management**
- `save_current_scan(results)` - Store encrypted scan results
- `compare_scans(current_id, previous_id)` - Intelligent comparison
- `export_results(format)` - Export as JSON/Text/HTML

## 🧠 AI Capabilities

The tool employs a **Hybrid AI Strategy**:

1.  **Phi Silica (Local):** Prioritized on supported hardware (Copilot+ PCs). It runs locally on the NPU, ensuring privacy and zero latency. It uses the `Microsoft.Windows.AI.Text` API via Windows App SDK.
2.  **OpenAI (Cloud):** Fallback for standard PCs. Requires an API key. Supports advanced function calling to autonomously select and run diagnostic tasks based on user queries.

## 🔄 Version History

### **v2.3.0 (Current) - Performance & Accuracy Improvements**
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
