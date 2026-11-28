#!/usr/bin/env python3
"""Mutation-test the generic DAG causal freshness gate (not a Rust substitute).

The mandatory architecture job runs the real Rust workspace and this source
guard. These mutations ensure neither the explicit graph hazards nor their
Rust regression tests silently disappear during the next owner extraction.
"""

from __future__ import annotations

import unittest

from check_reliability_contracts import (
    CI_WORKFLOW, OPERATION_PROTOCOL_OWNER, OPERATION_PROTOCOL_TESTS,
    POSTGRES_RECONCILE, SEMANTIC_EXTRACTION_OPERATIONS,
    SEMANTIC_OPERATION_RUNNER_TESTS, operation_freshness_contract_failures,
)


SOURCES = (
    OPERATION_PROTOCOL_OWNER, OPERATION_PROTOCOL_TESTS,
    SEMANTIC_EXTRACTION_OPERATIONS, POSTGRES_RECONCILE,
    SEMANTIC_OPERATION_RUNNER_TESTS, CI_WORKFLOW,
)


class CausalFreshnessGuardTests(unittest.TestCase):
    def sources(self) -> list[str]:
        return [path.read_text(encoding="utf-8") for path in SOURCES]

    def test_current_owner_and_real_rust_tests_are_guarded(self) -> None:
        self.assertEqual(operation_freshness_contract_failures(*self.sources()), [])

    def test_critical_causal_contract_mutations_fail_closed(self) -> None:
        sources = self.sources()
        mutations = (
            (0, "pub const PLAN_SCHEMA_REVISION: u16 = 2", "pub const PLAN_SCHEMA_REVISION: u16 = 1"),
            (0, "pub invalidates: BTreeSet<ArtifactTypeId>", "// invalidations removed"),
            (0, "serde_json::to_vec(self)", "serde_json::to_vec(&self.reference)"),
            (0, "contract.invalidates", "contract.effects"),
            (0, "StaleArtifact", "IgnoredStaleness"),
            (0, "UnorderedArtifactHazard", "IgnoredUnorderedReads"),
            (0, "UnorderedInvalidations", "IgnoredConcurrentWrites"),
            (0, "self.verify_freshness_dependencies(", "self.skip_freshness_validation("),
            (0, "return Err(PlanError::StaleArtifact {", "return Err(PlanError::Cycle /* stale ignored */ {"),
            (0, "return Err(PlanError::UnorderedInvalidations {", "return Err(PlanError::Cycle /* concurrent writes ignored */ {"),
            (1, "PlanError::StaleArtifact", "PlanError::Cycle"),
            (1, "PlanError::UnorderedArtifactHazard", "PlanError::Cycle"),
            (1, "PlanError::UnorderedInvalidations", "PlanError::Cycle"),
            (2, "invalidates: BTreeSet::new()", "// missing owner invalidations"),
            (3, "invalidates: BTreeSet::new()", "// missing PG invalidations"),
            (4, "assert!(contract.invalidates.is_empty())", "assert!(contract.effects.is_empty())"),
            (5, "python3 automation/test_operation_freshness_contract.py", "# freshness tests skipped"),
        )
        for index, old, new in mutations:
            with self.subTest(path=SOURCES[index].name, removed=old):
                self.assertIn(old, sources[index])
                changed = sources.copy()
                changed[index] = changed[index].replace(old, new)
                self.assertTrue(
                    operation_freshness_contract_failures(*changed),
                    f"mutation went unnoticed in {SOURCES[index].name}: {old}",
                )


if __name__ == "__main__":
    unittest.main()
