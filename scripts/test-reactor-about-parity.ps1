<#
.SYNOPSIS
Captures and validates the Microsoft Store 2.5.8 and native Reactor About dialogs.

.DESCRIPTION
This script is intentionally pinned to the Store-signed ARM64 2.5.8 oracle on
the validation host. It sizes each exact executable/PID to a 1440x1000 logical
viewport at 144 DPI, opens About with UI Automation, captures the foreground
DWM-visible frame with CopyFromScreen, rejects missing dialog pixels, and
writes source/native/combined images plus machine-readable evidence. The native
candidate must also initially focus its header Close button, dismiss with
Escape without exiting, dismiss through the exact about-close UIA button, and
restore keyboard focus to the About navigation button after a final close.

The script does not build Reactor. Supply a freshly built self-contained ARM64
candidate; its version probe must report 2.5.8.

.EXAMPLE
.\scripts\test-reactor-about-parity.ps1 `
  -Executable C:\path\to\aarch64-pc-windows-msvc\release\wfdiag-reactor-spike.exe
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$Executable,

    [string]$OutputDirectory,

    [ValidateRange(10, 120)]
    [int]$WaitSeconds = 30,

    [ValidateRange(0, 10)]
    [int]$SettleSeconds = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

$logicalWidth = 1440
$logicalHeight = 1000
$requiredDpi = 144
$physicalWidth = 2160
$physicalHeight = 1500
$oracleVersion = "2.5.8"
$oraclePackageVersion = [Version]"2.5.8.0"
$oraclePackageName = "32827MikeFara.WindowsForumDiagnostics"
$oraclePackageFamilyName = (
    "32827MikeFara.WindowsForumDiagnostics_t6j5qexy2jpp2")
$oraclePackageFullName = (
    "32827MikeFara.WindowsForumDiagnostics_2.5.8.0_arm64__t6j5qexy2jpp2")
$oraclePublisherId = "t6j5qexy2jpp2"
$oracleArchitecture = "Arm64"
$oracleApplicationId = "App"
$oracleExecutableName = "WindowsForum_Diagnostics.exe"
$oracleAumid = "$oraclePackageFamilyName!$oracleApplicationId"
$arm64PeMachine = [UInt16]0xAA64
$versionProbeFlag = "--wfdiag-version-probe"
$versionProbeEnvironment = "WFDIAG_REACTOR_VERSION_PROBE_FILE"
$repoRoot = Split-Path $PSScriptRoot -Parent
$versionPath = Join-Path $repoRoot "version.json"
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$releaseDirectory = Split-Path -Parent $resolvedExecutable
$startedAt = Get-Date
$captureStamp = Get-Date -Format "yyyyMMdd-HHmmss"

if (-not (Test-Path -LiteralPath $versionPath -PathType Leaf)) {
    throw "Canonical version file does not exist: $versionPath"
}
$canonicalVersion = [string](
    (Get-Content -LiteralPath $versionPath -Raw | ConvertFrom-Json).version)
if ($canonicalVersion -cne $oracleVersion) {
    throw "This oracle is pinned to $oracleVersion, but version.json reports '$canonicalVersion'."
}

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot (
        "reactor-spike\captures-{0}\about-validation\{1}" -f
            $oracleVersion,
            $captureStamp)
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {
    $existingOutput = @(Get-ChildItem -LiteralPath $OutputDirectory -Force)
    if ($existingOutput.Count -gt 0) {
        throw "OutputDirectory must be empty so evidence cannot be overwritten: $OutputDirectory"
    }
}
else {
    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
}

$storeLogicalPath = Join-Path $OutputDirectory "about-store-2.5.8.png"
$storePhysicalPath = Join-Path $OutputDirectory "about-store-2.5.8.physical.png"
$storeMetadataPath = Join-Path $OutputDirectory "about-store-2.5.8.capture.json"
$nativeLogicalPath = Join-Path $OutputDirectory "about-reactor-arm64-2.5.8.png"
$nativePhysicalPath = Join-Path $OutputDirectory "about-reactor-arm64-2.5.8.physical.png"
$nativeMetadataPath = Join-Path $OutputDirectory "about-reactor-arm64-2.5.8.capture.json"
$combinedPath = Join-Path $OutputDirectory "about-store-left-reactor-right.png"
$summaryPath = Join-Path $OutputDirectory "about-validation-summary.json"

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing

if ($null -eq ("WfDiagAboutParityNative" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class WfDiagAboutParityNative
{
    private const uint InputKeyboard = 1;
    private const uint KeyEventKeyUp = 0x0002;
    private const ushort VirtualKeyEscape = 0x001B;

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("dwmapi.dll")]
    public static extern int DwmGetWindowAttribute(
        IntPtr hwnd,
        int attribute,
        out Rect value,
        int valueSize);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetWindowRect(IntPtr hwnd, out Rect value);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool BringWindowToTop(IntPtr hwnd);

    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(
        IntPtr hwnd,
        out uint processId);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AttachThreadInput(
        uint sourceThreadId,
        uint targetThreadId,
        [MarshalAs(UnmanagedType.Bool)] bool attach);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool ShowWindow(IntPtr hwnd, int command);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr dpiContext);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetWindowPos(
        IntPtr hwnd,
        IntPtr insertAfter,
        int x,
        int y,
        int width,
        int height,
        uint flags);

    [StructLayout(LayoutKind.Sequential)]
    private struct KeyboardInput
    {
        public ushort VirtualKey;
        public ushort ScanCode;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct InputUnion
    {
        [FieldOffset(0)]
        public KeyboardInput Keyboard;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Input
    {
        public uint Type;
        public InputUnion Data;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(
        uint inputCount,
        [In] Input[] inputs,
        int inputSize);

    public static uint SendEscape()
    {
        Input[] inputs = new Input[2];
        inputs[0].Type = InputKeyboard;
        inputs[0].Data.Keyboard.VirtualKey = VirtualKeyEscape;
        inputs[1].Type = InputKeyboard;
        inputs[1].Data.Keyboard.VirtualKey = VirtualKeyEscape;
        inputs[1].Data.Keyboard.Flags = KeyEventKeyUp;
        return SendInput(
            (uint)inputs.Length,
            inputs,
            Marshal.SizeOf(typeof(Input)));
    }

    public static uint GetWindowProcessId(IntPtr hwnd)
    {
        uint processId;
        GetWindowThreadProcessId(hwnd, out processId);
        return processId;
    }

    public static bool ForceForegroundWindow(IntPtr hwnd)
    {
        if (hwnd == IntPtr.Zero)
        {
            return false;
        }

        uint ignored;
        uint currentThreadId = GetCurrentThreadId();
        uint targetThreadId = GetWindowThreadProcessId(hwnd, out ignored);
        IntPtr foreground = GetForegroundWindow();
        uint foregroundThreadId = foreground == IntPtr.Zero
            ? 0
            : GetWindowThreadProcessId(foreground, out ignored);
        bool attachedTarget = false;
        bool attachedForeground = false;

        try
        {
            if (targetThreadId != 0 && targetThreadId != currentThreadId)
            {
                attachedTarget = AttachThreadInput(
                    currentThreadId,
                    targetThreadId,
                    true);
            }
            if (foregroundThreadId != 0 &&
                foregroundThreadId != currentThreadId &&
                foregroundThreadId != targetThreadId)
            {
                attachedForeground = AttachThreadInput(
                    currentThreadId,
                    foregroundThreadId,
                    true);
            }

            ShowWindow(hwnd, 9);
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            return GetForegroundWindow() == hwnd;
        }
        finally
        {
            if (attachedForeground)
            {
                AttachThreadInput(currentThreadId, foregroundThreadId, false);
            }
            if (attachedTarget)
            {
                AttachThreadInput(currentThreadId, targetThreadId, false);
            }
        }
    }
}
"@
}

$previousDpiContext = [WfDiagAboutParityNative]::SetThreadDpiAwarenessContext(
    [IntPtr](-4))
if ($previousDpiContext -eq [IntPtr]::Zero) {
    throw "Unable to enter the per-monitor DPI-aware context required for exact capture."
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $Value | ConvertTo-Json -Depth 10 |
        Set-Content -LiteralPath $Path -Encoding UTF8
}

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
        if ($peOffset -lt 0 -or $peOffset -gt ($stream.Length - 6)) {
            throw "'$Path' has an invalid PE header offset."
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "'$Path' has an invalid PE signature."
        }
        return [UInt16]$reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Get-ReactorApplicationVersion {
    param([Parameter(Mandatory = $true)][string]$Path)

    $probePath = Join-Path ([IO.Path]::GetTempPath()) (
        "wfdiag-reactor-about-version-{0}.json" -f
            [Guid]::NewGuid().ToString("N"))
    $previousProbePath = [Environment]::GetEnvironmentVariable(
        $versionProbeEnvironment,
        "Process")
    $probe = $null
    try {
        [Environment]::SetEnvironmentVariable(
            $versionProbeEnvironment,
            $probePath,
            "Process")
        $probe = Start-Process `
            -FilePath $Path `
            -ArgumentList $versionProbeFlag `
            -PassThru
        if (-not $probe.WaitForExit(10000)) {
            Stop-Process -Id $probe.Id -Force -ErrorAction SilentlyContinue
            [void]$probe.WaitForExit(5000)
            throw "Reactor version probe did not exit within 10 seconds."
        }
        if ($probe.ExitCode -ne 0) {
            throw "Reactor version probe exited with code $($probe.ExitCode)."
        }
        if (-not (Test-Path -LiteralPath $probePath -PathType Leaf)) {
            throw "Reactor version probe did not create '$probePath'."
        }
        $document = Get-Content -LiteralPath $probePath -Raw | ConvertFrom-Json
        if ($document.schema -ne 1) {
            throw "Reactor version probe returned unsupported schema '$($document.schema)'."
        }
        $applicationVersion = [string]$document.application_version
        if ([string]::IsNullOrWhiteSpace($applicationVersion)) {
            throw "Reactor version probe did not report application_version."
        }
        return $applicationVersion
    }
    finally {
        [Environment]::SetEnvironmentVariable(
            $versionProbeEnvironment,
            $previousProbePath,
            "Process")
        Remove-Item -LiteralPath $probePath -Force -ErrorAction SilentlyContinue
    }
}

