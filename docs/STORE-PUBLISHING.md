# Microsoft Store Publishing Setup

This document describes how to set up automated Microsoft Store publishing via GitHub Actions.

## Overview

The `build-and-publish-store.yml` workflow automates:
1. Building x64 and ARM64 executables
2. Creating an unsigned MSIX bundle with x64/ARM64 packages and the Phi Silica AI SDK DLLs
3. Validating the package identity, capabilities, framework dependency, contents, and architectures
4. Generating GitHub artifact attestations
5. Creating a GitHub Release
6. Sending the bundle to Microsoft Store certification when requested

The repository does not apply the production Store signature. Microsoft signs the package delivered to customers after Store certification.

## Required GitHub Secrets

### For Microsoft Store Publishing

| Secret | Description |
|--------|-------------|
| `AZURE_TENANT_ID` | Your Azure AD tenant ID |
| `AZURE_CLIENT_ID` | Azure AD application (client) ID |
| `AZURE_CLIENT_SECRET` | Azure AD application client secret |
| `STORE_SELLER_ID` | Partner Center seller ID used by the Store CLI |

## Setting Up Microsoft Store API Access

### Step 1: Associate Azure AD with Partner Center

1. Go to [Partner Center](https://partner.microsoft.com/dashboard)
2. Navigate to **Settings** (gear icon) → **Account settings** → **User management**
3. Click **Azure AD applications** tab
4. Click **Create new application** or **Add existing Azure AD application**

### Step 2: Create an Azure AD Application

If you don't have an existing Azure AD app:

1. Go to [Azure Portal](https://portal.azure.com)
2. Navigate to **Azure Active Directory** → **App registrations**
3. Click **New registration**
4. Enter a name (e.g., "WindowsForum Diagnostics CI")
5. For **Supported account types**, select "Accounts in this organizational directory only"
6. Click **Register**

### Step 3: Generate Client Secret

1. In your app registration, go to **Certificates & secrets**
2. Click **New client secret**
3. Add a description and set expiration
4. Copy the **Value** immediately (it won't be shown again)
5. Save this as `AZURE_CLIENT_SECRET` in GitHub Secrets

### Step 4: Get Required IDs

From your Azure AD app registration:
- **Application (client) ID** → `AZURE_CLIENT_ID`
- **Directory (tenant) ID** → `AZURE_TENANT_ID`

### Step 5: Assign Role in Partner Center

1. Go to Partner Center → **Settings** → **User management** → **Azure AD applications**
2. Find your application and click on it
3. Assign the **Manager** role

### Step 6: Get Store Product ID

1. In Partner Center, go to your app
2. The Product ID is in the URL: `https://partner.microsoft.com/dashboard/products/{PRODUCT_ID}`
3. Or find it under **App overview** → **Product identity**

## Workflow Triggers

### Tag-Based Releases (Recommended)

Tags control what gets released:

| Tag Format | Example | GitHub Release | Store Publish |
|------------|---------|----------------|---------------|
| `v{major}.{minor}.{patch}` | `v2.5.8` | Yes | Yes |

The tag must match the version synchronized in `version.json` and every package/UI version source. CI and the release workflow enforce this before packaging.

**Release to GitHub and Microsoft Store:**
```bash
VERSION="$(python3 -c 'import json; print(json.load(open("version.json"))["version"])')"
git tag "v${VERSION}"
git push origin "v${VERSION}"
```

### Manual Dispatch

For releases without creating a tag:
```bash
VERSION="$(python3 -c 'import json; print(json.load(open("version.json"))["version"])')"
gh workflow run build-and-publish-store.yml \
  -f version="${VERSION}" \
  -f publish_to_store=true \
  -f create_release=true
```

## Workflow Jobs

```
┌──────────────┐     ┌───────────────┐
│  build-x64   │     │  build-arm64  │
└──────┬───────┘     └───────┬───────┘
       └──────────┬───────────┘
                  ▼
       ┌──────────────────────┐
       │ create and validate  │
       │ unsigned MSIX bundle │
       └──────────┬───────────┘
                  ▼
       ┌──────────────────────┐
       │ attest artifacts     │
       └──────────┬───────────┘
                  ▼
       ┌──────────────────────┐
       │ create GitHub release│
       └──────────┬───────────┘
                  ▼
       ┌──────────────────────┐
       │ optionally dispatch  │
       │ Store publish        │
       └──────────────────────┘
```

## Build Artifacts

The workflow produces:

| Artifact | Description |
|----------|-------------|
| `exe-x64` | x64 Windows executable |
| `exe-arm64` | ARM64 Windows executable |
| `frontend-dist` | Built frontend assets |
| `msixbundle-unsigned` | Unsigned MSIX bundle |

The unsigned bundle is the expected GitHub artifact and Store upload. The Microsoft-signed package is distributed by the Store and is not downloaded back into the workflow.

## MSIX Package Contents

Each architecture package includes:
- `WindowsForum_Diagnostics.exe` - Main executable
- `dist/` - Frontend assets
- `Logo.png`, `Square150x150Logo.png`, `Square44x44Logo.png` - App icons
- `AppxManifest.xml` - Package manifest with `systemAIModels` capability
- Windows App SDK AI DLLs (for Phi Silica support):
  - `Microsoft.Windows.AI.Text.dll`
  - `Microsoft.Windows.AI.Text.Projection.dll`
  - `Microsoft.WindowsAppRuntime.dll`
  - `Microsoft.WindowsAppRuntime.Bootstrap.dll`
  - `WinRT.Runtime.dll`

## Attestations

All artifacts are attested using GitHub's artifact attestation feature (Sigstore-based).

To verify an attestation:
```bash
VERSION="$(python3 -c 'import json; print(json.load(open("version.json"))["version"])')"
gh attestation verify "WindowsForum_Diagnostics_${VERSION}.msixbundle" \
  --owner faratech
```

## Troubleshooting

### ARM64 Build Fails
The GitHub-hosted Windows runners include ARM64 build tools. If builds fail:
1. Check that `aarch64-pc-windows-msvc` target is installed
2. Verify Visual Studio ARM64 components are available

### Store Submission Fails
1. Verify all Azure AD secrets are correct
2. Check that the app already exists in Partner Center (the API only updates existing apps)
3. Ensure the Azure AD app has the Manager role in Partner Center

## Local Development

For local builds, use the existing scripts:

```bash
# Build the same unsigned bundle submitted to the Store
python3 scripts/build-cross.py build-all --build-msix

# Optional: add --sign only for a locally sideloadable test package
python3 scripts/build-cross.py build-all --build-msix --sign
```

## References

- [Microsoft Store Dev CLI with GitHub Actions](https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/github-actions)
- [store-submission GitHub Action](https://github.com/microsoft/store-submission)
- [GitHub Artifact Attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
