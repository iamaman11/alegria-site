#!/usr/bin/env python3
"""Behavioral acceptance of exact-run immutable release provenance."""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from check_first_site_release import verify_release


RUN_ID = "9c337079-ae0b-4cb4-b052-bb3a0ca623c1"


class FirstSiteReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.root = self.base / ".alegria-static-releases"
        self.root.mkdir()
        self.release = self.root / "candidate-published"
        self.release.mkdir()
        self.public = self.base / "static-site"
        self.public.symlink_to(self.release, target_is_directory=True)
        (self.release / ".alegria_static_site").write_bytes(b"alegria_immutable_release@1\n")
        self.manifest = {
            "publication_run_id": RUN_ID,
            "publication_page_node_key": "page-one",
            "publication_revision_id": "revision-one",
            "page_count": 2,
            "pages": [
                {"page_node_key": "page-one", "revision_id": "revision-one",
                 "status": "published"},
                {"page_node_key": "page-two", "revision_id": "older-revision",
                 "status": "published"},
            ],
        }
        self.save()

    def save(self) -> None:
        (self.release / "alegria-static-manifest.json").write_text(
            json.dumps(self.manifest), encoding="utf-8"
        )

    def check(self) -> dict:
        return verify_release(self.public, self.root, RUN_ID)

    def test_exact_run_and_approved_revision_pass(self) -> None:
        result = self.check()
        self.assertEqual(result["run_id"], RUN_ID)
        self.assertEqual(result["page_count"], 2)

    def test_old_release_from_different_run_is_not_accepted(self) -> None:
        self.manifest["publication_run_id"] = "a499ea56-5ffc-41ac-bab7-c2435cc6792f"
        self.save()
        with self.assertRaisesRegex(ValueError, "not published by this"):
            self.check()

    def test_missing_originating_revision_is_blocked(self) -> None:
        self.manifest["publication_revision_id"] = "unrelated"
        self.save()
        with self.assertRaisesRegex(ValueError, "originating revision"):
            self.check()

    def test_non_published_dependency_is_blocked(self) -> None:
        self.manifest["pages"][1]["status"] = "approved"
        self.save()
        with self.assertRaisesRegex(ValueError, "originating revision"):
            self.check()

    def test_non_managed_directory_is_blocked(self) -> None:
        self.public.unlink()
        self.public.mkdir()
        with self.assertRaisesRegex(ValueError, "managed immutable"):
            self.check()

    def test_release_outside_expected_root_is_blocked(self) -> None:
        other = self.base / "foreign-published"
        other.mkdir()
        self.public.unlink()
        self.public.symlink_to(other, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "direct immutable release"):
            self.check()

    def test_corrupt_release_marker_is_blocked(self) -> None:
        (self.release / ".alegria_static_site").write_text("fake", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "marker"):
            self.check()

    def test_broken_release_pointer_is_blocked(self) -> None:
        self.release.rename(self.root / "gone")
        with self.assertRaises(OSError):
            self.check()

    def test_malformed_run_id_is_blocked(self) -> None:
        with self.assertRaises(ValueError):
            verify_release(self.public, self.root, "not-a-run-id")


if __name__ == "__main__":
    unittest.main()
