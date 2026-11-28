#!/usr/bin/env python3
"""Negative tests for source-derived #94 planning matrix drift detection."""

from __future__ import annotations

import copy
import json
import unittest

import audit_operation_inventory as ledger
from check_site_architect_action_matrix import MATRIX, validate


class ActionMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source_rows, source_errors = ledger.candidates()
        cls.phases, phase_errors = ledger.legacy_phases()
        if source_errors or phase_errors:
            raise AssertionError(source_errors + phase_errors)
        cls.original = json.loads(MATRIX.read_text(encoding="utf-8"))

    def check(self, manifest: dict) -> list[str]:
        return validate(manifest, self.source_rows, self.phases)

    def test_canonical_52_by_source_and_22_legacy_phases(self) -> None:
        self.assertEqual([], self.check(self.original))
        self.assertEqual(52, len(self.original["candidates"]))
        self.assertEqual(22, len(self.original["legacy_phase_mapping"]))

    def test_duplicate_candidate_identity_rejected(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["candidates"][1]["id"] = manifest["candidates"][0]["id"]
        self.assertIn("52 unique candidate IDs", " ".join(self.check(manifest)))

    def test_source_decision_drift_rejected(self) -> None:
        manifest = copy.deepcopy(self.original)
        next(row for row in manifest["candidates"] if row["id"] == "ACQ-01")["decision"] = "SPLIT"
        self.assertIn("classification differs", " ".join(self.check(manifest)))

    def test_internal_cannot_become_user_step(self) -> None:
        manifest = copy.deepcopy(self.original)
        next(row for row in manifest["candidates"] if row["id"] == "ACQ-03")["operations"] = [
            "acquisition.internal_gate@1"
        ]
        self.assertIn("INTERNAL cannot expose", " ".join(self.check(manifest)))

    def test_split_must_keep_independent_contracts(self) -> None:
        manifest = copy.deepcopy(self.original)
        next(row for row in manifest["candidates"] if row["id"] == "PL-01")["operations"] = [
            "dataforseo.google_organic.search@1"
        ]
        self.assertIn("SPLIT must identify", " ".join(self.check(manifest)))

    def test_cross_row_alias_must_be_declared(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["shared_operation_ids"] = {}
        self.assertIn("shared action aliases drift", " ".join(self.check(manifest)))

    def test_invented_source_declaration_rejected(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["declared_rust_contracts"]["cms.review.request@1"] = (
            "app/rust/crates/acquisition/src/operations.rs"
        )
        self.assertIn("no literal v1 contract ID", " ".join(self.check(manifest)))

    def test_legacy_phase_disappearance_rejected(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["legacy_phase_mapping"].pop("raw_knowledge_ingestion")
        self.assertIn("22 legacy phases not fully reconciled", " ".join(self.check(manifest)))

    def test_reviewed_legacy_macro_cannot_be_collapsed_into_one_action(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["legacy_phase_mapping"]["raw_knowledge_ingestion"] = ["ACQ-07"]
        self.assertIn("source-reviewed legacy phase mapping drift", " ".join(self.check(manifest)))

    def test_reviewed_support_refresh_reuses_exact_action(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["legacy_phase_dispositions"]["load_verified_support_bundle.refresh"]["action"] = (
            "truth.verified.commit@1"
        )
        self.assertIn("source-reviewed disposition kind/action drift", " ".join(self.check(manifest)))

    def test_reviewed_legacy_source_evidence_cannot_be_invented(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["legacy_phase_dispositions"]["raw_knowledge_ingestion"]["evidence"] = [
            "docs/source-not-present.rs"
        ]
        self.assertIn("invalid source evidence path", " ".join(self.check(manifest)))

    def test_reviewed_legacy_phase_cannot_remain_open(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["open_source_gaps"].append("Decide raw_knowledge_ingestion.")
        self.assertIn("reviewed legacy phase cannot remain", " ".join(self.check(manifest)))

    def test_external_inventory_gap_cannot_disappear_by_relabeling(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["open_source_gaps"] = []
        self.assertIn("pending external provider inventory gap disappeared", " ".join(self.check(manifest)))

    def test_proposed_contract_cannot_claim_runtime_readiness(self) -> None:
        manifest = copy.deepcopy(self.original)
        manifest["candidate_readiness"] = "ready"
        self.assertIn("must not claim deployed readiness", " ".join(self.check(manifest)))


if __name__ == "__main__":
    unittest.main()
