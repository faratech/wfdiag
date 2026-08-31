# Shared UIA + process helper module for the Reactor validation suites.
#
# Extracted from scripts/test-reactor-about-parity.ps1 and
# scripts/test-reactor-live-system.ps1 so the per-feature validation scripts
# (chat/report/remediation/x64) drive the candidate identically. The function
# bodies are intentionally unchanged — fix them at the source scripts if the
# underlying behavior drifts, then re-verify all suites.

Set-StrictMode -Version Latest

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing

$script:ReactorEnvVars = @(
    "WFDIAG_REACTOR_PAGE",
    "WFDIAG_REACTOR_VISUAL_STATE",
    "WFDIAG_REACTOR_FIXTURE",
    "WFDIAG_REACTOR_SETTINGS",
    "WFDIAG_REACTOR_WIDTH",
    "WFDIAG_REACTOR_HEIGHT",
    "WFDIAG_REACTOR_THEME"
)

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $Value | ConvertTo-Json -Depth 10 |
        Set-Content -LiteralPath $Path -Encoding UTF8
}

function Get-ReactorApplicationVersion {
    param([Parameter(Mandatory = $true)][string]$Executable,
          [Parameter(Mandatory = $true)][string]$ProbeFile)

    $env:WFDIAG_REACTOR_VERSION_PROBE_FILE = $ProbeFile
    try {
        # The candidate is a GUI-subsystem executable: PowerShell does not
        # populate $LASTEXITCODE for it, so read the exit code off the
        # process object instead.
        $probe = Start-Process -FilePath $Executable `
            -ArgumentList "--wfdiag-version-probe" -Wait -PassThru
        if ($probe.ExitCode -ne 0) {
            throw "Version probe exit code $($probe.ExitCode)."
        }
        $document = Get-Content -LiteralPath $ProbeFile -Raw | ConvertFrom-Json
        if ($document.schema -ne 1 -or
            -not $document.application_version) {
            throw "Version probe document is malformed."
        }
        return [string]$document.application_version
    }
    finally {
        Remove-Item Env:\WFDIAG_REACTOR_VERSION_PROBE_FILE -ErrorAction SilentlyContinue
    }
}

<#
.SYNOPSIS
Launch the candidate with a hermetic Reactor environment.
.EXAMPLE
$session = Start-ReactorCandidate -Executable $exe -Seconds 2 -Variables @{ WFDIAG_REACTOR_PAGE = "ai" }
... UIA work ...
Stop-ReactorCandidate $session
#>
function Start-ReactorCandidate {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [hashtable]$Variables = @{},
        [ValidateRange(0, 60)][int]$Seconds = 4
    )

    $saved = @{}
    foreach ($name in $script:ReactorEnvVars) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
        [Environment]::SetEnvironmentVariable($name, $null, "Process")
    }
    foreach ($key in $Variables.Keys) {
        [Environment]::SetEnvironmentVariable($key, [string]$Variables[$key], "Process")
    }

    $process = Start-Process -FilePath $Executable -PassThru
    $null = $process.WaitForExit(0)

    foreach ($name in $script:ReactorEnvVars) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process")
    }

    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $process.Refresh()
        if ($process.HasExited) {
            throw "Candidate PID $($process.Id) exited during startup with code $($process.ExitCode)."
        }
        if ($process.MainWindowHandle -ne 0) {
            break
        }
        Start-Sleep -Milliseconds 150
    }
    $process.Refresh()
    if ($process.MainWindowHandle -eq 0) {
        throw "Candidate did not acquire a main window within $Seconds seconds."
    }

    return [pscustomobject]@{
        process = $process
        startedAt = Get-Date
    }
}

function Stop-ReactorCandidate {
    param(
        [Parameter(Mandatory = $true)][pscustomobject]$Session,
        [Parameter(Mandatory = $true)][string[]]$ExecutablePaths,
        [ValidateRange(1, 60)][int]$GraceSeconds = 5
    )

    $crashes = @(Get-CrashEvents -ExecutablePaths $ExecutablePaths -StartTime $Session.startedAt)
    $process = $Session.process
    $process.Refresh()
    if ($process.HasExited) {
        return [pscustomobject]@{
            gracefulClose = ($process.ExitCode -eq 0)
            exitCode = $process.ExitCode
            crashEvents = $crashes
        }
    }

    $null = $process.CloseMainWindow()
    $graceful = $process.WaitForExit($GraceSeconds * 1000)
    if (-not $graceful) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    return [pscustomobject]@{
        gracefulClose = [bool]$graceful
        exitCode = $null
        crashEvents = $crashes
    }
}

function Get-UiaElementRecord {
    param($Element)

    if ($null -eq $Element) {
        return $null
    }
    try {
        $current = $Element.Current
        $bounds = $current.BoundingRectangle
        $runtimeId = @()
        try {
            $runtimeId = @($Element.GetRuntimeId())
        }
        catch {
            $runtimeId = @()
        }
        return [pscustomobject]@{
            unavailable = $false
            error = $null
            name = [string]$current.Name
            controlType = [string]$current.ControlType.ProgrammaticName
            localizedControlType = [string]$current.LocalizedControlType
            automationId = [string]$current.AutomationId
            className = [string]$current.ClassName
            frameworkId = [string]$current.FrameworkId
            processId = [int]$current.ProcessId
            isEnabled = [bool]$current.IsEnabled
            isOffscreen = [bool]$current.IsOffscreen
            runtimeId = $runtimeId
            bounds = [pscustomobject]@{
                x = [Math]::Round($bounds.X, 2)
                y = [Math]::Round($bounds.Y, 2)
                width = [Math]::Round($bounds.Width, 2)
                height = [Math]::Round($bounds.Height, 2)
            }
        }
    }
    catch {
        return [pscustomobject]@{
            unavailable = $true
            error = $_.Exception.Message
            name = $null
            controlType = $null
            localizedControlType = $null
            automationId = $null
            className = $null
            frameworkId = $null
            processId = $null
            isEnabled = $null
            isOffscreen = $null
            runtimeId = @()
            bounds = $null
        }
    }
}

function Get-ReactorUiaRoot {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    $process.Refresh()
    if ($process.MainWindowHandle -eq 0) {
        throw "Candidate has no main window handle."
    }
    return [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
}

function Get-UiaButtonCandidates {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [string]$Name,
        [string]$AutomationId,
        [switch]$AllowOffscreen
    )

    $matchName = $PSBoundParameters.ContainsKey("Name")
    $matchAutomationId = $PSBoundParameters.ContainsKey("AutomationId")
    if ($matchName -eq $matchAutomationId) {
        throw "Specify exactly one of Name or AutomationId for a UIA button lookup."
    }

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $candidates = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            if ($current.ControlType -ne [Windows.Automation.ControlType]::Button -or
                -not $current.IsEnabled -or
                ($matchName -and $current.Name -cne $Name) -or
                ($matchAutomationId -and $current.AutomationId -cne $AutomationId)) {
                continue
            }
            if ($current.IsOffscreen) {
                if (-not $AllowOffscreen) {
                    continue
                }
                # Scroll virtualized/off-viewport rows into view so Invoke
                # reaches a realized element.
                try {
                    $scrollItem = $element.GetCurrentPattern(
                        [Windows.Automation.ScrollItemPattern]::Pattern)
                    $scrollItem.ScrollIntoView()
                    Start-Sleep -Milliseconds 200
                    $current = $element.Current
                }
                catch {
                    continue
                }
            }
            [void]$element.GetCurrentPattern(
                [Windows.Automation.InvokePattern]::Pattern)
            $candidates += [pscustomobject]@{
                element = $element
                record = Get-UiaElementRecord -Element $element
            }
        }
        catch {
            continue
        }
    }
    return $candidates
}

function Get-UiaButtonCandidatesByPrefix {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Prefix,
        [switch]$AllowOffscreen
    )

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $candidates = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            if ($current.ControlType -ne [Windows.Automation.ControlType]::Button -or
                -not $current.IsEnabled -or
                -not $current.Name.StartsWith($Prefix, [StringComparison]::Ordinal)) {
                continue
            }
            if ($current.IsOffscreen -and -not $AllowOffscreen) {
                continue
            }
            [void]$element.GetCurrentPattern(
                [Windows.Automation.InvokePattern]::Pattern)
            $candidates += [pscustomobject]@{
                element = $element
                record = Get-UiaElementRecord -Element $element
            }
        }
        catch {
            continue
        }
    }
    return $candidates
}

function Wait-UniqueUiaButton {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [string]$Name,
        [string]$AutomationId,
        [switch]$AllowOffscreen
    )

    $lookup = @{}
    if ($PSBoundParameters.ContainsKey("Name")) {
        $lookup.Name = $Name
        $description = "Name='$Name'"
    }
    elseif ($PSBoundParameters.ContainsKey("AutomationId")) {
        $lookup.AutomationId = $AutomationId
        $description = "AutomationId='$AutomationId'"
    }
    else {
        throw "Wait-UniqueUiaButton requires Name or AutomationId."
    }
    if ($AllowOffscreen) {
        $lookup.AllowOffscreen = $true
    }

    $lastCandidates = @()
    do {
        $lastCandidates = @(Get-UiaButtonCandidates -Root $Root @lookup)
        if ($lastCandidates.Count -eq 1) {
            return $lastCandidates[0]
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)

    $records = @($lastCandidates | ForEach-Object { $_.record })
    $details = $records | ConvertTo-Json -Compress -Depth 5
    throw "Expected one visible invokable UIA button with $description; found $($records.Count): $details"
}

function Invoke-UiaButtonElement {
    param([Parameter(Mandatory = $true)]$Element)

    $pattern = [Windows.Automation.InvokePattern]$Element.GetCurrentPattern(
        [Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}

function Invoke-UiaButtonByName {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $button = Wait-UniqueUiaButton -Root $Root -Deadline $Deadline -Name $Name
    Invoke-UiaButtonElement -Element $button.element
    return $button.record
}

<#
.SYNOPSIS
Poll the UIA tree until some element's text equals the expected status
string. The status bar renders the text directly, so TextBlock Name IS the
status text.
#>
function Wait-StatusText {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [string[]]$Accepted = @(),
        [string]$AcceptedPrefix,
        [string]$FailurePattern
    )

    do {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $element = $elements.Item($index)
            $name = $null
            try { $name = [string]$element.Current.Name } catch { continue }
            foreach ($expected in $Accepted) {
                if ($name -ceq $expected) {
                    return [pscustomobject]@{
                        matched = $name
                        statusElement = $element
                    }
                }
            }
            if ($AcceptedPrefix -and $name.StartsWith($AcceptedPrefix, [StringComparison]::Ordinal)) {
                return [pscustomobject]@{
                    matched = $name
                    statusElement = $element
                }
            }
            if ($FailurePattern -and $name -match $FailurePattern) {
                throw "Status reported a failure: '$name'"
            }
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)

    throw ("Status text was not observed within the deadline. Expected one of: " +
           ($Accepted -join " | ") +
           ($(if ($FailurePattern) { " (failure pattern: $FailurePattern)" } else { "" })))
}

function Set-UiaTextValue {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$AutomationName,
        [Parameter(Mandatory = $true)][string]$Value,
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
                if ($current.Name -cne $AutomationName -or $current.IsOffscreen) {
                    continue
                }
                $valuePattern = [Windows.Automation.ValuePattern](
                    $element.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern))
                $valuePattern.SetValue($Value)
                return $true
            }
            catch {
                continue
            }
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)

    throw "Text control with automation name '$AutomationName' was not found."
}

<#
.SYNOPSIS
Scroll the window under the cursor with the mouse wheel (the UIA tree only
realizes virtualized rows that enter the viewport).
#>
function Send-WheelScroll {
    param([ValidateRange(1, 30)][int]$Notches = 3)

    if (-not ("WfWheelNative" -as [type])) {
        Add-Type -AssemblyName System.Windows.Forms
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WfWheelNative {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, UIntPtr extra);
}
'@
    }
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen
    $x = [int]($screen.WorkingArea.Width / 2)
    $y = [int]($screen.WorkingArea.Height / 2)
    $null = [WfWheelNative]::SetCursorPos($x, $y)
    for ($i = 0; $i -lt $Notches; $i++) {
        [WfWheelNative]::mouse_event(0x080A, 0, 0, -120, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 80
    }
}

function Get-CrashEvents {
    param(
        [Parameter(Mandatory = $true)][string[]]$ExecutablePaths,
        [Parameter(Mandatory = $true)][datetime]$StartTime
    )

    $candidateEvents = @()
    try {
        $candidateEvents = @(Get-WinEvent -FilterHashtable @{
            LogName = "Application"
            ProviderName = @("Application Error", "Windows Error Reporting")
            StartTime = $StartTime.AddSeconds(-2)
        } -ErrorAction Stop)
    }
    catch {
        if ($_.FullyQualifiedErrorId -like "NoMatchingEventsFound,*") {
            return @()
        }
        throw "Unable to query Application Error/WER: $($_.Exception.Message)"
    }

    return @($candidateEvents | Where-Object {
        $message = [string]$_.Message
        $matched = $false
        foreach ($path in $ExecutablePaths) {
            if ($message -match [Regex]::Escape($path) -or
                $message -match [Regex]::Escape([IO.Path]::GetFileName($path))) {
                $matched = $true
                break
            }
        }
        $matched
    } | ForEach-Object {
        [pscustomobject]@{
            timeCreatedUtc = $_.TimeCreated.ToUniversalTime().ToString("o")
            provider = $_.ProviderName
            id = $_.Id
            message = $_.Message
        }
    })
}

function Assert-NoWebViewModules {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    $process = Get-Process -Id $Process.Id -ErrorAction Stop
    $webview = @($process.Modules | Where-Object {
        $_.ModuleName -match "WebView2|msedge"
    })
    if ($webview.Count -gt 0) {
        throw "Candidate loaded a browser module: $($webview[0].ModuleName)"
    }
    $xaml = @($process.Modules | Where-Object { $_.ModuleName -ieq "Microsoft.UI.Xaml.dll" })
    if ($xaml.Count -eq 0) {
        throw "Candidate did not load a local Microsoft.UI.Xaml.dll."
    }
}

function New-CombinedImage {
    param(
        [Parameter(Mandatory = $true)][string]$LeftPath,
        [Parameter(Mandatory = $true)][string]$RightPath,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [string]$LeftLabel = "Store 2.5.8",
        [string]$RightLabel = "Reactor"
    )

    $left = [Drawing.Bitmap]::FromFile($LeftPath)
    $right = [Drawing.Bitmap]::FromFile($RightPath)
    try {
        $height = [Math]::Max($left.Height, $right.Height) + 40
        $width = $left.Width + $right.Width + 24
        $combined = New-Object Drawing.Bitmap($width, $height)
        $graphics = [Drawing.Graphics]::FromImage($combined)
        try {
            $graphics.Clear([Drawing.Color]::Black)
            $font = New-Object Drawing.Font("Segoe UI", 14)
            $brush = [Drawing.Brushes]::White
            $graphics.DrawImage($left, 0, 40)
            $graphics.DrawImage($right, $left.Width + 24, 40)
            $graphics.DrawString($LeftLabel, $font, $brush, 8, 8)
            $graphics.DrawString($RightLabel, $font, $brush, $left.Width + 32, 8)
        }
        finally {
            $graphics.Dispose()
        }
        $combined.Save($OutputPath, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $left.Dispose()
        $right.Dispose()
    }
}

Export-ModuleMember -Function @(
    "Write-JsonFile",
    "Get-ReactorApplicationVersion",
    "Start-ReactorCandidate",
    "Stop-ReactorCandidate",
    "Get-UiaElementRecord",
    "Get-ReactorUiaRoot",
    "Get-UiaButtonCandidates",
    "Wait-UniqueUiaButton",
    "Invoke-UiaButtonElement",
    "Invoke-UiaButtonByName",
    "Get-UiaButtonCandidatesByPrefix",
    "Wait-StatusText",
    "Set-UiaTextValue",
    "Get-CrashEvents",
    "Assert-NoWebViewModules",
    "New-CombinedImage",
    "Send-WheelScroll"
)
