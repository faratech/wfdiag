<#
.SYNOPSIS
Captures repeatable visual baselines from the installed Microsoft Store build
of WindowsForum Diagnostics.

.DESCRIPTION
The script resolves the Store package and AUMID from the installed manifest,
launches the package through shell:AppsFolder, finds the package-owned top-level
window, sizes its DWM-visible frame to an exact logical size, drives stable app
navigation with the application's keyboard shortcuts, and writes density-
normalized PNG files plus capture metadata.

It deliberately never clears scan history, enters credentials, sends an AI
prompt, or overwrites the old .playwright-mcp reference images. States that
depend on prepared scan/history/chat data must be explicitly acknowledged with
-TrustPreparedState.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\capture-store-baselines.ps1 `
  -State diagnostics-empty-desktop-dark `
  -OutputDirectory C:\Temp\wfdiag-store-2.5.8 `
  -RestartApplication

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\capture-store-baselines.ps1 `
  -CaptureAllFeasible `
  -OutputDirectory C:\Temp\wfdiag-store-2.5.8 `
  -RestartApplication
#>

[CmdletBinding(DefaultParameterSetName = "Capture")]
param(
    [Parameter(ParameterSetName = "Capture")]
    [string[]]$State = @("diagnostics-empty-desktop-dark"),

    [Parameter(ParameterSetName = "Capture")]
    [switch]$CaptureAllFeasible,

    [Parameter(ParameterSetName = "List")]
    [switch]$ListStates,

    [Parameter(ParameterSetName = "Capture")]
    [string]$OutputDirectory = "C:\Temp\wfdiag-store-2.5.8",

    [string]$PackageName = "32827MikeFara.WindowsForumDiagnostics",

    [string]$ExpectedVersion = "2.5.8.0",

    [int]$WaitSeconds = 25,

    [Parameter(ParameterSetName = "Capture")]
    [switch]$RestartApplication,

    [Parameter(ParameterSetName = "Capture")]
    [switch]$TrustPreparedState,

    [Parameter(ParameterSetName = "Capture")]
    [switch]$KeepPhysicalCapture,

    [Parameter(ParameterSetName = "Capture")]
    [switch]$KeepApplicationOpen
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

$stateCatalog = [ordered]@{
    "diagnostics-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^1"; WaitMs = 500
        Prepared = $false
        Requirement = "Fresh in-memory session with scan-on-startup disabled. Use -RestartApplication."
        Action = "page"
    }
    "monitor-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^2"; WaitMs = 1
        Prepared = $false
        Requirement = "Race-sensitive loading state; inspect the result because fast hosts may populate immediately."
        Action = "page"
    }
    "processes-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^3"; WaitMs = 1
        Prepared = $false
        Requirement = "Race-sensitive loading state; inspect the result because fast hosts may populate immediately."
        Action = "page"
    }
    "ai-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^4"; WaitMs = 700
        Prepared = $false
        Requirement = "Fresh in-memory AI session. No prompt is sent by this script."
        Action = "page"
    }
    "issues-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^5"; WaitMs = 500
        Prepared = $false
        Requirement = "Fresh in-memory session with no completed scan."
        Action = "page"
    }
    "history-empty-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 1000; Shortcut = "^6"; WaitMs = 700
        Prepared = $true
        Requirement = "The user's scan-history store must already be empty; this script never deletes it."
        Action = "page"
    }
    "ai-empty-compact-dark" = [pscustomobject]@{
        Width = 900; Height = 800; Shortcut = "^4"; WaitMs = 700
        Prepared = $false
        Requirement = "Fresh in-memory AI session. No prompt is sent by this script."
        Action = "page"
    }
    "diagnostics-populated-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^1"; WaitMs = 700
        Prepared = $true
        Requirement = "A representative scan must already be complete in the current app session."
        Action = "page"
    }
    "issues-populated-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^5"; WaitMs = 700
        Prepared = $true
        Requirement = "A representative scan with detected issues must already be complete."
        Action = "page"
    }
    "issue-to-chat-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^4"; WaitMs = 700
        Prepared = $true
        Requirement = "Use Ask AI on a detected issue first. The script never sends a prompt or invokes a provider."
        Action = "page"
    }
    "processes-populated-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^3"; WaitMs = 3000
        Prepared = $false
        Requirement = "Process inventory populates automatically; wait is built in."
        Action = "page"
    }
    "history-comparison-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^6"; WaitMs = 700
        Prepared = $true
        Requirement = "Two saved scans must already exist and a comparison must already be selected."
        Action = "page"
    }
    "monitor-populated-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^2"; WaitMs = 3500
        Prepared = $false
        Requirement = "Live telemetry populates automatically; wait is built in."
        Action = "page"
    }
    "ai-conversation-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^4"; WaitMs = 700
        Prepared = $true
        Requirement = "A non-sensitive test conversation must already exist in the current app session."
        Action = "chat-current"
    }
    "ai-conversation-top-compact-dark" = [pscustomobject]@{
        Width = 900; Height = 800; Shortcut = "^4"; WaitMs = 700
        Prepared = $true
        Requirement = "A non-sensitive test conversation must already exist in the current app session."
        Action = "chat-top"
    }
    "ai-conversation-bottom-compact-dark" = [pscustomobject]@{
        Width = 900; Height = 800; Shortcut = "^4"; WaitMs = 700
        Prepared = $true
        Requirement = "A non-sensitive test conversation must already exist in the current app session."
        Action = "chat-bottom"
    }
    "settings-top-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^2"; WaitMs = 900
        Prepared = $false
        Requirement = "Settings opens over Live Monitor, matching the reference context; no value is changed."
        Action = "settings-top"
    }
    "settings-bottom-desktop-dark" = [pscustomobject]@{
        Width = 1440; Height = 900; Shortcut = "^2"; WaitMs = 900
        Prepared = $false
        Requirement = "Settings opens over Live Monitor and its body is scrolled; no value is changed."
        Action = "settings-bottom"
    }
}

