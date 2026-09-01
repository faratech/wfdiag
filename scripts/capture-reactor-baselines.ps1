param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [string]$OutputDirectory,

    [int]$WaitSeconds = 8
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path $PSScriptRoot -Parent
$manifestPath = Join-Path $repoRoot "reactor-baselines\manifest.json"
$captureScript = Join-Path $PSScriptRoot "capture-window.ps1"
$versionProbeFlag = "--wfdiag-version-probe"
$versionProbeEnvironment = "WFDIAG_REACTOR_VERSION_PROBE_FILE"
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "apps\wfdiag\captures-2.5.8\final"
}

$Executable = [IO.Path]::GetFullPath($Executable)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "Reactor executable does not exist: $Executable"
}
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Baseline manifest does not exist: $manifestPath"
}
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null

function Get-ReactorApplicationVersion {
    param([Parameter(Mandatory = $true)][string]$Path)

    $probePath = Join-Path ([IO.Path]::GetTempPath()) (
        "wfdiag-reactor-version-{0}.json" -f [Guid]::NewGuid().ToString("N"))
    $previousProbePath = [Environment]::GetEnvironmentVariable(
        $versionProbeEnvironment,
        "Process")

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
            throw "Reactor version probe did not exit within 10 seconds; the executable may not support '$versionProbeFlag'."
        }
        if ($probe.ExitCode -ne 0) {
            throw "Reactor version probe exited with code $($probe.ExitCode)."
        }
        if (-not (Test-Path -LiteralPath $probePath -PathType Leaf)) {
            throw "Reactor version probe did not create '$probePath'."
        }

        try {
            $document = Get-Content -LiteralPath $probePath -Raw | ConvertFrom-Json
        }
        catch {
            throw "Reactor version probe returned invalid JSON: $($_.Exception.Message)"
        }
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

$states = @{
    "diagnostics-empty-desktop-dark" = @{ Page = "diagnostics" }
    "monitor-empty-desktop-dark" = @{ Visual = "monitor-empty-desktop-dark" }
    "processes-empty-desktop-dark" = @{ Visual = "processes-empty-desktop-dark" }
    "ai-empty-desktop-dark" = @{ Page = "ai" }
    "issues-empty-desktop-dark" = @{ Page = "issues" }
    "history-empty-desktop-dark" = @{ Visual = "history-empty-desktop-dark" }
    "ai-empty-compact-dark" = @{ Visual = "ai-empty-compact-dark" }
    "diagnostics-populated-desktop-dark" = @{ Page = "diagnostics"; Fixture = "populated" }
    "issues-populated-desktop-dark" = @{ Page = "issues"; Fixture = "populated" }
    "issue-to-chat-desktop-dark" = @{ Visual = "issue-to-chat-desktop-dark" }
    "processes-populated-desktop-dark" = @{ Page = "processes"; Fixture = "populated" }
    "history-comparison-desktop-dark" = @{ Page = "history"; Fixture = "populated" }
    "monitor-populated-desktop-dark" = @{ Page = "monitor"; Fixture = "populated" }
    "ai-conversation-desktop-dark" = @{ Visual = "ai-conversation-desktop-dark" }
    "ai-conversation-top-compact-dark" = @{ Visual = "ai-conversation-top-compact-dark" }
    "ai-conversation-bottom-compact-dark" = @{ Visual = "ai-conversation-bottom-compact-dark" }
    "settings-top-desktop-dark" = @{ Page = "monitor"; Fixture = "populated"; Settings = "1" }
    "settings-bottom-desktop-dark" = @{ Visual = "settings-bottom-desktop-dark" }
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.baseline.application_version -ne "2.5.8") {
    throw "This capture matrix is pinned to Store 2.5.8, but the manifest says '$($manifest.baseline.application_version)'."
}
$expectedApplicationVersion = [string]$manifest.baseline.application_version
$executableApplicationVersion = Get-ReactorApplicationVersion -Path $Executable
if ($executableApplicationVersion -cne $expectedApplicationVersion) {
    throw "Reactor executable reports application version '$executableApplicationVersion', but the baseline manifest requires '$expectedApplicationVersion'."
}
$screenshots = @($manifest.baseline.screenshots)
if ($screenshots.Count -ne $states.Count) {
    throw "Capture matrix has $($states.Count) states but the manifest has $($screenshots.Count)."
}

$environmentNames = @(
    "WFDIAG_REACTOR_PAGE",
    "WFDIAG_REACTOR_VISUAL_STATE",
    "WFDIAG_REACTOR_FIXTURE",
    "WFDIAG_REACTOR_SETTINGS",
    "WFDIAG_REACTOR_WIDTH",
    "WFDIAG_REACTOR_HEIGHT"
)
$savedEnvironment = @{}
foreach ($name in $environmentNames) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

try {
    foreach ($screenshot in $screenshots) {
        $id = [string]$screenshot.id
        if (-not $states.ContainsKey($id)) {
            throw "No deterministic Reactor launch state is defined for manifest id '$id'."
        }
        $state = $states[$id]
        $width = [int]$screenshot.viewport.width
        $height = [int]$screenshot.viewport.height

        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_PAGE", [string]$state.Page, "Process")
        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_VISUAL_STATE", [string]$state.Visual, "Process")
        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_FIXTURE", [string]$state.Fixture, "Process")
        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_SETTINGS", [string]$state.Settings, "Process")
        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_WIDTH", [string]$width, "Process")
        [Environment]::SetEnvironmentVariable("WFDIAG_REACTOR_HEIGHT", [string]$height, "Process")

        $outputPath = Join-Path $OutputDirectory "$id-reactor-final.png"
        $process = Start-Process -FilePath $Executable -PassThru
        try {
            & $captureScript `
                -ProcessId $process.Id `
                -OutputPath $outputPath `
                -WaitSeconds $WaitSeconds `
                -LogicalWidth $width `
                -LogicalHeight $height
        }
        finally {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            $process.WaitForExit()
        }

        Add-Type -AssemblyName System.Drawing
        $image = [Drawing.Image]::FromFile($outputPath)
        try {
            if ($image.Width -ne $width -or $image.Height -ne $height) {
                throw "Capture '$id' is $($image.Width)x$($image.Height), expected ${width}x${height}."
            }
        }
        finally {
            $image.Dispose()
        }
        Write-Host "Captured $id from PID $($process.Id) at ${width}x${height}."
    }
}
finally {
    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
    }
}

[pscustomobject]@{
    executable = $Executable
    sourceVersion = $expectedApplicationVersion
    executableVersion = $executableApplicationVersion
    captureCount = $screenshots.Count
    outputDirectory = $OutputDirectory
}
