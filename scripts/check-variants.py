"""Validate reactor-baselines/variants.json and the captures it references.

The variants document tracks rendering-parity captures outside the main
readiness manifest: theme and motion variants over deterministic fixture
states, plus the open rendering-defect list (seeded with the owner-reported
process-list refresh divergence).

Read-only: exits 0 when every record is structurally valid, references an
existing PNG whose SHA-256 matches, and covers the required default set.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

REQUIRED_DEFAULT_STATES = (
    "diagnostics-populated-dark-normal",
    "diagnostics-populated-light-normal",
    "monitor-empty-dark-normal",
    "monitor-empty-light-normal",
    "processes-empty-dark-normal",
    "processes-empty-light-normal",
    "settings-bottom-dark-normal",
    "settings-bottom-light-normal",
)

REQUIRED_DEFECT_IDS = ("processes-refresh-rendering",)


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def evaluate(document: dict, root: Path) -> list[dict]:
    findings: list[dict] = []

    def finding(severity: str, code: str, message: str) -> None:
        findings.append(
            {"severity": severity, "code": code, "message": message})

    if document.get("schema") != 1:
        finding("error", "variants.schema",
                f"variants schema must be 1, found {document.get('schema')!r}")

    defects = document.get("defects") or []
    defect_ids = {entry.get("id") for entry in defects}
    for required in REQUIRED_DEFECT_IDS:
        if required not in defect_ids:
            finding("error", "variants.defects",
                    f"variants document is missing the required defect '{required}'")

    variants = document.get("variants") or []
    seen: dict[str, dict] = {}
    for record in variants:
        identifier = record.get("id")
        if not identifier:
            finding("error", "variants.record", "a variant record has no id")
            continue
        if identifier in seen:
            finding("error", "variants.record",
                    f"duplicate variant id '{identifier}'")
            continue
        seen[identifier] = record

        png = record.get("png")
        expected_hash = record.get("sha256")
        if not png or not expected_hash:
            finding("error", "variants.capture",
                    f"variant '{identifier}' is missing its png path or sha256")
            continue
        png_path = root / png
        if not png_path.is_file():
            finding("error", "variants.capture",
                    f"variant '{identifier}' references a missing file: {png}")
            continue
        actual = sha256_of(png_path)
        if actual != expected_hash:
            finding("error", "variants.capture",
                    f"variant '{identifier}' sha256 mismatch ({expected_hash} != {actual})")

        for field in ("theme", "state", "applicationVersion"):
            if not record.get(field):
                finding("error", "variants.record",
                        f"variant '{identifier}' is missing '{field}'")

    for required in REQUIRED_DEFAULT_STATES:
        if required not in seen:
            finding("blocker", "variants.coverage",
                    f"the required default variant '{required}' has not been captured")

    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default="reactor-baselines/variants.json")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    manifest_path = Path(args.manifest)
    root = manifest_path.parent.parent
    if not manifest_path.is_file():
        print(json.dumps({
            "ready": False,
            "findings": [{"severity": "error", "code": "variants.manifest",
                          "message": f"variants manifest not found: {manifest_path}"}],
        }, indent=2))
        return 1

    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    findings = evaluate(document, root)
    blockers = sum(1 for item in findings if item["severity"] == "blocker")
    errors = sum(1 for item in findings if item["severity"] == "error")
    report = {
        "ready": blockers == 0 and errors == 0,
        "counts": {
            "blocker": blockers,
            "error": errors,
            "variant": len(document.get("variants") or []),
            "defect": len(document.get("defects") or []),
        },
        "findings": findings,
    }
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for item in findings:
            print(f"{item['severity']}: {item['code']}: {item['message']}")
        state = "READY" if report["ready"] else "NOT READY"
        print(f"variants: {state} "
              f"({report['counts']['variant']} variants, "
              f"{blockers} blockers, {errors} errors)")
    return 0 if report["ready"] else 1


if __name__ == "__main__":
    sys.exit(main())
