# Reactor validation: full AI flow against a hermetic mock provider.
#
# Starts a local OpenAI-compatible mock (scripts/lib/mock-provider.py),
# points the candidate at it through the validation settings file,
# and validates the REAL client paths end to end:
#   1. streaming chat round-trip (send -> SSE deltas -> complete status)
#   2. mid-stream cancel (Stop generating -> cancelled status)
#   3. exact-ten tool round-trip (mock verifies the closed schema, requests
#      list_remediations, and answers with a known native catalog ID)
#   4. report generation + forced regeneration
#
# Requires a candidate built with --features validation
# (validation builds only; never production artifacts).
#
# Output: JSON evidence under -OutputDirectory. Exit 1 on failure.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "apps\wfdiag\captures-2.5.8\validation-ai-flows",
    [ValidateRange(10, 120)][int]$StepWaitSeconds = 45,
    [ValidateRange(1, 65535)][int]$MockProviderPort = 18080
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$repoRoot = Split-Path -Parent $PSScriptRoot
$expectedVersion = [string]((Get-Content -LiteralPath (Join-Path $repoRoot "version.json") `
    -Raw | ConvertFrom-Json).version)
$version = Get-ReactorApplicationVersion -Executable $resolvedExecutable `
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-aiflows-version.json")
if ($version -ne $expectedVersion) {
    throw "Candidate version '$version' does not match repository version '$expectedVersion'."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "ai-flows"
    mockEndpoint = "http://127.0.0.1:$MockProviderPort"
    providerReady = $false
    streamingChat = $null
    cancelCheck = $null
    toolCheck = $null
    reportCheck = $null
    regenerateCheck = $null
    gracefulClose = $null
    crashEvents = @()
    lastUiText = @()
    failures = $failures
}

$session = $null
$mock = $null
$mockServerProcessId = $null
$root = $null
$settingsDirectory = Join-Path $env:TEMP ("wfdiag-aiflows-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString("N"))
$settingsPath = Join-Path $settingsDirectory "settings.json"
$mockScript = Join-Path $settingsDirectory "mock-provider.py"
$mockStdout = Join-Path $outputDirectory "mock-provider.stdout.log"
$mockStderr = Join-Path $outputDirectory "mock-provider.stderr.log"
$mockEndpoint = "http://127.0.0.1:$MockProviderPort"

try {
# --- Settings for the test-path candidate --------------------------------
New-Item -ItemType Directory -Path $settingsDirectory -Force | Out-Null
# UTF-8 WITHOUT BOM: the settings reader's serde_json rejects a BOM.
[System.IO.File]::WriteAllText($settingsPath,
    (@{
        aiEnabled = $true
        preferredAIProvider = "custom_openai"
        customEndpoint = $mockEndpoint
        customModel = "mock-model"
        theme = "dark"
        showNotifications = $false
        scanOnStartup = $false
    } | ConvertTo-Json -Depth 3),
    (New-Object System.Text.UTF8Encoding($false)))

# --- Mock provider ---------------------------------------------------------
# The Store Python alias cannot execute scripts from UNC paths; stage the
# mock in the machine-local temporary directory.
Copy-Item (Join-Path $PSScriptRoot "lib\mock-provider.py") $mockScript -Force
$occupiedListeners = @(Get-NetTCPConnection -State Listen -LocalPort $MockProviderPort `
    -ErrorAction SilentlyContinue)
if ($occupiedListeners.Count -gt 0) {
    $owners = @($occupiedListeners | Select-Object -ExpandProperty OwningProcess -Unique) -join ", "
    throw "Mock provider port $MockProviderPort is already occupied by process ID(s) $owners. Stop the stale listener or select another port."
}
$mock = Start-Process -FilePath "python" `
    -ArgumentList @("`"$mockScript`"", "--port", $MockProviderPort) `
    -RedirectStandardOutput $mockStdout -RedirectStandardError $mockStderr `
    -WindowStyle Hidden -PassThru
$mockReady = $false
$mockDeadline = (Get-Date).AddSeconds(15)
do {
    $mock.Refresh()
    if ($mock.HasExited) {
        $stderr = if (Test-Path -LiteralPath $mockStderr) {
            Get-Content -LiteralPath $mockStderr -Raw
        } else { "" }
        throw "Mock provider exited during startup with code $($mock.ExitCode): $stderr"
    }
    try {
        $probe = Invoke-WebRequest -Uri "$mockEndpoint/v1/models" -UseBasicParsing `
            -TimeoutSec 2
        $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $MockProviderPort `
            -ErrorAction SilentlyContinue)
        foreach ($listener in $listeners) {
            $owner = Get-CimInstance Win32_Process `
                -Filter "ProcessId = $($listener.OwningProcess)" -ErrorAction SilentlyContinue
            if ($null -ne $owner -and
                [string]$owner.CommandLine -like "*$mockScript*") {
                $mockServerProcessId = [int]$listener.OwningProcess
                break
            }
        }
        $mockReady = $probe.StatusCode -eq 200 -and $null -ne $mockServerProcessId
    }
    catch {
        Start-Sleep -Milliseconds 150
    }
} while (-not $mockReady -and (Get-Date) -lt $mockDeadline)
if (-not $mockReady) {
    throw "Mock provider did not become ready at $mockEndpoint."
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
        [Parameter(Mandatory = $true)][string]$Prefix,
        [string]$RequiredSubstring
    )

    do {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ($name.StartsWith($Prefix, [StringComparison]::Ordinal) -and
                (-not $RequiredSubstring -or
                 $name.IndexOf($RequiredSubstring, [StringComparison]::Ordinal) -ge 0)) {
                return $name
            }
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $Deadline)

    $requirement = if ($RequiredSubstring) {
        " containing '$RequiredSubstring'"
    } else { "" }
    throw "UI text beginning with '$Prefix'$requirement was not observed in time."
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
            if ($name.StartsWith("AI provider ready", [StringComparison]::Ordinal) -and
                $name.IndexOf("custom_openai", [StringComparison]::Ordinal) -ge 0) {
                $ready = $true
                break
            }
        }
        if ($ready) { break }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $providerReadyDeadline)
    if (-not $ready) {
        throw "The hermetic custom provider did not become ready within 30 seconds; no-provider is not a passing state."
    }
    $evidence.providerReady = $true

    # --- 1. Streaming chat round-trip ---------------------------------------
    # `hello` is an exact scan-free greeting in the app. Using `hello there`
    # would intentionally launch the prerequisite Quick Scan before chat.
    Send-ComposerText -Root $root -Value "hello"
    $complete = Wait-StatusPrefix -Root $root `
        -Deadline (Get-Date).AddSeconds($StepWaitSeconds) `
        -Prefix "AI response complete"
    $chatPassed = $true
    if ($complete -notlike "*custom*") {
        $failures.Add("Chat completed with unexpected provider attribution: '$complete'.")
        $chatPassed = $false
    }
    try {
        $null = Wait-StatusPrefix -Root $root `
            -Deadline (Get-Date).AddSeconds(10) -Prefix "MOCK_REPLY"
    }
    catch {
        $failures.Add("Streaming answer text was not rendered in the chat area.")
        $chatPassed = $false
    }
    $evidence.streamingChat = if ($chatPassed) { "passed" } else { "failed" }

    # --- 2. Mid-stream cancel ------------------------------------------------
    # Keep this prompt in the app's scan-free `write a ...` class so the
    # cancellation check does not consume the report phase's explicit scan.
    Send-ComposerText -Root $root -Value "Write a slow greeting."
    Start-Sleep -Seconds 3
    $stop = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(5) `
        -Name "Stop generating"
    Invoke-UiaButtonElement -Element $stop.element
    $null = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(15) `
        -Prefix "AI response cancelled"
    $evidence.cancelCheck = "passed"

    # --- 3. Tool round-trip ---------------------------------------------------
    # Likewise keep the read-only tool-contract check scan-free. The mock only
    # needs the word `tool`; the report phase validates the real Quick Scan.
    Send-ComposerText -Root $root -Value "Write a tool-contract reply that lists vetted remediations."
    try {
        $null = Wait-StatusPrefix -Root $root `
            -Deadline (Get-Date).AddSeconds($StepWaitSeconds + 15) `
            -Prefix "MOCK_TOOL_REPLY" -RequiredSubstring "open_disk_cleanup"
        $evidence.toolCheck = "passed"
    }
    catch {
        $failures.Add("The exact-ten tool round-trip did not return the native 'open_disk_cleanup' catalog ID.")
        $evidence.toolCheck = "failed"
    }

    # --- 4. Report generation + forced regeneration ----------------------------
    $diagnosticsPage = Wait-UniqueUiaButton -Root $root `
        -Deadline (Get-Date).AddSeconds(10) -Name "Diagnostics"
    Invoke-UiaButtonElement -Element $diagnosticsPage.element
    $scan = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Quick Scan"
    Invoke-UiaButtonElement -Element $scan.element
    $null = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(180) `
        -Prefix "Quick Scan complete"

    $aiPage = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "AI Analysis"
    Invoke-UiaButtonElement -Element $aiPage.element
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
    $reportBody = $null
    try {
        $reportBody = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(10) `
            -Prefix "MOCK_REPLY" -RequiredSubstring "Scan data:"
        $evidence.reportCheck = "passed"
    }
    catch {
        $failures.Add("The streamed report reached a ready status without rendering its mock-provider body.")
        $evidence.reportCheck = "failed"
    }

    $postsBeforeRegenerate = @(Select-String -LiteralPath $mockStdout `
        -Pattern '^POST /v1/chat/completions ' -ErrorAction SilentlyContinue).Count
    $regenerate = Wait-UniqueUiaButton -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Name "Regenerate report"
    Invoke-UiaButtonElement -Element $regenerate.element
    # Do not let the previous ready status and report body satisfy this step.
    # Observing the new preparing state proves the click was consumed; the POST
    # count below proves force-refresh crossed the provider boundary.
    $null = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(10) `
        -Prefix "Preparing AI report"
    $regenerated = Wait-StatusPrefix -Root $root `
        -Deadline (Get-Date).AddSeconds($StepWaitSeconds) `
        -Prefix "AI report ready"
    if ($regenerated -like "*cached*") {
        $failures.Add("Regenerate unexpectedly reused the cache instead of forcing a fresh report.")
    }
    $postsAfterRegenerate = @(Select-String -LiteralPath $mockStdout `
        -Pattern '^POST /v1/chat/completions ' -ErrorAction SilentlyContinue).Count
    if ($postsAfterRegenerate -le $postsBeforeRegenerate) {
        $failures.Add("Regenerate reached a ready status without making a fresh provider request.")
    }
    try {
        $regeneratedBody = Wait-StatusPrefix -Root $root -Deadline (Get-Date).AddSeconds(10) `
            -Prefix "MOCK_REPLY" -RequiredSubstring "Scan data:"
        if ($null -eq $reportBody -or $regeneratedBody -cne $reportBody) {
            throw "regenerated report body differed from the first deterministic report body"
        }
        $evidence.regenerateCheck = "passed"
    }
    catch {
        $failures.Add("Forced regeneration did not preserve the complete deterministic report body: $($_.Exception.Message)")
        $evidence.regenerateCheck = "failed"
    }
}
catch {
    if ($null -ne $root) {
        try {
            $visibleText = [System.Collections.Generic.List[string]]::new()
            $elements = $root.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            for ($index = 0; $index -lt $elements.Count; $index++) {
                $name = $null
                try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
                if ($name -and -not $visibleText.Contains($name)) {
                    $visibleText.Add($name)
                }
            }
            $evidence.lastUiText = @($visibleText)
        }
        catch {
        }
    }
    $failures.Add($_.Exception.Message)
}
finally {
    try {
        if ($null -eq $session) {
            throw "Candidate did not start; graceful-close validation was unavailable."
        }
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
        $failures.Add("Candidate cleanup failed: $($_.Exception.Message)")
    }
    try {
        if ($null -ne $mockServerProcessId) {
            Stop-Process -Id $mockServerProcessId -Force -ErrorAction SilentlyContinue
        }
        if ($null -ne $mock -and -not $mock.HasExited) {
            Stop-Process -Id $mock.Id -Force -ErrorAction SilentlyContinue
        }
    }
    catch {
    }
    Remove-Item -LiteralPath $settingsDirectory -Recurse -Force -ErrorAction SilentlyContinue
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
