# Reactor UIA regression suite for three user-reported native-XAML defects:
# command-palette keyboard/focus/layout behavior, Keyboard Shortcuts row
# geometry, and Network Connections row/cell geometry.
#
# The suite intentionally uses live UI Automation bounds instead of image
# heuristics. It writes one JSON evidence document and exits non-zero for
# missing controls, invalid rectangles, material overlaps, or focus failures.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\validation-ui-regressions",
    [ValidateRange(5, 60)][int]$StartupWaitSeconds = 10,
    [ValidateRange(5, 120)][int]$NetworkWaitSeconds = 30,
    [ValidateRange(1, 10)][int]$MinimumNetworkRows = 2,
    [ValidateRange(100, 3000)][int]$FocusObservationMilliseconds = 600,
    [ValidateRange(0, 10)][double]$OverlapTolerancePixels = 1.0
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force
Add-Type -AssemblyName System.Windows.Forms

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$evidencePath = Join-Path $outputDirectory "ui-regressions-$stamp.json"
$stderrPath = Join-Path $outputDirectory "ui-regressions-$stamp.stderr.log"
$candidateFile = Get-Item -LiteralPath $resolvedExecutable
$candidateMetadata = [ordered]@{
    path = $resolvedExecutable
    sha256 = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    sizeBytes = [long]$candidateFile.Length
    lastWriteTimeUtc = $candidateFile.LastWriteTimeUtc.ToString("o")
}
$probePath = Join-Path $env:TEMP (
    "wfdiag-reactor-ui-regressions-version-{0}.json" -f [Guid]::NewGuid().ToString("N"))
$version = Get-ReactorApplicationVersion -Executable $resolvedExecutable -ProbeFile $probePath
Remove-Item -LiteralPath $probePath -Force -ErrorAction SilentlyContinue
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    candidate = $candidateMetadata
    applicationVersion = $version
    suite = "ui-regressions"
    overlapTolerancePixels = $OverlapTolerancePixels
    commandPalette = [ordered]@{}
    keyboardShortcuts = [ordered]@{}
    networkConnections = [ordered]@{}
    gracefulClose = $null
    crashEvents = @()
    stderr = [ordered]@{
        path = $stderrPath
        sizeBytes = $null
        tail = $null
    }
    failures = $failures
}
$script:ReactorUiRegressionShell = New-Object -ComObject WScript.Shell
$script:ReactorUiRegressionProcess = $null
$script:ReactorUiRegressionExitReported = $false

function Get-CandidateStderrTail {
    if (-not (Test-Path -LiteralPath $stderrPath -PathType Leaf)) {
        return "<stderr file was not created>"
    }
    try {
        $text = (@(Get-Content -LiteralPath $stderrPath -Tail 40 -ErrorAction Stop) -join
            [Environment]::NewLine).Trim()
        if ([string]::IsNullOrWhiteSpace($text)) {
            return "<stderr was empty>"
        }
        if ($text.Length -gt 4096) {
            return $text.Substring($text.Length - 4096)
        }
        return $text
    }
    catch {
        return "<stderr could not be read: $($_.Exception.Message)>"
    }
}

function Get-CandidateUnexpectedExitMessage {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    $Process.Refresh()
    $exitCode = if ($Process.HasExited) { $Process.ExitCode } else { "<still running>" }
    $stderrTail = Get-CandidateStderrTail
    return "Candidate PID $($Process.Id) exited unexpectedly with code $exitCode. " +
        "stderr tail: $stderrTail (full stderr: $stderrPath)"
}

