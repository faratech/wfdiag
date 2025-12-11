# PowerShell script to verify Sigstore signatures locally

param(
    [Parameter(Mandatory=$true)]
    [string]$ArtifactPath,
    
    [Parameter(Mandatory=$true)]
    [string]$SignaturePath,
    
    [Parameter(Mandatory=$true)]
    [string]$CertificatePath
)

Write-Host "Verifying Sigstore signature for: $ArtifactPath" -ForegroundColor Green

# Check if files exist
if (-not (Test-Path $ArtifactPath)) {
    Write-Error "Artifact file not found: $ArtifactPath"
    exit 1
}

if (-not (Test-Path $SignaturePath)) {
    Write-Error "Signature file not found: $SignaturePath"
    exit 1
}

if (-not (Test-Path $CertificatePath)) {
    Write-Error "Certificate file not found: $CertificatePath"
    exit 1
}

# Set environment for experimental features
$env:COSIGN_EXPERIMENTAL = 1

try {
    Write-Host "Running cosign verification..." -ForegroundColor Yellow
    
    $result = & cosign-windows-amd64 verify-blob --signature $SignaturePath --certificate $CertificatePath $ArtifactPath
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ Signature verification PASSED" -ForegroundColor Green
        Write-Host "Verification details:" -ForegroundColor Cyan
        Write-Host $result
    } else {
        Write-Host "❌ Signature verification FAILED" -ForegroundColor Red
        exit 1
    }
} catch {
    Write-Error "Error during verification: $_"
    exit 1
}

Write-Host "Signature verification completed successfully!" -ForegroundColor Green