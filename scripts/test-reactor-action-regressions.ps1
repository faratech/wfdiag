# Native Reactor regression gate for the reported action/export defects.
#
# This suite drives a real WinUI 3 candidate through UI Automation and covers:
#   * partial remediation disclosure collapse/expand persistence;
#   * empty/unsupported export-format fallback through the native save picker;
#   * the catalog-backed Device Manager action and its spawned window;
#   * optional, user-approved UAC relaunch handoff (never bypasses UAC).
#
# The candidate MUST be a validation build compiled with
# `--features validation`. Closed fixture names are ignored by normal
# production builds. Settings live under a GUID-scoped temporary directory and
# the export picker is cancelled, so no user configuration/report is written.
#
# Device Manager cleanup is handle-specific: the suite refuses to run that
# case if a Device Manager window already exists, and closes only the new
# window it observed. The optional elevation case does not kill a high-
# integrity child; if UIPI prevents graceful close it asks for manual cleanup.

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "apps\wfdiag\captures-2.5.8\validation-action-regressions",
    [ValidateRange(5, 120)][int]$StepWaitSeconds = 30,
    [switch]$IncludeAdminRelaunch,
    [ValidateRange(15, 180)][int]$AdminWaitSeconds = 75
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

if ($null -eq ("WfDiagActionRegressionNative" -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class WfDiagActionRegressionNative
{
    [StructLayout(LayoutKind.Sequential)]
    public struct TokenElevation
    {
        public int TokenIsElevated;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(
        uint desiredAccess,
        [MarshalAs(UnmanagedType.Bool)] bool inheritHandle,
        uint processId);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool QueryFullProcessImageName(
        IntPtr process,
        uint flags,
        StringBuilder path,
        ref uint size);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool OpenProcessToken(
        IntPtr processHandle,
        uint desiredAccess,
        out IntPtr tokenHandle);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetTokenInformation(
        IntPtr tokenHandle,
        int tokenInformationClass,
        out TokenElevation tokenInformation,
        uint tokenInformationLength,
        out uint returnLength);

    [DllImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool CloseHandle(IntPtr handle);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
}
'@
}

function Get-ReactorProbeDocument {
    param([Parameter(Mandatory = $true)][string]$Path)

    $probePath = Join-Path $env:TEMP (
        "wfdiag-action-regression-version-{0}.json" -f [Guid]::NewGuid().ToString("N"))
    $previous = [Environment]::GetEnvironmentVariable(
        "WFDIAG_REACTOR_VERSION_PROBE_FILE", "Process")
    try {
        [Environment]::SetEnvironmentVariable(
            "WFDIAG_REACTOR_VERSION_PROBE_FILE", $probePath, "Process")
        $probe = Start-Process -FilePath $Path -ArgumentList "--wfdiag-version-probe" `
            -Wait -PassThru
        if ($probe.ExitCode -ne 0) {
            throw "Version probe exited with code $($probe.ExitCode)."
        }
        if (-not (Test-Path -LiteralPath $probePath -PathType Leaf)) {
            throw "Version probe did not create '$probePath'."
        }
        return Get-Content -LiteralPath $probePath -Raw | ConvertFrom-Json
    }
    finally {
        [Environment]::SetEnvironmentVariable(
            "WFDIAG_REACTOR_VERSION_PROBE_FILE", $previous, "Process")
        Remove-Item -LiteralPath $probePath -Force -ErrorAction SilentlyContinue
    }
}

function Get-VisibleUiaElementByExactName {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            $bounds = $current.BoundingRectangle
            if ($current.Name -ceq $Name -and
                -not $current.IsOffscreen -and
                $bounds.Width -gt 0 -and $bounds.Height -gt 0) {
                return $element
            }
        }
        catch {
            continue
        }
    }
    return $null
}

function Wait-VisibleUiaElementByExactName {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $element = Get-VisibleUiaElementByExactName -Root $Root -Name $Name
        if ($null -ne $element) {
            return $element
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)
    throw "Visible UIA element '$Name' was not observed before the deadline."
}

function Get-PartialRunExpander {
    param([Parameter(Mandatory = $true)]$Root)

    $statusPrefix = "Remediation finished with partial results"
    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $candidate = $elements.Item($index)
        try {
            $pattern = [Windows.Automation.ExpandCollapsePattern]$candidate.GetCurrentPattern(
                [Windows.Automation.ExpandCollapsePattern]::Pattern)
            $descendants = $candidate.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            for ($childIndex = 0; $childIndex -lt $descendants.Count; $childIndex++) {
                $name = [string]$descendants.Item($childIndex).Current.Name
                if ($name.StartsWith($statusPrefix, [StringComparison]::Ordinal)) {
                    return [pscustomobject]@{
                        element = $candidate
                        pattern = $pattern
                        record = Get-UiaElementRecord -Element $candidate
                    }
                }
            }
        }
        catch {
            continue
        }
    }
    return $null
}

function Wait-PartialRunExpander {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $expander = Get-PartialRunExpander -Root $Root
        if ($null -ne $expander) {
            return $expander
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)
    throw "The partial remediation run Expander was not exposed through UI Automation."
}

function Get-SelectedComboName {
    param([Parameter(Mandatory = $true)]$Combo)

    try {
        $selection = [Windows.Automation.SelectionPattern]$Combo.GetCurrentPattern(
            [Windows.Automation.SelectionPattern]::Pattern)
        $selected = @($selection.Current.GetSelection())
        if ($selected.Count -eq 1) {
            return [string]$selected[0].Current.Name
        }
    }
    catch {
        return $null
    }
    return $null
}

function Wait-ExportFormatTextSelection {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $element = $elements.Item($index)
            try {
                $current = $element.Current
                if ($current.AutomationId -cne "Export format" -and
                    $current.Name -cne "Export format") {
                    continue
                }
                if ($current.ControlType -ne [Windows.Automation.ControlType]::ComboBox) {
                    continue
                }
                $selected = Get-SelectedComboName -Combo $element
                if ($selected -ceq "Text") {
                    return [pscustomobject]@{
                        combo = Get-UiaElementRecord -Element $element
                        selected = $selected
                    }
                }
            }
            catch {
                continue
            }
        }
        Start-Sleep -Milliseconds 125
    } while ((Get-Date) -lt $Deadline)
    throw "Export format did not normalize to the visible Text selection."
}

function Get-TopLevelWindowsByNamePattern {
    param([Parameter(Mandatory = $true)][string]$Pattern)

    $desktop = [Windows.Automation.AutomationElement]::RootElement
    $windows = $desktop.FindAll(
        [Windows.Automation.TreeScope]::Children,
        [Windows.Automation.Condition]::TrueCondition)
    $matches = @()
    for ($index = 0; $index -lt $windows.Count; $index++) {
        $window = $windows.Item($index)
        try {
            $current = $window.Current
            if ($current.Name -like $Pattern) {
                $matches += [pscustomobject]@{
                    element = $window
                    hwnd = [IntPtr]$current.NativeWindowHandle
                    processId = [int]$current.ProcessId
                    name = [string]$current.Name
                    record = Get-UiaElementRecord -Element $window
                }
            }
        }
        catch {
            continue
        }
    }
    return $matches
}

function Wait-NewTopLevelWindow {
    param(
        [Parameter(Mandatory = $true)][string]$NamePattern,
        [Parameter(Mandatory = $true)][int[]]$ExcludedHandles,
        [int]$RequiredProcessId = 0,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $matches = @(Get-TopLevelWindowsByNamePattern -Pattern $NamePattern | Where-Object {
            [int]$_.hwnd -notin $ExcludedHandles -and
            ($RequiredProcessId -eq 0 -or $_.processId -eq $RequiredProcessId)
        })
        if ($matches.Count -eq 1) {
            return $matches[0]
        }
        if ($matches.Count -gt 1) {
            throw "More than one new top-level window matched '$NamePattern'; refusing ambiguous cleanup."
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "No new top-level window matching '$NamePattern' appeared before the deadline."
}

function Close-ExactTopLevelWindow {
    param(
        [Parameter(Mandatory = $true)]$Window,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    try {
        $pattern = [Windows.Automation.WindowPattern]$Window.element.GetCurrentPattern(
            [Windows.Automation.WindowPattern]::Pattern)
        $pattern.Close()
    }
    catch {
        if ($Window.hwnd -eq [IntPtr]::Zero -or
            -not [WfDiagActionRegressionNative]::PostMessage(
                $Window.hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
            throw "Could not send WM_CLOSE to the exact observed window '$($Window.name)'."
        }
    }

    do {
        $stillOpen = @(Get-TopLevelWindowsByNamePattern -Pattern $Window.name | Where-Object {
            $_.hwnd -eq $Window.hwnd -and $_.processId -eq $Window.processId
        })
        if ($stillOpen.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 125
    } while ((Get-Date) -lt $Deadline)
    throw "The exact observed window '$($Window.name)' did not close in time."
}

function Get-ProcessImagePath {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    $handle = [WfDiagActionRegressionNative]::OpenProcess(0x1000, $false, [uint32]$ProcessId)
    if ($handle -eq [IntPtr]::Zero) {
        return $null
    }
    try {
        $capacity = [uint32]32768
        $builder = New-Object Text.StringBuilder ([int]$capacity)
        if (-not [WfDiagActionRegressionNative]::QueryFullProcessImageName(
            $handle, 0, $builder, [ref]$capacity)) {
            return $null
        }
        return $builder.ToString()
    }
    finally {
        [void][WfDiagActionRegressionNative]::CloseHandle($handle)
    }
}

function Get-ProcessIsElevated {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    $processHandle = [WfDiagActionRegressionNative]::OpenProcess(
        0x1000, $false, [uint32]$ProcessId)
    if ($processHandle -eq [IntPtr]::Zero) {
        throw "OpenProcess failed for PID $ProcessId."
    }
    $token = [IntPtr]::Zero
    try {
        if (-not [WfDiagActionRegressionNative]::OpenProcessToken(
            $processHandle, 0x0008, [ref]$token)) {
            throw "OpenProcessToken failed for PID $ProcessId."
        }
        $elevation = New-Object WfDiagActionRegressionNative+TokenElevation
        [uint32]$returnedLength = 0
        $size = [uint32][Runtime.InteropServices.Marshal]::SizeOf($elevation)
        if (-not [WfDiagActionRegressionNative]::GetTokenInformation(
            $token, 20, [ref]$elevation, $size, [ref]$returnedLength)) {
            throw "GetTokenInformation(TokenElevation) failed for PID $ProcessId."
        }
        return $elevation.TokenIsElevated -ne 0
    }
    finally {
        if ($token -ne [IntPtr]::Zero) {
            [void][WfDiagActionRegressionNative]::CloseHandle($token)
        }
        [void][WfDiagActionRegressionNative]::CloseHandle($processHandle)
    }
}

function Get-NewCandidateProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$OriginalProcessId
    )

    $baseName = [IO.Path]::GetFileNameWithoutExtension($Path)
    $candidates = @(Get-Process -Name $baseName -ErrorAction SilentlyContinue | Where-Object {
        $_.Id -ne $OriginalProcessId -and
        (Get-ProcessImagePath -ProcessId $_.Id) -ieq $Path
    })
    if ($candidates.Count -eq 1) {
        return $candidates[0]
    }
    if ($candidates.Count -gt 1) {
        throw "Multiple new candidate processes appeared; elevated handoff is ambiguous."
    }
    return $null
}

function Assert-NoExactCandidateAlreadyRunning {
    param([Parameter(Mandatory = $true)][string]$Path)

    $baseName = [IO.Path]::GetFileNameWithoutExtension($Path)
    $matches = @(Get-Process -Name $baseName -ErrorAction SilentlyContinue | Where-Object {
        (Get-ProcessImagePath -ProcessId $_.Id) -ieq $Path
    })
    if ($matches.Count -gt 0) {
        throw "Candidate is already running from '$Path'; close it before this exclusive suite."
    }
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$repoRoot = Split-Path -Parent $PSScriptRoot
$expectedVersion = [string]((Get-Content -LiteralPath (
    Join-Path $repoRoot "version.json") -Raw | ConvertFrom-Json).version)
$probe = Get-ReactorProbeDocument -Path $resolvedExecutable
if ($probe.schema -ne 1 -or [string]$probe.application_version -cne $expectedVersion) {
    throw "Candidate probe does not match repository version '$expectedVersion'."
}
if ($probe.settings_test_path -ne $true) {
    throw "This suite requires a validation candidate built with --features validation."
}
Assert-NoExactCandidateAlreadyRunning -Path $resolvedExecutable

$failures = [System.Collections.Generic.List[string]]::new()
$allCrashEvents = [System.Collections.Generic.List[object]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = [string]$probe.application_version
    suite = "action-regressions"
    settingsIsolation = "validation cargo feature; GUID-scoped temporary files"
    remediationDisclosure = $null
    exportFallback = @()
    deviceManager = $null
    adminRelaunch = [ordered]@{
        requested = [bool]$IncludeAdminRelaunch
        result = if ($IncludeAdminRelaunch) { "pending" } else { "not-run" }
        limitation = if ($IncludeAdminRelaunch) {
            "The suite observes UAC and process handoff but never drives the secure desktop. Approve the prompt manually."
        } else {
            "Pass -IncludeAdminRelaunch for an interactive UAC handoff test; UAC cannot be safely bypassed by automation."
        }
    }
    gracefulClose = @()
    crashEvents = $allCrashEvents
    failures = $failures
}

$temporaryRoot = Join-Path $env:TEMP (
    "wfdiag-action-regressions-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString("N"))
[IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null

try {
    # --- Partial remediation disclosure -----------------------------------
    $session = $null
    try {
        $session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 10 `
            -Variables @{
                WFDIAG_REACTOR_PAGE = "issues"
                WFDIAG_REACTOR_VISUAL_STATE = "remediation-partial"
                WFDIAG_REACTOR_WIDTH = "1440"
                WFDIAG_REACTOR_HEIGHT = "1000"
            }
        Assert-NoWebViewModules -Process $session.process
        $root = Get-ReactorUiaRoot -Process $session.process
        $expander = Wait-PartialRunExpander -Root $root `
            -Deadline (Get-Date).AddSeconds($StepWaitSeconds)
        $detailName = "Reset TCP/IP stack $([char]0x2014) Failed $([char]0x00B7) Access was denied while applying one step."
        $null = Wait-VisibleUiaElementByExactName -Root $root -Name $detailName `
            -Deadline (Get-Date).AddSeconds(5)

        $expander.pattern.Collapse()
        $collapseDeadline = (Get-Date).AddSeconds(5)
        do {
            $collapsedDetail = Get-VisibleUiaElementByExactName -Root $root -Name $detailName
            if ($null -eq $collapsedDetail) { break }
            Start-Sleep -Milliseconds 100
        } while ((Get-Date) -lt $collapseDeadline)
        if ($null -ne $collapsedDetail) {
            throw "Partial action detail remained visible after collapsing its run."
        }

        $expander = Wait-PartialRunExpander -Root $root `
            -Deadline (Get-Date).AddSeconds(5)
        $expander.pattern.Expand()
        $null = Wait-VisibleUiaElementByExactName -Root $root -Name $detailName `
            -Deadline (Get-Date).AddSeconds(5)

        $samples = @()
        for ($sampleIndex = 0; $sampleIndex -lt 30; $sampleIndex++) {
            $detail = Get-VisibleUiaElementByExactName -Root $root -Name $detailName
            if ($null -eq $detail) {
                throw "Partial action detail disappeared after expansion at sample $sampleIndex."
            }
            $samples += Get-UiaElementRecord -Element $detail
            Start-Sleep -Milliseconds 100
        }
        $evidence.remediationDisclosure = [ordered]@{
            result = "passed"
            expander = $expander.record
            collapsedDetailHidden = $true
            expandedSamples = $samples.Count
            observationMilliseconds = 3000
            detail = $samples[-1]
        }
    }
    catch {
        $failures.Add("partial-remediation: $($_.Exception.Message)")
        $evidence.remediationDisclosure = [ordered]@{
            result = "failed"
            error = $_.Exception.Message
        }
    }
    finally {
        if ($null -ne $session) {
            $closed = Stop-ReactorCandidate -Session $session `
                -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
            $evidence.gracefulClose += [ordered]@{
                case = "partial-remediation"
                graceful = $closed.gracefulClose
            }
            foreach ($crash in $closed.crashEvents) { $allCrashEvents.Add($crash) }
            if (-not $closed.gracefulClose) {
                $failures.Add("partial-remediation: candidate did not close gracefully.")
            }
        }
    }

    # --- Export fallback: empty and unsupported persisted values -----------
    foreach ($case in @(
        [pscustomobject]@{ name = "empty"; value = "" },
        [pscustomobject]@{ name = "unsupported"; value = "pdf" }
    )) {
        $session = $null
        $caseEvidence = [ordered]@{
            case = $case.name
            persistedValue = $case.value
            result = "pending"
        }
        try {
            $settingsPath = Join-Path $temporaryRoot ("settings-{0}.json" -f $case.name)
            $settingsJson = @{
                exportFormat = $case.value
                theme = "dark"
                showNotifications = $false
                scanOnStartup = $false
                preferredAIProvider = "auto"
            } | ConvertTo-Json -Depth 3
            [IO.File]::WriteAllText(
                $settingsPath, $settingsJson, (New-Object Text.UTF8Encoding($false)))

            $session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 10 `
                -Variables @{
                    WFDIAG_REACTOR_PAGE = "diagnostics"
                    WFDIAG_REACTOR_LIVE_TEST_FIXTURE = "export-fallback"
                    WFDIAG_REACTOR_SETTINGS_TEST_PATH = $settingsPath
                    WFDIAG_REACTOR_WIDTH = "1440"
                    WFDIAG_REACTOR_HEIGHT = "1000"
                }
            Assert-NoWebViewModules -Process $session.process
            $root = Get-ReactorUiaRoot -Process $session.process
            Invoke-UiaButtonByName -Root $root -Name "Open Settings" `
                -Deadline (Get-Date).AddSeconds(10) | Out-Null
            $selection = Wait-ExportFormatTextSelection -Root $root `
                -Deadline (Get-Date).AddSeconds($StepWaitSeconds)
            Invoke-UiaButtonByName -Root $root -Name "Close Settings" `
                -Deadline (Get-Date).AddSeconds(10) | Out-Null

            $beforeDialog = @(Get-TopLevelWindowsByNamePattern -Pattern "Export Diagnostic Report")
            Invoke-UiaButtonByName -Root $root -Name "Export Report" `
                -Deadline (Get-Date).AddSeconds(10) | Out-Null
            $dialog = Wait-NewTopLevelWindow -NamePattern "Export Diagnostic Report" `
                -ExcludedHandles @($beforeDialog | ForEach-Object { [int]$_.hwnd }) `
                -RequiredProcessId $session.process.Id `
                -Deadline (Get-Date).AddSeconds($StepWaitSeconds)

            $dialogDescendants = $dialog.element.FindAll(
                [Windows.Automation.TreeScope]::Descendants,
                [Windows.Automation.Condition]::TrueCondition)
            $textEvidence = @()
            for ($index = 0; $index -lt $dialogDescendants.Count; $index++) {
                $element = $dialogDescendants.Item($index)
                try {
                    $current = $element.Current
                    if (-not [string]::IsNullOrWhiteSpace([string]$current.Name)) {
                        $textEvidence += [string]$current.Name
                    }
                    try {
                        $value = [Windows.Automation.ValuePattern]$element.GetCurrentPattern(
                            [Windows.Automation.ValuePattern]::Pattern)
                        if (-not [string]::IsNullOrWhiteSpace($value.Current.Value)) {
                            $textEvidence += $value.Current.Value
                        }
                    }
                    catch {}
                }
                catch { continue }
            }
            if (-not ($textEvidence | Where-Object {
                $_ -match '(?i)(\.txt\b|\*\.txt|Text)'
            })) {
                throw "The export picker did not expose Text/.txt format evidence."
            }
            $cancel = Wait-UniqueUiaButton -Root $dialog.element `
                -Deadline (Get-Date).AddSeconds(5) -Name "Cancel"
            Invoke-UiaButtonElement -Element $cancel.element
            Start-Sleep -Milliseconds 500

            $unavailable = Find-UiaElementsByNamePrefix -Root $root `
                -Prefix "The selected export format is not available"
            if ($unavailable.Count -gt 0) {
                throw "The obsolete selected-format-unavailable error was rendered."
            }
            $caseEvidence.result = "passed"
            $caseEvidence.selection = $selection
            $caseEvidence.picker = $dialog.record
            $caseEvidence.textEvidence = @($textEvidence | Sort-Object -Unique)
        }
        catch {
            $failures.Add("export-$($case.name): $($_.Exception.Message)")
            $caseEvidence.result = "failed"
            $caseEvidence.error = $_.Exception.Message
        }
        finally {
            $evidence.exportFallback += $caseEvidence
            if ($null -ne $session) {
                $closed = Stop-ReactorCandidate -Session $session `
                    -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
                $evidence.gracefulClose += [ordered]@{
                    case = "export-$($case.name)"
                    graceful = $closed.gracefulClose
                }
                foreach ($crash in $closed.crashEvents) { $allCrashEvents.Add($crash) }
                if (-not $closed.gracefulClose) {
                    $failures.Add("export-$($case.name): candidate did not close gracefully.")
                }
            }
        }
    }

    # --- Device Manager catalog route -------------------------------------
    $session = $null
    $deviceWindow = $null
    try {
        $existingDeviceWindows = @(Get-TopLevelWindowsByNamePattern -Pattern "*Device Manager*")
        if ($existingDeviceWindows.Count -gt 0) {
            throw "A Device Manager window is already open; refusing an ambiguous launch/cleanup test."
        }
        $settingsPath = Join-Path $temporaryRoot "settings-device-manager.json"
        [IO.File]::WriteAllText(
            $settingsPath,
            '{"showNotifications":false,"scanOnStartup":false}',
            (New-Object Text.UTF8Encoding($false)))
        $session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 10 `
            -Variables @{
                WFDIAG_REACTOR_PAGE = "issues"
                WFDIAG_REACTOR_LIVE_TEST_FIXTURE = "device-manager"
                WFDIAG_REACTOR_SETTINGS_TEST_PATH = $settingsPath
                WFDIAG_REACTOR_WIDTH = "1440"
                WFDIAG_REACTOR_HEIGHT = "1000"
            }
        Assert-NoWebViewModules -Process $session.process
        $root = Get-ReactorUiaRoot -Process $session.process
        $deviceAction = Wait-UniqueUiaButton -Root $root `
            -Deadline (Get-Date).AddSeconds($StepWaitSeconds) `
            -Name "Run Open Device Manager" -AllowOffscreen
        Invoke-UiaButtonElement -Element $deviceAction.element
        Invoke-UiaButtonByName -Root $root -Name "Run once" `
            -Deadline (Get-Date).AddSeconds($StepWaitSeconds) | Out-Null

        $deviceWindow = Wait-NewTopLevelWindow -NamePattern "*Device Manager*" `
            -ExcludedHandles @() -Deadline (Get-Date).AddSeconds($StepWaitSeconds)
        $status = Wait-StatusText -Root $root -Deadline (Get-Date).AddSeconds(10) `
            -AcceptedPrefix "Tool opened"
        $evidence.deviceManager = [ordered]@{
            result = "passed"
            status = $status.matched
            launchedWindow = $deviceWindow.record
            cleanup = "exact new HWND only"
        }
    }
    catch {
        $failures.Add("device-manager: $($_.Exception.Message)")
        $evidence.deviceManager = [ordered]@{
            result = "failed"
            error = $_.Exception.Message
        }
    }
    finally {
        if ($null -ne $deviceWindow) {
            try {
                Close-ExactTopLevelWindow -Window $deviceWindow `
                    -Deadline (Get-Date).AddSeconds(10)
                $evidence.deviceManager.cleanupResult = "closed"
            }
            catch {
                $failures.Add("device-manager-cleanup: $($_.Exception.Message)")
                $evidence.deviceManager.cleanupResult = "failed"
                $evidence.deviceManager.cleanupError = $_.Exception.Message
            }
        }
        if ($null -ne $session) {
            $closed = Stop-ReactorCandidate -Session $session `
                -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
            $evidence.gracefulClose += [ordered]@{
                case = "device-manager"
                graceful = $closed.gracefulClose
            }
            foreach ($crash in $closed.crashEvents) { $allCrashEvents.Add($crash) }
            if (-not $closed.gracefulClose) {
                $failures.Add("device-manager: candidate did not close gracefully.")
            }
        }
    }

    # --- Optional interactive administrator handoff -----------------------
    if ($IncludeAdminRelaunch) {
        $session = $null
        $elevatedChild = $null
        try {
            $settingsPath = Join-Path $temporaryRoot "settings-admin.json"
            [IO.File]::WriteAllText(
                $settingsPath,
                '{"showNotifications":false,"scanOnStartup":false}',
                (New-Object Text.UTF8Encoding($false)))
            $session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 10 `
                -Variables @{
                    WFDIAG_REACTOR_PAGE = "issues"
                    WFDIAG_REACTOR_LIVE_TEST_FIXTURE = "admin-relaunch"
                    WFDIAG_REACTOR_SETTINGS_TEST_PATH = $settingsPath
                    WFDIAG_REACTOR_WIDTH = "1440"
                    WFDIAG_REACTOR_HEIGHT = "1000"
                }
            if (Get-ProcessIsElevated -ProcessId $session.process.Id) {
                throw "Initial candidate is already elevated; a split-token UAC handoff cannot be tested from this shell."
            }
            $root = Get-ReactorUiaRoot -Process $session.process
            Write-Host "ACTION REQUIRED: approve the WFDiag UAC prompt. The suite will not interact with the secure desktop."
            Invoke-UiaButtonByName -Root $root -Name "Run Restart as administrator" `
                -Deadline (Get-Date).AddSeconds($StepWaitSeconds) | Out-Null

            $deadline = (Get-Date).AddSeconds($AdminWaitSeconds)
            do {
                $session.process.Refresh()
                $elevatedChild = Get-NewCandidateProcess -Path $resolvedExecutable `
                    -OriginalProcessId $session.process.Id
                if ($null -ne $elevatedChild -and $session.process.HasExited) { break }
                if (-not $session.process.HasExited) {
                    $cancelled = Find-UiaElementsByNamePrefix -Root $root `
                        -Prefix "Administrator relaunch was cancelled"
                    if ($cancelled.Count -gt 0) {
                        throw "UAC was cancelled; successful elevated handoff was not validated."
                    }
                }
                Start-Sleep -Milliseconds 200
            } while ((Get-Date) -lt $deadline)
            if ($null -eq $elevatedChild -or -not $session.process.HasExited) {
                throw "The original process did not hand off to one new candidate before timeout."
            }

            $childWindowDeadline = (Get-Date).AddSeconds(20)
            do {
                $elevatedChild.Refresh()
                if ($elevatedChild.HasExited) {
                    throw "The elevated child exited before presenting a window."
                }
                if ($elevatedChild.MainWindowHandle -ne [IntPtr]::Zero) { break }
                Start-Sleep -Milliseconds 150
            } while ((Get-Date) -lt $childWindowDeadline)
            if ($elevatedChild.MainWindowHandle -eq [IntPtr]::Zero) {
                throw "The elevated child did not acquire a main window."
            }
            if (-not (Get-ProcessIsElevated -ProcessId $elevatedChild.Id)) {
                throw "The replacement candidate does not have an elevated token."
            }
            $evidence.adminRelaunch.result = "passed"
            $evidence.adminRelaunch.originalProcessId = $session.process.Id
            $evidence.adminRelaunch.elevatedProcessId = $elevatedChild.Id
            $evidence.adminRelaunch.originalExited = $true
            $evidence.adminRelaunch.elevatedToken = $true

            $closeRequested = $elevatedChild.CloseMainWindow()
            $childClosed = $closeRequested -and $elevatedChild.WaitForExit(5000)
            if (-not $childClosed) {
                Write-Host "MANUAL CLEANUP REQUIRED: close elevated WFDiag PID $($elevatedChild.Id)."
                $manualDeadline = (Get-Date).AddSeconds(20)
                do {
                    $elevatedChild.Refresh()
                    if ($elevatedChild.HasExited) { $childClosed = $true; break }
                    Start-Sleep -Milliseconds 250
                } while ((Get-Date) -lt $manualDeadline)
            }
            $evidence.adminRelaunch.elevatedChildGracefulClose = [bool]$childClosed
            if (-not $childClosed) {
                $evidence.adminRelaunch.manualCleanupRequired = $true
                $failures.Add(
                    "admin-relaunch: elevated child PID $($elevatedChild.Id) requires manual cleanup; it was not force-terminated.")
            }
        }
        catch {
            $failures.Add("admin-relaunch: $($_.Exception.Message)")
            $evidence.adminRelaunch.result = "failed"
            $evidence.adminRelaunch.error = $_.Exception.Message
        }
        finally {
            if ($null -ne $session) {
                $session.process.Refresh()
                if (-not $session.process.HasExited) {
                    $closed = Stop-ReactorCandidate -Session $session `
                        -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
                    $evidence.gracefulClose += [ordered]@{
                        case = "admin-original"
                        graceful = $closed.gracefulClose
                    }
                    foreach ($crash in $closed.crashEvents) { $allCrashEvents.Add($crash) }
                }
            }
        }
    }
}
finally {
    Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
}

if ($allCrashEvents.Count -gt 0) {
    $failures.Add("Application Error/WER events were recorded for the candidate.")
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$evidencePath = Join-Path $outputDirectory "action-regressions-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Action regression validation passed."
exit 0