$feasibleStates = @(
    "diagnostics-empty-desktop-dark",
    "monitor-empty-desktop-dark",
    "processes-empty-desktop-dark",
    "ai-empty-desktop-dark",
    "issues-empty-desktop-dark",
    "ai-empty-compact-dark",
    "processes-populated-desktop-dark",
    "monitor-populated-desktop-dark",
    "settings-top-desktop-dark",
    "settings-bottom-desktop-dark"
)

if ($ListStates) {
    foreach ($entry in $stateCatalog.GetEnumerator()) {
        [pscustomobject]@{
            State = $entry.Key
            LogicalSize = "{0}x{1}" -f $entry.Value.Width, $entry.Value.Height
            RequiresPreparedState = $entry.Value.Prepared
            Requirement = $entry.Value.Requirement
        }
    }
    return
}

if ($CaptureAllFeasible) {
    $State = $feasibleStates
}

foreach ($stateId in $State) {
    if (-not $stateCatalog.Contains($stateId)) {
        throw "Unknown state '$stateId'. Run with -ListStates to see supported state IDs."
    }
    if ($stateCatalog[$stateId].Prepared -and -not $TrustPreparedState) {
        throw "State '$stateId' requires prepared application data: $($stateCatalog[$stateId].Requirement) Re-run with -TrustPreparedState after verifying that precondition."
    }
}

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class WfDiagStoreCaptureNative
{
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
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

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

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PrintWindow(IntPtr hwnd, IntPtr targetDc, uint flags);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool ShowWindow(IntPtr hwnd, int command);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    public static extern void mouse_event(
        uint flags,
        uint dx,
        uint dy,
        int data,
        UIntPtr extraInfo);
}
"@

# PowerShell itself is not guaranteed to be per-monitor DPI aware. Without a
# thread context, GetWindowRect can return DPI-virtualized logical coordinates
# while DwmGetWindowAttribute returns physical coordinates; mixing the two
# makes exact sizing oscillate. PER_MONITOR_AWARE_V2 keeps every Win32 metric
# in physical pixels for this script.
$previousDpiContext = [WfDiagStoreCaptureNative]::SetThreadDpiAwarenessContext([IntPtr](-4))
if ($previousDpiContext -eq [IntPtr]::Zero) {
    throw "Unable to enter the per-monitor DPI-aware thread context required for exact capture sizing."
}

