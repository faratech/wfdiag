#!/usr/bin/env pwsh
# PowerShell script to bump version number across all project files

param(
    [Parameter(Mandatory=$true)]
    [string]$NewVersion,

    [Parameter(Mandatory=$false)]
    [switch]$DryRun
)

# Validate version format (e.g., 2.1.3)
if ($NewVersion -notmatch '^\d+\.\d+\.\d+$') {
    Write-Error "Invalid version format. Expected format: X.Y.Z (e.g., 2.1.3)"
    exit 1
}

# Files to update with their patterns
$files = @(
    @{
        Path = "package.json"
        Pattern = '(\s+"version":\s+)"[^"]+"'
        Replacement = "`$1`"$NewVersion`""
    },
    @{
        Path = "package-lock.json"
        Pattern = '(\s+"version":\s+)"2\.\d+\.\d+"'
        Replacement = "`$1`"$NewVersion`""
    },
    @{
        Path = "AppxManifest.xml"
        Pattern = '(Version=)"2\.\d+\.\d+\.0"'
        Replacement = "`$1`"$NewVersion.0`""
    },
    @{
        Path = "src-tauri/Cargo.toml"
        Pattern = '(version\s+=\s+)"2\.\d+\.\d+"'
        Replacement = "`$1`"$NewVersion`""
    },
    @{
        Path = "src-tauri/tauri.conf.json"
        Pattern = '(\s+"version":\s+)"2\.\d+\.\d+"'
        Replacement = "`$1`"$NewVersion`""
    },
    @{
        Path = "src/App.tsx"
        Pattern = '(version=)"2\.\d+\.\d+"'
        Replacement = "`$1`"$NewVersion`""
    },
    @{
        Path = "src/components/AboutDialog.tsx"
        Pattern = '(Version\s+)2\.\d+\.\d+'
        Replacement = "`${1}$NewVersion"
    },
    @{
        Path = "src/components/NavigationHeader.tsx"
        Pattern = "(version\s+=\s+')2\.\d+\.\d+'"
        Replacement = "`${1}$NewVersion'"
    }
)

Write-Host "Bumping version to $NewVersion..." -ForegroundColor Cyan

$updatedFiles = 0
$errorFiles = @()

foreach ($file in $files) {
    $filePath = Join-Path $PSScriptRoot $file.Path

    if (-not (Test-Path $filePath)) {
        Write-Warning "File not found: $($file.Path)"
        $errorFiles += $file.Path
        continue
    }

    try {
        $content = Get-Content $filePath -Raw
        $originalContent = $content

        # Apply regex replacement
        $content = $content -replace $file.Pattern, $file.Replacement

        if ($content -eq $originalContent) {
            Write-Warning "No changes made to: $($file.Path)"
        } else {
            if ($DryRun) {
                Write-Host "[DRY RUN] Would update: $($file.Path)" -ForegroundColor Yellow
            } else {
                # Preserve original line endings
                $content | Set-Content $filePath -NoNewline
                Write-Host "✓ Updated: $($file.Path)" -ForegroundColor Green
                $updatedFiles++
            }
        }
    } catch {
        Write-Error "Failed to update $($file.Path): $_"
        $errorFiles += $file.Path
    }
}

Write-Host ""
if ($DryRun) {
    Write-Host "DRY RUN completed. $updatedFiles file(s) would be updated." -ForegroundColor Yellow
} else {
    Write-Host "Version bump completed! $updatedFiles file(s) updated to version $NewVersion" -ForegroundColor Green
}

if ($errorFiles.Count -gt 0) {
    Write-Host ""
    Write-Warning "Failed to update $($errorFiles.Count) file(s):"
    $errorFiles | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}

if (-not $DryRun) {
    Write-Host ""
    Write-Host "Next steps:" -ForegroundColor Cyan
    Write-Host "  1. Review changes: git diff"
    Write-Host "  2. Commit changes: git add -A && git commit -m 'Bump version to $NewVersion'"
    Write-Host "  3. Build release: npm run tauri build"
}

exit 0
