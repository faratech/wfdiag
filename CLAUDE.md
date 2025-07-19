# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
- **Run development server**: `npm run tauri dev` - Starts both Vite dev server and Tauri in development mode
- **Build for production**: `npm run tauri build` - Creates optimized production build
- **Build for Microsoft Store**: `npm run tauri build -- --bundles msix` - Creates MSIX package for Store submission
- **Build MSI installer**: `npm run tauri build -- --bundles msi` - Creates MSI for direct distribution
- **Frontend only dev**: `npm run dev` - Runs Vite dev server without Tauri (limited functionality)
- **Type checking**: `tsc` - Run TypeScript compiler to check for type errors

### Rust Backend
- **Check Rust code**: `cd src-tauri && cargo check` - Quick syntax and type checking
- **Format Rust code**: `cd src-tauri && cargo fmt` - Format code according to Rust standards
- **Run Rust lints**: `cd src-tauri && cargo clippy` - Run Rust linter for code quality

## Architecture

This is a Tauri application with a clear separation between frontend and backend:

### Frontend (src/)
- **React 18** with TypeScript for UI
- **Fluent UI React Components** for Windows-native look
- **Tauri API** for IPC communication with backend
- Single-page application in `App.tsx` (1,747 lines) handling all UI logic
- Uses Vite for fast development and optimized production builds

### Backend (src-tauri/)
- **Pure Rust** implementation for system diagnostics
- **main.rs**: Tauri command handlers and application lifecycle
- **diagnostics.rs**: Core diagnostic task definitions and execution logic
- **native_diagnostics.rs**: Windows-specific diagnostic implementations
- **windows_native.rs**: Direct Windows API bindings and wrappers
- Extensive use of Windows crates for WMI, registry, and system information

### Key Architectural Decisions
1. **IPC Bridge**: All system access goes through Tauri commands - frontend never directly accesses system
2. **Async Operations**: All diagnostic tasks run asynchronously using Tokio
3. **Task Batching**: Diagnostics run in batches of 5 to prevent overwhelming the system
4. **Error Handling**: Comprehensive error handling with fallbacks for each diagnostic
5. **State Management**: Frontend uses React hooks, backend uses Rust's ownership model

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
All backend functionality is exposed through these Tauri commands in `main.rs`:
- `get_system_info`: Basic system information
- `get_available_tasks`: List of all diagnostic tasks
- `start_diagnostics`: Begin a new diagnostic session
- `run_diagnostic_task`: Execute a specific diagnostic
- `get_session_results`: Retrieve results from current session
- `export_results`: Export in JSON/Text/Forum format
- `restart_as_admin`: Elevate to administrator privileges

### Windows API Integration
The backend makes extensive use of Windows APIs through the `windows` crate:
- WMI queries for system information
- Registry access for software and configuration
- Native APIs for hardware enumeration
- Performance counters for system metrics

### Security Considerations
- Application requests admin privileges for full diagnostic access
- All file system access is controlled through Tauri permissions
- No arbitrary code execution from frontend
- Secure IPC bridge prevents injection attacks

## Development Tips

1. **Frontend Changes**: Most UI logic is in `App.tsx`. The component is large but well-organized by feature
2. **Adding Diagnostics**: New diagnostics go in `diagnostics.rs` with implementation in `native_diagnostics.rs`
3. **Windows APIs**: Use existing patterns in `windows_native.rs` for new API calls
4. **Error Handling**: Always provide fallbacks - diagnostics should never crash the app
5. **Testing**: Manual testing required due to system-specific nature of diagnostics
6. **Performance**: Keep diagnostic batches small (5 tasks) to maintain UI responsiveness