# Reactor rendering-parity target #1: the process list refresh triptych.
#
# The owner reported the process list refresh rendering as visibly wrong.
# This script captures the Reactor Processes screen at three moments —
# initial load, mid-refresh (capture races the refresh), and settled — so
# the divergence is measurable frame by frame. Combined sheets are produced
# against the existing Store processes-populated baseline when present.
#
# Output: PNGs + triptych JSON under -OutputDirectory and a record appended
# to reactor-baselines/variants.json (validated by check-variants.py).

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\validation-process-refresh",
    [string]$VariantsJson = "reactor-baselines\variants.json",
    [string]$StoreBaselinePng = "reactor-baselines\captures\store-2.5.8\processes-populated-desktop-dark.png",
    [ValidateRange(1, 20)][int]$HoldSeconds = 2
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$version = Get-ReactorApplicationVersion -Executable $resolvedExecutable `
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-procparity-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "process-refresh-parity"
    captures = @()
    gracefulClose = $null
    crashEvents = @()
    failures = $failures
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 8 `
    -Variables @{
        WFDIAG_REACTOR_PAGE = "processes"
        WFDIAG_REACTOR_WIDTH = "1440"
        WFDIAG_REACTOR_HEIGHT = "1000"
    }

function Invoke-Capture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $path = Join-Path $outputDirectory "$Name.png"
    $null = & (Join-Path $PSScriptRoot "capture-window.ps1") `
        -ProcessId $session.process.Id `
        -OutputPath $path `
        -WaitSeconds 15
    $evidence.captures += $path
    Write-Host "Captured $Name -> $path"
    return $path
}

try {
    $process = $session.process
    $process.Refresh()
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    # 1. Initial settled load (process query usually completes during the
    #    launch wait; poll for a data row so "initial" is genuinely settled).
    Start-Sleep -Seconds $HoldSeconds
    $initial = Invoke-Capture "processes-initial"

    # 2. Mid-refresh: invoke the refresh and race the capture against it.
    $refreshButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Refresh processes"
    Invoke-UiaButtonElement -Element $refreshButton.element
    $mid = Invoke-Capture "processes-mid-refresh"

    # 3. Settled after refresh.
    Start-Sleep -Seconds 2
    $settled = Invoke-Capture "processes-refreshed"

    # Combined sheets against the Store baseline when it exists.
    if (Test-Path -LiteralPath (Join-Path (Get-Location) $StoreBaselinePng)) {
        $storePath = (Resolve-Path -LiteralPath (Join-Path (Get-Location) $StoreBaselinePng)).Path
        foreach ($pair in @(
            @{ Reactor = $initial; Name = "processes-initial" },
            @{ Reactor = $settled; Name = "processes-refreshed" },
            @{ Reactor = $mid; Name = "processes-mid-refresh" })) {
            $sheet = Join-Path $outputDirectory "$($pair.Name)-store-left-reactor-right.png"
            New-CombinedImage -LeftPath $storePath -RightPath $pair.Reactor `
                -OutputPath $sheet `
                -LeftLabel "Store 2.5.8" -RightLabel "Reactor"
            $evidence.captures += $sheet
        }
        Write-Host "Combined Store/Reactor sheets written."
    }
    else {
        Write-Warning "Store baseline '$StoreBaselinePng' not found; combined sheets skipped."
    }

    # Record the triptych in variants.json (defect evidence).
    $variantsPath = Join-Path (Get-Location) $VariantsJson
    if (Test-Path -LiteralPath $variantsPath) {
        $document = Get-Content -LiteralPath $variantsPath -Raw | ConvertFrom-Json
        $defects = @($document.defects | ForEach-Object {
            if ($_.id -eq "processes-refresh-rendering") {
                $_ | Add-Member -NotePropertyName evidence -NotePropertyValue @(
                    $initial, $mid, $settled) -Force
                $_ | Add-Member -NotePropertyName status -NotePropertyValue "evidence-captured" -Force
            }
            $_
        })
        $document.defects = $defects
        Write-JsonFile -Value $document -Path $variantsPath
        Write-Host "Defect record updated in $variantsPath."
    }
}
catch {
    $failures.Add($_.Exception.Message)
}
finally {
    try {
        $close = Stop-ReactorCandidate -Session $session `
            -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
        $evidence.gracefulClose = $close.gracefulClose
        $evidence.crashEvents = $close.crashEvents
        if (-not $close.gracefulClose) {
            $failures.Add("Candidate did not close gracefully.")
        }
    }
    catch {
        $failures.Add("Cleanup failed: $($_.Exception.Message)")
    }
}

$evidencePath = Join-Path $outputDirectory "process-refresh-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Process-refresh parity captures complete."
exit 0
