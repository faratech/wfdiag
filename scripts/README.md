# Version Management

This directory contains scripts to manage version numbers across the entire project from a single source of truth.

## Single Source of Truth

The version is defined in `/version.json`:

```json
{
  "version": "2.5.8",
  "name": "WF Diagnostics", 
  "description": "WindowsForum Diagnostic Tool"
}
```

## Update Scripts

### Node.js Script
```bash
# Update all files from version.json
node scripts/update-version.js

# Update to a specific version
node scripts/update-version.js 2.2.0

# Or use npm script
npm run update-version
npm run version-sync
```

### PowerShell Script
```powershell
# Update all files from version.json
.\scripts\update-version.ps1

# Update to a specific version
.\scripts\update-version.ps1 -NewVersion "2.2.0"

# Preview changes
.\scripts\update-version.ps1 -NewVersion "2.2.0" -DryRun
```

## Files Updated

All entry points delegate to `bump-version.py` and update:

- `version.json` - Version source of truth
- `package.json` - NPM package version
- `package-lock.json` - NPM lockfile package version
- `src-tauri/Cargo.toml` - Rust crate version and description
- `src-tauri/tauri.conf.json` - Tauri config version
- `AppxManifest.xml` - MSIX Identity version only
- `src/components/AboutDialog.tsx` - Version display
- `src/App.tsx` - Version display in UI
- `src-tauri/tauri.msix.conf.json` - Tauri MSIX version
- `README.md` - Version references

## MSIX Build Paths

Use `python3 scripts/build-cross.py build-all --build-msix` for the unsigned Microsoft Store/Phi Silica package. That path generates the Store identity manifest with `runFullTrust`, `systemAIModels`, Windows App Runtime dependency floors, and the architecture-specific MSIX bundle. Microsoft signs the package distributed through the Store; `--sign` is only for locally sideloadable test bundles.

`src-tauri/tauri.msix.conf.json` is kept in version sync only for basic Tauri MSIX experiments. Do not use it for Store submissions or Phi Silica validation; Tauri's MSIX config does not represent the Store package manifest used by the release workflow.

`crates/wfdiag-native-phi/src/windows_ai_bindings.rs` is reviewed, tracked generated source. Ordinary builds never rewrite it. Regenerate it explicitly with `python3 scripts/build-cross.py generate-bindings`, review the diff, and commit the result separately from a release build.

CI and release builds run `scripts/check-version-sync.py` before packaging. It verifies `version.json`, npm and Cargo manifests/locks, both Tauri/MSIX manifests, the frontend displays, and README version markers. Rust build commands use `--locked` so a release cannot silently resolve a different dependency graph.

## Native Reactor visual capture

`capture-reactor-baselines.ps1` launches the supplied Reactor executable once
for each of the 18 Store 2.5.8 states recorded in
`reactor-baselines/manifest.json`. It applies the exact viewport and
deterministic state variables, captures the launched PID rather than an
arbitrary process-name match, validates every PNG's dimensions, and restores
the caller's environment afterward. Before any WinUI window is created, the
script runs the executable's machine-readable version probe and rejects a
binary whose reported application version does not exactly match the pinned
baseline manifest. Unsupported older binaries time out and are rejected, so a
stale 2.5.4 development build cannot silently produce 2.5.8 evidence.

```powershell
.\scripts\capture-reactor-baselines.ps1 `
  -Executable C:\path\to\wfdiag-reactor-spike.exe
```

For one-off interactive captures, `capture-window.ps1` accepts either
`-ProcessId` or `-ProcessName`. Automated evidence should use `-ProcessId`;
name-based capture rejects ambiguous visible instances.

`test-reactor-live-system.ps1` validates a built candidate without deterministic
fixtures. It launches live Diagnostics at the Store 2.5.8 1440x1000 viewport,
checks the native machine/OS/elevation/architecture UI Automation projection,
footer version, local XAML and browserless module state, writes logical,
physical and UIA evidence, closes the exact PID, and checks Application
Error/WER. Build and run the startup gate first; this script does not build.

```powershell
.\scripts\test-reactor-live-system.ps1 `
  -Executable C:\path\to\wfdiag-reactor-spike.exe `
  -OutputDirectory C:\path\to\live-system-evidence `
  -HoldSeconds 2
```

`test-reactor-about-parity.ps1` captures the installed Microsoft Store 2.5.8
About dialog beside a freshly built native Reactor candidate. The oracle is
deliberately strict: it requires the exact Store-signed ARM64 2.5.8 package,
the exact package family/AUMID, a 150% (144 DPI) display, and no already-running
Store or candidate process. The candidate must be ARM64 and its version probe
must report 2.5.8. The script opens both dialogs through UI Automation, requires
the six exact control names plus the complete description, captures foreground
DWM-visible frames at 1440x1000, rejects blank/dim-overlay-only captures, and
writes source/native/combined PNGs and JSON evidence. It also verifies local
WinUI XAML, no loaded or staged WebView2 files, graceful exits, and Application
Error/WER. It does not build the candidate and refuses to overwrite a non-empty
output directory.

Run it only after the new ARM64 self-contained candidate has passed the startup
and live-system gates:

```powershell
.\scripts\test-reactor-about-parity.ps1 `
  -Executable C:\path\to\new-build\aarch64-pc-windows-msvc\release\wfdiag-reactor-spike.exe `
  -OutputDirectory C:\Temp\wfdiag-reactor-about-2.5.8-new
```

## Workflow

1. **To bump version:**
   ```bash
   # Option 1: Edit version.json manually, then run:
   npm run update-version
   
   # Option 2: Use a direct version parameter:
   node scripts/update-version.js 2.2.0
   .\scripts\update-version.ps1 -NewVersion "2.2.0"
   ```

2. **After version update:**
   ```bash
   # Commit the changes
   git add .
   git commit -m "Bump version to 2.2.0"
   
   # Build with new version
   npm run tauri build
   ```

## Benefits

- ✅ Single source of truth for version numbers
- ✅ Consistent versioning across all files
- ✅ Reduces manual errors when bumping versions
- ✅ Easy integration with CI/CD pipelines
- ✅ Cross-platform support (Node.js + PowerShell)
