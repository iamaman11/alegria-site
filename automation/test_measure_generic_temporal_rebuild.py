#!/usr/bin/env python3
"""No-Rust unit checks for strict empirical generic-parent independence A/B."""

from __future__ import annotations

import unittest

from compare_physical_build_options import EvidenceError
from measure_generic_temporal_rebuild import (
    DEFAULT_CMD, LEGACY_CMD, assert_expected_rebuilds,
)


def result(*targets: str) -> dict:
    return {"non_fresh_units": [{"target_name": target} for target in targets]}


class GenericTemporalRebuildTests(unittest.TestCase):
    def test_only_real_wrapper_free_default_feature_command_is_allowed(self) -> None:
        self.assertIn("--locked", DEFAULT_CMD)
        self.assertIn("--no-default-features", DEFAULT_CMD)
        self.assertNotIn("--features", DEFAULT_CMD)
        self.assertIn("--no-default-features", LEGACY_CMD)
        self.assertEqual(LEGACY_CMD[LEGACY_CMD.index("--features") + 1], "legacy-qdrant-v1")

    def test_default_cannot_recompile_or_relink_after_qdrant_source_change(self) -> None:
        assert_expected_rebuilds(result("temporal_orchestrator"), result(),
                                 result("qdrant_contracts", "temporal_orchestrator"), "same", "same")
        for bad_default in (result("temporal_orchestrator"), result("qdrant_contracts")):
            with self.subTest(bad_default=bad_default), self.assertRaises(EvidenceError):
                assert_expected_rebuilds(result(), bad_default,
                                         result("qdrant_contracts", "temporal_orchestrator"),
                                         "same", "same")
        with self.assertRaisesRegex(EvidenceError, "binary bytes"):
            assert_expected_rebuilds(result(), result(),
                                     result("qdrant_contracts", "temporal_orchestrator"),
                                     "same", "different")

    def test_legacy_must_actually_rebuild_both_changed_contract_and_parent(self) -> None:
        for missing in (result("qdrant_contracts"), result("temporal_orchestrator"), result()):
            with self.subTest(missing=missing), self.assertRaisesRegex(EvidenceError, "legacy-enabled"):
                assert_expected_rebuilds(result(), result(), missing, "same", "same")


if __name__ == "__main__":
    unittest.main()
