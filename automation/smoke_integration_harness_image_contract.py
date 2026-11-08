#!/usr/bin/env python3
"""The real e2e image pins must match the actual Compose runtime."""
import unittest

from check_integration_harness_present import image_contract_failures, ROOT


class IntegrationImageParityTests(unittest.TestCase):
    def setUp(self):
        self.compose = (ROOT / "docker-compose.yml").read_text(encoding="utf-8")
        self.harness = (
            ROOT / "app/rust/crates/integration_harness/src/containers.rs"
        ).read_text(encoding="utf-8")

    def test_real_harness_matches_real_compose(self):
        self.assertEqual(image_contract_failures(self.harness, self.compose), [])

    def test_stale_neo4j_harness_cannot_pass(self):
        wrong = self.harness.replace(
            'GenericImage::new("neo4j", "2026.10.1")',
            'GenericImage::new("neo4j", "5.26.0-community")',
        )
        self.assertIn("harness image mismatch: neo4j", " ".join(
            image_contract_failures(wrong, self.compose)
        ))

    def test_stale_qdrant_harness_cannot_pass(self):
        wrong = self.harness.replace(
            'GenericImage::new("qdrant/qdrant", "v1.20.0")',
            'GenericImage::new("qdrant/qdrant", "v1.9.0")',
        )
        self.assertIn("harness image mismatch: qdrant", " ".join(
            image_contract_failures(wrong, self.compose)
        ))

    def test_retired_temporal_auto_setup_cannot_pass(self):
        wrong = self.harness.replace(
            'GenericImage::new("temporalio/server", "1.32.2")',
            'GenericImage::new("temporalio/auto-setup", "1.28")',
        )
        self.assertIn("harness image mismatch: temporal-server", " ".join(
            image_contract_failures(wrong, self.compose)
        ))


if __name__ == "__main__":
    unittest.main()
