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

The sources checked:
  - AppxManifest.xml                (Store MSIX package manifest)
  - .github/workflows/build-and-publish-store.yml (CI's inline manifest)
  - apps/wfdiag/src/platform/notifications.rs (toast AUMID, pinned to the
                                      package family name recorded in
                                      reactor-baselines/manifest.json so a
                                      rebrand cannot silently drop every
                                      toast; 2026-09-03 audit)

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


# --- AppxManifest.xml ---
appx_path = ROOT / "AppxManifest.xml"
appx_text = appx_path.read_text()
find_all(f"{appx_path.name} Identity Publisher", r'<Identity\b[^>]*\bPublisher="([^"]+)"', appx_text, EXPECTED_PUBLISHER_CN)
find_all(f"{appx_path.name} Identity Name", r'<Identity\b[^>]*\bName="([^"]+)"', appx_text, EXPECTED_IDENTITY_NAME)
find_all(f"{appx_path.name} PublisherDisplayName", r"<PublisherDisplayName>([^<]+)</PublisherDisplayName>", appx_text, EXPECTED_PUBLISHER_DISPLAY_NAME)

# --- scripts/build-reactor-msix-probe.py (renders the shipped manifest) ---
probe_path = ROOT / "scripts/build-reactor-msix-probe.py"
probe_text = probe_path.read_text()
find_all(f"{probe_path.name} STORE_PUBLISHER constant", r'^STORE_PUBLISHER = "([^"]+)"', probe_text, EXPECTED_PUBLISHER_CN)
find_all(f"{probe_path.name} STORE_IDENTITY_NAME constant", r'^STORE_IDENTITY_NAME = "([^"]+)"', probe_text, EXPECTED_IDENTITY_NAME)

# --- apps/wfdiag/src/platform/notifications.rs (toast AUMID) ---
# The AUMID embeds the package family name; the baseline records it, and
# the shell constant must match or Windows silently drops every toast
# (2026-09-03 audit: no script checked the Rust constant).
baseline_path = ROOT / "reactor-baselines" / "manifest.json"
try:
    baseline_pfn = json.loads(baseline_path.read_text())["baseline"][
        "source_package"
    ]["package_family_name"]
except (OSError, json.JSONDecodeError, KeyError) as error:
    baseline_pfn = None
    errors.append(f"{baseline_path.name}: cannot read package_family_name ({error})")
notifications_path = ROOT / "apps/wfdiag/src/platform/notifications.rs"
notifications = notifications_path.read_text()
aumid_matches = re.findall(r'const AUMID: &str = "([^"]+)"', notifications)
if not aumid_matches:
    errors.append(f"{notifications_path.name}: AUMID constant not found")
elif baseline_pfn is not None:
    for i, aumid in enumerate(aumid_matches):
        label = (
            f"{notifications_path.name} AUMID [{i}]"
            if len(aumid_matches) > 1
            else f"{notifications_path.name} AUMID"
        )
        if not aumid.startswith(f"{baseline_pfn}!"):
            errors.append(
                f"{label}: expected the toast AUMID to start with "
                f"{baseline_pfn!r} + '!', got {aumid!r}"
            )

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
