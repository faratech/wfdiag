# Certificate Setup for GitHub Actions

This document explains how to set up the self-signed certificate for Windows code signing in GitHub Actions.

## Required GitHub Secrets

Add these secrets to your GitHub repository (Settings → Secrets and variables → Actions):

### 1. CERTIFICATE_BASE64
The base64-encoded PFX certificate file.

**To generate this value:**
```powershell
# Run this in PowerShell where wfdiag-signing-cert.pfx exists
$certBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes("wfdiag-signing-cert.pfx"))
Write-Host $certBase64
```

Copy the output and add it as a secret named `CERTIFICATE_BASE64`.

### 2. CERTIFICATE_PASSWORD  
The password for the PFX certificate.

**Current password:** `WindowsForum2024!`

Add this as a secret named `CERTIFICATE_PASSWORD`.

## Certificate Details

- **File**: `wfdiag-signing-cert.pfx`
- **Subject**: `CN=WindowsForum Fara Technologies LLC IT Department, OU=IT Department, O=WindowsForum Fara Technologies LLC, L=New York, ST=NY, C=US`
- **Purpose**: Code signing for Windows executables
- **Type**: Self-signed certificate
- **Thumbprint**: `1b7b07c6f72c5ec5683d559d5520860d207681c0`

## What the Workflow Does

The `sign-with-certificate.yml` workflow will:

1. ✅ **Find existing binaries** (*.exe, *.msi, *.msix) - no building
2. ✅ **Windows Code Signing**: Embeds digital signature in files using the PFX certificate
3. ✅ **Sigstore Signing**: Creates .sig and .crt files for cryptographic verification
4. ✅ **Verification**: Checks both signature types
5. ✅ **Reporting**: Creates detailed SIGNING_REPORT.md
6. ✅ **Upload**: All signed files and signatures as workflow artifacts

## Expected Results

After signing, your `wfdiag-2.0.8.exe` will have:

- **Embedded Windows signature** (visible in Properties → Digital Signatures)
- **Publisher information** showing "WindowsForum Fara Technologies LLC IT Department"
- **Sigstore signature files** for advanced verification
- **Reduced SmartScreen warnings** (self-signed, so some warnings remain)

## Running the Workflow

1. **Manual trigger**: Go to GitHub Actions → "Sign with Self-Signed Certificate" → "Run workflow"
2. **Automatic trigger**: Push changes to `releases/` directory

## Troubleshooting

If Windows signing fails:
- Verify `CERTIFICATE_BASE64` secret is correctly encoded
- Verify `CERTIFICATE_PASSWORD` secret matches the PFX password
- Check that the certificate hasn't expired

If Sigstore signing fails:
- This usually indicates a temporary service issue
- The workflow will continue with Windows signing only

## Security Notes

- ✅ Certificate private key is stored securely in GitHub Secrets
- ✅ Certificate is not committed to the repository  
- ✅ PFX file is created temporarily during workflow execution only
- ✅ All sensitive files are in .gitignore