function Assert-CandidateRunning {
    if ($null -eq $script:ReactorUiRegressionProcess) {
        return
    }
    $script:ReactorUiRegressionProcess.Refresh()
    if ($script:ReactorUiRegressionProcess.HasExited) {
        $script:ReactorUiRegressionExitReported = $true
        throw (Get-CandidateUnexpectedExitMessage `
            -Process $script:ReactorUiRegressionProcess)
    }
}

function Get-UiaElementCandidates {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [string]$Name,
        [string]$AutomationId,
        [string]$AutomationIdPrefix,
        [Windows.Automation.ControlType]$ControlType,
        [switch]$AllowOffscreen
    )

    Assert-CandidateRunning

    $selectorCount = @(
        $PSBoundParameters.ContainsKey("Name"),
        $PSBoundParameters.ContainsKey("AutomationId"),
        $PSBoundParameters.ContainsKey("AutomationIdPrefix")
    ).Where({ $_ }).Count
    if ($selectorCount -ne 1) {
        throw "Specify exactly one UIA selector: Name, AutomationId, or AutomationIdPrefix."
    }

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $candidates = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            if ($PSBoundParameters.ContainsKey("Name") -and $current.Name -cne $Name) {
                continue
            }
            if ($PSBoundParameters.ContainsKey("AutomationId") -and
                $current.AutomationId -cne $AutomationId) {
                continue
            }
            if ($PSBoundParameters.ContainsKey("AutomationIdPrefix") -and
                -not $current.AutomationId.StartsWith(
                    $AutomationIdPrefix,
                    [StringComparison]::Ordinal)) {
                continue
            }
            if ($PSBoundParameters.ContainsKey("ControlType") -and
                $current.ControlType -ne $ControlType) {
                continue
            }
            if ($current.IsOffscreen -and -not $AllowOffscreen) {
                continue
            }
            $record = Get-UiaElementRecord -Element $element
            if ($record.unavailable) {
                continue
            }
            $candidates += [pscustomobject]@{
                element = $element
                record = $record
            }
        }
        catch {
            # The native tree can change between FindAll and Current. Polling
            # callers will take a fresh snapshot on the next iteration.
            continue
        }
    }
    return $candidates
}

function Wait-UniqueUiaElement {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [string]$Name,
        [string]$AutomationId,
        [Windows.Automation.ControlType]$ControlType,
        [switch]$AllowOffscreen
    )

    $lookup = @{ Root = $Root }
    $description = $null
    if ($PSBoundParameters.ContainsKey("Name")) {
        $lookup.Name = $Name
        $description = "Name='$Name'"
    }
    elseif ($PSBoundParameters.ContainsKey("AutomationId")) {
        $lookup.AutomationId = $AutomationId
        $description = "AutomationId='$AutomationId'"
    }
    else {
        throw "Wait-UniqueUiaElement requires Name or AutomationId."
    }
    if ($PSBoundParameters.ContainsKey("ControlType")) {
        $lookup.ControlType = $ControlType
    }
    if ($AllowOffscreen) {
        $lookup.AllowOffscreen = $true
    }

    $lastCandidates = @()
    do {
        $lastCandidates = @(Get-UiaElementCandidates @lookup)
        if ($lastCandidates.Count -eq 1) {
            return $lastCandidates[0]
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)

    $records = @($lastCandidates | ForEach-Object { $_.record })
    throw "Expected one visible UIA element with $description; found $($records.Count): $($records | ConvertTo-Json -Compress -Depth 5)"
}

function Wait-UiaElementAbsent {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [string]$Name,
        [string]$AutomationId,
        [Windows.Automation.ControlType]$ControlType
    )

    $selectorCount = @(
        $PSBoundParameters.ContainsKey("Name"),
        $PSBoundParameters.ContainsKey("AutomationId")
    ).Where({ $_ }).Count
    if ($selectorCount -ne 1) {
        throw "Wait-UiaElementAbsent requires exactly one of Name or AutomationId."
    }
    $selector = if ($PSBoundParameters.ContainsKey("Name")) {
        @{ Name = $Name }
    }
    else {
        @{ AutomationId = $AutomationId }
    }
    $description = if ($PSBoundParameters.ContainsKey("Name")) {
        "Name='$Name'"
    }
    else {
        "AutomationId='$AutomationId'"
    }
    do {
        $lookup = @{ Root = $Root }
        foreach ($entry in $selector.GetEnumerator()) {
            $lookup[$entry.Key] = $entry.Value
        }
        if ($PSBoundParameters.ContainsKey("ControlType")) {
            $lookup.ControlType = $ControlType
        }
        if (@(Get-UiaElementCandidates @lookup).Count -eq 0) {
            return $true
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)
    throw "UIA element $description remained visible after its close deadline."
}

function Test-UiaRuntimeIdentity {
    param($Expected, $Actual)

    if ($null -eq $Expected -or $null -eq $Actual -or
        $Expected.unavailable -or $Actual.unavailable) {
        return $false
    }
    $expectedId = @($Expected.runtimeId)
    $actualId = @($Actual.runtimeId)
    if ($expectedId.Count -eq 0 -or $expectedId.Count -ne $actualId.Count) {
        return $false
    }
    for ($index = 0; $index -lt $expectedId.Count; $index++) {
        if ($expectedId[$index] -ne $actualId[$index]) {
            return $false
        }
    }
    return $true
}

function Get-UiaFocusedRecord {
    Assert-CandidateRunning
    try {
        return Get-UiaElementRecord -Element (
            [Windows.Automation.AutomationElement]::FocusedElement)
    }
    catch {
        return [pscustomobject]@{
            unavailable = $true
            error = $_.Exception.Message
            runtimeId = @()
        }
    }
}

function Wait-UiaFocusRecord {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $last = $null
    do {
        $last = Get-UiaFocusedRecord
        if (Test-UiaRuntimeIdentity -Expected $Expected -Actual $last) {
            return [pscustomobject]@{
                passed = $true
                expected = $Expected
                observed = $last
            }
        }
        Start-Sleep -Milliseconds 50
    } while ((Get-Date) -lt $Deadline)
    return [pscustomobject]@{
        passed = $false
        expected = $Expected
        observed = $last
    }
}

function Measure-UiaFocusStability {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][int]$DurationMilliseconds,
        [ValidateRange(10, 500)][int]$SampleIntervalMilliseconds = 25
    )

    $deadline = (Get-Date).AddMilliseconds($DurationMilliseconds)
    $sampleCount = 0
    $mismatchCount = 0
    $mismatches = @()
    do {
        $sampleCount++
        $focused = Get-UiaFocusedRecord
        if (-not (Test-UiaRuntimeIdentity -Expected $Expected -Actual $focused)) {
            $mismatchCount++
            if ($mismatches.Count -lt 20) {
                $mismatches += [pscustomobject]@{
                    capturedAtUtc = [DateTime]::UtcNow.ToString("o")
                    focused = $focused
                }
            }
        }
        Start-Sleep -Milliseconds $SampleIntervalMilliseconds
    } while ((Get-Date) -lt $deadline)

    return [pscustomobject]@{
        passed = $mismatchCount -eq 0
        sampleCount = $sampleCount
        mismatchCount = $mismatchCount
        mismatches = $mismatches
    }
}

function Send-ReactorKeys {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$Keys,
        [ValidateRange(0, 3000)][int]$WaitMilliseconds = 150
    )

    Assert-CandidateRunning
    if (-not $script:ReactorUiRegressionShell.AppActivate([int]$Process.Id)) {
        throw "Could not activate candidate PID $($Process.Id) for keyboard input."
    }
    Start-Sleep -Milliseconds 75
    [Windows.Forms.SendKeys]::SendWait($Keys)
    if ($WaitMilliseconds -gt 0) {
        Start-Sleep -Milliseconds $WaitMilliseconds
    }
}

function Get-UiaValue {
    param([Parameter(Mandatory = $true)]$Element)

    Assert-CandidateRunning
    $pattern = [Windows.Automation.ValuePattern]$Element.GetCurrentPattern(
        [Windows.Automation.ValuePattern]::Pattern)
    return [string]$pattern.Current.Value
}

function Set-UiaValue {
    param(
        [Parameter(Mandatory = $true)]$Element,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value
    )

    Assert-CandidateRunning
    $pattern = [Windows.Automation.ValuePattern]$Element.GetCurrentPattern(
        [Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($Value)
}

function Wait-UiaValue {
    param(
        [Parameter(Mandatory = $true)]$Element,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Expected,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $last = $null
    do {
        Assert-CandidateRunning
        try {
            $last = Get-UiaValue -Element $Element
            if ($last -ceq $Expected) {
                return $last
            }
        }
        catch {
            $last = "<unavailable: $($_.Exception.Message)>"
        }
        Start-Sleep -Milliseconds 50
    } while ((Get-Date) -lt $Deadline)
    throw "UIA value did not become '$Expected'; last value was '$last'."
}

function Test-UiaBounds {
    param($Bounds)

    if ($null -eq $Bounds) {
        return [pscustomobject]@{ valid = $false; reason = "missing bounds" }
    }
    try {
        $x = [double]$Bounds.x
        $y = [double]$Bounds.y
        $width = [double]$Bounds.width
        $height = [double]$Bounds.height
        $values = @($x, $y, $width, $height)
        if (@($values | Where-Object {
            [double]::IsNaN($_) -or [double]::IsInfinity($_)
        }).Count -gt 0) {
            return [pscustomobject]@{ valid = $false; reason = "non-finite bounds" }
        }
        if ($width -le 0 -or $height -le 0) {
            return [pscustomobject]@{
                valid = $false
                reason = "non-positive size $width x $height"
            }
        }
        return [pscustomobject]@{
            valid = $true
            reason = $null
            x = $x
            y = $y
            width = $width
            height = $height
            right = $x + $width
            bottom = $y + $height
        }
    }
    catch {
        return [pscustomobject]@{
            valid = $false
            reason = "unreadable bounds: $($_.Exception.Message)"
        }
    }
}

function Measure-UiaNonOverlappingLayout {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]]$Items,
        [Parameter(Mandatory = $true)][double]$TolerancePixels
    )

    $geometry = @()
    $invalid = @()
    foreach ($item in @($Items)) {
        $checked = Test-UiaBounds -Bounds $item.bounds
        if (-not $checked.valid) {
            $invalid += [pscustomobject]@{
                identity = [string]$item.identity
                reason = $checked.reason
                bounds = $item.bounds
            }
            continue
        }
        $geometry += [pscustomobject]@{
            identity = [string]$item.identity
            bounds = $item.bounds
            x = $checked.x
            y = $checked.y
            width = $checked.width
            height = $checked.height
            right = $checked.right
            bottom = $checked.bottom
        }
    }

    $overlapCount = 0
    $overlaps = @()
    for ($firstIndex = 0; $firstIndex -lt $geometry.Count; $firstIndex++) {
        $first = $geometry[$firstIndex]
        for ($secondIndex = $firstIndex + 1;
            $secondIndex -lt $geometry.Count;
            $secondIndex++) {
            $second = $geometry[$secondIndex]
            $vertical = [Math]::Min($first.bottom, $second.bottom) -
                [Math]::Max($first.y, $second.y)
            $horizontal = [Math]::Min($first.right, $second.right) -
                [Math]::Max($first.x, $second.x)
            if ($vertical -le $TolerancePixels -or $horizontal -le $TolerancePixels) {
                continue
            }
            $overlapCount++
            if ($overlaps.Count -lt 30) {
                $overlaps += [pscustomobject]@{
                    firstIdentity = $first.identity
                    secondIdentity = $second.identity
                    horizontalOverlapPixels = [Math]::Round($horizontal, 2)
                    verticalOverlapPixels = [Math]::Round($vertical, 2)
                    firstBounds = $first.bounds
                    secondBounds = $second.bounds
                }
            }
        }
    }

    return [pscustomobject]@{
        itemCount = $Items.Count
        validItemCount = $geometry.Count
        invalidCount = $invalid.Count
        invalid = @($invalid | Select-Object -First 30)
        overlapCount = $overlapCount
        overlaps = $overlaps
    }
}

function Register-UiaLayoutResult {
    param(
        [Parameter(Mandatory = $true)][string]$Surface,
        [Parameter(Mandatory = $true)]$Measurement
    )

    if ($Measurement.invalidCount -gt 0) {
        $first = $Measurement.invalid[0]
        $failures.Add(
            "$Surface exposed invalid geometry for '$($first.identity)': $($first.reason).")
    }
    if ($Measurement.overlapCount -gt 0) {
        $first = $Measurement.overlaps[0]
        $failures.Add(
            "$Surface overlaps '$($first.firstIdentity)' and '$($first.secondIdentity)' by $($first.horizontalOverlapPixels) x $($first.verticalOverlapPixels) px.")
    }
}

function New-UiaLayoutItem {
    param(
        [Parameter(Mandatory = $true)][string]$Identity,
        [Parameter(Mandatory = $true)]$Record
    )
    return [pscustomobject]@{
        identity = $Identity
        bounds = $Record.bounds
    }
}

function Get-UiaBoundsUnion {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateCount(1, 100)]
        [object[]]$Records
    )

    $checked = @($Records | ForEach-Object { Test-UiaBounds -Bounds $_.bounds })
    if (@($checked | Where-Object { -not $_.valid }).Count -gt 0) {
        throw "Cannot construct a row union from invalid UIA bounds."
    }
    $left = ($checked | Measure-Object -Property x -Minimum).Minimum
    $top = ($checked | Measure-Object -Property y -Minimum).Minimum
    $right = ($checked | Measure-Object -Property right -Maximum).Maximum
    $bottom = ($checked | Measure-Object -Property bottom -Maximum).Maximum
    return [pscustomobject]@{
        x = $left
        y = $top
        width = $right - $left
        height = $bottom - $top
    }
}

function Get-PaletteItems {
    param([Parameter(Mandatory = $true)]$Root)
    return @(Get-UiaElementCandidates -Root $Root -AutomationIdPrefix "palette-item-" `
        -ControlType ([Windows.Automation.ControlType]::Button) -AllowOffscreen)
}

function Wait-PaletteItems {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string[]]$RequiredAutomationIds,
        [Parameter(Mandatory = $true)][datetime]$Deadline,
        [int]$ExpectedCount = -1
    )

    $last = @()
    do {
        $last = @(Get-PaletteItems -Root $Root)
        $ids = @($last | ForEach-Object { $_.record.automationId })
        $hasRequired = @($RequiredAutomationIds | Where-Object { $_ -notin $ids }).Count -eq 0
        $countMatches = $ExpectedCount -lt 0 -or $last.Count -eq $ExpectedCount
        if ($hasRequired -and $countMatches) {
            return $last
        }
        Start-Sleep -Milliseconds 75
    } while ((Get-Date) -lt $Deadline)
    throw "Palette result set did not stabilize. Required=$($RequiredAutomationIds -join ', '), expectedCount=$ExpectedCount, observed=$(@($last | ForEach-Object { $_.record.automationId }) -join ', ')."
}

