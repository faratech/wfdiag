"""Regression tests for the Reactor PowerShell validation orchestrators.

These are intentionally host-independent contract checks.  The UIA scripts
still receive a real Windows parser/run in the live validation lane, while
these tests catch argument-routing and destructive-default regressions on any
development host.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).parent


def _source(name: str) -> str:
    return (SCRIPTS / name).read_text(encoding="utf-8")


class ValidationHarnessContractTest(unittest.TestCase):
    def test_x64_scan_timeout_is_only_routed_to_scan_suites(self):
        source = _source("test-reactor-x64.ps1")
        self.assertIn('@("report", "remediation") -contains $suite', source)
        self.assertNotIn('if ($suite -ne "live-system")', source)

    def test_all_expands_to_every_cutover_suite(self):
        source = _source("validate-reactor.ps1")
        expansion = re.search(
            r'if \(\$Suite -contains "all"\) \{\s*\$Suite = @\((.*?)\)\s*\}',
            source,
            flags=re.DOTALL,
        )
        self.assertIsNotNone(expansion)
        values = set(re.findall(r'"([a-z0-9-]+)"', expansion.group(1)))
        self.assertEqual(
            values,
            {
                "startup",
                "live-system",
                "about",
                "flows",
                "visual",
                "x64",
                "readiness",
                "gates",
            },
        )

    def test_visual_validation_uses_only_report_local_manifest(self):
        source = _source("validate-reactor.ps1")
        self.assertIn(
            '$visualManifest = Join-Path $reportDirectory "visual-variants.json"',
            source,
        )
        self.assertEqual(source.count("-VariantsJson $visualManifest"), 2)
        self.assertIn("--manifest $visualManifest --json", source)

    def test_reduced_motion_snapshots_and_always_restores(self):
        source = _source("capture-reactor-variants.ps1")
        self.assertIn("IntPtr pvParam", source)
        self.assertIn(
            "$value = if ($Enabled) { [IntPtr]1 } else { [IntPtr]0 }",
            source,
        )
        self.assertIn("$originalMotionValue = Get-ClientAreaAnimation", source)
        self.assertIn("$motionMutationAttempted = $true", source)
        self.assertIn("$SPIF_UPDATEINIFILE -bor $SPIF_SENDCHANGE", source)
        self.assertRegex(
            source,
            r"finally \{\s*if \(\$motionSnapshotTaken -and "
            r"\$motionMutationAttempted\) \{\s*"
            r"Set-ClientAreaAnimation -Enabled \$originalMotionValue",
        )

    def test_absolute_evidence_paths_are_supported(self):
        for name in (
            "capture-reactor-variants.ps1",
            "test-reactor-process-refresh-parity.ps1",
        ):
            with self.subTest(script=name):
                source = _source(name)
                self.assertIn("[IO.Path]::IsPathRooted($Path)", source)
                self.assertIn("Get-AbsolutePath -Path $VariantsJson", source)

    def test_ai_flow_uses_the_wire_exact_provider_key_and_scan_free_prompts(self):
        source = _source("test-reactor-ai-flows.ps1")
        self.assertIn('preferredAIProvider = "custom_openai"', source)
        self.assertNotIn('preferredAiProvider = "custom_openai"', source)
        self.assertIn('-Value "hello"', source)
        self.assertNotIn('-Value "hello there"', source)
        self.assertIn('-Value "Write a slow greeting."', source)
        self.assertIn(
            '-Value "Write a tool-contract reply that lists vetted remediations."',
            source,
        )

    def test_action_regression_gate_is_isolated_closed_and_non_destructive(self):
        source = _source("test-reactor-action-regressions.ps1")
        self.assertIn("$probe.settings_test_path -ne $true", source)
        self.assertIn(
            'WFDIAG_REACTOR_VISUAL_STATE = "remediation-partial"', source
        )
        self.assertIn(
            'WFDIAG_REACTOR_LIVE_TEST_FIXTURE = "export-fallback"', source
        )
        self.assertIn(
            'WFDIAG_REACTOR_LIVE_TEST_FIXTURE = "device-manager"', source
        )
        self.assertIn("GUID-scoped temporary files", source)
        self.assertIn("Close-ExactTopLevelWindow", source)
        self.assertIn("-IncludeAdminRelaunch", source)
        self.assertIn("never drives the secure desktop", source)
        self.assertNotRegex(source, r"Stop-Process[^\n]+\$deviceWindow")
        self.assertNotRegex(source, r"Stop-Process[^\n]+\$elevatedChild")


if __name__ == "__main__":
    unittest.main()
