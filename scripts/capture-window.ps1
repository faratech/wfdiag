param(
    [Parameter(Mandatory = $true, ParameterSetName = "ByName")]
    [string]$ProcessName,

    [Parameter(Mandatory = $true, ParameterSetName = "ById")]
    [ValidateRange(1, [int]::MaxValue)]
    [int]$ProcessId,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [int]$WaitSeconds = 20,

    [int]$LogicalWidth = 0,

    [int]$LogicalHeight = 0,

    [switch]$KeepPhysicalCapture
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class WfDiagCaptureNative
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
}
"@

# Keep GetWindowRect, DWM bounds, and SetWindowPos in one physical-pixel
# coordinate space. PowerShell otherwise applies DPI virtualization and exact
# source/native viewport comparisons drift at non-100% display scaling.
$previousDpiContext = [WfDiagCaptureNative]::SetThreadDpiAwarenessContext([IntPtr](-4))
if ($previousDpiContext -eq [IntPtr]::Zero) {
    throw "Unable to enter the per-monitor DPI-aware thread context required for capture."
}

function Get-CaptureWindowBounds {
    param([Parameter(Mandatory = $true)][IntPtr]$Hwnd)

    $window = New-Object WfDiagCaptureNative+Rect
    if (-not [WfDiagCaptureNative]::GetWindowRect($Hwnd, [ref]$window)) {
        throw "Unable to resolve the outer window bounds."
    }

    $visible = New-Object WfDiagCaptureNative+Rect
    $hr = [WfDiagCaptureNative]::DwmGetWindowAttribute(
        $Hwnd,
        9,
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

$processDescription = if ($PSCmdlet.ParameterSetName -eq "ById") {
    "process id $ProcessId"
} else {
    "process '$ProcessName'"
}

$deadline = (Get-Date).AddSeconds($WaitSeconds)
$process = $null
do {
    if ($PSCmdlet.ParameterSetName -eq "ById") {
        $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero }
    }
    else {
        $visibleProcesses = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero })
        if ($visibleProcesses.Count -gt 1) {
            $ids = ($visibleProcesses | ForEach-Object { $_.Id }) -join ", "
            throw "Multiple visible windows were found for process '$ProcessName' (PIDs: $ids). Use -ProcessId to select the exact capture target."
        }
        $process = $visibleProcesses | Select-Object -First 1
    }
    if ($null -eq $process) {
        Start-Sleep -Milliseconds 250
    }
} while ($null -eq $process -and (Get-Date) -lt $deadline)

if ($null -eq $process) {
    throw "No visible window was found for $processDescription within $WaitSeconds seconds."
}

$hwnd = $process.MainWindowHandle
$shell = New-Object -ComObject WScript.Shell
[void]$shell.AppActivate([int]$process.Id)
[void][WfDiagCaptureNative]::ShowWindow($hwnd, 9) # SW_RESTORE
[void][WfDiagCaptureNative]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 500

if (($LogicalWidth -gt 0) -xor ($LogicalHeight -gt 0)) {
    throw "LogicalWidth and LogicalHeight must be supplied together."
}

if ($LogicalWidth -gt 0 -and $LogicalHeight -gt 0) {
    $dpi = [WfDiagCaptureNative]::GetDpiForWindow($hwnd)
    if ($dpi -eq 0) {
        $dpi = 96
    }

    $targetVisibleWidth = [int][Math]::Round($LogicalWidth * $dpi / 96.0)
    $targetVisibleHeight = [int][Math]::Round($LogicalHeight * $dpi / 96.0)
    $sized = $false
    for ($attempt = 0; $attempt -lt 4; $attempt++) {
        $bounds = Get-CaptureWindowBounds -Hwnd $hwnd
        $extraWidth = $bounds.WindowWidth - $bounds.VisibleWidth
        $extraHeight = $bounds.WindowHeight - $bounds.VisibleHeight
        $leftInset = $bounds.Visible.Left - $bounds.Window.Left
        $topInset = $bounds.Visible.Top - $bounds.Window.Top

        if (-not [WfDiagCaptureNative]::SetWindowPos(
            $hwnd,
            [IntPtr]::Zero,
            -$leftInset,
            -$topInset,
            $targetVisibleWidth + $extraWidth,
            $targetVisibleHeight + $extraHeight,
            0x0014)) { # SWP_NOZORDER | SWP_NOACTIVATE
            $win32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "Unable to resize $processDescription for a logical ${LogicalWidth}x${LogicalHeight} capture (Win32 $win32Error)."
        }
        Start-Sleep -Milliseconds 350

        $measured = Get-CaptureWindowBounds -Hwnd $hwnd
        if ($measured.VisibleWidth -eq $targetVisibleWidth -and
            $measured.VisibleHeight -eq $targetVisibleHeight) {
            $sized = $true
            break
        }
    }
    if (-not $sized) {
        throw "Unable to obtain an exact logical ${LogicalWidth}x${LogicalHeight} visible frame."
    }
}