function Get-PaletteLayoutMeasurement {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][object[]]$Items
    )

    $layoutItems = @($Items | Where-Object { -not $_.record.isOffscreen } |
        ForEach-Object {
            New-UiaLayoutItem -Identity $_.record.automationId -Record $_.record
        })
    foreach ($section in @("NAVIGATE", "SCAN", "REPORT", "APP", "DIAGNOSTICS")) {
        $headings = @(Get-UiaElementCandidates -Root $Root -Name $section `
            -ControlType ([Windows.Automation.ControlType]::Text))
        foreach ($heading in $headings) {
            $layoutItems += New-UiaLayoutItem -Identity "section:$section" -Record $heading.record
        }
    }
    return Measure-UiaNonOverlappingLayout -Items $layoutItems `
        -TolerancePixels $OverlapTolerancePixels
}

function Get-VisibleTextDescendants {
    param([Parameter(Mandatory = $true)]$Root)

    Assert-CandidateRunning
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty,
        [Windows.Automation.ControlType]::Text)
    $elements = $Root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
    $records = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $record = Get-UiaElementRecord -Element $elements.Item($index)
        if ($null -ne $record -and -not $record.unavailable -and
            -not $record.isOffscreen -and
            -not [string]::IsNullOrWhiteSpace([string]$record.name)) {
            $records += $record
        }
    }
    return $records
}

