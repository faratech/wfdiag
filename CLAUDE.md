# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
- **Run development server**: `npm run tauri dev` - Starts both Vite dev server and Tauri in development mode
- **Build for production**: `npm run tauri build` - Creates optimized production build with exe, MSI, and NSIS installers
- **Build for Microsoft Store**: `npm run tauri build -- --bundles msix` - Creates MSIX package for Store submission
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
- **ai_service.rs / ai_cache.rs / ai_prompts.rs**: Unified AI layer (provider routing, response cache, prompts)
- **openai_integration.rs**: OpenAI Responses API integration; **phi_silica.rs**: on-device Phi Silica
- **issue_detector.rs / issue_fixer.rs**: Issue detection from scan results and whitelisted automated fixes
- **results_storage.rs**: Scan results storage and comparison system
- **state.rs**: AppState (current session, monitor, scan storage, cancelled sessions)
- **windows_ai_bindings.rs**: GENERATED by windows-bindgen — never hand-edit

### Key Dependencies
- Tauri: v2.11 (Tauri framework; plugins: fs, dialog, clipboard-manager, shell, notification)
- windows: v0.62 (Windows API bindings with native WMI support via Win32::System::Wmi)
- winreg: v0.55 (Windows Registry access)
- async-openai: v0.32 (OpenAI API client, Responses API)
- tokio: v1.52 (Async runtime)

### Key Architectural Decisions
1. **Tauri v2 Migration**: Uses new plugin architecture with separate packages for filesystem, dialog, clipboard, process, and shell
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
- File system access (read, write, create, remove)
- Dialog operations (open, save, message)
- Clipboard manager
- Process management
- Shell operations

### Plugin Usage
The app uses these Tauri v2 plugins:
- `tauri-plugin-fs`: File system operations
- `tauri-plugin-dialog`: Native dialogs
- `tauri-plugin-clipboard-manager`: Clipboard access
- `tauri-plugin-shell`: Shell operations

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
- `save_current_scan`, `list_scan_history`, `load_scan`, `compare_scans`, `clear_scan_history`
- `commands::settings::*`: load/save settings, keyring-backed API key storage
- AI: `ai_get_status`, `ai_analyze_diagnostic`, `ai_analyze_section`, `ai_explain_health`,
  `ai_set_preference`, `ai_clear_cache`, plus `phi_silica::*` and `openai_integration::*`

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

### Current Status: ✅ WORKING (December 2025)

The integration is **complete and working** on both ARM64 and x64 Copilot+ PCs!

### The Solution: Direct DLL Activation

The key breakthrough was understanding that **standard WinRT activation (`RoGetActivationFactory`) doesn't work** for Windows App SDK classes from third-party apps. The solution is to use `DllGetActivationFactory` directly from bundled DLLs, which is exactly how Microsoft's CsWinRT projection works with `WindowsAppSDKSelfContained=true`.

**What works:**
1. Bundle Windows App SDK 2.0-experimental3 DLLs with the app
2. Load `Microsoft.Windows.AI.Text.dll` from app directory
3. Call `DllGetActivationFactory("Microsoft.Windows.AI.Text.LanguageModel")` directly
4. Use the returned factory to create LanguageModel instances

**Why this works:** The bundled DLLs are Microsoft-signed and contain the full implementation. By calling their activation factory directly, we bypass the WinRT activation tables that block third-party apps.

### Files Involved
- **`src-tauri/src/phi_silica.rs`**: Main implementation with `create_language_model_direct()` function
- **`src-tauri/src/windows_ai_bindings.rs`**: Auto-generated WinRT bindings via `windows-bindgen` 0.65
- **`scripts/build-cross.py`**: Build script that bundles Windows App SDK DLLs

### Bundled DLLs (per architecture)
The MSIX package includes these DLLs for both x64 and ARM64:
- `Microsoft.WindowsAppRuntime.dll` (~2.3-2.7 MB)
- `Microsoft.Windows.AI.Text.dll` (~630-670 KB)
- `Microsoft.Windows.AI.Text.Projection.dll` (~240-260 KB)
- `Microsoft.WindowsAppRuntime.Bootstrap.dll` (~390 KB)
- `WinRT.Runtime.dll` (~1.4-1.6 MB)

### Technical Implementation

#### Direct DLL Activation (the key!)
```rust
fn create_language_model_direct() -> Result<LanguageModel, String> {
    // Load bundled DLL from app directory
    let app_dir = std::env::current_exe()?.parent()?;
    let dll_path = app_dir.join("Microsoft.Windows.AI.Text.dll");
    let module = LoadLibraryW(dll_path)?;

    // Get DllGetActivationFactory export
    let get_factory = GetProcAddress(module, "DllGetActivationFactory");

    // Create HSTRING for class name
    let class_name = HSTRING::from("Microsoft.Windows.AI.Text.LanguageModel");

    // Get activation factory directly from DLL (bypasses RoGetActivationFactory!)
    let mut factory_ptr = null_mut();
    get_factory(class_name.as_raw(), &mut factory_ptr);

    // Query for ILanguageModelStatics and call CreateAsync
    let statics: ILanguageModelStatics = factory.cast()?;
    let async_op = statics.CreateAsync()?;
    wait_for_async_blocking(async_op)
}
```

