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



def _windows_app_runtime_pin(script_dir: Path) -> dict[str, str]:
    """Single source for the Windows App Runtime framework pin.

    reactor-baselines/manifest.json (reactor_pin) is read by this script, by
    scripts/check-reactor-readiness.py, and by scripts/check-external-gates.py,
    so the manifest, the readiness gate, and the drift watcher can never
    disagree about which framework line the Store package depends on.
    """
    manifest = json.loads((script_dir / "reactor-baselines" / "manifest.json").read_text(encoding="utf-8"))
    pin = manifest["reactor_pin"]
    return {pin["windows_app_runtime_framework"]: pin["windows_app_runtime_min_version"]}


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
            if new_content == content:
                raise ValueError(
                    f"version pattern did not match in {file_path} - "
                    "the file was left unchanged"
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


def refresh_cargo_lock(cargo_dir: Path, dry_run: bool) -> bool:
    """Re-sync Cargo.lock's own-package version entry after Cargo.toml's
    [package].version changes — otherwise the next build leaves the tree
    dirty (or fails outright under --locked)."""
    if dry_run:
        print("  [DRY RUN] Would refresh Cargo.lock to match the new version")
        return True
    try:
        subprocess.run(
            ["cargo", "update", "--offline", "-p", "wfdiag"],
            cwd=cargo_dir,
            check=True,
            capture_output=True,
            text=True,
        )
        print("  Refreshed: Cargo.lock")
        return True
    except FileNotFoundError:
        print("  Warning: cargo not found on PATH — Cargo.lock was not refreshed; run "
              "'cargo update -p wfdiag' manually before committing")
        return False
    except subprocess.CalledProcessError as e:
        print(f"  Warning: failed to refresh Cargo.lock: {e.stderr.strip() if e.stderr else e}")
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
    for name, expected in _windows_app_runtime_pin(file_path.resolve().parent).items():
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
        unmatched = []

        for pattern, replacement in patterns:
            if re.search(pattern, content):
                if not dry_run:
                    content = re.sub(pattern, replacement.replace('VERSION', new_version), content)
            else:
                unmatched.append(pattern)

        # A file counted as updated when ANY pattern matched left stale
        # versions behind with a false pass (2026-09-03 audit): every
        # pattern must match, so a renamed heading or string fails loudly
        # instead of shipping an old version in one spot.
        if unmatched:
            print(f"  Warning: version patterns not found in {file_path}: {unmatched}")
            return False

        if patterns:
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
    if not re.fullmatch(r'\d+\.\d+\.\d+', new_version):
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

    # 2. apps/wfdiag/Cargo.toml (native shell) shares the app version with
    #    version.json; one lock refresh keeps Cargo.lock in step.
    total_count += 1
    shell_updated = update_cargo_toml(script_dir / 'apps' / 'wfdiag' / 'Cargo.toml', new_version, dry_run)
    success_count += int(shell_updated)
    if shell_updated:
        # Counted like every other write: a stale lock must fail the run,
        # or the next --locked build rejects the bump after a green exit.
        total_count += 1
        if refresh_cargo_lock(script_dir, dry_run):
            success_count += 1

    # 3. AppxManifest.xml
    total_count += 1
    if update_appx_manifest(script_dir / 'AppxManifest.xml', new_version, dry_run):
        success_count += 1

    # 4. README.md
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
        print("  2. Commit changes with explicit pathspecs (never 'git add -A'):")
        print(f"     git commit -m 'Bump version to {new_version}' -- version.json Cargo.lock apps/wfdiag/Cargo.toml AppxManifest.xml README.md")
        
    sys.exit(0 if success_count == total_count else 1)


if __name__ == '__main__':
    main()