function Invoke-CommandPaletteRegression {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    $defaultIds = @(
        "palette-item-diagnostics",
        "palette-item-monitor",
        "palette-item-processes",
        "palette-item-ai",
        "palette-item-issues",
        "palette-item-history",
        "palette-item-quick-scan",
        "palette-item-full-scan"
    )
    try {
        $preFocus = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Open Settings" -ControlType ([Windows.Automation.ControlType]::Button)
        $preFocus.element.SetFocus()
        $focused = Wait-UiaFocusRecord -Expected $preFocus.record `
            -Deadline (Get-Date).AddSeconds(3)
        $evidence.commandPalette.preOpenFocus = $focused
        if (-not $focused.passed) {
            $failures.Add("Command Palette precondition could not focus the Open Settings button.")
        }

        Send-ReactorKeys -Process $Process -Keys "^k"
        $query = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Search commands" -ControlType ([Windows.Automation.ControlType]::Edit)
        $openFocus = Wait-UiaFocusRecord -Expected $query.record `
            -Deadline (Get-Date).AddSeconds(3)
        $evidence.commandPalette.openFocus = $openFocus
        if (-not $openFocus.passed) {
            $failures.Add("Command Palette did not move focus into Search commands on open.")
        }

        $defaultItems = Wait-PaletteItems -Root $Root -RequiredAutomationIds $defaultIds `
            -Deadline (Get-Date).AddSeconds(8)
        if ($defaultItems.Count -notin @(8, 9)) {
            $failures.Add(
                "Command Palette default list exposed $($defaultItems.Count) rows; expected 8 idle rows or 9 while a scan is active.")
        }
        $defaultLayout = Get-PaletteLayoutMeasurement -Root $Root -Items $defaultItems
        $evidence.commandPalette.defaultResults = [ordered]@{
            itemCount = $defaultItems.Count
            automationIds = @($defaultItems | ForEach-Object { $_.record.automationId })
            visibleItems = @($defaultItems | Where-Object { -not $_.record.isOffscreen } |
                ForEach-Object { $_.record })
            layout = $defaultLayout
        }
        Register-UiaLayoutResult -Surface "Command Palette default list" `
            -Measurement $defaultLayout

        Send-ReactorKeys -Process $Process -Keys "{ESC}"
        Wait-UiaElementAbsent -Root $Root -Deadline (Get-Date).AddSeconds(5) `
            -Name "Search commands" -ControlType ([Windows.Automation.ControlType]::Edit) | Out-Null
        $restored = Wait-UiaFocusRecord -Expected $preFocus.record `
            -Deadline (Get-Date).AddSeconds(3)
        $evidence.commandPalette.restoredFocus = $restored
        if (-not $restored.passed) {
            $failures.Add("Command Palette did not restore the exact pre-open UIA focus target.")
        }

        Send-ReactorKeys -Process $Process -Keys "^k"
        $query = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Search commands" -ControlType ([Windows.Automation.ControlType]::Edit)
        $queryFocus = Wait-UiaFocusRecord -Expected $query.record `
            -Deadline (Get-Date).AddSeconds(3)
        if (-not $queryFocus.passed) {
            $failures.Add("Command Palette search did not regain focus on its second open.")
        }
        Send-ReactorKeys -Process $Process -Keys "ops" -WaitMilliseconds 50
        Wait-UiaValue -Element $query.element -Expected "ops" `
            -Deadline (Get-Date).AddSeconds(5) | Out-Null

        $fuzzyItems = Wait-PaletteItems -Root $Root -RequiredAutomationIds @(
            "palette-item-processes", "palette-item-settings") `
            -Deadline (Get-Date).AddSeconds(8)
        if ($fuzzyItems.Count -gt 14) {
            $failures.Add("Command Palette fuzzy search returned $($fuzzyItems.Count) rows; maximum is 14.")
        }
        $fuzzyLayout = Get-PaletteLayoutMeasurement -Root $Root -Items $fuzzyItems
        Register-UiaLayoutResult -Surface "Command Palette fuzzy list" `
            -Measurement $fuzzyLayout
        $postTypeFocus = Measure-UiaFocusStability -Expected $query.record `
            -DurationMilliseconds $FocusObservationMilliseconds
        if (-not $postTypeFocus.passed) {
            $failures.Add("Command Palette search focus left the query after typing 'ops'.")
        }

        $settingsItem = @($fuzzyItems | Where-Object {
            $_.record.automationId -ceq "palette-item-settings"
        })
        if ($settingsItem.Count -ne 1) {
            throw "Fuzzy query 'ops' did not expose exactly one Open Settings result."
        }
        if ($settingsItem[0].record.isOffscreen) {
            try {
                $scrollItem = [Windows.Automation.ScrollItemPattern]$settingsItem[0].element.GetCurrentPattern(
                    [Windows.Automation.ScrollItemPattern]::Pattern)
                $scrollItem.ScrollIntoView()
                Start-Sleep -Milliseconds 200
            }
            catch {
                throw "Open Settings fuzzy result was offscreen and could not be revealed: $($_.Exception.Message)"
            }
            $settingsItem = @(Get-UiaElementCandidates -Root $Root `
                -AutomationId "palette-item-settings" `
                -ControlType ([Windows.Automation.ControlType]::Button))
        }
        if ($settingsItem.Count -ne 1) {
            throw "Open Settings fuzzy result was not visibly realized."
        }
        $rowText = @(Get-VisibleTextDescendants -Root $settingsItem[0].element)
        $labelCandidates = @($rowText | Where-Object { $_.name -ceq "Open Settings" })
        if ($labelCandidates.Count -ne 1) {
            $failures.Add(
                "Command Palette Open Settings result exposed $($labelCandidates.Count) exact visible label nodes; expected one ellipsized label node.")
        }
        $rowTextLayout = Measure-UiaNonOverlappingLayout -Items @(
            $rowText | ForEach-Object {
                New-UiaLayoutItem -Identity "row-text:$($_.name)" -Record $_
            }) -TolerancePixels $OverlapTolerancePixels
        Register-UiaLayoutResult -Surface "Command Palette Open Settings row text" `
            -Measurement $rowTextLayout

        $labelContainedByResult = $false
        if ($labelCandidates.Count -eq 1) {
            $labelBounds = Test-UiaBounds -Bounds $labelCandidates[0].bounds
            $resultBounds = Test-UiaBounds -Bounds $settingsItem[0].record.bounds
            if ($labelBounds.valid -and $resultBounds.valid) {
                $labelContainedByResult =
                    $labelBounds.x -ge ($resultBounds.x - $OverlapTolerancePixels) -and
                    $labelBounds.y -ge ($resultBounds.y - $OverlapTolerancePixels) -and
                    $labelBounds.right -le ($resultBounds.right + $OverlapTolerancePixels) -and
                    $labelBounds.bottom -le ($resultBounds.bottom + $OverlapTolerancePixels)
            }
            if (-not $labelContainedByResult) {
                $failures.Add(
                    "Command Palette Open Settings label is not contained within its result button bounds.")
            }
        }
        $evidence.commandPalette.fuzzySearch = [ordered]@{
            query = "ops"
            itemCount = $fuzzyItems.Count
            automationIds = @($fuzzyItems | ForEach-Object { $_.record.automationId })
            names = @($fuzzyItems | ForEach-Object { $_.record.name })
            visibleItems = @($fuzzyItems | Where-Object { -not $_.record.isOffscreen } |
                ForEach-Object { $_.record })
            resultLayout = $fuzzyLayout
            rowText = $rowText
            exactLabelCount = $labelCandidates.Count
            exactLabel = if ($labelCandidates.Count -eq 1) { $labelCandidates[0] } else { $null }
            rowTextLayout = $rowTextLayout
            labelContainedByResult = $labelContainedByResult
            postTypeFocus = $postTypeFocus
        }

        Send-ReactorKeys -Process $Process -Keys "{DOWN}" -WaitMilliseconds 0
        $postArrowFocus = Measure-UiaFocusStability -Expected $query.record `
            -DurationMilliseconds $FocusObservationMilliseconds
        $evidence.commandPalette.postArrowFocus = $postArrowFocus
        if (-not $postArrowFocus.passed) {
            $failures.Add("Command Palette arrow navigation did not retain Search commands focus.")
        }

        Set-UiaValue -Element $query.element -Value ""
        Wait-UiaValue -Element $query.element -Expected "" `
            -Deadline (Get-Date).AddSeconds(3) | Out-Null
        Wait-PaletteItems -Root $Root -RequiredAutomationIds $defaultIds `
            -Deadline (Get-Date).AddSeconds(5) | Out-Null

        # Send one compound sequence so the native FIFO must preserve Down
        # before Enter. From the default list this executes Live Monitor.
        Send-ReactorKeys -Process $Process -Keys "{DOWN}{ENTER}" -WaitMilliseconds 100
        Wait-UiaElementAbsent -Root $Root -Deadline (Get-Date).AddSeconds(6) `
            -Name "Search commands" -ControlType ([Windows.Automation.ControlType]::Edit) | Out-Null
        $monitorHeading = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Live Monitor" -ControlType ([Windows.Automation.ControlType]::Text)
        $evidence.commandPalette.rapidDownEnter = [ordered]@{
            passed = $true
            expectedDestination = "Live Monitor"
            observedHeading = $monitorHeading.record
        }
    }
    finally {
        $openQueries = @(Get-UiaElementCandidates -Root $Root -Name "Search commands" `
            -ControlType ([Windows.Automation.ControlType]::Edit))
        if ($openQueries.Count -gt 0) {
            try {
                Send-ReactorKeys -Process $Process -Keys "{ESC}"
                Wait-UiaElementAbsent -Root $Root -Deadline (Get-Date).AddSeconds(3) `
                    -Name "Search commands" `
                    -ControlType ([Windows.Automation.ControlType]::Edit) | Out-Null
            }
            catch {
                $failures.Add("Command Palette cleanup failed: $($_.Exception.Message)")
            }
        }
    }
}

function Get-UiaAncestorByName {
    param(
        [Parameter(Mandatory = $true)]$Element,
        [Parameter(Mandatory = $true)][string]$Name,
        [ValidateRange(1, 30)][int]$MaximumDepth = 20
    )

    Assert-CandidateRunning
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    $current = $Element
    for ($depth = 0; $depth -lt $MaximumDepth; $depth++) {
        try {
            if ($current.Current.Name -ceq $Name) {
                return $current
            }
            $current = $walker.GetParent($current)
            if ($null -eq $current) {
                break
            }
        }
        catch {
            break
        }
    }
    throw "Could not find UIA ancestor Name='$Name' within $MaximumDepth levels."
}

function Invoke-KeyboardShortcutsRegression {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    $pairs = @(
        @("Open the command palette", "Ctrl+K"),
        # Windows PowerShell 5.1 reads UTF-8-without-BOM scripts through the
        # active ANSI code page. Construct the ellipsis so the UIA expectation
        # matches the UTF-8 Rust label instead of a mojibaited three-byte string.
        @("Switch between screens", ("Ctrl+1 {0} Ctrl+6" -f [char]0x2026)),
        @("Run a Quick Scan", "Ctrl+Shift+Q"),
        @("Run a Full Scan", "Ctrl+Shift+F"),
        @("Refresh", "Ctrl+R"),
        @("Show this shortcut list", "Ctrl+/"),
        @("Close dialogs and overlays", "Esc")
    )
    $closeButton = $null
    try {
        $openButton = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Keyboard shortcuts" `
            -ControlType ([Windows.Automation.ControlType]::Button)
        Invoke-UiaButtonElement -Element $openButton.element
        $closeButton = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -AutomationId "CloseButton" `
            -ControlType ([Windows.Automation.ControlType]::Button)
        $dialogRoot = Get-UiaAncestorByName -Element $closeButton.element `
            -Name "Keyboard Shortcuts"
        $evidence.keyboardShortcuts.dialog = Get-UiaElementRecord -Element $dialogRoot

        $rowEvidence = @()
        $rowLayoutItems = @()
        foreach ($pair in $pairs) {
            $description = Wait-UniqueUiaElement -Root $dialogRoot `
                -Deadline (Get-Date).AddSeconds(5) -Name $pair[0] `
                -ControlType ([Windows.Automation.ControlType]::Text)
            $chord = Wait-UniqueUiaElement -Root $dialogRoot `
                -Deadline (Get-Date).AddSeconds(5) -Name $pair[1] `
                -ControlType ([Windows.Automation.ControlType]::Text)
            $cellLayout = Measure-UiaNonOverlappingLayout -Items @(
                (New-UiaLayoutItem -Identity "description:$($pair[0])" -Record $description.record),
                (New-UiaLayoutItem -Identity "chord:$($pair[1])" -Record $chord.record)
            ) -TolerancePixels $OverlapTolerancePixels
            Register-UiaLayoutResult -Surface "Keyboard Shortcuts row '$($pair[1])'" `
                -Measurement $cellLayout

            $descriptionBounds = Test-UiaBounds -Bounds $description.record.bounds
            $chordBounds = Test-UiaBounds -Bounds $chord.record.bounds
            $verticalOverlap = 0.0
            $ordered = $false
            if ($descriptionBounds.valid -and $chordBounds.valid) {
                $verticalOverlap = [Math]::Min(
                    $descriptionBounds.bottom,
                    $chordBounds.bottom) - [Math]::Max(
                        $descriptionBounds.y,
                        $chordBounds.y)
                $ordered = $descriptionBounds.x -lt $chordBounds.x
                if ($verticalOverlap -le $OverlapTolerancePixels) {
                    $failures.Add(
                        "Keyboard Shortcuts pair '$($pair[0])' / '$($pair[1])' is not aligned on one row.")
                }
                if (-not $ordered) {
                    $failures.Add(
                        "Keyboard Shortcuts chord '$($pair[1])' is not positioned to the right of its description.")
                }
                $rowBounds = Get-UiaBoundsUnion -Records @(
                    $description.record, $chord.record)
                $rowLayoutItems += [pscustomobject]@{
                    identity = "shortcut-row:$($pair[1])"
                    bounds = $rowBounds
                }
            }
            $rowEvidence += [pscustomobject]@{
                description = $description.record
                chord = $chord.record
                cellLayout = $cellLayout
                verticalOverlapPixels = [Math]::Round($verticalOverlap, 2)
                chordIsRightOfDescription = $ordered
            }
        }

        $rowsLayout = Measure-UiaNonOverlappingLayout -Items $rowLayoutItems `
            -TolerancePixels $OverlapTolerancePixels
        Register-UiaLayoutResult -Surface "Keyboard Shortcuts rows" -Measurement $rowsLayout
        $evidence.keyboardShortcuts.rows = $rowEvidence
        $evidence.keyboardShortcuts.rowsLayout = $rowsLayout
    }
    finally {
        if ($null -ne $closeButton) {
            try {
                Invoke-UiaButtonElement -Element $closeButton.element
                Wait-UiaElementAbsent -Root $Root -Deadline (Get-Date).AddSeconds(5) `
                    -AutomationId "CloseButton" `
                    -ControlType ([Windows.Automation.ControlType]::Button) | Out-Null
            }
            catch {
                $cleanupError = $_.Exception.Message
                try { Send-ReactorKeys -Process $Process -Keys "{ESC}" } catch {}
                $failures.Add("Keyboard Shortcuts cleanup failed: $cleanupError")
            }
        }
    }
}

function Get-NetworkProtocolCandidates {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$HeaderRecord
    )

    Assert-CandidateRunning
    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $protocols = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            if ($current.ControlType -ne [Windows.Automation.ControlType]::Text -or
                $current.IsOffscreen -or
                [string]$current.Name -notmatch '^(TCP|UDP)(v6)?$') {
                continue
            }
            $record = Get-UiaElementRecord -Element $element
            if (-not $record.unavailable -and
                $record.bounds.y -gt $HeaderRecord.bounds.y) {
                $protocols += [pscustomobject]@{
                    element = $element
                    record = $record
                }
            }
        }
        catch {
            continue
        }
    }
    return @($protocols | Sort-Object { $_.record.bounds.y }, { $_.record.bounds.x })
}

