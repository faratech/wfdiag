param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [ValidateRange(1, 20)]
    [int]$Iterations = 3,

    [ValidateRange(1, 30)]
    [int]$HoldSeconds = 2
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

function Get-PeMachine {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "'$Path' is not a PE image."
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "'$Path' has an invalid PE signature."
        }
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Wait-ReactorWindow {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [int]$TimeoutSeconds = 15
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "Process $($Process.Id) exited during startup with code $($Process.ExitCode)."
        }
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            return $Process.MainWindowHandle
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $deadline)

    throw "Process $($Process.Id) did not create a visible window within $TimeoutSeconds seconds."
}

function Invoke-ReactorButton {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][string]$Name,
        [int]$TimeoutSeconds = 10
    )

    $root = [Windows.Automation.AutomationElement]::FromHandle($Hwnd)
    $condition = New-Object Windows.Automation.PropertyCondition(
        [Windows.Automation.AutomationElement]::NameProperty,
        $Name)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $button = $root.FindFirst(
            [Windows.Automation.TreeScope]::Descendants,
            $condition)
        if ($null -ne $button) {
            $pattern = $button.GetCurrentPattern(
                [Windows.Automation.InvokePattern]::Pattern)
            $pattern.Invoke()
            return
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $deadline)

    throw "Unable to find the '$Name' automation button."
}

function Start-ReactorCase {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$SettingsOpen,
        [Parameter(Mandatory = $true)][int]$Hold
    )

    $previousSettings = $env:WFDIAG_REACTOR_SETTINGS
    $env:WFDIAG_REACTOR_PAGE = "monitor"
    $env:WFDIAG_REACTOR_WIDTH = "1440"
    $env:WFDIAG_REACTOR_HEIGHT = "900"
    $env:WFDIAG_REACTOR_FIXTURE = "populated"
    if ($SettingsOpen) {
        $env:WFDIAG_REACTOR_SETTINGS = "1"
    }
    else {
        Remove-Item Env:WFDIAG_REACTOR_SETTINGS -ErrorAction SilentlyContinue
    }

    $process = $null
    try {
        $process = Start-Process -FilePath $Path -PassThru
        $hwnd = Wait-ReactorWindow -Process $process
        Start-Sleep -Seconds $Hold
        $process.Refresh()
        if ($process.HasExited) {
            throw "Process $($process.Id) exited with code $($process.ExitCode)."
        }
        return [pscustomobject]@{ Process = $process; Hwnd = $hwnd }
    }
    catch {
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            [void]$process.WaitForExit(5000)
        }
        throw
    }
    finally {
        if ($null -eq $previousSettings) {
            Remove-Item Env:WFDIAG_REACTOR_SETTINGS -ErrorAction SilentlyContinue
        }
        else {
            $env:WFDIAG_REACTOR_SETTINGS = $previousSettings
        }
    }
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$releaseDirectory = Split-Path -Parent $resolvedExecutable
$startedAt = Get-Date
$failures = New-Object Collections.Generic.List[string]
$passed = New-Object Collections.Generic.List[string]

$expectedMachine = Get-PeMachine -Path $resolvedExecutable
if ($expectedMachine -notin @(0x014C, 0x8664, 0xAA64)) {
    $failures.Add(("Unsupported executable PE machine: 0x{0:X4}" -f $expectedMachine))
}

$webViewProjection = Get-ChildItem -LiteralPath $releaseDirectory -File |
    Where-Object { $_.Name -ieq "Microsoft.Web.WebView2.Core.dll" } |
    Select-Object -First 1
if ($null -ne $webViewProjection) {
    $failures.Add("Unused WebView2 projection is still staged beside the native executable.")
}
else {
    $passed.Add("no-webview-projection")
}

# Keep this list aligned with the pinned windows-reactor-setup runtime manifest.
# The helper intentionally ignores missing source files, so validating every PE
# image catches both mixed-architecture caches and incomplete staging.
$runtimeDlls = @(
    "CoreMessagingXP.dll",
    "dcompi.dll",
    "dwmcorei.dll",
    "DwmSceneI.dll",
    "DWriteCore.dll",
    "marshal.dll",
    "Microsoft.DirectManipulation.dll",
    "Microsoft.Graphics.Imaging.dll",
    "Microsoft.InputStateManager.dll",
    "Microsoft.Internal.FrameworkUdk.dll",
    "Microsoft.UI.Composition.OSSupport.dll",
    "Microsoft.UI.dll",
    "Microsoft.UI.Input.dll",
    "Microsoft.UI.Windowing.Core.dll",
    "Microsoft.UI.Windowing.dll",
    "Microsoft.UI.Xaml.Controls.dll",
    "Microsoft.UI.Xaml.Internal.dll",
    "Microsoft.UI.Xaml.Phone.dll",
    "Microsoft.ui.xaml.dll",
    "Microsoft.ui.xaml.resources.19h1.dll",
    "Microsoft.ui.xaml.resources.common.dll",
    "Microsoft.Windows.ApplicationModel.Resources.dll",
    "Microsoft.WindowsAppRuntime.dll",
    "MRM.dll",
    "SessionHandleIPCProxyStub.dll",
    "WinUIEdit.dll",
    "wuceffectsi.dll"
)
$runtimeValidationFailed = $false
foreach ($runtimeName in $runtimeDlls) {
    $runtimePath = Join-Path $releaseDirectory $runtimeName
    if (-not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        $failures.Add("Missing staged runtime: $runtimeName")
        $runtimeValidationFailed = $true
        continue
    }
    $runtimeMachine = Get-PeMachine -Path $runtimePath
    if ($runtimeMachine -ne $expectedMachine) {
        $failures.Add(
            ("PE architecture mismatch: {0}=0x{1:X4}, executable=0x{2:X4}" -f
                $runtimeName, $runtimeMachine, $expectedMachine))
        $runtimeValidationFailed = $true
    }
}
if (-not $runtimeValidationFailed) {
    $passed.Add("complete-runtime-pe-alignment")
}

