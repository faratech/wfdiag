param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [string]$OutputDirectory,

    [ValidateRange(0, 60)]
    [int]$HoldSeconds = 2,

    [ValidateRange(5, 120)]
    [int]$WaitSeconds = 20
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$logicalWidth = 1440
$logicalHeight = 1000
$repoRoot = Split-Path $PSScriptRoot -Parent
$versionPath = Join-Path $repoRoot "version.json"
$captureScript = Join-Path $PSScriptRoot "capture-window.ps1"
$versionProbeFlag = "--wfdiag-version-probe"
$versionProbeEnvironment = "WFDIAG_REACTOR_VERSION_PROBE_FILE"

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$releaseDirectory = Split-Path -Parent $resolvedExecutable
if (-not (Test-Path -LiteralPath $versionPath -PathType Leaf)) {
    throw "Canonical version file does not exist: $versionPath"
}
if (-not (Test-Path -LiteralPath $captureScript -PathType Leaf)) {
    throw "Capture helper does not exist: $captureScript"
}

$canonicalVersion = [string](
    (Get-Content -LiteralPath $versionPath -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($canonicalVersion)) {
    throw "Canonical version.json has no string version."
}

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot (
        "apps\wfdiag\captures-{0}\live-system-validation" -f $canonicalVersion)
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing

if ($null -eq ("WfDiagLiveSystemNative" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class WfDiagLiveSystemNative
{
    [StructLayout(LayoutKind.Sequential)]
    public struct TokenElevation
    {
        public int TokenIsElevated;
    }

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

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool IsWow64Process2(
        IntPtr processHandle,
        out ushort processMachine,
        out ushort nativeMachine);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hwnd);
}
"@
}

function Get-ReactorApplicationVersion {
    param([Parameter(Mandatory = $true)][string]$Path)

    $probePath = Join-Path ([IO.Path]::GetTempPath()) (
        "wfdiag-reactor-live-version-{0}.json" -f [Guid]::NewGuid().ToString("N"))
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

function Get-WindowsVersionLabel {
    $currentVersion = Get-ItemProperty `
        -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
    $productName = [string]$currentVersion.ProductName
    $currentBuild = 0
    if (-not [int]::TryParse(
        [string]$currentVersion.CurrentBuild,
        [ref]$currentBuild)) {
        throw "Windows CurrentBuild is not numeric."
    }

    $baseName = if ($currentBuild -ge 22000) {
        "Windows 11"
    }
    elseif ($productName.Contains("Windows 10")) {
        "Windows 10"
    }
    else {
        $productName
    }

    $parts = @($baseName)
    if ($null -ne $currentVersion.EditionID) {
        $parts += [string]$currentVersion.EditionID
    }
    if ($null -ne $currentVersion.DisplayVersion) {
        $parts += "($([string]$currentVersion.DisplayVersion))"
    }
    return $parts -join " "
}

function Convert-MachineName {
    param([Parameter(Mandatory = $true)][UInt16]$Machine)

    switch ($Machine) {
        0x014C { return "x86" }
        0x01C4 { return "ARM" }
        0x8664 { return "x64" }
        0xAA64 { return "ARM64" }
        default { throw ("Unsupported Windows machine type 0x{0:X4}." -f $Machine) }
    }
}

function Get-ProcessArchitectureEvidence {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    [UInt16]$processMachine = 0
    [UInt16]$nativeMachine = 0
    if (-not [WfDiagLiveSystemNative]::IsWow64Process2(
        $Process.Handle,
        [ref]$processMachine,
        [ref]$nativeMachine)) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "IsWow64Process2 failed for PID $($Process.Id) (Win32 $errorCode)."
    }

    $nativeName = Convert-MachineName -Machine $nativeMachine
    $isEmulated = $processMachine -notin @(0x0000, 0x0001)
    $processName = if ($isEmulated) {
        Convert-MachineName -Machine $processMachine
    }
    else {
        $nativeName
    }
    $status = if ($isEmulated) {
        "$processName app running on $nativeName hardware"
    }
    else {
        "Native $nativeName execution"
    }

    return [pscustomobject]@{
        processMachine = ("0x{0:X4}" -f $processMachine)
        processArchitecture = $processName
        nativeMachine = ("0x{0:X4}" -f $nativeMachine)
        nativeArchitecture = $nativeName
        isEmulated = $isEmulated
        status = $status
    }
}

function Get-ProcessPrivilegeLabel {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    $token = [IntPtr]::Zero
    try {
        if (-not [WfDiagLiveSystemNative]::OpenProcessToken(
            $Process.Handle,
            0x0008,
            [ref]$token)) {
            $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "OpenProcessToken failed for PID $($Process.Id) (Win32 $errorCode)."
        }

        $elevation = New-Object WfDiagLiveSystemNative+TokenElevation
        [UInt32]$returnedLength = 0
        $size = [UInt32][Runtime.InteropServices.Marshal]::SizeOf($elevation)
        if (-not [WfDiagLiveSystemNative]::GetTokenInformation(
            $token,
            20,
            [ref]$elevation,
            $size,
            [ref]$returnedLength)) {
            $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "GetTokenInformation failed for PID $($Process.Id) (Win32 $errorCode)."
        }
        if ($elevation.TokenIsElevated -ne 0) {
            return "Administrator"
        }
        return "Standard user"
    }
    finally {
        if ($token -ne [IntPtr]::Zero) {
            [void][WfDiagLiveSystemNative]::CloseHandle($token)
        }
    }
}

$executableVersion = Get-ReactorApplicationVersion -Path $resolvedExecutable
if ($executableVersion -cne $canonicalVersion) {
    throw "Reactor executable reports '$executableVersion', but version.json requires '$canonicalVersion'."
}

$stagedWebView = Get-ChildItem -LiteralPath $releaseDirectory -File |
    Where-Object { $_.Name -ieq "Microsoft.Web.WebView2.Core.dll" } |
    Select-Object -First 1
if ($null -ne $stagedWebView) {
    throw "Unused WebView2 projection is staged beside the native candidate."
}

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
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$startedAt = Get-Date
$process = $null
$architecture = $null
$machineAutomationName = $null
$footerAutomationName = $null
$expectedComputerName = [Environment]::MachineName
$expectedOs = Get-WindowsVersionLabel
$expectedPrivilege = $null
$dpi = 0
$expectedPhysicalWidth = 0
$expectedPhysicalHeight = 0
$capturePath = $null
$physicalCapturePath = $null
$uiaEvidencePath = $null
$failures = New-Object Collections.Generic.List[string]
$gracefulClose = $false

try {
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
        $process = Start-Process -FilePath $resolvedExecutable -PassThru
    }
    finally {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable(
                $name,
                $savedEnvironment[$name],
                "Process")
        }
    }

    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    do {
        $process.Refresh()
        if ($process.HasExited) {
            throw "PID $($process.Id) exited during startup with code $($process.ExitCode)."
        }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            break
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $deadline)
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "PID $($process.Id) did not create a Diagnostics window within $WaitSeconds seconds."
    }

    $expectedPrivilege = Get-ProcessPrivilegeLabel -Process $process
    $architecture = Get-ProcessArchitectureEvidence -Process $process
    $expectedMachineAutomationName = (
        "Computer {0}, {1}, {2}, {3}" -f
            $expectedComputerName,
            $expectedOs,
            $expectedPrivilege,
            $architecture.status)

    $root = [Windows.Automation.AutomationElement]::FromHandle(
        $process.MainWindowHandle)
    $trueCondition = [Windows.Automation.Condition]::TrueCondition
    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    $machineElement = $null
    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            $trueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            try {
                $name = [string]$elements.Item($index).Current.Name
            }
            catch {
                continue
            }
            if ($name -ceq $expectedMachineAutomationName) {
                $machineElement = $elements.Item($index)
                $machineAutomationName = $name
                break
            }
        }
        if ($null -eq $machineElement) {
            Start-Sleep -Milliseconds 150
        }
    } while ($null -eq $machineElement -and (Get-Date) -lt $deadline)
    if ($null -eq $machineElement) {
        throw "UI Automation did not expose '$expectedMachineAutomationName'."
    }

    $names = New-Object Collections.Generic.List[string]
    $elements = $root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        $trueCondition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        try {
            $name = [string]$elements.Item($index).Current.Name
        }
        catch {
            continue
        }
        if (-not [string]::IsNullOrWhiteSpace($name)) {
            $names.Add($name)
        }
    }
    foreach ($requiredText in @($expectedComputerName, $expectedOs, $expectedPrivilege)) {
        if (-not $names.Contains($requiredText)) {
            throw "Required live UI Automation text is absent: '$requiredText'."
        }
    }

    # Windows PowerShell 5.1 treats a BOM-less .ps1 as the active ANSI code
    # page. Construct the middle dot rather than embedding a non-ASCII literal.
    $middleDot = [string][char]0x00B7
    $footerPattern = (
        "^{0}\s+wfdiag\s+{1}\s+{2}\s+WindowsForum\.com$" -f
            [Regex]::Escape($expectedPrivilege),
            [Regex]::Escape($canonicalVersion),
            [Regex]::Escape($middleDot))
    $footerAutomationName = $names |
        Where-Object { $_ -match $footerPattern } |
        Select-Object -First 1
    if ($null -eq $footerAutomationName) {
        throw "The status footer does not expose '$expectedPrivilege' and wfdiag $canonicalVersion."
    }

    $process.Refresh()
    $loadedXaml = $process.Modules |
        Where-Object { $_.ModuleName -ieq "Microsoft.UI.Xaml.dll" } |
        Select-Object -First 1
    if ($null -eq $loadedXaml) {
        throw "Microsoft.UI.Xaml.dll is not loaded by PID $($process.Id)."
    }
    $loadedXamlDirectory = [IO.Path]::GetFullPath(
        (Split-Path -Parent $loadedXaml.FileName))
    if (-not [string]::Equals(
        $loadedXamlDirectory,
        [IO.Path]::GetFullPath($releaseDirectory),
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "Microsoft.UI.Xaml.dll loaded outside the candidate directory: $($loadedXaml.FileName)"
    }
    $loadedWebView = $process.Modules |
        Where-Object { $_.ModuleName -match "WebView2|msedge" } |
        Select-Object -First 1
    if ($null -ne $loadedWebView) {
        throw "A WebView2/Edge module is loaded: $($loadedWebView.ModuleName)"
    }

    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $capturePath = Join-Path $OutputDirectory (
        "diagnostics-live-system-{0}.png" -f $stamp)
    $physicalCapturePath = Join-Path $OutputDirectory (
        "diagnostics-live-system-{0}.physical.png" -f $stamp)
    $uiaEvidencePath = Join-Path $OutputDirectory (
        "diagnostics-live-system-{0}.uia.json" -f $stamp)

    $captureCommandOutput = & powershell.exe `
        -NoProfile `
        -ExecutionPolicy Bypass `
        -File $captureScript `
        -ProcessId $process.Id `
        -OutputPath $capturePath `
        -WaitSeconds $WaitSeconds `
        -LogicalWidth $logicalWidth `
        -LogicalHeight $logicalHeight `
        -KeepPhysicalCapture
    if ($LASTEXITCODE -ne 0) {
        throw "Native window capture failed with exit code $LASTEXITCODE."
    }
    $captureOutputLines = @($captureCommandOutput)
    if ($captureOutputLines.Count -ne 1 -or
        -not [string]::Equals(
            [IO.Path]::GetFullPath([string]$captureOutputLines[0]),
            $capturePath,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "Native window capture did not report the expected output path."
    }

    if (-not (Test-Path -LiteralPath $physicalCapturePath -PathType Leaf)) {
        throw "Physical capture was not created: $physicalCapturePath"
    }
    # capture-window may move the window to the primary monitor. Resolve DPI
    # afterward so physical-size validation uses the monitor actually captured.
    $dpi = [int][WfDiagLiveSystemNative]::GetDpiForWindow($process.MainWindowHandle)
    if ($dpi -eq 0) {
        $dpi = 96
    }
    $expectedPhysicalWidth = [int][Math]::Round($logicalWidth * $dpi / 96.0)
    $expectedPhysicalHeight = [int][Math]::Round($logicalHeight * $dpi / 96.0)

    $logicalImage = $null
    $physicalImage = $null
    try {
        $logicalImage = [Drawing.Image]::FromFile($capturePath)
        $physicalImage = [Drawing.Image]::FromFile($physicalCapturePath)
        if ($logicalImage.Width -ne $logicalWidth -or
            $logicalImage.Height -ne $logicalHeight) {
            throw "Logical capture is $($logicalImage.Width)x$($logicalImage.Height), expected ${logicalWidth}x${logicalHeight}."
        }
        if ($physicalImage.Width -ne $expectedPhysicalWidth -or
            $physicalImage.Height -ne $expectedPhysicalHeight) {
            throw "Physical capture is $($physicalImage.Width)x$($physicalImage.Height), expected ${expectedPhysicalWidth}x${expectedPhysicalHeight} at $dpi DPI."
        }
    }
    finally {
        if ($null -ne $logicalImage) {
            $logicalImage.Dispose()
        }
        if ($null -ne $physicalImage) {
            $physicalImage.Dispose()
        }
    }

    $uiaEvidence = [pscustomobject]@{
        executable = $resolvedExecutable
        executableSha256 = (Get-FileHash $resolvedExecutable -Algorithm SHA256).Hash
        applicationVersion = $canonicalVersion
        pid = $process.Id
        computerName = $expectedComputerName
        osVersion = $expectedOs
        privilege = $expectedPrivilege
        architecture = $architecture
        machineCardAutomationName = $machineAutomationName
        footerAutomationName = $footerAutomationName
        logicalViewport = [pscustomobject]@{
            width = $logicalWidth
            height = $logicalHeight
        }
        dpi = $dpi
        physicalViewport = [pscustomobject]@{
            width = $expectedPhysicalWidth
            height = $expectedPhysicalHeight
        }
        fixtureEnvironmentCleared = $true
        localXamlPath = $loadedXaml.FileName
        webViewModuleLoaded = $false
    }
    $uiaEvidence | ConvertTo-Json -Depth 6 |
        Set-Content -LiteralPath $uiaEvidencePath -Encoding UTF8

    if ($HoldSeconds -gt 0) {
        Start-Sleep -Seconds $HoldSeconds
    }
    $process.Refresh()
    if ($process.HasExited) {
        throw "PID $($process.Id) exited before the live validation hold completed."
    }
}
catch {
    $failures.Add($_.Exception.Message)
}
finally {
    if ($null -ne $process) {
        try {
            $process.Refresh()
            if ($process.HasExited) {
                if ($failures.Count -eq 0) {
                    $failures.Add("PID $($process.Id) exited before graceful shutdown.")
                }
            }
            else {
                $gracefulClose = $process.CloseMainWindow()
                if (-not $gracefulClose -or -not $process.WaitForExit(5000)) {
                    $failures.Add("PID $($process.Id) did not close gracefully within 5 seconds.")
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                    [void]$process.WaitForExit(5000)
                }
            }
        }
        catch {
            $failures.Add("Unable to close PID $($process.Id): $($_.Exception.Message)")
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

# Event Log writes can trail the process. Match the complete candidate path so
# another Reactor build cannot be attributed to this validation run.
Start-Sleep -Seconds 2
$crashEvents = @()
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
        $failures.Add(
            "Unable to inspect the Windows Application event log: $($_.Exception.Message)")
    }
}
if ($crashEvents.Count -gt 0) {
    $failures.Add(
        "Windows recorded $($crashEvents.Count) crash event(s) for this candidate path.")
}

$result = [pscustomobject]@{
    executable = $resolvedExecutable
    executableVersion = $executableVersion
    canonicalVersion = $canonicalVersion
    pid = if ($null -ne $process) { $process.Id } else { $null }
    architecture = $architecture
    machineCardAutomationName = $machineAutomationName
    footerAutomationName = $footerAutomationName
    logicalCapture = $capturePath
    physicalCapture = $physicalCapturePath
    uiaEvidence = $uiaEvidencePath
    gracefulClose = $gracefulClose
    crashEventCount = $crashEvents.Count
    failures = $failures
}
$result | ConvertTo-Json -Depth 6

if ($failures.Count -gt 0) {
    exit 1
}
