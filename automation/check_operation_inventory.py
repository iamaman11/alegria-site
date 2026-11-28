#!/usr/bin/env python3
"""Regression tests for the source-driven Issue #94 audit (no Rust build)."""

from __future__ import annotations

import json
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

    def test_all_52_candidates_have_conservative_machine_readable_readiness(self) -> None:
        report = inventory.audit()
        self.assertEqual(report["validation_errors"], [])
        self.assertEqual(report["candidate_readiness_summary"], {
            "all_candidates": 52,
            "provisional_decisions_by_type": {"INTERNAL": 11, "KEEP": 29, "SPLIT": 12},
            "provisional_public_candidate_rows": 41,
            "provisional_internal_helper_rows": 11,
            "pure_subset_typed_catalog_runner_source_evidenced": 5,
            "acquisition_owner_typed_only_source_evidenced": 2,
            "fully_accepted_independent_candidates": 0,
            "candidate_physical_e2e_attested": 0,
            "candidate_acceptance_pending": 52,
        })
        rows = report["candidate_decisions"]
        self.assertEqual(len(rows), 52)
        self.assertEqual(len({row["id"] for row in rows}), 52)
        self.assertEqual({row["id"] for row in rows}, inventory.BASELINE_CANDIDATE_IDS)
        self.assertEqual(len(json.loads(json.dumps(report))["candidate_decisions"]), 52)

        runnable_subset_ids = {"EX-01", "EX-02", "EX-03", "EX-05", "EX-06"}
        owner_only_ids = {"ACQ-01", "ACQ-02"}
        for row in rows:
            evidence = row["readiness_evidence"]
            self.assertTrue(evidence["source_paths_checked"], row["id"])
            self.assertEqual(evidence["decision_status"], "provisional_source_audit")
            if row["decision"] == "INTERNAL":
                self.assertEqual(
                    evidence["disposition"], "helper_not_proposed_as_independent_operation"
                )
            self.assertEqual(evidence["independent_physical_e2e"], "not_attributed_to_candidate")
            self.assertEqual(evidence["external_effect_admission"], "not_verified_for_complete_candidate")
            self.assertEqual(evidence["complete_candidate_acceptance"], "unresolved")
            if row["id"] in runnable_subset_ids:
                self.assertEqual(evidence["typed_domain_contract"], "pure_input_output_failure_source_verified")
                self.assertEqual(evidence["installed_site_architect_contract"], "pure_catalog_registration_source_verified")
                self.assertEqual(evidence["single_operation_executor"], "pure_runner_registration_source_verified")
                self.assertEqual(evidence["typed_owner_contract"]["failure_type"], "Infallible")
                self.assertEqual(len(evidence["source_evidence_paths"]), 3)
            elif row["id"] in owner_only_ids:
                self.assertEqual(evidence["typed_domain_contract"], "owner_input_output_failure_source_verified")
                self.assertEqual(evidence["installed_site_architect_contract"], "not_verified")
                self.assertEqual(evidence["single_operation_executor"], "not_verified")
                self.assertEqual(evidence["typed_owner_contract"]["failure_type"], "Infallible")
            else:
                self.assertEqual(evidence["typed_domain_contract"], "not_verified")
                self.assertEqual(evidence["single_operation_executor"], "not_verified")
                self.assertIsNone(evidence["typed_owner_contract"])
        self.assertEqual(
            next(row for row in rows if row["id"] == "EX-03")["readiness_evidence"]["pure_subset_scope"],
            "known_partial_split",
        )

    def test_pure_source_registration_drift_fails_closed(self) -> None:
        catalog = inventory.PURE_CATALOG.read_text(encoding="utf-8")
        runner = inventory.PURE_RUNNER.read_text(encoding="utf-8")
        self.assertIn("LayerRoute::contract().reference", catalog)
        self.assertIn("register_pure::<LayerRoute>()", runner)
        with tempfile.TemporaryDirectory() as tmp:
            catalog_path = Path(tmp) / "catalog.rs"
            catalog_path.write_text(
                catalog.replace("LayerRoute::contract().reference", "AbsentLayerRoute::contract().reference", 1),
                encoding="utf-8",
            )
            runner_path = Path(tmp) / "runner.rs"
            runner_path.write_text(
                runner.replace("register_pure::<LayerRoute>()", "register_pure::<AbsentLayerRoute>()", 1),
                encoding="utf-8",
            )
            with patch.object(inventory, "PURE_CATALOG", catalog_path), patch.object(inventory, "PURE_RUNNER", runner_path):
                _, errors = inventory.audited_pure_contracts()
        self.assertTrue(any("catalog registration" in error for error in errors), errors)
        self.assertTrue(any("pure runner registration" in error for error in errors), errors)

    def test_acquisition_typed_owner_drift_fails_closed(self) -> None:
        original = inventory.ACQUISITION_OWNER_OPS.read_text(encoding="utf-8")
        self.assertIn("impl Operation for ClassifySourcePage", original)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "operations.rs"
            path.write_text(
                original.replace("impl Operation for ClassifySourcePage", "impl Operation for RemovedSourcePage", 1),
                encoding="utf-8",
            )
            with patch.object(inventory, "ACQUISITION_OWNER_OPS", path):
                _, errors = inventory.audited_acquisition_owner_contracts()
        self.assertTrue(any("ClassifySourcePage lacks typed Operation contract" in error for error in errors), errors)

    def test_any_provisional_decision_change_fails_closed(self) -> None:
        original = inventory.MATRIX.read_text(encoding="utf-8")
        self.assertIn("| ACQ-03 | INTERNAL |", original)
        altered = original.replace("| ACQ-03 | INTERNAL |", "| ACQ-03 | KEEP |", 1)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "matrix.md"
            path.write_text(altered, encoding="utf-8")
            with patch.object(inventory, "MATRIX", path):
                _, errors = inventory.candidates()
        self.assertTrue(any("decision ledger changed" in error for error in errors), errors)

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
        # Match the canonical matrix row, not an earlier status/progress
        # table that may mention the same candidate by name.
        altered = original.replace("| ACQ-01 | KEEP |", "| ACQ-99 | KEEP |", 1)
        self.assertNotEqual(altered, original)
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