# Do not intentionally launch a payload that is already known to contain a
# foreign-architecture or incomplete native runtime. That was the original CTD
# failure mode this gate was created to prevent.
if ($failures.Count -gt 0) {
    [pscustomobject]@{
        executable = $resolvedExecutable
        peMachine = ("0x{0:X4}" -f $expectedMachine)
        passed = $passed
        failures = $failures
        crashEventCount = 0
    } | ConvertTo-Json -Depth 5
    exit 1
}

foreach ($settingsOpen in @($false, $true)) {
    $label = if ($settingsOpen) { "settings-startup" } else { "normal-startup" }
    for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
        $case = $null
        try {
            $case = Start-ReactorCase `
                -Path $resolvedExecutable `
                -SettingsOpen $settingsOpen `
                -Hold $HoldSeconds
            $passed.Add("$label-$iteration")
        }
        catch {
            $failures.Add("${label}-${iteration}: $($_.Exception.Message)")
        }
        finally {
            if ($null -ne $case -and -not $case.Process.HasExited) {
                Stop-Process -Id $case.Process.Id -Force
                [void]$case.Process.WaitForExit(5000)
            }
        }
    }
}

$interaction = $null
try {
    $interaction = Start-ReactorCase `
        -Path $resolvedExecutable `
        -SettingsOpen $false `
        -Hold $HoldSeconds
    for ($cycle = 1; $cycle -le $Iterations; $cycle++) {
        Invoke-ReactorButton -Hwnd $interaction.Hwnd -Name "Open Settings"
        Start-Sleep -Milliseconds 700
        $interaction.Process.Refresh()
        if ($interaction.Process.HasExited) {
            throw "Process exited while opening Settings in cycle $cycle."
        }
        Invoke-ReactorButton -Hwnd $interaction.Hwnd -Name "Close Settings"
        Start-Sleep -Milliseconds 700
        $interaction.Process.Refresh()
        if ($interaction.Process.HasExited) {
            throw "Process exited while closing Settings in cycle $cycle."
        }
        $passed.Add("settings-open-close-$cycle")
    }

    $loadedXaml = $interaction.Process.Modules |
        Where-Object { $_.ModuleName -eq "Microsoft.UI.Xaml.dll" } |
        Select-Object -First 1
    if ($null -eq $loadedXaml) {
        $failures.Add("Microsoft.UI.Xaml.dll was not loaded by the interaction process.")
    }
    elseif ((Split-Path -Parent $loadedXaml.FileName) -ne $releaseDirectory) {
        $failures.Add("Microsoft.UI.Xaml.dll loaded outside the staged release directory.")
    }
    else {
        $passed.Add("local-xaml-runtime")
    }

    $loadedWebView = $interaction.Process.Modules |
        Where-Object { $_.ModuleName -match "WebView2|msedge" } |
        Select-Object -First 1
    if ($null -ne $loadedWebView) {
        $failures.Add("A WebView2/Edge module was loaded by the native Reactor process.")
    }
    else {
        $passed.Add("no-webview-modules-loaded")
    }
}
catch {
    $failures.Add("settings-interaction: $($_.Exception.Message)")
}
finally {
    if ($null -ne $interaction -and -not $interaction.Process.HasExited) {
        Stop-Process -Id $interaction.Process.Id -Force
        [void]$interaction.Process.WaitForExit(5000)
    }
}

# Event Log writes can trail the faulting process by a moment. Query failures
# must not be mistaken for a clean log, and matching the full candidate path
# avoids attributing a concurrent smoke run of another build to this one.
Start-Sleep -Seconds 2
$crashEvents = @()
$eventQuerySucceeded = $true
try {
    $candidateEvents = @(Get-WinEvent -FilterHashtable @{
        LogName = "Application"
        ProviderName = @("Application Error", "Windows Error Reporting")
        StartTime = $startedAt.AddSeconds(-2)
    } -ErrorAction Stop)
    $crashEvents = @($candidateEvents | Where-Object {
        $_.Message -match [Regex]::Escape($resolvedExecutable)
    })
}
catch {
    if ($_.FullyQualifiedErrorId -notlike "NoMatchingEventsFound,*") {
        $eventQuerySucceeded = $false
        $failures.Add("Unable to inspect the Windows Application event log: $($_.Exception.Message)")
    }
}
if ($eventQuerySucceeded) {
    if ($crashEvents.Count -gt 0) {
        $failures.Add("Windows recorded $($crashEvents.Count) crash event(s) for this candidate path.")
    }
    else {
        $passed.Add("no-crash-events")
    }
}

$result = [pscustomobject]@{
    executable = $resolvedExecutable
    peMachine = ("0x{0:X4}" -f $expectedMachine)
    passed = $passed
    failures = $failures
    crashEventCount = $crashEvents.Count
}
$result | ConvertTo-Json -Depth 5

if ($failures.Count -gt 0) {
    exit 1
}
