# Git-Crypt Setup for Private Release Files

## Install Git-Crypt
```bash
# Windows (via Chocolatey)
choco install git-crypt

# Or download from: https://github.com/AGWA/git-crypt
```

## Setup Process
```bash
# Initialize git-crypt in repository
git-crypt init

# Add collaborators (optional - for team access)
git-crypt add-gpg-user YOUR_GPG_KEY_ID

# Export the symmetric key for GitHub Actions
git-crypt export-key .git-crypt-key

# Add the key as a GitHub Secret
base64 -w 0 .git-crypt-key  # Copy this to GITHUB_SECRET
```

## GitHub Actions Integration
Add this step to workflows that need access to encrypted files:

```yaml
- name: Decrypt repository files
  env:
    GIT_CRYPT_KEY: ${{ secrets.GIT_CRYPT_KEY }}
  run: |
    echo "$GIT_CRYPT_KEY" | base64 -d > .git-crypt-key
    git-crypt unlock .git-crypt-key
    rm .git-crypt-key
```

## Benefits
- ✅ Files encrypted in repository
- ✅ Actions can decrypt with secret key
- ✅ Only authorized users can access
- ✅ Transparent encryption/decryption