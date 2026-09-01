# Reactor validation orchestrator: run validation suites and aggregate one
# JSON report.
#
# Suites:
#   startup - repeated native startup/settings lifecycle gate
#   live-system - live native system projection + crash/module checks
#   about   - installed Store oracle vs Reactor About-dialog parity
#   flows   - mandatory hermetic AI plus supplemental live chat/report and remediation
#   visual  - capture-reactor-variants.ps1 + check-variants.py
#   x64     - build + validate the x64 candidate (host or CI)
#   readiness - repository-wide cutover readiness contract
#   gates   - scripts/check-external-gates.py (crates.io, runtime drift)
#   all     - everything above
#
# Aggregated report: validation-reports/<stamp>/summary.json

param(
    [ValidateSet("startup", "live-system", "about", "flows", "visual", "x64", "readiness", "gates", "all")]
    [string[]]$Suite = @("all"),
    [string]$Executable = "C:\Temp\claude\wfdiag\target\aarch64-pc-windows-msvc\debug\wfdiag-reactor-spike.exe",
    [string]$BuildRoot = "C:\Temp\claude\wfdiag",
    [string]$ReportsRoot = "validation-reports"
)

$ErrorActionPreference = "Continue"
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
if ($Suite -contains "all") {
    $Suite = @(
        "startup", "live-system", "about", "flows", "visual", "x64",
        "readiness", "gates"
    )
}
$reportDirectory = Join-Path $ReportsRoot $stamp
if (-not (Test-Path -LiteralPath $reportDirectory)) {
    New-Item -ItemType Directory -Path $reportDirectory -Force | Out-Null
}
$reportDirectory = (Resolve-Path -LiteralPath $reportDirectory).Path

$summary = [ordered]@{
    startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    executable = $Executable
    suites = [ordered]@{}
    failures = [System.Collections.Generic.List[string]]::new()
}

if ($Suite -contains "startup") {
    Write-Host "`n=== startup ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "test-reactor-startup.ps1") `
        -Executable $Executable 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["startup"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("startup exited with code $code")
    }
}

if ($Suite -contains "live-system") {
    Write-Host "`n=== live-system ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "test-reactor-live-system.ps1") `
        -Executable $Executable `
        -OutputDirectory (Join-Path $reportDirectory "live-system") 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["live-system"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("live-system exited with code $code")
    }
}

if ($Suite -contains "about") {
    Write-Host "`n=== about: Store/Reactor parity ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "test-reactor-about-parity.ps1") `
        -Executable $Executable `
        -OutputDirectory (Join-Path $reportDirectory "about-parity") 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["about"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("about exited with code $code")
    }
}

if ($Suite -contains "flows") {
    foreach ($name in @("ai-flows", "chat", "report", "remediation")) {
        Write-Host "`n=== flows: $name ==="
        $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "test-reactor-$name.ps1") `
            -Executable $Executable `
            -OutputDirectory (Join-Path $reportDirectory $name) 2>&1
        $code = $LASTEXITCODE
        $output | Select-Object -Last 8 | ForEach-Object { Write-Host "  $_" }
        $summary.suites["flows/$name"] = @{ exitCode = $code }
        if ($code -ne 0) {
            $summary.failures.Add("flows/$name exited with code $code")
        }
    }
}

if ($Suite -contains "visual") {
    # Validation evidence is deliberately report-local. The capture scripts
    # still support their historical tracked-manifest defaults for explicit
    # baseline maintenance, but an ordinary validation run must never rewrite
    # reactor-baselines/variants.json.
    $visualManifest = Join-Path $reportDirectory "visual-variants.json"
    $visualCaptureDirectory = Join-Path $reportDirectory "visual-variants"

    Write-Host "`n=== visual: variants ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "capture-reactor-variants.ps1") `
        -Executable $Executable `
        -OutputDirectory $visualCaptureDirectory `
        -VariantsJson $visualManifest 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 8 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["visual/variants"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("visual/variants exited with code $code")
    }

    Write-Host "`n=== visual: process-refresh triptych ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "test-reactor-process-refresh-parity.ps1") `
        -Executable $Executable `
        -OutputDirectory (Join-Path $reportDirectory "process-refresh") `
        -VariantsJson $visualManifest `
        -StoreBaselinePng (Join-Path $repoRoot "reactor-baselines\captures\store-2.5.8\processes-populated-desktop-dark.png") 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 8 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["visual/process-refresh"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("visual/process-refresh exited with code $code")
    }

    Write-Host "`n=== visual: variants check ==="
    $output = & python (Join-Path $PSScriptRoot "check-variants.py") `
        --manifest $visualManifest --json 2>&1
    $code = $LASTEXITCODE
    $output | ForEach-Object { Write-Host "  $_" }
    $summary.suites["visual/check"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("visual/check reported the isolated variants document not ready")
    }
}

if ($Suite -contains "x64") {
    Write-Host "`n=== x64 ==="
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "test-reactor-x64.ps1") `
        -BuildRoot $BuildRoot 2>&1
    $code = $LASTEXITCODE
    $output | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
    $summary.suites["x64"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("x64 exited with code $code")
    }
}

if ($Suite -contains "readiness") {
    Write-Host "`n=== readiness ==="
    $output = & python (Join-Path $PSScriptRoot "check-reactor-readiness.py") --json 2>&1
    $code = $LASTEXITCODE
    $output | ForEach-Object { Write-Host "  $_" }
    $summary.suites["readiness"] = @{ exitCode = $code }
    if ($code -ne 0) {
        $summary.failures.Add("readiness reported unresolved cutover requirements")
    }
}

if ($Suite -contains "gates") {
    Write-Host "`n=== gates: external ==="
    $output = & python (Join-Path $PSScriptRoot "check-external-gates.py") --json 2>&1
    $code = $LASTEXITCODE
    $output | ForEach-Object { Write-Host "  $_" }
    $summary.suites["gates/external"] = @{
        exitCode = $code
        note = "exit 1 = actionable external change (informational for the owner)"
    }
    if ($code -eq 2) {
        $summary.failures.Add("gates/external failed to run")
    }
}

$summary.completedAtUtc = (Get-Date).ToUniversalTime().ToString("o")
$summaryPath = Join-Path $reportDirectory "summary.json"
$summary | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $summaryPath -Encoding UTF8
Write-Host "`nSummary: $summaryPath"
if ($summary.failures.Count -gt 0) {
    foreach ($failure in $summary.failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "All requested suites passed."
exit 0