function Wait-NetworkProtocols {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)]$HeaderRecord,
        [Parameter(Mandatory = $true)][int]$MinimumRows,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $last = @()
    do {
        $last = @(Get-NetworkProtocolCandidates -Root $Root -HeaderRecord $HeaderRecord)
        if ($last.Count -ge $MinimumRows) {
            return $last
        }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $Deadline)
    throw "Network Connections exposed $($last.Count) visible protocol rows; expected at least $MinimumRows."
}

function Get-NetworkRowCells {
    param(
        [Parameter(Mandatory = $true)]$AllTextRecords,
        [Parameter(Mandatory = $true)]$ProtocolRecord
    )

    $protocolBounds = Test-UiaBounds -Bounds $ProtocolRecord.bounds
    if (-not $protocolBounds.valid) {
        return @($ProtocolRecord)
    }
    return @($AllTextRecords | Where-Object {
        $candidate = Test-UiaBounds -Bounds $_.bounds
        if (-not $candidate.valid) {
            return $false
        }
        $vertical = [Math]::Min($protocolBounds.bottom, $candidate.bottom) -
            [Math]::Max($protocolBounds.y, $candidate.y)
        $required = [Math]::Min($protocolBounds.height, $candidate.height) * 0.5
        # The protocol is the first data column, so its left edge is also the
        # durable left boundary of this table row. Text in the navigation rail
        # can share the same y-band (for example, About) but must never be
        # counted as a connection cell.
        $insideDataRegion = $candidate.x -ge
            ($protocolBounds.x - $OverlapTolerancePixels)
        return $insideDataRegion -and $vertical -ge $required
    } | Sort-Object { $_.bounds.x })
}

