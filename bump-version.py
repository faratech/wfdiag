#!/usr/bin/env python3
"""
Version bump script for WindowsForum Diagnostics
Updates version numbers across all project files.
"""

import argparse
import json
import re
import sys
from pathlib import Path


def update_json_file(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update version in a JSON file (only the root version field)."""
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')
        data = json.loads(content)

        old_version = data.get('version', 'unknown')
        if old_version == new_version:
            print(f"  Skipped (already {new_version}): {file_path}")
            return True

        if dry_run:
            print(f"  [DRY RUN] Would update: {file_path} ({old_version} -> {new_version})")
        else:
            # Only replace the first occurrence (root level version)
            # Use count=1 to avoid replacing versions in dependencies
            new_content = re.sub(
                r'("version"\s*:\s*)"[^"]+"',
                f'\\1"{new_version}"',
                content,
                count=1  # Only replace the first match (root version)
            )
            file_path.write_text(new_content, encoding='utf-8')
            print(f"  Updated: {file_path} ({old_version} -> {new_version})")

        return True
    except Exception as e:
        print(f"  Error updating {file_path}: {e}")
        return False


def update_cargo_toml(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update version in Cargo.toml."""
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')

        # Match version in [package] section (first occurrence)
        pattern = r'(^\s*version\s*=\s*")[^"]+(")'
        match = re.search(pattern, content, re.MULTILINE)

        if not match:
            print(f"  Warning: Version pattern not found in: {file_path}")
            return False

        old_version = content[match.start(1)+len(match.group(1)):match.end(2)-1]

        if dry_run:
            print(f"  [DRY RUN] Would update: {file_path} ({old_version} -> {new_version})")
        else:
            new_content = re.sub(pattern, f'\\g<1>{new_version}\\g<2>', content, count=1, flags=re.MULTILINE)
            file_path.write_text(new_content, encoding='utf-8')
            print(f"  Updated: {file_path} ({old_version} -> {new_version})")

        return True
    except Exception as e:
        print(f"  Error updating {file_path}: {e}")
        return False


def update_appx_manifest(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update version in AppxManifest.xml (adds .0 suffix)."""
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')

        # AppxManifest uses X.Y.Z.0 format
        version_with_suffix = f"{new_version}.0"
        pattern = r'(Version=")[^"]+(")'

        match = re.search(pattern, content)
        if not match:
            print(f"  Warning: Version pattern not found in: {file_path}")
            return False

        old_version = content[match.start(1)+len('Version="'):match.end(2)-1]

        if dry_run:
            print(f"  [DRY RUN] Would update: {file_path} ({old_version} -> {version_with_suffix})")
        else:
            new_content = re.sub(pattern, f'\\g<1>{version_with_suffix}\\g<2>', content)
            file_path.write_text(new_content, encoding='utf-8')
            print(f"  Updated: {file_path} ({old_version} -> {version_with_suffix})")

        return True
    except Exception as e:
        print(f"  Error updating {file_path}: {e}")
        return False


def update_tsx_file(file_path: Path, new_version: str, patterns: list, dry_run: bool) -> bool:
    """Update version in a TSX file using provided patterns."""
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')
        updated = False

        for pattern, replacement in patterns:
            if re.search(pattern, content):
                if not dry_run:
                    content = re.sub(pattern, replacement.replace('VERSION', new_version), content)
                updated = True

        if updated:
            if dry_run:
                print(f"  [DRY RUN] Would update: {file_path}")
            else:
                file_path.write_text(content, encoding='utf-8')
                print(f"  Updated: {file_path}")
            return True
        else:
            print(f"  Warning: No version patterns found in: {file_path}")
            return False
    except Exception as e:
        print(f"  Error updating {file_path}: {e}")
        return False


def main():
    parser = argparse.ArgumentParser(description='Bump version across all project files')
    parser.add_argument('version', help='New version number (e.g., 2.1.6)')
    parser.add_argument('--dry-run', action='store_true', help='Preview changes without modifying files')
    args = parser.parse_args()

    new_version = args.version
    dry_run = args.dry_run

    # Validate version format
    if not re.match(r'^\d+\.\d+\.\d+$', new_version):
        print(f"Error: Invalid version format '{new_version}'. Expected format: X.Y.Z (e.g., 2.1.6)")
        sys.exit(1)

    # Get project root
    script_dir = Path(__file__).parent.resolve()

    print(f"{'[DRY RUN] ' if dry_run else ''}Bumping version to {new_version}...")
    print()

    success_count = 0
    total_count = 0

    # 1. version.json
    total_count += 1
    if update_json_file(script_dir / 'version.json', new_version, dry_run):
        success_count += 1

    # 2. package.json
    total_count += 1
    if update_json_file(script_dir / 'package.json', new_version, dry_run):
        success_count += 1

    # 3. package-lock.json
    total_count += 1
    if update_json_file(script_dir / 'package-lock.json', new_version, dry_run):
        success_count += 1

    # 4. src-tauri/Cargo.toml
    total_count += 1
    if update_cargo_toml(script_dir / 'src-tauri' / 'Cargo.toml', new_version, dry_run):
        success_count += 1

    # 5. src-tauri/tauri.conf.json
    total_count += 1
    if update_json_file(script_dir / 'src-tauri' / 'tauri.conf.json', new_version, dry_run):
        success_count += 1

    # 6. AppxManifest.xml
    total_count += 1
    if update_appx_manifest(script_dir / 'AppxManifest.xml', new_version, dry_run):
        success_count += 1

    # 7. src/App.tsx - version="X.Y.Z"
    total_count += 1
    if update_tsx_file(
        script_dir / 'src' / 'App.tsx',
        new_version,
        [(r'version="[\d.]+"', 'version="VERSION"')],
        dry_run
    ):
        success_count += 1

    # 8. src/components/AboutDialog.tsx - Version X.Y.Z
    total_count += 1
    if update_tsx_file(
        script_dir / 'src' / 'components' / 'AboutDialog.tsx',
        new_version,
        [(r'Version\s+[\d.]+', 'Version VERSION')],
        dry_run
    ):
        success_count += 1

    # 9. src/components/NavigationHeader.tsx - version = 'X.Y.Z'
    total_count += 1
    if update_tsx_file(
        script_dir / 'src' / 'components' / 'NavigationHeader.tsx',
        new_version,
        [(r"version\s*=\s*'[\d.]+'", f"version = 'VERSION'")],
        dry_run
    ):
        success_count += 1

    print()
    if dry_run:
        print(f"[DRY RUN] {success_count}/{total_count} files would be updated to version {new_version}")
    else:
        print(f"Version bump complete! {success_count}/{total_count} files updated to version {new_version}")
        print()
        print("Next steps:")
        print("  1. Review changes: git diff")
        print(f"  2. Commit changes: git add -A && git commit -m 'Bump version to {new_version}'")
        print("  3. Build release: python3 build-cross.py build-all")

    sys.exit(0 if success_count == total_count else 1)


if __name__ == '__main__':
    main()
