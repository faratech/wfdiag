# Microsoft Store Release Notes

## Version 2.4.0

### What's New

**Scan History Becomes Drift Analysis**
- Side-by-side before/after view for every changed diagnostic, with field-level change highlights
- Search your scan history by label, date, or machine
- Label scans ("baseline", "after update") right from the History screen
- Flaky-task badges show how often a task failed across recent scans

**System Tray**
- New tray icon with Show/Hide, Quick Scan, and Exit
- Optional "Close to tray" mode keeps diagnostics a click away

**Three AI Providers, Clear Status**
- On-device Phi Silica (Copilot+ PCs, Store version), Foundry Local (free local AI server), or OpenAI cloud
- The AI screen now explains exactly which providers are available and how to enable the rest

**Quality**
- New automated test coverage for AI routing, issue detection, fix safety, and scan comparison
- Updated AI client libraries and dependency cleanup

### Requirements
- Windows 11 for full functionality
- Copilot+ PC and the Microsoft Store version required for on-device AI (Phi Silica)
- Foundry Local (free) or an OpenAI API key for AI features on any PC

---

## Version 2.3.0

### What's New

**A Faster, More Polished App**
- Command palette (Ctrl+K) and keyboard shortcuts for everything
- Modern custom titlebar, taskbar scan progress, and scan-complete notifications
- Refined visuals: loading skeletons, clearer empty states, accessible controls

**Smarter Scanning**
- Stop a scan and immediately start another (previous versions could lock up)
- Per-category progress while scanning

**Local AI Without the Store**
- New Foundry Local support: free, fully local AI analysis on any PC — no API key, no cloud

---

## Version 2.1.5

### What's New

**Smarter On-Device AI (Phi Silica)**
- AI now intelligently selects which system diagnostics to run based on your question
- Ask about your "computer model" and it automatically checks system info
- Ask about "security" and it runs firewall and service checks
- Faster responses with optimized context handling

**Improved Cloud AI**
- Upgraded to latest GPT-5.2 model for better system analysis

**Bug Fixes**
- Fixed AI displaying incorrect diagnostic information
- Improved response accuracy and relevance

### Requirements
- Windows 11 for full functionality
- Copilot+ PC required for on-device AI features (Phi Silica)
- OpenAI API key optional for cloud-based AI analysis

---

## Version 2.1.4

### What's New
- Phi Silica integration for Copilot+ PCs
- On-device AI analysis using NPU
- No internet required for local AI features

---

## Version 2.1.3

### What's New
- ARM64 native support for Windows on ARM devices
- Improved system detection and diagnostics
- Performance optimizations

---

*For detailed technical changelog, visit: https://github.com/faratech/wfdiag/releases*
