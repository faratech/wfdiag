# Reactor rendering-parity target #1: the process list refresh triptych.
#
# The owner reported the process list refresh rendering as visibly wrong.
# This script captures the Reactor Processes screen at three moments and also
# samples its UIA process-row identities and bounding rectangles through
# repeated refreshes. It fails when the live list collapses, rows overlap or
# expose invalid geometry, or surviving rows are destroyed and recreated,
# turning the former capture-only repro into a regression gate.
# Combined sheets are produced against the existing Store baseline when present.
#
# Output: PNGs + triptych JSON under -OutputDirectory and a record appended
# to reactor-baselines/variants.json (validated by check-variants.py).

param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = "reactor-spike\captures-2.5.8\validation-process-refresh",
    [string]$VariantsJson = "reactor-baselines\variants.json",
    [string]$StoreBaselinePng = "reactor-baselines\captures\store-2.5.8\processes-populated-desktop-dark.png",
    [ValidateRange(1, 20)][int]$HoldSeconds = 2,
    [ValidateRange(1, 5)][int]$RefreshCycles = 3,
    [ValidateRange(50, 1000)][int]$SampleIntervalMilliseconds = 100,
    [ValidateRange(500, 10000)][int]$ObserveMilliseconds = 2500,
    [ValidateRange(50, 100)][int]$IdentityChurnPercent = 80,
    [ValidateRange(0, 10)][double]$LayoutOverlapTolerancePixels = 1.0
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "lib\ReactorUia.psm1") -Force

function Get-AbsolutePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $Path))
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
}
$outputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$version = Get-ReactorApplicationVersion -Executable $resolvedExecutable `
    -ProbeFile (Join-Path $env:TEMP "wfdiag-reactor-procparity-version.json")
if ($version -ne "2.5.8") {
    throw "Candidate version '$version' is not the pinned 2.5.8 oracle."
}

$failures = [System.Collections.Generic.List[string]]::new()
$evidence = [ordered]@{
    executable = $resolvedExecutable
    applicationVersion = $version
    suite = "process-refresh-parity"
    captures = @()
    refreshChecks = [ordered]@{
        refreshCycles = $RefreshCycles
        sampleIntervalMilliseconds = $SampleIntervalMilliseconds
        observeMilliseconds = $ObserveMilliseconds
        identityChurnPercent = $IdentityChurnPercent
        layoutOverlapTolerancePixels = $LayoutOverlapTolerancePixels
        baselineRowCount = 0
        collapseFloor = 0
        samples = @()
        collapseSamples = @()
        identityChanges = @()
        identityChurnSamples = @()
        invalidGeometrySamples = @()
        layoutOverlapSamples = @()
    }
    gracefulClose = $null
    crashEvents = @()
    failures = $failures
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$session = Start-ReactorCandidate -Executable $resolvedExecutable -Seconds 8 `
    -Variables @{
        WFDIAG_REACTOR_PAGE = "processes"
        WFDIAG_REACTOR_WIDTH = "1440"
        WFDIAG_REACTOR_HEIGHT = "1000"
    }

