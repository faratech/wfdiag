# Reactor x64 evidence lane: build the candidate for x64 and run the
# validation suites against it.
#
# On the ARM64 dev host the x64 binary executes under Windows-on-Windows
# emulation — accepted INTERIM evidence (recorded as emulated = true). The
# clean x64 signal comes from .github/workflows/reactor-validation.yml on a
# windows-latest runner.
#
# Requires the host's native cargo with the x86_64-pc-windows-msvc target and
# a staged build tree (see docs/validation/clean-machine-protocol.md).

param(
    [string]$BuildRoot = "C:\Temp\claude\wfdiag\reactor-spike",
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\validation-x64",
    [string[]]$Suites = @("live-system", "chat", "report", "remediation"),
    [ValidateRange(30, 600)][int]$ScanWaitSeconds = 240
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
if (-not (Test-Path -LiteralPath $cargo)) {
    throw "Native cargo not found at $cargo."
}

$buildArgs = @(
    "build", "--target", "x86_64-pc-windows-msvc", "--features", "self-contained"
)
Write-Host "Building x64 self-contained candidate..."
& $cargo @buildArgs --manifest-path (Join-Path $BuildRoot "Cargo.toml") 2>&1 |
    Select-Object -Last 3
if ($LASTEXITCODE -ne 0) {
    throw "x64 build failed with exit code $LASTEXITCODE."
}

$executable = Join-Path $BuildRoot "target\x86_64-pc-windows-msvc\debug\wfdiag-reactor-spike.exe"
if (-not (Test-Path -LiteralPath $executable)) {
    throw "Built x64 executable not found: $executable"
}

$isEmulated = $true
$arch = (Get-CimInstance Win32_Processor).Architecture
if ($arch -eq 9) {
    # x64 host: this is native x64 execution, not emulation.
    $isEmulated = $false
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$failures = [System.Collections.Generic.List[string]]::new()
$results = @()

foreach ($suite in $Suites) {
    $scriptPath = if ($suite -eq "live-system") {
        Join-Path $repoRoot "scripts\test-reactor-live-system.ps1"
    }
    else {
        Join-Path $repoRoot "scripts\test-reactor-$suite.ps1"
    }
    if (-not (Test-Path -LiteralPath $scriptPath)) {
        $failures.Add("Suite script missing: $scriptPath")
        continue
    }

    Write-Host "=== Suite: $suite (emulated=$isEmulated) ==="
    $outputDirectory = Join-Path $BuildRoot "validation-$suite"
    $arguments = @{
        Executable = $executable
        OutputDirectory = $outputDirectory
    }
    # Only scan-driven suites declare this parameter. In particular, the
    # chat suite accepts ProviderWaitSeconds/HoldSeconds and Windows
    # PowerShell rejects an unknown -ScanWaitSeconds argument before it can
    # launch the test.
    if (@("report", "remediation") -contains $suite) {
        $arguments.ScanWaitSeconds = $ScanWaitSeconds
    }

    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File $scriptPath @arguments 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 6 | ForEach-Object { Write-Host "  $_" }
    $results += [pscustomobject]@{
        suite = $suite
        exitCode = $code
        emulated = $isEmulated
        output = @($output | Select-Object -Last 12)
    }
    if ($code -ne 0) {
        $failures.Add("Suite '$suite' exited with code $code.")
    }
}

$report = [ordered]@{
    executable = $executable
    emulated = $isEmulated
    suites = $results
    failures = $failures
}
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$reportPath = Join-Path $BuildRoot "x64-validation-$stamp.json"
$report | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host "Report: $reportPath"

if ($failures.Count -gt 0) {
    exit 1
}
Write-Host "x64 validation passed."
exit 0
