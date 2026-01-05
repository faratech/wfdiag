# Sigstore Setup for WF Diagnostics

This repository is now configured with Sigstore for keyless code signing using GitHub Actions and CLI tools.

## What's Included

### 🔧 CLI Tools Installed
- **Cosign v2.5.3** - Primary Sigstore signing tool
- **Command**: `cosign-windows-amd64`

### 📋 GitHub Actions Workflow
- **File**: `.github/workflows/sigstore-sign.yml`
- **Triggers**: Push to main, tags, PRs, releases
- **Features**:
  - Builds Tauri app with latest dependencies
  - Signs executables, MSI, and NSIS installers
  - Generates GitHub attestations for releases
  - Verifies signatures automatically
  - Uploads signatures as artifacts

### 🛠️ Local Scripts
- **`scripts/sign-local.ps1`** - Sign artifacts locally (requires GitHub login)
- **`scripts/verify-signatures.ps1`** - Verify Sigstore signatures
- **`signatures/`** - Directory for storing signature files

## Usage

### Automatic Signing (GitHub Actions)
1. **Push to main** or **create a release** 
2. GitHub Actions will automatically:
   - Build your Tauri application
   - Sign all executables and installers
   - Generate attestations
   - Upload signatures as artifacts

### Manual Local Signing
```powershell
# Sign an executable locally
.\scripts\sign-local.ps1 -ArtifactPath "src-tauri\target\release\wfdiag-tauri.exe"

# This will:
# 1. Open browser for GitHub OIDC authentication
# 2. Create signature files in signatures/ directory
# 3. Verify the signature immediately
```

### Verify Signatures
```powershell
# Verify a signed artifact
.\scripts\verify-signatures.ps1 `
  -ArtifactPath "src-tauri\target\release\wfdiag-tauri.exe" `
  -SignaturePath "signatures\wfdiag-tauri.exe.sig" `
  -CertificatePath "signatures\wfdiag-tauri.exe.pem"
```

### CLI Commands
```bash
# Sign a file (requires GitHub login)
cosign-windows-amd64 sign-blob --yes \
  --output-signature file.sig \
  --output-certificate file.pem \
  file.exe

# Verify a signature
set COSIGN_EXPERIMENTAL=1
cosign-windows-amd64 verify-blob \
  --signature file.sig \
  --certificate file.pem \
  file.exe
```

## Security Features

### ✅ Keyless Signing
- No private keys to manage
- Uses GitHub OIDC tokens for authentication
- Identity verified through transparency logs

### ✅ Transparency
- All signatures logged in public Rekor transparency log
- Verifiable build provenance with GitHub attestations
- Certificate transparency through Fulcio CA

### ✅ Verification
- Anyone can verify signatures without secrets
- Supports offline verification with certificate chains
- GitHub provides additional attestation verification

## Permissions Required

The GitHub Actions workflow requires these permissions:
- `contents: read` - Access repository code
- `id-token: write` - Generate OIDC tokens for Sigstore
- `attestations: write` - Create GitHub attestations

## File Extensions

Generated signature files:
- `.sig` - Sigstore signature
- `.pem` - X.509 certificate with identity information

## Distribution

When distributing your application:
1. Include the signature files (.sig, .pem) alongside executables
2. Users can verify authenticity with Cosign
3. GitHub releases automatically include attestations

## Verification by End Users

Users can verify your releases:
```bash
# Install cosign
winget install sigstore.cosign

# Verify a downloaded executable
set COSIGN_EXPERIMENTAL=1
cosign-windows-amd64 verify-blob \
  --signature wfdiag-tauri.exe.sig \
  --certificate wfdiag-tauri.exe.pem \
  wfdiag-tauri.exe
```

## Next Steps

1. **Push to GitHub** - Trigger the first signing workflow
2. **Create a release** - Generate attestations and signatures
3. **Test verification** - Download and verify signatures work
4. **Update documentation** - Add verification instructions for users

Your application now has enterprise-grade code signing without managing certificates! 🎉