function Get-InstalledStoreApplication {
    $package = Get-AppxPackage -Name $PackageName |
        Sort-Object -Property Version -Descending |
        Select-Object -First 1
    if ($null -eq $package) {
        throw "The Microsoft Store package '$PackageName' is not installed for the current Windows user."
    }

    if ($package.SignatureKind.ToString() -ne "Store") {
        throw "Package '$($package.PackageFullName)' is not Store-signed (SignatureKind=$($package.SignatureKind))."
    }

    $expected = [Version]$ExpectedVersion
    if ($package.Version -ne $expected) {
        throw "Expected Store version $expected but found $($package.Version) at '$($package.InstallLocation)'."
    }

    $manifestPath = Join-Path $package.InstallLocation "AppxManifest.xml"
    [xml]$manifest = Get-Content -LiteralPath $manifestPath
    $application = $manifest.SelectSingleNode("/*[local-name()='Package']/*[local-name()='Applications']/*[local-name()='Application'][1]")
    if ($null -eq $application -or [string]::IsNullOrWhiteSpace($application.Id)) {
        throw "No Application Id was found in '$manifestPath'."
    }

    $executable = $application.Executable
    if ([string]::IsNullOrWhiteSpace($executable)) {
        throw "No executable was found for Application '$($application.Id)' in '$manifestPath'."
    }

    [pscustomobject]@{
        Package = $package
        ApplicationId = $application.Id
        Aumid = "$($package.PackageFamilyName)!$($application.Id)"
        Executable = $executable
        ExecutablePath = Join-Path $package.InstallLocation $executable
        ProcessName = [IO.Path]::GetFileNameWithoutExtension($executable)
    }
}

function Find-StoreWindowProcess {
    param([Parameter(Mandatory = $true)]$StoreApplication)

    Get-Process -Name $StoreApplication.ProcessName -ErrorAction SilentlyContinue |
        Where-Object {
            $_.MainWindowHandle -ne [IntPtr]::Zero -and
            $_.Path -eq $StoreApplication.ExecutablePath
        } |
        Select-Object -First 1
}

