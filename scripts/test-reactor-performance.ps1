<#
.SYNOPSIS
Measure foreground and minimized native-shell resources without fixtures.
.DESCRIPTION
Foreground ownership is asserted on every sample: an inactive/paused window
cannot silently pass as a live-monitor CPU measurement. Only the exact process
launched by this script is closed, through the shared ownership-checking helper.
Use identical release-feature builds for before/after comparisons. Results are
measurements of this machine, not universal CPU/RAM guarantees.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateSet('diagnostics', 'monitor', 'processes', 'ai')][string[]]$Pages = @('diagnostics', 'monitor', 'processes', 'ai'),
    [ValidateRange(3, 120)][int]$SampleSeconds = 15,
    [ValidateRange(1, 60)][int]$SettleSeconds = 8
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'lib\ReactorUia.psm1') -Force
# The WFDIAG_REACTOR_* knobs passed to the candidate are compile-time
# fixtures: without the validation feature they do not exist, a plain build
# ignores them (and writes the developer's real settings), and every page
# would record an identically mislabeled "passed" run. The bounded version
# probe fails fast on such a build and stamps each record with the build
# actually measured.
$candidateVersion = Get-ReactorApplicationVersion -Executable $Executable `
    -ProbeFile (Join-Path $env:TEMP 'wfdiag-reactor-performance-version.json')
Write-Host "Candidate version: $candidateVersion"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WfdiagPerfWindow {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@
[void][IO.Directory]::CreateDirectory($OutputDirectory)
$imageStream = [IO.File]::OpenRead($Executable)
$hasher = [Security.Cryptography.SHA256]::Create()
try {
    $imageHash = [BitConverter]::ToString($hasher.ComputeHash($imageStream)).Replace('-', '').ToLowerInvariant()
} finally {
    $hasher.Dispose()
    $imageStream.Dispose()
}
$results = [Collections.Generic.List[object]]::new()
foreach ($page in $Pages) {
    $session = $null
    $record = [ordered]@{
        page = $page; executable = $Executable; sha256 = $imageHash; version = $candidateVersion
        logicalProcessors = [Environment]::ProcessorCount
        capturedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
        samples = @(); status = 'running'; error = $null; close = $null
    }
    try {
        $session = Start-ReactorCandidate -Executable $Executable -Seconds 20 -Variables @{
            WFDIAG_REACTOR_PAGE = $page
            WFDIAG_REACTOR_SETTINGS_TEST_PATH = (Join-Path $OutputDirectory "$page-settings.json")
            WFDIAG_NO_TRAY = '1'
        }
        $window = $session.process.MainWindowHandle
        [void][WfdiagPerfWindow]::ShowWindow($window, 9)
        [void][WfdiagPerfWindow]::SetForegroundWindow($window)
        $activation = New-Object -ComObject WScript.Shell
        [void]$activation.AppActivate($session.process.Id)
        Start-Sleep -Seconds $SettleSeconds
        $samples = [Collections.Generic.List[object]]::new()
        foreach ($phase in @('foreground', 'minimized')) {
            if ($phase -eq 'minimized') {
                [void][WfdiagPerfWindow]::ShowWindow($window, 6)
                Start-Sleep -Seconds 3
            }
            $watch = [Diagnostics.Stopwatch]::StartNew()
            for ($index = 0; $index -le $SampleSeconds; $index++) {
                $session.process.Refresh()
                if ($session.process.HasExited) { throw 'Candidate exited during sampling' }
                $foreground = [WfdiagPerfWindow]::GetForegroundWindow() -eq $window
                if ($phase -eq 'foreground' -and -not $foreground) {
                    [uint32]$foregroundProcess = 0
                    [void][WfdiagPerfWindow]::GetWindowThreadProcessId([WfdiagPerfWindow]::GetForegroundWindow(), [ref]$foregroundProcess)
                    throw "Candidate PID $($session.process.Id) lost foreground ownership to PID $foregroundProcess; live CPU measurement is invalid"
                }
                $samples.Add([pscustomobject]@{
                    phase = $phase; elapsedMs = $watch.Elapsed.TotalMilliseconds
                    foreground = $foreground; cpuMs = $session.process.TotalProcessorTime.TotalMilliseconds
                    privateMiB = $session.process.PrivateMemorySize64 / 1MB
                    workingSetMiB = $session.process.WorkingSet64 / 1MB
                    threads = $session.process.Threads.Count; handles = $session.process.HandleCount
                })
                if ($index -lt $SampleSeconds) { Start-Sleep -Seconds 1 }
            }
        }
        $record.samples = @($samples.ToArray())
        $record.status = 'passed'
    } catch {
        $record.status = 'failed'
        $record.error = $_.Exception.Message
    } finally {
        if ($null -ne $session) {
            $record.close = Stop-ReactorCandidate -Session $session -ExecutablePaths @($Executable) -GraceSeconds 8
            if (-not $record.close.gracefulClose -or @($record.close.crashEvents).Count -gt 0) {
                $record.status = 'failed'
                $record.error = 'Candidate did not close cleanly or reported a crash'
            }
        }
        $results.Add([pscustomobject]$record)
        $results.ToArray() | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'performance.json') -Encoding UTF8
    }
}
$results | Select-Object page, status, error | Format-Table -AutoSize
if (@($results | Where-Object { $_.status -ne 'passed' }).Count -gt 0) { exit 1 }
