import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from migrate_store_draft import (
    MigrationError,
    _assert_media_baseline,
    build_update_payload,
    create_upload_zip,
    load_snapshot,
    make_snapshot as _make_snapshot,
    media_manifest,
    metadata_projection,
    replace_packages,
    validate_recoverable_screenshots,
    validate_snapshot_target,
)

PNG_BYTES = b"\x89PNG\r\n\x1a\nminimal-test-png"


def make_snapshot(**kwargs):
    kwargs.setdefault("target_version", "2.5.6")
    return _make_snapshot(**kwargs)


def screenshot_provenance(source):
    return {
        "screenshot_provenance_media": media_manifest(source),
        "screenshot_provenance": {
            "repository": "faratech/wfdiag",
            "workflowRunId": "29125510899",
            "workflowRunHeadSha": "620b5a7f2f68415153e0e9fb612a353ab04c377f",
            "submissionId": "1152921505701400128",
            "releaseTag": "v2.5.6",
            "snapshotCommitSha": "e0b72838bff7d5dabfda8b82756513154481b614",
        },
    }


def listing(title="WindowsForum Diagnostics", description="Published"):
    return {
        "baseListing": {
            "title": title,
            "description": description,
            "releaseNotes": "old",
            "sortTitle": "WindowsForum",
            "images": [
                {
                    "id": "image-1",
                    "fileName": "screen.png",
                    "fileStatus": "Uploaded",
                    "imageType": "Screenshot",
                    "description": "Published screenshot",
                }
            ],
        },
        "platformOverrides": {},
    }


def submission(submission_id, *, description="Published"):
    return {
        "id": submission_id,
        "applicationCategory": "UtilitiesAndTools",
        "pricing": {
            "trialPeriod": "NoFreeTrial",
            "marketSpecificPricings": {},
            "sales": [],
            "priceId": "Free",
            "isAdvancedPricingModel": True,
        },
        "visibility": "Public",
        "targetPublishMode": "Immediate",
        "targetPublishDate": "1601-01-01T00:00:00Z",
        "listings": {"en-us": listing(description=description)},
        "hardwarePreferences": ["Keyboard", "Mouse"],
        "automaticBackupEnabled": False,
        "canInstallOnRemovableMedia": True,
        "isGameDvrEnabled": False,
        "gamingOptions": [],
        "hasExternalInAppProducts": False,
        "meetAccessibilityGuidelines": True,
        "notesForCertification": "Please test model discovery.",
        "status": "PendingCommit",
        "statusDetails": {"errors": []},
        "fileUploadUrl": "https://blob.invalid/file?sig=secret",
        "applicationPackages": [
            {
                "id": "package-1",
                "fileName": "WindowsForum_Diagnostics_2.5.4.msixbundle",
                "fileStatus": "Uploaded",
                "minimumDirectXVersion": "None",
                "minimumSystemRam": "None",
            }
        ],
        "packageDeliveryOptions": {
            "packageRollout": {
                "isPackageRollout": False,
                "packageRolloutPercentage": 0.0,
                "packageRolloutStatus": "PackageRolloutNotStarted",
                "fallbackSubmissionId": "0",
            },
            "isMandatoryUpdate": False,
            "mandatoryUpdateEffectiveDate": "1601-01-01T00:00:00Z",
        },
        "enterpriseLicensing": "Online",
        "allowMicrosoftDecideAppAvailabilityToFutureDeviceFamilies": True,
        "allowTargetFutureDeviceFamilies": {"Desktop": True},
        "friendlyName": "Submission 7",
        "trailers": [],
    }


