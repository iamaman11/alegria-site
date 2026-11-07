#!/usr/bin/env python3
"""Regression tests for the staging-only Cloudflare deployment boundary."""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from deploy_seo_static_cloudflare import EXPECTED_ORIGIN, preflight, validate_release


class CloudflareStaticDeploymentSmoke(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="alegria-cf-static-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "releases"
        self.root.mkdir()
        self.release = self.root / "approved-123-published"
        self.release.mkdir()
        (self.release / ".alegria_static_site").write_text(
            "alegria_immutable_release@1\n", encoding="utf-8"
        )
        (self.release / "index.html").write_text(
            '<!doctype html><html lang="ru"><head><link rel="canonical" '
            f'href="{EXPECTED_ORIGIN}/"></head><body><h1>Проверка</h1></body></html>',
            encoding="utf-8",
        )
        (self.release / "sitemap.xml").write_text(
            '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
            f'<url><loc>{EXPECTED_ORIGIN}/</loc></url></urlset>',
            encoding="utf-8",
        )
        (self.release / "robots.txt").write_text(
            "User-agent: *\nDisallow: /\n", encoding="utf-8"
        )
        self.manifest = {
            "base_url": EXPECTED_ORIGIN, "page_count": 1,
            "pages": [{
                "page_node_key": "test", "revision_id": "revision",
                "canonical_url_path": "/", "title": "Проверка", "status": "published",
            }],
        }
        self.persist_manifest()

    def persist_manifest(self) -> None:
        (self.release / "alegria-static-manifest.json").write_text(
            json.dumps(self.manifest), encoding="utf-8"
        )

    def test_valid_immutable_release_passes_same_public_contract(self) -> None:
        validated = validate_release(
            self.release, root=self.root, expected_origin=EXPECTED_ORIGIN
        )
        self.assertEqual(validated, self.release)
        preflight(validated)

    def test_unapproved_page_is_rejected(self) -> None:
        self.manifest["pages"][0]["status"] = "approved"
        self.persist_manifest()
        with self.assertRaisesRegex(ValueError, "published status"):
            validate_release(self.release, root=self.root, expected_origin=EXPECTED_ORIGIN)

    def test_incorrect_or_placeholder_origin_is_rejected(self) -> None:
        self.manifest["base_url"] = "https://example.com"
        self.persist_manifest()
        with self.assertRaisesRegex(ValueError, "origin"):
            validate_release(self.release, root=self.root, expected_origin=EXPECTED_ORIGIN)

    def test_symlinked_asset_is_rejected(self) -> None:
        (self.release / "unexpected.html").symlink_to(self.release / "index.html")
        with self.assertRaisesRegex(ValueError, "symlinks"):
            validate_release(self.release, root=self.root, expected_origin=EXPECTED_ORIGIN)

    def test_foreign_release_root_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "immediate immutable child"):
            validate_release(self.release, root=Path(self.temp.name), expected_origin=EXPECTED_ORIGIN)

    def test_no_immutable_marker_is_rejected(self) -> None:
        (self.release / ".alegria_static_site").unlink()
        with self.assertRaisesRegex(ValueError, "marker"):
            validate_release(self.release, root=self.root, expected_origin=EXPECTED_ORIGIN)


if __name__ == "__main__":
    unittest.main()
