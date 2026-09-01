# Reactor visual-variant capture: theme x motion over deterministic fixture
# states, recorded into reactor-baselines/variants.json.
#
# Variants are captured with the deterministic fixture environment so the
# only difference between two captures of one state is the requested system
# rendering variable (theme via WFDIAG_REACTOR_THEME, animation via
# SPI_SETCLIENTAREAANIMATION). Reduced-motion capture first snapshots the
# setting before Windows applies the temporary personalization change, then
# restores and verifies that exact value in a finally block.
#
# Output: PNG captures under -OutputDirectory plus one variants.json record
# per capture (validated by scripts/check-variants.py).

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\variants",
    [string]$VariantsJson = "reactor-baselines\variants.json",
    [string[]]$Themes = @("dark", "light"),
    [ValidateSet("diagnostics-populated", "monitor-empty", "processes-empty", "settings-bottom")]
    [string[]]$States = @("diagnostics-populated", "monitor-empty", "processes-empty", "settings-bottom"),
    [switch]$IncludeReducedMotion
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WfVariantNative {
    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetSystemParametersInfo(uint action, uint uiParam, out int pvParam, uint fWinIni);

    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetSystemParametersInfo(uint action, uint uiParam, IntPtr pvParam, uint fWinIni);
}
'@

$SPI_GETCLIENTAREAANIMATION = 0x1042
$SPI_SETCLIENTAREAANIMATION = 0x1043
$SPIF_UPDATEINIFILE = 0x01
$SPIF_SENDCHANGE = 0x02

function Get-ClientAreaAnimation {
    $value = 0
    if (-not [WfVariantNative]::GetSystemParametersInfo(
            $SPI_GETCLIENTAREAANIMATION, 0, [ref]$value, 0)) {
        $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Could not read the client-area animation setting (Win32 error $code)."
    }
    return $value -ne 0
}

function Set-ClientAreaAnimation {
    param([Parameter(Mandatory = $true)][bool]$Enabled)

    # For this SET action pvParam carries the BOOL value in the pointer-sized
    # argument; passing ref bool (as the old harness did) reports success but
    # leaves the setting unchanged.
    $value = if ($Enabled) { [IntPtr]1 } else { [IntPtr]0 }
    # Windows does not apply SPI_SETCLIENTAREAANIMATION when fWinIni is zero.
    # Snapshot-before-mutation plus the verified outer finally makes this
    # temporary profile-backed change non-destructive.
    if (-not [WfVariantNative]::SetSystemParametersInfo(
            $SPI_SETCLIENTAREAANIMATION, 0, $value,
            $SPIF_UPDATEINIFILE -bor $SPIF_SENDCHANGE)) {
        $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Could not set client-area animation to '$Enabled' (Win32 error $code)."
    }

    $observed = Get-ClientAreaAnimation
    if ($observed -ne $Enabled) {
        throw "Client-area animation verification failed: requested '$Enabled', observed '$observed'."
    }
}

function Get-AbsolutePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $Path))
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$version = Get-ReactorApplicationVersion -Executable $resolvedExecutable `
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-variants-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

# State -> (env page, visual state, width, height). Fixture states only:
# deterministic, and visual variants must be pure rendering diffs.
$stateCatalog = @{
    "diagnostics-populated" = @{ Page = "diagnostics"; Fixture = "populated"; Width = 1440; Height = 900 }
    "monitor-empty" = @{ Page = "monitor"; Visual = "monitor-empty"; Width = 1440; Height = 1000 }
    "processes-empty" = @{ Page = "processes"; Visual = "processes-empty"; Width = 1440; Height = 1000 }
    "settings-bottom" = @{ Page = "ai"; Visual = "settings-bottom"; Width = 1440; Height = 900 }
}

$motionSnapshotTaken = $false
$motionMutationAttempted = $false
$originalMotionValue = $false
try {
    if ($IncludeReducedMotion) {
        # Never attempt the mutation unless the exact original value is known.
        $originalMotionValue = Get-ClientAreaAnimation
        $motionSnapshotTaken = $true
        # Mark the attempt before calling SET so even a partial Win32 failure
        # takes the restoration path.
        $motionMutationAttempted = $true
        Set-ClientAreaAnimation -Enabled $false
        Start-Sleep -Milliseconds 300
    }

    $records = @()
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"

    foreach ($theme in $Themes) {
        foreach ($state in $States) {
            $configuration = $stateCatalog[$state]
            $motionReduced = $false
            $motionLabel = "normal"
            if ($IncludeReducedMotion) {
                $motionReduced = $true
                $motionLabel = "reduced"
            }

            $variables = @{
                WFDIAG_REACTOR_THEME = $theme
                WFDIAG_REACTOR_WIDTH = [string]$configuration.Width
                WFDIAG_REACTOR_HEIGHT = [string]$configuration.Height
            }
            if ($configuration.Contains("Fixture")) {
                $variables.WFDIAG_REACTOR_FIXTURE = $configuration["Fixture"]
                $variables.WFDIAG_REACTOR_PAGE = $configuration["Page"]
            }
            else {
                $variables.WFDIAG_REACTOR_VISUAL_STATE = $configuration["Visual"]
            }

            $session = Start-ReactorCandidate -Executable $resolvedExecutable `
                -Seconds 10 -Variables $variables
            try {
                $captureName = "$state-$theme-$motionLabel"
                $capturePath = Join-Path $outputDirectory "$captureName.png"
                # Record the repo-relative form passed via -OutputDirectory.
                $captureRecordPath = (Join-Path $OutputDirectory "$captureName.png") `
                    -replace '\\', '/' 
                $null = & (Join-Path $PSScriptRoot "capture-window.ps1") `
                    -ProcessId $session.process.Id `
                    -OutputPath $capturePath `
                    -WaitSeconds 15

                $sha256 = (Get-FileHash -LiteralPath $capturePath -Algorithm SHA256).Hash
                $records += [pscustomobject]@{
                    id = $captureName
                    theme = $theme
                    highContrast = $false
                    reducedMotion = $motionReduced
                    scale = 1.0
                    experimental = $false
                    state = $state
                    applicationVersion = $version
                    executable = $resolvedExecutable
                    capturedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
                    png = $captureRecordPath
                    sha256 = $sha256
                }
                Write-Host "Captured $captureName"
            }
            finally {
                Stop-ReactorCandidate -Session $session `
                    -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8 | Out-Null
            }
        }
    }

    # Merge into variants.json
    $variantsPath = Get-AbsolutePath -Path $VariantsJson
    $document = if (Test-Path -LiteralPath $variantsPath) {
        Get-Content -LiteralPath $variantsPath -Raw | ConvertFrom-Json
    }
    else {
        [pscustomobject]@{
            schema = 1
            applicationVersion = $version
            defects = @(
                [pscustomobject]@{
                    id = "processes-refresh-rendering"
                    status = "open"
                    reported = "2026-08-30"
                    note = "Owner-reported: the process list refresh rendering diverges badly from the Store build. Triptych captures feed the fix."
                }
            )
            variants = @()
        }
    }
    $existing = @($document.variants | Where-Object { $_.id })
    $kept = @($existing | Where-Object {
        $record = $_
        -not ($records | Where-Object { $_.id -eq $record.id })
    })
    $document.variants = @($kept) + @($records)
    $document.applicationVersion = $version
    Write-JsonFile -Value $document -Path $variantsPath
    Write-Host "Variants document updated: $variantsPath ($($records.Count) new records)."
}
finally {
    if ($motionSnapshotTaken -and $motionMutationAttempted) {
        Set-ClientAreaAnimation -Enabled $originalMotionValue
    }
}