function Invoke-Capture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $path = Join-Path $outputDirectory "$Name.png"
    $null = & (Join-Path $PSScriptRoot "capture-window.ps1") `
        -ProcessId $session.process.Id `
        -OutputPath $path `
        -WaitSeconds 15
    $evidence.captures += $path
    Write-Host "Captured $Name -> $path"
    return $path
}

function Measure-ProcessRowLayout {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]]$Rows,
        [Parameter(Mandatory = $true)][double]$OverlapTolerancePixels
    )

    $geometryRows = @()
    $invalidRows = @()
    foreach ($row in @($Rows | Where-Object { -not $_.isOffscreen })) {
        $bounds = $row.bounds
        if ($null -eq $bounds) {
            $invalidRows += [pscustomobject]@{
                identity = $row.identity
                reason = "missing bounds"
                bounds = $null
            }
            continue
        }

        try {
            $x = [double]$bounds.x
            $y = [double]$bounds.y
            $width = [double]$bounds.width
            $height = [double]$bounds.height
            $values = @($x, $y, $width, $height)
            $nonFinite = @($values | Where-Object {
                [double]::IsNaN($_) -or [double]::IsInfinity($_)
            })
            if ($nonFinite.Count -gt 0 -or $width -le 0 -or $height -le 0) {
                $invalidRows += [pscustomobject]@{
                    identity = $row.identity
                    reason = "non-finite or non-positive bounds"
                    bounds = $bounds
                }
                continue
            }

            $geometryRows += [pscustomobject]@{
                identity = $row.identity
                x = $x
                y = $y
                width = $width
                height = $height
                right = $x + $width
                bottom = $y + $height
            }
        }
        catch {
            $invalidRows += [pscustomobject]@{
                identity = $row.identity
                reason = "unreadable bounds: $($_.Exception.Message)"
                bounds = $bounds
            }
        }
    }

    # UIA bounding rectangles use screen coordinates. Compare every pair that
    # can still intersect vertically so equal-Y/stacked rows cannot hide behind
    # an adjacent-only check. The horizontal intersection prevents a future
    # multi-column layout from being mistaken for overlap.
    $orderedRows = @($geometryRows | Sort-Object -Property `
        @{ Expression = { $_.y } }, @{ Expression = { $_.x } }, identity)
    $overlapCount = 0
    $overlaps = @()
    for ($firstIndex = 0; $firstIndex -lt $orderedRows.Count; $firstIndex++) {
        $first = $orderedRows[$firstIndex]
        for ($secondIndex = $firstIndex + 1;
            $secondIndex -lt $orderedRows.Count;
            $secondIndex++) {
            $second = $orderedRows[$secondIndex]
            if ($second.y -ge ($first.bottom - $OverlapTolerancePixels)) {
                break
            }

            $verticalOverlap = [Math]::Min($first.bottom, $second.bottom) -
                [Math]::Max($first.y, $second.y)
            $horizontalOverlap = [Math]::Min($first.right, $second.right) -
                [Math]::Max($first.x, $second.x)
            if ($verticalOverlap -le $OverlapTolerancePixels -or
                $horizontalOverlap -le $OverlapTolerancePixels) {
                continue
            }

            $overlapCount++
            if ($overlaps.Count -lt 20) {
                $overlaps += [pscustomobject]@{
                    firstIdentity = $first.identity
                    secondIdentity = $second.identity
                    verticalOverlapPixels = [Math]::Round($verticalOverlap, 2)
                    horizontalOverlapPixels = [Math]::Round($horizontalOverlap, 2)
                    firstBounds = [pscustomobject]@{
                        x = $first.x
                        y = $first.y
                        width = $first.width
                        height = $first.height
                    }
                    secondBounds = [pscustomobject]@{
                        x = $second.x
                        y = $second.y
                        width = $second.width
                        height = $second.height
                    }
                }
            }
        }
    }

    return [pscustomobject]@{
        visibleRowCount = @($Rows | Where-Object { -not $_.isOffscreen }).Count
        geometryRowCount = $geometryRows.Count
        invalidVisibleBoundsCount = $invalidRows.Count
        invalidVisibleBounds = @($invalidRows | Select-Object -First 20)
        overlapCount = $overlapCount
        overlaps = $overlaps
    }
}

