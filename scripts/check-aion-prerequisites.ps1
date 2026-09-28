# Read-only check for the optional Aion Preview deployment profile.
# Run as the same Windows user who runs WFDiag. No downloads or installs.
$ErrorActionPreference = 'Stop'
$checks = @()
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$checks += [pscustomobject]@{ Check = 'ARM64 host (Snapdragon NPU also required)'; Passed = ($arch -eq 'Arm64'); Detail = $arch }
foreach ($requirement in @(
    @{ Name = 'Microsoft.AionInstructPreview.Framework.1.0'; Floor = [version]'1.0.0.0' },
    @{ Name = 'Microsoft.WindowsAppRuntime.1.8'; Floor = [version]'8000.836.2153.0' },
    @{ Name = 'MicrosoftCorporationII.WinML.Qualcomm.QNN.EP.1.8*'; Floor = [version]'1.8.41.0' }
)) {
    $packages = @(Get-AppxPackage -Name $requirement.Name | Where-Object {
        $_.Architecture -eq 'Arm64' -and [version]$_.Version -ge $requirement.Floor
    })
    $checks += [pscustomobject]@{
        Check = $requirement.Name
        Passed = ($packages.Count -gt 0)
        Detail = ($packages | ForEach-Object { $_.PackageFullName }) -join '; '
    }
}
$checks | Format-Table -AutoSize | Out-Host
if (@($checks | Where-Object { -not $_.Passed }).Count -gt 0) {
    Write-Host 'See docs/AION_PREVIEW_SETUP.md for the pinned Microsoft framework and QNN acquisition tool.'
    exit 1
}
Write-Host 'Package prerequisites found. Run the official sample and WFDiag under registered identity to verify NPU activation and inference.'
exit 0
