# Version Management

This directory contains scripts to manage version numbers across the entire project from a single source of truth.

## Single Source of Truth

The version is defined in `/version.json`:

```json
{
  "version": "2.1.1",
  "name": "WF Diagnostics", 
  "description": "WindowsForum Diagnostic Tool"
}
```

## Update Scripts

### Node.js Script
```bash
# Update all files from version.json
node scripts/update-version.js

# Or use npm script
npm run update-version
npm run version-sync
```

### PowerShell Script
```powershell
# Update all files from version.json
.\scripts\update-version.ps1

# Update to a specific version (updates version.json first, then all files)
.\scripts\update-version.ps1 -NewVersion "2.2.0"
```

## Files Updated

Both scripts automatically update:

- `package.json` - NPM package version
- `src-tauri/Cargo.toml` - Rust crate version and description
- `tauri.conf.json` - Tauri root config version and product name
- `src-tauri/tauri.conf.json` - Tauri source config version and product name  
- `src/App.tsx` - Version display in UI

## Workflow

1. **To bump version:**
   ```bash
   # Option 1: Edit version.json manually, then run:
   npm run update-version
   
   # Option 2: Use PowerShell with direct version parameter:
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