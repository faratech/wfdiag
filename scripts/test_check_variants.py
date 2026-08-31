"""Tests for scripts/check-variants.py (variant document evaluation)."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


def _load_module():
    spec = importlib.util.spec_from_file_location(
        "check_variants", Path(__file__).parent / "check-variants.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _write_capture(directory: Path, name: str, payload: bytes = b"png") -> dict:
    path = directory / f"{name}.png"
    path.write_bytes(payload)
    return {
        "id": name,
        "theme": "dark",
        "state": "monitor-empty",
        "applicationVersion": "2.5.8",
        "png": str(path),
        "sha256": hashlib.sha256(payload).hexdigest().upper(),
    }


class CheckVariantsTest(unittest.TestCase):
    def setUp(self):
        self.module = _load_module()
        self.directory = Path(tempfile.mkdtemp())

    def test_valid_document_is_ready(self):
        capture = _write_capture(self.directory,
                                 "diagnostics-populated-dark-normal")
        document = {
            "schema": 1,
            "defects": [{"id": "processes-refresh-rendering"}],
            "variants": [capture],
        }
        # Only the default-coverage blocker should fire for a single capture.
        findings = self.module.evaluate(document, self.directory)
        severities = [finding["severity"] for finding in findings]
        self.assertNotIn("error", severities)
        self.assertEqual(
            len(self.module.REQUIRED_DEFAULT_STATES) - 1,
            sum(1 for finding in findings if finding["severity"] == "blocker"),
        )

    def test_missing_file_is_an_error(self):
        capture = _write_capture(self.directory, "ghost")
        capture["png"] = str(self.directory / "ghost-missing.png")
        document = {"schema": 1, "defects": [], "variants": [capture]}
        findings = self.module.evaluate(document, self.directory)
        self.assertTrue(
            any(f["severity"] == "error" and "missing file" in f["message"]
                for f in findings))

    def test_hash_mismatch_is_an_error(self):
        capture = _write_capture(self.directory, "broken")
        capture["sha256"] = "0" * 64
        document = {"schema": 1, "defects": [], "variants": [capture]}
        findings = self.module.evaluate(document, self.directory)
        self.assertTrue(
            any(f["severity"] == "error" and "sha256 mismatch" in f["message"]
                for f in findings))

    def test_wrong_schema_is_an_error(self):
        findings = self.module.evaluate({"schema": 2, "defects": [], "variants": []},
                                        self.directory)
        self.assertTrue(
            any(f["code"] == "variants.schema" for f in findings))

    def test_required_defect_must_be_tracked(self):
        findings = self.module.evaluate({"schema": 1, "defects": [], "variants": []},
                                        self.directory)
        self.assertTrue(
            any(f["code"] == "variants.defects" and
                "processes-refresh-rendering" in f["message"] for f in findings))


if __name__ == "__main__":
    sys.exit(unittest.main())