function Wait-ForStoreWindow {
    param(
        [Parameter(Mandatory = $true)]$StoreApplication,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    do {
        $candidate = Find-StoreWindowProcess -StoreApplication $StoreApplication
        if ($null -ne $candidate) {
            return $candidate
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $Deadline)

    throw "No visible package-owned window for '$($StoreApplication.Aumid)' appeared before the deadline."
}

function Get-WindowBounds {
    param([Parameter(Mandatory = $true)][IntPtr]$Hwnd)

    $window = New-Object WfDiagStoreCaptureNative+Rect
    if (-not [WfDiagStoreCaptureNative]::GetWindowRect($Hwnd, [ref]$window)) {
        throw "GetWindowRect failed for HWND $Hwnd."
    }

    $visible = New-Object WfDiagStoreCaptureNative+Rect
    $hr = [WfDiagStoreCaptureNative]::DwmGetWindowAttribute(
        $Hwnd,
        9, # DWMWA_EXTENDED_FRAME_BOUNDS
        [ref]$visible,
        [Runtime.InteropServices.Marshal]::SizeOf($visible))
    if ($hr -ne 0) {
        $visible = $window
    }

    [pscustomobject]@{
        Window = $window
        Visible = $visible
        WindowWidth = $window.Right - $window.Left
        WindowHeight = $window.Bottom - $window.Top
        VisibleWidth = $visible.Right - $visible.Left
        VisibleHeight = $visible.Bottom - $visible.Top
    }
}

function Focus-StoreWindow {
    param([Parameter(Mandatory = $true)][IntPtr]$Hwnd)

    $targetProcessId = [uint32]0
    [void][WfDiagStoreCaptureNative]::GetWindowThreadProcessId($Hwnd, [ref]$targetProcessId)
    if ($targetProcessId -ne 0) {
        $shell = New-Object -ComObject WScript.Shell
        [void]$shell.AppActivate([int]$targetProcessId)
    }
    [void][WfDiagStoreCaptureNative]::ShowWindow($Hwnd, 9) # SW_RESTORE
    [void][WfDiagStoreCaptureNative]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 250
}

function Set-ExactLogicalWindowSize {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][int]$LogicalWidth,
        [Parameter(Mandatory = $true)][int]$LogicalHeight
    )

    Focus-StoreWindow -Hwnd $Hwnd
    $dpi = [WfDiagStoreCaptureNative]::GetDpiForWindow($Hwnd)
    if ($dpi -eq 0) { $dpi = 96 }

    $targetVisibleWidth = [int][Math]::Round($LogicalWidth * $dpi / 96.0)
    $targetVisibleHeight = [int][Math]::Round($LogicalHeight * $dpi / 96.0)

    for ($attempt = 0; $attempt -lt 4; $attempt++) {
        $bounds = Get-WindowBounds -Hwnd $Hwnd
        $extraWidth = $bounds.WindowWidth - $bounds.VisibleWidth
        $extraHeight = $bounds.WindowHeight - $bounds.VisibleHeight
        $leftInset = $bounds.Visible.Left - $bounds.Window.Left
        $topInset = $bounds.Visible.Top - $bounds.Window.Top
        $outerWidth = $targetVisibleWidth + $extraWidth
        $outerHeight = $targetVisibleHeight + $extraHeight

        # Keep the entire visible frame on-screen so CopyFromScreen can capture
        # the GPU-composited WebView content. The thread is explicitly PMv2
        # aware above, so these coordinates and all bounds are physical pixels.
        $outerX = -$leftInset
        $outerY = -$topInset
        if (-not [WfDiagStoreCaptureNative]::SetWindowPos(
            $Hwnd,
            [IntPtr]::Zero,
            $outerX,
            $outerY,
            $outerWidth,
            $outerHeight,
            0x0014)) { # SWP_NOZORDER | SWP_NOACTIVATE
            $win32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "SetWindowPos failed for logical ${LogicalWidth}x${LogicalHeight} (Win32 $win32Error)."
        }
        Start-Sleep -Milliseconds 350

        $measured = Get-WindowBounds -Hwnd $Hwnd
        Write-Verbose (
            "Sizing attempt {0}: dpi={1}, requested visible={2}x{3}, requested outer={4}x{5}, measured outer={6}x{7}, measured visible={8}x{9}" -f
            ($attempt + 1), $dpi, $targetVisibleWidth, $targetVisibleHeight,
            $outerWidth, $outerHeight, $measured.WindowWidth, $measured.WindowHeight,
            $measured.VisibleWidth, $measured.VisibleHeight)
        if ($measured.VisibleWidth -eq $targetVisibleWidth -and $measured.VisibleHeight -eq $targetVisibleHeight) {
            return [pscustomobject]@{
                Dpi = $dpi
                Scale = $dpi / 96.0
                Bounds = $measured
            }
        }
    }

    $final = Get-WindowBounds -Hwnd $Hwnd
    throw "Unable to obtain exact visible bounds for logical ${LogicalWidth}x${LogicalHeight}: expected ${targetVisibleWidth}x${targetVisibleHeight} physical, measured outer $($final.WindowWidth)x$($final.WindowHeight) and visible $($final.VisibleWidth)x$($final.VisibleHeight) at DPI $dpi."
}

function Send-AppKeys {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][string]$Keys,
        [int]$WaitMs = 250
    )

    Focus-StoreWindow -Hwnd $Hwnd
    [Windows.Forms.SendKeys]::SendWait($Keys)
    if ($WaitMs -gt 0) { Start-Sleep -Milliseconds $WaitMs }
}

function Set-LogicalCursorPosition {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][int]$LogicalX,
        [Parameter(Mandatory = $true)][int]$LogicalY
    )

    $dpi = [WfDiagStoreCaptureNative]::GetDpiForWindow($Hwnd)
    if ($dpi -eq 0) { $dpi = 96 }
    $bounds = Get-WindowBounds -Hwnd $Hwnd
    $x = $bounds.Visible.Left + [int][Math]::Round($LogicalX * $dpi / 96.0)
    $y = $bounds.Visible.Top + [int][Math]::Round($LogicalY * $dpi / 96.0)
    [void][WfDiagStoreCaptureNative]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 100
}

function Send-MouseWheel {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][int]$LogicalX,
        [Parameter(Mandatory = $true)][int]$LogicalY,
        [Parameter(Mandatory = $true)][int]$Notches
    )

    Focus-StoreWindow -Hwnd $Hwnd
    Set-LogicalCursorPosition -Hwnd $Hwnd -LogicalX $LogicalX -LogicalY $LogicalY
    $direction = [Math]::Sign($Notches)
    for ($i = 0; $i -lt [Math]::Abs($Notches); $i++) {
        [WfDiagStoreCaptureNative]::mouse_event(0x0800, 0, 0, 120 * $direction, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 25
    }
    Start-Sleep -Milliseconds 350
}