function Get-OracleStoreApplication {
    $packages = @(Get-AppxPackage -Name $oraclePackageName | Where-Object {
        $_.PackageFamilyName -ceq $oraclePackageFamilyName
    })
    $matching = @($packages | Where-Object {
        $_.PackageFullName -ceq $oraclePackageFullName
    })
    if ($matching.Count -ne 1) {
        $found = if ($packages.Count -eq 0) {
            "none"
        }
        else {
            ($packages | ForEach-Object { $_.PackageFullName }) -join ", "
        }
        throw "Expected exact Store package '$oraclePackageFullName'; found: $found."
    }

    $package = $matching[0]
    if ($package.Name -cne $oraclePackageName) {
        throw "Store package Name mismatch: '$($package.Name)'."
    }
    if ($package.PackageFamilyName -cne $oraclePackageFamilyName) {
        throw "Store PackageFamilyName mismatch: '$($package.PackageFamilyName)'."
    }
    if ($package.PublisherId -cne $oraclePublisherId) {
        throw "Store PublisherId mismatch: '$($package.PublisherId)'."
    }
    if ($package.Version -ne $oraclePackageVersion) {
        throw "Store version mismatch: '$($package.Version)'."
    }
    if ($package.Architecture.ToString() -cne $oracleArchitecture) {
        throw "Store architecture mismatch: '$($package.Architecture)'."
    }
    if ($package.SignatureKind.ToString() -cne "Store") {
        throw "Package '$($package.PackageFullName)' is not Store-signed."
    }

    $manifestPath = Join-Path $package.InstallLocation "AppxManifest.xml"
    [xml]$manifest = Get-Content -LiteralPath $manifestPath
    $identity = $manifest.SelectSingleNode(
        "/*[local-name()='Package']/*[local-name()='Identity']")
    if ($null -eq $identity -or
        $identity.Name -cne $oraclePackageName -or
        [Version]$identity.Version -ne $oraclePackageVersion -or
        $identity.ProcessorArchitecture -cne "arm64") {
        throw "Installed AppxManifest identity does not match the ARM64 2.5.8 oracle."
    }

    $application = $manifest.SelectSingleNode(
        "/*[local-name()='Package']/*[local-name()='Applications']/*[local-name()='Application'][1]")
    if ($null -eq $application -or
        $application.Id -cne $oracleApplicationId -or
        $application.Executable -cne $oracleExecutableName) {
        throw "Installed Store Application identity does not match '$oracleAumid'."
    }

    $executablePath = Join-Path $package.InstallLocation $application.Executable
    if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
        throw "Installed Store executable is missing: $executablePath"
    }

    return [pscustomobject]@{
        Package = $package
        ManifestPath = $manifestPath
        ApplicationId = [string]$application.Id
        Aumid = $oracleAumid
        ExecutablePath = $executablePath
        ProcessName = [IO.Path]::GetFileNameWithoutExtension($executablePath)
    }
}

function Get-ExactProcesses {
    param([Parameter(Mandatory = $true)][string]$ExecutablePath)

    return @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
        try {
            [string]::Equals(
                $_.Path,
                $ExecutablePath,
                [StringComparison]::OrdinalIgnoreCase)
        }
        catch {
            $false
        }
    })
}

function Assert-NotRunning {
    param(
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $existing = @(Get-ExactProcesses -ExecutablePath $ExecutablePath)
    if ($existing.Count -gt 0) {
        throw "$Label is already running as PID(s) $($existing.Id -join ', '). Close it before validation."
    }
}

function Wait-ExactWindowByPath {
    param(
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $matches = @(Get-ExactProcesses -ExecutablePath $ExecutablePath)
        if ($matches.Count -gt 1) {
            throw "Multiple exact-path processes appeared for '$ExecutablePath': $($matches.Id -join ', ')."
        }
        if ($matches.Count -eq 1) {
            $process = $matches[0]
            $process.Refresh()
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
                return $process
            }
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "No visible exact-path window appeared for '$ExecutablePath'."
}

function Wait-ProcessWindow {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "PID $($Process.Id) exited during startup with code $($Process.ExitCode)."
        }
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            return $Process
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "PID $($Process.Id) did not create a visible window before the deadline."
}

function Get-WindowBounds {
    param([Parameter(Mandatory = $true)][IntPtr]$Hwnd)

    $window = New-Object WfDiagAboutParityNative+Rect
    if (-not [WfDiagAboutParityNative]::GetWindowRect($Hwnd, [ref]$window)) {
        throw "GetWindowRect failed for HWND $Hwnd."
    }
    $visible = New-Object WfDiagAboutParityNative+Rect
    $hr = [WfDiagAboutParityNative]::DwmGetWindowAttribute(
        $Hwnd,
        9,
        [ref]$visible,
        [Runtime.InteropServices.Marshal]::SizeOf($visible))
    if ($hr -ne 0) {
        $visible = $window
    }
    return [pscustomobject]@{
        Window = $window
        Visible = $visible
        WindowWidth = $window.Right - $window.Left
        WindowHeight = $window.Bottom - $window.Top
        VisibleWidth = $visible.Right - $visible.Left
        VisibleHeight = $visible.Bottom - $visible.Top
    }
}

function Focus-ExactWindow {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $shell = New-Object -ComObject WScript.Shell
    $lastForeground = [IntPtr]::Zero
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "PID $($Process.Id) exited before it could be focused."
        }
        $hwnd = $Process.MainWindowHandle
        [void]$shell.AppActivate([int]$Process.Id)
        [void][WfDiagAboutParityNative]::ForceForegroundWindow($hwnd)
        Start-Sleep -Milliseconds 100
        $lastForeground = [WfDiagAboutParityNative]::GetForegroundWindow()
        if ($lastForeground -eq $hwnd) {
            return
        }
    } while ((Get-Date) -lt $Deadline)

    $foregroundPid = [WfDiagAboutParityNative]::GetWindowProcessId(
        $lastForeground)
    $foregroundOwner = Get-Process -Id $foregroundPid -ErrorAction SilentlyContinue
    $foregroundName = if ($null -eq $foregroundOwner) {
        "<unavailable>"
    }
    else {
        $foregroundOwner.ProcessName
    }
    $foregroundHwnd = "0x{0:X}" -f $lastForeground.ToInt64()
    if ($foregroundName -ceq "LockApp") {
        throw (
            "Interactive desktop is locked: LockApp PID $foregroundPid owns " +
            "foreground HWND $foregroundHwnd, so exact target PID " +
            "$($Process.Id) cannot be foregrounded. Unlock the host and rerun.")
    }
    throw (
        "Exact PID $($Process.Id) did not become the foreground window; " +
        "last foreground owner was PID $foregroundPid ($foregroundName), " +
        "HWND $foregroundHwnd.")
}

