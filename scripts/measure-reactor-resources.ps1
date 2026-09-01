<#
.SYNOPSIS
Measure live idle-page and Quick Scan resources for Reactor and Tauri builds.

.DESCRIPTION
Uses fresh, non-fixture native launches and writes raw CSV/JSON samples,
per-run evidence, aggregate metrics, and second-minus-first deltas when two
executables are supplied. Every sample retains the launched process metrics
and also totals its verified descendant process tree, which is required for a
fair comparison with Tauri/WebView2. Defaults run one warm-up, three
repetitions, all six pages, 15-second settling, 30-second 1 Hz idle windows,
and Quick Scan at 250 ms through completion plus a 15-second settle and
30-second retained window.

.EXAMPLE
.\scripts\measure-reactor-resources.ps1 `
  -Executable C:\code\wfdiag-new\wfdiag.exe `
  -OutputDirectory C:\code\wfdiag-new-resource-evidence

.EXAMPLE
.\scripts\measure-reactor-resources.ps1 `
  -Executable @(
    'C:\code\wfdiag-before\wfdiag.exe',
    'C:\code\wfdiag-after\wfdiag.exe'
  ) `
  -Label @('before', 'after') `
  -OutputDirectory C:\code\wfdiag-resource-comparison

.EXAMPLE
.\scripts\measure-reactor-resources.ps1 `
  -Executable @(
    'C:\Program Files\WindowsApps\Publisher.App_2.5.8.0_arm64__publisher\app.exe',
    'C:\code\wfdiag-reactor\wfdiag.exe'
  ) `
  -LaunchMode @('tauri', 'reactor') `
  -Label @('tauri', 'reactor') `
  -OutputDirectory C:\code\wfdiag-resource-comparison
#>

# Repeatable live resource benchmark for native Reactor and Tauri candidates.
#
# The harness deliberately launches each page in a fresh, non-fixture process.
# It records the exact process started by this script and recursively samples
# only descendants whose live parentage and creation times prove that they
# belong to that launch. It never terminates a pre-existing process, and forced
# cleanup is permitted only after the root PID, image path, and start time are
# revalidated against that owned process.
#
# Evidence written under -OutputDirectory:
#   raw-samples.csv  - one row per process sample
#   raw-samples.json - the same samples without CSV type loss
#   runs.json        - per-run outcomes and summaries
#   summary.json     - concise cross-run aggregates and optional deltas

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias("Executables")]
    [ValidateCount(1, 2)]
    [string[]]$Executable,

    [string[]]$Label = @(),

    [ValidateSet("reactor", "tauri")]
    [string[]]$LaunchMode = @(),

    [ValidateSet("diagnostics", "monitor", "processes", "ai", "issues", "history")]
    [string[]]$PageTag = @(
        "diagnostics", "monitor", "processes", "ai", "issues", "history"
    ),

    [ValidateRange(0, 10)]
    [int]$WarmupRuns = 1,

    [ValidateRange(0, 120)]
    [int]$WarmupSeconds = 10,

    [ValidateRange(1, 20)]
    [int]$Repetitions = 3,

    [ValidateRange(0, 600)]
    [int]$SettleSeconds = 15,

    [ValidateRange(1, 1800)]
    [int]$IdleSampleSeconds = 30,

    [ValidateRange(100, 10000)]
    [int]$IdleSampleMilliseconds = 1000,

    [ValidateRange(0, 600)]
    [int]$QuickScanBaselineSeconds = 15,

    [ValidateRange(30, 1800)]
    [int]$QuickScanTimeoutSeconds = 180,

    [ValidateRange(100, 5000)]
    [int]$QuickScanSampleMilliseconds = 250,

    [ValidateRange(0, 600)]
    [int]$RetainedSettleSeconds = 15,

    [ValidateRange(1, 1800)]
    [int]$RetainedSampleSeconds = 30,

    [ValidateRange(1, 60)]
    [int]$StartupWaitSeconds = 10,

    [ValidateRange(1, 60)]
    [int]$GracefulCloseSeconds = 8,

    [ValidateRange(640, 7680)]
    [int]$WindowWidth = 1440,

    [ValidateRange(480, 4320)]
    [int]$WindowHeight = 1000,

    [switch]$SkipQuickScan,

    [string]$OutputDirectory
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

# Toolhelp is substantially cheaper than a Win32_Process CIM query at the
# Quick Scan sampling rate. The snapshot supplies only live PID/parent-PID
# edges; PowerShell still opens every selected process and validates its start
# time before any metrics are accepted.
if ($null -eq ("WfDiagProcessTreeNative" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;

public sealed class WfDiagProcessParent
{
    public int ProcessId { get; private set; }
    public int ParentProcessId { get; private set; }

    public WfDiagProcessParent(int processId, int parentProcessId)
    {
        ProcessId = processId;
        ParentProcessId = parentProcessId;
    }
}

public static class WfDiagProcessTreeNative
{
    private const uint TH32CS_SNAPPROCESS = 0x00000002;
    private static readonly IntPtr InvalidHandleValue = new IntPtr(-1);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct PROCESSENTRY32
    {
        public uint dwSize;
        public uint cntUsage;
        public uint th32ProcessID;
        public IntPtr th32DefaultHeapID;
        public uint th32ModuleID;
        public uint cntThreads;
        public uint th32ParentProcessID;
        public int pcPriClassBase;
        public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)]
        public string szExeFile;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint processId);

    [DllImport("kernel32.dll", EntryPoint = "Process32FirstW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool Process32First(IntPtr snapshot, ref PROCESSENTRY32 entry);

    [DllImport("kernel32.dll", EntryPoint = "Process32NextW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool Process32Next(IntPtr snapshot, ref PROCESSENTRY32 entry);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);

    public static WfDiagProcessParent[] SnapshotParents()
    {
        IntPtr snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if (snapshot == InvalidHandleValue)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS) failed.");
        }

        try
        {
            var result = new List<WfDiagProcessParent>();
            var entry = new PROCESSENTRY32();
            entry.dwSize = (uint)Marshal.SizeOf(typeof(PROCESSENTRY32));
            bool available = Process32First(snapshot, ref entry);
            while (available)
            {
                result.Add(new WfDiagProcessParent(
                    unchecked((int)entry.th32ProcessID),
                    unchecked((int)entry.th32ParentProcessID)));
                entry.dwSize = (uint)Marshal.SizeOf(typeof(PROCESSENTRY32));
                available = Process32Next(snapshot, ref entry);
            }
            return result.ToArray();
        }
        finally
        {
            CloseHandle(snapshot);
        }
    }
}
"@
}

$schemaVersion = 2
$logicalProcessorCount = [Environment]::ProcessorCount
if ($logicalProcessorCount -lt 1) {
    throw "Windows reported no logical processors; normalized CPU cannot be calculated."
}

if ($Label.Count -gt 0 -and $Label.Count -ne $Executable.Count) {
    throw "-Label must be omitted or contain exactly one label per executable."
}
if ($LaunchMode.Count -ne 0 -and
    $LaunchMode.Count -ne 1 -and
    $LaunchMode.Count -ne $Executable.Count) {
    throw (
        "-LaunchMode must be omitted, contain one mode for all executables, or " +
        "contain exactly one mode per executable.")
}
if (@($PageTag | Select-Object -Unique).Count -ne $PageTag.Count) {
    throw "-PageTag contains a duplicate page. Supply each page at most once."
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path (Get-Location).ProviderPath (
        "reactor-resource-measurements\$stamp")
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {
    $existing = @(Get-ChildItem -LiteralPath $OutputDirectory -Force -ErrorAction Stop)
    if ($existing.Count -gt 0) {
        throw "Output directory is not empty; refusing to overwrite evidence: $OutputDirectory"
    }
}
else {
    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
}

$rawSamples = [System.Collections.Generic.List[object]]::new()
$runs = [System.Collections.Generic.List[object]]::new()
$failures = [System.Collections.Generic.List[object]]::new()
$startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")

function ConvertTo-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar)
}

function Test-SamePath {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )

    return [string]::Equals(
        (ConvertTo-FullPath -Path $Left),
        (ConvertTo-FullPath -Path $Right),
        [StringComparison]::OrdinalIgnoreCase)
}

function Get-ProcessImagePath {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    try {
        $Process.Refresh()
        $path = [string]$Process.Path
        if ([string]::IsNullOrWhiteSpace($path)) {
            return $null
        }
        return ConvertTo-FullPath -Path $path
    }
    catch {
        return $null
    }
}

function Get-ProcessDescription {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    $path = Get-ProcessImagePath -Process $Process
    if ([string]::IsNullOrWhiteSpace($path)) {
        $path = "<path unavailable>"
    }
    return "PID $($Process.Id) '$path'"
}

function Assert-NoConflictingProcess {
    param([Parameter(Mandatory = $true)][string]$ExecutablePath)

    $baseName = [IO.Path]::GetFileNameWithoutExtension($ExecutablePath)
    $existing = @(Get-Process -Name $baseName -ErrorAction SilentlyContinue)
    if ($existing.Count -eq 0) {
        return
    }

    $details = @($existing | ForEach-Object { Get-ProcessDescription -Process $_ }) -join "; "
    throw (
        "A '$baseName' process already exists. This harness refuses to close or reuse " +
        "a process it did not launch. Close it manually and retry. Observed: $details")
}

function Get-VerifiedOwnedProcess {
    param([Parameter(Mandatory = $true)]$Session)

    $process = Get-Process -Id $Session.processId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        throw "Owned candidate PID $($Session.processId) is no longer running."
    }

    $actualPath = Get-ProcessImagePath -Process $process
    if ([string]::IsNullOrWhiteSpace($actualPath)) {
        throw (
            "Windows would not disclose the image path for PID $($Session.processId). " +
            "Refusing to treat it as an owned process.")
    }
    if (-not (Test-SamePath -Left $actualPath -Right $Session.executable)) {
        throw (
            "PID $($Session.processId) no longer belongs to the launched executable. " +
            "Expected '$($Session.executable)', observed '$actualPath'.")
    }

    try {
        $actualStart = $process.StartTime.ToUniversalTime()
    }
    catch {
        throw "Windows would not disclose the start time for owned PID $($Session.processId)."
    }
    $difference = [Math]::Abs(($actualStart - $Session.processStartUtc).TotalSeconds)
    if ($difference -gt 1.0) {
        throw (
            "PID $($Session.processId) was reused by another process. Refusing cleanup; " +
            "start-time difference was $([Math]::Round($difference, 3)) seconds.")
    }
    return $process
}

function Set-OwnedCandidatePage {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Page
    )

    # Reactor consumes WFDIAG_REACTOR_PAGE before it creates the window. The
    # Tauri/WebView baseline does not, so drive its real navigation control
    # after launch and verify that the destination heading actually rendered.
    if ($Candidate.launchMode -ne "tauri") {
        return
    }

    $navigationName = @{
        diagnostics = "Diagnostics"
        monitor = "Live Monitor"
        processes = "Processes"
        ai = "AI Analysis"
        issues = "Issues"
        history = "History"
    }[$Page]
    $headingName = @{
        diagnostics = "System Analysis"
        monitor = "Live Monitor"
        processes = "Processes"
        ai = "AI Analysis"
        issues = "Issues"
        history = "History"
    }[$Page]
    if ([string]::IsNullOrWhiteSpace($navigationName) -or
        [string]::IsNullOrWhiteSpace($headingName)) {
        throw "No Tauri navigation mapping exists for page '$Page'."
    }

    $root = Get-ReactorUiaRoot -Process $Session.process
    $deadline = (Get-Date).AddSeconds($StartupWaitSeconds)
    $button = Wait-UniqueUiaButton -Root $root -Deadline $deadline -Name $navigationName
    Invoke-UiaButtonElement -Element $button.element

    do {
        $elements = $root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            try {
                $current = $elements.Item($index).Current
                if (-not $current.IsOffscreen -and
                    $current.Name -eq $headingName -and
                    $current.ControlType -ne [Windows.Automation.ControlType]::Button) {
                    return
                }
            }
            catch {
                continue
            }
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)

    throw (
        "Tauri navigation invoked '$navigationName', but the '$headingName' " +
        "page heading did not render within $StartupWaitSeconds seconds.")
}

function Get-OwnedProcessTreeSnapshot {
    param([Parameter(Mandatory = $true)]$Session)

    # Revalidate the destructive-operation identity boundary even though this
    # function only reads metrics. That prevents a recycled root PID from ever
    # becoming an ownership anchor for unrelated descendants.
    $root = Get-VerifiedOwnedProcess -Session $Session
    $rootStartUtc = $root.StartTime.ToUniversalTime()
    $parentSnapshot = @([WfDiagProcessTreeNative]::SnapshotParents())
    if (@($parentSnapshot | Where-Object {
        $_.ProcessId -eq $Session.processId
    }).Count -ne 1) {
        throw (
            "The verified root PID $($Session.processId) was absent from the " +
            "Toolhelp process snapshot; refusing a partial process-tree sample.")
    }

    $childrenByParent = @{}
    foreach ($entry in $parentSnapshot) {
        $parentId = [int]$entry.ParentProcessId
        if (-not $childrenByParent.ContainsKey($parentId)) {
            $childrenByParent[$parentId] = [System.Collections.Generic.List[int]]::new()
        }
        $childrenByParent[$parentId].Add([int]$entry.ProcessId)
    }

    $owned = [System.Collections.Generic.List[object]]::new()
    $pending = [System.Collections.Generic.Queue[object]]::new()
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $rootRecord = [pscustomobject]@{
        process = $root
        processId = [int]$root.Id
        parentProcessId = $null
        startTimeUtc = $rootStartUtc
    }
    $owned.Add($rootRecord)
    $pending.Enqueue($rootRecord)
    [void]$seen.Add([int]$root.Id)

    while ($pending.Count -gt 0) {
        $parent = $pending.Dequeue()
        $parentId = [int]$parent.processId
        if (-not $childrenByParent.ContainsKey($parentId)) {
            continue
        }

        foreach ($childId in $childrenByParent[$parentId]) {
            if ($seen.Contains($childId)) {
                continue
            }

            $child = Get-Process -Id $childId -ErrorAction SilentlyContinue
            if ($null -eq $child) {
                # A short-lived child can disappear after the atomic Toolhelp
                # snapshot. It no longer contributes live memory, so skipping
                # it is the only coherent instantaneous result.
                continue
            }
            try {
                $child.Refresh()
                if ($child.HasExited) { continue }
                $childStartUtc = $child.StartTime.ToUniversalTime()
            }
            catch {
                if ($null -ne (Get-Process -Id $childId -ErrorAction SilentlyContinue)) {
                    throw (
                        "Could not validate start time for live descendant PID " +
                        "$childId of owned PID ${parentId}: $($_.Exception.Message)")
                }
                continue
            }

            # ParentProcessId can point at a newly reused PID. A process that
            # predates its alleged parent cannot belong to this launch, so it
            # must never be adopted into the sample.
            if ($childStartUtc -lt $parent.startTimeUtc) {
                continue
            }

            $record = [pscustomobject]@{
                process = $child
                processId = [int]$child.Id
                parentProcessId = $parentId
                startTimeUtc = $childStartUtc
            }
            [void]$seen.Add([int]$child.Id)
            $owned.Add($record)
            $pending.Enqueue($record)
        }
    }

    $metrics = [System.Collections.Generic.List[object]]::new()
    [int64]$privateBytes = 0
    [int64]$workingSetBytes = 0
    [double]$cpuMilliseconds = 0.0
    [int64]$threadCount = 0
    [int64]$handleCount = 0
    foreach ($record in @($owned | Sort-Object processId)) {
        $process = $record.process
        try {
            $process.Refresh()
            if ($process.HasExited) {
                if ($record.processId -eq $Session.processId) {
                    throw "Owned root PID $($Session.processId) exited during process-tree sampling."
                }
                continue
            }
            $processPrivateBytes = [int64]$process.PrivateMemorySize64
            $processWorkingSetBytes = [int64]$process.WorkingSet64
            $processCpuMilliseconds = [double]$process.TotalProcessorTime.TotalMilliseconds
            $processThreadCount = [int]$process.Threads.Count
            $processHandleCount = [int]$process.HandleCount
        }
        catch {
            if ($record.processId -eq $Session.processId -or
                $null -ne (Get-Process -Id $record.processId -ErrorAction SilentlyContinue)) {
                throw (
                    "Could not read complete resource metrics for owned PID " +
                    "$($record.processId): $($_.Exception.Message)")
            }
            continue
        }

        $privateBytes += $processPrivateBytes
        $workingSetBytes += $processWorkingSetBytes
        $cpuMilliseconds += $processCpuMilliseconds
        $threadCount += $processThreadCount
        $handleCount += $processHandleCount
        $metrics.Add([pscustomobject][ordered]@{
            identity = "$($record.processId)/$($record.startTimeUtc.Ticks)"
            processId = $record.processId
            parentProcessId = $record.parentProcessId
            startTimeUtc = $record.startTimeUtc
            privateBytes = $processPrivateBytes
            workingSetBytes = $processWorkingSetBytes
            cpuTotalMilliseconds = $processCpuMilliseconds
            threadCount = $processThreadCount
            handleCount = $processHandleCount
        })
    }

    $metricArray = @($metrics.ToArray())
    $rootMetrics = $metricArray | Where-Object {
        $_.processId -eq $Session.processId
    } | Select-Object -First 1
    if ($null -eq $rootMetrics) {
        throw "The owned root PID was not available in its completed process-tree sample."
    }
    return [pscustomobject][ordered]@{
        root = $rootMetrics
        processes = $metricArray
        processCount = $metricArray.Count
        descendantCount = [Math]::Max(0, $metricArray.Count - 1)
        privateBytes = $privateBytes
        workingSetBytes = $workingSetBytes
        cpuTotalMilliseconds = $cpuMilliseconds
        threadCount = $threadCount
        handleCount = $handleCount
    }
}

function Start-OwnedCandidate {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$InitialPage
    )

    Assert-NoConflictingProcess -ExecutablePath $Candidate.executable

    $saved = @{}
    foreach ($name in $environmentNames) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
        [Environment]::SetEnvironmentVariable($name, $null, "Process")
    }
    $process = $null
    $startedAt = Get-Date
    try {
        if ($Candidate.launchMode -eq "reactor") {
            [Environment]::SetEnvironmentVariable(
                "WFDIAG_REACTOR_PAGE", $InitialPage, "Process")
            [Environment]::SetEnvironmentVariable(
                "WFDIAG_REACTOR_WIDTH", [string]$WindowWidth, "Process")
            [Environment]::SetEnvironmentVariable(
                "WFDIAG_REACTOR_HEIGHT", [string]$WindowHeight, "Process")
            [Environment]::SetEnvironmentVariable("WFDIAG_NO_TRAY", "1", "Process")
        }
        $process = Start-Process -FilePath $Candidate.executable -PassThru
    }
    finally {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process")
        }
    }

    # Process.Path can be temporarily unavailable during native image startup.
    # Poll only the Start-Process object we just received; do not search by
    # process name or adopt an arbitrary existing PID.
    $actualPath = $null
    $identityDeadline = (Get-Date).AddSeconds([Math]::Min($StartupWaitSeconds, 5))
    do {
        $process.Refresh()
        if ($process.HasExited) { break }
        $actualPath = Get-ProcessImagePath -Process $process
        if (-not [string]::IsNullOrWhiteSpace($actualPath)) { break }
        Start-Sleep -Milliseconds 50
    } while ((Get-Date) -lt $identityDeadline)
    if ([string]::IsNullOrWhiteSpace($actualPath) -or
        -not (Test-SamePath -Left $actualPath -Right $Candidate.executable)) {
        # Do not terminate a PID unless exact ownership can be established.
        throw (
            "The launched PID $($process.Id) could not be tied to '$($Candidate.executable)'. " +
            "No cleanup signal was sent; inspect that PID manually.")
    }

    $session = [pscustomobject]@{
        process = $process
        processId = $process.Id
        processStartUtc = $process.StartTime.ToUniversalTime()
        executable = $Candidate.executable
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        startedAtUtc = $startedAt.ToUniversalTime()
    }
    $deadline = (Get-Date).AddSeconds($StartupWaitSeconds)
    try {
        do {
            $process.Refresh()
            if ($process.HasExited) {
                throw (
                    "Candidate PID $($process.Id) exited during startup with code " +
                    "$($process.ExitCode). Another Reactor instance may hold the single-instance mutex.")
            }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
                Set-OwnedCandidatePage -Session $session -Candidate $Candidate `
                    -Page $InitialPage
                return $session
            }
            Start-Sleep -Milliseconds 150
        } while ((Get-Date) -lt $deadline)
        throw (
            "Candidate PID $($process.Id) did not create a main window within " +
            "$StartupWaitSeconds seconds.")
    }
    catch {
        $startupError = $_.Exception.Message
        $cleanup = Stop-OwnedCandidate -Session $session
        if ($cleanup.refusedUnknownProcess) {
            throw "$startupError Cleanup was refused: $($cleanup.error)"
        }
        throw $startupError
    }
}