function Invoke-NetworkConnectionsRegression {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    $monitorNav = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
        -Name "Live Monitor" -ControlType ([Windows.Automation.ControlType]::Button)
    Invoke-UiaButtonElement -Element $monitorNav.element
    $header = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(10) `
        -Name "NETWORK CONNECTIONS" `
        -ControlType ([Windows.Automation.ControlType]::Text)

    $loadButtons = @(Get-UiaElementCandidates -Root $Root -Name "Load" `
        -ControlType ([Windows.Automation.ControlType]::Button))
    $loadMethod = $null
    $loadRecord = $null
    if ($loadButtons.Count -eq 1) {
        Invoke-UiaButtonElement -Element $loadButtons[0].element
        $loadMethod = "InvokePattern"
        $loadRecord = $loadButtons[0].record
    }
    else {
        $loadText = Wait-UniqueUiaElement -Root $Root -Deadline (Get-Date).AddSeconds(8) `
            -Name "Load" -ControlType ([Windows.Automation.ControlType]::Text)
        if (-not $script:ReactorUiRegressionShell.AppActivate([int]$Process.Id)) {
            throw "Could not activate the candidate before clicking Network Connections Load."
        }
        Start-Sleep -Milliseconds 75
        Invoke-UiaElementByMouseClick -Record $loadText.record
        $loadMethod = "physical-click-on-text"
        $loadRecord = $loadText.record
    }
    $evidence.networkConnections.load = [ordered]@{
        method = $loadMethod
        control = $loadRecord
    }

    $protocols = Wait-NetworkProtocols -Root $Root -HeaderRecord $header.record `
        -MinimumRows $MinimumNetworkRows `
        -Deadline (Get-Date).AddSeconds($NetworkWaitSeconds)
    $allTexts = @(Get-VisibleTextDescendants -Root $Root | Where-Object {
        $_.bounds.y -gt $header.record.bounds.y
    })

    $rowEvidence = @()
    $rowLayoutItems = @()
    foreach ($protocol in $protocols) {
        $cells = @(Get-NetworkRowCells -AllTextRecords $allTexts `
            -ProtocolRecord $protocol.record)
        $identity = "$($protocol.record.name)@$($protocol.record.bounds.y)"
        $cellItems = @($cells | ForEach-Object {
            New-UiaLayoutItem -Identity "$($identity):$($_.name)" -Record $_
        })
        $cellLayout = Measure-UiaNonOverlappingLayout -Items $cellItems `
            -TolerancePixels $OverlapTolerancePixels
        Register-UiaLayoutResult -Surface "Network Connections row $identity" `
            -Measurement $cellLayout
        if ($cells.Count -ne 4) {
            $failures.Add(
                "Network Connections row $identity exposed $($cells.Count) vertically aligned text cells; expected exactly 4.")
        }

        $ordered = $true
        for ($index = 1; $index -lt $cells.Count; $index++) {
            if ([double]$cells[$index].bounds.x -le [double]$cells[$index - 1].bounds.x) {
                $ordered = $false
                break
            }
        }
        if (-not $ordered) {
            $failures.Add(
                "Network Connections row $identity does not have strictly increasing cell columns.")
        }
        if ($cells.Count -eq 4 -and $cellLayout.invalidCount -eq 0) {
            $rowLayoutItems += [pscustomobject]@{
                identity = "network-row:$identity"
                bounds = Get-UiaBoundsUnion -Records $cells
            }
        }
        $rowEvidence += [pscustomobject]@{
            identity = $identity
            cells = $cells
            cellLayout = $cellLayout
            columnsStrictlyIncrease = $ordered
        }
    }

    $rowsLayout = Measure-UiaNonOverlappingLayout -Items $rowLayoutItems `
        -TolerancePixels $OverlapTolerancePixels
    Register-UiaLayoutResult -Surface "Network Connections rows" -Measurement $rowsLayout
    $evidence.networkConnections.header = $header.record
    $evidence.networkConnections.visibleProtocolCount = $protocols.Count
    $evidence.networkConnections.rows = $rowEvidence
    $evidence.networkConnections.rowsLayout = $rowsLayout
}

$session = $null
try {
    $session = Start-ReactorCandidate -Executable $resolvedExecutable `
        -Seconds $StartupWaitSeconds -StderrFile $stderrPath -Variables @{
            WFDIAG_REACTOR_PAGE = "diagnostics"
            WFDIAG_REACTOR_WIDTH = "1440"
            WFDIAG_REACTOR_HEIGHT = "1000"
        }
    $process = $session.process
    $script:ReactorUiRegressionProcess = $process
    $process.Refresh()
    Assert-CandidateRunning
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    try {
        Invoke-CommandPaletteRegression -Root $root -Process $process
    }
    catch {
        $failures.Add("Command Palette regression flow failed: $($_.Exception.Message)")
    }
    Assert-CandidateRunning

    try {
        Invoke-KeyboardShortcutsRegression -Root $root -Process $process
    }
    catch {
        $failures.Add("Keyboard Shortcuts regression flow failed: $($_.Exception.Message)")
    }
    Assert-CandidateRunning

    try {
        Invoke-NetworkConnectionsRegression -Root $root -Process $process
    }
    catch {
        $failures.Add("Network Connections regression flow failed: $($_.Exception.Message)")
    }
    Assert-CandidateRunning
}
catch {
    $failureMessage = if ($null -eq $session) {
        "Candidate startup failed: $($_.Exception.Message)"
    }
    else {
        "Candidate UIA regression aborted: $($_.Exception.Message)"
    }
    if ($null -eq $session) {
        $failureMessage += ". stderr tail: $(Get-CandidateStderrTail) " +
            "(full stderr: $stderrPath)"
    }
    $failures.Add($failureMessage)
}
finally {
    if ($null -ne $session) {
        try {
            $session.process.Refresh()
            if ($session.process.HasExited -and
                -not $script:ReactorUiRegressionExitReported) {
                $script:ReactorUiRegressionExitReported = $true
                $failures.Add((Get-CandidateUnexpectedExitMessage `
                    -Process $session.process))
            }
            $close = Stop-ReactorCandidate -Session $session `
                -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
            $evidence.gracefulClose = $close.gracefulClose
            $evidence.crashEvents = $close.crashEvents
            if (-not $close.gracefulClose) {
                $failures.Add("Candidate did not close gracefully.")
            }
            if ($close.crashEvents.Count -gt 0) {
                $failures.Add("Crash events were recorded for the candidate.")
            }
        }
        catch {
            $failures.Add("Candidate cleanup failed: $($_.Exception.Message)")
        }
    }
}

if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
    $stderrFile = Get-Item -LiteralPath $stderrPath
    $evidence.stderr.sizeBytes = [long]$stderrFile.Length
}
$evidence.stderr.tail = Get-CandidateStderrTail

Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"
if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Command Palette, Keyboard Shortcuts, and Network Connections UI regressions passed."
exit 0
