import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest


SCRIPT_PATH = Path(__file__).with_name("check-reactor-readiness.py")
SPEC = importlib.util.spec_from_file_location("check_reactor_readiness", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
readiness = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = readiness
SPEC.loader.exec_module(readiness)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def png(width: int = 100, height: int = 80) -> bytes:
    # The checker intentionally needs only the signature and IHDR dimensions.
    return (
        b"\x89PNG\r\n\x1a\n"
        + struct.pack(">I", 13)
        + b"IHDR"
        + struct.pack(">II", width, height)
        + b"\x08\x02\x00\x00\x00"
        + b"\x00\x00\x00\x00"
    )


def appx_manifest(runtime: str) -> str:
    return f'''<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         xmlns:systemai="http://schemas.microsoft.com/appx/manifest/systemai/windows10">
  <Identity Name="{readiness.EXPECTED_IDENTITY_NAME}"
            Publisher="{readiness.EXPECTED_PUBLISHER}"
            Version="2.5.8.0" ProcessorArchitecture="x64" />
  <Properties>
    <PublisherDisplayName>{readiness.EXPECTED_PUBLISHER_DISPLAY_NAME}</PublisherDisplayName>
  </Properties>
  <Dependencies><PackageDependency Name="{runtime}" MinVersion="1.0.0.0" /></Dependencies>
  <Capabilities>
    <Capability Name="internetClient" />
    <Capability Name="internetClientServer" />
    <Capability Name="privateNetworkClientServer" />
    <rescap:Capability Name="runFullTrust" />
    <systemai:Capability Name="systemAIModels" />
  </Capabilities>
</Package>
'''


def passed_gates(ids):
    return [
        {
            "id": gate_id,
            "status": "passed",
            "summary": f"{gate_id} verified",
            "evidence": [f"evidence/{gate_id}.json"],
        }
        for gate_id in sorted(ids)
    ]


def passed_backend_surfaces():
    return [
        {
            "id": surface_id,
            "status": "passed",
            "summary": f"{surface_id} verified",
            "evidence": [f"evidence/{surface_id}.json"],
        }
        for surface_id in sorted(readiness.REQUIRED_BACKEND_SURFACES)
    ]


class Fixture:
    def __init__(self, root: Path):
        self.root = root
        screenshot = png()
        asset = b"brand-asset"
        self.write_bytes("baselines/reference.png", screenshot)
        self.write_bytes("assets/brand.bin", asset)
        source_package = {
            "name": readiness.EXPECTED_IDENTITY_NAME,
            "package_full_name": (
                f"{readiness.EXPECTED_IDENTITY_NAME}_2.5.8.0_x64__testpublisher"
            ),
            "package_family_name": f"{readiness.EXPECTED_IDENTITY_NAME}_testpublisher",
            "version": "2.5.8.0",
            "architecture": "X64",
            "signature_kind": "Store",
        }
        capture_metadata = {
            "state": "reference",
            "package_full_name": source_package["package_full_name"],
            "package_family_name": source_package["package_family_name"],
            "package_version": source_package["version"],
            "architecture": source_package["architecture"],
            "signature_kind": source_package["signature_kind"],
            "logical_width": 100,
            "logical_height": 80,
            "dpi": 96,
            "scale": 1.0,
            "physical_visible_width": 100,
            "physical_visible_height": 80,
        }
        capture_metadata_bytes = json.dumps(capture_metadata).encode()
        self.write_bytes("baselines/reference.capture.json", capture_metadata_bytes)
        self.write_text("version.json", json.dumps({"version": "2.5.8"}))
        self.write_text(
            "AppxManifest.xml",
            appx_manifest(readiness.EXPECTED_REACTOR_FRAMEWORK),
        )
        self.write_text(
            "reactor-spike/Cargo.toml",
            f'''[package]
name = "test-reactor-spike"
version = "0.0.0"
edition = "2024"

[dependencies]
windows-reactor = {{ git = "{readiness.EXPECTED_REACTOR_REPOSITORY}", rev = "{readiness.EXPECTED_REACTOR_REVISION}" }}

[build-dependencies]
windows-reactor-setup = {{ git = "{readiness.EXPECTED_REACTOR_REPOSITORY}", rev = "{readiness.EXPECTED_REACTOR_REVISION}" }}
''',
        )
        self.write_text(
            "reactor-spike/src/main.rs",
            "use windows_reactor::*;\nfn main() {}\n",
        )
        self.manifest = {
            "schema_version": readiness.EXPECTED_SCHEMA_VERSION,
            "store_manifest": "AppxManifest.xml",
            "ui_architecture": {
                "kind": readiness.EXPECTED_UI_ARCHITECTURE,
                "source_root": readiness.EXPECTED_UI_SOURCE_ROOT,
                "webview_ui_allowed": False,
            },
            "reactor_pin": {
                "repository": readiness.EXPECTED_REACTOR_REPOSITORY,
                "revision": readiness.EXPECTED_REACTOR_REVISION,
                "expected_crate_version": readiness.EXPECTED_REACTOR_VERSION,
                "prototype_manifest": "reactor-spike/Cargo.toml",
                "windows_app_runtime_release": readiness.EXPECTED_REACTOR_RUNTIME_RELEASE,
                "windows_app_runtime_framework": readiness.EXPECTED_REACTOR_FRAMEWORK,
            },
            "store_identity": {
                "name": readiness.EXPECTED_IDENTITY_NAME,
                "publisher": readiness.EXPECTED_PUBLISHER,
                "publisher_display_name": readiness.EXPECTED_PUBLISHER_DISPLAY_NAME,
                "capabilities": sorted(readiness.EXPECTED_CAPABILITIES),
            },
            "baseline": {
                "application_version": "2.5.8",
                "version_file": "version.json",
                "source_package": source_package,
                "screenshots": [
                    {
                        "id": "reference",
                        "path": "baselines/reference.png",
                        "screen": "diagnostics",
                        "state": "populated",
                        "theme": "dark",
                        "source_application_version": "2.5.8",
                        "viewport": {"width": 100, "height": 80},
                        "sha256": sha256(screenshot),
                        "capture_metadata": {
                            "path": "baselines/reference.capture.json",
                            "sha256": sha256(capture_metadata_bytes),
                        },
                    }
                ],
                "assets": [
                    {
                        "id": "brand",
                        "path": "assets/brand.bin",
                        "sha256": sha256(asset),
                    }
                ],
            },
            "visual_acceptance_categories": [
                {"id": category, "acceptance": f"Review {category}."}
                for category in sorted(readiness.REQUIRED_ACCEPTANCE_CATEGORIES)
            ],
            "backend_parity": passed_backend_surfaces(),
            "upstream_api_gates": passed_gates(readiness.REQUIRED_UPSTREAM_GATES),
            "cutover_gates": passed_gates(readiness.REQUIRED_CUTOVER_GATES),
        }
        self.save_manifest()

    def write_text(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def write_bytes(self, relative: str, content: bytes) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    def save_manifest(self) -> None:
        self.write_text(
            "reactor-baselines/manifest.json",
            json.dumps(self.manifest, indent=2),
        )

    def report(self):
        return readiness.evaluate_readiness(self.root)


def codes(report, severity=None):
    return {
        finding.code
        for finding in report.findings
        if severity is None or finding.severity == severity
    }


class ReactorReadinessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.fixture = Fixture(self.root)

    def tearDown(self):
        self.temporary.cleanup()

    def test_complete_fixture_is_ready(self):
        report = self.fixture.report()

        self.assertTrue(report.ready, report.to_dict())
        self.assertFalse(codes(report, "blocker"))
        self.assertFalse(codes(report, "error"))

    def test_runtime_1_8_to_2_4_mismatch_is_a_blocker(self):
        self.fixture.write_text(
            "AppxManifest.xml",
            appx_manifest("Microsoft.WindowsAppRuntime.1.8"),
        )

        report = self.fixture.report()

        self.assertFalse(report.ready)
        self.assertIn("runtime.alignment", codes(report, "blocker"))
        finding = next(f for f in report.findings if f.code == "runtime.alignment")
        self.assertEqual(
            finding.details["store_framework"],
            "Microsoft.WindowsAppRuntime.1.8",
        )
        self.assertEqual(
            finding.details["reactor_runtime_release"],
            readiness.EXPECTED_REACTOR_RUNTIME_RELEASE,
        )

    def test_store_identity_and_capability_drift_are_blockers(self):
        changed = appx_manifest(readiness.EXPECTED_REACTOR_FRAMEWORK)
        changed = changed.replace(
            readiness.EXPECTED_IDENTITY_NAME,
            "Example.DifferentIdentity",
        ).replace('<Capability Name="internetClient" />\n', "")
        self.fixture.write_text("AppxManifest.xml", changed)

        report = self.fixture.report()

        blockers = codes(report, "blocker")
        self.assertIn("store.identity", blockers)
        self.assertIn("store.capabilities", blockers)

    def test_floating_or_changed_reactor_pin_is_a_blocker(self):
        cargo_path = self.root / "reactor-spike/Cargo.toml"
        cargo = cargo_path.read_text(encoding="utf-8")
        cargo = cargo.replace(
            f'rev = "{readiness.EXPECTED_REACTOR_REVISION}"',
            'branch = "master"',
            1,
        )
        cargo_path.write_text(cargo, encoding="utf-8")

        report = self.fixture.report()

        self.assertIn("reactor.prototype", codes(report, "blocker"))

    def test_webview_dependency_is_a_blocker(self):
        cargo_path = self.root / "reactor-spike/Cargo.toml"
        cargo = cargo_path.read_text(encoding="utf-8")
        cargo = cargo.replace(
            "[build-dependencies]",
            'browser-host = { package = "webview2-com-sys", version = "0.38" }\n\n'
            "[build-dependencies]",
        )
        cargo_path.write_text(cargo, encoding="utf-8")

        report = self.fixture.report()

        self.assertIn("ui.native", codes(report, "blocker"))
        finding = next(f for f in report.findings if f.code == "ui.native")
        self.assertIn("forbidden_dependencies", finding.details["problems"])

    def test_webview_source_or_frontend_asset_is_a_blocker(self):
        self.fixture.write_text(
            "reactor-spike/src/browser.rs",
            "fn attach(controller: CoreWebView2Controller) {}\n",
        )
        self.fixture.write_text("reactor-spike/src/index.html", "<main>WFDiag</main>")

        report = self.fixture.report()

        self.assertIn("ui.native", codes(report, "blocker"))
        finding = next(f for f in report.findings if f.code == "ui.native")
        self.assertIn("forbidden_source_markers", finding.details["problems"])
        self.assertIn("web_frontend_assets", finding.details["problems"])

    def test_captured_webview_process_name_is_data_not_a_ui_marker(self):
        self.fixture.write_text(
            "reactor-spike/src/process_fixture.rs",
            'const PROCESS_NAME: &str = "msedgewebview2.exe";\n',
        )

        report = self.fixture.report()

        finding = next(f for f in report.findings if f.code == "ui.native")
        self.assertEqual(finding.severity, "pass")

    def test_webview_permission_cannot_be_enabled_in_contract(self):
        self.fixture.manifest["ui_architecture"]["webview_ui_allowed"] = True
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("ui.contract", codes(report, "error"))

    def test_missing_or_changed_baseline_and_asset_are_blockers(self):
        (self.root / "baselines/reference.png").unlink()
        (self.root / "assets/brand.bin").write_bytes(b"changed")

        report = self.fixture.report()

        blockers = codes(report, "blocker")
        self.assertIn("baseline.screenshots", blockers)
        self.assertIn("baseline.assets", blockers)

    def test_missing_capture_metadata_is_a_blocker(self):
        (self.root / "baselines/reference.capture.json").unlink()

        report = self.fixture.report()

        self.assertIn("baseline.capture_metadata", codes(report, "blocker"))

    def test_non_store_capture_provenance_is_a_blocker(self):
        self.fixture.manifest["baseline"]["source_package"]["signature_kind"] = "Developer"
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("baseline.source_package", codes(report, "blocker"))

    def test_required_upstream_gate_cannot_be_omitted(self):
        self.fixture.manifest["upstream_api_gates"] = [
            gate
            for gate in self.fixture.manifest["upstream_api_gates"]
            if gate["id"] != "window_lifecycle"
        ]
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("upstream.contract", codes(report, "error"))

    def test_required_backend_surface_cannot_be_omitted(self):
        self.fixture.manifest["backend_parity"] = [
            surface
            for surface in self.fixture.manifest["backend_parity"]
            if surface["id"] != "settings_and_credentials"
        ]
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("backend.contract", codes(report, "error"))

    def test_partial_backend_surface_blocks_cutover(self):
        surface = self.fixture.manifest["backend_parity"][0]
        surface["status"] = "partial"
        surface["summary"] = "The native adapter exists but the screen is not wired."
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("backend.parity", codes(report, "blocker"))
        finding = next(f for f in report.findings if f.code == "backend.parity")
        self.assertIn(
            {"id": surface["id"], "status": "partial"},
            finding.details["incomplete"],
        )

    def test_passed_backend_surface_requires_evidence(self):
        surface = self.fixture.manifest["backend_parity"][0]
        surface["evidence"] = []
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("backend.parity", codes(report, "error"))

    def test_malformed_acceptance_category_is_reported_without_crashing(self):
        self.fixture.manifest["visual_acceptance_categories"] = ["invalid"]
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("baseline.acceptance", codes(report, "error"))

    def test_blocked_gate_prevents_cutover(self):
        gate = self.fixture.manifest["cutover_gates"][0]
        gate["status"] = "blocked"
        gate["summary"] = "Still waiting for an official release."
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertFalse(report.ready)
        self.assertIn(f"cutover.{gate['id']}", codes(report, "blocker"))

    def test_stale_capture_version_prevents_cutover(self):
        self.fixture.manifest["baseline"]["application_version"] = "2.5.4"
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("baseline.version", codes(report, "blocker"))

    def test_stale_individual_screenshot_version_prevents_cutover(self):
        screenshot = self.fixture.manifest["baseline"]["screenshots"][0]
        screenshot["source_application_version"] = "2.5.4"
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("baseline.screenshot_versions", codes(report, "blocker"))
        finding = next(
            f for f in report.findings if f.code == "baseline.screenshot_versions"
        )
        self.assertEqual(finding.details["current_version"], "2.5.8")
        self.assertEqual(finding.details["current_count"], 0)
        self.assertEqual(finding.details["stale_count"], 1)
        self.assertEqual(finding.details["stale"][0]["id"], "reference")

    def test_screenshot_source_version_is_required(self):
        screenshot = self.fixture.manifest["baseline"]["screenshots"][0]
        del screenshot["source_application_version"]
        self.fixture.save_manifest()

        report = self.fixture.report()

        self.assertIn("baseline.screenshots", codes(report, "blocker"))

    def test_evaluation_does_not_modify_fixture(self):
        def snapshot():
            return {
                str(path.relative_to(self.root)): sha256(path.read_bytes())
                for path in sorted(self.root.rglob("*"))
                if path.is_file()
            }

        before = snapshot()
        self.fixture.report()
        after = snapshot()

        self.assertEqual(before, after)


if __name__ == "__main__":
    unittest.main()