class SnapshotTests(unittest.TestCase):
    def test_snapshot_excludes_server_fields_and_packages(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        source["status"] = "CertificationFailed"
        source["listings"]["en-us"]["baseListing"]["description"] = "Draft text"

        snapshot = make_snapshot(
            app_id="9NJ59RH053PV", source=source, published=published
        )

        self.assertNotIn("fileUploadUrl", snapshot["metadata"])
        self.assertNotIn("applicationPackages", snapshot["metadata"])
        self.assertNotIn("status", snapshot["metadata"])
        self.assertEqual(
            snapshot["metadata"]["listings"]["en-us"]["baseListing"][
                "description"
            ],
            "Draft text",
        )

    def test_snapshot_rejects_portal_only_media(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        source["listings"]["en-us"]["baseListing"]["images"][0][
            "id"
        ] = "new-portal-image"

        with self.assertRaises(MigrationError):
            make_snapshot(
                app_id="9NJ59RH053PV", source=source, published=published
            )

    def test_snapshot_rejects_screenshot_reordering(self):
        published = submission("published")
        second_image = {
            "id": "image-2",
            "fileName": "second.png",
            "fileStatus": "Uploaded",
            "imageType": "Screenshot",
        }
        published["listings"]["en-us"]["baseListing"]["images"].append(
            second_image
        )
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        source["listings"]["en-us"]["baseListing"]["images"].reverse()

        with self.assertRaises(MigrationError):
            make_snapshot(
                app_id="9NJ59RH053PV", source=source, published=published
            )

    def test_snapshot_accepts_exact_checked_in_screenshot_replacement(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        image = source["listings"]["en-us"]["baseListing"]["images"][0]
        image["id"] = "portal-image"
        image["fileName"] = "01-current.png"
        with tempfile.TemporaryDirectory() as directory:
            screenshot_dir = Path(directory)
            (screenshot_dir / "01-current.png").write_bytes(PNG_BYTES)
            snapshot = make_snapshot(
                app_id="9NJ59RH053PV",
                source=source,
                published=published,
                screenshots_dir=screenshot_dir,
                **screenshot_provenance(source),
            )

        self.assertEqual(
            snapshot["mediaPreservationMode"],
            "embedded-provenance-screenshots",
        )
        self.assertIn(
            "01-current.png", snapshot["recoverableScreenshotFiles"]
        )
        self.assertEqual(
            validate_recoverable_screenshots(snapshot)["01-current.png"],
            PNG_BYTES,
        )

    def test_snapshot_rejects_media_that_only_matches_by_filename(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        source_image = source["listings"]["en-us"]["baseListing"]["images"][0]
        source_image["id"] = "portal-image"
        source_image["fileName"] = "01-current.png"
        provenance_submission = copy.deepcopy(source)
        provenance_submission["listings"]["en-us"]["baseListing"]["images"][0][
            "id"
        ] = "different-store-asset"
        with tempfile.TemporaryDirectory() as directory:
            screenshot_dir = Path(directory)
            (screenshot_dir / "01-current.png").write_bytes(PNG_BYTES)
            with self.assertRaises(MigrationError):
                make_snapshot(
                    app_id="9NJ59RH053PV",
                    source=source,
                    published=published,
                    screenshots_dir=screenshot_dir,
                    screenshot_provenance_media=media_manifest(
                        provenance_submission
                    ),
                    screenshot_provenance=screenshot_provenance(source)[
                        "screenshot_provenance"
                    ],
                )

    def test_snapshot_rejects_symlinked_screenshot(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        image = source["listings"]["en-us"]["baseListing"]["images"][0]
        image["id"] = "portal-image"
        image["fileName"] = "01-current.png"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            secret = root / "secret.bin"
            secret.write_bytes(PNG_BYTES)
            screenshot_dir = root / "shots"
            screenshot_dir.mkdir()
            (screenshot_dir / "01-current.png").symlink_to(secret)
            with self.assertRaises(MigrationError):
                make_snapshot(
                    app_id="9NJ59RH053PV",
                    source=source,
                    published=published,
                    screenshots_dir=screenshot_dir,
                    **screenshot_provenance(source),
                )

    def test_snapshot_rejects_windows_traversal_screenshot_name(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        image = source["listings"]["en-us"]["baseListing"]["images"][0]
        image["id"] = "portal-image"
        image["fileName"] = r"..\outside.png"
        with tempfile.TemporaryDirectory() as directory:
            screenshot_dir = Path(directory)
            (screenshot_dir / r"..\outside.png").write_bytes(PNG_BYTES)
            with self.assertRaises(MigrationError):
                make_snapshot(
                    app_id="9NJ59RH053PV",
                    source=source,
                    published=published,
                    screenshots_dir=screenshot_dir,
                    **screenshot_provenance(source),
                )

    def test_media_manifest_ignores_description_only(self):
        first = submission("one")
        second = copy.deepcopy(first)
        second["listings"]["en-us"]["baseListing"]["images"][0][
            "description"
        ] = "New description"
        self.assertEqual(media_manifest(first), media_manifest(second))

    def test_snapshot_integrity_is_verified_on_load(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        snapshot = make_snapshot(
            app_id="9NJ59RH053PV", source=source, published=published
        )
        snapshot["metadata"]["notesForCertification"] = "tampered"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "snapshot.json"
            path.write_text(json.dumps(snapshot), encoding="utf-8")
            with self.assertRaises(MigrationError):
                load_snapshot(path, "9NJ59RH053PV", "portal-draft")

    def test_snapshot_is_bound_to_target_version(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        snapshot = make_snapshot(
            app_id="9NJ59RH053PV",
            source=source,
            published=published,
            target_version="2.5.6",
        )
        validate_snapshot_target(snapshot, "2.5.6")
        with self.assertRaises(MigrationError):
            validate_snapshot_target(snapshot, "2.5.7")

    def test_embedded_screenshot_provenance_is_bound_to_release_tag(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        image = source["listings"]["en-us"]["baseListing"]["images"][0]
        image["id"] = "portal-image"
        image["fileName"] = "01-current.png"
        with tempfile.TemporaryDirectory() as directory:
            screenshot_dir = Path(directory)
            (screenshot_dir / "01-current.png").write_bytes(PNG_BYTES)
            snapshot = make_snapshot(
                app_id="9NJ59RH053PV",
                source=source,
                published=published,
                screenshots_dir=screenshot_dir,
                **screenshot_provenance(source),
            )
        snapshot["screenshotProvenance"]["releaseTag"] = "v2.5.7"
        with self.assertRaises(MigrationError):
            validate_snapshot_target(snapshot, "2.5.6")


class MergeTests(unittest.TestCase):
    def test_merge_uses_new_media_and_packages_but_source_text(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        source["status"] = "CertificationFailed"
        source["listings"]["en-us"]["baseListing"]["description"] = "Draft text"
        source["listings"]["en-us"]["baseListing"]["sortTitle"] = "Diagnostics"
        source["listings"]["en-us"]["baseListing"]["images"][0][
            "description"
        ] = "Draft image description"
        snapshot = make_snapshot(
            app_id="9NJ59RH053PV", source=source, published=published
        )

        new_submission = submission("api-draft")
        new_submission["fileUploadUrl"] = "https://blob.invalid/new?sig=secret"
        payload = build_update_payload(
            new_submission=new_submission,
            snapshot=snapshot,
            package_file_name="WindowsForum_Diagnostics_2.5.6.msixbundle",
            release_notes="Version 2.5.6 notes",
        )

        base = payload["listings"]["en-us"]["baseListing"]
        self.assertEqual(base["description"], "Draft text")
        self.assertEqual(base["sortTitle"], "Diagnostics")
        self.assertEqual(base["releaseNotes"], "Version 2.5.6 notes")
        self.assertEqual(base["images"][0]["id"], "image-1")
        self.assertEqual(
            base["images"][0]["description"], "Draft image description"
        )
        self.assertEqual(
            payload["applicationPackages"][0]["fileStatus"], "PendingDelete"
        )
        self.assertEqual(payload["applicationPackages"][0]["id"], "package-1")
        self.assertEqual(
            payload["applicationPackages"][1],
            {
                "fileName": "WindowsForum_Diagnostics_2.5.6.msixbundle",
                "fileStatus": "PendingUpload",
                "minimumDirectXVersion": "None",
                "minimumSystemRam": "None",
            },
        )
        for forbidden in (
            "id",
            "status",
            "statusDetails",
            "fileUploadUrl",
            "friendlyName",
        ):
            self.assertNotIn(forbidden, payload)
        self.assertNotIn("isAdvancedPricingModel", payload["pricing"])

    def test_package_replacement_is_idempotent_on_resume(self):
        existing = submission("api-draft")["applicationPackages"]
        first = replace_packages(
            existing, "WindowsForum_Diagnostics_2.5.6.msixbundle"
        )
        second = replace_packages(
            first, "WindowsForum_Diagnostics_2.5.6.msixbundle"
        )
        names = [package["fileName"] for package in second]
        self.assertEqual(
            names.count("WindowsForum_Diagnostics_2.5.6.msixbundle"), 1
        )
        self.assertEqual(second[0]["id"], "package-1")
        self.assertEqual(second[0]["fileStatus"], "PendingDelete")

    def test_clone_media_must_match_snapshot(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        snapshot = make_snapshot(
            app_id="9NJ59RH053PV", source=source, published=published
        )
        clone = submission("api-draft")
        clone["listings"]["en-us"]["baseListing"]["images"][0][
            "id"
        ] = "different-image"
        with self.assertRaises(MigrationError):
            _assert_media_baseline(clone, snapshot)

    def test_recoverable_screenshot_is_reuploaded_in_source_order(self):
        published = submission("published")
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        image = source["listings"]["en-us"]["baseListing"]["images"][0]
        image["id"] = "portal-image"
        image["fileName"] = "01-current.png"
        image["description"] = "Current UI"
        with tempfile.TemporaryDirectory() as directory:
            screenshot_dir = Path(directory)
            (screenshot_dir / "01-current.png").write_bytes(PNG_BYTES)
            snapshot = make_snapshot(
                app_id="9NJ59RH053PV",
                source=source,
                published=published,
                screenshots_dir=screenshot_dir,
                **screenshot_provenance(source),
            )

        clone = submission("api-draft")
        _assert_media_baseline(clone, snapshot)
        payload = build_update_payload(
            new_submission=clone,
            snapshot=snapshot,
            package_file_name="WindowsForum_Diagnostics_2.5.6.msixbundle",
            release_notes="Version 2.5.6 notes",
        )
        images = payload["listings"]["en-us"]["baseListing"]["images"]
        self.assertEqual(
            images,
            [
                {
                    "fileName": "01-current.png",
                    "imageType": "Screenshot",
                    "description": "Current UI",
                    "fileStatus": "PendingUpload",
                }
            ],
        )

    def test_upload_zip_contains_package_and_recovered_screenshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "WindowsForum_Diagnostics_2.5.6.msixbundle"
            archive = root / "upload.zip"
            package.write_bytes(b"package")
            create_upload_zip(
                package, archive, {"01-current.png": PNG_BYTES}
            )
            import zipfile

            with zipfile.ZipFile(archive) as upload:
                self.assertEqual(
                    upload.namelist(),
                    [
                        "WindowsForum_Diagnostics_2.5.6.msixbundle",
                        "01-current.png",
                    ],
                )
                self.assertEqual(upload.read("01-current.png"), PNG_BYTES)

    def test_restored_validation_keeps_existing_image_ids_strict(self):
        desired = submission("desired")
        actual = copy.deepcopy(desired)
        actual["listings"]["en-us"]["baseListing"]["images"][0][
            "id"
        ] = "different-image"
        self.assertNotEqual(
            metadata_projection(desired), metadata_projection(actual)
        )

    def test_unknown_image_metadata_remains_bound_to_image_identity(self):
        published = submission("published")
        second = copy.deepcopy(
            published["listings"]["en-us"]["baseListing"]["images"][0]
        )
        second["id"] = "image-2"
        second["fileName"] = "second.png"
        published["listings"]["en-us"]["baseListing"]["images"].append(second)
        source = copy.deepcopy(published)
        source["id"] = "portal-draft"
        published["listings"]["en-us"]["baseListing"]["images"][0][
            "futureField"
        ] = "value"
        source["listings"]["en-us"]["baseListing"]["images"][1][
            "futureField"
        ] = "value"
        with self.assertRaises(MigrationError):
            make_snapshot(
                app_id="9NJ59RH053PV",
                source=source,
                published=published,
            )

    def test_projection_ignores_server_package_state(self):
        desired = submission("desired")
        actual = copy.deepcopy(desired)
        actual["applicationPackages"][0]["fileStatus"] = "Uploaded"
        self.assertEqual(metadata_projection(desired), metadata_projection(actual))

    def test_requires_existing_bundle(self):
        with self.assertRaises(MigrationError):
            replace_packages(
                [{"fileName": "legacy.appx", "fileStatus": "Uploaded"}],
                "WindowsForum_Diagnostics_2.5.6.msixbundle",
            )


if __name__ == "__main__":
    unittest.main()
