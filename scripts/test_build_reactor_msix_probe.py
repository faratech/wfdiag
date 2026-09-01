import hashlib
import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile


SCRIPT_PATH = Path(__file__).with_name("build-reactor-msix-probe.py")
SPEC = importlib.util.spec_from_file_location("build_reactor_msix_probe", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
probe = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = probe
SPEC.loader.exec_module(probe)


def pe_image(machine: int) -> bytes:
    result = bytearray(256)
    result[0:2] = b"MZ"
    struct.pack_into("<I", result, 0x3C, 0x80)
    result[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<H", result, 0x84, machine)
    return bytes(result)


class ReactorMsixProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.exe_bytes = pe_image(0x8664) + b"reactor-executable"
        self.bootstrap_bytes = pe_image(0x8664) + b"reactor-2.4-bootstrap"
        self.target = probe.Target(
            "x64",
            "x86_64-pc-windows-msvc",
            "x64",
            0x8664,
            hashlib.sha256(self.bootstrap_bytes).hexdigest(),
        )

    def tearDown(self):
        self.temporary.cleanup()

    def _payload_files(self):
        executable = self.root / "source" / probe.REACTOR_BINARY
        bootstrap = self.root / "source" / probe.BOOTSTRAP_DLL
        executable.parent.mkdir(parents=True)
        executable.write_bytes(self.exe_bytes)
        bootstrap.write_bytes(self.bootstrap_bytes)
        return executable, bootstrap

    def test_manifest_preserves_store_contract_and_aligns_runtime_2_4(self):
        probe.assert_reactor_dependency_contract()
        source_root = ET.parse(probe.STORE_MANIFEST).getroot()
        generated = probe.render_probe_manifest(probe.TARGETS["x64"])
        generated_root = ET.fromstring(generated)

        self.assertEqual(
            probe._capabilities(generated_root), probe._capabilities(source_root)
        )
        self.assertTrue(
            probe.REQUIRED_CAPABILITIES.issubset(probe._capabilities(generated_root))
        )
        self.assertEqual(
            probe._device_families(generated_root), probe._device_families(source_root)
        )
        self.assertEqual(
            probe.manifest_asset_paths(generated_root),
            probe.manifest_asset_paths(source_root),
        )
        self.assertEqual(
            probe.manifest_asset_paths(generated_root),
            {
                "Logo.png",
                "Square150x150Logo.png",
                "Square44x44Logo.png",
                "Wide310x150Logo.png",
            },
        )

        dependencies = generated_root.find(
            probe._qname(probe.NS_FOUNDATION, "Dependencies")
        )
        self.assertIsNotNone(dependencies)
        runtimes = [
            child
            for child in dependencies
            if child.tag == probe._qname(probe.NS_FOUNDATION, "PackageDependency")
            and child.attrib.get("Name", "").startswith("Microsoft.WindowsAppRuntime.")
        ]
        self.assertEqual(len(runtimes), 1)
        self.assertEqual(runtimes[0].attrib["Name"], "Microsoft.WindowsAppRuntime.2")
        self.assertEqual(runtimes[0].attrib["MinVersion"], "2.4.0.0")

    def test_layout_contains_only_reactor_exe_bootstrap_manifest_and_assets(self):
        executable, bootstrap = self._payload_files()
        output = self.root / "output"
        output.mkdir()

        layout = probe.stage_layout(
            output, self.target, executable, bootstrap
        )

        probe.assert_layout_contract(layout, self.target)
        dlls = [
            path.name
            for path in layout.rglob("*")
            if path.is_file() and path.suffix.casefold() == ".dll"
        ]
        self.assertEqual(dlls, [probe.BOOTSTRAP_DLL])
        self.assertFalse((layout / "Microsoft.WindowsAppRuntime.dll").exists())
        self.assertFalse((layout / "Microsoft.Windows.AI.Text.dll").exists())

    def test_layout_rejects_dual_runtime_and_app_local_ai_dlls(self):
        executable, bootstrap = self._payload_files()
        output = self.root / "output"
        output.mkdir()
        layout = probe.stage_layout(output, self.target, executable, bootstrap)

        for forbidden in (
            "Microsoft.WindowsAppRuntime.dll",
            "Microsoft.UI.Xaml.dll",
            "Microsoft.Windows.AI.Text.dll",
        ):
            with self.subTest(forbidden=forbidden):
                path = layout / forbidden
                path.write_bytes(b"forbidden")
                with self.assertRaises(probe.ProbeBuildError):
                    probe.assert_layout_contract(layout, self.target)
                path.unlink()

    def test_stale_ai_sdk_bootstrap_is_rejected_by_pinned_hash(self):
        stale = self.root / probe.BOOTSTRAP_DLL
        stale.write_bytes(pe_image(0x8664) + b"stale-1.8-bootstrap")

        with self.assertRaisesRegex(
            probe.ProbeBuildError, "not the pinned Reactor Windows App Runtime 2.4"
        ):
            probe.assert_bootstrap_identity(stale, self.target)

    def test_msix_archive_contract_rejects_app_local_ai_payload_drift(self):
        executable, bootstrap = self._payload_files()
        output = self.root / "output"
        output.mkdir()
        layout = probe.stage_layout(output, self.target, executable, bootstrap)
        package = self.root / "probe.msix"

        with zipfile.ZipFile(package, "w") as archive:
            for path in layout.rglob("*"):
                if path.is_file():
                    archive.write(path, path.relative_to(layout).as_posix())
        probe.assert_msix_contract(package, self.target)

        with zipfile.ZipFile(package, "a") as archive:
            archive.writestr("Microsoft.Windows.AI.Text.dll", b"stale")
        with self.assertRaisesRegex(probe.ProbeBuildError, "app-local AI DLL"):
            probe.assert_msix_contract(package, self.target)

    def test_msix_archive_contract_rejects_a_signature(self):
        executable, bootstrap = self._payload_files()
        output = self.root / "output"
        output.mkdir()
        layout = probe.stage_layout(output, self.target, executable, bootstrap)
        package = self.root / "signed-probe.msix"

        with zipfile.ZipFile(package, "w") as archive:
            for path in layout.rglob("*"):
                if path.is_file():
                    archive.write(path, path.relative_to(layout).as_posix())
            archive.writestr("AppxSignature.p7x", b"unexpected-signature")

        with self.assertRaisesRegex(probe.ProbeBuildError, "unexpectedly signed"):
            probe.assert_msix_contract(package, self.target)

    def test_framework_build_command_never_enables_validation_or_self_contained_features(self):
        command = probe._cargo_command(probe.TARGETS["arm64"], True)

        self.assertIn("--locked", command)
        self.assertIn("aarch64-pc-windows-msvc", command)
        self.assertIn("--release", command)
        self.assertNotIn("--features", command)
        self.assertNotIn("self-contained", command)
        self.assertNotIn("settings-test-path", command)

    def test_only_reactor_build_script_outputs_are_removed_before_restaging(self):
        profile = self.root / "target" / "release"
        reactor_output = profile / "build" / "wfdiag-deadbeefdeadbeef"
        dependency_output = profile / "build" / "serde-deadbeefdeadbeef"
        # Same workspace target dir, same `wfdiag-` prefix: must survive.
        tauri_output = profile / "build" / "wfdiag-tauri-deadbeefdeadbeef"
        engine_output = profile / "build" / "wfdiag-native-phi-deadbeefdeadbeef"
        reactor_output.mkdir(parents=True)
        for sibling in (dependency_output, tauri_output, engine_output):
            sibling.mkdir()

        probe._remove_reactor_build_script_outputs(profile)

        self.assertFalse(reactor_output.exists())
        self.assertTrue(dependency_output.is_dir())
        self.assertTrue(tauri_output.is_dir())
        self.assertTrue(engine_output.is_dir())

    def test_owned_directory_guard_rejects_output_root(self):
        with self.assertRaisesRegex(probe.ProbeBuildError, "non-child path"):
            probe._reset_owned_directory(self.root, self.root)


if __name__ == "__main__":
    unittest.main()
