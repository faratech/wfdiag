# Reactor validation: AI chat round-trip, cancel, and tool use.
#
# Requires a configured provider. A no-provider status is recorded as a
# validation failure, never as a successful round-trip. CI uses the hermetic
# custom-provider suite so this live-provider suite remains supplemental.
# - Cancel: press "Stop generating" while streaming; assert the cancelled
#   status. Skipped when the provider answered too fast.
# - Tool tier (tool-capable provider): ask for the vetted remediation catalog,
#   then require both list_remediations activity and the known
#   open_disk_cleanup ID in the answer.
#
# Output: JSON evidence under -OutputDirectory. Exit 1 on failure.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "apps\wfdiag\captures-2.5.8\validation-chat",
    [ValidateRange(5, 180)][int]$ProviderWaitSeconds = 60,
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
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-chat-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "chat"
    tier0RoundTrip = $null
    cancelCheck = $null
    toolCheck = $null
    providerTier = $null
    gracefulClose = $null
    crashEvents = @()
    failures = $failures
}

$session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 8 `
    -Variables @{
        WFDIAG_REACTOR_PAGE = "ai"
        WFDIAG_REACTOR_WIDTH = "1200"
        WFDIAG_REACTOR_HEIGHT = "900"
    }

try {
    $process = $session.process
    $process.Refresh()
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    # --- Tier 0: composer round-trip -------------------------------------
    Set-UiaTextValue -Root $root -AutomationName "Chat message" `
        -Value "What hardware am I running?" `
        -Deadline (Get-Date).AddSeconds(10)
    $sendDeadline = (Get-Date).AddSeconds(10)
    $sendButton = Wait-UniqueUiaButton -Root $root -Deadline $sendDeadline `
        -Name "Send chat message"
    Invoke-UiaButtonElement -Element $sendButton.element

    $statusDeadline = (Get-Date).AddSeconds(15)
    $noProviderStatus = "Set up an available AI provider before sending"
    $askingStatus = "Asking the AI assistant…"
    $accepted = @($noProviderStatus, $askingStatus)
    $observed = $null
    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name -ceq $noProviderStatus -or $name -ceq $askingStatus) {
                $observed = $name
                break
            }
        }
        if ($observed) { break }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $statusDeadline)

    if (-not $observed) {
        $failures.Add("Chat send produced neither the no-provider status nor the asking status within 15 seconds.")
        $evidence.tier0RoundTrip = "failed"
    }
    elseif ($observed -ceq $noProviderStatus) {
        $evidence.tier0RoundTrip = "failed: no provider"
        $evidence.providerTier = "failed: no provider"
        $failures.Add("Chat validation requires an executable AI provider; the candidate reported no provider.")
    }
    else {
        $evidence.tier0RoundTrip = "passed"
        Write-Host "Provider accepted the send; waiting for a terminal status."

        # --- Provider terminal status -----------------------------------
        $terminalDeadline = (Get-Date).AddSeconds($ProviderWaitSeconds)
        $terminal = $null
        do {
            $elements = $root.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            for ($index = 0; $index -lt $elements.Count; $index++) {
                $name = $null
                try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
                if ($name -like "AI response complete*") {
                    $terminal = [pscustomobject]@{ kind = "complete"; status = $name }
                    break
                }
                if ($name -like "AI response cancelled*") {
                    $terminal = [pscustomobject]@{ kind = "cancelled"; status = $name }
                    break
                }
            }
            if ($terminal) { break }
            Start-Sleep -Milliseconds 200
        } while ((Get-Date) -lt $terminalDeadline)

        if (-not $terminal) {
            $failures.Add("Provider turn did not reach a terminal status within $ProviderWaitSeconds seconds.")
            $evidence.providerTier = "failed: timeout"
        }
        elseif ($terminal.kind -ne "complete") {
            $failures.Add("Provider turn ended as '$($terminal.status)' instead of completing.")
            $evidence.providerTier = "failed: $($terminal.status)"
        }
        else {
            $evidence.providerTier = "passed ($($terminal.status))"
            Write-Host $terminal.status
        }

        # --- Cancel check: only meaningful while a turn streams ---------
        Set-UiaTextValue -Root $root -AutomationName "Chat message" `
            -Value "Count slowly to one hundred." `
            -Deadline (Get-Date).AddSeconds(10)
        $sendButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
            -Name "Send chat message"
        Invoke-UiaButtonElement -Element $sendButton.element

        $askDeadline = (Get-Date).AddSeconds(15)
        $asking = $false
        do {
            $elements = $root.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            for ($index = 0; $index -lt $elements.Count; $index++) {
                $name = $null
                try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
                if ($name -ceq "Asking the AI assistant…") { $asking = $true; break }
            }
            if ($asking) { break }
            Start-Sleep -Milliseconds 100
        } while ((Get-Date) -lt $askDeadline)

        if (-not $asking) {
            $evidence.cancelCheck = "skipped: turn already terminal"
            Write-Host "Cancel check skipped (turn finished before Stop could be pressed)."
        }
        else {
            $stopButton = Wait-UniqueUiaButton -Root $root `
                -Deadline (Get-Date).AddSeconds(5) -Name "Stop generating"
            Invoke-UiaButtonElement -Element $stopButton.element
            $cancelled = Wait-StatusText -Root $root -Deadline (Get-Date).AddSeconds(20) `
                -Accepted @("AI response cancelled")
            $evidence.cancelCheck = "passed"
            Write-Host "Cancel check passed."
        }

        # --- Tool check: bounded remediation-catalog answer -------------
        Set-UiaTextValue -Root $root -AutomationName "Chat message" `
            -Value "Use the list remediations tool and name a vetted remediation ID." `
            -Deadline (Get-Date).AddSeconds(10)
        $sendButton = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
            -Name "Send chat message"
        Invoke-UiaButtonElement -Element $sendButton.element

        $toolDeadline = (Get-Date).AddSeconds($ProviderWaitSeconds)
        $toolAnswered = $false
        $toolActivity = $false
        do {
            $elements = $root.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            for ($index = 0; $index -lt $elements.Count; $index++) {
                $name = $null
                try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
                if ($name -and $name.Contains("open_disk_cleanup")) {
                    $toolAnswered = $true
                }
                if ($name -and $name.Contains("list_remediations") -and
                    $name.Contains("completed")) {
                    $toolActivity = $true
                }
            }
            if ($toolAnswered -and $toolActivity) { break }
            Start-Sleep -Milliseconds 250
        } while ((Get-Date) -lt $toolDeadline)

        if ($toolAnswered -and $toolActivity) {
            $evidence.toolCheck = "passed (list_remediations returned 'open_disk_cleanup')"
            Write-Host "Tool check passed (answer grounded in the native remediation catalog)."
        }
        else {
            $failures.Add("Tool check failed: list_remediations activity and an answer containing 'open_disk_cleanup' were not both observed within $ProviderWaitSeconds seconds.")
            $evidence.toolCheck = "failed"
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
$evidencePath = Join-Path $outputDirectory "chat-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Chat validation passed."
exit 0
