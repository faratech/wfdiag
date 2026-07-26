#!/usr/bin/env python3
"""Safely migrate a Partner Center draft into an API-owned Store submission.

Microsoft does not allow the Store submission API to update, delete, or commit
a draft that was created or edited in Partner Center.  This tool therefore
implements the supported migration:

1. Snapshot the writable metadata and, when necessary, embed authenticated
   copies of recoverable Store screenshots.
2. Wait for a human to delete the original draft in Partner Center.
3. Create an API-owned clone of the last published submission.
4. Restore the snapshotted metadata, replace the package, upload, and commit.

The snapshot intentionally excludes fileUploadUrl and every other server-owned
field.  The workflow that calls this script encrypts the self-contained
snapshot before retaining it as a GitHub Actions artifact.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import copy
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
import zipfile


API_ROOT = "https://manage.devcenter.microsoft.com/v1.0/my/applications"
TOKEN_RESOURCE = "https://manage.devcenter.microsoft.com"
SNAPSHOT_SCHEMA_VERSION = 2
DEFAULT_PORTAL_PRODUCT = "9NJ59RH053PV"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_SCREENSHOT_COUNT = 50
MAX_SCREENSHOT_BYTES = 20 * 1024 * 1024
MAX_SCREENSHOT_TOTAL_BYTES = 100 * 1024 * 1024
SAFE_SCREENSHOT_NAME = re.compile(
    r"[A-Za-z0-9][A-Za-z0-9._-]{0,118}\.png"
)
SAFE_ARCHIVE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,159}")
WINDOWS_RESERVED_NAMES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}

TOP_LEVEL_METADATA_FIELDS = (
    "applicationCategory",
    "visibility",
    "targetPublishMode",
    "targetPublishDate",
    "hardwarePreferences",
    "automaticBackupEnabled",
    "canInstallOnRemovableMedia",
    "isGameDvrEnabled",
    "gamingOptions",
    "hasExternalInAppProducts",
    "meetAccessibilityGuidelines",
    "notesForCertification",
    "enterpriseLicensing",
    "allowMicrosoftDecideAppAvailabilityToFutureDeviceFamilies",
    "allowTargetFutureDeviceFamilies",
)

PRICING_FIELDS = (
    "trialPeriod",
    "marketSpecificPricings",
    "priceId",
)

BASE_LISTING_TEXT_FIELDS = (
    "copyrightAndTrademarkInfo",
    "keywords",
    "licenseTerms",
    "privacyPolicy",
    "supportContact",
    "websiteUrl",
    "description",
    "features",
    "releaseNotes",
    "recommendedHardware",
    "minimumHardware",
    "title",
    "shortDescription",
    "shortTitle",
    "sortTitle",
    "voiceTitle",
    "devStudio",
)

PACKAGE_DELIVERY_FIELDS = (
    "isMandatoryUpdate",
    "mandatoryUpdateEffectiveDate",
)

PACKAGE_ROLLOUT_FIELDS = (
    "isPackageRollout",
    "packageRolloutPercentage",
)

SERVER_OWNED_TOP_LEVEL_FIELDS = {
    "id",
    "status",
    "statusDetails",
    "fileUploadUrl",
    "friendlyName",
    "flightId",
}

COMMIT_WAIT_STATES = {
    "",
    "None",
    "PendingCommit",
    "CommitStarted",
}

COMMIT_SUCCESS_STATES = {
    "PreProcessing",
    "Certification",
    "PendingCertification",
    "CertificationPassed",
    "PendingPublication",
    "Publishing",
    "Release",
    "ReleaseReady",
    "Published",
}

COMMIT_FAILURE_STATES = {
    "CommitFailed",
    "PreProcessingFailed",
    "CertificationFailed",
    "PublishFailed",
    "ReleaseFailed",
    "Canceled",
    "Cancelled",
}

_NO_BODY = object()


class MigrationError(RuntimeError):
    """An expected, actionable Store migration failure."""


class AmbiguousStoreOperation(MigrationError):
    """A non-idempotent request may have succeeded despite a transport error."""


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def sha256_json(value: Any) -> str:
    return hashlib.sha256(canonical_json(value)).hexdigest()


def secrets_compare_digest(first: str, second: str) -> bool:
    return secrets.compare_digest(first.encode("ascii"), second.encode("ascii"))


def safe_error_text(value: str, limit: int = 1200) -> str:
    """Remove tokens and SAS query strings before an error reaches a log."""
    value = re.sub(r"(?i)(authorization:\s*bearer\s+)[^\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)(client_secret[\"'=:\s]+)[^&\s\"']+", r"\1[redacted]", value)
    value = re.sub(r"(?i)([?&](?:sig|se|sp|sv|sr)=)[^&\s\"']+", r"\1[redacted]", value)
    value = re.sub(r"https://[^?\s\"']+\?[^ \n\"']+", "[redacted SAS URL]", value)
    return value[:limit]


def deep_copy_present(source: dict[str, Any], fields: tuple[str, ...]) -> dict[str, Any]:
    return {key: copy.deepcopy(source[key]) for key in fields if key in source}


def package_summary(submission: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        {
            key: copy.deepcopy(package[key])
            for key in ("fileName", "fileStatus", "version", "architecture")
            if key in package
        }
        for package in submission.get("applicationPackages", [])
    ]


def _image_record(
    path: str, ordinal: int, image: dict[str, Any]
) -> dict[str, Any]:
    return {
        "path": path,
        "ordinal": ordinal,
        "id": image.get("id"),
        "fileName": image.get("fileName"),
        "fileStatus": image.get("fileStatus"),
        "imageType": image.get("imageType"),
    }


def media_manifest(submission: dict[str, Any]) -> dict[str, Any]:
    """Return asset identity only; text such as image descriptions is excluded."""
    listing_images: list[dict[str, Any]] = []
    for language, listing in sorted(submission.get("listings", {}).items()):
        base = listing.get("baseListing", {})
        for ordinal, image in enumerate(base.get("images", []) or []):
            listing_images.append(
                _image_record(
                    f"listings/{language}/baseListing", ordinal, image
                )
            )
        for platform, override in sorted(
            (listing.get("platformOverrides", {}) or {}).items()
        ):
            for ordinal, image in enumerate(override.get("images", []) or []):
                listing_images.append(
                    _image_record(
                        f"listings/{language}/platformOverrides/{platform}",
                        ordinal,
                        image,
                    )
                )

    # Trailer binaries and thumbnail IDs are not transferable between drafts.
    # Compare the full trailer objects, which also prevents silently dropping a
    # portal-only localized trailer title.
    return {
        "listingLanguages": sorted(submission.get("listings", {}).keys()),
        "listingImages": sorted(
            listing_images,
            key=lambda item: (
                item["path"],
                item["ordinal"],
                str(item["imageType"]),
                str(item["id"]),
                str(item["fileName"]),
                str(item["fileStatus"]),
            ),
        ),
        "trailers": copy.deepcopy(submission.get("trailers", []) or []),
    }


def recoverable_media_projection(manifest: dict[str, Any]) -> dict[str, Any]:
    """Compare recoverable screenshots without draft-scoped IDs/status."""
    images: list[dict[str, Any]] = []
    for image in manifest.get("listingImages", []):
        if image.get("imageType") == "Screenshot":
            images.append(
                {
                    key: copy.deepcopy(image.get(key))
                    for key in ("path", "ordinal", "fileName", "imageType")
                }
            )
        else:
            images.append(copy.deepcopy(image))
    return {
        "listingLanguages": copy.deepcopy(
            manifest.get("listingLanguages", [])
        ),
        "listingImages": images,
        "trailers": copy.deepcopy(manifest.get("trailers", [])),
    }


def _validate_flat_file_name(
    name: str, *, screenshot: bool = False
) -> str:
    pattern = SAFE_SCREENSHOT_NAME if screenshot else SAFE_ARCHIVE_NAME
    if (
        not isinstance(name, str)
        or not pattern.fullmatch(name)
        or "/" in name
        or "\\" in name
        or any(ord(character) < 32 or ord(character) == 127 for character in name)
        or name.split(".", 1)[0].upper() in WINDOWS_RESERVED_NAMES
    ):
        label = "screenshot" if screenshot else "archive"
        raise MigrationError(f"Unsafe {label} file name: {name!r}")
    return name


def _read_regular_file(path: Path, *, label: str) -> bytes:
    if path.is_symlink():
        raise MigrationError(f"{label} must not be a symbolic link: {path.name}")
    flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise MigrationError(f"Could not safely open {label}: {path.name}") from error
    try:
        file_stat = os.fstat(descriptor)
        if not stat.S_ISREG(file_stat.st_mode):
            raise MigrationError(f"{label} is not a regular file: {path.name}")
        with os.fdopen(descriptor, "rb", closefd=False) as handle:
            return handle.read()
    finally:
        os.close(descriptor)


def _screenshot_files(directory: Path) -> dict[str, bytes]:
    if directory.is_symlink():
        raise MigrationError(
            f"Screenshot directory must not be a symbolic link: {directory}"
        )
    try:
        root = directory.resolve(strict=True)
    except OSError as error:
        raise MigrationError(
            f"Screenshot directory does not exist: {directory}"
        ) from error
    if not root.is_dir():
        raise MigrationError(f"Screenshot directory does not exist: {directory}")

    files: dict[str, bytes] = {}
    for path in sorted(root.iterdir(), key=lambda item: item.name):
        if path.suffix != ".png":
            continue
        name = _validate_flat_file_name(path.name, screenshot=True)
        if path.is_symlink():
            raise MigrationError(
                f"Screenshot must not be a symbolic link: {name}"
            )
        try:
            resolved = path.resolve(strict=True)
        except OSError as error:
            raise MigrationError(f"Could not resolve screenshot: {name}") from error
        if resolved.parent != root:
            raise MigrationError(
                f"Screenshot escapes its configured directory: {name}"
            )
        content = _read_regular_file(path, label="Screenshot")
        if not content:
            raise MigrationError(f"Screenshot is empty: {name}")
        if len(content) > MAX_SCREENSHOT_BYTES:
            raise MigrationError(f"Screenshot is too large: {name}")
        if not content.startswith(PNG_SIGNATURE):
            raise MigrationError(f"Screenshot is not a valid PNG file: {name}")
        files[name] = content

    if not files:
        raise MigrationError(f"No PNG screenshots found in {directory}.")
    if len(files) > MAX_SCREENSHOT_COUNT:
        raise MigrationError("Too many Store screenshots to preserve safely.")
    if sum(len(content) for content in files.values()) > MAX_SCREENSHOT_TOTAL_BYTES:
        raise MigrationError("Store screenshots exceed the recovery size limit.")
    return files


def _validate_screenshot_provenance(
    provenance: dict[str, str] | None,
) -> dict[str, str]:
    if provenance is None:
        raise MigrationError(
            "Recovering changed screenshots requires pinned Store and GitHub "
            "provenance. Nothing was deleted."
        )
    required = {
        "repository",
        "workflowRunId",
        "workflowRunHeadSha",
        "submissionId",
        "releaseTag",
        "snapshotCommitSha",
    }
    if set(provenance) != required:
        raise MigrationError(
            "Screenshot provenance is incomplete. Nothing was deleted."
        )
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", provenance["repository"]):
        raise MigrationError("Screenshot provenance has an invalid repository.")
    if not re.fullmatch(r"\d+", provenance["workflowRunId"]):
        raise MigrationError("Screenshot provenance has an invalid workflow run.")
    if not re.fullmatch(r"\d+", provenance["submissionId"]):
        raise MigrationError("Screenshot provenance has an invalid Store submission.")
    for field in ("workflowRunHeadSha", "snapshotCommitSha"):
        if not re.fullmatch(r"[0-9a-f]{40}", provenance[field]):
            raise MigrationError(
                f"Screenshot provenance has an invalid {field}."
            )
    if not re.fullmatch(r"v\d+\.\d+\.\d+", provenance["releaseTag"]):
        raise MigrationError("Screenshot provenance has an invalid release tag.")
    return copy.deepcopy(provenance)


def _media_preservation_plan(
    *,
    source_media: dict[str, Any],
    published_media: dict[str, Any],
    screenshots_dir: Path | None,
    screenshot_provenance_media: dict[str, Any] | None,
    screenshot_provenance: dict[str, str] | None,
    target_version: str,
) -> tuple[str, dict[str, dict[str, Any]], dict[str, str] | None]:
    if source_media == published_media:
        return "published-clone", {}, None

    if screenshots_dir is None:
        raise MigrationError(
            "The Partner Center draft contains listing media or language changes "
            "that cannot be recreated from JSON alone. Nothing was deleted."
        )
    if (
        source_media.get("listingLanguages")
        != published_media.get("listingLanguages")
    ):
        raise MigrationError(
            "The Partner Center draft adds or removes listing languages. "
            "Nothing was deleted; migrate this draft manually."
        )
    if source_media.get("trailers") != published_media.get("trailers"):
        raise MigrationError(
            "The Partner Center draft changes trailer binaries or metadata. "
            "Nothing was deleted; migrate this draft manually."
        )

    source_non_screenshots = [
        image
        for image in source_media.get("listingImages", [])
        if image.get("imageType") != "Screenshot"
    ]
    published_non_screenshots = [
        image
        for image in published_media.get("listingImages", [])
        if image.get("imageType") != "Screenshot"
    ]
    if source_non_screenshots != published_non_screenshots:
        raise MigrationError(
            "The Partner Center draft changes non-screenshot listing media. "
            "Nothing was deleted; migrate this draft manually."
        )
    if screenshot_provenance_media != source_media:
        raise MigrationError(
            "The draft media does not exactly match the Store submission that "
            "the successful screenshot workflow created. Filename equality is "
            "not sufficient, so nothing was deleted."
        )
    verified_provenance = _validate_screenshot_provenance(
        screenshot_provenance
    )
    if verified_provenance["releaseTag"] != f"v{target_version}":
        raise MigrationError(
            "Screenshot provenance is not bound to the target release. "
            "Nothing was deleted."
        )

    source_screenshots = [
        image
        for image in source_media.get("listingImages", [])
        if image.get("imageType") == "Screenshot"
    ]
    if not source_screenshots:
        raise MigrationError(
            "The Partner Center draft removes every screenshot. Nothing was deleted."
        )
    names = [str(image.get("fileName", "")) for image in source_screenshots]
    if (
        any(
            not name
            or not SAFE_SCREENSHOT_NAME.fullmatch(name)
            or "/" in name
            or "\\" in name
            or name.split(".", 1)[0].upper() in WINDOWS_RESERVED_NAMES
            for name in names
        )
        or len(names) != len(set(names))
        or len({name.casefold() for name in names}) != len(names)
        or any(
            image.get("fileStatus") not in {"Uploaded", "PendingUpload"}
            for image in source_screenshots
        )
    ):
        raise MigrationError(
            "The draft screenshot references are not safe to reconstruct. "
            "Nothing was deleted."
        )

    repo_files = _screenshot_files(screenshots_dir)
    if set(names) != set(repo_files):
        raise MigrationError(
            "The draft screenshots do not exactly match the checked-in Store "
            f"assets. Draft: {', '.join(sorted(names))}. Repository: "
            f"{', '.join(sorted(repo_files))}. Nothing was deleted."
        )
    embedded_files = {
        name: {
            "sha256": hashlib.sha256(content).hexdigest(),
            "size": len(content),
            "contentBase64": base64.b64encode(content).decode("ascii"),
        }
        for name, content in repo_files.items()
    }
    return "embedded-provenance-screenshots", embedded_files, verified_provenance


def _snapshot_listing(listing: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    base = listing.get("baseListing", {})
    base_snapshot = deep_copy_present(base, BASE_LISTING_TEXT_FIELDS)
    if "images" in base:
        base_snapshot["images"] = [
            deep_copy_present(
                image,
                ("id", "fileName", "fileStatus", "imageType", "description"),
            )
            for image in base.get("images", []) or []
        ]
    result["baseListing"] = base_snapshot

    overrides: dict[str, Any] = {}
    for platform, override in (listing.get("platformOverrides", {}) or {}).items():
        override_snapshot = deep_copy_present(override, BASE_LISTING_TEXT_FIELDS)
        if "images" in override:
            override_snapshot["images"] = [
                deep_copy_present(
                    image,
                    ("id", "fileName", "fileStatus", "imageType", "description"),
                )
                for image in override.get("images", []) or []
            ]
        overrides[platform] = override_snapshot
    result["platformOverrides"] = overrides
    return result


def snapshot_metadata(submission: dict[str, Any]) -> dict[str, Any]:
    metadata = deep_copy_present(submission, TOP_LEVEL_METADATA_FIELDS)
    metadata["pricing"] = deep_copy_present(
        submission.get("pricing", {}), PRICING_FIELDS
    )
    metadata["listings"] = {
        language: _snapshot_listing(listing)
        for language, listing in submission.get("listings", {}).items()
    }

    delivery = submission.get("packageDeliveryOptions", {}) or {}
    delivery_snapshot = deep_copy_present(delivery, PACKAGE_DELIVERY_FIELDS)
    delivery_snapshot["packageRollout"] = deep_copy_present(
        delivery.get("packageRollout", {}) or {}, PACKAGE_ROLLOUT_FIELDS
    )
    metadata["packageDeliveryOptions"] = delivery_snapshot
    return metadata


def _unknown_listing_projection(listing: dict[str, Any]) -> dict[str, Any]:
    known_listing = {"baseListing", "platformOverrides"}
    known_base = set(BASE_LISTING_TEXT_FIELDS) | {"images"}
    known_image = {"id", "fileName", "fileStatus", "imageType", "description"}
    result = {
        "listing": {
            key: copy.deepcopy(value)
            for key, value in listing.items()
            if key not in known_listing
        }
    }

    def section_projection(section: dict[str, Any]) -> dict[str, Any]:
        unknown_images = []
        for ordinal, image in enumerate(section.get("images", []) or []):
            unknown = {
                key: copy.deepcopy(value)
                for key, value in image.items()
                if key not in known_image
            }
            if unknown:
                unknown_images.append(
                    {
                        "ordinal": ordinal,
                        "identity": deep_copy_present(
                            image, ("id", "fileName", "imageType")
                        ),
                        "unknown": unknown,
                    }
                )
        return {
            "section": {
                key: copy.deepcopy(value)
                for key, value in section.items()
                if key not in known_base
            },
            "images": unknown_images,
        }

    result["baseListing"] = section_projection(listing.get("baseListing", {}))
    result["platformOverrides"] = {
        platform: section_projection(override)
        for platform, override in (
            listing.get("platformOverrides", {}) or {}
        ).items()
    }
    return result


def unhandled_projection(submission: dict[str, Any]) -> dict[str, Any]:
    """Fields not explicitly restored must remain unchanged from published."""
    handled_top_level = (
        set(TOP_LEVEL_METADATA_FIELDS)
        | SERVER_OWNED_TOP_LEVEL_FIELDS
        | {
            "pricing",
            "listings",
            "applicationPackages",
            "packageDeliveryOptions",
            "trailers",
        }
    )
    delivery = submission.get("packageDeliveryOptions", {}) or {}
    rollout = delivery.get("packageRollout", {}) or {}
    return {
        "topLevel": {
            key: copy.deepcopy(value)
            for key, value in submission.items()
            if key not in handled_top_level
        },
        "pricing": {
            key: copy.deepcopy(value)
            for key, value in (submission.get("pricing", {}) or {}).items()
            if key not in set(PRICING_FIELDS) | {"sales", "isAdvancedPricingModel"}
        },
        "listings": {
            language: _unknown_listing_projection(listing)
            for language, listing in submission.get("listings", {}).items()
        },
        "packageDeliveryOptions": {
            "topLevel": {
                key: copy.deepcopy(value)
                for key, value in delivery.items()
                if key
                not in set(PACKAGE_DELIVERY_FIELDS) | {"packageRollout"}
            },
            "packageRollout": {
                key: copy.deepcopy(value)
                for key, value in rollout.items()
                if key
                not in set(PACKAGE_ROLLOUT_FIELDS)
                | {"packageRolloutStatus", "fallbackSubmissionId"}
            },
        },
    }


def source_state_projection(submission: dict[str, Any]) -> dict[str, Any]:
    """Stable draft state used to detect portal edits after snapshotting."""
    return {
        "metadata": snapshot_metadata(submission),
        "media": media_manifest(submission),
        "unhandled": unhandled_projection(submission),
        "packages": package_summary(submission),
    }


def make_snapshot(
    *,
    app_id: str,
    source: dict[str, Any],
    published: dict[str, Any],
    target_version: str,
    screenshots_dir: Path | None = None,
    screenshot_provenance_media: dict[str, Any] | None = None,
    screenshot_provenance: dict[str, str] | None = None,
) -> dict[str, Any]:
    if not re.fullmatch(r"\d+\.\d+\.\d+", target_version):
        raise MigrationError("Snapshot target version must use the form X.Y.Z.")
    if (
        source.get("packageDeliveryOptions", {}) or {}
    ).get("isMandatoryUpdate") is True:
        raise MigrationError(
            "Microsoft's submission API does not support mandatory-update apps. "
            "Nothing was deleted; submit this draft manually in Partner Center."
        )

    source_media = media_manifest(source)
    published_media = media_manifest(published)
    (
        preservation_mode,
        embedded_screenshots,
        verified_screenshot_provenance,
    ) = _media_preservation_plan(
        source_media=source_media,
        published_media=published_media,
        screenshots_dir=screenshots_dir,
        screenshot_provenance_media=screenshot_provenance_media,
        screenshot_provenance=screenshot_provenance,
        target_version=target_version,
    )

    if unhandled_projection(source) != unhandled_projection(published):
        raise MigrationError(
            "The Partner Center draft changes fields that this migration does "
            "not explicitly restore. Nothing was deleted; submit the draft "
            "manually to avoid silent metadata loss."
        )

    source_id = str(source.get("id", ""))
    published_id = str(published.get("id", ""))
    if not source_id or not published_id:
        raise MigrationError("The source or last-published submission has no ID.")

    snapshot: dict[str, Any] = {
        "schemaVersion": SNAPSHOT_SCHEMA_VERSION,
        "capturedAtUtc": dt.datetime.now(dt.UTC).isoformat(),
        "appId": app_id,
        "targetVersion": target_version,
        "sourceSubmissionId": source_id,
        "sourceStatus": source.get("status"),
        "lastPublishedSubmissionId": published_id,
        "sourceRawSha256": sha256_json(source),
        "sourceStateSha256": sha256_json(source_state_projection(source)),
        "publishedCloneStateSha256": sha256_json(
            source_state_projection(published)
        ),
        "mediaManifestSha256": sha256_json(source_media),
        "publishedMediaManifestSha256": sha256_json(published_media),
        "sourceMediaManifest": source_media,
        "mediaPreservationMode": preservation_mode,
        "recoverableScreenshotFiles": embedded_screenshots,
        "screenshotProvenance": verified_screenshot_provenance,
        "mediaPreservationVerified": True,
        "metadata": snapshot_metadata(source),
        "sourcePackages": package_summary(source),
    }
    snapshot["snapshotSha256"] = sha256_json(snapshot)
    return snapshot


def _image_identity(image: dict[str, Any]) -> tuple[Any, ...]:
    return (
        image.get("id"),
        image.get("fileName"),
        image.get("imageType"),
    )


def merge_images(
    new_images: list[dict[str, Any]],
    source_images: list[dict[str, Any]],
    *,
    recover_screenshots: bool = False,
) -> list[dict[str, Any]]:
    """Keep clone-owned media IDs and restore only editable descriptions."""
    source_by_identity = {
        _image_identity(image): image for image in source_images or []
    }
    new_by_identity = {
        _image_identity(image): image for image in new_images or []
    }
    if recover_screenshots:
        recovered: list[dict[str, Any]] = []
        for source_image in source_images or []:
            if source_image.get("imageType") == "Screenshot":
                image = deep_copy_present(
                    source_image, ("fileName", "imageType", "description")
                )
                image["fileStatus"] = "PendingUpload"
                recovered.append(image)
                continue
            new_image = new_by_identity.get(_image_identity(source_image))
            if new_image is None:
                raise MigrationError(
                    "A non-screenshot listing asset is missing from the API clone."
                )
            merged_image = copy.deepcopy(new_image)
            if "description" in source_image:
                merged_image["description"] = copy.deepcopy(
                    source_image["description"]
                )
            recovered.append(merged_image)
        return recovered

    merged = copy.deepcopy(new_images or [])
    for image in merged:
        source_image = source_by_identity.get(_image_identity(image))
        if source_image is not None and "description" in source_image:
            image["description"] = copy.deepcopy(source_image["description"])
    return merged


def _merge_listing_section(
    new_section: dict[str, Any],
    source_section: dict[str, Any],
    *,
    recover_screenshots: bool = False,
) -> dict[str, Any]:
    merged = copy.deepcopy(new_section)
    for field in BASE_LISTING_TEXT_FIELDS:
        if field in source_section:
            merged[field] = copy.deepcopy(source_section[field])
    if "images" in new_section:
        merged["images"] = merge_images(
            new_section.get("images", []) or [],
            source_section.get("images", []) or [],
            recover_screenshots=recover_screenshots,
        )
    elif recover_screenshots and source_section.get("images"):
        merged["images"] = merge_images(
            [],
            source_section.get("images", []) or [],
            recover_screenshots=True,
        )
    return merged


def merge_listings(
    new_listings: dict[str, Any],
    source_listings: dict[str, Any],
    release_notes: str,
    *,
    recover_screenshots: bool = False,
) -> dict[str, Any]:
    if set(new_listings) != set(source_listings):
        raise MigrationError(
            "The API-created clone does not contain the same listing languages "
            "as the preserved draft. Refusing to lose a locale."
        )

    merged: dict[str, Any] = {}
    for language, new_listing in new_listings.items():
        source_listing = source_listings[language]
        result_listing = copy.deepcopy(new_listing)
        result_listing["baseListing"] = _merge_listing_section(
            new_listing.get("baseListing", {}),
            source_listing.get("baseListing", {}),
            recover_screenshots=recover_screenshots,
        )
        result_listing["baseListing"]["releaseNotes"] = release_notes

        # The source draft is authoritative for which platform overrides exist.
        # Media inside an override is still taken from the new clone.
        overrides: dict[str, Any] = {}
        for platform, source_override in (
            source_listing.get("platformOverrides", {}) or {}
        ).items():
            new_override = (
                new_listing.get("platformOverrides", {}) or {}
            ).get(platform, {})
            overrides[platform] = _merge_listing_section(
                new_override,
                source_override,
                recover_screenshots=recover_screenshots,
            )
        result_listing["platformOverrides"] = overrides
        merged[language] = result_listing
    return merged


def _merge_package_delivery(
    new_delivery: dict[str, Any], source_delivery: dict[str, Any]
) -> dict[str, Any]:
    merged = copy.deepcopy(new_delivery)
    for field in PACKAGE_DELIVERY_FIELDS:
        if field in source_delivery:
            merged[field] = copy.deepcopy(source_delivery[field])
    merged.setdefault("packageRollout", {})
    for field in PACKAGE_ROLLOUT_FIELDS:
        if field in source_delivery.get("packageRollout", {}):
            merged["packageRollout"][field] = copy.deepcopy(
                source_delivery["packageRollout"][field]
            )
    return merged


def _request_package(package: dict[str, Any], status: str) -> dict[str, Any]:
    result = copy.deepcopy(package)
    result["fileStatus"] = status
    result.setdefault("minimumDirectXVersion", "None")
    result.setdefault("minimumSystemRam", "None")
    return result


def replace_packages(
    new_packages: list[dict[str, Any]], package_file_name: str
) -> list[dict[str, Any]]:
    extension = Path(package_file_name).suffix.lower()
    if extension != ".msixbundle":
        raise MigrationError("The Store release asset must be an .msixbundle.")

    result: list[dict[str, Any]] = []
    replaced = 0
    for package in new_packages:
        if not package.get("fileName"):
            continue
        if package["fileName"] == package_file_name:
            # Recovery after a prior PUT must not create a duplicate target.
            continue
        package_extension = Path(package["fileName"]).suffix.lower()
        if package_extension == extension:
            result.append(_request_package(package, "PendingDelete"))
            replaced += 1
        else:
            result.append(
                _request_package(package, package.get("fileStatus", "Uploaded"))
            )
    if replaced == 0:
        raise MigrationError(
            "The API-created clone has no existing .msixbundle to replace."
        )
    result.append(
        {
            "fileName": package_file_name,
            "fileStatus": "PendingUpload",
            "minimumDirectXVersion": "None",
            "minimumSystemRam": "None",
        }
    )
    return result


def build_update_payload(
    *,
    new_submission: dict[str, Any],
    snapshot: dict[str, Any],
    package_file_name: str,
    release_notes: str,
) -> dict[str, Any]:
    metadata = snapshot["metadata"]
    payload: dict[str, Any] = {}

    for field in TOP_LEVEL_METADATA_FIELDS:
        if field in metadata:
            payload[field] = copy.deepcopy(metadata[field])
        elif field in new_submission:
            payload[field] = copy.deepcopy(new_submission[field])

    pricing = deep_copy_present(new_submission.get("pricing", {}), PRICING_FIELDS)
    pricing.update(copy.deepcopy(metadata.get("pricing", {})))
    # The API's example includes an empty sales array, but sale edits are
    # deprecated and ignored. Never restore portal sale state.
    pricing["sales"] = []
    payload["pricing"] = pricing

    payload["listings"] = merge_listings(
        new_submission.get("listings", {}),
        metadata.get("listings", {}),
        release_notes,
        recover_screenshots=(
            snapshot.get("mediaPreservationMode")
            == "embedded-provenance-screenshots"
        ),
    )
    payload["packageDeliveryOptions"] = _merge_package_delivery(
        new_submission.get("packageDeliveryOptions", {}) or {},
        metadata.get("packageDeliveryOptions", {}) or {},
    )
    payload["trailers"] = copy.deepcopy(new_submission.get("trailers", []) or [])
    payload["applicationPackages"] = replace_packages(
        new_submission.get("applicationPackages", []) or [],
        package_file_name,
    )

    forbidden = {
        "id",
        "status",
        "statusDetails",
        "fileUploadUrl",
        "friendlyName",
    }
    leaked = forbidden.intersection(payload)
    if leaked:
        raise AssertionError(f"Server-owned fields leaked into PUT body: {leaked}")
    return payload


def metadata_projection(submission: dict[str, Any]) -> dict[str, Any]:
    projected = deep_copy_present(submission, TOP_LEVEL_METADATA_FIELDS)
    projected["pricing"] = deep_copy_present(
        submission.get("pricing", {}), PRICING_FIELDS
    )
    projected["listings"] = {
        language: _snapshot_listing(listing)
        for language, listing in submission.get("listings", {}).items()
    }
    for listing in projected["listings"].values():
        sections = [listing.get("baseListing", {})]
        sections.extend(
            (listing.get("platformOverrides", {}) or {}).values()
        )
        for section in sections:
            for image in section.get("images", []) or []:
                if (
                    image.get("imageType") == "Screenshot"
                    and image.get("fileStatus") == "PendingUpload"
                ):
                    # Partner Center may assign a draft-scoped ID as soon as a
                    # reconstructed screenshot record is PUT. Existing and
                    # non-screenshot asset IDs remain integrity checked.
                    image.pop("id", None)
    # File statuses can be normalized immediately after PUT. Metadata
    # verification deliberately excludes package server state.
    projected["packageDeliveryOptions"] = {
        **deep_copy_present(
            submission.get("packageDeliveryOptions", {}),
            PACKAGE_DELIVERY_FIELDS,
        ),
        "packageRollout": deep_copy_present(
            submission.get("packageDeliveryOptions", {}).get(
                "packageRollout", {}
            ),
            PACKAGE_ROLLOUT_FIELDS,
        ),
    }
    return projected


def validate_restored_submission(
    *,
    desired: dict[str, Any],
    actual: dict[str, Any],
    package_file_name: str,
) -> None:
    if metadata_projection(desired) != metadata_projection(actual):
        raise MigrationError(
            "Partner Center did not preserve the restored metadata exactly. "
            "The new API draft was left uncommitted for inspection."
        )

    packages = {
        package.get("fileName"): package.get("fileStatus")
        for package in actual.get("applicationPackages", [])
    }
    if packages.get(package_file_name) not in {"PendingUpload", "Uploaded"}:
        raise MigrationError(
            f"The restored draft does not contain {package_file_name} as an upload."
        )
    if not any(
        name != package_file_name and status == "PendingDelete"
        for name, status in packages.items()
    ):
        raise MigrationError(
            "The restored draft did not mark the previous bundle for deletion."
        )


class StoreApi:
    def __init__(
        self,
        *,
        tenant_id: str,
        client_id: str,
        client_secret: str,
        app_id: str,
    ) -> None:
        self.tenant_id = tenant_id
        self.client_id = client_id
        self.client_secret = client_secret
        self.app_id = app_id
        self.base = f"{API_ROOT}/{urllib.parse.quote(app_id, safe='')}"
        self._token = ""
        self._token_expires_at = 0.0

    def _authenticate(self) -> None:
        endpoint = (
            "https://login.microsoftonline.com/"
            f"{urllib.parse.quote(self.tenant_id, safe='')}/oauth2/token"
        )
        form = urllib.parse.urlencode(
            {
                "grant_type": "client_credentials",
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "resource": TOKEN_RESOURCE,
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            endpoint,
            data=form,
            method="POST",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = json.load(response)
        except (urllib.error.HTTPError, urllib.error.URLError) as error:
            body = ""
            if isinstance(error, urllib.error.HTTPError):
                body = error.read().decode("utf-8", "replace")
            raise MigrationError(
                "Partner Center authentication failed. "
                + safe_error_text(body or str(error))
            ) from error
        self._token = payload.get("access_token", "")
        if not self._token:
            raise MigrationError("Partner Center authentication returned no token.")
        expires_in = int(payload.get("expires_in", 3600))
        self._token_expires_at = time.monotonic() + max(expires_in - 300, 60)

    def _ensure_token(self, force: bool = False) -> None:
        if force or not self._token or time.monotonic() >= self._token_expires_at:
            self._authenticate()

    def request(
        self,
        method: str,
        suffix: str = "",
        body: Any = _NO_BODY,
        *,
        retry_transient: bool = True,
    ) -> dict[str, Any]:
        url = f"{self.base}{suffix}"
        delays = (5, 15, 30, 60, 90)
        forced_refresh = False
        for attempt, delay in enumerate(delays, start=1):
            self._ensure_token()
            data: bytes | None
            headers = {"Authorization": f"Bearer {self._token}"}
            if body is _NO_BODY:
                data = b"" if method == "POST" else None
                if method == "POST":
                    headers["Content-Type"] = "application/json"
            else:
                data = canonical_json(body)
                headers["Content-Type"] = "application/json"
            request = urllib.request.Request(
                url, data=data, method=method, headers=headers
            )
            try:
                with urllib.request.urlopen(request, timeout=180) as response:
                    raw = response.read()
                return json.loads(raw) if raw.strip() else {}
            except urllib.error.HTTPError as error:
                raw_error = error.read().decode("utf-8", "replace")
                if error.code == 401 and not forced_refresh:
                    forced_refresh = True
                    self._ensure_token(force=True)
                    continue
                retryable = error.code == 429 or error.code >= 500
                if retryable and not retry_transient:
                    raise AmbiguousStoreOperation(
                        f"Store API {method} returned HTTP {error.code}; "
                        "the server-side result is ambiguous."
                    ) from error
                if not retryable or attempt == len(delays):
                    raise MigrationError(
                        f"Store API {method} failed with HTTP {error.code}: "
                        f"{safe_error_text(raw_error)}"
                    ) from error
                retry_after = error.headers.get("Retry-After")
                sleep_for = int(retry_after) if retry_after and retry_after.isdigit() else delay
                print(
                    f"Store API transient HTTP {error.code}; retrying in "
                    f"{sleep_for}s ({attempt}/{len(delays)}).",
                    flush=True,
                )
                time.sleep(sleep_for)
            except urllib.error.URLError as error:
                if not retry_transient:
                    raise AmbiguousStoreOperation(
                        f"Store API {method} had an ambiguous network failure: "
                        f"{safe_error_text(str(error))}"
                    ) from error
                if attempt == len(delays):
                    raise MigrationError(
                        f"Store API network failure: {safe_error_text(str(error))}"
                    ) from error
                print(
                    f"Store API network failure; retrying in {delay}s "
                    f"({attempt}/{len(delays)}).",
                    flush=True,
                )
                time.sleep(delay)
        raise AssertionError("unreachable")

    def get_app(self) -> dict[str, Any]:
        return self.request("GET")

    def get_submission(self, submission_id: str) -> dict[str, Any]:
        return self.request(
            "GET", f"/submissions/{urllib.parse.quote(submission_id, safe='')}"
        )

    def create_submission(self) -> dict[str, Any]:
        try:
            return self.request(
                "POST", "/submissions", retry_transient=False
            )
        except AmbiguousStoreOperation as error:
            print(
                "Create response was ambiguous; reconciling through the app "
                "submission pointer instead of retrying POST.",
                flush=True,
            )
            for _ in range(12):
                app = self.get_app()
                pending_id = _submission_pointer_id(
                    app.get("pendingApplicationSubmission")
                )
                if pending_id:
                    return self.get_submission(pending_id)
                time.sleep(10)
            raise MigrationError(
                "Partner Center did not expose a created draft after an "
                "ambiguous create response. The encrypted snapshot is safe; "
                "rerun the migration to reconcile."
            ) from error

    def update_submission(
        self, submission_id: str, payload: dict[str, Any]
    ) -> dict[str, Any]:
        return self.request(
            "PUT",
            f"/submissions/{urllib.parse.quote(submission_id, safe='')}",
            payload,
        )

    def commit_submission(self, submission_id: str) -> dict[str, Any]:
        return self.request(
            "POST",
            f"/submissions/{urllib.parse.quote(submission_id, safe='')}/commit",
            retry_transient=False,
        )

    def delete_submission(self, submission_id: str) -> None:
        suffix = f"/submissions/{urllib.parse.quote(submission_id, safe='')}"
        try:
            self.request("DELETE", suffix, retry_transient=False)
        except AmbiguousStoreOperation as error:
            print(
                "Delete response was ambiguous; reconciling through the app "
                "submission pointer instead of retrying DELETE.",
                flush=True,
            )
            ambiguous_error: AmbiguousStoreOperation | None = error
        else:
            ambiguous_error = None
        for _ in range(12):
            app = self.get_app()
            pending_id = _submission_pointer_id(
                app.get("pendingApplicationSubmission")
            )
            if pending_id != submission_id:
                return
            time.sleep(10)
        message = f"Could not confirm deletion of API draft {submission_id}."
        if ambiguous_error is not None:
            raise MigrationError(message) from ambiguous_error
        raise MigrationError(message)

    def get_submission_status(self, submission_id: str) -> dict[str, Any]:
        return self.request(
            "GET",
            f"/submissions/{urllib.parse.quote(submission_id, safe='')}/status",
        )


def require_credentials() -> tuple[str, str, str]:
    values = tuple(
        os.environ.get(name, "")
        for name in (
            "AZURE_TENANT_ID",
            "AZURE_CLIENT_ID",
            "AZURE_CLIENT_SECRET",
        )
    )
    if not all(values):
        raise MigrationError(
            "AZURE_TENANT_ID, AZURE_CLIENT_ID, and AZURE_CLIENT_SECRET are required."
        )
    return values  # type: ignore[return-value]


def validate_submission_id(value: str, label: str = "submission ID") -> str:
    if not re.fullmatch(r"\d+", value):
        raise MigrationError(f"{label.capitalize()} must contain digits only.")
    return value


def write_private_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def load_snapshot(path: Path, app_id: str, source_id: str) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        snapshot = json.load(handle)
    if snapshot.get("schemaVersion") != SNAPSHOT_SCHEMA_VERSION:
        raise MigrationError("Unsupported Store draft snapshot schema.")
    if snapshot.get("appId") != app_id:
        raise MigrationError("The snapshot belongs to a different Store app.")
    if str(snapshot.get("sourceSubmissionId")) != source_id:
        raise MigrationError("The snapshot belongs to a different source submission.")
    if not snapshot.get("mediaPreservationVerified"):
        raise MigrationError("The snapshot did not pass media-preservation checks.")
    stored_digest = snapshot.get("snapshotSha256")
    value_without_digest = copy.deepcopy(snapshot)
    value_without_digest.pop("snapshotSha256", None)
    if not stored_digest or not secrets_compare_digest(
        str(stored_digest), sha256_json(value_without_digest)
    ):
        raise MigrationError("The Store draft snapshot failed its integrity check.")
    return snapshot


def validate_snapshot_target(snapshot: dict[str, Any], version: str) -> None:
    if snapshot.get("targetVersion") != version:
        raise MigrationError(
            f"The recovery snapshot targets version "
            f"{snapshot.get('targetVersion') or 'unknown'}, not {version}."
        )
    if snapshot.get("mediaPreservationMode") == "embedded-provenance-screenshots":
        provenance = snapshot.get("screenshotProvenance")
        release_tag = (
            provenance.get("releaseTag")
            if isinstance(provenance, dict)
            else None
        )
        if release_tag != f"v{version}":
            raise MigrationError(
                "The embedded screenshot provenance does not match the "
                f"requested v{version} release."
            )


def _submission_pointer_id(pointer: Any) -> str:
    return str(pointer.get("id", "")) if isinstance(pointer, dict) else ""


def command_snapshot(args: argparse.Namespace) -> None:
    validate_submission_id(args.source_submission_id, "source submission ID")
    tenant_id, client_id, client_secret = require_credentials()
    api = StoreApi(
        tenant_id=tenant_id,
        client_id=client_id,
        client_secret=client_secret,
        app_id=args.app_id,
    )
    app = api.get_app()
    pending_id = _submission_pointer_id(app.get("pendingApplicationSubmission"))
    if pending_id != args.source_submission_id:
        raise MigrationError(
            f"Expected pending submission {args.source_submission_id}, "
            f"but Partner Center reports {pending_id or 'none'}."
        )
    published_id = _submission_pointer_id(
        app.get("lastPublishedApplicationSubmission")
    )
    if not published_id:
        raise MigrationError("Partner Center reports no last-published submission.")

    source = api.get_submission(args.source_submission_id)
    published = api.get_submission(published_id)
    provenance_submission = None
    provenance = None
    if args.screenshot_provenance_submission_id:
        validate_submission_id(
            args.screenshot_provenance_submission_id,
            "screenshot provenance submission ID",
        )
        provenance_submission = api.get_submission(
            args.screenshot_provenance_submission_id
        )
        provenance = {
            "repository": args.screenshot_provenance_repository or "",
            "workflowRunId": args.screenshot_provenance_run_id or "",
            "workflowRunHeadSha": args.screenshot_provenance_head_sha or "",
            "submissionId": args.screenshot_provenance_submission_id,
            "releaseTag": args.screenshot_release_tag or "",
            "snapshotCommitSha": args.snapshot_commit_sha or "",
        }
    snapshot = make_snapshot(
        app_id=args.app_id,
        source=source,
        published=published,
        screenshots_dir=(
            Path(args.screenshots_dir) if args.screenshots_dir else None
        ),
        screenshot_provenance_media=(
            media_manifest(provenance_submission)
            if provenance_submission is not None
            else None
        ),
        screenshot_provenance=provenance,
        target_version=args.target_version,
    )
    output = Path(args.output)
    write_private_json(output, snapshot)

    languages = ", ".join(snapshot["metadata"]["listings"].keys()) or "none"
    packages = ", ".join(
        package.get("fileName", "unknown")
        for package in snapshot["sourcePackages"]
    )
    print(
        f"Preserved submission {args.source_submission_id} "
        f"(status {snapshot['sourceStatus']}).",
        flush=True,
    )
    print(f"Listing languages: {languages}", flush=True)
    print(f"Source packages: {packages}", flush=True)
    print(f"Snapshot SHA-256: {snapshot['snapshotSha256']}", flush=True)
    mode = snapshot["mediaPreservationMode"]
    if mode == "embedded-provenance-screenshots":
        names = ", ".join(snapshot["recoverableScreenshotFiles"])
        provenance = snapshot["screenshotProvenance"]
        print(
            "The Store API media identities exactly match submission "
            f"{provenance['submissionId']}, which successful workflow run "
            f"{provenance['workflowRunId']} uploaded from "
            f"{provenance['workflowRunHeadSha']}. Embedded canonical "
            f"replacement screenshots: {names}",
            flush=True,
        )
    else:
        print(
            "Media matches the published submission; no portal-only binary "
            "will be lost.",
            flush=True,
        )


def validate_package(path: Path, expected_version: str) -> tuple[str, str]:
    if not re.fullmatch(r"\d+\.\d+\.\d+", expected_version):
        raise MigrationError("Version must use the form X.Y.Z.")
    expected_name = f"WindowsForum_Diagnostics_{expected_version}.msixbundle"
    if path.name != expected_name:
        raise MigrationError(
            f"Expected release asset {expected_name}, found {path.name}."
        )
    _validate_flat_file_name(path.name)
    if path.is_symlink():
        raise MigrationError("The release asset must not be a symbolic link.")
    try:
        content = _read_regular_file(path, label="Release asset")
    except (FileNotFoundError, OSError) as error:
        raise MigrationError(f"Release asset is missing: {path}") from error
    if not content:
        raise MigrationError(f"Release asset is missing or empty: {path}")
    digest = hashlib.sha256(content).hexdigest()
    return expected_name, digest


def validate_recoverable_screenshots(
    snapshot: dict[str, Any],
) -> dict[str, bytes]:
    expected = snapshot.get("recoverableScreenshotFiles", {}) or {}
    if not expected:
        return {}
    if snapshot.get("mediaPreservationMode") != "embedded-provenance-screenshots":
        raise MigrationError(
            "The preserved screenshots do not use the authenticated embedded "
            "recovery format."
        )
    if not isinstance(expected, dict) or len(expected) > MAX_SCREENSHOT_COUNT:
        raise MigrationError("The embedded screenshot manifest is invalid.")
    recovered: dict[str, bytes] = {}
    total_size = 0
    for name, record in expected.items():
        _validate_flat_file_name(name, screenshot=True)
        if not isinstance(record, dict) or set(record) != {
            "sha256",
            "size",
            "contentBase64",
        }:
            raise MigrationError(
                f"Embedded screenshot metadata is invalid: {name}"
            )
        if not isinstance(record["size"], int) or not (
            1 <= record["size"] <= MAX_SCREENSHOT_BYTES
        ):
            raise MigrationError(f"Embedded screenshot size is invalid: {name}")
        if not re.fullmatch(r"[0-9a-f]{64}", str(record["sha256"])):
            raise MigrationError(
                f"Embedded screenshot digest is invalid: {name}"
            )
        try:
            content = base64.b64decode(
                str(record["contentBase64"]).encode("ascii"), validate=True
            )
        except (UnicodeEncodeError, binascii.Error, ValueError) as error:
            raise MigrationError(
                f"Embedded screenshot encoding is invalid: {name}"
            ) from error
        if len(content) != record["size"]:
            raise MigrationError(f"Embedded screenshot size changed: {name}")
        actual_digest = hashlib.sha256(content).hexdigest()
        if not secrets_compare_digest(actual_digest, str(record["sha256"])):
            raise MigrationError(f"Embedded screenshot digest changed: {name}")
        if not content.startswith(PNG_SIGNATURE):
            raise MigrationError(f"Embedded screenshot is not PNG data: {name}")
        total_size += len(content)
        if total_size > MAX_SCREENSHOT_TOTAL_BYTES:
            raise MigrationError("Embedded screenshots exceed the recovery limit.")
        recovered[name] = content

    source_names = {
        str(image.get("fileName", ""))
        for image in snapshot.get("sourceMediaManifest", {}).get(
            "listingImages", []
        )
        if image.get("imageType") == "Screenshot"
    }
    if set(recovered) != source_names:
        raise MigrationError(
            "The embedded screenshot set does not match the preserved Store "
            "media manifest."
        )
    return recovered


def create_upload_zip(
    package: Path,
    destination: Path,
    extra_files: dict[str, bytes] | None = None,
    *,
    expected_package_sha256: str | None = None,
) -> None:
    package_name = _validate_flat_file_name(package.name)
    if package.is_symlink():
        raise MigrationError("The release package must not be a symbolic link.")
    package_content = _read_regular_file(package, label="Release package")
    if not package_content:
        raise MigrationError("The release package is empty.")
    package_digest = hashlib.sha256(package_content).hexdigest()
    if expected_package_sha256 and not secrets_compare_digest(
        package_digest, expected_package_sha256
    ):
        raise MigrationError(
            "The release package changed after provenance validation."
        )

    archive_names = {package_name.casefold()}
    validated_extras: list[tuple[str, bytes]] = []
    for name, content in sorted((extra_files or {}).items()):
        _validate_flat_file_name(name, screenshot=True)
        normalized = name.casefold()
        if normalized in archive_names:
            raise MigrationError(f"Duplicate upload archive name: {name}")
        archive_names.add(normalized)
        if not content or not content.startswith(PNG_SIGNATURE):
            raise MigrationError(f"Invalid screenshot archive content: {name}")
        validated_extras.append((name, content))

    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_STORED) as archive:
        archive.writestr(package_name, package_content)
        for name, content in validated_extras:
            archive.writestr(name, content)


def upload_blob(sas_url: str, archive: Path) -> None:
    if not sas_url.startswith("https://"):
        raise MigrationError("The API-created submission has no valid upload URL.")
    payload = archive.read_bytes()
    delays = (5, 15, 30, 60, 90)
    for attempt, delay in enumerate(delays, start=1):
        request = urllib.request.Request(
            sas_url,
            data=payload,
            method="PUT",
            headers={
                "x-ms-blob-type": "BlockBlob",
                "Content-Type": "application/octet-stream",
                "Content-Length": str(len(payload)),
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=900) as response:
                response.read()
                status = response.status
            if status in {200, 201}:
                return
            raise MigrationError(f"Store package upload returned HTTP {status}.")
        except urllib.error.HTTPError as error:
            body = error.read().decode("utf-8", "replace")
            retryable = error.code == 429 or error.code >= 500
            if not retryable or attempt == len(delays):
                raise MigrationError(
                    f"Store package upload failed with HTTP {error.code}: "
                    f"{safe_error_text(body)}"
                ) from error
            print(
                f"Package upload transient HTTP {error.code}; retrying in "
                f"{delay}s ({attempt}/{len(delays)}).",
                flush=True,
            )
            time.sleep(delay)
        except urllib.error.URLError as error:
            if attempt == len(delays):
                raise MigrationError(
                    f"Store package upload network failure: {safe_error_text(str(error))}"
                ) from error
            time.sleep(delay)


def _sas_expiry(sas_url: str) -> dt.datetime:
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(sas_url).query)
    raw_expiry = (query.get("se") or [""])[0]
    if not raw_expiry:
        raise MigrationError("The API draft upload URL has no SAS expiry.")
    try:
        expiry = dt.datetime.fromisoformat(raw_expiry.replace("Z", "+00:00"))
    except ValueError as error:
        raise MigrationError("The API draft upload URL has an invalid SAS expiry.") from error
    if expiry.tzinfo is None:
        expiry = expiry.replace(tzinfo=dt.UTC)
    return expiry.astimezone(dt.UTC)


def _published_baseline_id(app: dict[str, Any]) -> str:
    return _submission_pointer_id(app.get("lastPublishedApplicationSubmission"))


def _assert_published_baseline(
    app: dict[str, Any], snapshot: dict[str, Any]
) -> None:
    actual = _published_baseline_id(app)
    expected = str(snapshot["lastPublishedSubmissionId"])
    if actual != expected:
        raise MigrationError(
            f"The published Store baseline changed from {expected} to "
            f"{actual or 'none'} after the snapshot. Nothing new was committed."
        )


def _assert_media_baseline(
    submission: dict[str, Any], snapshot: dict[str, Any]
) -> None:
    actual_manifest = media_manifest(submission)
    actual = sha256_json(actual_manifest)
    expected = str(snapshot["mediaManifestSha256"])
    if secrets_compare_digest(actual, expected):
        return
    if snapshot.get("mediaPreservationMode") == "embedded-provenance-screenshots":
        published = str(snapshot["publishedMediaManifestSha256"])
        if secrets_compare_digest(actual, published):
            return
        if recoverable_media_projection(
            actual_manifest
        ) == recoverable_media_projection(snapshot["sourceMediaManifest"]):
            return
    raise MigrationError(
        "The API-owned clone does not contain the media preserved by the "
        "snapshot. The draft was left uncommitted."
    )


def _migration_draft_ownership(
    submission: dict[str, Any],
    snapshot: dict[str, Any],
    package_name: str,
) -> str | None:
    packages = submission.get("applicationPackages", []) or []
    target_packages = [
        package for package in packages if package.get("fileName") == package_name
    ]
    old_deletes = [
        package
        for package in packages
        if package.get("fileName") != package_name
        and package.get("fileStatus") == "PendingDelete"
    ]
    if (
        len(target_packages) == 1
        and target_packages[0].get("fileStatus") in {"PendingUpload", "Uploaded"}
        and old_deletes
    ):
        return "package-marked"
    status = str(submission.get("status", ""))
    if (
        len(target_packages) == 1
        and target_packages[0].get("fileStatus") in {"PendingUpload", "Uploaded"}
        and status != "PendingCommit"
        and (
            status in COMMIT_WAIT_STATES
            or status in COMMIT_SUCCESS_STATES
            or status in COMMIT_FAILURE_STATES
        )
    ):
        return "committed-target"

    clean_clone_digest = sha256_json(source_state_projection(submission))
    if secrets_compare_digest(
        clean_clone_digest, str(snapshot["publishedCloneStateSha256"])
    ):
        return "exact-published-clone"
    return None


def _pending_submission_for_recovery(
    api: StoreApi,
    pending_id: str,
    source_id: str,
    package_name: str,
    snapshot: dict[str, Any],
) -> dict[str, Any] | None:
    if not pending_id or pending_id == source_id:
        return None
    submission = api.get_submission(pending_id)
    status = str(submission.get("status", ""))
    ownership = _migration_draft_ownership(
        submission, snapshot, package_name
    )
    if ownership is None:
        raise MigrationError(
            f"Pending submission {pending_id} cannot be proven to belong to "
            "this migration. It was not changed or deleted."
        )
    package_names = {
        package.get("fileName")
        for package in submission.get("applicationPackages", [])
    }

    if status == "PendingCommit" and submission.get("fileUploadUrl"):
        _assert_media_baseline(submission, snapshot)
        expiry = _sas_expiry(str(submission["fileUploadUrl"]))
        if expiry <= dt.datetime.now(dt.UTC) + dt.timedelta(minutes=5):
            print(
                f"Deleting expired API-owned draft {pending_id} before recreation.",
                flush=True,
            )
            api.delete_submission(pending_id)
            return None
        print(f"Resuming API-owned draft {pending_id}.", flush=True)
        return submission

    if ownership in {"package-marked", "committed-target"} and package_name in package_names and (
        status in COMMIT_WAIT_STATES
        or status in COMMIT_SUCCESS_STATES
        or status in COMMIT_FAILURE_STATES
    ):
        _assert_media_baseline(submission, snapshot)
        return submission

    if status != "PendingCommit":
        raise MigrationError(
            f"Unexpected pending submission {pending_id} "
            f"(status {status or 'None'})."
        )
    raise MigrationError(
        f"Pending submission {pending_id} is not a verified API-owned draft."
    )


def _assert_source_unchanged(
    api: StoreApi, source_id: str, snapshot: dict[str, Any]
) -> None:
    current_source = api.get_submission(source_id)
    current_digest = sha256_json(source_state_projection(current_source))
    if not secrets_compare_digest(
        current_digest, str(snapshot["sourceStateSha256"])
    ):
        raise MigrationError(
            "The Partner Center draft changed after it was snapshotted. "
            "Nothing was deleted; start a fresh migration snapshot."
        )


def wait_for_portal_deletion(
    api: StoreApi,
    *,
    source_id: str,
    package_name: str,
    snapshot: dict[str, Any],
    wait_minutes: int,
    poll_seconds: int,
) -> dict[str, Any] | None:
    initial_app = api.get_app()
    _assert_published_baseline(initial_app, snapshot)
    initial_pending_id = _submission_pointer_id(
        initial_app.get("pendingApplicationSubmission")
    )
    if not initial_pending_id:
        return None
    if initial_pending_id != source_id:
        initial_recovery = _pending_submission_for_recovery(
            api, initial_pending_id, source_id, package_name, snapshot
        )
        if initial_recovery is not None:
            return initial_recovery
        # An expired, proven API-owned draft was deleted.
        return None
    _assert_source_unchanged(api, source_id, snapshot)

    portal_url = (
        f"https://partner.microsoft.com/dashboard/products/{api.app_id}"
        f"/submissions/{source_id}"
    )
    print("Encrypted draft backup is complete.", flush=True)
    print(
        f"Delete submission {source_id} in Partner Center to continue: {portal_url}",
        flush=True,
    )
    deadline = time.monotonic() + wait_minutes * 60
    poll = 0
    while time.monotonic() < deadline:
        app = api.get_app()
        _assert_published_baseline(app, snapshot)
        pending_id = _submission_pointer_id(
            app.get("pendingApplicationSubmission")
        )
        if not pending_id:
            print(
                "Partner Center confirms that the old draft has been deleted.",
                flush=True,
            )
            return None
        recovered = _pending_submission_for_recovery(
            api, pending_id, source_id, package_name, snapshot
        )
        if recovered is not None:
            return recovered
        if pending_id == source_id:
            _assert_source_unchanged(api, source_id, snapshot)
        poll += 1
        if poll == 1 or poll % 10 == 0:
            print(
                f"Still waiting for portal draft {source_id} to be deleted "
                f"(poll {poll}).",
                flush=True,
            )
        time.sleep(poll_seconds)
    raise MigrationError(
        f"Timed out waiting for Partner Center draft {source_id} to be deleted. "
        "The encrypted snapshot was retained as a workflow artifact."
    )


def poll_commit(
    api: StoreApi,
    submission_id: str,
    *,
    poll_minutes: int,
    poll_seconds: int,
) -> str:
    deadline = time.monotonic() + poll_minutes * 60
    poll = 0
    while time.monotonic() < deadline:
        status_payload = api.get_submission_status(submission_id)
        status = str(status_payload.get("status", ""))
        poll += 1
        print(f"Commit poll {poll}: {status or 'None'}", flush=True)
        if status in COMMIT_SUCCESS_STATES:
            return status
        if status in COMMIT_FAILURE_STATES:
            details = safe_error_text(
                json.dumps(status_payload.get("statusDetails", {}))
            )
            raise MigrationError(
                f"Store submission reached failure state {status}: {details}"
            )
        if status not in COMMIT_WAIT_STATES:
            raise MigrationError(
                f"Store submission reached unknown state {status!r}; "
                "refusing to report a successful commit."
            )
        time.sleep(poll_seconds)
    raise MigrationError(
        "Timed out before Partner Center confirmed a successful commit. "
        "Check the API-owned draft in Partner Center."
    )


def append_github_summary(
    *,
    app_id: str,
    source_id: str,
    new_id: str,
    version: str,
    status: str,
) -> None:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return
    portal_url = (
        f"https://partner.microsoft.com/dashboard/products/{app_id}"
        f"/submissions/{new_id}"
    )
    with open(summary_path, "a", encoding="utf-8") as summary:
        summary.write("## Microsoft Store draft migration\n\n")
        summary.write(f"- Source draft: `{source_id}`\n")
        summary.write(f"- New API submission: `{new_id}`\n")
        summary.write(f"- Version: `{version}`\n")
        summary.write(f"- Confirmed status: `{status}`\n")
        summary.write(f"- [Open submission in Partner Center]({portal_url})\n")


def set_github_outputs(submission_id: str, status: str) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT")
    if not output_path:
        return
    with open(output_path, "a", encoding="utf-8") as output:
        output.write(f"submission_id={submission_id}\n")
        output.write(f"status={status}\n")


def record_submission_success(
    *,
    app_id: str,
    source_id: str,
    submission_id: str,
    version: str,
    status: str,
) -> None:
    print(
        f"STORE_SUBMISSION_COMMITTED id={submission_id} status={status}",
        flush=True,
    )
    set_github_outputs(submission_id, status)
    append_github_summary(
        app_id=app_id,
        source_id=source_id,
        new_id=submission_id,
        version=version,
        status=status,
    )


def command_migrate(args: argparse.Namespace) -> None:
    validate_submission_id(args.source_submission_id, "source submission ID")
    snapshot = load_snapshot(
        Path(args.snapshot), args.app_id, args.source_submission_id
    )
    validate_snapshot_target(snapshot, args.version)
    package = Path(args.package)
    package_name, package_digest = validate_package(package, args.version)
    recoverable_screenshots = validate_recoverable_screenshots(snapshot)
    release_notes = Path(args.release_notes_file).read_text(
        encoding="utf-8"
    ).strip()
    if not release_notes:
        raise MigrationError("Store release notes are empty.")
    if len(release_notes) > 1500:
        raise MigrationError("Store release notes exceed 1500 characters.")

    tenant_id, client_id, client_secret = require_credentials()
    api = StoreApi(
        tenant_id=tenant_id,
        client_id=client_id,
        client_secret=client_secret,
        app_id=args.app_id,
    )
    print(
        f"Package: {package_name} ({package.stat().st_size} bytes, "
        f"SHA-256 {package_digest})",
        flush=True,
    )
    new_submission = wait_for_portal_deletion(
        api,
        source_id=args.source_submission_id,
        package_name=package_name,
        snapshot=snapshot,
        wait_minutes=args.wait_minutes,
        poll_seconds=args.poll_seconds,
    )
    if new_submission is None:
        print("Creating an API-owned clone of the published submission.", flush=True)
        new_submission = api.create_submission()

    new_id = str(new_submission.get("id", ""))
    if not new_id:
        raise MigrationError("Partner Center returned a submission with no ID.")
    if new_id == args.source_submission_id:
        raise MigrationError("Partner Center returned the deleted source draft.")

    new_status = str(new_submission.get("status", ""))
    package_names = {
        package.get("fileName")
        for package in new_submission.get("applicationPackages", [])
    }
    if new_status in COMMIT_FAILURE_STATES:
        raise MigrationError(
            f"Recovered Store submission {new_id} is in failure state "
            f"{new_status}."
        )
    if new_status in COMMIT_SUCCESS_STATES:
        record_submission_success(
            app_id=args.app_id,
            source_id=args.source_submission_id,
            submission_id=new_id,
            version=args.version,
            status=new_status,
        )
        return
    if new_status in COMMIT_WAIT_STATES and new_status != "PendingCommit":
        if package_name not in package_names:
            raise MigrationError(
                f"In-flight submission {new_id} does not contain {package_name}."
            )
        status = poll_commit(
            api,
            new_id,
            poll_minutes=args.commit_poll_minutes,
            poll_seconds=args.poll_seconds,
        )
        record_submission_success(
            app_id=args.app_id,
            source_id=args.source_submission_id,
            submission_id=new_id,
            version=args.version,
            status=status,
        )
        return
    if (
        new_status != "PendingCommit"
        or not new_submission.get("fileUploadUrl")
    ):
        raise MigrationError(
            "Partner Center did not return an editable API-owned submission."
        )

    app_after_create = api.get_app()
    _assert_published_baseline(app_after_create, snapshot)
    if _submission_pointer_id(
        app_after_create.get("pendingApplicationSubmission")
    ) != new_id:
        raise MigrationError(
            "Partner Center's pending pointer does not match the API draft."
        )
    _assert_media_baseline(new_submission, snapshot)
    if _migration_draft_ownership(
        new_submission, snapshot, package_name
    ) is None:
        raise MigrationError(
            f"API draft {new_id} cannot be proven to belong to this migration."
        )
    if _sas_expiry(str(new_submission["fileUploadUrl"])) <= (
        dt.datetime.now(dt.UTC) + dt.timedelta(minutes=5)
    ):
        raise MigrationError("The API draft upload URL expired before migration.")
    print(f"API-owned submission: {new_id}", flush=True)

    payload = build_update_payload(
        new_submission=new_submission,
        snapshot=snapshot,
        package_file_name=package_name,
        release_notes=release_notes,
    )
    api.update_submission(new_id, payload)
    restored = api.get_submission(new_id)
    validate_restored_submission(
        desired=payload,
        actual=restored,
        package_file_name=package_name,
    )
    print("Draft metadata and package replacement verified.", flush=True)

    archive = package.parent / f"{package.stem}-store-upload.zip"
    create_upload_zip(
        package,
        archive,
        recoverable_screenshots,
        expected_package_sha256=package_digest,
    )
    restored_upload_url = str(restored.get("fileUploadUrl", ""))
    if _sas_expiry(restored_upload_url) <= (
        dt.datetime.now(dt.UTC) + dt.timedelta(minutes=5)
    ):
        raise MigrationError("The API draft upload URL expired before package upload.")
    upload_blob(restored_upload_url, archive)
    print(f"Uploaded package archive ({archive.stat().st_size} bytes).", flush=True)

    try:
        commit_response = api.commit_submission(new_id)
        initial_status = str(commit_response.get("status", ""))
        print(f"Commit accepted: {initial_status or 'status pending'}.", flush=True)
    except AmbiguousStoreOperation:
        print(
            "Commit response was ambiguous; polling status without reposting.",
            flush=True,
        )
    status = poll_commit(
        api,
        new_id,
        poll_minutes=args.commit_poll_minutes,
        poll_seconds=args.poll_seconds,
    )
    record_submission_success(
        app_id=args.app_id,
        source_id=args.source_submission_id,
        submission_id=new_id,
        version=args.version,
        status=status,
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    snapshot = subparsers.add_parser(
        "snapshot", help="Capture and validate the Partner Center draft."
    )
    snapshot.add_argument("--app-id", default=DEFAULT_PORTAL_PRODUCT)
    snapshot.add_argument("--source-submission-id", required=True)
    snapshot.add_argument("--target-version", required=True)
    snapshot.add_argument("--output", required=True)
    snapshot.add_argument("--screenshots-dir")
    snapshot.add_argument("--screenshot-provenance-submission-id")
    snapshot.add_argument("--screenshot-provenance-run-id")
    snapshot.add_argument("--screenshot-provenance-head-sha")
    snapshot.add_argument("--screenshot-provenance-repository")
    snapshot.add_argument("--screenshot-release-tag")
    snapshot.add_argument("--snapshot-commit-sha")
    snapshot.set_defaults(func=command_snapshot)

    migrate = subparsers.add_parser(
        "migrate", help="Wait for deletion, restore, upload, and commit."
    )
    migrate.add_argument("--app-id", default=DEFAULT_PORTAL_PRODUCT)
    migrate.add_argument("--source-submission-id", required=True)
    migrate.add_argument("--snapshot", required=True)
    migrate.add_argument("--version", required=True)
    migrate.add_argument("--package", required=True)
    migrate.add_argument("--release-notes-file", required=True)
    migrate.add_argument("--wait-minutes", type=int, default=240)
    migrate.add_argument("--commit-poll-minutes", type=int, default=20)
    migrate.add_argument("--poll-seconds", type=int, default=30)
    migrate.set_defaults(func=command_migrate)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
    except MigrationError as error:
        print(f"ERROR: {safe_error_text(str(error))}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
