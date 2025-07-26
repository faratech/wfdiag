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

### Rust Backend
- **Check Rust code**: `cd src-tauri && cargo check` - Quick syntax and type checking
- **Format Rust code**: `cd src-tauri && cargo fmt` - Format code according to Rust standards
- **Run Rust lints**: `cd src-tauri && cargo clippy` - Run Rust linter for code quality
- **Update dependencies**: `cd src-tauri && cargo update` - Update Rust dependencies to latest compatible versions

## Architecture

This is a Tauri v2 application with a clear separation between frontend and backend:

### Frontend (src/)
- **React 18** with TypeScript for UI
- **Fluent UI React Components** (@fluentui/react-components) for Windows-native look
- **Tauri v2 API** for IPC communication with backend
- **Chart.js** with react-chartjs-2 for real-time system monitoring graphs
- Single-page application in `App.tsx` handling all UI logic
- Real-time monitoring component in `SystemMonitoring.tsx`
- OpenAI integration component in `OpenAIIntegration.tsx`
- Uses Vite for fast development and optimized production builds

### Backend (src-tauri/)
- **Tauri v2** with lib.rs/main.rs structure for mobile support
- **Pure Rust** implementation for system diagnostics
- **lib.rs**: Main application logic and Tauri command handlers
- **main.rs**: Entry point that calls the run function from lib.rs
- **diagnostics.rs**: Core diagnostic task definitions and execution logic
- **native_diagnostics.rs**: Windows-specific diagnostic implementations
- **windows_native.rs**: Direct Windows API bindings and wrappers
- **monitoring.rs**: Real-time system monitoring with CPU, memory, disk, and network stats
- **openai_integration.rs**: OpenAI Responses API integration for AI-powered system analysis

### Key Dependencies (Latest Versions)
- Tauri: v2.6.2
- sysinfo: v0.36 (system information)
- windows: v0.61 (Windows API bindings)
- wmi: v0.17 (Windows Management Instrumentation)
- reqwest: v0.12 (HTTP client for OpenAI)
- winreg: v0.55 (Windows Registry access)

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
- `tauri-plugin-process`: Process management
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
All backend functionality is exposed through these Tauri commands in `lib.rs`:
- `get_system_info`: Basic system information and admin status
- `get_available_tasks`: List of all diagnostic tasks
- `start_diagnostics`: Begin a new diagnostic session
- `run_diagnostic_task`: Execute a specific diagnostic
- `get_session_results`: Retrieve results from current session
- `export_results`: Export in JSON/Text/Forum format
- `save_results_to_file`: Save results to disk
- `get_uptime`: System uptime information
- `restart_as_admin`: Elevate to administrator privileges
- `start_monitoring`: Begin real-time system monitoring
- `stop_monitoring`: Stop real-time monitoring
- `get_current_stats`: Get current system statistics
- `get_network_connections`: Get active network connections
- `analyze_with_openai`: Legacy OpenAI analysis
- `analyze_system_with_ai`: OpenAI Responses API with function calling

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

1. **Frontend Changes**: Most UI logic is in `App.tsx`. The component is large but well-organized by feature
2. **Adding Diagnostics**: New diagnostics go in `diagnostics.rs` with implementation in `native_diagnostics.rs`
3. **Windows APIs**: Use existing patterns in `windows_native.rs` for new API calls
4. **Error Handling**: Always provide fallbacks - diagnostics should never crash the app
5. **Testing**: Manual testing required due to system-specific nature of diagnostics
6. **Performance**: Keep diagnostic batches small (5 tasks) to maintain UI responsiveness
7. **Tauri v2 Imports**: Use `@tauri-apps/api/core` for invoke, plugin packages for specific features
8. **Real-time Updates**: Use Tauri events (`emit` from backend, `listen` in frontend) for streaming data