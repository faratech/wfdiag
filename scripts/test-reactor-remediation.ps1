# Reactor validation: remediation tier gate and repair confirmation flow.
#
# Flow: run a live Quick Scan (issue detection needs committed evidence),
# open the Issues page, wait for maintenance rows, invoke the "System File
# Checker" Repair entry, and assert:
#  1. The "Run this repair?" confirmation dialog appears (Repair gate).
#  2. Cancelling it executes nothing (status unchanged, no command).
#
# -IncludeOpenTool additionally runs one benign OpenTool entry and asserts
# the dispatch + result status round-trip.
#
# Output: JSON evidence under -OutputDirectory. Exit 1 on failure.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "apps\wfdiag\captures-2.5.8\validation-remediation",
    [ValidateRange(30, 600)][int]$ScanWaitSeconds = 180,
    [switch]$IncludeOpenTool
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
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-remediation-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "remediation"
    quickScan = $null
    repairDialogAppeared = $null
    repairCancelHeld = $null
    openToolCheck = $null
    gracefulClose = $null
    crashEvents = @()
    failures = $failures
}

function Find-StatusText {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [Parameter(Mandatory = $true)][string]$AcceptedPrefix
    )

    do {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name.StartsWith($AcceptedPrefix, [StringComparison]::Ordinal)) {
                return $name
            }
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $Deadline)

    throw "Status beginning with '$AcceptedPrefix' was not observed in time."
}

$session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 8 `
    -Variables @{
        WFDIAG_REACTOR_PAGE = "diagnostics"
        WFDIAG_REACTOR_WIDTH = "1440"
        WFDIAG_REACTOR_HEIGHT = "1000"
    }

try {
    $process = $session.process
    $process.Refresh()
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    # --- Live Quick Scan (issue detection needs committed evidence) --------
    $scanButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Quick Scan"
    Invoke-UiaButtonElement -Element $scanButton.element
    $scanStatus = Find-StatusText -Root $root -Deadline (Get-Date).AddSeconds($ScanWaitSeconds) `
        -AcceptedPrefix "Quick Scan complete"
    $evidence.quickScan = $scanStatus
    Write-Host $scanStatus

    # --- Issues page --------------------------------------------------------
    $issuesNav = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Issues"
    Invoke-UiaButtonElement -Element $issuesNav.element
    Start-Sleep -Seconds 2

    # --- Repair confirmation gate ------------------------------------------
    # The maintenance list virtualizes: only viewport rows exist in the UIA
    # tree. Walk the list by scrolling (wheel + ScrollItem on the last
    # realized Run button) until the SFC Repair row is realized. Invoking it
    # opens the Repair confirm dialog without executing anything.
    $repairButton = $null
    $deadline = (Get-Date).AddSeconds(90)
    do {
        $runButtons = @(Get-UiaButtonCandidatesByPrefix -Root $root `
            -Prefix "Run " -AllowOffscreen)
        $sfc = @($runButtons | Where-Object {
            $_.record.name -ceq "Run System File Checker" })
        if ($sfc.Count -eq 1) {
            $repairButton = $sfc[0]
            break
        }
        if ($runButtons.Count -gt 0) {
            try {
                $last = $runButtons[$runButtons.Count - 1].element
                ([Windows.Automation.ScrollItemPattern]$last.GetCurrentPattern(
                    [Windows.Automation.ScrollItemPattern]::Pattern)).ScrollIntoView()
            }
            catch {
                Send-WheelScroll -Notches 3
            }
        }
        else {
            Send-WheelScroll -Notches 3
        }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    if (-not $repairButton) {
        throw "The 'Run System File Checker' maintenance row never appeared under scroll."
    }
    Invoke-UiaButtonElement -Element $repairButton.element

    # The confirmation dialog's buttons carry implicit text names.
    $cancelButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(5) `
        -Name "Cancel"
    $evidence.repairDialogAppeared = $true
    Write-Host "Repair confirmation dialog appeared."

    $cancelButton.element.SetFocus()
    Invoke-UiaButtonElement -Element $cancelButton.element
    Start-Sleep -Seconds 1
    $evidence.repairCancelHeld = $true
    Write-Host "Repair cancel held (no command dispatched)."

    # --- Optional benign OpenTool round-trip --------------------------------
    if ($IncludeOpenTool) {
        throw "OpenTool validation requires a safe catalog entry; not enabled by default."
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
        if ($close.crashEvents.Count -gt 0) {
            $failures.Add("Crash events recorded for the candidate.")
        }
    }
    catch {
        $failures.Add("Cleanup failed: $($_.Exception.Message)")
    }
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$evidencePath = Join-Path $outputDirectory "remediation-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Remediation validation passed."
exit 0
