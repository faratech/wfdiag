# Reactor validation: AI scan report generation, regeneration, and provider tiers.
#
# Flow: run a live Quick Scan ("Quick Scan" automation button), wait for the
# completion status, switch to the AI page's Report mode, and generate the
# one-click report.
#
# Requires a configured provider. A no-provider status is a validation
# failure, never a successful report round-trip. CI uses the hermetic custom
# provider suite so this live-provider suite remains supplemental.
#
# Output: JSON evidence under -OutputDirectory. Exit 1 on failure.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "apps\wfdiag\captures-2.5.8\validation-report",
    [ValidateRange(30, 600)][int]$ScanWaitSeconds = 180,
    [ValidateRange(5, 300)][int]$ProviderWaitSeconds = 120
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
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-report-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "report"
    quickScan = $null
    tier0RoundTrip = $null
    providerTier = $null
    regenerateCheck = $null
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

    # --- Live Quick Scan ---------------------------------------------------
    $scanButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Quick Scan"
    Invoke-UiaButtonElement -Element $scanButton.element
    $scanStatus = Find-StatusText -Root $root -Deadline (Get-Date).AddSeconds($ScanWaitSeconds) `
        -AcceptedPrefix "Quick Scan complete"
    $evidence.quickScan = $scanStatus
    Write-Host $scanStatus

    # --- AI page, Report mode ----------------------------------------------
    $aiNav = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "AI Analysis"
    Invoke-UiaButtonElement -Element $aiNav.element
    Start-Sleep -Milliseconds 500
    $reportTab = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Scan Report"
    Invoke-UiaButtonElement -Element $reportTab.element
    Start-Sleep -Milliseconds 500

    $generateButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Generate report"
    Invoke-UiaButtonElement -Element $generateButton.element

    # --- Tier split ----------------------------------------------------------
    $noProviderStatus = "Set up an available AI provider before generating"
    $readyPrefix = "AI report ready"
    $statusDeadline = (Get-Date).AddSeconds($ProviderWaitSeconds)
    $observed = $null
    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name -ceq $noProviderStatus) {
                $observed = "no-provider"
                break
            }
            if ($name.StartsWith($readyPrefix, [StringComparison]::Ordinal)) {
                $observed = "ready"
                break
            }
        }
        if ($observed) { break }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $statusDeadline)

    if (-not $observed) {
        $failures.Add("Report generation produced neither the no-provider status nor a ready status within $ProviderWaitSeconds seconds.")
        $evidence.tier0RoundTrip = "failed"
    }
    elseif ($observed -eq "no-provider") {
        $evidence.tier0RoundTrip = "failed: no provider"
        $evidence.providerTier = "failed: no provider"
        $evidence.regenerateCheck = "failed: no provider"
        $failures.Add("Report validation requires an executable AI provider; the candidate reported no provider.")
    }
    else {
        $evidence.tier0RoundTrip = "passed"
        $evidence.providerTier = "passed"
        Write-Host "Report ready."

        # --- Force-refresh Regenerate ----------------------------------
        $regenerateButton = Wait-UniqueUiaButton -Root $root `
            -Deadline (Get-Date).AddSeconds(10) -Name "Regenerate report"
        Invoke-UiaButtonElement -Element $regenerateButton.element
        try {
            $regenerated = Find-StatusText -Root $root `
                -Deadline (Get-Date).AddSeconds($ProviderWaitSeconds) `
                -AcceptedPrefix "AI report ready"
            if ($regenerated -notlike "*cached*") {
                $evidence.regenerateCheck = "passed"
                Write-Host "Fresh report regeneration passed."
            }
            else {
                $evidence.regenerateCheck = "failed: '$regenerated' reused the cache"
                $failures.Add("Regenerate did not force a fresh report: '$regenerated'.")
            }
        }
        catch {
            $evidence.regenerateCheck = "failed: $($_.Exception.Message)"
            $failures.Add("Fresh report regeneration did not complete in time.")
        }
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
$evidencePath = Join-Path $outputDirectory "report-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Report validation passed."
exit 0
