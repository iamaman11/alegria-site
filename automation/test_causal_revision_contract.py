#!/usr/bin/env python3
"""Mutation tests proving the required source fence rejects unsafe revisions.

The source-level gate supplements, but never replaces, real PostgreSQL
concurrent writer/dispatcher and provider E2E tests.
"""

from __future__ import annotations

import unittest

from check_reliability_contracts import (
    CANONICAL_VERIFIED_WRITER, GENERIC_OUTBOX_WRITER, POSTGRES_OUTBOX,
    REVISION_MIGRATION, SCHEMA, SQLX_VERIFIED_WRITER,
    causal_revision_contract_failures,
)


SOURCES = (
    SCHEMA, REVISION_MIGRATION, POSTGRES_OUTBOX,
    GENERIC_OUTBOX_WRITER, CANONICAL_VERIFIED_WRITER, SQLX_VERIFIED_WRITER,
)


class CausalRevisionSourceGuardTests(unittest.TestCase):
    def test_current_repository_satisfies_transactional_causal_contract(self) -> None:
        source = [path.read_text(encoding="utf-8") for path in SOURCES]
        self.assertEqual(causal_revision_contract_failures(*source), [])

    def test_invariants_reject_migration_ordering_and_writer_bypasses(self) -> None:
        sources = [path.read_text(encoding="utf-8") for path in SOURCES]
        mutations = (
            (0, "CREATE TABLE IF NOT EXISTS system.verified_rule_projection_heads",
             "CREATE TABLE IF NOT EXISTS system.unsafe_projection_heads"),
            (1, "OR (projection_revision IS NOT NULL AND projection_revision > 0)) NOT VALID",
             "OR projection_revision > 0) NOT VALID"),
            (2, "ORDER BY projection_revision DESC LIMIT 1",
             "ORDER BY created_at DESC, event_id DESC LIMIT 1"),
            (2, "current_revision=system.verified_rule_projection_heads.current_revision + 1",
             "current_revision=1"),
            (2, "projection_revision IS NULL", "projection_revision IS NOT NULL"),
            (3, "insert_verified_rule_outbox_in_tx(", "unsafe_outbox_insert("),
            (4, "sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(tx, &events)",
             "unsafe_standalone_outbox_emit(&events)"),
            (5, "lock_and_reject_projected_legacy_truth_write_in_tx(",
             "lock_and_ensure_verified_rules_writable_in_tx("),
        )
        for source_index, original, replacement in mutations:
            with self.subTest(source=SOURCES[source_index].name, removed=original):
                self.assertIn(original, sources[source_index])
                changed = sources.copy()
                changed[source_index] = changed[source_index].replace(original, replacement)
                self.assertTrue(causal_revision_contract_failures(*changed))


if __name__ == "__main__":
    unittest.main()
