# WFDiag Tauri - WindowsForum Diagnostic Tool

A modern, lightweight diagnostic tool built with Tauri and React for Windows systems. This is a complete rewrite of the WinUI 3 version using Rust and web technologies.

## Features

- 🚀 **Lightweight**: ~10MB vs ~100MB for WinUI 3 version
- 🦀 **Pure Rust Backend**: Direct integration with wfdiag-backend
- ⚛️ **Modern React UI**: Built with Fluent UI React components
- 📦 **Microsoft Store Ready**: MSIX packaging support included
- 🔒 **Secure**: Tauri's security-first architecture
- 🎨 **Native Look**: Fluent Design System matching Windows 11

## Architecture

```
Frontend (React + TypeScript)
    ↕️ Tauri IPC Bridge
Backend (Rust - wfdiag-backend)
```

## Prerequisites

1. **Rust**: Install from https://rustup.rs/
2. **Node.js**: v16 or higher
3. **Windows SDK**: For building Windows apps
4. **Visual Studio Build Tools**: C++ build tools

## Development

1. Install dependencies:
```bash
npm install
```

2. Run in development mode:
```bash
npm run tauri dev
```

3. Build for production:
```bash
npm run tauri build
```

## Building for Microsoft Store

The project is pre-configured for MSIX packaging:

```bash
npm run tauri build -- --bundles msix
```

This creates an MSIX package ready for Microsoft Store submission.

## Project Structure

```
wfdiag-tauri/
├── src/                    # React frontend
│   ├── App.tsx            # Main application component
│   ├── main.tsx           # Entry point
│   └── index.css          # Styles
├── src-tauri/             # Rust backend
│   ├── src/
│   │   └── main.rs        # Tauri commands and logic
│   ├── Cargo.toml         # Rust dependencies
│   └── tauri.conf.json    # Tauri configuration
├── package.json           # Node dependencies
└── README.md             # This file
```

## Key Differences from WinUI 3 Version

| Feature | WinUI 3 | Tauri |
|---------|----------|--------|
| Binary Size | ~100MB | ~10MB |
| Memory Usage | ~150MB | ~50MB |
| Startup Time | 2-3s | <1s |
| Language | C# + Rust | Rust + TypeScript |
| UI Framework | XAML | React |
| Store Publishing | Native | MSIX Bridge |

## Commands

The Tauri backend exposes these commands:

- `get_system_info`: Get system information
- `get_available_tasks`: List all diagnostic tasks
- `start_diagnostics`: Begin a diagnostic session
- `run_diagnostic_task`: Execute a specific task
- `get_session_results`: Retrieve results
- `export_results`: Export in various formats
- `restart_as_admin`: Elevate privileges

## Deployment

### Microsoft Store
1. Build MSIX: `npm run tauri build -- --bundles msix`
2. Test with Windows App Certification Kit
3. Submit to Partner Center

### Direct Distribution
1. Build MSI: `npm run tauri build -- --bundles msi`
2. Sign the installer
3. Distribute .msi file

## License

Same as the main wfdiag project.