#!/usr/bin/env python3
"""Offline mutation contract for strict exact-source Rust binary handoffs."""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import rust_ci_artifacts as artifacts


SHA = "a" * 40


class RustArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / "target" / "debug"
        self.target.mkdir(parents=True)
        self.patch_source = patch.object(artifacts, "assert_source_sha", side_effect=lambda actual: (
            None if actual == SHA else (_ for _ in ()).throw(ValueError("wrong source"))
        ))
        self.patch_source.start()
        self.addCleanup(self.patch_source.stop)

    def executable(self, name: str, data: bytes = b"rust-executable\n") -> Path:
        path = self.target / name
        path.write_bytes(data)
        path.chmod(0o755)
        return path

    def test_harness_bundle_content_exact_and_non_overwriting(self) -> None:
        source = self.executable("original-test-binary")
        bundle = self.root / "bundle"
        manifest = artifacts.package("harness", SHA, bundle, harness=source)
        self.assertEqual(set(manifest["files"]), {"integration_harness_test"})
        destination = self.root / "installed" / "debug"
        artifacts.install("harness", SHA, bundle, target=destination)
        installed = destination / "deps" / "integration_harness-ci-exact"
        self.assertEqual(installed.read_bytes(), source.read_bytes())
        self.assertTrue(installed.stat().st_mode & 0o111)
        with self.assertRaisesRegex(ValueError, "overwrite"):
            artifacts.install("harness", SHA, bundle, target=destination)

    def test_pg_v3_owner_is_distinct_verified_exact_source_test_artifact(self) -> None:
        source = self.executable("integration_pg_v3_e2e-build-hash", b"real isolated PG E2E executable")
        bundle = self.root / "pg_bundle"
        manifest = artifacts.package("pg_harness", SHA, bundle, harness=source)
        self.assertEqual(set(manifest["files"]), {"integration_pg_v3_e2e_test"})
        self.assertEqual(manifest["total_bytes"], source.stat().st_size)
        self.assertEqual(manifest["files"]["integration_pg_v3_e2e_test"]["size"], source.stat().st_size)
        destination = self.root / "pg_install" / "debug"
        with self.assertRaisesRegex(ValueError, "manifest"):
            artifacts.install("harness", SHA, bundle, target=destination)
        artifacts.install("pg_harness", SHA, bundle, target=destination)
        installed = destination / "deps" / "integration_pg_v3_e2e-ci-exact"
        self.assertEqual(installed.read_bytes(), source.read_bytes())
        self.assertFalse((destination / "deps" / "integration_harness-ci-exact").exists())
        (bundle / "integration_pg_v3_e2e_test").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "digest"):
            artifacts.install("pg_harness", SHA, bundle, target=self.root / "tampered_install")

    def test_worker_owner_rejects_tamper_wrong_sha_and_wrong_kind(self) -> None:
        with patch.object(artifacts, "TARGET", self.target):
            self.executable("temporal_worker", b"legacy-real-binary")
            self.executable("outbox_worker", b"outbox-real-binary")
            bundle = self.root / "bundle"
            artifacts.package("legacy", SHA, bundle)
            with self.assertRaisesRegex(ValueError, "wrong source"):
                artifacts.install("legacy", "b" * 40, bundle, target=self.root / "installed")
            with self.assertRaisesRegex(ValueError, "manifest"):
                artifacts.install("v3", SHA, bundle, target=self.root / "installed")
            (bundle / "outbox_worker").write_bytes(b"forged")
            with self.assertRaisesRegex(ValueError, "digest"):
                artifacts.install("legacy", SHA, bundle, target=self.root / "installed")

    def test_rejects_manifest_confusion_symlink_and_extra_payload(self) -> None:
        source = self.executable("real")
        bundle = self.root / "bundle"
        artifacts.package("harness", SHA, bundle, harness=source)
        (bundle / "unexpected.exe").write_bytes(b"unknown")
        with self.assertRaisesRegex(ValueError, "manifest"):
            artifacts.install("harness", SHA, bundle, target=self.root / "installed")
        (bundle / "unexpected.exe").unlink()
        (bundle / "integration_harness_test").unlink()
        (bundle / "integration_harness_test").symlink_to(source)
        with self.assertRaisesRegex(ValueError, "invalid executable"):
            artifacts.install("harness", SHA, bundle, target=self.root / "installed")

    def test_manifest_is_json_typed_and_cannot_select_paths(self) -> None:
        source = self.executable("real")
        bundle = self.root / "bundle"
        artifacts.package("harness", SHA, bundle, harness=source)
        manifest = bundle / "manifest.json"
        body = json.loads(manifest.read_text())
        body["files"] = {"../escape": body["files"]["integration_harness_test"]}
        manifest.write_text(json.dumps(body))
        with self.assertRaisesRegex(ValueError, "manifest"):
            artifacts.install("harness", SHA, bundle, target=self.root / "installed")

    def test_total_bytes_is_content_verified_not_a_cost_guess(self) -> None:
        source = self.executable("real")
        bundle = self.root / "bundle"
        manifest = artifacts.package("harness", SHA, bundle, harness=source)
        self.assertEqual(manifest["total_bytes"], len(source.read_bytes()))
        body = json.loads((bundle / "manifest.json").read_text())
        body["total_bytes"] += 1
        (bundle / "manifest.json").write_text(json.dumps(body))
        with self.assertRaisesRegex(ValueError, "total byte count"):
            artifacts.install("harness", SHA, bundle, target=self.root / "installed")


if __name__ == "__main__":
    unittest.main()
