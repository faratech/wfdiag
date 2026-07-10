# PowerShell script to store release file as GitHub Secret
param(
    [string]$FilePath = "release\wfdiag-2.1.1.exe",
    [string]$SecretName = "RELEASE_BINARY_BASE64"
)

if (-not (Test-Path $FilePath)) {
    Write-Error "File not found: $FilePath"
    exit 1
}

Write-Host "🔐 Encoding $FilePath for GitHub Secrets..." -ForegroundColor Cyan

try {
    $fileBytes = [IO.File]::ReadAllBytes((Resolve-Path $FilePath).Path)
    $base64 = [Convert]::ToBase64String($fileBytes)
    
    Write-Host "✅ File encoded successfully!" -ForegroundColor Green
    Write-Host "📋 Next steps:" -ForegroundColor Yellow
    Write-Host "1. Copy the base64 string below" -ForegroundColor White
    Write-Host "2. Add GitHub Secret named: $SecretName" -ForegroundColor White
    Write-Host "3. Paste the base64 value" -ForegroundColor White
    Write-Host ""
    Write-Host "Base64 value (copy this):" -ForegroundColor Cyan
    Write-Host $base64 -ForegroundColor White
    
} catch {
    Write-Error "Failed to encode file: $_"
    exit 1
}