function Send-LogicalClick {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][int]$LogicalX,
        [Parameter(Mandatory = $true)][int]$LogicalY
    )

    Focus-StoreWindow -Hwnd $Hwnd
    Set-LogicalCursorPosition -Hwnd $Hwnd -LogicalX $LogicalX -LogicalY $LogicalY
    [WfDiagStoreCaptureNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # LEFTDOWN
    Start-Sleep -Milliseconds 40
    [WfDiagStoreCaptureNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # LEFTUP
    Start-Sleep -Milliseconds 120
}

function Open-SettingsDialog {
    param([Parameter(Mandatory = $true)][IntPtr]$Hwnd)

    # At the fixed 1440x900 settings baseline size, the footer is stable. The
    # icon column remains at x=35 whether the user's rail preference is open
    # or collapsed, so this works in both layouts without clipboard/text input.
    Send-LogicalClick -Hwnd $Hwnd -LogicalX 35 -LogicalY 698
    Start-Sleep -Milliseconds 850
}

function Set-CaptureState {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)]$Definition
    )

    # Close a preceding modal/palette before resizing or navigating. Escape is
    # inert on ordinary pages and does not mutate app data.
    Send-AppKeys -Hwnd $Hwnd -Keys "{ESC}" -WaitMs 400
    $size = Set-ExactLogicalWindowSize `
        -Hwnd $Hwnd `
        -LogicalWidth $Definition.Width `
        -LogicalHeight $Definition.Height

    if (-not [string]::IsNullOrWhiteSpace($Definition.Shortcut)) {
        Send-AppKeys -Hwnd $Hwnd -Keys $Definition.Shortcut -WaitMs 0
    }

    switch ($Definition.Action) {
        "settings-top" {
            Start-Sleep -Milliseconds 450
            Open-SettingsDialog -Hwnd $Hwnd
        }
        "settings-bottom" {
            Start-Sleep -Milliseconds 450
            Open-SettingsDialog -Hwnd $Hwnd
            Send-MouseWheel -Hwnd $Hwnd -LogicalX 720 -LogicalY 450 -Notches -28
        }
        "chat-top" {
            Send-MouseWheel -Hwnd $Hwnd -LogicalX 450 -LogicalY 390 -Notches 28
        }
        "chat-bottom" {
            Send-MouseWheel -Hwnd $Hwnd -LogicalX 450 -LogicalY 390 -Notches -28
        }
    }

    if ($Definition.WaitMs -gt 0) {
        Start-Sleep -Milliseconds $Definition.WaitMs
    }
    return $size
}

function Save-WindowCapture {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Hwnd,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][int]$LogicalWidth,
        [Parameter(Mandatory = $true)][int]$LogicalHeight,
        [switch]$SavePhysical
    )

    $bounds = Get-WindowBounds -Hwnd $Hwnd
    $absoluteOutput = [IO.Path]::GetFullPath($OutputPath)
    $directory = [IO.Path]::GetDirectoryName($absoluteOutput)
    [IO.Directory]::CreateDirectory($directory) | Out-Null

    $visibleBitmap = $null
    $normalizedBitmap = $null
    try {
        Focus-StoreWindow -Hwnd $Hwnd
        Start-Sleep -Milliseconds 350

        # PrintWindow can report success after WebView2 switches to its GPU
        # composition surface while returning only black pixels. This helper
        # owns the foreground and keeps the exact frame on-screen, so a screen
        # copy is the reliable source for the signed Store application.
        $visibleBitmap = New-Object Drawing.Bitmap(
            $bounds.VisibleWidth,
            $bounds.VisibleHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $visibleGraphics = [Drawing.Graphics]::FromImage($visibleBitmap)
        try {
            $visibleGraphics.CopyFromScreen(
                $bounds.Visible.Left,
                $bounds.Visible.Top,
                0,
                0,
                (New-Object Drawing.Size($bounds.VisibleWidth, $bounds.VisibleHeight)),
                [Drawing.CopyPixelOperation]::SourceCopy)
        }
        finally {
            $visibleGraphics.Dispose()
        }

        if ($SavePhysical) {
            $physicalPath = [IO.Path]::Combine(
                $directory,
                ([IO.Path]::GetFileNameWithoutExtension($absoluteOutput) + ".physical.png"))
            $visibleBitmap.Save($physicalPath, [Drawing.Imaging.ImageFormat]::Png)
        }

        $normalizedBitmap = New-Object Drawing.Bitmap(
            $LogicalWidth,
            $LogicalHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $normalizedGraphics = [Drawing.Graphics]::FromImage($normalizedBitmap)
        try {
            $normalizedGraphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $normalizedGraphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $normalizedGraphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $normalizedGraphics.DrawImage($visibleBitmap, 0, 0, $LogicalWidth, $LogicalHeight)
        }
        finally {
            $normalizedGraphics.Dispose()
        }
        $normalizedBitmap.Save($absoluteOutput, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        if ($null -ne $normalizedBitmap) { $normalizedBitmap.Dispose() }
        if ($null -ne $visibleBitmap) { $visibleBitmap.Dispose() }
    }

    return $absoluteOutput
}

$storeApplication = Get-InstalledStoreApplication
$existing = Find-StoreWindowProcess -StoreApplication $storeApplication

if ($RestartApplication -and $null -ne $existing) {
    [void]$existing.CloseMainWindow()
    try {
        Wait-Process -Id $existing.Id -Timeout 8 -ErrorAction Stop
    }
    catch {
        # Explicit -RestartApplication grants permission to terminate this one
        # exact Store executable after a graceful close timed out.
        Stop-Process -Id $existing.Id -Force -ErrorAction Stop
        Wait-Process -Id $existing.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    $existing = $null
}

if ($null -eq $existing) {
    Start-Process explorer.exe -ArgumentList "shell:AppsFolder\$($storeApplication.Aumid)"
}

$windowProcess = Wait-ForStoreWindow `
    -StoreApplication $storeApplication `
    -Deadline (Get-Date).AddSeconds($WaitSeconds)
$hwnd = $windowProcess.MainWindowHandle

Write-Verbose "Package: $($storeApplication.Package.PackageFullName)"
Write-Verbose "AUMID: $($storeApplication.Aumid)"
Write-Verbose "Process: $($windowProcess.Id) $($storeApplication.ExecutablePath)"

$captureResults = @()
try {
    foreach ($stateId in $State) {
        $definition = $stateCatalog[$stateId]
        Write-Host "Capturing $stateId ($($definition.Width)x$($definition.Height))"
        Write-Verbose "Requirement: $($definition.Requirement)"

        $measurement = Set-CaptureState -Hwnd $hwnd -Definition $definition
        $outputPath = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) "$stateId.png"
        $savedPath = Save-WindowCapture `
            -Hwnd $hwnd `
            -OutputPath $outputPath `
            -LogicalWidth $definition.Width `
            -LogicalHeight $definition.Height `
            -SavePhysical:$KeepPhysicalCapture

        $metadataPath = [IO.Path]::ChangeExtension($savedPath, ".capture.json")
        $metadata = [ordered]@{
            state = $stateId
            captured_at_utc = [DateTime]::UtcNow.ToString("o")
            package_full_name = $storeApplication.Package.PackageFullName
            package_family_name = $storeApplication.Package.PackageFamilyName
            package_version = $storeApplication.Package.Version.ToString()
            architecture = $storeApplication.Package.Architecture.ToString()
            signature_kind = $storeApplication.Package.SignatureKind.ToString()
            aumid = $storeApplication.Aumid
            executable = $storeApplication.ExecutablePath
            logical_width = $definition.Width
            logical_height = $definition.Height
            dpi = $measurement.Dpi
            scale = $measurement.Scale
            physical_visible_width = $measurement.Bounds.VisibleWidth
            physical_visible_height = $measurement.Bounds.VisibleHeight
            capture_method = "CopyFromScreen (foreground DWM-visible frame)"
            requires_prepared_state = $definition.Prepared
            requirement = $definition.Requirement
        }
        $metadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $metadataPath -Encoding UTF8

        $captureResults += [pscustomobject]@{
            State = $stateId
            Png = $savedPath
            Metadata = $metadataPath
            LogicalSize = "$($definition.Width)x$($definition.Height)"
            PhysicalSize = "$($measurement.Bounds.VisibleWidth)x$($measurement.Bounds.VisibleHeight)"
            Dpi = $measurement.Dpi
        }
    }
}
finally {
    if (-not $KeepApplicationOpen) {
        # Leave the Store application in its normal close-to-tray lifecycle;
        # do not force-stop it or disturb persisted state.
        [void]$windowProcess.CloseMainWindow()
    }
}

$captureResults