function Register-ProcessRowLayoutResult {
    param([Parameter(Mandatory = $true)]$Snapshot)

    $layout = $Snapshot.layout
    if ($layout.invalidVisibleBoundsCount -gt 0) {
        $isFirstInvalidSample = $evidence.refreshChecks.invalidGeometrySamples.Count -eq 0
        $evidence.refreshChecks.invalidGeometrySamples += [pscustomobject]@{
            phase = $Snapshot.phase
            sampleIndex = $Snapshot.sampleIndex
            capturedAtUtc = $Snapshot.capturedAtUtc
            invalidVisibleBoundsCount = $layout.invalidVisibleBoundsCount
            rows = @($layout.invalidVisibleBounds)
        }
        if ($isFirstInvalidSample) {
            $first = $layout.invalidVisibleBounds[0]
            $failures.Add(
                "Process row '$($first.identity)' exposed invalid visible geometry during '$($Snapshot.phase)': $($first.reason).")
        }
    }

    if ($layout.overlapCount -gt 0) {
        $isFirstOverlapSample = $evidence.refreshChecks.layoutOverlapSamples.Count -eq 0
        $evidence.refreshChecks.layoutOverlapSamples += [pscustomobject]@{
            phase = $Snapshot.phase
            sampleIndex = $Snapshot.sampleIndex
            capturedAtUtc = $Snapshot.capturedAtUtc
            overlapCount = $layout.overlapCount
            overlaps = @($layout.overlaps)
        }
        if ($isFirstOverlapSample) {
            $first = $layout.overlaps[0]
            $failures.Add(
                "Process rows overlap during '$($Snapshot.phase)': '$($first.firstIdentity)' and '$($first.secondIdentity)' intersect vertically by $($first.verticalOverlapPixels) px (tolerance $LayoutOverlapTolerancePixels px).")
        }
    }
}