$bounds = Get-CaptureWindowBounds -Hwnd $hwnd
$rect = $bounds.Visible
$width = $bounds.VisibleWidth
$height = $bounds.VisibleHeight
if ($width -le 0 -or $height -le 0) {
    throw "Resolved invalid window bounds ${width}x${height}."
}

$absoluteOutput = [IO.Path]::GetFullPath($OutputPath)
$outputDirectory = [IO.Path]::GetDirectoryName($absoluteOutput)
if (-not [string]::IsNullOrWhiteSpace($outputDirectory)) {
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
}

$physicalBitmap = New-Object Drawing.Bitmap(
    $width,
    $height,
    [Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [Drawing.Graphics]::FromImage($physicalBitmap)
$normalizedBitmap = $null
try {
    # Native WinUI renders correctly through PrintWindow and this avoids
    # foreground-stealing terminals or notifications entering the evidence.
    # Retain a screen-copy fallback for hosts where PrintWindow is unavailable.
    [void]$shell.AppActivate([int]$process.Id)
    [void][WfDiagCaptureNative]::ShowWindow($hwnd, 9)
    [void][WfDiagCaptureNative]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 350
    $targetDc = $graphics.GetHdc()
    try {
        $captured = [WfDiagCaptureNative]::PrintWindow($hwnd, $targetDc, 2)
    }
    finally {
        $graphics.ReleaseHdc($targetDc)
    }
    if (-not $captured) {
        $graphics.CopyFromScreen(
            $rect.Left,
            $rect.Top,
            0,
            0,
            (New-Object Drawing.Size($width, $height)),
            [Drawing.CopyPixelOperation]::SourceCopy)
    }

    if ($KeepPhysicalCapture) {
        $physicalPath = [IO.Path]::Combine(
            $outputDirectory,
            ([IO.Path]::GetFileNameWithoutExtension($absoluteOutput) + ".physical.png"))
        $physicalBitmap.Save($physicalPath, [Drawing.Imaging.ImageFormat]::Png)
    }

    if ($LogicalWidth -gt 0 -and $LogicalHeight -gt 0) {
        $normalizedBitmap = New-Object Drawing.Bitmap(
            $LogicalWidth,
            $LogicalHeight,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $normalizedGraphics = [Drawing.Graphics]::FromImage($normalizedBitmap)
        try {
            $normalizedGraphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $normalizedGraphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $normalizedGraphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $normalizedGraphics.DrawImage($physicalBitmap, 0, 0, $LogicalWidth, $LogicalHeight)
        }
        finally {
            $normalizedGraphics.Dispose()
        }
        $normalizedBitmap.Save($absoluteOutput, [Drawing.Imaging.ImageFormat]::Png)
    }
    else {
        $physicalBitmap.Save($absoluteOutput, [Drawing.Imaging.ImageFormat]::Png)
    }
}
finally {
    $graphics.Dispose()
    if ($null -ne $normalizedBitmap) { $normalizedBitmap.Dispose() }
    $physicalBitmap.Dispose()
    if ($previousDpiContext -ne [IntPtr]::Zero) {
        [void][WfDiagCaptureNative]::SetThreadDpiAwarenessContext($previousDpiContext)
    }
}

Write-Output $absoluteOutput
