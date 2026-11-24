#!/usr/bin/env python3
"""Regression tests for the source-driven Issue #94 audit (no Rust build)."""

from __future__ import annotations

import tempfile
import unittest
import sys
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import audit_operation_inventory as inventory


class OperationInventoryTests(unittest.TestCase):
    def test_exact_registered_source_inventory_is_internally_consistent(self) -> None:
        report = inventory.audit()
        self.assertEqual(report["validation_errors"], [])
        self.assertEqual(
            report["counts"],
            {
                "registered_temporal_activities": 73,
                "legacy_seo_phase_keys": 22,
                "candidate_operation_decisions": 52,
                "generic_pure_operation_implementations": 5,
            },
        )
        self.assertEqual(len(report["remaining_unknown_contracts"]), 52)
        self.assertFalse(any(phase["normative_operation"] for phase in report["legacy_phases"]))

    def test_activity_routes_are_source_references_not_registered_reachability_claims(self) -> None:
        items, errors = inventory.activity_registry()
        self.assertFalse(errors)
        self.assertEqual(sum("canonical_v1" in i["referenced_by"] for i in items), 55)
        self.assertEqual(sum("source_only_v2" in i["referenced_by"] for i in items), 5)
        self.assertEqual(sum(i["referenced_by"] == ["test_workflow"] for i in items), 2)
        self.assertTrue(
            all(i["effect_contract"].startswith("TBD:") for i in items)
        )

    def test_dropped_activity_declaration_is_detected(self) -> None:
        original = inventory.REGISTRY.read_text(encoding="utf-8")
        altered = original.replace("#[activity]", "#[non_activity]", 1)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "registry.rs"
            path.write_text(altered, encoding="utf-8")
            with patch.object(inventory, "REGISTRY", path):
                _, errors = inventory.activity_registry()
        self.assertTrue(any("Activity count drift" in x for x in errors), errors)
        self.assertTrue(any("Activity family ledger mismatch" in x for x in errors), errors)

    def test_duplicate_legacy_stable_id_detected(self) -> None:
        original = inventory.RECIPE.read_text(encoding="utf-8")
        altered = original.replace(
            '"serp_ingest" => Self::SerpIngest',
            '"load_verified_support_bundle" => Self::SerpIngest',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "recipe.rs"
            path.write_text(altered, encoding="utf-8")
            with patch.object(inventory, "RECIPE", path):
                _, errors = inventory.legacy_phases()
        self.assertTrue(any("duplicate SeoPhaseKey stable ID" in x for x in errors), errors)
        self.assertTrue(any("phase_label/stable-ID" in x for x in errors), errors)

    def test_candidate_drift_and_unresolved_source_are_detected(self) -> None:
        original = inventory.MATRIX.read_text(encoding="utf-8")
        altered = original.replace("| ACQ-01 |", "| ACQ-99 |", 1)
        altered = altered.replace("app/rust/crates/acquisition/src/source_relevance.rs",
                                  "app/rust/crates/acquisition/src/missing.rs", 1)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "matrix.md"
            path.write_text(altered, encoding="utf-8")
            with patch.object(inventory, "MATRIX", path):
                _, errors = inventory.candidates()
        self.assertTrue(any("candidate identity drift" in x for x in errors), errors)
        self.assertTrue(any("invalid source path" in x for x in errors), errors)


if __name__ == "__main__":
    unittest.main()
