"""External-gate watcher for the windows-reactor migration.

Read-only checks over the three gates that cannot be closed from this
repository alone, so drift is detected the day it happens instead of at
cutover review:

1. crates.io watch — has `windows-reactor` published a real (non-placeholder)
   release? Any version above 0.0.0 makes `cutover.official_reactor_release`
   actionable.
2. Runtime drift — Store manifest `PackageDependency` vs the Reactor
   staging pin (`Microsoft.WindowsAppRuntime.2` / 2.4.0) vs the frameworks
   installed on this host (when -HostFrameworks is supplied).
3. Packaging pre-flight — presence of the artifacts and manifests the
   clean-machine protocol (docs/validation/clean-machine-protocol.md) needs.

Exit codes: 0 = no actionable external change, 1 = an actionable external
change was detected (e.g. an official release published) or an alignment
input drifted (store manifest vs the pinned framework/floor, missing
protocol inputs), 2 = check failure.
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
PLACEHOLDER_VERSION = "0.0.0"


def _reactor_pin() -> dict:
    # The adopted pin is single-sourced from reactor-baselines/manifest.json
    # like every other consumer of reactor_pin. A hard-coded copy here made
    # the watcher report the adopted release itself as "newer than adopted"
    # forever after any pin move that followed CLAUDE.md's checklist, which
    # named the probe script (now manifest-driven) but not this one (#331).
    baseline = Path(__file__).resolve().parent.parent / "reactor-baselines" / "manifest.json"
    try:
        return json.loads(baseline.read_text(encoding="utf-8"))["reactor_pin"]
    except (OSError, json.JSONDecodeError, KeyError) as error:
        raise SystemExit(f"cannot read reactor pin from {baseline}: {error}") from error


_PIN = _reactor_pin()
ADOPTED_REACTOR_VERSION = _PIN["expected_crate_version"]
EXPECTED_RUNTIME_FRAMEWORK = _PIN["windows_app_runtime_framework"]
EXPECTED_RUNTIME_RELEASE = _PIN["windows_app_runtime_release"]


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
    # 0.100.0 is ADOPTED as the dependency pin (2026-09-03), so only a
    # release *newer* than the adopted one is actionable; the placeholder
    # and the adopted version itself are clear.
    def newer_than_adopted(version: str) -> bool:
        def key(value: str) -> list[int]:
            return [int(part) if part.isdigit() else 0 for part in value.split(".")]
        return key(version) > key(ADOPTED_REACTOR_VERSION)

    newer = sorted(
        (version for version in versions if version != PLACEHOLDER_VERSION
         and newer_than_adopted(version)),
        key=lambda value: [int(part) if part.isdigit() else 0
                           for part in value.split(".")],
    )
    if newer:
        return {
            "check": "crates_io",
            "status": "actionable",
            "message": (f"windows-reactor has published versions newer than the "
                        f"adopted {ADOPTED_REACTOR_VERSION}: {newer}. "
                        f"Review the release and update the dependency pin."),
            "versions": newer,
        }
    return {
        "check": "crates_io",
        "status": "clear",
        "message": (f"windows-reactor {ADOPTED_REACTOR_VERSION} is the adopted "
                    f"release; nothing newer published."),
    }


def read_runtime_pins(root: Path) -> dict:
    # The framework name and floor are single-sourced from
    # reactor-baselines/manifest.json (reactor_pin); the old prototype-scan
    # loop assigned the constant it already held, so nothing read from
    # apps/wfdiag/Cargo.toml could change any outcome (2026-09-03 audit).
    baseline_path = root / "reactor-baselines" / "manifest.json"
    store_manifest = root / "AppxManifest.xml"
    pins = {
        "baseline_loaded": False,
        "store_framework": None,
        "store_min_version": None,
        "reactor_framework": EXPECTED_RUNTIME_FRAMEWORK,
        "reactor_release": EXPECTED_RUNTIME_RELEASE,
        "reactor_min_version": None,
    }
    try:
        pin = json.loads(baseline_path.read_text(encoding="utf-8"))["reactor_pin"]
        pins["reactor_framework"] = pin["windows_app_runtime_framework"]
        pins["reactor_release"] = pin["windows_app_runtime_release"]
        pins["reactor_min_version"] = pin["windows_app_runtime_min_version"]
        pins["baseline_loaded"] = True
    except (OSError, json.JSONDecodeError, KeyError):
        pass
    if store_manifest.is_file():
        try:
            tree = ET.parse(store_manifest)
        except ET.ParseError:
            return pins
        for dependency in tree.getroot().iter():
            if dependency.tag.endswith("PackageDependency"):
                name = dependency.get("Name") or ""
                if name.startswith("Microsoft.WindowsAppRuntime."):
                    pins["store_framework"] = name
                    pins["store_min_version"] = dependency.get("MinVersion")
    return pins


def check_runtime_drift(root: Path, host_frameworks: list[str] | None) -> dict:
    pins = read_runtime_pins(root)
    if not pins["baseline_loaded"]:
        return {
            "check": "runtime_alignment",
            "status": "error",
            "message": "reactor-baselines/manifest.json is missing or unreadable; "
            "the runtime alignment pin cannot be checked.",
            "pins": pins,
        }
    drift = (
        pins["store_framework"] != pins["reactor_framework"]
        or pins["store_min_version"] != pins["reactor_min_version"]
    )
    report = {
        "check": "runtime_alignment",
        "status": "drift" if drift else "aligned",
        "message": (
            f"Store manifest pins {pins['store_framework']}"
            f" MinVersion {pins['store_min_version']} while the Reactor staging "
            f"targets {pins['reactor_framework']} MinVersion "
            f"{pins['reactor_min_version']} ({pins['reactor_release']})."
            if drift else
            f"Store manifest and Reactor staging agree on {pins['reactor_framework']}"
            f" MinVersion {pins['reactor_min_version']}."
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
        "apps/wfdiag/Cargo.toml",
        "apps/wfdiag/build.rs",
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
    # Drift and incomplete protocol inputs are the watcher's reason to
    # exist; they used to print and exit 0 (2026-09-03 audit).
    drifted = [
        check for check in checks if check["status"] in ("drift", "incomplete")
    ]
    return 1 if actionable or drifted else 0


if __name__ == "__main__":
    sys.exit(main())
