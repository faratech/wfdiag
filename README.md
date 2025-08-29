# WF Diagnostics v2.0.9 - WindowsForum Diagnostic Tool

[![License: CC BY-NC-ND 4.0](https://img.shields.io/badge/License-CC_BY--NC--ND_4.0-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-nd/4.0/)
[![Version](https://img.shields.io/badge/version-2.0.9-blue.svg)](https://github.com/faratech/wfdiag/releases)
[![Platform](https://img.shields.io/badge/platform-Windows-lightblue.svg)](https://github.com/faratech/wfdiag)
[![Security](https://img.shields.io/badge/security-hardened-green.svg)](https://github.com/faratech/wfdiag)

A **security-hardened**, modern diagnostic tool built with **Tauri v2** and **React** for Windows systems. Developed by **Fara Technologies LLC** for **WindowsForum.com**. This comprehensive rewrite features advanced security architecture, encrypted data storage, and intelligent system analysis.

## 🚀 Key Features

### **Core Capabilities**
- **38+ Diagnostic Tasks** across 8 categories (33 standard user + 5 admin-only)
- **Real-time System Monitoring** with CPU, memory, disk, and network stats
- **Encrypted Data Storage** using AES-256-GCM with machine-specific keys
- **AI-Powered Analysis** via OpenAI integration with function calling
- **OAuth2 Authentication** with WindowsForum.com integration
- **Scan History & Comparison** with intelligent change detection
- **Multiple Export Formats** (JSON, Text, Forum-formatted)

### **Security Architecture** 🔒
- **Command Injection Prevention** with strict command whitelisting (12 allowed commands)
- **PowerShell Script Filtering** blocks dangerous operations, allows only diagnostic cmdlets
- **Filesystem Access Restrictions** limited to 6 specific directory patterns
- **Encrypted Local Storage** for sensitive scan data and API keys
- **Secure OAuth2 Implementation** with PKCE and state validation
- **Input Validation & Sanitization** on all user inputs and system commands

### **Performance & UX** ⚡
- **Lightweight**: ~10MB vs ~100MB traditional tools
- **Fast Startup**: <1 second application launch
- **Batch Processing**: Runs 5 tasks concurrently for optimal performance
- **Smart Privilege Model**: 33/38 tasks work without admin privileges
- **Real-time Progress**: Live updates with health scoring
- **Responsive UI**: Modern Fluent Design System

## 📋 Diagnostic Categories

| Category | Tasks | Admin Required | Description |
|----------|-------|----------------|-------------|
| **System** | 7 | 0 | OS info, BIOS, boot config, environment variables |
| **Hardware** | 6 | 0 | CPU, RAM, motherboard, TPM, device enumeration |
| **Storage** | 5 | 2 | Disks, partitions, volumes, health checks |
| **Network** | 4 | 0 | Adapters, IP config, routing, connectivity |
| **Drivers** | 3 | 1 | System drivers, versions, digital signatures |
| **Software** | 6 | 0 | Programs, services, Windows features |
| **Logs** | 4 | 1 | Event logs, Windows Update, reliability |
| **Debug** | 3 | 1 | BSOD analysis, crash dumps, system files |

## 🛠️ Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Frontend (React + TypeScript)        │
├─────────────────────────────────────────────────────────┤
│  • App.tsx - Main application & diagnostics UI         │
│  • SystemMonitoring.tsx - Real-time stats dashboard    │
│  • ComparisonView.tsx - Scan comparison interface      │
│  • OAuthLogin.tsx - WindowsForum authentication        │
│  • OpenAIIntegration.tsx - AI analysis interface       │
├─────────────────────────────────────────────────────────┤
│                    Tauri v2 IPC Bridge                 │
├─────────────────────────────────────────────────────────┤
│                Backend (Pure Rust)                     │
├─────────────────────────────────────────────────────────┤
│  • lib.rs - 30+ Tauri commands & app lifecycle         │
│  • diagnostics.rs - Task definitions & execution       │
│  • security.rs - Command validation & filtering        │
│  • native_diagnostics.rs - Windows API implementations │
│  • encrypted_storage.rs - AES-256-GCM data encryption  │
│  • oauth.rs - WindowsForum OAuth2 with PKCE           │
│  • monitoring.rs - Real-time system stats collection   │
│  • openai_integration.rs - AI analysis with functions  │
└─────────────────────────────────────────────────────────┘
```

## 🔐 Security Features

### **Command Execution Hardening**
- **Whitelist-Only Execution**: Only 12 pre-approved system commands
- **Argument Validation**: Strict parameter checking for each command
- **PowerShell Protection**: Blocks dangerous cmdlets (Invoke-Expression, Start-Process, etc.)
- **WMI Query Filtering**: Approved Windows Management classes only

### **Data Protection**
- **AES-256-GCM Encryption**: All scan data encrypted at rest
- **Machine-Specific Keys**: Derived from Windows GUID + user context
- **PBKDF2 Key Derivation**: 100,000 iterations with unique salts
- **Memory Safety**: Sensitive keys zeroized after use

### **Network Security**
- **HTTPS-Only Communication**: All external requests use TLS
- **OAuth2 with PKCE**: Industry-standard authentication flow
- **Token Encryption**: Access tokens stored encrypted in Windows Credential Manager
- **Request Validation**: All API calls validated and sanitized

## 📦 Installation & Usage

### **Prerequisites**
- **Windows 10/11** (x64)
- **Administrator privileges** for 5 advanced diagnostic tasks
- **Internet connection** for AI analysis and updates

### **Development Setup**
```bash
# Install Rust
winget install Rustlang.Rust.GNU

# Install Node.js
winget install OpenJS.NodeJS

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
- `get_system_info()` - System info and admin status
- `get_available_tasks()` - List all 38 diagnostic tasks
- `start_diagnostics(task_ids)` - Begin diagnostic session
- `run_diagnostic_task(task_id)` - Execute single task
- `run_diagnostics_parallel(task_ids)` - Batch execution (5 concurrent)
- `get_session_results(session_id)` - Retrieve results
- `export_results(format, results)` - Export as JSON/Text/Forum

### **Security & Authentication**
- `store_api_key(key)` - Encrypt and store OpenAI API key
- `load_api_key()` - Decrypt and load API key
- `oauth_start_flow()` - Initiate WindowsForum OAuth2
- `oauth_handle_callback(code)` - Complete OAuth flow
- `authenticate(username, password)` - Direct login
- `logout()` - Clear authentication tokens

### **Data Management**
- `save_current_scan(results)` - Store encrypted scan results
- `list_scan_history()` - List all saved scans
- `load_scan(scan_id)` - Load specific scan results
- `compare_scans(current_id, previous_id)` - Intelligent comparison

### **Monitoring & Analysis**
- `start_monitoring()` - Begin real-time system monitoring
- `get_current_stats()` - Current CPU/memory/disk/network stats
- `get_network_connections()` - Active network connections
- `analyze_system_with_ai(api_key, results)` - OpenAI function calling analysis

### **System Integration**
- `restart_as_admin()` - Elevate privileges for advanced tasks
- `shell_open(path)` - Open files/folders in default applications
- `save_results_to_file(path, content)` - Export to disk

## 🧠 AI Analysis Features

- **OpenAI Responses API** integration with function calling
- **Automated problem detection** across all diagnostic categories
- **Intelligent recommendations** based on system state
- **Context-aware analysis** using diagnostic task metadata
- **Secure API key storage** with Windows Credential Manager

## 📊 System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| **OS** | Windows 10 (1809) | Windows 11 22H2+ |
| **Memory** | 4GB RAM | 8GB+ RAM |
| **Storage** | 50MB free | 100MB+ free |
| **Network** | Optional | Required for AI analysis |
| **Privileges** | Standard user | Admin for 5 advanced tasks |

## 🔄 Version History

### **v2.0.8b (Current) - Security Hardened Release**
- ✅ **Complete security architecture overhaul**
- ✅ **AES-256-GCM encryption** for all data storage
- ✅ **Command injection prevention** with strict whitelisting
- ✅ **OAuth2 authentication** with WindowsForum.com
- ✅ **Scan comparison system** with intelligent change detection
- ✅ **Real-time monitoring** dashboard
- ✅ **Enhanced UI** with login dialogs and comparison views

### **Previous Versions**
- **v2.0.7b** - Initial Tauri v2 implementation
- **v1.x** - WinUI 3 version (deprecated)

## 📄 License

This project is licensed under the **Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International License**.

### **Permissions & Restrictions**
- ✅ **Free for personal, educational, and research use**
- ✅ **Attribution required**: Must credit Fara Technologies LLC and WindowsForum.com  
- ❌ **No commercial use** without permission
- ❌ **No derivative works** - modifications not allowed for redistribution
- ❌ **No selling or profit** from this software

### **Commercial Licensing**
For commercial use, enterprise licensing, or to create derivative works, contact:
- * [contact](https://windowsforum.com/misc/contact)

### **Attribution Requirements**
When using or sharing this software, include:
> "WF Diagnostics Tool developed by Fara Technologies LLC for WindowsForum.com"

## 🤝 Contributing

While derivative works are restricted under the CC BY-NC-ND 4.0 license, you can:
- **Report Issues**: Submit bug reports and feature requests
- **Provide Feedback**: Share suggestions for improvements  
- **Documentation**: Help improve documentation and guides
- **Testing**: Help test new features and report compatibility issues

## 📞 Support

- **Issues**: [GitHub Issues](https://github.com/faratech/wfdiag/issues)
- **Documentation**: [WindowsForum.com](https://windowsforum.com/resources/windowsforum-com-diagnostic-tool.1/)
- **Commercial Support**: admin@windowsforum.com

---
**Copyright © 2025 Fara Technologies LLC. All rights reserved.**

**Developed for WindowsForum.com - Your trusted Windows diagnostic solution.**
