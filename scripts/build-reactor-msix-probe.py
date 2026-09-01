#!/usr/bin/env python3
"""Build an unsigned, non-publishing Windows Reactor Store/MSIX probe.

This path is deliberately separate from the shipping Tauri Store workflow. It
builds the framework-dependent Reactor executable for x64 and ARM64, stages
only Reactor's matching Windows App Runtime bootstrap DLL, derives the package
manifest from the canonical Store manifest, and packages unsigned MSIX files
for offline inspection.

It never signs, installs, registers, uploads, or publishes a package.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET
import zipfile


PROJECT_ROOT = Path(__file__).resolve().parent.parent
REACTOR_MANIFEST = PROJECT_ROOT / "reactor-spike" / "Cargo.toml"
STORE_MANIFEST = PROJECT_ROOT / "AppxManifest.xml"
VERSION_FILE = PROJECT_ROOT / "version.json"
ICONS_DIR = PROJECT_ROOT / "src-tauri" / "icons"

STORE_IDENTITY_NAME = "32827MikeFara.WindowsForumDiagnostics"
STORE_PUBLISHER = "CN=ABDB6B3F-DF9E-447D-BC0E-4DA7BAFD14C4"
STORE_EXECUTABLE = "wfdiag-reactor-spike.exe"
REACTOR_BINARY = "wfdiag-reactor-spike.exe"
BOOTSTRAP_DLL = "Microsoft.WindowsAppRuntime.Bootstrap.dll"
WINDOWS_APP_RUNTIME_FRAMEWORK = "Microsoft.WindowsAppRuntime.2"
WINDOWS_APP_RUNTIME_MIN_VERSION = "2.4.0.0"
REACTOR_REPOSITORY = "https://github.com/microsoft/windows-rs"
REACTOR_REVISION = "1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8"

NS_FOUNDATION = "http://schemas.microsoft.com/appx/manifest/foundation/windows10"
NS_PHONE = "http://schemas.microsoft.com/appx/2014/phone/manifest"
NS_UAP = "http://schemas.microsoft.com/appx/manifest/uap/windows10"
NS_RESCAP = (
    "http://schemas.microsoft.com/appx/manifest/foundation/windows10/"
    "restrictedcapabilities"
)
NS_SYSTEM_AI = "http://schemas.microsoft.com/appx/manifest/systemai/windows10"

ET.register_namespace("", NS_FOUNDATION)
ET.register_namespace("mp", NS_PHONE)
ET.register_namespace("uap", NS_UAP)
ET.register_namespace("rescap", NS_RESCAP)
ET.register_namespace("systemai", NS_SYSTEM_AI)

REQUIRED_CAPABILITIES = {
    (NS_FOUNDATION, "internetClient"),
    (NS_FOUNDATION, "internetClientServer"),
    (NS_FOUNDATION, "privateNetworkClientServer"),
    (NS_RESCAP, "runFullTrust"),
    (NS_SYSTEM_AI, "systemAIModels"),
}

ASSET_SOURCES = {
    "Logo.png": ICONS_DIR / "icon.png",
    "Square150x150Logo.png": ICONS_DIR / "512x512.png",
    "Square44x44Logo.png": (
        ICONS_DIR / "256x256.png"
        if (ICONS_DIR / "256x256.png").is_file()
        else ICONS_DIR / "128x128@2x.png"
    ),
    "Wide310x150Logo.png": ICONS_DIR / "Wide310x150Logo.png",
}


@dataclass(frozen=True)
class Target:
    name: str
    triple: str
    manifest_architecture: str
    pe_machine: int
    bootstrap_sha256: str


TARGETS = {
    "x64": Target(
        "x64",
        "x86_64-pc-windows-msvc",
        "x64",
        0x8664,
        "44752d799b8d7cead99d6a20cb9a46009a9a2dfaa9701176a573e50a30ab089c",
    ),
    "arm64": Target(
        "arm64",
        "aarch64-pc-windows-msvc",
        "arm64",
        0xAA64,
        "cd7e3ecba5615152fe1cb508b30781bdaf108960847c1cab3636a5771e61fdcd",
    ),
}


class ProbeBuildError(RuntimeError):
    """A probe invariant failed before any package could be trusted."""


def _qname(namespace: str, local_name: str) -> str:
    return f"{{{namespace}}}{local_name}"


def _split_qname(name: str) -> tuple[str, str]:
    if name.startswith("{") and "}" in name:
        namespace, local_name = name[1:].split("}", 1)
        return namespace, local_name
    return "", name


def _one(root: ET.Element, path: str, description: str) -> ET.Element:
    matches = root.findall(path)
    if len(matches) != 1:
        raise ProbeBuildError(
            f"expected exactly one {description}, found {len(matches)}"
        )
    return matches[0]


def canonical_version() -> str:
    try:
        document = json.loads(VERSION_FILE.read_text(encoding="utf-8"))
        value = document["version"]
    except (OSError, KeyError, json.JSONDecodeError) as error:
        raise ProbeBuildError(f"cannot read canonical version: {error}") from error
    parts = value.split(".") if isinstance(value, str) else []
    if len(parts) != 3 or not all(part.isdigit() for part in parts):
        raise ProbeBuildError(f"canonical version must be numeric X.Y.Z, got {value!r}")
    return value


def assert_reactor_dependency_contract() -> None:
    try:
        document = tomllib.loads(REACTOR_MANIFEST.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ProbeBuildError(f"cannot parse {REACTOR_MANIFEST}: {error}") from error
    expected = (
        ("dependencies", "windows-reactor"),
        ("build-dependencies", "windows-reactor-setup"),
    )
    for table_name, dependency_name in expected:
        dependency = document.get(table_name, {}).get(dependency_name)
        if not isinstance(dependency, dict):
            raise ProbeBuildError(f"{dependency_name} must be an exact git dependency")
        if dependency.get("git") != REACTOR_REPOSITORY or dependency.get(
            "rev"
        ) != REACTOR_REVISION:
            raise ProbeBuildError(
                f"{dependency_name} is not pinned to the reviewed Reactor revision"
            )


def _capabilities(root: ET.Element) -> set[tuple[str, str]]:
    container = _one(
        root,
        f"./{_qname(NS_FOUNDATION, 'Capabilities')}",
        "Capabilities element",
    )
    return {
        (_split_qname(child.tag)[0], child.attrib.get("Name", ""))
        for child in container
        if _split_qname(child.tag)[1] == "Capability"
    }


def _device_families(root: ET.Element) -> list[dict[str, str]]:
    dependencies = _one(
        root,
        f"./{_qname(NS_FOUNDATION, 'Dependencies')}",
        "Dependencies element",
    )
    return [
        dict(child.attrib)
        for child in dependencies
        if child.tag == _qname(NS_FOUNDATION, "TargetDeviceFamily")
    ]


def manifest_asset_paths(root: ET.Element) -> set[str]:
    assets: set[str] = set()
    for element in root.iter():
        element_local_name = _split_qname(element.tag)[1].lower()
        if element_local_name in {"logo", "image"} and element.text:
            assets.add(element.text.strip().replace("\\", "/"))
        for raw_name, value in element.attrib.items():
            local_name = _split_qname(raw_name)[1].lower()
            if local_name.endswith("logo") or local_name == "image":
                assets.add(value.replace("\\", "/"))
    return assets


def render_probe_manifest(target: Target) -> bytes:
    """Derive the probe manifest without changing the shipping manifest."""
    try:
        source_tree = ET.parse(STORE_MANIFEST)
    except (OSError, ET.ParseError) as error:
        raise ProbeBuildError(f"cannot parse {STORE_MANIFEST}: {error}") from error

    source_root = source_tree.getroot()
    source_capabilities = _capabilities(source_root)
    source_device_families = _device_families(source_root)
    source_assets = manifest_asset_paths(source_root)

    identity = _one(
        source_root,
        f"./{_qname(NS_FOUNDATION, 'Identity')}",
        "package Identity",
    )
    expected_version = f"{canonical_version()}.0"
    if identity.attrib.get("Name") != STORE_IDENTITY_NAME:
        raise ProbeBuildError("canonical Store identity name drifted")
    if identity.attrib.get("Publisher") != STORE_PUBLISHER:
        raise ProbeBuildError("canonical Store publisher drifted")
    if identity.attrib.get("Version") != expected_version:
        raise ProbeBuildError(
            "canonical Store manifest version does not match version.json: "
            f"{identity.attrib.get('Version')!r} != {expected_version!r}"
        )
    identity.set("ProcessorArchitecture", target.manifest_architecture)

    application = _one(
        source_root,
        (
            f"./{_qname(NS_FOUNDATION, 'Applications')}/"
            f"{_qname(NS_FOUNDATION, 'Application')}"
        ),
        "Store Application",
    )
    if application.attrib.get("EntryPoint") != "Windows.FullTrustApplication":
        raise ProbeBuildError("canonical Store application is no longer full trust")
    application.set("Executable", STORE_EXECUTABLE)

    dependencies = _one(
        source_root,
        f"./{_qname(NS_FOUNDATION, 'Dependencies')}",
        "Dependencies element",
    )
    for child in list(dependencies):
        if child.tag == _qname(NS_FOUNDATION, "PackageDependency") and child.attrib.get(
            "Name", ""
        ).startswith("Microsoft.WindowsAppRuntime."):
            dependencies.remove(child)
    ET.SubElement(
        dependencies,
        _qname(NS_FOUNDATION, "PackageDependency"),
        {
            "Name": WINDOWS_APP_RUNTIME_FRAMEWORK,
            "MinVersion": WINDOWS_APP_RUNTIME_MIN_VERSION,
            "Publisher": (
                "CN=Microsoft Corporation, O=Microsoft Corporation, "
                "L=Redmond, S=Washington, C=US"
            ),
        },
    )

    ET.indent(source_tree, space="  ")
    payload = ET.tostring(
        source_root,
        encoding="utf-8",
        xml_declaration=True,
        short_empty_elements=True,
    )
    assert_manifest_contract(
        payload,
        target,
        expected_capabilities=source_capabilities,
        expected_device_families=source_device_families,
        expected_assets=source_assets,
    )
    return payload


def assert_manifest_contract(
    payload: bytes,
    target: Target,
    *,
    expected_capabilities: set[tuple[str, str]] | None = None,
    expected_device_families: list[dict[str, str]] | None = None,
    expected_assets: set[str] | None = None,
) -> None:
    try:
        root = ET.fromstring(payload)
    except ET.ParseError as error:
        raise ProbeBuildError(f"generated manifest is invalid XML: {error}") from error

    identity = _one(
        root, f"./{_qname(NS_FOUNDATION, 'Identity')}", "package Identity"
    )
    expected_identity = {
        "Name": STORE_IDENTITY_NAME,
        "Publisher": STORE_PUBLISHER,
        "Version": f"{canonical_version()}.0",
        "ProcessorArchitecture": target.manifest_architecture,
    }
    for name, expected in expected_identity.items():
        if identity.attrib.get(name) != expected:
            raise ProbeBuildError(
                f"generated manifest Identity {name} is "
                f"{identity.attrib.get(name)!r}, expected {expected!r}"
            )

    application = _one(
        root,
        (
            f"./{_qname(NS_FOUNDATION, 'Applications')}/"
            f"{_qname(NS_FOUNDATION, 'Application')}"
        ),
        "Store Application",
    )
    if application.attrib.get("Executable") != STORE_EXECUTABLE:
        raise ProbeBuildError("generated manifest does not launch the Reactor executable")
    if application.attrib.get("EntryPoint") != "Windows.FullTrustApplication":
        raise ProbeBuildError("generated manifest lost the full-trust entry point")

    families = _device_families(root)
    family_names = {family.get("Name") for family in families}
    if not {"Windows.Universal", "Windows.Desktop"}.issubset(family_names):
        raise ProbeBuildError("generated manifest must retain both TargetDeviceFamily entries")
    if expected_device_families is not None and families != expected_device_families:
        raise ProbeBuildError("generated manifest changed a TargetDeviceFamily contract")

    capabilities = _capabilities(root)
    missing_capabilities = REQUIRED_CAPABILITIES - capabilities
    if missing_capabilities:
        raise ProbeBuildError(
            f"generated manifest is missing capabilities: {sorted(missing_capabilities)!r}"
        )
    if expected_capabilities is not None and capabilities != expected_capabilities:
        raise ProbeBuildError("generated manifest changed the canonical capability set")

    dependencies = _one(
        root,
        f"./{_qname(NS_FOUNDATION, 'Dependencies')}",
        "Dependencies element",
    )
    runtime_dependencies = [
        child
        for child in dependencies
        if child.tag == _qname(NS_FOUNDATION, "PackageDependency")
        and child.attrib.get("Name", "").startswith("Microsoft.WindowsAppRuntime.")
    ]
    if len(runtime_dependencies) != 1:
        raise ProbeBuildError(
            "generated manifest must contain exactly one Windows App Runtime dependency"
        )
    runtime = runtime_dependencies[0]
    if runtime.attrib.get("Name") != WINDOWS_APP_RUNTIME_FRAMEWORK or runtime.attrib.get(
        "MinVersion"
    ) != WINDOWS_APP_RUNTIME_MIN_VERSION:
        raise ProbeBuildError(
            "generated manifest is not aligned to Microsoft.WindowsAppRuntime.2 2.4.0.0"
        )

    assets = manifest_asset_paths(root)
    if expected_assets is not None and assets != expected_assets:
        raise ProbeBuildError("generated manifest changed the canonical asset references")


def pe_machine_bytes(data: bytes, description: str) -> int:
    if data[:2] != b"MZ" or len(data) < 0x40:
        raise ProbeBuildError(f"{description} is not a PE image")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise ProbeBuildError(f"{description} has an invalid PE header")
    if len(data) < pe_offset + 6:
        raise ProbeBuildError(f"{description} has a truncated PE header")
    return struct.unpack_from("<H", data, pe_offset + 4)[0]


def assert_pe_architecture(path: Path, target: Target) -> None:
    try:
        machine = pe_machine_bytes(path.read_bytes(), str(path))
    except OSError as error:
        raise ProbeBuildError(f"cannot read PE image {path}: {error}") from error
    if machine != target.pe_machine:
        raise ProbeBuildError(
            f"{path} has PE machine 0x{machine:04X}; "
            f"{target.name} requires 0x{target.pe_machine:04X}"
        )


def assert_bootstrap_identity(path: Path, target: Target) -> None:
    actual = _sha256(path)
    if actual != target.bootstrap_sha256:
        raise ProbeBuildError(
            f"{path} is not the pinned Reactor Windows App Runtime 2.4 bootstrap "
            f"for {target.name}: sha256 {actual}"
        )


def _case_insensitive_file(directory: Path, name: str) -> Path:
    matches = [
        path
        for path in directory.iterdir()
        if path.is_file() and path.name.casefold() == name.casefold()
    ]
    if len(matches) != 1:
        raise ProbeBuildError(
            f"expected exactly one {name} in {directory}, found {len(matches)}"
        )
    return matches[0]


def _profile_root_dlls(directory: Path) -> list[Path]:
    return sorted(
        (
            path
            for path in directory.iterdir()
            if path.is_file() and path.suffix.casefold() == ".dll"
        ),
        key=lambda path: path.name.casefold(),
    )


def _remove_previous_root_deployment_files(profile_dir: Path) -> None:
    """Remove only deployment files from this probe's dedicated target root."""
    if not profile_dir.exists():
        return
    for path in profile_dir.iterdir():
        if not path.is_file():
            continue
        if path.name == REACTOR_BINARY or path.suffix.casefold() == ".dll":
            path.unlink()


