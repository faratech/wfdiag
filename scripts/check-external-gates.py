"""External-gate watcher for the windows-reactor migration.

Read-only checks over the three gates that cannot be closed from this
repository alone, so drift is detected the day it happens instead of at
cutover review:

1. crates.io watch — has `windows-reactor` published a real (non-placeholder)
   release? Any version above 0.0.0 makes `cutover.official_reactor_release`
   actionable.
2. Runtime drift — Store manifest `PackageDependency` (1.8 line) vs the Reactor
   staging pin (`Microsoft.WindowsAppRuntime.2` / 2.4.0) vs the frameworks
   installed on this host (when -HostFrameworks is supplied).
3. Packaging pre-flight — presence of the artifacts and manifests the
   clean-machine protocol (docs/validation/clean-machine-protocol.md) needs.

Exit codes: 0 = no actionable external change, 1 = an actionable external
change was detected (e.g. an official release published), 2 = check failure.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

CRATES_IO_URL = "https://crates.io/api/v1/crates/windows-reactor"
EXPECTED_REACTOR_VERSION = "0.100.0"
EXPECTED_RUNTIME_FRAMEWORK = "Microsoft.WindowsAppRuntime.2"
EXPECTED_RUNTIME_RELEASE = "2.4.0"
PLACEHOLDER_VERSION = "0.0.0"


def check_crates_io(timeout: float) -> dict:
    request = urllib.request.Request(
        CRATES_IO_URL,
        headers={"User-Agent": "wfdiag-external-gate-watcher (repo validation)"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - report, do not crash the watcher
        return {"check": "crates_io", "status": "error",
                "message": f"crates.io query failed: {error}"}

    versions = [
        (entry.get("num") or "")
        for entry in (payload.get("versions") or [])
        if not entry.get("yanked")
    ]
    real = [version for version in versions if version != PLACEHOLDER_VERSION]
    real.sort(key=lambda value: [int(part) if part.isdigit() else 0
                                 for part in value.split(".")])
    if real:
        return {
            "check": "crates_io",
            "status": "actionable",
            "message": (f"windows-reactor has published non-placeholder versions "
                        f"{real}; the pinned prototype expects {EXPECTED_REACTOR_VERSION}. "
                        f"Review the release and make cutover.official_reactor_release actionable."),
            "versions": real,
        }
    return {
        "check": "crates_io",
        "status": "clear",
        "message": "windows-reactor is still the placeholder 0.0.0 release.",
    }


def read_runtime_pins(root: Path) -> dict:
    store_manifest = root / "AppxManifest.xml"
    prototype = root / "reactor-spike" / "Cargo.toml"
    pins = {
        "store_framework": None,
        "reactor_framework": EXPECTED_RUNTIME_FRAMEWORK,
        "reactor_release": EXPECTED_RUNTIME_RELEASE,
    }
    if store_manifest.is_file():
        tree = ET.parse(store_manifest)
        namespace = {"default": "http://schemas.microsoft.com/appx/manifest/foundation/windows10"}
        for dependency in tree.getroot().iter():
            if dependency.tag.endswith("PackageDependency"):
                name = dependency.get("Name") or ""
                if name.startswith("Microsoft.WindowsAppRuntime."):
                    pins["store_framework"] = name
    if prototype.is_file():
        text = prototype.read_text(encoding="utf-8")
        for framework in ("Microsoft.WindowsAppRuntime.1", "Microsoft.WindowsAppRuntime.2"):
            if framework in text:
                pins["reactor_framework"] = EXPECTED_RUNTIME_FRAMEWORK
    return pins


def check_runtime_drift(root: Path, host_frameworks: list[str] | None) -> dict:
    pins = read_runtime_pins(root)
    drift = pins["store_framework"] != pins["reactor_framework"]
    report = {
        "check": "runtime_alignment",
        "status": "drift" if drift else "aligned",
        "message": (
            f"Store manifest pins {pins['store_framework']} while the Reactor staging "
            f"targets {pins['reactor_framework']} ({pins['reactor_release']})."
            if drift else
            f"Store manifest and Reactor staging agree on {pins['reactor_framework']}."
        ),
        "pins": pins,
    }
    if host_frameworks:
        report["hostFrameworks"] = host_frameworks
        has_two = any(name.startswith(EXPECTED_RUNTIME_FRAMEWORK)
                      for name in host_frameworks)
        report["hostHasReactorFramework"] = has_two
        report["message"] += (
            " Host has the Reactor framework installed."
            if has_two else
            " Host does NOT have the Reactor framework installed; framework-dependent candidates will fail to launch.")
    return report


def check_packaging_pre_flight(root: Path) -> dict:
    expected = [
        "AppxManifest.xml",
        "reactor-spike/Cargo.toml",
        "reactor-spike/build.rs",
        "docs/validation/clean-machine-protocol.md",
    ]
    missing = [name for name in expected if not (root / name).is_file()]
    return {
        "check": "packaging_pre_flight",
        "status": "incomplete" if missing else "ready",
        "message": ("Clean-machine protocol inputs present."
                    if not missing else
                    f"Missing protocol inputs: {missing}"),
        "missing": missing,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    parser.add_argument("--timeout", type=float, default=15.0,
                        help="crates.io request timeout in seconds")
    parser.add_argument("--skip-network", action="store_true",
                        help="skip the crates.io query (offline runs)")
    parser.add_argument("--host-frameworks", nargs="*",
                        help="Installed Microsoft.WindowsAppRuntime* package names")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    checks = []
    if not args.skip_network:
        checks.append(check_crates_io(args.timeout))
    checks.append(check_runtime_drift(root, args.host_frameworks))
    checks.append(check_packaging_pre_flight(root))

    actionable = [check for check in checks if check["status"] == "actionable"]
    errors = [check for check in checks if check["status"] == "error"]
    report = {
        "actionable": actionable,
        "checks": checks,
        "summary": f"{len(actionable)} actionable, {len(errors)} errors, {len(checks)} checks",
    }

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for check in checks:
            print(f"[{check['status']}] {check['check']}: {check['message']}")
        print(f"external gates: {report['summary']}")

    if errors:
        return 2
    return 1 if actionable else 0


if __name__ == "__main__":
    sys.exit(main())
