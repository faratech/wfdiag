# Version Management

This directory contains scripts to manage version numbers across the entire project from a single source of truth.

## Single Source of Truth

The version is defined in `/version.json`:

```json
{
  "version": "2.5.4",
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

`src-tauri/src/windows_ai_bindings.rs` is reviewed, tracked generated source. Ordinary builds never rewrite it. Regenerate it explicitly with `python3 scripts/build-cross.py generate-bindings`, review the diff, and commit the result separately from a release build.

CI and release builds run `scripts/check-version-sync.py` before packaging. It verifies `version.json`, npm and Cargo manifests/locks, both Tauri/MSIX manifests, the frontend displays, and README version markers. Rust build commands use `--locked` so a release cannot silently resolve a different dependency graph.

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