function Stop-OwnedCandidate {
    param([Parameter(Mandatory = $true)]$Session)

    $process = Get-Process -Id $Session.processId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return [pscustomobject]@{
            graceful = $false
            forced = $false
            refusedUnknownProcess = $false
            exitCode = $null
            error = "Owned candidate exited before a graceful close was requested."
        }
    }

    try {
        $process = Get-VerifiedOwnedProcess -Session $Session
    }
    catch {
        return [pscustomobject]@{
            graceful = $false
            forced = $false
            refusedUnknownProcess = $true
            exitCode = $null
            error = "Refused to close an unverified process: $($_.Exception.Message)"
        }
    }

    $closeRequested = $false
    try {
        $closeRequested = $process.CloseMainWindow()
    }
    catch {
        $closeRequested = $false
    }
    if ($closeRequested -and $process.WaitForExit($GracefulCloseSeconds * 1000)) {
        $exitCode = $null
        try {
            # The Process object returned by Get-Process can lose ExitCode
            # access after the handle signals. The original Start-Process
            # object retains that ownership relationship.
            [void]$Session.process.WaitForExit(0)
            $exitCode = $Session.process.ExitCode
        }
        catch { $exitCode = $null }
        $normalExit = ($null -eq $exitCode -or $exitCode -eq 0)
        return [pscustomobject]@{
            graceful = $normalExit
            forced = $false
            refusedUnknownProcess = $false
            exitCode = $exitCode
            error = $(if ($normalExit) {
                $null
            } else {
                "Candidate closed with exit code $exitCode."
            })
        }
    }

    # Cleanup after a failed graceful close is restricted to the exact image
    # and start time recorded at launch. A mismatch is left untouched.
    try {
        $process = Get-VerifiedOwnedProcess -Session $Session
    }
    catch {
        return [pscustomobject]@{
            graceful = $false
            forced = $false
            refusedUnknownProcess = $true
            exitCode = $null
            error = "Graceful close timed out, then cleanup was refused: $($_.Exception.Message)"
        }
    }

    Stop-Process -Id $process.Id -Force -ErrorAction Stop
    [void]$process.WaitForExit(5000)
    return [pscustomobject]@{
        graceful = $false
        forced = $true
        refusedUnknownProcess = $false
        exitCode = $null
        error = (
            "Candidate did not close within $GracefulCloseSeconds seconds and required " +
            "forced cleanup of the verified owned PID.")
    }
}

