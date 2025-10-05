#!/usr/bin/env bash
# Bash script to bump version number across all project files

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Function to display usage
usage() {
    echo "Usage: $0 <new-version> [--dry-run]"
    echo ""
    echo "Example: $0 2.1.4"
    echo "         $0 2.1.4 --dry-run"
    exit 1
}

# Check arguments
if [ $# -lt 1 ]; then
    usage
fi

NEW_VERSION="$1"
DRY_RUN=false

if [ $# -eq 2 ] && [ "$2" = "--dry-run" ]; then
    DRY_RUN=true
fi

# Validate version format
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo -e "${RED}Error: Invalid version format. Expected format: X.Y.Z (e.g., 2.1.3)${NC}"
    exit 1
fi

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${CYAN}Bumping version to $NEW_VERSION...${NC}"
echo ""

UPDATED_FILES=0
ERROR_FILES=()

# Function to update file
update_file() {
    local file="$1"
    local pattern="$2"
    local replacement="$3"

    if [ ! -f "$file" ]; then
        echo -e "${YELLOW}Warning: File not found: $file${NC}"
        ERROR_FILES+=("$file")
        return
    fi

    if $DRY_RUN; then
        if grep -qE "$pattern" "$file"; then
            echo -e "${YELLOW}[DRY RUN] Would update: $file${NC}"
            ((UPDATED_FILES++)) || true
        else
            echo -e "${YELLOW}Warning: Pattern not found in: $file${NC}"
        fi
    else
        if grep -qE "$pattern" "$file"; then
            # Use perl for in-place regex replacement (cross-platform)
            if command -v perl > /dev/null 2>&1; then
                perl -pi -e "$replacement" "$file"
                echo -e "${GREEN}✓ Updated: $file${NC}"
                ((UPDATED_FILES++)) || true
            else
                # Fallback to sed
                sed -i.bak -E "$replacement" "$file"
                rm -f "${file}.bak"
                echo -e "${GREEN}✓ Updated: $file${NC}"
                ((UPDATED_FILES++)) || true
            fi
        else
            echo -e "${YELLOW}Warning: Pattern not found in: $file${NC}"
        fi
    fi
}

# Update package.json
update_file "package.json" \
    '"version":\s+"2\.[0-9]+\.[0-9]+"' \
    's/("version":\s+)"2\.[0-9]+\.[0-9]+"/$1"'"$NEW_VERSION"'"/g'

# Update package-lock.json (multiple occurrences)
update_file "package-lock.json" \
    '"version":\s+"2\.[0-9]+\.[0-9]+"' \
    's/("version":\s+)"2\.[0-9]+\.[0-9]+"/$1"'"$NEW_VERSION"'"/g'

# Update AppxManifest.xml (needs .0 suffix)
update_file "AppxManifest.xml" \
    'Version="2\.[0-9]+\.[0-9]+\.0"' \
    's/(Version=)"2\.[0-9]+\.[0-9]+\.0"/$1"'"$NEW_VERSION"'.0"/g'

# Update Cargo.toml
update_file "src-tauri/Cargo.toml" \
    'version\s+=\s+"2\.[0-9]+\.[0-9]+"' \
    's/(version\s+=\s+)"2\.[0-9]+\.[0-9]+"/$1"'"$NEW_VERSION"'"/g'

# Update tauri.conf.json
update_file "src-tauri/tauri.conf.json" \
    '"version":\s+"2\.[0-9]+\.[0-9]+"' \
    's/("version":\s+)"2\.[0-9]+\.[0-9]+"/$1"'"$NEW_VERSION"'"/g'

# Update App.tsx
update_file "src/App.tsx" \
    'version="2\.[0-9]+\.[0-9]+"' \
    's/(version=)"2\.[0-9]+\.[0-9]+"/$1"'"$NEW_VERSION"'"/g'

# Update AboutDialog.tsx
update_file "src/components/AboutDialog.tsx" \
    'Version\s+2\.[0-9]+\.[0-9]+' \
    's/(Version\s+)2\.[0-9]+\.[0-9]+/$1'"$NEW_VERSION"'/g'

# Update NavigationHeader.tsx
update_file "src/components/NavigationHeader.tsx" \
    "version\s+=\s+'2\.[0-9]+\.[0-9]+'" \
    "s/(version\s+=\s+)'2\.[0-9]+\.[0-9]+'/"'$1'"'$NEW_VERSION'"'/g"

echo ""
if $DRY_RUN; then
    echo -e "${YELLOW}DRY RUN completed. $UPDATED_FILES file(s) would be updated.${NC}"
else
    echo -e "${GREEN}Version bump completed! $UPDATED_FILES file(s) updated to version $NEW_VERSION${NC}"
fi

if [ ${#ERROR_FILES[@]} -gt 0 ]; then
    echo ""
    echo -e "${YELLOW}Warning: Some files could not be updated:${NC}"
    for file in "${ERROR_FILES[@]}"; do
        echo -e "  ${RED}- $file${NC}"
    done
    exit 1
fi

if ! $DRY_RUN; then
    echo ""
    echo -e "${CYAN}Next steps:${NC}"
    echo "  1. Review changes: git diff"
    echo "  2. Commit changes: git add -A && git commit -m 'Bump version to $NEW_VERSION'"
    echo "  3. Build release: npm run tauri build"
fi

exit 0
