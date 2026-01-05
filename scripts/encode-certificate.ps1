# PowerShell script to encode the certificate for GitHub Secrets
# Run this script to get the base64 value needed for CERTIFICATE_BASE64 secret

$certPath = "wfdiag-signing-cert.pfx"

if (-not (Test-Path $certPath)) {
    Write-Error "Certificate file not found: $certPath"
    Write-Host "Make sure you're in the directory containing the certificate file." -ForegroundColor Yellow
    exit 1
}

Write-Host "🔐 Encoding certificate for GitHub Secrets..." -ForegroundColor Cyan
Write-Host "Certificate file: $certPath" -ForegroundColor Gray

try {
    $certBytes = [IO.File]::ReadAllBytes((Resolve-Path $certPath).Path)
    $certBase64 = [Convert]::ToBase64String($certBytes)
    
    Write-Host "`n✅ Certificate encoded successfully!" -ForegroundColor Green
    Write-Host "===============================================" -ForegroundColor Cyan
    Write-Host "CERTIFICATE_BASE64 value:" -ForegroundColor Yellow
    Write-Host "===============================================" -ForegroundColor Cyan
    Write-Host $certBase64 -ForegroundColor White
    Write-Host "===============================================" -ForegroundColor Cyan
    
    Write-Host "`n📋 Next steps:" -ForegroundColor Yellow
    Write-Host "1. Copy the base64 value above" -ForegroundColor White
    Write-Host "2. Go to GitHub repository → Settings → Secrets and variables → Actions" -ForegroundColor White
    Write-Host "3. Add new secret named: CERTIFICATE_BASE64" -ForegroundColor White
    Write-Host "4. Paste the base64 value" -ForegroundColor White
    Write-Host "5. Add another secret named: CERTIFICATE_PASSWORD" -ForegroundColor White
    Write-Host "6. Set password value to: WindowsForum2024!" -ForegroundColor White
    
    Write-Host "`n🚀 Then run the 'Sign with Self-Signed Certificate' workflow!" -ForegroundColor Green
    
} catch {
    Write-Error "Failed to encode certificate: $($_.Exception.Message)"
    exit 1
}