function New-SampleState {
    return @{
        lastCpuMilliseconds = $null
        lastProcessTreeCpuMilliseconds = $null
        lastElapsedMilliseconds = $null
        processTreeCpuByIdentity = @{}
        processTreeCpuTotalMilliseconds = 0.0
        processTreeCpuInitialized = $false
    }
}

function Update-ProcessTreeCpuTotal {
    param(
        [Parameter(Mandatory = $true)]$Tree,
        [Parameter(Mandatory = $true)][hashtable]$State
    )

    [double]$increment = 0.0
    [double]$initial = 0.0
    foreach ($process in $Tree.processes) {
        $identity = [string]$process.identity
        $current = [double]$process.cpuTotalMilliseconds
        $initial += $current
        if ($State.processTreeCpuByIdentity.ContainsKey($identity)) {
            $delta = $current - [double]$State.processTreeCpuByIdentity[$identity]
            if ($delta -gt 0.0) { $increment += $delta }
        }
        elseif ($State.processTreeCpuInitialized) {
            # Count all CPU a newly observed child consumed since creation.
            $increment += $current
        }
        $State.processTreeCpuByIdentity[$identity] = $current
    }

    if (-not $State.processTreeCpuInitialized) {
        $State.processTreeCpuTotalMilliseconds = $initial
        $State.processTreeCpuInitialized = $true
    }
    else {
        $State.processTreeCpuTotalMilliseconds =
            [double]$State.processTreeCpuTotalMilliseconds + $increment
    }
    return [double]$State.processTreeCpuTotalMilliseconds
}