function Get-ProcessRowSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][int]$SampleIndex
    )

    $elements = $Root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
    $rows = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        try {
            $current = $element.Current
            if ($current.ControlType -ne [Windows.Automation.ControlType]::Button -or
                [string]$current.Name -notmatch '^.+ PID [0-9]+$') {
                continue
            }
            $record = Get-UiaElementRecord -Element $element
            if ($record.unavailable -or @($record.runtimeId).Count -eq 0) {
                continue
            }
            $rows += [pscustomobject]@{
                identity = [string]$record.name
                runtimeId = [string]::Join('.', @($record.runtimeId))
                isOffscreen = [bool]$record.isOffscreen
                bounds = $record.bounds
            }
        }
        catch {
            # A row can disappear between FindAll and Current while a broken
            # build resets the collection. The next sample records the result.
            continue
        }
    }

    $orderedRows = @($rows | Sort-Object identity)
    return [pscustomobject]@{
        phase = $Phase
        sampleIndex = $SampleIndex
        capturedAtUtc = [DateTime]::UtcNow.ToString('o')
        rowCount = @($rows).Count
        visibleRowCount = @($rows | Where-Object { -not $_.isOffscreen }).Count
        rows = $orderedRows
        layout = Measure-ProcessRowLayout -Rows $orderedRows `
            -OverlapTolerancePixels $LayoutOverlapTolerancePixels
    }
}

function Wait-ProcessRowSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Root,
        [Parameter(Mandatory = $true)][datetime]$Deadline
    )

    $last = $null
    do {
        $last = Get-ProcessRowSnapshot -Root $Root -Phase 'baseline' -SampleIndex 0
        if ($last.rowCount -gt 0) {
            return $last
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $Deadline)
    throw "Processes did not expose any UIA data rows before the baseline deadline. Last snapshot: $($last | ConvertTo-Json -Compress -Depth 5)"
}

function Compare-ProcessRowRuntimeIds {
    param(
        [Parameter(Mandatory = $true)]$Baseline,
        [Parameter(Mandatory = $true)]$Candidate
    )

    $baselineByIdentity = @{}
    foreach ($row in @($Baseline.rows)) {
        $baselineByIdentity[$row.identity] = $row.runtimeId
    }
    $common = @()
    $changed = @()
    foreach ($row in @($Candidate.rows)) {
        if (-not $baselineByIdentity.ContainsKey($row.identity)) {
            continue
        }
        $common += $row.identity
        $before = [string]$baselineByIdentity[$row.identity]
        if ($before -cne [string]$row.runtimeId) {
            $changed += [pscustomobject]@{
                identity = $row.identity
                baselineRuntimeId = $before
                observedRuntimeId = $row.runtimeId
            }
        }
    }
    return [pscustomobject]@{
        commonCount = @($common).Count
        changedCount = @($changed).Count
        changed = @($changed)
    }
}

try {
    $process = $session.process
    $process.Refresh()
    Assert-NoWebViewModules -Process $process
    $root = Get-ReactorUiaRoot -Process $process

    # 1. Establish a settled UIA baseline before capturing. The collapse floor
    #    deliberately allows ordinary process churn while rejecting the 0/1/9
    #    row reset frames produced by the old virtual-collection replacement.
    Start-Sleep -Seconds $HoldSeconds
    $baseline = Wait-ProcessRowSnapshot -Root $root -Deadline (Get-Date).AddSeconds(10)
    $collapseFloor = [Math]::Max(1, [Math]::Floor($baseline.rowCount * 0.5))
    $evidence.refreshChecks.baselineRowCount = $baseline.rowCount
    $evidence.refreshChecks.collapseFloor = $collapseFloor
    $evidence.refreshChecks.samples += $baseline
    Register-ProcessRowLayoutResult -Snapshot $baseline
    $initial = Invoke-Capture "processes-initial"

    # 2. Repeatedly refresh and sample both row count and RuntimeId. RuntimeId
    #    is the UIA identity of the native row control; broad simultaneous
    #    changes distinguish collection replacement from ordinary row moves.
    $mid = $null
    $sampleIndex = 0
    for ($cycle = 1; $cycle -le $RefreshCycles; $cycle++) {
        $refreshButton = Wait-UniqueUiaButton -Root $root `
            -Deadline (Get-Date).AddSeconds(10) -Name "Refresh processes"
        Invoke-UiaButtonElement -Element $refreshButton.element

        $deadline = (Get-Date).AddMilliseconds($ObserveMilliseconds)
        do {
            $sampleIndex++
            $snapshot = Get-ProcessRowSnapshot -Root $root `
                -Phase "refresh-$cycle" -SampleIndex $sampleIndex
            $comparison = Compare-ProcessRowRuntimeIds `
                -Baseline $baseline -Candidate $snapshot
            $snapshot | Add-Member -NotePropertyName commonBaselineRows `
                -NotePropertyValue $comparison.commonCount -Force
            $snapshot | Add-Member -NotePropertyName changedRuntimeIds `
                -NotePropertyValue $comparison.changedCount -Force
            $changedPercent = if ($comparison.commonCount -eq 0) {
                0.0
            }
            else {
                100.0 * $comparison.changedCount / $comparison.commonCount
            }
            $snapshot | Add-Member -NotePropertyName changedRuntimeIdPercent `
                -NotePropertyValue ([Math]::Round($changedPercent, 2)) -Force
            $evidence.refreshChecks.samples += $snapshot
            Register-ProcessRowLayoutResult -Snapshot $snapshot

            if ($snapshot.rowCount -lt $collapseFloor) {
                $collapse = [pscustomobject]@{
                    cycle = $cycle
                    sampleIndex = $sampleIndex
                    rowCount = $snapshot.rowCount
                    expectedMinimum = $collapseFloor
                    capturedAtUtc = $snapshot.capturedAtUtc
                }
                $evidence.refreshChecks.collapseSamples += $collapse
                if ($evidence.refreshChecks.collapseSamples.Count -eq 1) {
                    $failures.Add(
                        "Process list collapsed during refresh: sample $sampleIndex exposed $($snapshot.rowCount) rows; expected at least $collapseFloor from a $($baseline.rowCount)-row baseline.")
                }
            }

            if ($comparison.changedCount -gt 0) {
                $identityChange = [pscustomobject]@{
                    cycle = $cycle
                    sampleIndex = $sampleIndex
                    commonCount = $comparison.commonCount
                    changedCount = $comparison.changedCount
                    changed = @($comparison.changed | Select-Object -First 10)
                }
                $evidence.refreshChecks.identityChanges += $identityChange
            }

            # CPU sorting legitimately moves a minority of rows, and WinUI can
            # assign a moved element a new RuntimeId. The regression destroyed
            # the collection wholesale, changing every shared row (100% in the
            # captured failing build). Require a broad 80% churn event across
            # at least five stable identities so row movement/process exits do
            # not become flaky failures.
            if ($comparison.commonCount -ge 5 -and
                $changedPercent -ge $IdentityChurnPercent) {
                $identityChurn = [pscustomobject]@{
                    cycle = $cycle
                    sampleIndex = $sampleIndex
                    commonCount = $comparison.commonCount
                    changedCount = $comparison.changedCount
                    changedPercent = [Math]::Round($changedPercent, 2)
                    changed = @($comparison.changed | Select-Object -First 10)
                }
                $evidence.refreshChecks.identityChurnSamples += $identityChurn
                if ($evidence.refreshChecks.identityChurnSamples.Count -eq 1) {
                    $first = $comparison.changed[0]
                    $failures.Add(
                        "Process row UIA identity churned during refresh: '$($first.identity)' changed RuntimeId from '$($first.baselineRuntimeId)' to '$($first.observedRuntimeId)' ($($comparison.changedCount) of $($comparison.commonCount) shared rows, $([Math]::Round($changedPercent, 1))%, changed in sample $sampleIndex; threshold is $IdentityChurnPercent%).")
                }
            }

            if ($null -eq $mid) {
                $mid = Invoke-Capture "processes-mid-refresh"
                # Keep a full observation window after the screenshot helper,
                # which may itself take long enough for one refresh to settle.
                $deadline = (Get-Date).AddMilliseconds($ObserveMilliseconds)
            }
            Start-Sleep -Milliseconds $SampleIntervalMilliseconds
        } while ((Get-Date) -lt $deadline)
    }

    # 3. Settled after refresh.
    Start-Sleep -Seconds 2
    $settled = Invoke-Capture "processes-refreshed"

    # Combined sheets against the Store baseline when it exists.
    $storeBaselinePath = Get-AbsolutePath -Path $StoreBaselinePng
    if (Test-Path -LiteralPath $storeBaselinePath) {
        $storePath = (Resolve-Path -LiteralPath $storeBaselinePath).Path
        foreach ($pair in @(
            @{ Reactor = $initial; Name = "processes-initial" },
            @{ Reactor = $settled; Name = "processes-refreshed" },
            @{ Reactor = $mid; Name = "processes-mid-refresh" })) {
            $sheet = Join-Path $outputDirectory "$($pair.Name)-store-left-reactor-right.png"
            New-CombinedImage -LeftPath $storePath -RightPath $pair.Reactor `
                -OutputPath $sheet `
                -LeftLabel "Store 2.5.8" -RightLabel "Reactor"
            $evidence.captures += $sheet
        }
        Write-Host "Combined Store/Reactor sheets written."
    }
    else {
        Write-Warning "Store baseline '$StoreBaselinePng' not found; combined sheets skipped."
    }

    # Record the triptych in variants.json (defect evidence).
    $variantsPath = Get-AbsolutePath -Path $VariantsJson
    if (Test-Path -LiteralPath $variantsPath) {
        $document = Get-Content -LiteralPath $variantsPath -Raw | ConvertFrom-Json
        $defectStatus = if ($failures.Count -eq 0) { "fixed" } else { "open" }
        $defects = @($document.defects | ForEach-Object {
            if ($_.id -eq "processes-refresh-rendering") {
                $_ | Add-Member -NotePropertyName evidence -NotePropertyValue @(
                    $initial, $mid, $settled) -Force
                $_ | Add-Member -NotePropertyName status -NotePropertyValue $defectStatus -Force
            }
            $_
        })
        $document.defects = $defects
        Write-JsonFile -Value $document -Path $variantsPath
        Write-Host "Defect record updated in $variantsPath."
    }
}
catch {
    $failures.Add($_.Exception.Message)
}
finally {
    try {
        $close = Stop-ReactorCandidate -Session $session `
            -ExecutablePaths @($resolvedExecutable) -GraceSeconds 8
        $evidence.gracefulClose = $close.gracefulClose
        $evidence.crashEvents = $close.crashEvents
        if (-not $close.gracefulClose) {
            $failures.Add("Candidate did not close gracefully.")
        }
    }
    catch {
        $failures.Add("Cleanup failed: $($_.Exception.Message)")
    }
}

$evidencePath = Join-Path $outputDirectory "process-refresh-$stamp.json"
Write-JsonFile -Value $evidence -Path $evidencePath
Write-Host "Evidence: $evidencePath"

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "FAIL: $failure"
    }
    exit 1
}
Write-Host "Process-refresh parity captures complete."
exit 0
