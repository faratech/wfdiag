# Reactor validation: full AI flow against a hermetic mock provider.
#
# Starts a local OpenAI-compatible mock (scripts/lib/mock-provider.py),
# points the candidate at it through the settings-test-path settings file,
# and validates the REAL client paths end to end:
#   1. streaming chat round-trip (send -> SSE deltas -> complete status)
#   2. mid-stream cancel (Stop generating -> cancelled status)
#   3. tool round-trip (mock requests get_system_overview, answers with the
#      machine name from the injected snapshot)
#   4. report generation + cached regenerate
#
# Requires a candidate built with --features settings-test-path
# (validation builds only; never production artifacts).
#
# Output: JSON evidence under -OutputDirectory. Exit 1 on failure.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\validation-ai-flows",
    [ValidateRange(10, 120)][int]$StepWaitSeconds = 45
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
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-aiflows-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "ai-flows"
    streamingChat = $null
    cancelCheck = $null
    toolCheck = $null
    reportCheck = $null
    cachedCheck = $null
    gracefulClose = $null
    crashEvents = @()
    failures = $failures
}

# --- Settings for the test-path candidate --------------------------------
$settingsDirectory = Join-Path $env:TEMP "wfdiag-aiflows-settings"
if (-not (Test-Path -LiteralPath $settingsDirectory)) {
    New-Item -ItemType Directory -Path $settingsDirectory -Force | Out-Null
}
$settingsPath = Join-Path $settingsDirectory "settings.json"
# UTF-8 WITHOUT BOM: the settings reader's serde_json rejects a BOM.
[System.IO.File]::WriteAllText($settingsPath,
    (@{
        aiEnabled = $true
        preferredAiProvider = "custom_openai"
        customEndpoint = "http://127.0.0.1:18080"
        customModel = "mock-model"
        theme = "dark"
        showNotifications = $false
        scanOnStartup = $false
    } | ConvertTo-Json -Depth 3),
    (New-Object System.Text.UTF8Encoding($false)))

# --- Mock provider ---------------------------------------------------------
# The Store python alias cannot execute scripts from UNC paths; stage the
# mock next to the candidate executable (always on a local drive).
$mockScript = Join-Path (Split-Path -Parent $resolvedExecutable) "mock-provider.py"
Copy-Item (Join-Path $PSScriptRoot "lib\mock-provider.py") $mockScript -Force
$mock = Start-Process -FilePath "python" `
    -ArgumentList "`"$mockScript`"" `
    -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 1
if ($mock.HasExited) {
    throw "Mock provider failed to start."
}

$session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 10 -Variables @{
    WFDIAG_REACTOR_PAGE = "ai"
    WFDIAG_REACTOR_WIDTH = "1200"
    WFDIAG_REACTOR_HEIGHT = "900"
    WFDIAG_REACTOR_SETTINGS_TEST_PATH = $settingsPath
}

function Wait-StatusPrefix {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [Parameter(Mandatory = $true)][string]$Prefix
    )

    do {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name.StartsWith($Prefix, [StringComparison]::Ordinal)) {
                return $name
            }
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $Deadline)

    throw "Status beginning with '$Prefix' not observed in time."
}

function Send-ComposerText {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Value
    )

    Set-UiaTextValue -Root $Root -AutomationName "Chat message" `
        -Value $Value -Deadline (Get-Date).AddSeconds(10)
    $send = Wait-UniqueUiaButton -Root $Root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Send chat message"
    Invoke-UiaButtonElement -Element $send.element
}

try {
    $process = $session.process
    $process.Refresh()
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    # Wait for the async provider probe to publish the ready pill (the same
    # signal a user waits for before sending).
    $providerReadyDeadline = (Get-Date).AddSeconds(30)
    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        $ready = $false
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name.StartsWith("AI provider ready", [StringComparison]::Ordinal)) {
                $ready = $true
                break
            }
        }
        if ($ready) { break }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $providerReadyDeadline)

    # --- 1. Streaming chat round-trip ---------------------------------------
    Send-ComposerText -Root $root -Value "hello there"
    $complete = Wait-StatusPrefix -Root $root `
        -Deadline (Get-Date).AddSeconds($StepWaitSeconds) `
        -Prefix "AI response complete"
    if ($complete -notlike "*custom*") {
        $failures.Add("Chat completed with unexpected provider attribution: '$complete'.")
    }
    $answerText = $null
    $elements = $root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $name = $null
        try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
        if ($name -like "MOCK_REPLY*") {
            $answerText = $name
            break
        }
    }
    if ($null -eq $answerText) {
        $failures.Add("Streaming answer text was not rendered in the chat area.")
    }
    $evidence.streamingChat = "passed"

    # --- 2. Mid-stream cancel ------------------------------------------------
    Send-ComposerText -Root $root -Value "tell me something slow"
    Start-Sleep -Seconds 3
    $stop = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(5) `
        -Name "Stop generating"
    Invoke-UiaButtonElement -Element $stop.element
    $null = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(15) `
        -Prefix "AI response cancelled"
    $evidence.cancelCheck = "passed"

    # --- 3. Tool round-trip ---------------------------------------------------
    Send-ComposerText -Root $root -Value "What hardware am I running? Use the overview tool."
    $toolDeadline = (Get-Date).AddSeconds($StepWaitSeconds + 15)
    $machineAnswered = $false
    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name -like "MOCK_TOOL_REPLY*") {
                $machineAnswered = $true
                break
            }
        }
        if ($machineAnswered) { break }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $toolDeadline)

    if ($machineAnswered) {
        $evidence.toolCheck = "passed"
    }
    else {
        $failures.Add("Tool round-trip did not produce the machine-grounded answer.")
        $evidence.toolCheck = "failed"
    }

    # --- 4. Report generation + cached regenerate -----------------------------
    $scan = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Quick Scan"
    Invoke-UiaButtonElement -Element $scan.element
    $null = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(180) `
        -Prefix "Quick Scan complete"

    $reportTab = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Scan Report"
    Invoke-UiaButtonElement -Element $reportTab.element
    Start-Sleep -Milliseconds 500
    $generate = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Generate report"
    Invoke-UiaButtonElement -Element $generate.element
    $ready = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds($StepWaitSeconds + 30) `
        -Prefix "AI report ready"
    if ($ready -like "*cached*") {
        $failures.Add("First report generation unexpectedly hit the cache.")
    }
    $evidence.reportCheck = "passed"

    $regenerate = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Regenerate report"
    Invoke-UiaButtonElement -Element $regenerate.element
    $cachedReady = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds($StepWaitSeconds) `
        -Prefix "AI report ready"
    if ($cachedReady -like "*cached*") {
        $evidence.cachedCheck = "passed"
    }
    else {
        $failures.Add("Regenerate did not hit the cache.")
        $evidence.cachedCheck = "failed"
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
    try {
        if ($null -ne $mock -and -not $mock.HasExited) {
            Stop-Process -Id $mock.Id -Force -ErrorAction SilentlyContinue
        }
    }
    catch {
    }
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$evidencePath = Join-Path $outputDirectory "ai-flows-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "AI flows validation passed."
exit 0
