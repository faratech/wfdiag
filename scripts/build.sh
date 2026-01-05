#!/bin/bash
# Build script that ensures dependencies are updated before building

set -e

TARGET="${1:-all}"
FORCE="${2}"

echo "WindowsForum Diagnostic Tool - Build with Dependency Updates"
echo "============================================================"

update_dependencies() {
    echo
    echo "Updating NPM dependencies..."
    if npm update; then
        echo "✓ NPM dependencies updated successfully"
    else
        echo "❌ Failed to update NPM dependencies"
        exit 1
    fi

    echo
    echo "Updating Cargo dependencies..."
    if cd src-tauri && cargo update && cd ..; then
        echo "✓ Cargo dependencies updated successfully"
    else
        echo "❌ Failed to update Cargo dependencies"
        exit 1
    fi
}

build_application() {
    local build_type="$1"
    
    echo
    echo "Building application ($build_type)..."
    
    case "$build_type" in
        "all")
            npm run tauri build
            ;;
        "msix")
            npm run tauri build -- --bundles msix
            ;;
        "msi")
            npm run tauri build -- --bundles msi
            ;;
        "nsis")
            npm run tauri build -- --bundles nsis
            ;;
        "exe")
            cd src-tauri && cargo build --release && cd ..
            ;;
        *)
            npm run tauri build
            ;;
    esac
    
    echo "✓ Build completed successfully"
}

# Main execution
if [ "$FORCE" != "--force" ]; then
    update_dependencies
else
    echo "Skipping dependency updates (--force specified)"
fi

build_application "$TARGET"

echo
echo "🎉 Build process completed successfully!"
echo "Built target: $TARGET"