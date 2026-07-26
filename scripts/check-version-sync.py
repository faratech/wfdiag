#!/usr/bin/env python3
"""Fail when release/package/UI version sources disagree."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_json(path: str) -> dict:
    return json.loads((ROOT / path).read_text(encoding="utf-8"))


def extract(path: str, pattern: str, label: str) -> str:
    text = (ROOT / path).read_text(encoding="utf-8")
    match = re.search(pattern, text, re.MULTILINE)
    if not match:
        raise ValueError(f"{label}: version marker not found in {path}")
    return match.group(1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", help="Expected tag/manual release version")
    args = parser.parse_args()

    canonical = str(read_json("version.json")["version"])
    if not re.fullmatch(r"\d+\.\d+\.\d+", canonical):
        print(f"ERROR: version.json contains invalid SemVer {canonical!r}", file=sys.stderr)
        return 1
    package_lock = read_json("package-lock.json")
    cargo_toml = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
    cargo_lock = tomllib.loads((ROOT / "src-tauri/Cargo.lock").read_text(encoding="utf-8"))
    appx = (ROOT / "AppxManifest.xml").read_text(encoding="utf-8")
    appx_match = re.search(r'<Identity\b[^>]*\bVersion="([^"]+)"', appx, re.DOTALL)
    if not appx_match:
        print("ERROR: AppxManifest.xml has no Identity Version", file=sys.stderr)
        return 1

    cargo_lock_version = next(
        (
            package["version"]
            for package in cargo_lock.get("package", [])
            if package.get("name") == "wfdiag-tauri"
        ),
        None,
    )
    msix_version = read_json("src-tauri/tauri.msix.conf.json")["bundle"]["windows"]["msix"]["msixVersion"]
    appx_version = appx_match.group(1)
    expected_windows_version = f"{canonical}.0"
    windows_versions = {
        "src-tauri/tauri.msix.conf.json": msix_version,
        "AppxManifest.xml": appx_version,
    }
    invalid_windows_versions = {
        name: value
        for name, value in windows_versions.items()
        if value != expected_windows_version
    }
    if invalid_windows_versions:
        print(
            f"ERROR: Windows package versions must equal {expected_windows_version}",
            file=sys.stderr,
        )
        for name, value in invalid_windows_versions.items():
            print(f"  {name}: {value}", file=sys.stderr)
        return 1

    sources = {
        "version.json": canonical,
        "package.json": read_json("package.json")["version"],
        "package-lock.json (root)": package_lock["version"],
        "package-lock.json": package_lock["packages"][""]["version"],
        "src-tauri/Cargo.toml": cargo_toml["package"]["version"],
        "src-tauri/Cargo.lock": cargo_lock_version,
        "src-tauri/tauri.conf.json": read_json("src-tauri/tauri.conf.json")["version"],
        "src/App.tsx": extract(
            "src/App.tsx", r"const\s+APP_VERSION\s*=\s*['\"]([\d.]+)['\"]", "App UI"
        ),
        "src/components/AboutDialog.tsx": extract(
            "src/components/AboutDialog.tsx", r"Version\s+([\d.]+)", "About dialog"
        ),
        "README.md (heading)": extract(
            "README.md", r"^#\s+WF Diagnostics v([\d.]+)\b", "README heading"
        ),
        "README.md (badge)": extract(
            "README.md", r"badge/version-([\d.]+)-blue\.svg", "README badge"
        ),
        "README.md (current release)": extract(
            "README.md", r"^###\s+\*\*v([\d.]+) \(Current\)", "README current release"
        ),
    }
    if args.expected:
        if not re.fullmatch(r"\d+\.\d+\.\d+", args.expected):
            print(
                f"ERROR: requested/tag version must be X.Y.Z without a leading v; got {args.expected!r}",
                file=sys.stderr,
            )
            return 1
        sources["requested/tag version"] = args.expected

    mismatches = {name: value for name, value in sources.items() if value != canonical}
    if mismatches:
        print(f"ERROR: expected every version source to equal {canonical}", file=sys.stderr)
        for name, value in mismatches.items():
            print(f"  {name}: {value}", file=sys.stderr)
        return 1

    print(f"Version sources are synchronized at {canonical}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