function Add-ResourceSample {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Scenario,
        [Parameter(Mandatory = $true)][string]$Page,
        [Parameter(Mandatory = $true)][int]$Repetition,
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][int]$SampleIndex,
        [Parameter(Mandatory = $true)][Diagnostics.Stopwatch]$Stopwatch,
        [Parameter(Mandatory = $true)][hashtable]$State,
        [Parameter(Mandatory = $true)]$Destination
    )

    $tree = Get-OwnedProcessTreeSnapshot -Session $Session
    $main = $tree.root
    $elapsedMilliseconds = $Stopwatch.Elapsed.TotalMilliseconds
    $cpuMilliseconds = [double]$main.cpuTotalMilliseconds
    $processTreeCpuMilliseconds = Update-ProcessTreeCpuTotal -Tree $tree -State $State
    $normalizedCpu = $null
    $processTreeNormalizedCpu = $null
    if ($null -ne $State.lastCpuMilliseconds -and
        $null -ne $State.lastElapsedMilliseconds) {
        $cpuDelta = $cpuMilliseconds - [double]$State.lastCpuMilliseconds
        $processTreeCpuDelta =
            $processTreeCpuMilliseconds - [double]$State.lastProcessTreeCpuMilliseconds
        $wallDelta = $elapsedMilliseconds - [double]$State.lastElapsedMilliseconds
        if ($wallDelta -gt 0) {
            $normalizedCpu = 100.0 * $cpuDelta / $wallDelta / $logicalProcessorCount
            if ($normalizedCpu -lt 0) { $normalizedCpu = 0.0 }
            $processTreeNormalizedCpu =
                100.0 * $processTreeCpuDelta / $wallDelta / $logicalProcessorCount
            if ($processTreeNormalizedCpu -lt 0) { $processTreeNormalizedCpu = 0.0 }
        }
    }
    $State.lastCpuMilliseconds = $cpuMilliseconds
    $State.lastProcessTreeCpuMilliseconds = $processTreeCpuMilliseconds
    $State.lastElapsedMilliseconds = $elapsedMilliseconds

    $privateBytes = [int64]$main.privateBytes
    $workingSetBytes = [int64]$main.workingSetBytes
    $sample = [pscustomobject][ordered]@{
        schemaVersion = $schemaVersion
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        executable = $Candidate.executable
        processId = $Session.processId
        scenario = $Scenario
        pageTag = $Page
        repetition = $Repetition
        phase = $Phase
        sampleIndex = $SampleIndex
        timestampUtc = (Get-Date).ToUniversalTime().ToString("o")
        elapsedMilliseconds = [Math]::Round($elapsedMilliseconds, 3)
        privateBytes = $privateBytes
        privateMiB = [Math]::Round($privateBytes / 1MB, 3)
        workingSetBytes = $workingSetBytes
        workingSetMiB = [Math]::Round($workingSetBytes / 1MB, 3)
        cpuTotalMilliseconds = [Math]::Round($cpuMilliseconds, 3)
        normalizedCpuPercent = $(if ($null -eq $normalizedCpu) {
            $null
        } else {
            [Math]::Round($normalizedCpu, 4)
        })
        threadCount = $main.threadCount
        handleCount = $main.handleCount
        processTreeProcessCount = $tree.processCount
        processTreeDescendantCount = $tree.descendantCount
        processTreePrivateBytes = [int64]$tree.privateBytes
        processTreePrivateMiB = [Math]::Round([int64]$tree.privateBytes / 1MB, 3)
        processTreeWorkingSetBytes = [int64]$tree.workingSetBytes
        processTreeWorkingSetMiB = [Math]::Round([int64]$tree.workingSetBytes / 1MB, 3)
        processTreeCpuTotalMilliseconds = [Math]::Round($processTreeCpuMilliseconds, 3)
        processTreeNormalizedCpuPercent = $(if ($null -eq $processTreeNormalizedCpu) {
            $null
        } else {
            [Math]::Round($processTreeNormalizedCpu, 4)
        })
        processTreeThreadCount = [int64]$tree.threadCount
        processTreeHandleCount = [int64]$tree.handleCount
    }
    $rawSamples.Add($sample)
    $Destination.Add($sample)
    return $sample
}

function Measure-ResourcePhase {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Scenario,
        [Parameter(Mandatory = $true)][string]$Page,
        [Parameter(Mandatory = $true)][int]$Repetition,
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][int]$DurationSeconds,
        [Parameter(Mandatory = $true)][int]$IntervalMilliseconds,
        [Parameter(Mandatory = $true)][Diagnostics.Stopwatch]$Stopwatch,
        [Parameter(Mandatory = $true)][hashtable]$State
    )

    $samples = [System.Collections.Generic.List[object]]::new()
    $phaseEnd = $Stopwatch.Elapsed.TotalMilliseconds + ($DurationSeconds * 1000.0)
    $sampleIndex = 0
    while ($true) {
        [void](Add-ResourceSample `
            -Session $Session `
            -Candidate $Candidate `
            -Scenario $Scenario `
            -Page $Page `
            -Repetition $Repetition `
            -Phase $Phase `
            -SampleIndex $sampleIndex `
            -Stopwatch $Stopwatch `
            -State $State `
            -Destination $samples)
        $sampleIndex++

        $remaining = $phaseEnd - $Stopwatch.Elapsed.TotalMilliseconds
        if ($remaining -le 0) {
            break
        }
        Start-Sleep -Milliseconds ([int][Math]::Min($IntervalMilliseconds, [Math]::Ceiling($remaining)))
    }
    return @($samples.ToArray())
}

function Get-Percentile {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Value,
        [Parameter(Mandatory = $true)][ValidateRange(0.0, 1.0)][double]$Percentile
    )

    $values = @($Value | Where-Object { $null -ne $_ } | ForEach-Object { [double]$_ } | Sort-Object)
    if ($values.Count -eq 0) { return $null }
    if ($Percentile -eq 0.5 -and $values.Count % 2 -eq 0) {
        $upper = [int]($values.Count / 2)
        return ($values[$upper - 1] + $values[$upper]) / 2.0
    }
    $index = [Math]::Ceiling($Percentile * $values.Count) - 1
    if ($index -lt 0) { $index = 0 }
    if ($index -ge $values.Count) { $index = $values.Count - 1 }
    return $values[$index]
}