function Set-ExactCaptureViewport {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    Focus-ExactWindow -Process $Process -Deadline $Deadline
    $Process.Refresh()
    $hwnd = $Process.MainWindowHandle
    $dpi = [int][WfDiagAboutParityNative]::GetDpiForWindow($hwnd)
    if ($dpi -ne $requiredDpi) {
        throw "PID $($Process.Id) is at $dpi DPI; this oracle requires exactly $requiredDpi DPI (150%)."
    }

    for ($attempt = 0; $attempt -lt 4; $attempt++) {
        $bounds = Get-WindowBounds -Hwnd $hwnd
        $extraWidth = $bounds.WindowWidth - $bounds.VisibleWidth
        $extraHeight = $bounds.WindowHeight - $bounds.VisibleHeight
        $leftInset = $bounds.Visible.Left - $bounds.Window.Left
        $topInset = $bounds.Visible.Top - $bounds.Window.Top
        if (-not [WfDiagAboutParityNative]::SetWindowPos(
            $hwnd,
            [IntPtr]::Zero,
            -$leftInset,
            -$topInset,
            $physicalWidth + $extraWidth,
            $physicalHeight + $extraHeight,
            0x0014)) {
            $win32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "Unable to size PID $($Process.Id) to ${logicalWidth}x${logicalHeight} (Win32 $win32Error)."
        }
        Start-Sleep -Milliseconds 350
        $measured = Get-WindowBounds -Hwnd $hwnd
        $measuredDpi = [int][WfDiagAboutParityNative]::GetDpiForWindow($hwnd)
        if ($measured.Visible.Left -eq 0 -and
            $measured.Visible.Top -eq 0 -and
            $measured.VisibleWidth -eq $physicalWidth -and
            $measured.VisibleHeight -eq $physicalHeight -and
            $measuredDpi -eq $requiredDpi) {
            Focus-ExactWindow -Process $Process -Deadline $Deadline
            return [pscustomobject]@{
                Dpi = $measuredDpi
                Scale = $measuredDpi / 96.0
                Bounds = $measured
            }
        }
    }
    throw "Unable to obtain an exact ${logicalWidth}x${logicalHeight} logical frame at $requiredDpi DPI."
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
            nativeWindowHandle = [int]$current.NativeWindowHandle
            isEnabled = [bool]$current.IsEnabled
            isOffscreen = [bool]$current.IsOffscreen
            isKeyboardFocusable = [bool]$current.IsKeyboardFocusable
            hasKeyboardFocus = [bool]$current.HasKeyboardFocus
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
            nativeWindowHandle = $null
            isEnabled = $null
            isOffscreen = $null
            isKeyboardFocusable = $null
            hasKeyboardFocus = $null
            runtimeId = @()
            bounds = $null
        }
    }
}

function Get-UiaFocusSnapshot {
    $foreground = [WfDiagAboutParityNative]::GetForegroundWindow()
    $focused = $null
    try {
        $focused = [Windows.Automation.AutomationElement]::FocusedElement
    }
    catch {
        return [pscustomobject]@{
            capturedAtUtc = [DateTime]::UtcNow.ToString("o")
            foregroundHwnd = ("0x{0:X}" -f $foreground.ToInt64())
            focused = $null
            error = $_.Exception.Message
        }
    }
    return [pscustomobject]@{
        capturedAtUtc = [DateTime]::UtcNow.ToString("o")
        foregroundHwnd = ("0x{0:X}" -f $foreground.ToInt64())
        focused = Get-UiaElementRecord -Element $focused
        error = $null
    }
}

function Test-UiaRecordIdentity {
    param(
        $Expected,
        $Actual
    )

    if ($null -eq $Expected -or
        $null -eq $Actual -or
        $Expected.unavailable -or
        $Actual.unavailable) {
        return $false
    }
    $expectedRuntimeId = @($Expected.runtimeId)
    $actualRuntimeId = @($Actual.runtimeId)
    if ($expectedRuntimeId.Count -eq 0 -or
        $expectedRuntimeId.Count -ne $actualRuntimeId.Count) {
        return $false
    }
    for ($index = 0; $index -lt $expectedRuntimeId.Count; $index++) {
        if ($expectedRuntimeId[$index] -ne $actualRuntimeId[$index]) {
            return $false
        }
    }
    return $true
}

function Wait-UiaKeyboardFocus {
    param(
        [Parameter(Mandatory = $true)]$ExpectedElement,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $expected = Get-UiaElementRecord -Element $ExpectedElement
    $lastSnapshot = $null
    do {
        $lastSnapshot = Get-UiaFocusSnapshot
        if ($null -ne $lastSnapshot.focused -and
            (Test-UiaRecordIdentity `
                -Expected $expected `
                -Actual $lastSnapshot.focused)) {
            return [pscustomobject]@{
                passed = $true
                expected = $expected
                observed = $lastSnapshot
                deadlineUtc = $Deadline.ToUniversalTime().ToString("o")
            }
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)

    return [pscustomobject]@{
        passed = $false
        expected = $expected
        observed = $lastSnapshot
        deadlineUtc = $Deadline.ToUniversalTime().ToString("o")
    }
}

function Get-UiaButtonCandidates {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [string]$Name,
        [string]$AutomationId
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
                $current.IsOffscreen -or
                -not $current.IsEnabled -or
                ($matchName -and $current.Name -cne $Name) -or
                ($matchAutomationId -and $current.AutomationId -cne $AutomationId)) {
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
        [string]$AutomationId
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

function Get-UiaRecords {
    param([Parameter(Mandatory = $true)]$Root)

    $records = @()
    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $record = Get-UiaElementRecord -Element $element
            if ($record.unavailable -or
                [string]::IsNullOrWhiteSpace($record.name)) {
                continue
            }
            $records += $record
        }
        catch {
            continue
        }
    }
    return $records
}

function Find-UiaButton {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Name,
        [switch]$PreferBottom
    )

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $candidates = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            if ($element.Current.Name -cne $Name -or
                $element.Current.ControlType -ne [Windows.Automation.ControlType]::Button -or
                $element.Current.IsOffscreen -or
                -not $element.Current.IsEnabled) {
                continue
            }
            [void]$element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
            $candidates += [pscustomobject]@{
                element = $element
                y = $element.Current.BoundingRectangle.Y
            }
        }
        catch {
            continue
        }
    }
    if ($candidates.Count -eq 0) {
        return $null
    }
    if ($PreferBottom) {
        return ($candidates |
            Sort-Object y -Descending |
            Select-Object -First 1).element
    }
    return ($candidates | Sort-Object y | Select-Object -First 1).element
}

