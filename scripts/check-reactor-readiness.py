#!/usr/bin/env python3
"""Read-only cutover gate for the native Windows Reactor migration.

The checker intentionally exits non-zero while any migration prerequisite is
missing or unresolved.  It only reads repository files; it never creates,
updates, or deletes them.  The target UI is browserless: WebView/WebView2,
Tauri/Wry, and web-frontend assets are rejected from the Reactor prototype.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import tomllib
import xml.etree.ElementTree as ET
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Iterable


EXPECTED_REACTOR_REPOSITORY = "https://github.com/microsoft/windows-rs"
EXPECTED_REACTOR_REVISION = "1be5649497b59fe7cc2fb0ae5b0ebd7787327cc8"
EXPECTED_REACTOR_VERSION = "0.100.0"
EXPECTED_REACTOR_RUNTIME_RELEASE = "2.4.0"
EXPECTED_REACTOR_FRAMEWORK = "Microsoft.WindowsAppRuntime.2"
EXPECTED_SCHEMA_VERSION = 2

EXPECTED_UI_ARCHITECTURE = "native_winui3_reactor"
EXPECTED_UI_SOURCE_ROOT = "apps/wfdiag/src"
FORBIDDEN_UI_DEPENDENCIES = {
    "cef",
    "cef-sys",
    "chromiumoxide",
    "tauri",
    "tauri-runtime",
    "tauri-runtime-wry",
    "web-view",
    "webview2",
    "webview2-com",
    "wry",
}
FORBIDDEN_UI_SOURCE_MARKERS = (
    "corewebview2",
    "webview",
)
# The Processes reference fixture is allowed to contain the executable name
# captured from the shipping app.  It is host process data, not a browser UI
# API or dependency.  Remove only this exact literal before scanning the
# surrounding Rust source so genuine WebView identifiers are still blocked.
ALLOWED_UI_SOURCE_DATA_LITERALS = (
    '"msedgewebview2.exe"',
)
FORBIDDEN_WEB_ASSET_SUFFIXES = {
    ".css",
    ".htm",
    ".html",
    ".js",
    ".jsx",
    ".svelte",
    ".ts",
    ".tsx",
    ".vue",
}

EXPECTED_IDENTITY_NAME = "32827MikeFara.WindowsForumDiagnostics"
EXPECTED_PUBLISHER = "CN=ABDB6B3F-DF9E-447D-BC0E-4DA7BAFD14C4"
EXPECTED_PUBLISHER_DISPLAY_NAME = "WindowsForum.com"
EXPECTED_CAPABILITIES = {
    "internetClient",
    "internetClientServer",
    "privateNetworkClientServer",
    "runFullTrust",
    "systemAIModels",
}

REQUIRED_UPSTREAM_GATES = {"window_lifecycle", "global_accelerators"}
REQUIRED_CUTOVER_GATES = {
    "official_reactor_release",
    "current_baseline_capture",
    "native_control_parity",
    "aion_store_validation",
    "store_packaging_validation",
    "direct_distribution_validation",
}
REQUIRED_ACCEPTANCE_CATEGORIES = {
    "information_architecture",
    "content_and_state",
    "brand_and_materials",
    "layout_and_responsiveness",
    "typography_and_iconography",
    "motion_and_feedback",
    "accessibility",
    "data_visualization",
}
REQUIRED_BACKEND_SURFACES = {
    "ai_chat",
    "ai_provider_management",
    "ai_report",
    "architecture_and_system_info",
    "diagnostics_scan",
    "elevation_and_external_links",
    "export_and_share",
    "file_dialog_clipboard_notifications",
    "global_commands_and_shortcuts",
    "history_storage",
    "issue_detection",
    "live_monitoring",
    "on_device_ai_and_package_identity",
    "process_inventory",
    "remediation_and_action_broker",
    "settings_and_credentials",
    "single_instance",
    "tray_and_window_lifecycle",
    "update_check",
}


@dataclass(frozen=True)
class Finding:
    code: str
    severity: str
    message: str
    details: dict[str, Any] = field(default_factory=dict)


@dataclass
class ReadinessReport:
    root: str
    manifest: str
    findings: list[Finding] = field(default_factory=list)

    def add(
        self,
        code: str,
        severity: str,
        message: str,
        **details: Any,
    ) -> None:
        self.findings.append(Finding(code, severity, message, details))

    @property
    def ready(self) -> bool:
        return not any(
            finding.severity in {"blocker", "error"}
            for finding in self.findings
        )

    def to_dict(self) -> dict[str, Any]:
        counts = {
            severity: sum(f.severity == severity for f in self.findings)
            for severity in ("pass", "warning", "blocker", "error")
        }
        return {
            "ready": self.ready,
            "root": self.root,
            "manifest": self.manifest,
            "counts": counts,
            "findings": [asdict(finding) for finding in self.findings],
        }


def _read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _safe_repo_path(root: Path, relative: object) -> Path:
    if not isinstance(relative, str) or not relative:
        raise ValueError("path must be a non-empty string")
    value = Path(relative)
    if value.is_absolute():
        raise ValueError(f"absolute path is not allowed: {relative}")
    root_resolved = root.resolve()
    resolved = (root / value).resolve()
    try:
        resolved.relative_to(root_resolved)
    except ValueError as error:
        raise ValueError(f"path escapes repository root: {relative}") from error
    return resolved


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _png_dimensions(path: Path) -> tuple[int, int]:
    with path.open("rb") as handle:
        header = handle.read(24)
    if len(header) < 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG file")
    if header[12:16] != b"IHDR":
        raise ValueError("PNG does not start with an IHDR chunk")
    return struct.unpack(">II", header[16:24])


def _local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _elements_named(root: ET.Element, name: str) -> Iterable[ET.Element]:
    return (element for element in root.iter() if _local_name(element.tag) == name)


def _check_manifest_contract(
    report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    if manifest.get("schema_version") != EXPECTED_SCHEMA_VERSION:
        report.add(
            "manifest.schema",
            "error",
            f"reactor baseline manifest must use schema_version {EXPECTED_SCHEMA_VERSION}",
            actual=manifest.get("schema_version"),
        )

    architecture = manifest.get("ui_architecture")
    expected_architecture = {
        "kind": EXPECTED_UI_ARCHITECTURE,
        "source_root": EXPECTED_UI_SOURCE_ROOT,
        "webview_ui_allowed": False,
    }
    if architecture != expected_architecture:
        report.add(
            "ui.contract",
            "error",
            "Native Reactor UI contract drifted; WebView rendering is not permitted",
            expected=expected_architecture,
            actual=architecture,
        )
    else:
        report.add(
            "ui.contract",
            "pass",
            "Readiness manifest requires browserless native WinUI 3 controls",
        )

    pin = manifest.get("reactor_pin")
    if not isinstance(pin, dict):
        report.add("reactor.pin", "error", "reactor_pin object is missing")
        return

    expected_pin = {
        "repository": EXPECTED_REACTOR_REPOSITORY,
        "revision": EXPECTED_REACTOR_REVISION,
        "expected_crate_version": EXPECTED_REACTOR_VERSION,
        "windows_app_runtime_release": EXPECTED_REACTOR_RUNTIME_RELEASE,
        "windows_app_runtime_framework": EXPECTED_REACTOR_FRAMEWORK,
    }
    mismatches = {
        key: {"expected": expected, "actual": pin.get(key)}
        for key, expected in expected_pin.items()
        if pin.get(key) != expected
    }
    if mismatches:
        report.add(
            "reactor.pin",
            "error",
            "Reactor source/version expectation drifted",
            mismatches=mismatches,
        )
    else:
        report.add(
            "reactor.pin",
            "pass",
            "Reactor prototype expectation is pinned to the reviewed revision",
            revision=EXPECTED_REACTOR_REVISION,
            expected_crate_version=EXPECTED_REACTOR_VERSION,
        )

    identity = manifest.get("store_identity")
    expected_identity = {
        "name": EXPECTED_IDENTITY_NAME,
        "publisher": EXPECTED_PUBLISHER,
        "publisher_display_name": EXPECTED_PUBLISHER_DISPLAY_NAME,
        "capabilities": sorted(EXPECTED_CAPABILITIES),
    }
    if not isinstance(identity, dict):
        report.add(
            "store.contract", "error", "store_identity object is missing"
        )
    else:
        actual_identity = dict(identity)
        if isinstance(actual_identity.get("capabilities"), list):
            actual_identity["capabilities"] = sorted(actual_identity["capabilities"])
        if actual_identity != expected_identity:
            report.add(
                "store.contract",
                "error",
                "Protected Store identity contract drifted in the readiness manifest",
                expected=expected_identity,
                actual=actual_identity,
            )
        else:
            report.add(
                "store.contract",
                "pass",
                "Readiness manifest preserves the Store identity contract",
            )


def _check_reactor_prototype(
    root: Path, report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    reactor = manifest.get("reactor_pin", {})
    relative = reactor.get("prototype_manifest", "apps/wfdiag/Cargo.toml")
    try:
        cargo_path = _safe_repo_path(root, relative)
    except ValueError as error:
        report.add("reactor.prototype", "error", str(error))
        return
    if not cargo_path.is_file():
        report.add(
            "reactor.prototype",
            "blocker",
            "Pinned Reactor prototype manifest is missing",
            path=str(relative),
        )
        return

    try:
        cargo = tomllib.loads(cargo_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        report.add(
            "reactor.prototype",
            "error",
            "Could not parse Reactor prototype Cargo.toml",
            path=str(relative),
            error=str(error),
        )
        return

    dependencies = (
        ("dependencies.windows-reactor", cargo.get("dependencies", {}).get("windows-reactor")),
        (
            "build-dependencies.windows-reactor-setup",
            cargo.get("build-dependencies", {}).get("windows-reactor-setup"),
        ),
    )
    problems: dict[str, Any] = {}
    for label, dependency in dependencies:
        if not isinstance(dependency, dict):
            problems[label] = "must be a git dependency table"
            continue
        dependency_problems: dict[str, Any] = {}
        if dependency.get("git") != EXPECTED_REACTOR_REPOSITORY:
            dependency_problems["git"] = dependency.get("git")
        if dependency.get("rev") != EXPECTED_REACTOR_REVISION:
            dependency_problems["rev"] = dependency.get("rev")
        for floating_key in ("branch", "tag"):
            if floating_key in dependency:
                dependency_problems[floating_key] = dependency[floating_key]
        if dependency_problems:
            problems[label] = dependency_problems

    if problems:
        report.add(
            "reactor.prototype",
            "blocker",
            "Reactor prototype dependencies are not pinned to the reviewed revision",
            path=str(relative),
            problems=problems,
        )
    else:
        report.add(
            "reactor.prototype",
            "pass",
            "Reactor and reactor-setup prototype dependencies use the exact reviewed revision",
            path=str(relative),
        )


def _dependency_tables(
    value: object, path: tuple[str, ...] = ()
) -> Iterable[tuple[str, dict[str, Any]]]:
    """Yield Cargo dependency tables, including target-specific tables."""

    if not isinstance(value, dict):
        return
    for key, child in value.items():
        child_path = (*path, str(key))
        if key in {"dependencies", "dev-dependencies", "build-dependencies"}:
            if isinstance(child, dict):
                yield (".".join(child_path), child)
            continue
        yield from _dependency_tables(child, child_path)


def _dependency_names(name: str, specification: object) -> set[str]:
    names = {name.lower().replace("_", "-")}
    if isinstance(specification, dict):
        package = specification.get("package")
        if isinstance(package, str):
            names.add(package.lower().replace("_", "-"))
        git = specification.get("git")
        if isinstance(git, str):
            names.add(git.rstrip("/").rsplit("/", 1)[-1].lower().replace("_", "-"))
    return names


def _forbidden_dependency_matches(identities: set[str]) -> list[str]:
    return sorted(
        forbidden
        for forbidden in FORBIDDEN_UI_DEPENDENCIES
        if any(
            identity == forbidden or identity.startswith(f"{forbidden}-")
            for identity in identities
        )
    )


def _check_native_ui(
    root: Path, report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    """Reject browser-hosted UI code from the native Reactor candidate."""

    reactor = manifest.get("reactor_pin", {})
    relative_manifest = reactor.get("prototype_manifest", "apps/wfdiag/Cargo.toml")
    try:
        cargo_path = _safe_repo_path(root, relative_manifest)
        cargo = tomllib.loads(cargo_path.read_text(encoding="utf-8"))
        source_root = _safe_repo_path(root, EXPECTED_UI_SOURCE_ROOT)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        report.add(
            "ui.native",
            "error",
            "Could not inspect the native Reactor UI implementation",
            error=str(error),
        )
        return

    dependency_hits: list[dict[str, str]] = []
    for table_name, dependencies in _dependency_tables(cargo):
        for name, specification in dependencies.items():
            identities = _dependency_names(str(name), specification)
            matches = _forbidden_dependency_matches(identities)
            if matches:
                dependency_hits.append(
                    {
                        "table": table_name,
                        "dependency": str(name),
                        "matched": ", ".join(matches),
                    }
                )

    if not source_root.is_dir():
        report.add(
            "ui.native",
            "blocker",
            "Native Reactor source root is missing",
            path=EXPECTED_UI_SOURCE_ROOT,
        )
        return

    source_hits: list[dict[str, Any]] = []
    web_assets: list[str] = []
    rust_sources = sorted(source_root.rglob("*.rs"))
    for path in sorted(source_root.rglob("*")):
        if path.is_file() and path.suffix.lower() in FORBIDDEN_WEB_ASSET_SUFFIXES:
            web_assets.append(str(path.relative_to(root)))
    for path in rust_sources:
        try:
            for line_number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                lowered = line.lower()
                marker_source = lowered
                for literal in ALLOWED_UI_SOURCE_DATA_LITERALS:
                    marker_source = marker_source.replace(literal, "")
                matches = [
                    marker
                    for marker in FORBIDDEN_UI_SOURCE_MARKERS
                    if marker in marker_source
                ]
                if matches:
                    source_hits.append(
                        {
                            "path": str(path.relative_to(root)),
                            "line": line_number,
                            "markers": matches,
                        }
                    )
        except (OSError, UnicodeError) as error:
            source_hits.append(
                {"path": str(path.relative_to(root)), "error": str(error)}
            )

    native_entrypoint = any(
        "windows_reactor" in path.read_text(encoding="utf-8", errors="ignore")
        for path in rust_sources
    )
    problems: dict[str, Any] = {}
    if dependency_hits:
        problems["forbidden_dependencies"] = dependency_hits
    if source_hits:
        problems["forbidden_source_markers"] = source_hits
    if web_assets:
        problems["web_frontend_assets"] = web_assets
    if not rust_sources:
        problems["rust_sources"] = "no Rust source files found"
    elif not native_entrypoint:
        problems["native_entrypoint"] = "windows_reactor is not referenced by the UI source"

    if problems:
        report.add(
            "ui.native",
            "blocker",
            "Reactor candidate is not a browserless native WinUI 3 implementation",
            source_root=EXPECTED_UI_SOURCE_ROOT,
            problems=problems,
        )
    else:
        report.add(
            "ui.native",
            "pass",
            "Reactor candidate uses native Rust/WinUI controls with no WebView UI dependency",
            source_root=EXPECTED_UI_SOURCE_ROOT,
            rust_source_count=len(rust_sources),
        )


def _check_baselines(
    root: Path, report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    baseline = manifest.get("baseline")
    if not isinstance(baseline, dict):
        report.add("baseline.contract", "error", "baseline object is missing")
        return

    current_version: str | None = None
    try:
        version_path = _safe_repo_path(root, baseline.get("version_file", "version.json"))
        current_version = str(_read_json(version_path)["version"])
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        report.add(
            "baseline.version",
            "error",
            "Could not read the current application version",
            error=str(error),
        )

    capture_version = baseline.get("application_version")
    if current_version is not None:
        if capture_version != current_version:
            report.add(
                "baseline.version",
                "blocker",
                "Visual baselines must be recaptured from the current shipping UI",
                captured_version=capture_version,
                current_version=current_version,
            )
        else:
            report.add(
                "baseline.version",
                "pass",
                "Visual baselines match the current application version",
                version=current_version,
            )

    source_package = baseline.get("source_package")
    source_package_errors: list[str] = []
    required_package_fields = {
        "name",
        "package_full_name",
        "package_family_name",
        "version",
        "architecture",
        "signature_kind",
    }
    if not isinstance(source_package, dict):
        source_package_errors.append("source_package object is missing")
        source_package = {}
    else:
        missing_package_fields = sorted(required_package_fields - set(source_package))
        if missing_package_fields:
            source_package_errors.append(
                f"missing fields: {', '.join(missing_package_fields)}"
            )
        if source_package.get("name") != EXPECTED_IDENTITY_NAME:
            source_package_errors.append("package identity name does not match the Store app")
        if source_package.get("signature_kind") != "Store":
            source_package_errors.append("capture source is not Store-signed")
        if current_version is not None and source_package.get("version") != f"{current_version}.0":
            source_package_errors.append("package version does not match version.json")
        if source_package.get("architecture") not in {"Arm64", "X64"}:
            source_package_errors.append("capture architecture must be Arm64 or X64")

    if source_package_errors:
        report.add(
            "baseline.source_package",
            "blocker",
            "Store baseline package provenance is incomplete or inconsistent",
            failures=source_package_errors,
        )
    else:
        report.add(
            "baseline.source_package",
            "pass",
            "Baseline provenance identifies the installed Store-signed package",
            package_full_name=source_package["package_full_name"],
            architecture=source_package["architecture"],
        )

    screenshots = baseline.get("screenshots")
    if not isinstance(screenshots, list) or not screenshots:
        report.add(
            "baseline.screenshots", "error", "baseline screenshots are missing"
        )
        screenshots = []

    screenshot_errors: list[dict[str, Any]] = []
    capture_metadata_errors: list[dict[str, Any]] = []
    stale_screenshots: list[dict[str, str]] = []
    seen_ids: set[str] = set()
    required_fields = {
        "id",
        "path",
        "screen",
        "state",
        "theme",
        "viewport",
        "sha256",
        "source_application_version",
    }
    for index, entry in enumerate(screenshots):
        if not isinstance(entry, dict):
            screenshot_errors.append({"index": index, "error": "entry is not an object"})
            continue
        missing = sorted(required_fields - set(entry))
        if missing:
            screenshot_errors.append(
                {"index": index, "id": entry.get("id"), "missing": missing}
            )
            continue
        entry_id = entry["id"]
        if not isinstance(entry_id, str) or entry_id in seen_ids:
            screenshot_errors.append(
                {"index": index, "id": entry_id, "error": "id is invalid or duplicated"}
            )
            continue
        seen_ids.add(entry_id)
        source_version = entry["source_application_version"]
        if not isinstance(source_version, str) or not source_version.strip():
            screenshot_errors.append(
                {
                    "id": entry_id,
                    "error": "source_application_version must be a non-empty string",
                }
            )
        elif current_version is not None and source_version != current_version:
            stale_screenshots.append(
                {
                    "id": entry_id,
                    "source_application_version": source_version,
                }
            )
        viewport = entry["viewport"]
        if (
            not isinstance(viewport, dict)
            or not isinstance(viewport.get("width"), int)
            or not isinstance(viewport.get("height"), int)
        ):
            screenshot_errors.append(
                {"id": entry_id, "error": "viewport must contain integer width and height"}
            )
            continue
        try:
            path = _safe_repo_path(root, entry["path"])
            if not path.is_file():
                raise FileNotFoundError(str(path))
            actual_hash = _sha256(path)
            actual_dimensions = _png_dimensions(path)
            expected_dimensions = (viewport["width"], viewport["height"])
            if actual_hash != entry["sha256"]:
                screenshot_errors.append(
                    {"id": entry_id, "error": "sha256 mismatch", "actual": actual_hash}
                )
            if actual_dimensions != expected_dimensions:
                screenshot_errors.append(
                    {
                        "id": entry_id,
                        "error": "viewport mismatch",
                        "expected": expected_dimensions,
                        "actual": actual_dimensions,
                    }
                )
        except (OSError, ValueError) as error:
            screenshot_errors.append({"id": entry_id, "error": str(error)})

        metadata_ref = entry.get("capture_metadata")
        if not isinstance(metadata_ref, dict) or not {"path", "sha256"} <= set(
            metadata_ref
        ):
            capture_metadata_errors.append(
                {"id": entry_id, "error": "capture_metadata path/hash are required"}
            )
            continue
        try:
            metadata_path = _safe_repo_path(root, metadata_ref["path"])
            if not metadata_path.is_file():
                raise FileNotFoundError(str(metadata_path))
            metadata_hash = _sha256(metadata_path)
            if metadata_hash != metadata_ref["sha256"]:
                capture_metadata_errors.append(
                    {
                        "id": entry_id,
                        "error": "capture metadata sha256 mismatch",
                        "actual": metadata_hash,
                    }
                )
            metadata = json.loads(metadata_path.read_text(encoding="utf-8-sig"))
            expected_metadata = {
                "state": entry_id,
                "package_full_name": source_package.get("package_full_name"),
                "package_family_name": source_package.get("package_family_name"),
                "package_version": source_package.get("version"),
                "architecture": source_package.get("architecture"),
                "signature_kind": source_package.get("signature_kind"),
                "logical_width": viewport["width"],
                "logical_height": viewport["height"],
            }
            mismatches = {
                key: {"expected": expected, "actual": metadata.get(key)}
                for key, expected in expected_metadata.items()
                if metadata.get(key) != expected
            }
            scale = metadata.get("scale")
            dpi = metadata.get("dpi")
            physical_width = metadata.get("physical_visible_width")
            physical_height = metadata.get("physical_visible_height")
            if not isinstance(scale, (int, float)) or scale <= 0:
                mismatches["scale"] = {"expected": "positive number", "actual": scale}
            elif (
                not isinstance(dpi, int)
                or dpi != round(96 * scale)
                or physical_width != round(viewport["width"] * scale)
                or physical_height != round(viewport["height"] * scale)
            ):
                mismatches["dpi_physical_geometry"] = {
                    "expected": {
                        "dpi": round(96 * scale),
                        "physical_visible_width": round(viewport["width"] * scale),
                        "physical_visible_height": round(viewport["height"] * scale),
                    },
                    "actual": {
                        "dpi": dpi,
                        "physical_visible_width": physical_width,
                        "physical_visible_height": physical_height,
                    },
                }
            if mismatches:
                capture_metadata_errors.append(
                    {"id": entry_id, "error": "capture metadata mismatch", "fields": mismatches}
                )
        except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
            capture_metadata_errors.append({"id": entry_id, "error": str(error)})

    if screenshot_errors:
        report.add(
            "baseline.screenshots",
            "blocker",
            "One or more visual baseline files are missing or changed",
            failures=screenshot_errors,
        )
    else:
        report.add(
            "baseline.screenshots",
            "pass",
            "All visual baseline files match their dimensions and checksums",
            count=len(screenshots),
        )

    if capture_metadata_errors:
        report.add(
            "baseline.capture_metadata",
            "blocker",
            "One or more Store captures lack trustworthy package/DPI provenance",
            failures=capture_metadata_errors,
        )
    else:
        report.add(
            "baseline.capture_metadata",
            "pass",
            "Every Store baseline has checksum-pinned package and DPI provenance",
            count=len(screenshots),
        )

    if current_version is not None and screenshots:
        if stale_screenshots:
            report.add(
                "baseline.screenshot_versions",
                "blocker",
                "One or more visual baseline states were captured from an older application version",
                current_version=current_version,
                current_count=len(screenshots) - len(stale_screenshots),
                stale_count=len(stale_screenshots),
                stale=stale_screenshots,
            )
        else:
            report.add(
                "baseline.screenshot_versions",
                "pass",
                "Every visual baseline state was captured from the current application version",
                version=current_version,
                count=len(screenshots),
            )

    asset_errors: list[dict[str, Any]] = []
    assets = baseline.get("assets")
    if not isinstance(assets, list) or not assets:
        asset_errors.append({"error": "asset list is missing"})
        assets = []
    for entry in assets:
        if not isinstance(entry, dict) or not {"id", "path", "sha256"} <= set(entry):
            asset_errors.append({"entry": entry, "error": "invalid asset entry"})
            continue
        try:
            path = _safe_repo_path(root, entry["path"])
            if not path.is_file():
                raise FileNotFoundError(str(path))
            actual_hash = _sha256(path)
            if actual_hash != entry["sha256"]:
                asset_errors.append(
                    {"id": entry["id"], "error": "sha256 mismatch", "actual": actual_hash}
                )
        except (OSError, ValueError) as error:
            asset_errors.append({"id": entry.get("id"), "error": str(error)})
    if asset_errors:
        report.add(
            "baseline.assets",
            "blocker",
            "One or more protected brand assets are missing or changed",
            failures=asset_errors,
        )
    else:
        report.add(
            "baseline.assets",
            "pass",
            "All protected brand assets match their checksums",
            count=len(assets),
        )

    raw_categories = manifest.get("visual_acceptance_categories")
    categories = raw_categories if isinstance(raw_categories, list) else []
    category_ids = {
        entry.get("id")
        for entry in categories
        if isinstance(entry, dict) and isinstance(entry.get("id"), str)
    }
    missing_categories = sorted(REQUIRED_ACCEPTANCE_CATEGORIES - category_ids)
    invalid_categories = [
        entry.get("id") if isinstance(entry, dict) else None
        for entry in categories
        if not isinstance(entry, dict)
        or not isinstance(entry.get("acceptance"), str)
        or not entry.get("acceptance", "").strip()
    ]
    if not isinstance(raw_categories, list) or missing_categories or invalid_categories:
        report.add(
            "baseline.acceptance",
            "error",
            "Visual acceptance categories are incomplete",
            missing=missing_categories,
            invalid=invalid_categories,
        )
    else:
        report.add(
            "baseline.acceptance",
            "pass",
            "Visual acceptance categories cover every required review dimension",
            count=len(category_ids),
        )


def _check_store_manifest(
    root: Path, report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    relative = manifest.get("store_manifest", "AppxManifest.xml")
    try:
        appx_path = _safe_repo_path(root, relative)
        appx_root = ET.parse(appx_path).getroot()
    except (OSError, ValueError, ET.ParseError) as error:
        report.add(
            "store.manifest",
            "error",
            "Could not parse the Store AppxManifest.xml",
            error=str(error),
        )
        return

    identities = list(_elements_named(appx_root, "Identity"))
    publisher_names = list(_elements_named(appx_root, "PublisherDisplayName"))
    actual_identity = {
        "name": identities[0].get("Name") if len(identities) == 1 else None,
        "publisher": identities[0].get("Publisher") if len(identities) == 1 else None,
        "publisher_display_name": (
            publisher_names[0].text.strip()
            if len(publisher_names) == 1 and publisher_names[0].text
            else None
        ),
    }
    expected_identity = {
        "name": EXPECTED_IDENTITY_NAME,
        "publisher": EXPECTED_PUBLISHER,
        "publisher_display_name": EXPECTED_PUBLISHER_DISPLAY_NAME,
    }
    if actual_identity != expected_identity:
        report.add(
            "store.identity",
            "blocker",
            "Store identity or publisher changed",
            expected=expected_identity,
            actual=actual_identity,
        )
    else:
        report.add(
            "store.identity",
            "pass",
            "Store identity and publisher are unchanged",
        )

    capabilities = {
        element.get("Name")
        for element in appx_root.iter()
        if _local_name(element.tag) == "Capability" and element.get("Name")
    }
    if capabilities != EXPECTED_CAPABILITIES:
        report.add(
            "store.capabilities",
            "blocker",
            "Store capabilities changed",
            expected=sorted(EXPECTED_CAPABILITIES),
            actual=sorted(capabilities),
            missing=sorted(EXPECTED_CAPABILITIES - capabilities),
            unexpected=sorted(capabilities - EXPECTED_CAPABILITIES),
        )
    else:
        report.add(
            "store.capabilities",
            "pass",
            "Store capabilities are unchanged, including systemAIModels",
        )

    runtime_dependencies = [
        element.get("Name")
        for element in _elements_named(appx_root, "PackageDependency")
        if (element.get("Name") or "").startswith("Microsoft.WindowsAppRuntime.")
    ]
    target_framework = manifest.get("reactor_pin", {}).get(
        "windows_app_runtime_framework"
    )
    target_release = manifest.get("reactor_pin", {}).get(
        "windows_app_runtime_release"
    )
    if len(runtime_dependencies) != 1:
        report.add(
            "runtime.alignment",
            "blocker",
            "Store manifest must declare exactly one Windows App Runtime framework",
            actual=runtime_dependencies,
            reactor_target=target_framework,
        )
    elif runtime_dependencies[0] != target_framework:
        report.add(
            "runtime.alignment",
            "blocker",
            "Store uses Windows App Runtime 1.8 while the pinned Reactor revision stages 2.4; do not cut over until one runtime strategy passes Store and on-device AI validation",
            store_framework=runtime_dependencies[0],
            reactor_framework=target_framework,
            reactor_runtime_release=target_release,
        )
    else:
        report.add(
            "runtime.alignment",
            "pass",
            "Store and Reactor use the same Windows App Runtime framework",
            framework=target_framework,
            reactor_runtime_release=target_release,
        )


def _check_named_gates(
    report: ReadinessReport,
    manifest: dict[str, Any],
    field_name: str,
    required_ids: set[str],
    code_prefix: str,
) -> None:
    raw_gates = manifest.get(field_name)
    if not isinstance(raw_gates, list):
        report.add(
            f"{code_prefix}.contract",
            "error",
            f"{field_name} must be a list",
        )
        return
    gates = {
        gate.get("id"): gate
        for gate in raw_gates
        if isinstance(gate, dict) and isinstance(gate.get("id"), str)
    }
    missing = sorted(required_ids - set(gates))
    if missing:
        report.add(
            f"{code_prefix}.contract",
            "error",
            "Required readiness gates are not represented",
            missing=missing,
        )
    else:
        report.add(
            f"{code_prefix}.contract",
            "pass",
            "All required readiness gates are represented",
            gates=sorted(required_ids),
        )

    for gate_id in sorted(required_ids & set(gates)):
        gate = gates[gate_id]
        status = gate.get("status")
        evidence = gate.get("evidence")
        if status == "passed" and isinstance(evidence, list) and evidence:
            report.add(
                f"{code_prefix}.{gate_id}",
                "pass",
                gate.get("summary", f"{gate_id} gate passed"),
                evidence=evidence,
            )
        elif status == "passed":
            report.add(
                f"{code_prefix}.{gate_id}",
                "error",
                "A passed gate must include non-empty evidence",
            )
        elif status == "blocked":
            report.add(
                f"{code_prefix}.{gate_id}",
                "blocker",
                gate.get("summary", f"{gate_id} gate remains blocked"),
                evidence=evidence or [],
            )
        else:
            report.add(
                f"{code_prefix}.{gate_id}",
                "error",
                "Gate status must be 'blocked' or 'passed'",
                actual=status,
            )


def _check_backend_parity(
    report: ReadinessReport, manifest: dict[str, Any]
) -> None:
    """Require an explicit, evidence-backed disposition for every app service.

    Visual parity alone cannot prove that the native shell is a production
    replacement.  Keeping this matrix in the protected readiness manifest
    prevents fixture-only screens from being mistaken for feature-complete
    backend integration.
    """

    raw_surfaces = manifest.get("backend_parity")
    if not isinstance(raw_surfaces, list):
        report.add(
            "backend.contract",
            "error",
            "backend_parity must be a list",
        )
        return

    entries = [entry for entry in raw_surfaces if isinstance(entry, dict)]
    ids = [entry.get("id") for entry in entries if isinstance(entry.get("id"), str)]
    represented = set(ids)
    duplicates = sorted({surface_id for surface_id in ids if ids.count(surface_id) > 1})
    missing = sorted(REQUIRED_BACKEND_SURFACES - represented)
    unexpected = sorted(represented - REQUIRED_BACKEND_SURFACES)
    malformed = len(entries) != len(raw_surfaces)

    if missing or unexpected or duplicates or malformed:
        report.add(
            "backend.contract",
            "error",
            "Native backend parity surfaces are incomplete or malformed",
            missing=missing,
            unexpected=unexpected,
            duplicates=duplicates,
            malformed_entries=len(raw_surfaces) - len(entries),
        )
    else:
        report.add(
            "backend.contract",
            "pass",
            "Every required native backend surface is represented",
            surfaces=sorted(REQUIRED_BACKEND_SURFACES),
        )

    invalid: list[dict[str, Any]] = []
    incomplete: list[dict[str, str]] = []
    for entry in entries:
        surface_id = entry.get("id")
        if surface_id not in REQUIRED_BACKEND_SURFACES:
            continue
        status = entry.get("status")
        summary = entry.get("summary")
        evidence = entry.get("evidence")
        if status not in {"blocked", "partial", "passed"}:
            invalid.append(
                {"id": surface_id, "error": "status must be blocked, partial, or passed"}
            )
            continue
        if not isinstance(summary, str) or not summary.strip():
            invalid.append({"id": surface_id, "error": "summary must be non-empty"})
        if not isinstance(evidence, list) or any(
            not isinstance(item, str) or not item.strip() for item in evidence
        ):
            invalid.append({"id": surface_id, "error": "evidence must be a string list"})
        elif status in {"partial", "passed"} and not evidence:
            invalid.append(
                {"id": surface_id, "error": f"{status} status requires evidence"}
            )
        if status != "passed":
            incomplete.append({"id": surface_id, "status": status})

    if invalid:
        report.add(
            "backend.parity",
            "error",
            "Native backend parity entries are invalid",
            failures=invalid,
        )
    elif incomplete:
        report.add(
            "backend.parity",
            "blocker",
            "The Reactor shell is not yet feature-complete against the shipping backend",
            passed=len(REQUIRED_BACKEND_SURFACES) - len(incomplete),
            required=len(REQUIRED_BACKEND_SURFACES),
            incomplete=sorted(incomplete, key=lambda item: item["id"]),
        )
    else:
        report.add(
            "backend.parity",
            "pass",
            "Every shipping backend surface has native Reactor integration evidence",
            count=len(REQUIRED_BACKEND_SURFACES),
        )


def evaluate_readiness(root: Path, manifest_path: Path | None = None) -> ReadinessReport:
    """Evaluate repository state without modifying it."""

    root = root.resolve()
    manifest_path = (
        manifest_path.resolve()
        if manifest_path is not None
        else root / "reactor-baselines" / "manifest.json"
    )
    report = ReadinessReport(str(root), str(manifest_path))
    try:
        manifest = _read_json(manifest_path)
    except (OSError, json.JSONDecodeError) as error:
        report.add(
            "manifest.load",
            "error",
            "Could not load the Reactor readiness manifest",
            error=str(error),
        )
        return report
    if not isinstance(manifest, dict):
        report.add("manifest.load", "error", "Readiness manifest must be an object")
        return report

    _check_manifest_contract(report, manifest)
    _check_reactor_prototype(root, report, manifest)
    _check_native_ui(root, report, manifest)
    _check_baselines(root, report, manifest)
    _check_store_manifest(root, report, manifest)
    _check_backend_parity(report, manifest)
    _check_named_gates(
        report,
        manifest,
        "upstream_api_gates",
        REQUIRED_UPSTREAM_GATES,
        "upstream",
    )
    _check_named_gates(
        report,
        manifest,
        "cutover_gates",
        REQUIRED_CUTOVER_GATES,
        "cutover",
    )
    return report


def _print_human(report: ReadinessReport) -> None:
    for finding in report.findings:
        marker = {
            "pass": "PASS",
            "warning": "WARN",
            "blocker": "BLOCK",
            "error": "ERROR",
        }[finding.severity]
        print(f"[{marker}] {finding.code}: {finding.message}")
        for key, value in finding.details.items():
            print(f"        {key}: {json.dumps(value, sort_keys=True)}")
    print()
    if report.ready:
        print("READY: every automated native Reactor cutover gate passed.")
    else:
        print("NOT READY: the shipping Tauri frontend must remain the production UI.")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (defaults to the script's parent repository)",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="manifest path (defaults to reactor-baselines/manifest.json under --root)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit the complete report as JSON",
    )
    args = parser.parse_args(argv)
    manifest_path = args.manifest
    if manifest_path is not None and not manifest_path.is_absolute():
        manifest_path = args.root / manifest_path
    report = evaluate_readiness(args.root, manifest_path)
    if args.json:
        print(json.dumps(report.to_dict(), indent=2, sort_keys=True))
    else:
        _print_human(report)
    return 0 if report.ready else 1


if __name__ == "__main__":
    sys.exit(main())