#### Manifest Configuration
```xml
<Package xmlns:systemai="http://schemas.microsoft.com/appx/manifest/systemai/windows10"
         IgnorableNamespaces="uap rescap systemai">
  <Dependencies>
    <!-- Both Universal and Desktop required for systemAIModels capability -->
    <TargetDeviceFamily Name="Windows.Universal" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26226.0" />
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26226.0" />
    <!-- Optional: Framework dependency (not strictly required since we bundle DLLs) -->
    <PackageDependency Name="Microsoft.WindowsAppRuntime.2.0-experimental3"
                       MinVersion="0.676.658.0"
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
3. **MSIX packaging** with `systemAIModels` capability and bundled DLLs

### LAF Token (Not Required!)
Originally thought to be required, but the direct DLL activation approach works without LAF approval:
- LAF unlock returns "Unavailable"
- But Phi Silica works anyway because we bypass RoGetActivationFactory

### Error Codes Reference
| Code | Name | Meaning |
|------|------|---------|
| `0x80040154` | CLASS_E_CLASSNOTREGISTERED | WinRT class not found - need bundled DLLs |
| `0x80070005` | E_ACCESSDENIED | Using RoGetActivationFactory - switch to DllGetActivationFactory |
| `0x80070032` | ERROR_NOT_SUPPORTED | Bootstrap API not supported for packaged apps |

### Historical Approaches (What Didn't Work)

1. **RoGetActivationFactory with PackageDependency** → `0x80070005` (blocked for third-party)
2. **LAF token unlock** → Returns "Unavailable" for third-party apps
3. **Loading DLL without using DllGetActivationFactory** → Still uses RoGetActivationFactory internally
4. **Bootstrapper initialization** → `0x80070032` (not supported for packaged apps)

### Build Commands
```bash
# Full build with MSIX and signing (includes DLL bundling)
python3 scripts/build-cross.py build-all --build-msix --sign

# Just rebuild MSIX (without recompiling)
python3 scripts/build-cross.py build-msix --sign

# Sparse identity packages for the loose exes (Phi Silica without full MSIX)
python3 scripts/build-cross.py build-sparse --sign
```

### Sparse Packaging (Phi Silica without full MSIX)
The Windows AI APIs require package identity + the `systemAIModels` capability,
but NOT full MSIX packaging. `build-sparse` creates per-arch "package with
external location" identity packages: the exe stays loose on disk and a tiny
signed MSIX (manifest + logos only, `AllowExternalContent=true`) grants it
identity when registered. Registration is automatic: at startup the app
self-registers `wfdiag-sparse-<arch>.msix` found next to the exe via
`PackageManager.AddPackageByUriAsync` + `ExternalLocationUri`
(`src-tauri/src/sparse_identity.rs`) and relaunches itself once so identity
attaches — no PowerShell needed. `Install-SparseIdentity.ps1` remains as the
manual fallback. The only remaining manual prerequisite with the self-signed
cert is trusting it once (Trusted Root, admin); a publicly trusted cert
removes that too. The exe's embedded application manifest
(`src-tauri/windows-app.manifest`) carries the matching `<msix>` element —
without it Windows cannot attach identity to a directly-launched process.
Gotchas encoded in the manifests: concrete per-arch `ProcessorArchitecture`
(neutral breaks x64-on-ARM64 WinAppSDK resolution) and `MinVersion
10.0.26100.0` (lower values silently drop the systemai capability).

### AI Provider Fallback Chain
`ai_service.rs` routes Auto preference as: Phi Silica (on-device WinRT, needs
identity) → Foundry Local (local OpenAI-compatible server, no identity; model
`phi-4-mini` — Phi Silica itself is not served by Foundry Local) → OpenAI
(cloud, needs API key). The Foundry Local port is dynamic by design; it is
discovered via `foundry service status` or the `localAiEndpoint` setting —
never hardcode it.

### Testing
```powershell
# Remove old version
Get-AppxPackage *WindowsForumDiagnostics* | Remove-AppxPackage

# Install new version
Add-AppxPackage -Path "C:\code\WindowsForum_Diagnostics_2.1.5.msixbundle"

# Check logs at C:\temp\phi-silica-rust.log
```

### Supported Hardware
- ✅ ARM64 Copilot+ PCs (Snapdragon X Elite/Plus) - Tested
- ✅ x64 Copilot+ PCs (Intel Core Ultra, AMD Ryzen AI) - DLLs bundled