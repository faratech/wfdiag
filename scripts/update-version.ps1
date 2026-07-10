param(
    [string]$NewVersion = $null,
    [switch]$DryRun
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir = Split-Path -Parent $ScriptDir

if (-not $NewVersion) {
    $VersionFile = Join-Path $RootDir "version.json"
    $NewVersion = (Get-Content $VersionFile -Raw | ConvertFrom-Json).version
}

if (-not $NewVersion) {
    Write-Error "No version supplied and version.json has no version field."
    exit 1
}

$Python = Get-Command python3 -ErrorAction SilentlyContinue
if (-not $Python) {
    $Python = Get-Command python -ErrorAction SilentlyContinue
}
if (-not $Python) {
    Write-Error "Python 3 was not found. Install Python 3 or run scripts/bump-version.py directly."
    exit 1
}

$Args = @((Join-Path $ScriptDir "bump-version.py"), $NewVersion)
if ($DryRun) {
    $Args += "--dry-run"
}

& $Python.Source @Args
exit $LASTEXITCODE
