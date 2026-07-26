#!/usr/bin/env python3
"""
Version bump script for WindowsForum Diagnostics
Updates version numbers across all project files.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
import xml.etree.ElementTree as ET


APPX_TARGET_DEVICE_FAMILY_MIN_VERSIONS = {
    "Windows.Universal": "10.0.26100.0",
    "Windows.Desktop": "10.0.17763.0",
}

APPX_PACKAGE_DEPENDENCY_MIN_VERSIONS = {
    "Microsoft.WindowsAppRuntime.1.8": "8000.675.1142.0",
}


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


def update_package_lock(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update package-lock.json root and packages[""].version fields."""
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        data = json.loads(file_path.read_text(encoding='utf-8'))
        old_root = data.get('version', 'unknown')
        packages = data.get('packages')
        root_package = packages.get('') if isinstance(packages, dict) else None
        old_package = root_package.get('version', 'unknown') if isinstance(root_package, dict) else 'missing'

        if old_root == new_version and old_package == new_version:
            print(f"  Skipped (already {new_version}): {file_path}")
            return True

        if dry_run:
            print(
                f"  [DRY RUN] Would update: {file_path} "
                f"(root {old_root} -> {new_version}, package {old_package} -> {new_version})"
            )
        else:
            data['version'] = new_version
            if isinstance(root_package, dict):
                root_package['version'] = new_version
            file_path.write_text(json.dumps(data, indent=4) + '\n', encoding='utf-8')
            print(
                f"  Updated: {file_path} "
                f"(root {old_root} -> {new_version}, package {old_package} -> {new_version})"
            )
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


def refresh_cargo_lock(cargo_dir: Path, dry_run: bool) -> None:
    """Re-sync Cargo.lock's own-package version entry after Cargo.toml's
    [package].version changes — otherwise the next build leaves the tree
    dirty (or fails outright under --locked)."""
    if dry_run:
        print("  [DRY RUN] Would refresh src-tauri/Cargo.lock to match the new version")
        return
    try:
        subprocess.run(
            ["cargo", "update", "--offline", "-p", "wfdiag-tauri"],
            cwd=cargo_dir,
            check=True,
            capture_output=True,
            text=True,
        )
        print("  Refreshed: src-tauri/Cargo.lock")
    except FileNotFoundError:
        print("  Warning: cargo not found on PATH — Cargo.lock was not refreshed; run "
              "'cargo update -p wfdiag-tauri' manually before committing")
    except subprocess.CalledProcessError as e:
        print(f"  Warning: failed to refresh Cargo.lock: {e.stderr.strip() if e.stderr else e}")


