#!/usr/bin/env python3
"""Mutation tests for the Issue #94 PG Truth review effect boundary audit."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_v3_effect_owner_boundary as boundary


class PgTruthReviewOwnerBoundaryTests(unittest.TestCase):
    def test_current_source_boundary_is_intact(self) -> None:
        failures, _notes = boundary.audit()
        self.assertEqual([], failures)

    def test_both_direct_cli_arms_are_fail_closed_before_io(self) -> None:
        original = boundary.LEGACY_CLI.read_text(encoding="utf-8")
        self.assertEqual([], boundary.check_disabled_cli(original))
        for variant in ("SeoRun", "CrawlPendingSources"):
            altered = original.replace(
                f"return Err(UnsupportedDirectSiteCommand::{variant}.into());",
                "return Ok(());",
                1,
            )
            self.assertIn(
                f"direct CLI {variant} must reject before any I/O",
                boundary.check_disabled_cli(altered),
            )

    def test_disabled_cli_retains_operator_diagnostic(self) -> None:
        original = boundary.CLI_TYPES.read_text(encoding="utf-8")
        self.assertEqual([], boundary.check_disabled_cli_diagnostic(original))
        altered = original.replace(
            "CLI_DIRECT_SITE_COMMAND_DISABLED:", "CLI_OLD_COMMAND:", 1
        )
        self.assertTrue(boundary.check_disabled_cli_diagnostic(altered))

    def test_v1_truth_write_cannot_shed_atomic_review_reconcile(self) -> None:
        original = boundary.TRUTH_WRITE.read_text(encoding="utf-8")
        self.assertEqual([], boundary.check_atomic_v1_truth_write(original))
        altered = original.replace(
            "reconcile_truth_candidate_reviews_in_tx(&mut tx, context_key, run_id)",
            "reconcile_truth_candidate_reviews(&pool, context_key, run_id)",
            1,
        )
        self.assertTrue(boundary.check_atomic_v1_truth_write(altered))

    def test_canonical_reconcile_cannot_verify_truth_or_remove_snapshot_fence(self) -> None:
        original = boundary.RECONCILE_MIGRATION.read_text(encoding="utf-8")
        self.assertEqual([], boundary.check_truth_reconcile_sql(original))
        marker = "RETURN created_count;"
        self.assertIn(marker, boundary.reconcile_function(original))
        dangerous = original.replace(
            marker,
            "UPDATE verified.rule_instances SET epistemic_status = 'verified';\n    " + marker,
            1,
        )
        self.assertTrue(any(
            "cannot UPDATE canonical Truth" in violation
            for violation in boundary.check_truth_reconcile_sql(dangerous)
        ))
        no_fence = original.replace("ON CONFLICT (task_key) DO NOTHING", "ON CONFLICT DO UPDATE", 1)
        self.assertTrue(any(
            "lost SQL fence: ON CONFLICT (task_key) DO NOTHING" in violation
            for violation in boundary.check_truth_reconcile_sql(no_fence)
        ))

    def test_canonical_invoker_and_migration_schema_parity_are_enforced(self) -> None:
        migration = boundary.RECONCILE_MIGRATION.read_text(encoding="utf-8")
        schema = boundary.RECONCILE_SCHEMA.read_text(encoding="utf-8")
        self.assertEqual(boundary.reconcile_function(migration), boundary.reconcile_function(schema))
        self.assertEqual([], boundary.check_truth_reconcile_sql(schema))
        self.assertTrue(boundary.check_truth_reconcile_sql(
            boundary.reconcile_function(migration).replace("SECURITY INVOKER", "SECURITY DEFINER", 1)
        ))
        caller = boundary.PG_RECONCILE.read_text(encoding="utf-8")
        self.assertEqual([], boundary.check_truth_reconcile_wiring(caller))
        self.assertTrue(boundary.check_truth_reconcile_wiring(
            caller.replace("SELECT site.reconcile_truth_candidate_reviews_v1($1, $2)",
                           "SELECT site.untrusted_reconcile($1, $2)", 1)
        ))

    def test_pg_owner_dependency_closure_rejects_transitive_mixed_facade(self) -> None:
        self.assertEqual([], boundary.check_pg_owner_dependencies(boundary.PG_MANIFEST))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "owner").mkdir()
            (root / "pg").mkdir()
            (root / "mixed").mkdir()
            (root / "owner/Cargo.toml").write_text(
                '[package]\nname = "pg_worker"\nversion = "0.1.0"\n'
                '[dependencies]\npostgres_store = { path = "../pg" }\n',
                encoding="utf-8",
            )
            (root / "pg/Cargo.toml").write_text(
                '[package]\nname = "postgres_store"\nversion = "0.1.0"\n'
                '[dependencies]\nruntime_composition = { path = "../mixed" }\n',
                encoding="utf-8",
            )
            (root / "mixed/Cargo.toml").write_text(
                '[package]\nname = "runtime_composition"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            violations = boundary.check_pg_owner_dependencies(root / "owner/Cargo.toml")
            self.assertTrue(any("runtime_composition" in violation for violation in violations))


if __name__ == "__main__":
    unittest.main()
