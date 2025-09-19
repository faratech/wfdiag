# PowerShell script to update version from version.json

param(
    [string]$NewVersion = $null
)

# Get script directory
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir = Split-Path -Parent $ScriptDir
$VersionFile = Join-Path $RootDir "version.json"

# If a new version is provided, update version.json first
if ($NewVersion) {
    Write-Host "Updating version.json to $NewVersion..." -ForegroundColor Cyan
    $versionData = Get-Content $VersionFile | ConvertFrom-Json
    $versionData.version = $NewVersion
    $versionData | ConvertTo-Json -Depth 10 | Set-Content $VersionFile -Encoding UTF8
}

# Read version from version.json
$versionData = Get-Content $VersionFile | ConvertFrom-Json
$version = $versionData.version
$name = $versionData.name
$description = $versionData.description

Write-Host "Updating all files to version $version..." -ForegroundColor Green

# Update package.json
$packageJsonPath = Join-Path $RootDir "package.json"
$packageJson = Get-Content $packageJsonPath | ConvertFrom-Json
$packageJson.version = $version
$packageJson | ConvertTo-Json -Depth 10 | Set-Content $packageJsonPath -Encoding UTF8
Write-Host "Updated $packageJsonPath" -ForegroundColor Green

# Update Cargo.toml
$cargoTomlPath = Join-Path $RootDir "src-tauri\Cargo.toml"
$cargoLines = Get-Content $cargoTomlPath
$inPackageSection = $false
$updatedLines = @()

foreach ($line in $cargoLines) {
    if ($line -match '^\[package\]') {
        $inPackageSection = $true
        $updatedLines += $line
    }
    elseif ($line -match '^\[') {
        $inPackageSection = $false
        $updatedLines += $line
    }
    elseif ($inPackageSection -and $line -match '^version = ') {
        $updatedLines += "version = `"$version`""
    }
    elseif ($inPackageSection -and $line -match '^description = ') {
        $updatedLines += "description = `"$description`""
    }
    else {
        $updatedLines += $line
    }
}

$updatedLines | Set-Content $cargoTomlPath -Encoding UTF8
Write-Host "Updated $cargoTomlPath" -ForegroundColor Green

# Update tauri.conf.json files
$tauriConfigPaths = @(
    (Join-Path $RootDir "tauri.conf.json"),
    (Join-Path $RootDir "src-tauri\tauri.conf.json")
)

foreach ($configPath in $tauriConfigPaths) {
    if (Test-Path $configPath) {
        $config = Get-Content $configPath | ConvertFrom-Json
        $config.version = $version
        $config.productName = $name
        $config | ConvertTo-Json -Depth 10 | Set-Content $configPath -Encoding UTF8
        Write-Host "Updated $configPath" -ForegroundColor Green
    }
}

# Update App.tsx version display
$appTsxPath = Join-Path $RootDir "src\App.tsx"
$appTsx = Get-Content $appTsxPath -Raw
$appTsx = $appTsx -replace 'v\d+\.\d+\.\d+', "v$version"
$appTsx | Set-Content $appTsxPath -NoNewline -Encoding UTF8
Write-Host "Updated $appTsxPath" -ForegroundColor Green

Write-Host ""
Write-Host "All files updated to version $version!" -ForegroundColor Yellow
Write-Host ""
Write-Host "Files updated:" -ForegroundColor Cyan
Write-Host "- package.json" -ForegroundColor White
Write-Host "- src-tauri/Cargo.toml" -ForegroundColor White  
Write-Host "- tauri.conf.json" -ForegroundColor White
Write-Host "- src-tauri/tauri.conf.json" -ForegroundColor White
Write-Host "- src/App.tsx" -ForegroundColor White
Write-Host ""
Write-Host "Usage:" -ForegroundColor Cyan
Write-Host "Update to version 2.2.0: .\update-version.ps1 -NewVersion 2.2.0" -ForegroundColor White
Write-Host "Update from version.json: .\update-version.ps1" -ForegroundColor White