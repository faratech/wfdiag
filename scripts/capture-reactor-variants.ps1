# Reactor visual-variant capture: theme x motion over deterministic fixture
# states, recorded into reactor-baselines/variants.json.
#
# Variants are captured with the deterministic fixture environment so the
# only difference between two captures of one state is the requested system
# rendering variable (theme via WFDIAG_REACTOR_THEME, animation via
# SPI_SETCLIENTAREAANIMATION). System personalization is saved and restored.
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
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SystemParametersInfo(uint action, uint uiParam, ref bool pvParam, uint fWinIni);
}
'@

$SPI_GETCLIENTAREAANIMATION = 0x1042
$SPI_SETCLIENTAREAANIMATION = 0x1043
$SPIF_UPDATEINIFILE = 0x01
$SPIF_SENDCHANGE = 0x02

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

$motionSaved = $false
try {
    $motionValue = $false
    $motionSaved = [WfVariantNative]::SystemParametersInfo(
        $SPI_GETCLIENTAREAANIMATION, 0, [ref]$motionValue, 0)
    if (-not $motionSaved) {
        Write-Warning "Could not read the client-area animation setting; reduced-motion restore will be skipped."
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
                $off = $false
                $null = [WfVariantNative]::SystemParametersInfo(
                    $SPI_SETCLIENTAREAANIMATION, 0, [ref]$off,
                    $SPIF_UPDATEINIFILE -bor $SPIF_SENDCHANGE)
                Start-Sleep -Milliseconds 300
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

            if ($IncludeReducedMotion -and $motionSaved) {
                $restore = [bool]$motionValue
                $null = [WfVariantNative]::SystemParametersInfo(
                    $SPI_SETCLIENTAREAANIMATION, 0, [ref]$restore,
                    $SPIF_UPDATEINIFILE -bor $SPIF_SENDCHANGE)
            }
        }
    }

    # Merge into variants.json
    $variantsPath = Join-Path (Get-Location) $VariantsJson
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
    if ($motionSaved) {
        $restore = [bool]$motionValue
        $null = [WfVariantNative]::SystemParametersInfo(
            $SPI_SETCLIENTAREAANIMATION, 0, [ref]$restore,
            $SPIF_UPDATEINIFILE -bor $SPIF_SENDCHANGE)
    }
}