function Round-Nullable {
    param($Value, [int]$Digits = 3)

    if ($null -eq $Value) { return $null }
    return [Math]::Round([double]$Value, $Digits)
}

function Get-SampleSummary {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Sample)

    $samples = @($Sample)
    if ($samples.Count -eq 0) { return $null }
    $last = $samples[$samples.Count - 1]
    $cpu = @($samples | ForEach-Object { $_.normalizedCpuPercent } | Where-Object { $null -ne $_ })
    $processTreeCpu = @($samples | ForEach-Object {
        $_.processTreeNormalizedCpuPercent
    } | Where-Object { $null -ne $_ })
    $averageCpu = $null
    $averageProcessTreeCpu = $null
    if ($samples.Count -gt 1) {
        $first = $samples[0]
        $wallDelta = [double]$last.elapsedMilliseconds - [double]$first.elapsedMilliseconds
        $cpuDelta = [double]$last.cpuTotalMilliseconds - [double]$first.cpuTotalMilliseconds
        $processTreeCpuDelta =
            [double]$last.processTreeCpuTotalMilliseconds -
            [double]$first.processTreeCpuTotalMilliseconds
        if ($wallDelta -gt 0) {
            $averageCpu = 100.0 * $cpuDelta / $wallDelta / $logicalProcessorCount
            $averageProcessTreeCpu =
                100.0 * $processTreeCpuDelta / $wallDelta / $logicalProcessorCount
        }
    }
    return [pscustomobject][ordered]@{
        sampleCount = $samples.Count
        privateMiB = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile -Value @($samples.privateMiB) -Percentile 0.5)
            p95 = Round-Nullable (Get-Percentile -Value @($samples.privateMiB) -Percentile 0.95)
            max = Round-Nullable (($samples.privateMiB | Measure-Object -Maximum).Maximum)
            end = Round-Nullable $last.privateMiB
        }
        workingSetMiB = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile -Value @($samples.workingSetMiB) -Percentile 0.5)
            p95 = Round-Nullable (Get-Percentile -Value @($samples.workingSetMiB) -Percentile 0.95)
            max = Round-Nullable (($samples.workingSetMiB | Measure-Object -Maximum).Maximum)
            end = Round-Nullable $last.workingSetMiB
        }
        normalizedCpuPercent = [pscustomobject][ordered]@{
            average = Round-Nullable $averageCpu 4
            p95 = Round-Nullable (Get-Percentile -Value $cpu -Percentile 0.95) 4
            max = Round-Nullable (($cpu | Measure-Object -Maximum).Maximum) 4
        }
        processTreePrivateMiB = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile `
                -Value @($samples.processTreePrivateMiB) -Percentile 0.5)
            p95 = Round-Nullable (Get-Percentile `
                -Value @($samples.processTreePrivateMiB) -Percentile 0.95)
            max = Round-Nullable (
                ($samples.processTreePrivateMiB | Measure-Object -Maximum).Maximum)
            end = Round-Nullable $last.processTreePrivateMiB
        }
        processTreeWorkingSetMiB = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile `
                -Value @($samples.processTreeWorkingSetMiB) -Percentile 0.5)
            p95 = Round-Nullable (Get-Percentile `
                -Value @($samples.processTreeWorkingSetMiB) -Percentile 0.95)
            max = Round-Nullable (
                ($samples.processTreeWorkingSetMiB | Measure-Object -Maximum).Maximum)
            end = Round-Nullable $last.processTreeWorkingSetMiB
        }
        processTreeNormalizedCpuPercent = [pscustomobject][ordered]@{
            average = Round-Nullable $averageProcessTreeCpu 4
            p95 = Round-Nullable (
                Get-Percentile -Value $processTreeCpu -Percentile 0.95) 4
            max = Round-Nullable (($processTreeCpu | Measure-Object -Maximum).Maximum) 4
        }
        processTreeProcesses = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile `
                -Value @($samples.processTreeProcessCount) -Percentile 0.5)
            max = Round-Nullable (
                ($samples.processTreeProcessCount | Measure-Object -Maximum).Maximum)
            end = $last.processTreeProcessCount
        }
        threads = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile -Value @($samples.threadCount) -Percentile 0.5)
            max = Round-Nullable (($samples.threadCount | Measure-Object -Maximum).Maximum)
            end = $last.threadCount
        }
        handles = [pscustomobject][ordered]@{
            median = Round-Nullable (Get-Percentile -Value @($samples.handleCount) -Percentile 0.5)
            max = Round-Nullable (($samples.handleCount | Measure-Object -Maximum).Maximum)
            end = $last.handleCount
        }
    }
}

function Get-ScanStatusSnapshot {
    param([Parameter(Mandatory = $true)]$Root)

    $interesting = [System.Collections.Generic.List[string]]::new()
    try {
        $elements = $Root.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $elements.Count; $index++) {
            $name = $null
            try { $name = [string]$elements.Item($index).Current.Name } catch { continue }
            if ([string]::IsNullOrWhiteSpace($name)) { continue }
            if ($name.StartsWith("Quick Scan complete", [StringComparison]::Ordinal)) {
                return [pscustomobject]@{ state = "complete"; text = $name; details = @() }
            }
            if ($name -match "^(Could not start diagnostics|Native diagnostics are unavailable|Quick Scan has no available tasks|Quick Scan failed|Quick Scan stopped|Quick Scan returned an invalid result set|Quick Scan could not enter|Native diagnostic event delivery stopped)") {
                return [pscustomobject]@{ state = "failed"; text = $name; details = @() }
            }
            if ($name -match "(?i)(quick scan|diagnostic|finalizing|saving.+history)") {
                if (-not $interesting.Contains($name)) { $interesting.Add($name) }
            }
        }
        return [pscustomobject]@{
            state = "pending"
            text = $null
            details = @($interesting | Select-Object -Last 12)
        }
    }
    catch {
        return [pscustomobject]@{
            state = "pending"
            text = $null
            details = @("UI Automation probe failed transiently: $($_.Exception.Message)")
        }
    }
}

function Add-Failure {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Scenario,
        [string]$Page,
        [int]$Repetition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $failures.Add([pscustomobject][ordered]@{
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        scenario = $Scenario
        pageTag = $Page
        repetition = $Repetition
        message = $Message
    })
}

function Invoke-WarmupRun {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][int]$WarmupIndex
    )

    $record = [ordered]@{
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        executable = $Candidate.executable
        scenario = "warmup"
        pageTag = "diagnostics"
        repetition = $WarmupIndex
        startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
        completedAtUtc = $null
        status = "running"
        error = $null
        close = $null
        summary = $null
    }
    $session = $null
    try {
        $session = Start-OwnedCandidate -Candidate $Candidate -InitialPage "diagnostics"
        if ($WarmupSeconds -gt 0) { Start-Sleep -Seconds $WarmupSeconds }
        $record.status = "success"
    }
    catch {
        $record.status = "failed"
        $record.error = $_.Exception.Message
    }
    finally {
        if ($null -ne $session) {
            $record.close = Stop-OwnedCandidate -Session $session
            if (-not $record.close.graceful) {
                $record.status = "failed"
                if ($null -eq $record.error) { $record.error = $record.close.error }
            }
        }
        $record.completedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    }
    if ($record.status -ne "success") {
        Add-Failure -Candidate $Candidate -Scenario "warmup" -Page "diagnostics" `
            -Repetition $WarmupIndex -Message $record.error
    }
    $runs.Add([pscustomobject]$record)
}