function Wait-InvokeUiaButton {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [switch]$PreferBottom
    )

    do {
        $button = Find-UiaButton `
            -Root $Root `
            -Name $Name `
            -PreferBottom:$PreferBottom
        if ($null -ne $button) {
            $pattern = [Windows.Automation.InvokePattern]$button.GetCurrentPattern(
                [Windows.Automation.InvokePattern]::Pattern)
            $pattern.Invoke()
            return
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "UI Automation button '$Name' did not become invokable."
}

function Select-UiaRecord {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ControlType,
        [double]$MinimumY = [double]::NegativeInfinity,
        [double]$MaximumY = [double]::PositiveInfinity,
        [switch]$PreferBottom
    )

    $matches = @($Records | Where-Object {
        $_.name -ceq $Name -and
        $_.controlType -ceq $ControlType -and
        -not $_.isOffscreen -and
        $_.bounds.width -gt 0 -and
        $_.bounds.height -gt 0 -and
        $_.bounds.y -ge $MinimumY -and
        $_.bounds.y -le $MaximumY
    })
    if ($matches.Count -eq 0) {
        return $null
    }
    if ($PreferBottom) {
        return $matches |
            Sort-Object { $_.bounds.y } -Descending |
            Select-Object -First 1
    }
    return $matches | Sort-Object { $_.bounds.y } | Select-Object -First 1
}

function Wait-AboutEvidence {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$WindowMeasurement,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $description = (
        "A native Windows diagnostics tool by WindowsForum.com. Runs hardware, " +
        "driver, storage, network, security and log diagnostics locally " +
        [string][char]0x2014 +
        " with optional on-device or cloud AI analysis.")
    $contentMinimumY = (
        $WindowMeasurement.Bounds.Visible.Top +
        ($WindowMeasurement.Bounds.VisibleHeight * 0.15))

    do {
        $records = @(Get-UiaRecords -Root $Root)
        $dialogWindow = Select-UiaRecord `
            -Records $records `
            -Name "About" `
            -ControlType "ControlType.Window"
        $applicationTitle = Select-UiaRecord `
            -Records $records `
            -Name "WindowsForum Diagnostics" `
            -ControlType "ControlType.Text" `
            -MinimumY $contentMinimumY `
            -PreferBottom
        $dialogTitleMaximumY = if ($null -ne $applicationTitle) {
            $applicationTitle.bounds.y - 1
        }
        else {
            [double]::PositiveInfinity
        }
        $dialogTitle = Select-UiaRecord `
            -Records $records `
            -Name "About" `
            -ControlType "ControlType.Text" `
            -MinimumY $contentMinimumY `
            -MaximumY $dialogTitleMaximumY `
            -PreferBottom
        $version = Select-UiaRecord `
            -Records $records `
            -Name "Version $oracleVersion" `
            -ControlType "ControlType.Text" `
            -MinimumY $contentMinimumY `
            -PreferBottom
        $descriptionRecord = Select-UiaRecord `
            -Records $records `
            -Name $description `
            -ControlType "ControlType.Text" `
            -MinimumY $contentMinimumY `
            -PreferBottom
        $windowsForum = Select-UiaRecord `
            -Records $records `
            -Name "WindowsForum" `
            -ControlType "ControlType.Button" `
            -MinimumY $contentMinimumY `
            -PreferBottom
        $github = Select-UiaRecord `
            -Records $records `
            -Name "GitHub" `
            -ControlType "ControlType.Button" `
            -MinimumY $contentMinimumY `
            -PreferBottom
        $close = Select-UiaRecord `
            -Records $records `
            -Name "Close" `
            -ControlType "ControlType.Button" `
            -MinimumY $contentMinimumY `
            -PreferBottom

        if ($null -notin @(
            $dialogWindow,
            $dialogTitle,
            $applicationTitle,
            $version,
            $descriptionRecord,
            $windowsForum,
            $github,
            $close)) {
            return [pscustomobject]@{
                requiredControlNames = @(
                    "About",
                    "WindowsForum Diagnostics",
                    "Version $oracleVersion",
                    "WindowsForum",
                    "GitHub",
                    "Close"
                )
                requiredDescription = $description
                allRecords = $records
                dialogWindow = $dialogWindow
                dialogTitle = $dialogTitle
                applicationTitle = $applicationTitle
                version = $version
                description = $descriptionRecord
                windowsForumButton = $windowsForum
                githubButton = $github
                closeButton = $close
            }
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "UI Automation did not expose the complete exact About dialog before the deadline."
}

function Wait-AboutClosed {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Description,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $records = @(Get-UiaRecords -Root $Root)
        $visibleDescription = @($records | Where-Object {
            $_.name -ceq $Description -and -not $_.isOffscreen
        })
        if ($visibleDescription.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)
    throw "About dialog did not close after the requested dismissal action."
}

function Save-ForegroundCapture {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Measurement,
        [Parameter(Mandatory = $true)][string]$LogicalPath,
        [Parameter(Mandatory = $true)][string]$PhysicalPath,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    if ($Measurement.Dpi -ne $requiredDpi -or
        $Measurement.Bounds.VisibleWidth -ne $physicalWidth -or
        $Measurement.Bounds.VisibleHeight -ne $physicalHeight) {
        throw "The supplied viewport measurement is not the required capture geometry."
    }
    Focus-ExactWindow -Process $Process -Deadline $Deadline
    if ($SettleSeconds -gt 0) {
        Start-Sleep -Seconds $SettleSeconds
    }
    $Process.Refresh()
    $hwnd = $Process.MainWindowHandle
    if ([WfDiagAboutParityNative]::GetForegroundWindow() -ne $hwnd) {
        throw "PID $($Process.Id) lost foreground ownership before capture."
    }
    $bounds = Get-WindowBounds -Hwnd $hwnd
    $dpi = [int][WfDiagAboutParityNative]::GetDpiForWindow($hwnd)
    if ($dpi -ne $requiredDpi -or
        $bounds.Visible.Left -ne 0 -or
        $bounds.Visible.Top -ne 0 -or
        $bounds.VisibleWidth -ne $physicalWidth -or
        $bounds.VisibleHeight -ne $physicalHeight) {
        throw "Capture target drifted from the exact 1440x1000/144-DPI viewport."
    }

    $physicalBitmap = $null
    $physicalGraphics = $null
    $logicalBitmap = $null
    $logicalGraphics = $null
    try {
        $physicalBitmap = New-Object Drawing.Bitmap(
            $physicalWidth,
            $physicalHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $physicalGraphics = [Drawing.Graphics]::FromImage($physicalBitmap)
        $physicalGraphics.CopyFromScreen(
            $bounds.Visible.Left,
            $bounds.Visible.Top,
            0,
            0,
            (New-Object Drawing.Size($physicalWidth, $physicalHeight)),
            [Drawing.CopyPixelOperation]::SourceCopy)
        $physicalBitmap.Save($PhysicalPath, [Drawing.Imaging.ImageFormat]::Png)

        $logicalBitmap = New-Object Drawing.Bitmap(
            $logicalWidth,
            $logicalHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $logicalGraphics = [Drawing.Graphics]::FromImage($logicalBitmap)
        $logicalGraphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
        $logicalGraphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $logicalGraphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $logicalGraphics.DrawImage(
            $physicalBitmap,
            0,
            0,
            $logicalWidth,
            $logicalHeight)
        $logicalBitmap.Save($LogicalPath, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        if ($null -ne $logicalGraphics) { $logicalGraphics.Dispose() }
        if ($null -ne $logicalBitmap) { $logicalBitmap.Dispose() }
        if ($null -ne $physicalGraphics) { $physicalGraphics.Dispose() }
        if ($null -ne $physicalBitmap) { $physicalBitmap.Dispose() }
    }

    return [pscustomobject]@{
        method = "Foreground CopyFromScreen of exact DWM-visible frame"
        logicalPath = [IO.Path]::GetFullPath($LogicalPath)
        physicalPath = [IO.Path]::GetFullPath($PhysicalPath)
        logicalWidth = $logicalWidth
        logicalHeight = $logicalHeight
        physicalWidth = $physicalWidth
        physicalHeight = $physicalHeight
        dpi = $dpi
        scale = $dpi / 96.0
        visibleBounds = [pscustomobject]@{
            left = $bounds.Visible.Left
            top = $bounds.Visible.Top
            width = $bounds.VisibleWidth
            height = $bounds.VisibleHeight
        }
    }
}

function Get-RecordPixelMetrics {
    param(
        [Parameter(Mandatory = $true)][Drawing.Bitmap]$Bitmap,
        [Parameter(Mandatory = $true)]$Record,
        [Parameter(Mandatory = $true)]$Capture,
        [ValidateRange(1, 8)][int]$SampleStep = 1
    )

    $rawLeft = [int][Math]::Floor(
        $Record.bounds.x - $Capture.visibleBounds.left)
    $rawTop = [int][Math]::Floor(
        $Record.bounds.y - $Capture.visibleBounds.top)
    $rawRight = [int][Math]::Ceiling(
        $Record.bounds.x + $Record.bounds.width - $Capture.visibleBounds.left)
    $rawBottom = [int][Math]::Ceiling(
        $Record.bounds.y + $Record.bounds.height - $Capture.visibleBounds.top)
    if ($rawRight -le 0 -or
        $rawBottom -le 0 -or
        $rawLeft -ge $Bitmap.Width -or
        $rawTop -ge $Bitmap.Height) {
        throw "UI Automation record '$($Record.name)' does not intersect the captured frame."
    }
    $left = $rawLeft
    $top = $rawTop
    $right = $rawRight
    $bottom = $rawBottom
    $left = [Math]::Max(0, [Math]::Min($Bitmap.Width - 1, $left))
    $top = [Math]::Max(0, [Math]::Min($Bitmap.Height - 1, $top))
    $right = [Math]::Max($left + 1, [Math]::Min($Bitmap.Width, $right))
    $bottom = [Math]::Max($top + 1, [Math]::Min($Bitmap.Height, $bottom))

    $minimum = 255.0
    $maximum = 0.0
    $sum = 0.0
    $sumSquares = 0.0
    $brightSamples = 0
    $sampleCount = 0
    for ($y = $top; $y -lt $bottom; $y += $SampleStep) {
        for ($x = $left; $x -lt $right; $x += $SampleStep) {
            $pixel = $Bitmap.GetPixel($x, $y)
            $luminance = (
                (299.0 * $pixel.R) +
                (587.0 * $pixel.G) +
                (114.0 * $pixel.B)) / 1000.0
            $minimum = [Math]::Min($minimum, $luminance)
            $maximum = [Math]::Max($maximum, $luminance)
            $sum += $luminance
            $sumSquares += $luminance * $luminance
            if ($luminance -ge 130.0) {
                $brightSamples++
            }
            $sampleCount++
        }
    }
    if ($sampleCount -eq 0) {
        throw "No pixels intersect UI Automation record '$($Record.name)'."
    }
    $mean = $sum / $sampleCount
    $variance = [Math]::Max(0.0, ($sumSquares / $sampleCount) - ($mean * $mean))
    return [pscustomobject]@{
        name = $Record.name
        controlType = $Record.controlType
        sampleStep = $SampleStep
        sampleCount = $sampleCount
        minimumLuminance = [Math]::Round($minimum, 3)
        maximumLuminance = [Math]::Round($maximum, 3)
        luminanceRange = [Math]::Round($maximum - $minimum, 3)
        standardDeviation = [Math]::Round([Math]::Sqrt($variance), 3)
        brightSamples = $brightSamples
        captureBounds = [pscustomobject]@{
            left = $left
            top = $top
            width = $right - $left
            height = $bottom - $top
        }
    }
}

function Assert-DialogPixels {
    param(
        [Parameter(Mandatory = $true)][string]$PhysicalPath,
        [Parameter(Mandatory = $true)]$Capture,
        [Parameter(Mandatory = $true)]$AboutEvidence
    )

    $bitmap = $null
    try {
        $bitmap = [Drawing.Bitmap]::FromFile($PhysicalPath)
        if ($bitmap.Width -ne $physicalWidth -or
            $bitmap.Height -ne $physicalHeight) {
            throw "Physical capture is $($bitmap.Width)x$($bitmap.Height), expected ${physicalWidth}x${physicalHeight}."
        }
        $targets = @(
            $AboutEvidence.dialogTitle,
            $AboutEvidence.applicationTitle,
            $AboutEvidence.version,
            $AboutEvidence.description,
            $AboutEvidence.windowsForumButton,
            $AboutEvidence.githubButton,
            $AboutEvidence.closeButton
        )
        $metrics = @()
        foreach ($target in $targets) {
            $metric = Get-RecordPixelMetrics `
                -Bitmap $bitmap `
                -Record $target `
                -Capture $Capture `
                -SampleStep 1
            $metrics += $metric
            if ($metric.maximumLuminance -lt 130.0 -or
                $metric.luminanceRange -lt 36.0 -or
                $metric.standardDeviation -lt 6.0 -or
                $metric.brightSamples -lt 3) {
                throw ((
                    "Capture lacks rendered dialog pixels for '{0}' " +
                    "(max={1}, range={2}, sd={3}, bright={4}).") -f
                    $metric.name,
                    $metric.maximumLuminance,
                    $metric.luminanceRange,
                    $metric.standardDeviation,
                    $metric.brightSamples)
            }
        }
        return [pscustomobject]@{
            result = "pass"
            rule = "All seven visible About targets must contain high-contrast rendered pixels"
            rejectedState = "blank or dim-overlay-only capture"
            targetMetrics = $metrics
        }
    }
    finally {
        if ($null -ne $bitmap) { $bitmap.Dispose() }
    }
}

function Get-NativeRuntimeEvidence {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$CandidateDirectory
    )

    $Process.Refresh()
    $modules = @($Process.Modules)
    $xaml = @($modules | Where-Object {
        $_.ModuleName -ieq "Microsoft.UI.Xaml.dll"
    })
    if ($xaml.Count -ne 1) {
        throw "PID $($Process.Id) did not load exactly one Microsoft.UI.Xaml.dll."
    }
    $xamlPath = [IO.Path]::GetFullPath($xaml[0].FileName)
    $xamlDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $xamlPath))
    if (-not [string]::Equals(
        $xamlDirectory,
        [IO.Path]::GetFullPath($CandidateDirectory),
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "Microsoft.UI.Xaml.dll loaded outside the candidate directory: $xamlPath"
    }

    $loadedWebView = @($modules | Where-Object {
        $_.ModuleName -match "WebView2|msedge"
    } | ForEach-Object { $_.FileName })
    if ($loadedWebView.Count -gt 0) {
        throw "WebView2/Edge modules are loaded: $($loadedWebView -join ', ')."
    }

    $stagedWebView = @(Get-ChildItem `
        -LiteralPath $CandidateDirectory `
        -Recurse `
        -File | Where-Object {
            $_.Name -match "WebView2|msedge"
        } | ForEach-Object { $_.FullName })
    if ($stagedWebView.Count -gt 0) {
        throw "WebView2/Edge files are staged with the candidate: $($stagedWebView -join ', ')."
    }

    return [pscustomobject]@{
        localXaml = [pscustomobject]@{
            loaded = $true
            path = $xamlPath
            localToCandidate = $true
        }
        webView = [pscustomobject]@{
            loadedModules = $loadedWebView
            stagedFiles = $stagedWebView
        }
    }
}

function Close-AboutAndProcess {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$AboutEvidence,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    Wait-InvokeUiaButton `
        -Root $Root `
        -Name "Close" `
        -PreferBottom `
        -Deadline $Deadline
    Wait-AboutClosed `
        -Root $Root `
        -Description $AboutEvidence.requiredDescription `
        -Deadline $Deadline

    $Process.Refresh()
    if ($Process.HasExited) {
        return [pscustomobject]@{
            dialogCloseInvoked = $true
            mainCloseRequested = $false
            processExited = $true
        }
    }
    $mainCloseRequested = $Process.CloseMainWindow()
    if (-not $mainCloseRequested) {
        throw "$Label PID $($Process.Id) rejected CloseMainWindow."
    }
    if (-not $Process.WaitForExit(8000)) {
        throw "$Label PID $($Process.Id) did not exit gracefully within 8 seconds; it was not force-stopped."
    }
    return [pscustomobject]@{
        dialogCloseInvoked = $true
        mainCloseRequested = $true
        processExited = $true
    }
}

function Get-ProcessStateEvidence {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    try {
        $Process.Refresh()
        if ($Process.HasExited) {
            return [pscustomobject]@{
                pid = $Process.Id
                alive = $false
                mainWindowHandle = "0x0"
                exitCode = $Process.ExitCode
                error = $null
            }
        }
        return [pscustomobject]@{
            pid = $Process.Id
            alive = $true
            mainWindowHandle = ("0x{0:X}" -f $Process.MainWindowHandle.ToInt64())
            exitCode = $null
            error = $null
        }
    }
    catch {
        return [pscustomobject]@{
            pid = $Process.Id
            alive = $false
            mainWindowHandle = $null
            exitCode = $null
            error = $_.Exception.Message
        }
    }
}

function Test-AboutVisible {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $records = @(Get-UiaRecords -Root $Root)
    return @($records | Where-Object {
        $_.name -ceq $Description -and -not $_.isOffscreen
    }).Count -gt 0
}

function Measure-InitialHeaderCloseFocus {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    try {
        $candidate = Wait-UniqueUiaButton `
            -Root $Root `
            -AutomationId "about-close" `
            -Deadline $Deadline
        $focus = Wait-UiaKeyboardFocus `
            -ExpectedElement $candidate.element `
            -Deadline $Deadline
        $passed = (
            $focus.passed -and
            $null -ne $focus.observed.focused -and
            $focus.observed.focused.processId -eq $Process.Id -and
            $focus.observed.focused.automationId -ceq "about-close")
        return [pscustomobject]@{
            passed = $passed
            expectedAutomationId = "about-close"
            candidateCount = 1
            target = $candidate.record
            focus = $focus
            error = if ($passed) {
                $null
            }
            else {
                "The initially focused UIA element was not the candidate's about-close button."
            }
        }
    }
    catch {
        $candidates = @(Get-UiaButtonCandidates `
            -Root $Root `
            -AutomationId "about-close")
        return [pscustomobject]@{
            passed = $false
            expectedAutomationId = "about-close"
            candidateCount = $candidates.Count
            target = if ($candidates.Count -eq 1) {
                $candidates[0].record
            }
            else {
                $null
            }
            candidates = @($candidates | ForEach-Object { $_.record })
            focus = [pscustomobject]@{
                passed = $false
                expected = $null
                observed = Get-UiaFocusSnapshot
                deadlineUtc = $Deadline.ToUniversalTime().ToString("o")
            }
            error = $_.Exception.Message
        }
    }
}

function Send-EscapeToExactProcess {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    Focus-ExactWindow -Process $Process -Deadline $Deadline
    $Process.Refresh()
    if ($Process.HasExited) {
        throw "PID $($Process.Id) exited before Escape could be sent."
    }
    $hwnd = $Process.MainWindowHandle
    $foregroundBefore = [WfDiagAboutParityNative]::GetForegroundWindow()
    if ($foregroundBefore -ne $hwnd) {
        throw "PID $($Process.Id) did not own the foreground immediately before Escape."
    }
    $focusBefore = Get-UiaFocusSnapshot
    $sent = [WfDiagAboutParityNative]::SendEscape()
    $win32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($sent -ne 2) {
        throw "SendInput injected $sent of 2 Escape events (Win32 $win32Error)."
    }
    return [pscustomobject]@{
        method = "user32 SendInput VK_ESCAPE key-down/key-up"
        targetPid = $Process.Id
        targetHwnd = ("0x{0:X}" -f $hwnd.ToInt64())
        foregroundBefore = ("0x{0:X}" -f $foregroundBefore.ToInt64())
        focusBefore = $focusBefore
        eventsInjected = $sent
        win32Error = $win32Error
    }
}

function Open-AboutFromNavigation {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$WindowMeasurement,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    Focus-ExactWindow -Process $Process -Deadline $Deadline
    $navigation = Wait-UniqueUiaButton `
        -Root $Root `
        -Name "About" `
        -Deadline $Deadline
    $focusRequestError = $null
    try {
        $navigation.element.SetFocus()
    }
    catch {
        $focusRequestError = $_.Exception.Message
    }
    $preOpenFocus = if ($null -eq $focusRequestError) {
        Wait-UiaKeyboardFocus `
            -ExpectedElement $navigation.element `
            -Deadline $Deadline
    }
    else {
        [pscustomobject]@{
            passed = $false
            expected = $navigation.record
            observed = Get-UiaFocusSnapshot
            deadlineUtc = $Deadline.ToUniversalTime().ToString("o")
            error = $focusRequestError
        }
    }
    Invoke-UiaButtonElement -Element $navigation.element
    $about = Wait-AboutEvidence `
        -Root $Root `
        -WindowMeasurement $WindowMeasurement `
        -Deadline $Deadline
    return [pscustomobject]@{
        navigationElement = $navigation.element
        navigationButton = $navigation.record
        navigationCandidateCount = 1
        preOpenFocus = $preOpenFocus
        about = $about
    }
}

function Try-RecoverClosedAbout {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Description,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    try {
        if (-not (Test-AboutVisible -Root $Root -Description $Description)) {
            return [pscustomobject]@{
                needed = $false
                passed = $true
                error = $null
            }
        }
        Wait-InvokeUiaButton `
            -Root $Root `
            -Name "Close" `
            -PreferBottom `
            -Deadline $Deadline
        Wait-AboutClosed `
            -Root $Root `
            -Description $Description `
            -Deadline $Deadline
        return [pscustomobject]@{
            needed = $true
            passed = $true
            error = $null
        }
    }
    catch {
        return [pscustomobject]@{
            needed = $true
            passed = $false
            error = $_.Exception.Message
        }
    }
}

function Invoke-NativeAboutBehaviorValidation {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$WindowMeasurement,
        [Parameter(Mandatory = $true)]$InitialAbout,
        [Parameter(Mandatory = $true)]$InitialHeaderFocus,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds
    )

    $failures = @()
    $evidence = [ordered]@{
        passed = $false
        initialHeaderFocus = $InitialHeaderFocus
        escapeDismissal = $null
        topCloseDismissal = $null
        focusRestoration = $null
        failures = @()
    }
    if (-not $InitialHeaderFocus.passed) {
        $failures += (
            "Initial About focus was not automationId 'about-close'; see behaviorValidation.initialHeaderFocus for exact UIA data.")
    }

    $escapeInput = $null
    $escapeClosed = $false
    try {
        $escapeInput = Send-EscapeToExactProcess `
            -Process $Process `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        Wait-AboutClosed `
            -Root $Root `
            -Description $InitialAbout.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $escapeClosed = $true
        $processState = Get-ProcessStateEvidence -Process $Process
        if (-not $processState.alive -or $processState.mainWindowHandle -eq "0x0") {
            throw "The Reactor main window was not alive after Escape dismissed About."
        }
        $evidence.escapeDismissal = [pscustomobject]@{
            passed = $true
            input = $escapeInput
            dialogClosed = $true
            process = $processState
            error = $null
            recovery = $null
        }
    }
    catch {
        $processState = Get-ProcessStateEvidence -Process $Process
        $recovery = Try-RecoverClosedAbout `
            -Root $Root `
            -Description $InitialAbout.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $evidence.escapeDismissal = [pscustomobject]@{
            passed = $false
            input = $escapeInput
            dialogClosed = $escapeClosed
            process = $processState
            error = $_.Exception.Message
            recovery = $recovery
        }
        $failures += "Escape did not dismiss About while preserving the Reactor main window: $($_.Exception.Message)"
    }

    $topOpen = $null
    $topButton = $null
    $topClosed = $false
    try {
        $topOpen = Open-AboutFromNavigation `
            -Process $Process `
            -Root $Root `
            -WindowMeasurement $WindowMeasurement `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $topButton = Wait-UniqueUiaButton `
            -Root $Root `
            -AutomationId "about-close" `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        Invoke-UiaButtonElement -Element $topButton.element
        Wait-AboutClosed `
            -Root $Root `
            -Description $topOpen.about.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $topClosed = $true
        $processState = Get-ProcessStateEvidence -Process $Process
        if (-not $processState.alive -or $processState.mainWindowHandle -eq "0x0") {
            throw "The Reactor main window was not alive after about-close dismissed About."
        }
        $evidence.topCloseDismissal = [pscustomobject]@{
            passed = $true
            invokedAutomationId = "about-close"
            target = $topButton.record
            navigationButton = $topOpen.navigationButton
            navigationPreOpenFocus = $topOpen.preOpenFocus
            dialogClosed = $true
            process = $processState
            error = $null
            recovery = $null
        }
    }
    catch {
        $processState = Get-ProcessStateEvidence -Process $Process
        $recovery = Try-RecoverClosedAbout `
            -Root $Root `
            -Description $InitialAbout.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $evidence.topCloseDismissal = [pscustomobject]@{
            passed = $false
            invokedAutomationId = "about-close"
            target = if ($null -ne $topButton) { $topButton.record } else { $null }
            navigationButton = if ($null -ne $topOpen) { $topOpen.navigationButton } else { $null }
            navigationPreOpenFocus = if ($null -ne $topOpen) { $topOpen.preOpenFocus } else { $null }
            dialogClosed = $topClosed
            process = $processState
            error = $_.Exception.Message
            recovery = $recovery
        }
        $failures += "The exact about-close UIA button did not dismiss About: $($_.Exception.Message)"
    }

    $restoreOpen = $null
    $bottomButton = $null
    $restoreClosed = $false
    try {
        $restoreOpen = Open-AboutFromNavigation `
            -Process $Process `
            -Root $Root `
            -WindowMeasurement $WindowMeasurement `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $bottomButtonElement = Find-UiaButton `
            -Root $Root `
            -Name "Close" `
            -PreferBottom
        if ($null -eq $bottomButtonElement) {
            throw "The bottom Close button was not available for focus-restoration validation."
        }
        $bottomButton = Get-UiaElementRecord -Element $bottomButtonElement
        Invoke-UiaButtonElement -Element $bottomButtonElement
        Wait-AboutClosed `
            -Root $Root `
            -Description $restoreOpen.about.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $restoreClosed = $true
        $processState = Get-ProcessStateEvidence -Process $Process
        if (-not $processState.alive -or $processState.mainWindowHandle -eq "0x0") {
            throw "The Reactor main window was not alive after the final About close."
        }
        $restoredFocus = Wait-UiaKeyboardFocus `
            -ExpectedElement $restoreOpen.navigationElement `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $restorationPassed = (
            $restoreOpen.navigationCandidateCount -eq 1 -and
            $restoreOpen.preOpenFocus.passed -and
            $restoredFocus.passed)
        $evidence.focusRestoration = [pscustomobject]@{
            passed = $restorationPassed
            reliablyIdentified = $restoreOpen.navigationCandidateCount -eq 1
            navigationCandidateCount = $restoreOpen.navigationCandidateCount
            navigationButton = $restoreOpen.navigationButton
            navigationPreOpenFocus = $restoreOpen.preOpenFocus
            closeButton = $bottomButton
            dialogClosed = $true
            restoredFocus = $restoredFocus
            process = $processState
            error = if ($restorationPassed) {
                $null
            }
            else {
                "Keyboard focus did not return to the unique About navigation button."
            }
            recovery = $null
        }
        if (-not $restorationPassed) {
            $failures += (
                "Focus did not restore to the unique About navigation button; see behaviorValidation.focusRestoration for exact UIA data.")
        }
    }
    catch {
        $processState = Get-ProcessStateEvidence -Process $Process
        $recovery = Try-RecoverClosedAbout `
            -Root $Root `
            -Description $InitialAbout.requiredDescription `
            -Deadline (Get-Date).AddSeconds($TimeoutSeconds)
        $evidence.focusRestoration = [pscustomobject]@{
            passed = $false
            reliablyIdentified = if ($null -ne $restoreOpen) {
                $restoreOpen.navigationCandidateCount -eq 1
            }
            else {
                $false
            }
            navigationCandidateCount = if ($null -ne $restoreOpen) {
                $restoreOpen.navigationCandidateCount
            }
            else {
                0
            }
            navigationButton = if ($null -ne $restoreOpen) { $restoreOpen.navigationButton } else { $null }
            navigationPreOpenFocus = if ($null -ne $restoreOpen) { $restoreOpen.preOpenFocus } else { $null }
            closeButton = $bottomButton
            dialogClosed = $restoreClosed
            restoredFocus = Get-UiaFocusSnapshot
            process = $processState
            error = $_.Exception.Message
            recovery = $recovery
        }
        $failures += "Focus-restoration validation failed: $($_.Exception.Message)"
    }

    $evidence.failures = @($failures)
    $evidence.passed = $failures.Count -eq 0
    return [pscustomobject]$evidence
}

function Close-MainProcessAfterBehavior {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$Label,
        [bool]$DialogCloseInvoked = $true
    )

    $Process.Refresh()
    if ($Process.HasExited) {
        return [pscustomobject]@{
            dialogCloseInvoked = $DialogCloseInvoked
            mainCloseRequested = $false
            processExited = $true
        }
    }
    if ($Process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "$Label PID $($Process.Id) has no main window after behavior validation."
    }
    $mainCloseRequested = $Process.CloseMainWindow()
    if (-not $mainCloseRequested) {
        throw "$Label PID $($Process.Id) rejected CloseMainWindow."
    }
    if (-not $Process.WaitForExit(8000)) {
        throw "$Label PID $($Process.Id) did not exit gracefully within 8 seconds; it was not force-stopped."
    }
    return [pscustomobject]@{
        dialogCloseInvoked = $DialogCloseInvoked
        mainCloseRequested = $true
        processExited = $true
    }
}

function New-CombinedImage {
    param(
        [Parameter(Mandatory = $true)][string]$LeftPath,
        [Parameter(Mandatory = $true)][string]$RightPath,
        [Parameter(Mandatory = $true)][string]$OutputPath
    )

    $left = $null
    $right = $null
    $combined = $null
    $graphics = $null
    try {
        $left = [Drawing.Image]::FromFile($LeftPath)
        $right = [Drawing.Image]::FromFile($RightPath)
        if ($left.Width -ne $logicalWidth -or
            $left.Height -ne $logicalHeight -or
            $right.Width -ne $logicalWidth -or
            $right.Height -ne $logicalHeight) {
            throw "Source/native logical images are not both ${logicalWidth}x${logicalHeight}."
        }
        $combined = New-Object Drawing.Bitmap(
            ($logicalWidth * 2),
            $logicalHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($combined)
        $graphics.DrawImageUnscaled($left, 0, 0)
        $graphics.DrawImageUnscaled($right, $logicalWidth, 0)
        $combined.Save($OutputPath, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        if ($null -ne $graphics) { $graphics.Dispose() }
        if ($null -ne $combined) { $combined.Dispose() }
        if ($null -ne $left) { $left.Dispose() }
        if ($null -ne $right) { $right.Dispose() }
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

function Try-GracefulCleanup {
    param(
        [Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$Label,
        $Root,
        [string]$AboutDescription
    )

    if ($null -eq $Process) {
        return $null
    }
    try {
        $Process.Refresh()
        if ($Process.HasExited) {
            return $null
        }
        if ($null -ne $Root -and
            -not [string]::IsNullOrWhiteSpace($AboutDescription)) {
            $records = @(Get-UiaRecords -Root $Root)
            $dialogVisible = @($records | Where-Object {
                $_.name -ceq $AboutDescription -and -not $_.isOffscreen
            })
            if ($dialogVisible.Count -gt 0) {
                Wait-InvokeUiaButton `
                    -Root $Root `
                    -Name "Close" `
                    -PreferBottom `
                    -Deadline (Get-Date).AddSeconds(3)
                Start-Sleep -Milliseconds 250
                $Process.Refresh()
                if ($Process.HasExited) {
                    return $null
                }
            }
        }
        if ($Process.MainWindowHandle -eq [IntPtr]::Zero) {
            return "$Label PID $($Process.Id) remains running without a visible window."
        }
        [void]$Process.CloseMainWindow()
        if (-not $Process.WaitForExit(5000)) {
            return "$Label PID $($Process.Id) did not exit during graceful cleanup."
        }
        return $null
    }
    catch {
        return "Unable to clean up $Label PID $($Process.Id): $($_.Exception.Message)"
    }
}

$candidateMachine = Get-PeMachine -Path $resolvedExecutable
if ($candidateMachine -ne $arm64PeMachine) {
    throw ("Reactor candidate must be ARM64 PE machine 0xAA64; found 0x{0:X4}." -f
        $candidateMachine)
}
Assert-NotRunning `
    -ExecutablePath $resolvedExecutable `
    -Label "Reactor candidate"
$candidateVersion = Get-ReactorApplicationVersion -Path $resolvedExecutable
if ($candidateVersion -cne $oracleVersion) {
    throw "Reactor executable reports '$candidateVersion'; the About oracle requires '$oracleVersion'."
}

$storeApplication = Get-OracleStoreApplication
Assert-NotRunning `
    -ExecutablePath $storeApplication.ExecutablePath `
    -Label "Exact Store oracle"
Assert-NotRunning `
    -ExecutablePath $resolvedExecutable `
    -Label "Reactor candidate"

$storeProcess = $null
$nativeProcess = $null
$storeRoot = $null
$nativeRoot = $null
$storeAbout = $null
$nativeAbout = $null
$storeClose = $null
$nativeClose = $null
$storeMetadata = $null
$nativeMetadata = $null
$runtimeEvidence = $null
$nativeInitialHeaderFocus = $null
$nativeBehavior = $null
$validationError = $null
$cleanupErrors = @()

try {
    Start-Process explorer.exe -ArgumentList "shell:AppsFolder\$($storeApplication.Aumid)"
    $storeProcess = Wait-ExactWindowByPath `
        -ExecutablePath $storeApplication.ExecutablePath `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $storeMeasurement = Set-ExactCaptureViewport `
        -Process $storeProcess `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $storeRoot = [Windows.Automation.AutomationElement]::FromHandle(
        $storeProcess.MainWindowHandle)
    Wait-InvokeUiaButton `
        -Root $storeRoot `
        -Name "Diagnostics" `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    Start-Sleep -Milliseconds 300
    Wait-InvokeUiaButton `
        -Root $storeRoot `
        -Name "About" `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $storeAbout = Wait-AboutEvidence `
        -Root $storeRoot `
        -WindowMeasurement $storeMeasurement `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $storeCapture = Save-ForegroundCapture `
        -Process $storeProcess `
        -Measurement $storeMeasurement `
        -LogicalPath $storeLogicalPath `
        -PhysicalPath $storePhysicalPath `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $storePixels = Assert-DialogPixels `
        -PhysicalPath $storePhysicalPath `
        -Capture $storeCapture `
        -AboutEvidence $storeAbout
    $storeClose = Close-AboutAndProcess `
        -Process $storeProcess `
        -Root $storeRoot `
        -AboutEvidence $storeAbout `
        -Label "Store oracle" `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)

    $storeMetadata = [ordered]@{
        capturedAtUtc = [DateTime]::UtcNow.ToString("o")
        role = "source-left"
        package = [ordered]@{
            name = $storeApplication.Package.Name
            fullName = $storeApplication.Package.PackageFullName
            familyName = $storeApplication.Package.PackageFamilyName
            publisherId = $storeApplication.Package.PublisherId
            version = $storeApplication.Package.Version.ToString()
            architecture = $storeApplication.Package.Architecture.ToString()
            signatureKind = $storeApplication.Package.SignatureKind.ToString()
            installLocation = $storeApplication.Package.InstallLocation
            manifest = $storeApplication.ManifestPath
            applicationId = $storeApplication.ApplicationId
            aumid = $storeApplication.Aumid
        }
        executable = $storeApplication.ExecutablePath
        executableSha256 = (
            Get-FileHash $storeApplication.ExecutablePath -Algorithm SHA256).Hash
        pid = $storeProcess.Id
        openMethod = "Exact executable/PID root + UI Automation InvokePattern on About"
        uiAutomation = [ordered]@{
            requiredControlNames = $storeAbout.requiredControlNames
            requiredDescription = $storeAbout.requiredDescription
            selectedDialogRecords = @(
                $storeAbout.dialogWindow,
                $storeAbout.dialogTitle,
                $storeAbout.applicationTitle,
                $storeAbout.version,
                $storeAbout.description,
                $storeAbout.windowsForumButton,
                $storeAbout.githubButton,
                $storeAbout.closeButton)
            allRecords = $storeAbout.allRecords
        }
        capture = $storeCapture
        dialogPixelValidation = $storePixels
        gracefulClose = $storeClose
    }
    Write-JsonFile -Value $storeMetadata -Path $storeMetadataPath

    $environmentNames = @(
        "WFDIAG_REACTOR_PAGE",
        "WFDIAG_REACTOR_VISUAL_STATE",
        "WFDIAG_REACTOR_FIXTURE",
        "WFDIAG_REACTOR_SETTINGS",
        "WFDIAG_REACTOR_SETTINGS_TEST_PATH",
        "WFDIAG_REACTOR_WIDTH",
        "WFDIAG_REACTOR_HEIGHT",
        $versionProbeEnvironment
    )
    $savedEnvironment = @{}
    foreach ($name in $environmentNames) {
        $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable(
            $name,
            "Process")
    }
    try {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable($name, $null, "Process")
        }
        [Environment]::SetEnvironmentVariable(
            "WFDIAG_REACTOR_PAGE", "diagnostics", "Process")
        [Environment]::SetEnvironmentVariable(
            "WFDIAG_REACTOR_WIDTH", [string]$logicalWidth, "Process")
        [Environment]::SetEnvironmentVariable(
            "WFDIAG_REACTOR_HEIGHT", [string]$logicalHeight, "Process")
        $nativeProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    }
    finally {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable(
                $name,
                $savedEnvironment[$name],
                "Process")
        }
    }

    $nativeProcess = Wait-ProcessWindow `
        -Process $nativeProcess `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $nativeProcess.Refresh()
    if (-not [string]::Equals(
        $nativeProcess.Path,
        $resolvedExecutable,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "Started PID path does not match the supplied Reactor executable."
    }
    $nativeMeasurement = Set-ExactCaptureViewport `
        -Process $nativeProcess `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $nativeRoot = [Windows.Automation.AutomationElement]::FromHandle(
        $nativeProcess.MainWindowHandle)
    Wait-InvokeUiaButton `
        -Root $nativeRoot `
        -Name "Diagnostics" `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    Start-Sleep -Milliseconds 300
    Wait-InvokeUiaButton `
        -Root $nativeRoot `
        -Name "About" `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $nativeAbout = Wait-AboutEvidence `
        -Root $nativeRoot `
        -WindowMeasurement $nativeMeasurement `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $nativeInitialHeaderFocus = Measure-InitialHeaderCloseFocus `
        -Process $nativeProcess `
        -Root $nativeRoot `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $runtimeEvidence = Get-NativeRuntimeEvidence `
        -Process $nativeProcess `
        -CandidateDirectory $releaseDirectory
    $nativeCapture = Save-ForegroundCapture `
        -Process $nativeProcess `
        -Measurement $nativeMeasurement `
        -LogicalPath $nativeLogicalPath `
        -PhysicalPath $nativePhysicalPath `
        -Deadline (Get-Date).AddSeconds($WaitSeconds)
    $nativePixels = Assert-DialogPixels `
        -PhysicalPath $nativePhysicalPath `
        -Capture $nativeCapture `
        -AboutEvidence $nativeAbout
    $nativeBehavior = Invoke-NativeAboutBehaviorValidation `
        -Process $nativeProcess `
        -Root $nativeRoot `
        -WindowMeasurement $nativeMeasurement `
        -InitialAbout $nativeAbout `
        -InitialHeaderFocus $nativeInitialHeaderFocus `
        -TimeoutSeconds $WaitSeconds
    $nativeClose = Close-MainProcessAfterBehavior `
        -Process $nativeProcess `
        -Label "Reactor candidate"

    $nativeMetadata = [ordered]@{
        capturedAtUtc = [DateTime]::UtcNow.ToString("o")
        role = "native-right"
        executable = $resolvedExecutable
        executableSha256 = (
            Get-FileHash $resolvedExecutable -Algorithm SHA256).Hash
        peMachine = ("0x{0:X4}" -f $candidateMachine)
        architecture = "ARM64"
        applicationVersion = $candidateVersion
        pid = $nativeProcess.Id
        fixtureEnvironmentCleared = $true
        openMethod = "Exact supplied PID root + UI Automation InvokePattern on About"
        uiAutomation = [ordered]@{
            requiredControlNames = $nativeAbout.requiredControlNames
            requiredDescription = $nativeAbout.requiredDescription
            selectedDialogRecords = @(
                $nativeAbout.dialogWindow,
                $nativeAbout.dialogTitle,
                $nativeAbout.applicationTitle,
                $nativeAbout.version,
                $nativeAbout.description,
                $nativeAbout.windowsForumButton,
                $nativeAbout.githubButton,
                $nativeAbout.closeButton)
            allRecords = $nativeAbout.allRecords
        }
        capture = $nativeCapture
        dialogPixelValidation = $nativePixels
        runtime = $runtimeEvidence
        behaviorValidation = $nativeBehavior
        gracefulClose = $nativeClose
    }
    Write-JsonFile -Value $nativeMetadata -Path $nativeMetadataPath

    New-CombinedImage `
        -LeftPath $storeLogicalPath `
        -RightPath $nativeLogicalPath `
        -OutputPath $combinedPath
}
catch {
    $validationError = $_.Exception.Message
}
finally {
    $nativeCleanup = Try-GracefulCleanup `
        -Process $nativeProcess `
        -Label "Reactor candidate" `
        -Root $nativeRoot `
        -AboutDescription $(if ($null -ne $nativeAbout) {
            $nativeAbout.requiredDescription
        }
        else { $null })
    if (-not [string]::IsNullOrWhiteSpace($nativeCleanup)) {
        $cleanupErrors += $nativeCleanup
    }
    $storeCleanup = Try-GracefulCleanup `
        -Process $storeProcess `
        -Label "Store oracle" `
        -Root $storeRoot `
        -AboutDescription $(if ($null -ne $storeAbout) {
            $storeAbout.requiredDescription
        }
        else { $null })
    if (-not [string]::IsNullOrWhiteSpace($storeCleanup)) {
        $cleanupErrors += $storeCleanup
    }
}

Start-Sleep -Seconds 2
$crashEvents = @()
$eventLogError = $null
try {
    $crashEvents = @(Get-CrashEvents `
        -ExecutablePaths @(
            $storeApplication.ExecutablePath,
            $resolvedExecutable) `
        -StartTime $startedAt)
}
catch {
    $eventLogError = $_.Exception.Message
}

$failures = @()
if (-not [string]::IsNullOrWhiteSpace($validationError)) {
    $failures += $validationError
}
if ($null -ne $nativeBehavior -and -not $nativeBehavior.passed) {
    $failures += @($nativeBehavior.failures)
}
elseif ($null -eq $nativeBehavior -and
    $null -ne $nativeInitialHeaderFocus -and
    -not $nativeInitialHeaderFocus.passed) {
    $failures += (
        "Initial About focus was not automationId 'about-close'; behavior validation did not complete.")
}
$failures += $cleanupErrors
if (-not [string]::IsNullOrWhiteSpace($eventLogError)) {
    $failures += $eventLogError
}
if ($crashEvents.Count -gt 0) {
    $failures += "Windows recorded $($crashEvents.Count) Application Error/WER event(s) for the validated executables."
}

$summary = [ordered]@{
    completedAtUtc = [DateTime]::UtcNow.ToString("o")
    passed = $failures.Count -eq 0
    oracle = [ordered]@{
        packageFullName = $storeApplication.Package.PackageFullName
        packageFamilyName = $storeApplication.Package.PackageFamilyName
        version = $storeApplication.Package.Version.ToString()
        architecture = $storeApplication.Package.Architecture.ToString()
        signatureKind = $storeApplication.Package.SignatureKind.ToString()
        aumid = $storeApplication.Aumid
    }
    candidate = [ordered]@{
        executable = $resolvedExecutable
        executableSha256 = (
            Get-FileHash $resolvedExecutable -Algorithm SHA256).Hash
        applicationVersion = $candidateVersion
        peMachine = ("0x{0:X4}" -f $candidateMachine)
        localXaml = if ($null -ne $runtimeEvidence) {
            $runtimeEvidence.localXaml
        }
        else {
            $null
        }
        webView = if ($null -ne $runtimeEvidence) {
            $runtimeEvidence.webView
        }
        else {
            $null
        }
    }
    viewport = [ordered]@{
        logicalWidth = $logicalWidth
        logicalHeight = $logicalHeight
        dpi = $requiredDpi
        scale = $requiredDpi / 96.0
        physicalWidth = $physicalWidth
        physicalHeight = $physicalHeight
    }
    artifacts = [ordered]@{
        sourceLogical = if (Test-Path -LiteralPath $storeLogicalPath) {
            $storeLogicalPath
        }
        else { $null }
        sourcePhysical = if (Test-Path -LiteralPath $storePhysicalPath) {
            $storePhysicalPath
        }
        else { $null }
        sourceMetadata = if (Test-Path -LiteralPath $storeMetadataPath) {
            $storeMetadataPath
        }
        else { $null }
        nativeLogical = if (Test-Path -LiteralPath $nativeLogicalPath) {
            $nativeLogicalPath
        }
        else { $null }
        nativePhysical = if (Test-Path -LiteralPath $nativePhysicalPath) {
            $nativePhysicalPath
        }
        else { $null }
        nativeMetadata = if (Test-Path -LiteralPath $nativeMetadataPath) {
            $nativeMetadataPath
        }
        else { $null }
        combined = if (Test-Path -LiteralPath $combinedPath) {
            $combinedPath
        }
        else { $null }
    }
    gracefulClose = [ordered]@{
        store = $storeClose
        native = $nativeClose
        cleanupErrors = $cleanupErrors
    }
    behaviorValidation = $nativeBehavior
    applicationErrorAndWer = [ordered]@{
        queryStartUtc = $startedAt.ToUniversalTime().ToString("o")
        queryError = $eventLogError
        events = $crashEvents
    }
    failures = $failures
}
Write-JsonFile -Value $summary -Path $summaryPath

$result = [pscustomobject]@{
    passed = $failures.Count -eq 0
    outputDirectory = $OutputDirectory
    sourceCapture = if (Test-Path -LiteralPath $storeLogicalPath) {
        $storeLogicalPath
    }
    else { $null }
    nativeCapture = if (Test-Path -LiteralPath $nativeLogicalPath) {
        $nativeLogicalPath
    }
    else { $null }
    combinedCapture = if (Test-Path -LiteralPath $combinedPath) {
        $combinedPath
    }
    else { $null }
    summary = $summaryPath
    crashEventCount = $crashEvents.Count
    behaviorPassed = if ($null -ne $nativeBehavior) {
        $nativeBehavior.passed
    }
    else {
        $false
    }
    failures = $failures
}
$result | ConvertTo-Json -Depth 6
if ($failures.Count -gt 0) {
    exit 1
}