def update_msix_conf(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update the nested msixVersion (X.Y.Z.0) in tauri.msix.conf.json.

    The generic JSON helper only rewrites a root-level "version" key, which
    this file does not have.
    """
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')
        version_with_suffix = f"{new_version}.0"
        pattern = r'("msixVersion"\s*:\s*)"[^"]+"'

        match = re.search(pattern, content)
        if not match:
            print(f"  Warning: msixVersion not found in: {file_path}")
            return False

        old_version = re.search(r'"msixVersion"\s*:\s*"([^"]+)"', content).group(1)
        if old_version == version_with_suffix:
            print(f"  Skipped (already {version_with_suffix}): {file_path}")
            return True

        if dry_run:
            print(f"  [DRY RUN] Would update: {file_path} ({old_version} -> {version_with_suffix})")
        else:
            new_content = re.sub(pattern, f'\\g<1>"{version_with_suffix}"', content, count=1)
            file_path.write_text(new_content, encoding='utf-8')
            print(f"  Updated: {file_path} ({old_version} -> {version_with_suffix})")

        return True
    except Exception as e:
        print(f"  Error updating {file_path}: {e}")
        return False


def validate_appx_manifest_version_invariants(content: str, file_path: Path) -> bool:
    """Verify OS/framework MinVersion fields were not changed to app versions."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError as e:
        print(f"  Error parsing {file_path}: {e}")
        return False

    ok = True
    families = {
        node.attrib.get("Name"): node.attrib.get("MinVersion")
        for node in root.findall(".//{*}TargetDeviceFamily")
    }
    for name, expected in APPX_TARGET_DEVICE_FAMILY_MIN_VERSIONS.items():
        actual = families.get(name)
        if actual != expected:
            print(
                f"  Error: {file_path} TargetDeviceFamily {name} MinVersion "
                f"is {actual!r}, expected {expected!r}"
            )
            ok = False

    dependencies = {
        node.attrib.get("Name"): node.attrib.get("MinVersion")
        for node in root.findall(".//{*}PackageDependency")
    }
    for name, expected in APPX_PACKAGE_DEPENDENCY_MIN_VERSIONS.items():
        actual = dependencies.get(name)
        if actual != expected:
            print(
                f"  Error: {file_path} PackageDependency {name} MinVersion "
                f"is {actual!r}, expected {expected!r}"
            )
            ok = False

    return ok


def update_appx_manifest(file_path: Path, new_version: str, dry_run: bool) -> bool:
    """Update the package Identity version in AppxManifest.xml (adds .0 suffix).

    Do not rewrite every Version/MinVersion attribute: TargetDeviceFamily and
    PackageDependency MinVersion values are OS/framework versions, not the app
    version.
    """
    if not file_path.exists():
        print(f"  Warning: File not found: {file_path}")
        return False

    try:
        content = file_path.read_text(encoding='utf-8')
        if not validate_appx_manifest_version_invariants(content, file_path):
            return False

        # AppxManifest uses X.Y.Z.0 format
        version_with_suffix = f"{new_version}.0"
        pattern = r'(<Identity\b[^>]*\bVersion=")([^"]+)(")'

        match = re.search(pattern, content, flags=re.DOTALL)
        if not match:
            print(f"  Warning: Identity Version pattern not found in: {file_path}")
            return False

        old_version = match.group(2)

        new_content = re.sub(
            pattern,
            f'\\g<1>{version_with_suffix}\\g<3>',
            content,
            count=1,
            flags=re.DOTALL,
        )
        if not validate_appx_manifest_version_invariants(new_content, file_path):
            return False

        if dry_run:
            print(f"  [DRY RUN] Would update: {file_path} ({old_version} -> {version_with_suffix})")
        else:
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

    # Get project root (go up from scripts/ directory)
    script_dir = Path(__file__).parent.parent.resolve()

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
    if update_package_lock(script_dir / 'package-lock.json', new_version, dry_run):
        success_count += 1

    # 4. src-tauri/Cargo.toml
    total_count += 1
    if update_cargo_toml(script_dir / 'src-tauri' / 'Cargo.toml', new_version, dry_run):
        success_count += 1
        refresh_cargo_lock(script_dir / 'src-tauri', dry_run)

    # 5. src-tauri/tauri.conf.json
    total_count += 1
    if update_json_file(script_dir / 'src-tauri' / 'tauri.conf.json', new_version, dry_run):
        success_count += 1

    # 6. AppxManifest.xml
    total_count += 1
    if update_appx_manifest(script_dir / 'AppxManifest.xml', new_version, dry_run):
        success_count += 1

    # 7. src/components/AboutDialog.tsx - Version X.Y.Z
    total_count += 1
    if update_tsx_file(
        script_dir / 'src' / 'components' / 'AboutDialog.tsx',
        new_version,
        [(r'Version\s+[\d.]+', 'Version VERSION')],
        dry_run
    ):
        success_count += 1

    # 8. src/App.tsx - APP_VERSION constant (rendered in the nav rail and status bar)
    total_count += 1
    if update_tsx_file(
        script_dir / 'src' / 'App.tsx',
        new_version,
        # Capture groups keep the identifier out of the replacement text —
        # a literal "APP_VERSION = '...'" template would have its own
        # VERSION substring rewritten by the placeholder substitution.
        [(r"(APP_VERSION = ')[\d.]+(')", r"\g<1>VERSION\g<2>")],
        dry_run
    ):
        success_count += 1

    # 9. src-tauri/tauri.msix.conf.json - nested msixVersion (X.Y.Z.0)
    total_count += 1
    if update_msix_conf(script_dir / 'src-tauri' / 'tauri.msix.conf.json', new_version, dry_run):
        success_count += 1

    # 10. README.md
    total_count += 1
    if update_tsx_file(
        script_dir / 'README.md',
        new_version,
        [
            (r'# WF Diagnostics v[\d.]+', '# WF Diagnostics vVERSION'),
            (r'badge/version-[\d.]+-blue\.svg', 'badge/version-VERSION-blue.svg'),
            (r'v[\d.]+ \(Current\)', 'vVERSION (Current)')
        ],
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
