# Version Bump Scripts

Automated scripts to bump the version number across all project files.

## Usage

### PowerShell (Windows)

```powershell
# Bump version to 2.1.4
.\bump-version.ps1 2.1.4

# Dry run (preview changes without modifying files)
.\bump-version.ps1 2.1.4 -DryRun
```

### Bash (Linux/macOS/Git Bash)

```bash
# Make script executable (first time only)
chmod +x bump-version.sh

# Bump version to 2.1.4
./bump-version.sh 2.1.4

# Dry run (preview changes without modifying files)
./bump-version.sh 2.1.4 --dry-run
```

## What Gets Updated

The scripts automatically update the version number in these files:

1. **package.json** - NPM package version
2. **package-lock.json** - NPM lockfile version (2 occurrences)
3. **AppxManifest.xml** - Windows Store package version (format: X.Y.Z.0)
4. **src-tauri/Cargo.toml** - Rust package version
5. **src-tauri/tauri.conf.json** - Tauri configuration version
6. **src/App.tsx** - Frontend app version prop
7. **src/components/AboutDialog.tsx** - About dialog display version
8. **src/components/NavigationHeader.tsx** - Header component default version

## Version Format

Version must follow semantic versioning: `MAJOR.MINOR.PATCH`

Examples:
- ✅ `2.1.3`
- ✅ `3.0.0`
- ✅ `2.2.1`
- ❌ `2.1` (missing patch)
- ❌ `v2.1.3` (no 'v' prefix)
- ❌ `2.1.3-beta` (no suffixes)

## Workflow

1. **Run the script:**
   ```bash
   ./bump-version.ps1 2.1.4
   ```

2. **Review changes:**
   ```bash
   git diff
   ```

3. **Commit changes:**
   ```bash
   git add -A
   git commit -m "Bump version to 2.1.4"
   ```

4. **Tag the release:**
   ```bash
   git tag -a v2.1.4 -m "Release version 2.1.4"
   ```

5. **Build the release:**
   ```bash
   npm run tauri build
   ```

6. **Push to remote:**
   ```bash
   git push origin dev
   git push origin v2.1.4
   ```

## Troubleshooting

### Script not found or permission denied

**PowerShell:**
```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

**Bash:**
```bash
chmod +x bump-version.sh
```

### Pattern not found

If the script reports "Pattern not found", it means the version number format in that file doesn't match the expected pattern. Check the file manually and update the regex pattern in the script if needed.

### Version mismatch

Always ensure all files have the same version before running the bump script. Use:

```bash
# PowerShell
Select-String -Path package.json,src-tauri/Cargo.toml,src/App.tsx -Pattern "2\.\d+\.\d+"

# Bash
grep -rn "2\.[0-9]\+\.[0-9]\+" package.json src-tauri/Cargo.toml src/App.tsx
```

## Notes

- The script preserves original file line endings
- AppxManifest.xml version gets a `.0` suffix (e.g., `2.1.4` becomes `2.1.4.0`)
- Always test with `--dry-run` first before applying changes
- The script only updates version numbers starting with `2.` (current major version)