function Invoke-IdleRun {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Page,
        [Parameter(Mandatory = $true)][int]$Repetition
    )

    $record = [ordered]@{
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        executable = $Candidate.executable
        scenario = "idle"
        pageTag = $Page
        repetition = $Repetition
        startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
        completedAtUtc = $null
        status = "running"
        error = $null
        close = $null
        summary = $null
    }
    $session = $null
    try {
        $session = Start-OwnedCandidate -Candidate $Candidate -InitialPage $Page
        if ($SettleSeconds -gt 0) { Start-Sleep -Seconds $SettleSeconds }
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        $state = New-SampleState
        $samples = @(Measure-ResourcePhase `
            -Session $session `
            -Candidate $Candidate `
            -Scenario "idle" `
            -Page $Page `
            -Repetition $Repetition `
            -Phase "idle" `
            -DurationSeconds $IdleSampleSeconds `
            -IntervalMilliseconds $IdleSampleMilliseconds `
            -Stopwatch $stopwatch `
            -State $state)
        $record.summary = Get-SampleSummary -Sample $samples
        $record.status = "success"
    }
    catch {
        $record.status = "failed"
        $record.error = $_.Exception.Message
    }
    finally {
        if ($null -ne $session) {
            $record.close = Stop-OwnedCandidate -Session $session
            if (-not $record.close.graceful) {
                $record.status = "failed"
                if ($null -eq $record.error) { $record.error = $record.close.error }
            }
        }
        $record.completedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    }
    if ($record.status -ne "success") {
        Add-Failure -Candidate $Candidate -Scenario "idle" -Page $Page `
            -Repetition $Repetition -Message $record.error
    }
    $runs.Add([pscustomobject]$record)
}

function Invoke-QuickScanRun {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][int]$Repetition
    )

    $record = [ordered]@{
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        executable = $Candidate.executable
        scenario = "quick-scan"
        pageTag = "diagnostics"
        repetition = $Repetition
        startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
        completedAtUtc = $null
        status = "running"
        error = $null
        close = $null
        scanStatus = $null
        scanDurationSeconds = $null
        summary = $null
    }
    $session = $null
    try {
        $session = Start-OwnedCandidate -Candidate $Candidate -InitialPage "diagnostics"
        if ($SettleSeconds -gt 0) { Start-Sleep -Seconds $SettleSeconds }

        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        $state = New-SampleState
        $preSamples = @(Measure-ResourcePhase `
            -Session $session `
            -Candidate $Candidate `
            -Scenario "quick-scan" `
            -Page "diagnostics" `
            -Repetition $Repetition `
            -Phase "pre" `
            -DurationSeconds $QuickScanBaselineSeconds `
            -IntervalMilliseconds 1000 `
            -Stopwatch $stopwatch `
            -State $state)

        $root = Get-ReactorUiaRoot -Process $session.process
        $button = Wait-UniqueUiaButton -Root $root `
            -Deadline (Get-Date).AddSeconds(10) -Name "Quick Scan"
        Invoke-UiaButtonElement -Element $button.element

        $scanStartMilliseconds = $stopwatch.Elapsed.TotalMilliseconds
        $scanDeadlineMilliseconds = $scanStartMilliseconds + ($QuickScanTimeoutSeconds * 1000.0)
        $activeSamples = [System.Collections.Generic.List[object]]::new()
        $activeIndex = 0
        $nextStatusCheck = $scanStartMilliseconds
        $lastStatus = [pscustomobject]@{ state = "pending"; text = $null; details = @() }
        while ($true) {
            [void](Add-ResourceSample `
                -Session $session `
                -Candidate $Candidate `
                -Scenario "quick-scan" `
                -Page "diagnostics" `
                -Repetition $Repetition `
                -Phase "active" `
                -SampleIndex $activeIndex `
                -Stopwatch $stopwatch `
                -State $state `
                -Destination $activeSamples)
            $activeIndex++

            $elapsed = $stopwatch.Elapsed.TotalMilliseconds
            if ($elapsed -ge $nextStatusCheck) {
                $lastStatus = Get-ScanStatusSnapshot -Root $root
                $nextStatusCheck = $elapsed + 1000.0
                if ($lastStatus.state -eq "complete") { break }
                if ($lastStatus.state -eq "failed") {
                    throw "Quick Scan reported a failure: $($lastStatus.text)"
                }
            }
            if ($elapsed -ge $scanDeadlineMilliseconds) {
                $details = @($lastStatus.details) -join " | "
                if ([string]::IsNullOrWhiteSpace($details)) { $details = "no scan status text was exposed" }
                throw (
                    "Quick Scan did not complete within $QuickScanTimeoutSeconds seconds. " +
                    "Last UI Automation evidence: $details")
            }
            Start-Sleep -Milliseconds $QuickScanSampleMilliseconds
        }

        $record.scanStatus = $lastStatus.text
        $record.scanDurationSeconds = [Math]::Round(
            ($stopwatch.Elapsed.TotalMilliseconds - $scanStartMilliseconds) / 1000.0, 3)

        $postSettleSamples = @(Measure-ResourcePhase `
            -Session $session `
            -Candidate $Candidate `
            -Scenario "quick-scan" `
            -Page "diagnostics" `
            -Repetition $Repetition `
            -Phase "post-settle" `
            -DurationSeconds $RetainedSettleSeconds `
            -IntervalMilliseconds $QuickScanSampleMilliseconds `
            -Stopwatch $stopwatch `
            -State $state)
        $retainedSamples = @(Measure-ResourcePhase `
            -Session $session `
            -Candidate $Candidate `
            -Scenario "quick-scan" `
            -Page "diagnostics" `
            -Repetition $Repetition `
            -Phase "retained" `
            -DurationSeconds $RetainedSampleSeconds `
            -IntervalMilliseconds $QuickScanSampleMilliseconds `
            -Stopwatch $stopwatch `
            -State $state)

        $preSummary = Get-SampleSummary -Sample $preSamples
        $activeSummary = Get-SampleSummary -Sample @($activeSamples.ToArray())
        $postSettleSummary = Get-SampleSummary -Sample $postSettleSamples
        $retainedSummary = Get-SampleSummary -Sample $retainedSamples
        $record.summary = [pscustomobject][ordered]@{
            pre = $preSummary
            active = $activeSummary
            postSettle = $postSettleSummary
            retained = $retainedSummary
            retainedPrivateDeltaMiB = Round-Nullable (
                $retainedSummary.privateMiB.median - $preSummary.privateMiB.median)
            retainedWorkingSetDeltaMiB = Round-Nullable (
                $retainedSummary.workingSetMiB.median - $preSummary.workingSetMiB.median)
            retainedProcessTreePrivateDeltaMiB = Round-Nullable (
                $retainedSummary.processTreePrivateMiB.median -
                $preSummary.processTreePrivateMiB.median)
            retainedProcessTreeWorkingSetDeltaMiB = Round-Nullable (
                $retainedSummary.processTreeWorkingSetMiB.median -
                $preSummary.processTreeWorkingSetMiB.median)
        }
        $record.status = "success"
    }
    catch {
        $record.status = "failed"
        $record.error = $_.Exception.Message
    }
    finally {
        if ($null -ne $session) {
            $record.close = Stop-OwnedCandidate -Session $session
            if (-not $record.close.graceful) {
                $record.status = "failed"
                if ($null -eq $record.error) { $record.error = $record.close.error }
            }
        }
        $record.completedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    }
    if ($record.status -ne "success") {
        Add-Failure -Candidate $Candidate -Scenario "quick-scan" -Page "diagnostics" `
            -Repetition $Repetition -Message $record.error
    }
    $runs.Add([pscustomobject]$record)
}

function Get-RunOrder {
    param(
        [Parameter(Mandatory = $true)][object[]]$Candidate,
        [Parameter(Mandatory = $true)][int]$Repetition
    )

    if ($Candidate.Count -eq 2 -and $Repetition % 2 -eq 0) {
        return @($Candidate[1], $Candidate[0])
    }
    return @($Candidate)
}

function Get-AggregateRecord {
    param(
        [Parameter(Mandatory = $true)]$Candidate,
        [Parameter(Mandatory = $true)][string]$Scenario,
        [Parameter(Mandatory = $true)][string]$Page,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $successfulRuns = @($runs | Where-Object {
        $_.candidateId -eq $Candidate.id -and
        $_.scenario -eq $Scenario -and
        $_.pageTag -eq $Page -and
        $_.status -eq "success"
    })
    $successfulRepetitions = @($successfulRuns | ForEach-Object { $_.repetition })
    $samples = @($rawSamples | Where-Object {
        $_.candidateId -eq $Candidate.id -and
        $_.scenario -eq $Scenario -and
        $_.pageTag -eq $Page -and
        $_.phase -eq $Phase -and
        $_.repetition -in $successfulRepetitions
    })
    return [pscustomobject][ordered]@{
        candidateId = $Candidate.id
        candidateLabel = $Candidate.label
        scenario = $Scenario
        pageTag = $Page
        phase = $Phase
        successfulRuns = $successfulRuns.Count
        requestedRuns = $Repetitions
        metrics = Get-SampleSummary -Sample $samples
    }
}

function Get-DeltaValue {
    param($First, $Second)

    if ($null -eq $First -or $null -eq $Second) { return $null }
    return Round-Nullable ([double]$Second - [double]$First) 4
}

function Get-PercentDelta {
    param($First, $Second)

    if ($null -eq $First -or $null -eq $Second -or [double]$First -eq 0.0) { return $null }
    return Round-Nullable (100.0 * ([double]$Second - [double]$First) / [double]$First) 3
}

function New-SummaryDocument {
    param([Parameter(Mandatory = $true)][object[]]$Candidate)

    $aggregates = [System.Collections.Generic.List[object]]::new()
    foreach ($item in $Candidate) {
        foreach ($page in $PageTag) {
            $aggregates.Add((Get-AggregateRecord `
                -Candidate $item -Scenario "idle" -Page $page -Phase "idle"))
        }
        if (-not $SkipQuickScan) {
            foreach ($phase in @("pre", "active", "retained")) {
                $aggregates.Add((Get-AggregateRecord `
                    -Candidate $item -Scenario "quick-scan" `
                    -Page "diagnostics" -Phase $phase))
            }
        }
    }

    $quickScan = [System.Collections.Generic.List[object]]::new()
    if (-not $SkipQuickScan) {
        foreach ($item in $Candidate) {
            $successful = @($runs | Where-Object {
                $_.candidateId -eq $item.id -and
                $_.scenario -eq "quick-scan" -and
                $_.status -eq "success"
            })
            $quickScan.Add([pscustomobject][ordered]@{
                candidateId = $item.id
                candidateLabel = $item.label
                successfulRuns = $successful.Count
                requestedRuns = $Repetitions
                scanDurationSecondsMedian = Round-Nullable (
                    Get-Percentile -Value @($successful.scanDurationSeconds) -Percentile 0.5)
                retainedPrivateDeltaMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.retainedPrivateDeltaMiB
                        }) `
                        -Percentile 0.5)
                retainedWorkingSetDeltaMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.retainedWorkingSetDeltaMiB
                        }) `
                        -Percentile 0.5)
                retainedProcessTreePrivateDeltaMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.retainedProcessTreePrivateDeltaMiB
                        }) `
                        -Percentile 0.5)
                retainedProcessTreeWorkingSetDeltaMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.retainedProcessTreeWorkingSetDeltaMiB
                        }) `
                        -Percentile 0.5)
                activePrivatePeakMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.active.privateMiB.max
                        }) `
                        -Percentile 0.5)
                activeProcessTreePrivatePeakMiBMedian = Round-Nullable (
                    Get-Percentile `
                        -Value @($successful | ForEach-Object {
                            $_.summary.active.processTreePrivateMiB.max
                        }) `
                        -Percentile 0.5)
            })
        }
    }

    $comparisons = [System.Collections.Generic.List[object]]::new()
    if ($Candidate.Count -eq 2) {
        $firstAggregates = @($aggregates | Where-Object {
            $_.candidateId -eq $Candidate[0].id
        })
        foreach ($first in $firstAggregates) {
            $second = $aggregates | Where-Object {
                $_.candidateId -eq $Candidate[1].id -and
                $_.scenario -eq $first.scenario -and
                $_.pageTag -eq $first.pageTag -and
                $_.phase -eq $first.phase
            } | Select-Object -First 1
            if ($null -eq $second -or $null -eq $first.metrics -or $null -eq $second.metrics) {
                continue
            }
            $comparisons.Add([pscustomobject][ordered]@{
                scenario = $first.scenario
                pageTag = $first.pageTag
                phase = $first.phase
                firstCandidateId = $Candidate[0].id
                secondCandidateId = $Candidate[1].id
                semantics = "second minus first"
                delta = [pscustomobject][ordered]@{
                    privateMedianMiB = Get-DeltaValue `
                        $first.metrics.privateMiB.median $second.metrics.privateMiB.median
                    privateMedianPercent = Get-PercentDelta `
                        $first.metrics.privateMiB.median $second.metrics.privateMiB.median
                    workingSetMedianMiB = Get-DeltaValue `
                        $first.metrics.workingSetMiB.median $second.metrics.workingSetMiB.median
                    workingSetMedianPercent = Get-PercentDelta `
                        $first.metrics.workingSetMiB.median $second.metrics.workingSetMiB.median
                    normalizedCpuAveragePercent = Get-DeltaValue `
                        $first.metrics.normalizedCpuPercent.average `
                        $second.metrics.normalizedCpuPercent.average
                    processTreePrivateMedianMiB = Get-DeltaValue `
                        $first.metrics.processTreePrivateMiB.median `
                        $second.metrics.processTreePrivateMiB.median
                    processTreePrivateMedianPercent = Get-PercentDelta `
                        $first.metrics.processTreePrivateMiB.median `
                        $second.metrics.processTreePrivateMiB.median
                    processTreeWorkingSetMedianMiB = Get-DeltaValue `
                        $first.metrics.processTreeWorkingSetMiB.median `
                        $second.metrics.processTreeWorkingSetMiB.median
                    processTreeWorkingSetMedianPercent = Get-PercentDelta `
                        $first.metrics.processTreeWorkingSetMiB.median `
                        $second.metrics.processTreeWorkingSetMiB.median
                    processTreeNormalizedCpuAveragePercent = Get-DeltaValue `
                        $first.metrics.processTreeNormalizedCpuPercent.average `
                        $second.metrics.processTreeNormalizedCpuPercent.average
                    threadMedian = Get-DeltaValue `
                        $first.metrics.threads.median $second.metrics.threads.median
                    handleMedian = Get-DeltaValue `
                        $first.metrics.handles.median $second.metrics.handles.median
                }
            })
        }
    }

    return [pscustomobject][ordered]@{
        schemaVersion = $schemaVersion
        startedAtUtc = $startedAtUtc
        completedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
        outputDirectory = $OutputDirectory
        logicalProcessorCount = $logicalProcessorCount
        cpuDefinition = (
            "100 * launched-process CPU-time delta / wall-time delta / logical processor count")
        processTreeCpuDefinition = (
            "100 * verified owned-tree CPU-time delta / wall-time delta / logical processor count")
        processTreeOwnershipDefinition = (
            "Exact launched PID plus recursively discovered live descendants; " +
            "every edge is checked against process creation time to reject stale parent-PID reuse.")
        comparisonSemantics = $(if ($Candidate.Count -eq 2) {
            "All deltas are second supplied executable minus first supplied executable."
        } else { $null })
        configuration = [pscustomobject][ordered]@{
            pageTags = @($PageTag)
            warmupRuns = $WarmupRuns
            warmupSeconds = $WarmupSeconds
            repetitions = $Repetitions
            settleSeconds = $SettleSeconds
            idleSampleSeconds = $IdleSampleSeconds
            idleSampleMilliseconds = $IdleSampleMilliseconds
            quickScanIncluded = (-not $SkipQuickScan)
            quickScanBaselineSeconds = $QuickScanBaselineSeconds
            quickScanTimeoutSeconds = $QuickScanTimeoutSeconds
            quickScanSampleMilliseconds = $QuickScanSampleMilliseconds
            retainedSettleSeconds = $RetainedSettleSeconds
            retainedSampleSeconds = $RetainedSampleSeconds
            windowWidth = $WindowWidth
            windowHeight = $WindowHeight
            liveNonFixture = $true
            noTray = $true
            launchModes = @($Candidate | ForEach-Object { $_.launchMode })
            processTreeMetricsIncluded = $true
        }
        candidates = @($Candidate)
        aggregates = @($aggregates.ToArray())
        quickScan = @($quickScan.ToArray())
        comparisons = @($comparisons.ToArray())
        failureCount = $failures.Count
        failures = @($failures.ToArray())
    }
}

function Write-Evidence {
    param([Parameter(Mandatory = $true)][object[]]$Candidate)

    $rawArray = @($rawSamples.ToArray())
    $csvPath = Join-Path $OutputDirectory "raw-samples.csv"
    if ($rawArray.Count -gt 0) {
        $rawArray | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8
    }
    else {
        @"
"schemaVersion","candidateId","candidateLabel","executable","processId","scenario","pageTag","repetition","phase","sampleIndex","timestampUtc","elapsedMilliseconds","privateBytes","privateMiB","workingSetBytes","workingSetMiB","cpuTotalMilliseconds","normalizedCpuPercent","threadCount","handleCount","processTreeProcessCount","processTreeDescendantCount","processTreePrivateBytes","processTreePrivateMiB","processTreeWorkingSetBytes","processTreeWorkingSetMiB","processTreeCpuTotalMilliseconds","processTreeNormalizedCpuPercent","processTreeThreadCount","processTreeHandleCount"
"@.Trim() | Set-Content -LiteralPath $csvPath -Encoding UTF8
    }
    ConvertTo-Json -InputObject $rawArray -Depth 8 |
        Set-Content -LiteralPath (Join-Path $OutputDirectory "raw-samples.json") -Encoding UTF8
    ConvertTo-Json -InputObject @($runs.ToArray()) -Depth 12 |
        Set-Content -LiteralPath (Join-Path $OutputDirectory "runs.json") -Encoding UTF8
    New-SummaryDocument -Candidate $Candidate | ConvertTo-Json -Depth 12 |
        Set-Content -LiteralPath (Join-Path $OutputDirectory "summary.json") -Encoding UTF8
}

$candidates = [System.Collections.Generic.List[object]]::new()
for ($index = 0; $index -lt $Executable.Count; $index++) {
    $resolved = (Resolve-Path -LiteralPath $Executable[$index] -ErrorAction Stop).ProviderPath
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Executable is not a file: $resolved"
    }
    if ([IO.Path]::GetExtension($resolved) -ine ".exe") {
        throw "Candidate path is not an .exe: $resolved"
    }
    $resolved = ConvertTo-FullPath -Path $resolved
    if (@($candidates | Where-Object { Test-SamePath $_.executable $resolved }).Count -gt 0) {
        throw "The same executable was supplied more than once: $resolved"
    }
    $candidateLabel = if ($Label.Count -gt 0) {
        $Label[$index]
    }
    else {
        Split-Path -Leaf (Split-Path -Parent $resolved)
    }
    if ([string]::IsNullOrWhiteSpace($candidateLabel)) {
        $candidateLabel = [IO.Path]::GetFileName($resolved)
    }
    $file = Get-Item -LiteralPath $resolved
    $candidateLaunchMode = if ($LaunchMode.Count -eq 0) {
        "reactor"
    }
    elseif ($LaunchMode.Count -eq 1) {
        $LaunchMode[0]
    }
    else {
        $LaunchMode[$index]
    }
    $candidates.Add([pscustomobject][ordered]@{
        id = "candidate-$($index + 1)"
        label = $candidateLabel
        executable = $resolved
        launchMode = $candidateLaunchMode
        sha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash
        bytes = $file.Length
        lastWriteTimeUtc = $file.LastWriteTimeUtc.ToString("o")
    })
}
$candidateArray = @($candidates.ToArray())

# Start-ReactorCandidate already restores its launch environment. Preserve the
# same variables around the complete benchmark as a second boundary so an
# unexpected exception cannot leak a measurement setting into the caller.
$environmentNames = @(
    "WFDIAG_REACTOR_PAGE",
    "WFDIAG_REACTOR_VISUAL_STATE",
    "WFDIAG_REACTOR_FIXTURE",
    "WFDIAG_REACTOR_SETTINGS",
    "WFDIAG_REACTOR_SETTINGS_TEST_PATH",
    "WFDIAG_REACTOR_LIVE_TEST_FIXTURE",
    "WFDIAG_REACTOR_WIDTH",
    "WFDIAG_REACTOR_HEIGHT",
    "WFDIAG_REACTOR_THEME",
    "WFDIAG_NO_TRAY",
    "WFDIAG_NO_WORKERS"
)
$savedEnvironment = @{}
foreach ($name in $environmentNames) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$fatalError = $null
try {
    foreach ($candidate in $candidateArray) {
        for ($warmup = 1; $warmup -le $WarmupRuns; $warmup++) {
            Write-Host "Warm-up $warmup/${WarmupRuns}: $($candidate.label)"
            Invoke-WarmupRun -Candidate $candidate -WarmupIndex $warmup
        }
    }

    foreach ($page in $PageTag) {
        for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
            foreach ($candidate in @(Get-RunOrder `
                -Candidate $candidateArray -Repetition $repetition)) {
                Write-Host (
                    "Idle {0}, repetition {1}/{2}: {3}" -f
                    $page, $repetition, $Repetitions, $candidate.label)
                Invoke-IdleRun -Candidate $candidate -Page $page -Repetition $repetition
            }
        }
    }

    if (-not $SkipQuickScan) {
        for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
            foreach ($candidate in @(Get-RunOrder `
                -Candidate $candidateArray -Repetition $repetition)) {
                Write-Host (
                    "Quick Scan, repetition {0}/{1}: {2}" -f
                    $repetition, $Repetitions, $candidate.label)
                Invoke-QuickScanRun -Candidate $candidate -Repetition $repetition
            }
        }
    }
}
catch {
    $fatalError = $_.Exception.Message
    $failures.Add([pscustomobject][ordered]@{
        candidateId = $null
        candidateLabel = $null
        scenario = "harness"
        pageTag = $null
        repetition = 0
        message = $fatalError
    })
}
finally {
    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
    }
    Write-Evidence -Candidate $candidateArray
}

Write-Host "Raw CSV: $(Join-Path $OutputDirectory 'raw-samples.csv')"
Write-Host "Raw JSON: $(Join-Path $OutputDirectory 'raw-samples.json')"
Write-Host "Run evidence: $(Join-Path $OutputDirectory 'runs.json')"
Write-Host "Summary: $(Join-Path $OutputDirectory 'summary.json')"

if ($null -ne $fatalError) {
    Write-Error "Resource benchmark aborted: $fatalError"
    exit 1
}
if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Warning (
            "{0}/{1}/run {2}: {3}" -f
            $failure.scenario, $failure.pageTag, $failure.repetition, $failure.message)
    }
    Write-Error "Resource benchmark completed with $($failures.Count) failed run(s)."
    exit 1
}

Write-Host "Resource benchmark completed successfully."
exit 0
