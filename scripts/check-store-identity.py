#!/usr/bin/env python3
"""Verify Store package-identity fields agree across every manifest source.

Regression guard for the bug fixed in 151c46c: publisherDisplayName drifted
to "Mike Fara" in some of these sources while staying correct in others (all
silently, buried inside an unrelated 50-file commit), and Partner Center
rejected the next fresh package upload three weeks later. Nothing caught the
drift in between because screenshot-only Store updates reuse the existing
package, and automated publish attempts kept failing on Partner Center API
timeouts before ever reaching real validation. Run in CI on every push so
any future drift on these fields fails fast and loud instead of silently
blocking the next Store submission.

The six sources checked:
  - src-tauri/tauri.msix.conf.json  (local build-cross.py Store MSIX path)
  - AppxManifest.xml                (Store MSIX package manifest)
  - scripts/build-cross.py          (Python-generated Store and dev-only
                                      sparse manifests)
  - src-tauri/windows-app.manifest  (loose executable's sparse association)
  - .github/workflows/build-and-publish-store.yml (CI's inline manifest)

The sparse-identity package must use the Store identity because the LAF token
is bound to the full Store package family name.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

EXPECTED_PUBLISHER_DISPLAY_NAME = "WindowsForum.com"
EXPECTED_IDENTITY_NAME = "32827MikeFara.WindowsForumDiagnostics"
EXPECTED_PUBLISHER_CN = "CN=ABDB6B3F-DF9E-447D-BC0E-4DA7BAFD14C4"

errors: list[str] = []


def check(label: str, actual: str | None, expected: str) -> None:
    if actual != expected:
        errors.append(f"{label}: expected {expected!r}, got {actual!r}")


def find_all(label: str, pattern: str, text: str, expected: str) -> None:
    matches = re.findall(pattern, text, re.MULTILINE)
    if not matches:
        errors.append(f"{label}: pattern not found ({pattern!r})")
        return
    for i, actual in enumerate(matches):
        check(f"{label} [{i}]" if len(matches) > 1 else label, actual, expected)


# --- src-tauri/tauri.msix.conf.json ---
msix_conf_path = ROOT / "src-tauri/tauri.msix.conf.json"
msix_conf = json.loads(msix_conf_path.read_text())
bundle = msix_conf["bundle"]
msix = bundle["windows"]["msix"]
check(f"{msix_conf_path.name} bundle.publisher", bundle["publisher"], EXPECTED_PUBLISHER_CN)
check(f"{msix_conf_path.name} msix.publisherDisplayName", msix["publisherDisplayName"], EXPECTED_PUBLISHER_DISPLAY_NAME)
check(f"{msix_conf_path.name} msix.identityName", msix["identityName"], EXPECTED_IDENTITY_NAME)

# --- AppxManifest.xml ---
appx_path = ROOT / "AppxManifest.xml"
appx_text = appx_path.read_text()
find_all(f"{appx_path.name} Identity Publisher", r'<Identity\b[^>]*\bPublisher="([^"]+)"', appx_text, EXPECTED_PUBLISHER_CN)
find_all(f"{appx_path.name} Identity Name", r'<Identity\b[^>]*\bName="([^"]+)"', appx_text, EXPECTED_IDENTITY_NAME)
find_all(f"{appx_path.name} PublisherDisplayName", r"<PublisherDisplayName>([^<]+)</PublisherDisplayName>", appx_text, EXPECTED_PUBLISHER_DISPLAY_NAME)

# --- scripts/build-cross.py ---
build_cross_path = ROOT / "scripts/build-cross.py"
build_cross_text = build_cross_path.read_text()
find_all(f"{build_cross_path.name} PUBLISHER constant", r'^PUBLISHER = "([^"]+)"', build_cross_text, EXPECTED_PUBLISHER_CN)
find_all(f"{build_cross_path.name} SPARSE_PACKAGE_NAME constant", r'^SPARSE_PACKAGE_NAME = "([^"]+)"', build_cross_text, EXPECTED_IDENTITY_NAME)
find_all(f"{build_cross_path.name} PublisherDisplayName", r"<PublisherDisplayName>([^<]+)</PublisherDisplayName>", build_cross_text, EXPECTED_PUBLISHER_DISPLAY_NAME)
appx_manifest_fn = re.search(r"def create_appx_manifest\(.*?\n(?=def |\Z)", build_cross_text, re.S)
if not appx_manifest_fn:
    errors.append(f"{build_cross_path.name}: create_appx_manifest() not found")
else:
    find_all(f"{build_cross_path.name} create_appx_manifest Identity Name", r'<Identity\b[^>]*\bName="([^"]+)"', appx_manifest_fn.group(0), EXPECTED_IDENTITY_NAME)

# --- src-tauri/windows-app.manifest ---
windows_manifest_path = ROOT / "src-tauri/windows-app.manifest"
windows_manifest_text = windows_manifest_path.read_text()
find_all(f"{windows_manifest_path.name} msix publisher", r'<msix\b[^>]*\bpublisher="([^"]+)"', windows_manifest_text, EXPECTED_PUBLISHER_CN)
find_all(f"{windows_manifest_path.name} msix packageName", r'<msix\b[^>]*\bpackageName="([^"]+)"', windows_manifest_text, EXPECTED_IDENTITY_NAME)

# --- .github/workflows/build-and-publish-store.yml ---
ci_manifest_path = ROOT / ".github/workflows/build-and-publish-store.yml"
ci_text = ci_manifest_path.read_text()
find_all(f"{ci_manifest_path.name} PUBLISHER env", r"^\s*PUBLISHER:\s*(\S+)", ci_text, EXPECTED_PUBLISHER_CN)
find_all(f"{ci_manifest_path.name} PublisherDisplayName", r"<PublisherDisplayName>([^<]+)</PublisherDisplayName>", ci_text, EXPECTED_PUBLISHER_DISPLAY_NAME)
find_all(f"{ci_manifest_path.name} Identity Name", r'<Identity\b[^>]*\bName="([^"]+)"', ci_text, EXPECTED_IDENTITY_NAME)

if errors:
    print("Store package-identity mismatch detected:\n")
    for e in errors:
        print(f"  - {e}")
    print(
        f"\nAll manifest sources must agree on:\n"
        f"  publisherDisplayName = {EXPECTED_PUBLISHER_DISPLAY_NAME!r}\n"
        f"  Identity Name        = {EXPECTED_IDENTITY_NAME!r}\n"
        f"  Publisher CN         = {EXPECTED_PUBLISHER_CN!r}\n"
        f"\nIf this is an intentional rebrand/re-registration, update the\n"
        f"EXPECTED_* constants at the top of scripts/check-store-identity.py\n"
        f"to match, then update every source listed above to agree."
    )
    sys.exit(1)

print("Store package-identity fields agree across all manifest sources.")
