#!/usr/bin/env python3
"""Regression tests for the staging-only Cloudflare deployment boundary."""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import subprocess

from deploy_seo_static_cloudflare import (
    EXPECTED_ORIGIN, preflight, prepare_public_assets, run_cf_deploy, validate_release,
)


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

    def test_upload_excludes_internal_release_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            public = prepare_public_assets(self.release, Path(directory) / "public")
            self.assertTrue((public / "index.html").exists())
            self.assertTrue((public / "robots.txt").exists())
            self.assertFalse((public / "alegria-static-manifest.json").exists())
            self.assertFalse((public / ".alegria_static_site").exists())

    def test_native_cf_build_prebuilt_deploy_only(self) -> None:
        commands = []
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "cf"
            config.mkdir()

            def invoke(argv, **kwargs):
                commands.append(list(argv))
                source = Path(kwargs["env"]["ALEGRIA_CF_PUBLIC_DIR"])
                self.assertTrue((source / "index.html").is_file())
                self.assertFalse((source / "alegria-static-manifest.json").exists())
                if argv[1] == "build":
                    output = config / ".cloudflare/output/v0"
                    output.mkdir(parents=True)
                    (output / "config.json").write_text("{}", encoding="utf-8")
                return subprocess.CompletedProcess(argv, 0)

            with mock.patch("deploy_seo_static_cloudflare.CLOUD_CONFIG", config):
                with mock.patch("deploy_seo_static_cloudflare.subprocess.run", side_effect=invoke):
                    self.assertTrue(run_cf_deploy(Path("/usr/bin/cf"), self.release))
        self.assertEqual(
            [argv[1:] for argv in commands],
            [["build"], ["deploy", "--prebuilt", "--mode", "production"]],
        )

    def test_failed_cf_build_never_uploads(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory)
            with mock.patch("deploy_seo_static_cloudflare.CLOUD_CONFIG", config):
                with mock.patch(
                    "deploy_seo_static_cloudflare.subprocess.run",
                    return_value=subprocess.CompletedProcess(["cf", "build"], 1),
                ) as runner:
                    self.assertFalse(run_cf_deploy(Path("/usr/bin/cf"), self.release))
                    runner.assert_called_once()


if __name__ == "__main__":
    unittest.main()