def _cargo_command(target: Target, release: bool) -> list[str]:
    command = ["cargo"]
    if os.name != "nt":
        command.append("xwin")
    command.extend(
        [
            "build",
            "--locked",
            "--manifest-path",
            str(REACTOR_MANIFEST),
            "--target",
            target.triple,
        ]
    )
    if release:
        command.append("--release")
    return command


def run_command(command: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> None:
    print(f">>> {' '.join(command)}", flush=True)
    try:
        subprocess.run(command, cwd=cwd, env=env, check=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise ProbeBuildError(f"command failed: {' '.join(command)}: {error}") from error


def _remove_reactor_build_script_outputs(profile_dir: Path) -> None:
    """Force the package build script to restage its bootstrap side effect."""
    build_root = profile_dir / "build"
    if not build_root.is_dir():
        return
    for path in build_root.glob("wfdiag-reactor-spike-*"):
        if path.is_dir():
            shutil.rmtree(path)


def build_framework_dependent_payload(
    target: Target, cargo_target_dir: Path, release: bool
) -> tuple[Path, Path]:
    profile = "release" if release else "debug"
    profile_dir = cargo_target_dir / target.triple / profile

    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(cargo_target_dir)
    llvm_bin = Path("/usr/lib/llvm-20/bin")
    if os.name != "nt" and llvm_bin.is_dir():
        environment["PATH"] = f"{llvm_bin}{os.pathsep}{environment.get('PATH', '')}"
    # The bootstrap is a build-script side effect, not a Cargo artifact. If a
    # previous run's bootstrap is deleted while Cargo considers the build
    # script fresh, Cargo will not recreate it. Remove only this package's
    # target-specific build-script outputs so Reactor setup runs on every probe
    # build while compiled dependencies remain cached.
    _remove_reactor_build_script_outputs(profile_dir)
    _remove_previous_root_deployment_files(profile_dir)
    run_command(_cargo_command(target, release), cwd=PROJECT_ROOT, env=environment)

    executable = profile_dir / REACTOR_BINARY
    if not executable.is_file():
        raise ProbeBuildError(f"Reactor build did not produce {executable}")
    bootstrap = _case_insensitive_file(profile_dir, BOOTSTRAP_DLL)
    root_dlls = _profile_root_dlls(profile_dir)
    if [path.name.casefold() for path in root_dlls] != [BOOTSTRAP_DLL.casefold()]:
        raise ProbeBuildError(
            "framework-dependent build staged DLLs other than the Reactor bootstrap: "
            f"{[path.name for path in root_dlls]!r}"
        )

    assert_pe_architecture(executable, target)
    assert_pe_architecture(bootstrap, target)
    assert_bootstrap_identity(bootstrap, target)
    return executable, bootstrap


def _reset_owned_directory(path: Path, owned_root: Path) -> None:
    resolved_path = path.resolve()
    resolved_root = owned_root.resolve()
    if resolved_path == resolved_root or resolved_root not in resolved_path.parents:
        raise ProbeBuildError(f"refusing to reset non-child path: {resolved_path}")
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)


def _copy_manifest_assets(layout: Path, asset_paths: set[str]) -> None:
    for relative in sorted(asset_paths):
        normalized = relative.replace("\\", "/")
        if normalized.startswith("/") or ".." in Path(normalized).parts:
            raise ProbeBuildError(f"unsafe manifest asset path: {relative!r}")
        source = ASSET_SOURCES.get(normalized)
        if source is None:
            raise ProbeBuildError(
                f"Store manifest references an unmapped package asset: {relative!r}"
            )
        if not source.is_file() or source.stat().st_size == 0:
            raise ProbeBuildError(f"package asset source is missing or empty: {source}")
        destination = layout / normalized
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def assert_layout_contract(layout: Path, target: Target) -> None:
    manifest_path = layout / "AppxManifest.xml"
    if not manifest_path.is_file():
        raise ProbeBuildError(f"layout has no AppxManifest.xml: {layout}")
    payload = manifest_path.read_bytes()
    assert_manifest_contract(payload, target)
    root = ET.fromstring(payload)
    assets = manifest_asset_paths(root)

    expected_files = {
        "AppxManifest.xml",
        STORE_EXECUTABLE,
        BOOTSTRAP_DLL,
        *assets,
    }
    actual_files = {
        path.relative_to(layout).as_posix()
        for path in layout.rglob("*")
        if path.is_file()
    }
    if actual_files != expected_files:
        raise ProbeBuildError(
            "probe layout must contain only the manifest, Store assets, Reactor "
            "executable, and Reactor bootstrap; "
            f"missing={sorted(expected_files - actual_files)!r}, "
            f"unexpected={sorted(actual_files - expected_files)!r}"
        )

    dlls = sorted(
        path.relative_to(layout).as_posix()
        for path in layout.rglob("*")
        if path.is_file() and path.suffix.casefold() == ".dll"
    )
    if [name.casefold() for name in dlls] != [BOOTSTRAP_DLL.casefold()]:
        raise ProbeBuildError(
            "probe layout contains a dual runtime or app-local AI DLL: "
            f"{dlls!r}"
        )

    for asset in assets:
        asset_path = layout / asset
        if not asset_path.is_file() or asset_path.stat().st_size == 0:
            raise ProbeBuildError(f"manifest asset is missing or empty: {asset_path}")
    assert_pe_architecture(layout / STORE_EXECUTABLE, target)
    assert_pe_architecture(layout / BOOTSTRAP_DLL, target)
    assert_bootstrap_identity(layout / BOOTSTRAP_DLL, target)


def stage_layout(
    output_root: Path,
    target: Target,
    executable: Path,
    bootstrap: Path,
) -> Path:
    layout = output_root / f"layout-{target.name}"
    _reset_owned_directory(layout, output_root)

    manifest = render_probe_manifest(target)
    (layout / "AppxManifest.xml").write_bytes(manifest)
    root = ET.fromstring(manifest)
    _copy_manifest_assets(layout, manifest_asset_paths(root))
    shutil.copy2(executable, layout / STORE_EXECUTABLE)
    shutil.copy2(bootstrap, layout / BOOTSTRAP_DLL)
    assert_layout_contract(layout, target)
    return layout


def _version_key(path: Path) -> tuple[int, ...]:
    try:
        return tuple(int(part) for part in path.parts[-3].split("."))
    except (ValueError, IndexError):
        return (0,)


def find_makeappx(explicit: Path | None = None) -> Path:
    candidates: list[Path] = []
    if explicit is not None:
        candidates.append(explicit)
    configured = os.environ.get("MAKEAPPX_EXE")
    if configured:
        candidates.append(Path(configured))
    if os.name == "nt":
        program_files_x86 = os.environ.get("ProgramFiles(x86)")
        if program_files_x86:
            candidates.extend(
                sorted(
                    Path(program_files_x86).glob(
                        "Windows Kits/10/bin/*/x64/MakeAppx.exe"
                    ),
                    key=_version_key,
                    reverse=True,
                )
            )
    else:
        candidates.extend(
            sorted(
                Path("/mnt/c/Program Files (x86)/Windows Kits/10/bin").glob(
                    "*/x64/MakeAppx.exe"
                ),
                key=_version_key,
                reverse=True,
            )
        )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise ProbeBuildError(
        "MakeAppx.exe was not found; install the Windows SDK or set MAKEAPPX_EXE"
    )


def windows_path(path: Path) -> str:
    absolute = path.resolve()
    if os.name == "nt":
        return str(absolute)
    parts = absolute.parts
    if len(parts) >= 4 and parts[1] == "mnt" and len(parts[2]) == 1:
        drive = parts[2].upper()
        return f"{drive}:\\{'\\'.join(parts[3:])}"
    raise ProbeBuildError(
        f"MakeAppx requires a Windows-accessible path under /mnt/<drive>: {absolute}"
    )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def assert_msix_contract(package: Path, target: Target) -> None:
    try:
        with zipfile.ZipFile(package) as archive:
            names = archive.namelist()
            names_folded = {name.casefold() for name in names}
            if "appxsignature.p7x" in names_folded:
                raise ProbeBuildError(f"probe package was unexpectedly signed: {package}")
            dlls = sorted(name for name in names if name.casefold().endswith(".dll"))
            if [name.casefold() for name in dlls] != [BOOTSTRAP_DLL.casefold()]:
                raise ProbeBuildError(
                    f"packaged probe contains a dual runtime or app-local AI DLL: {dlls!r}"
                )
            manifest_name = next(
                (name for name in names if name.casefold() == "appxmanifest.xml"), None
            )
            if manifest_name is None:
                raise ProbeBuildError(f"package contains no AppxManifest.xml: {package}")
            assert_manifest_contract(archive.read(manifest_name), target)
            executable_name = next(
                (name for name in names if name.casefold() == STORE_EXECUTABLE.casefold()),
                None,
            )
            bootstrap_name = next(
                (name for name in names if name.casefold() == BOOTSTRAP_DLL.casefold()),
                None,
            )
            if executable_name is None or bootstrap_name is None:
                raise ProbeBuildError(f"package is missing the Reactor payload: {package}")
            if pe_machine_bytes(archive.read(executable_name), executable_name) != target.pe_machine:
                raise ProbeBuildError(f"packaged executable architecture is wrong: {package}")
            if pe_machine_bytes(archive.read(bootstrap_name), bootstrap_name) != target.pe_machine:
                raise ProbeBuildError(f"packaged bootstrap architecture is wrong: {package}")
            bootstrap_hash = hashlib.sha256(archive.read(bootstrap_name)).hexdigest()
            if bootstrap_hash != target.bootstrap_sha256:
                raise ProbeBuildError(
                    f"packaged bootstrap is not Reactor's pinned 2.4 payload: {package}"
                )
    except (OSError, zipfile.BadZipFile) as error:
        raise ProbeBuildError(f"cannot inspect MSIX {package}: {error}") from error


def pack_msix(
    makeappx: Path,
    layout: Path,
    package: Path,
    target: Target,
) -> None:
    package.parent.mkdir(parents=True, exist_ok=True)
    if package.exists():
        package.unlink()
    run_command(
        [
            str(makeappx),
            "pack",
            "/d",
            windows_path(layout),
            "/p",
            windows_path(package),
            "/o",
        ],
        cwd=PROJECT_ROOT,
    )
    assert_msix_contract(package, target)


def pack_bundle(makeappx: Path, packages_dir: Path, bundle: Path) -> None:
    if bundle.exists():
        bundle.unlink()
    run_command(
        [
            str(makeappx),
            "bundle",
            "/d",
            windows_path(packages_dir),
            "/p",
            windows_path(bundle),
            "/o",
        ],
        cwd=PROJECT_ROOT,
    )
    try:
        with zipfile.ZipFile(bundle) as archive:
            names = archive.namelist()
            if any(name.casefold() == "appxsignature.p7x" for name in names):
                raise ProbeBuildError(f"probe bundle was unexpectedly signed: {bundle}")
            package_names = [name for name in names if name.casefold().endswith(".msix")]
            if len(package_names) != 2:
                raise ProbeBuildError(
                    f"probe bundle must contain x64 and ARM64 packages, got {package_names!r}"
                )
    except (OSError, zipfile.BadZipFile) as error:
        raise ProbeBuildError(f"cannot inspect bundle {bundle}: {error}") from error


def default_output_root() -> Path:
    configured = os.environ.get("WFDIAG_REACTOR_PROBE_OUTPUT")
    if configured:
        return Path(configured)
    if os.name == "nt":
        return PROJECT_ROOT / "artifacts" / "reactor-store-probe"
    return Path("/mnt/c/code/wfdiag-reactor-store-probe")


def build_probe(args: argparse.Namespace) -> Path:
    assert_reactor_dependency_contract()
    output_root = args.output.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    # Resolve this before a long Cargo build so a WSL invocation cannot finish
    # with payloads that MakeAppx cannot access.
    windows_path(output_root)
    makeappx = find_makeappx(args.makeappx)

    cargo_target_dir = args.cargo_target_dir.resolve()
    cargo_target_dir.mkdir(parents=True, exist_ok=True)
    packages_dir = output_root / "packages"
    _reset_owned_directory(packages_dir, output_root)

    version = canonical_version()
    payloads: dict[str, dict[str, object]] = {}
    for target in TARGETS.values():
        print(f"\n=== Building framework-dependent Reactor payload: {target.name} ===")
        executable, bootstrap = build_framework_dependent_payload(
            target, cargo_target_dir, not args.debug
        )
        layout = stage_layout(output_root, target, executable, bootstrap)
        package = packages_dir / (
            f"WindowsForum_Diagnostics_ReactorProbe_{version}_{target.name}.msix"
        )
        pack_msix(makeappx, layout, package, target)
        payloads[target.name] = {
            "target": target.triple,
            "layout": str(layout),
            "executable_sha256": _sha256(layout / STORE_EXECUTABLE),
            "bootstrap_sha256": _sha256(layout / BOOTSTRAP_DLL),
            "package": str(package),
            "package_sha256": _sha256(package),
        }

    bundle = output_root / f"WindowsForum_Diagnostics_ReactorProbe_{version}.msixbundle"
    pack_bundle(makeappx, packages_dir, bundle)

    report = {
        "schema_version": 1,
        "created_at_utc": datetime.now(timezone.utc).isoformat(),
        "purpose": "non-publishing Reactor Store/MSIX alignment probe",
        "signed": False,
        "installed": False,
        "published": False,
        "store_identity": STORE_IDENTITY_NAME,
        "store_publisher": STORE_PUBLISHER,
        "windows_app_runtime": {
            "framework": WINDOWS_APP_RUNTIME_FRAMEWORK,
            "minimum_version": WINDOWS_APP_RUNTIME_MIN_VERSION,
            "deployment": "framework-dependent",
            "app_local_runtime_present": False,
            "app_local_ai_dll_present": False,
            "staged_dlls": [BOOTSTRAP_DLL],
        },
        "payloads": payloads,
        "bundle": str(bundle),
        "bundle_sha256": _sha256(bundle),
    }
    (output_root / "probe-report.json").write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    (output_root / "NON-PUBLISHING-PROBE.txt").write_text(
        "This directory contains unsigned Reactor MSIX inspection artifacts.\n"
        "The build path did not sign, install, register, upload, or publish them.\n"
        "Do not submit these probe artifacts to the Microsoft Store.\n",
        encoding="utf-8",
    )

    print("\nReactor Store/MSIX probe complete (unsigned; not installed or published).")
    print(f"Bundle: {bundle}")
    print(f"Report: {output_root / 'probe-report.json'}")
    return bundle


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=default_output_root(),
        help="Windows-accessible probe output directory",
    )
    parser.add_argument(
        "--cargo-target-dir",
        type=Path,
        default=PROJECT_ROOT / "reactor-spike" / "target" / "framework-dependent-probe",
        help="dedicated Cargo target directory for the framework-dependent build",
    )
    parser.add_argument(
        "--makeappx",
        type=Path,
        help="explicit MakeAppx.exe path (otherwise auto-detected)",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="build debug payloads (release is the default)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    try:
        build_probe(parse_args(argv))
    except ProbeBuildError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
