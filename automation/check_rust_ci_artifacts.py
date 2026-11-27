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
        self.assertTrue((bundle / "integration_harness_test").stat().st_mode & 0o111)
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

    def test_outbox_a14_owner_is_distinct_hashed_and_fail_closed(self) -> None:
        source = self.executable("integration_outbox_e2e-build-hash", b"real A14 test binary")
        bundle = self.root / "outbox_bundle"
        manifest = artifacts.package("outbox_harness", SHA, bundle, harness=source)
        self.assertEqual(set(manifest["files"]), {"integration_outbox_e2e_test"})
        self.assertEqual(manifest["total_bytes"], source.stat().st_size)
        installed = self.root / "outbox_install" / "debug"
        with self.assertRaisesRegex(ValueError, "manifest"):
            artifacts.install("harness", SHA, bundle, target=installed)
        with self.assertRaisesRegex(ValueError, "manifest"):
            artifacts.install("pg_harness", SHA, bundle, target=installed)
        artifacts.install("outbox_harness", SHA, bundle, target=installed)
        self.assertEqual(
            (installed / "deps/integration_outbox_e2e-ci-exact").read_bytes(),
            source.read_bytes(),
        )
        self.assertFalse((installed / "deps/integration_harness-ci-exact").exists())
        (bundle / "integration_outbox_e2e_test").write_bytes(b"forged")
        with self.assertRaisesRegex(ValueError, "digest"):
            artifacts.install("outbox_harness", SHA, bundle, target=self.root / "forged_install")

    def test_next_physical_owner_artifacts_cannot_be_substituted_for_each_other(self) -> None:
        # The new PG, Qdrant, Graph and Temporal E2E binaries are independently
        # compiled, attest the same exact source SHA, and never satisfy one
        # another's manifest. A graph test cannot accidentally run Temporal.
        cases = {
            "graph_harness": "integration_graph_e2e",
            "qdrant_harness": "integration_qdrant_e2e",
            "qdrant_temporal_harness": "integration_qdrant_temporal_e2e",
            "temporal_harness": "integration_temporal_e2e",
            "pg_app_harness": "integration_pg_app_e2e",
        }
        for kind, package in cases.items():
            with self.subTest(kind=kind):
                source = self.executable(f"{package}-hash", kind.encode())
                bundle = self.root / f"{kind}_bundle"
                manifest = artifacts.package(kind, SHA, bundle, harness=source)
                self.assertEqual(set(manifest["files"]), {f"{package}_test"})
                installed = self.root / f"{kind}_installed"
                for wrong in set(cases) - {kind}:
                    with self.assertRaisesRegex(ValueError, "manifest"):
                        artifacts.install(wrong, SHA, bundle, target=installed)
                artifacts.install(kind, SHA, bundle, target=installed)
                self.assertEqual(
                    (installed / f"deps/{package}-ci-exact").read_bytes(), source.read_bytes(),
                )
                (bundle / f"{package}_test").write_bytes(b"changed after signing")
                with self.assertRaisesRegex(ValueError, "digest"):
                    artifacts.install(kind, SHA, bundle, target=self.root / f"{kind}_forged")

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

    def test_qdrant_effect_worker_is_independent_of_pg_and_legacy_workers(self) -> None:
        with patch.object(artifacts, "TARGET", self.target):
            original = self.executable("temporal_qdrant_effects", b"real independent Qdrant SDK worker")
            bundle = self.root / "qdrant_worker_bundle"
            manifest = artifacts.package("qdrant_effects", SHA, bundle)
            self.assertEqual(set(manifest["files"]), {"temporal_qdrant_effects"})
            for incompatible in ("v3", "legacy", "qdrant_temporal_harness"):
                with self.subTest(incompatible=incompatible):
                    with self.assertRaisesRegex(ValueError, "manifest"):
                        artifacts.install(incompatible, SHA, bundle, target=self.root / incompatible)
            destination = self.root / "installed_qdrant"
            artifacts.install("qdrant_effects", SHA, bundle, target=destination)
            self.assertEqual((destination / "temporal_qdrant_effects").read_bytes(), original.read_bytes())
            self.assertFalse((destination / "temporal_orchestrator").exists())
            (bundle / "temporal_qdrant_effects").write_bytes(b"forged")
            with self.assertRaisesRegex(ValueError, "digest"):
                artifacts.install("qdrant_effects", SHA, bundle, target=self.root / "forged_qdrant")

    def test_legacy_qdrant_parent_separate_from_provider_free_generic_binary(self) -> None:
        with patch.object(artifacts, "TARGET", self.target):
            source = self.executable("temporal_orchestrator", b"feature-enabled frozen Qdrant V1")
            legacy_bundle = self.root / "legacy_qdrant"
            packaged = artifacts.package("legacy_qdrant_parent", SHA, legacy_bundle)
            self.assertEqual(set(packaged["files"]), {"temporal_orchestrator_legacy_qdrant_v1"})
            self.assertTrue((legacy_bundle / "temporal_orchestrator_legacy_qdrant_v1").stat().st_mode & 0o111)
            # Bundle bytes are frozen when the default-feature Cargo build is
            # packaged earlier; later legacy Cargo linking cannot overwrite it.
            installed = self.root / "installed"
            artifacts.install("legacy_qdrant_parent", SHA, legacy_bundle, target=installed)
            self.assertEqual((installed / "temporal_orchestrator_legacy_qdrant_v1").read_bytes(), source.read_bytes())
            self.assertFalse((installed / "temporal_orchestrator").exists())
            for wrong in ("v3", "legacy", "qdrant_effects"):
                with self.subTest(wrong=wrong), self.assertRaisesRegex(ValueError, "manifest"):
                    artifacts.install(wrong, SHA, legacy_bundle, target=self.root / f"wrong-{wrong}")
            (legacy_bundle / "temporal_orchestrator_legacy_qdrant_v1").write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "digest"):
                artifacts.install("legacy_qdrant_parent", SHA, legacy_bundle, target=self.root / "tampered")

    def test_acquisition_worker_and_independent_source_issuer_are_exact_sha_tamper_evident(self) -> None:
        with patch.object(artifacts, "TARGET", self.target):
            source = self.executable("temporal_acquisition_pure", b"native pure acquisition owner")
            issuer = self.executable("acquisition_source_issuer", b"separately credentialed raw HTML issuer")
            bundle = self.root / "acquisition_bundle"
            manifest = artifacts.package("acquisition_pure", SHA, bundle)
            self.assertEqual(set(manifest["files"]), {
                "temporal_acquisition_pure", "acquisition_source_issuer",
            })
            for wrong in ("v3", "dataforseo_effects", "qdrant_effects"):
                with self.subTest(wrong=wrong), self.assertRaisesRegex(ValueError, "manifest"):
                    artifacts.install(wrong, SHA, bundle, target=self.root / f"wrong_{wrong}")
            destination = self.root / "acquisition_install"
            artifacts.install("acquisition_pure", SHA, bundle, target=destination)
            self.assertEqual((destination / "temporal_acquisition_pure").read_bytes(), source.read_bytes())
            self.assertEqual((destination / "acquisition_source_issuer").read_bytes(), issuer.read_bytes())
            (bundle / "temporal_acquisition_pure").write_bytes(b"forged owner executable")
            with self.assertRaisesRegex(ValueError, "digest"):
                artifacts.install("acquisition_pure", SHA, bundle, target=self.root / "forged")
            (bundle / "temporal_acquisition_pure").write_bytes(source.read_bytes())
            (bundle / "acquisition_source_issuer").write_bytes(b"forged SQL issuer executable")
            with self.assertRaisesRegex(ValueError, "digest"):
                artifacts.install("acquisition_pure", SHA, bundle, target=self.root / "forged_issuer")

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
