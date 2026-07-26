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
    load_snapshot,
    make_snapshot,
    media_manifest,
    metadata_projection,
    replace_packages,
)


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
