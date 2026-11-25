#!/usr/bin/env python3
"""Regression tests for the isolated PG V3 Temporal History JSON boundary."""

from __future__ import annotations

import unittest

from check_json_boundary_policy import (
    EXTRACTED_OWNER_JSON_BOUNDARIES,
    FORBIDDEN_PATTERNS,
    PG_V3_E2E_HISTORY_TEST_BOUNDARIES,
    ROOT,
    is_allowed_json_boundary,
)


class PgHistoryJsonBoundaryTests(unittest.TestCase):
    def test_extracted_owner_json_grants_are_exact_files(self) -> None:
        self.assertEqual(EXTRACTED_OWNER_JSON_BOUNDARIES, {
            "app/rust/crates/integration_outbox_e2e/src/tests/outbox_delivery_tests.rs",
            "app/rust/crates/projection_delivery/src/projection_materialize_adapter.rs",
            "app/rust/crates/voyage_api/src/lib.rs",
        })
        for relative in EXTRACTED_OWNER_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))

        for relative in (
            "app/rust/crates/integration_outbox_e2e/src/lib.rs",
            "app/rust/crates/integration_outbox_e2e/src/tests/new_json_test.rs",
            "app/rust/crates/projection_delivery/src/new_provider.rs",
            "app/rust/crates/voyage_api/src/operation.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_only_two_actual_temporal_history_test_files_are_exempt(self) -> None:
        self.assertEqual(PG_V3_E2E_HISTORY_TEST_BOUNDARIES, {
            "app/rust/crates/integration_pg_v3_e2e/src/lib.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_temporal_cross_queue_tests.rs",
        })
        for relative in PG_V3_E2E_HISTORY_TEST_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))

    def test_new_pg_crate_files_do_not_inherit_test_exception(self) -> None:
        for relative in (
            "app/rust/crates/integration_pg_v3_e2e/src/containers.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/new_operation.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/bin/production.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/unknown_json_test.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_production_owner_code_remains_deny_by_default(self) -> None:
        for relative in (
            "app/rust/crates/postgres_store/src/truth_review_effect_v3.rs",
            "app/rust/crates/operation_protocol/src/lib.rs",
            "app/rust/services/temporal_pg_effects/src/lib.rs",
            "app/rust/crates/seo_domain/src/domain.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_actual_history_decoders_are_still_covered_by_pattern(self) -> None:
        self.assertRegex("serde_json::from_slice(&history)", FORBIDDEN_PATTERNS["serde_json::from_slice"])
        self.assertRegex("serde_json::from_str::<History>(&history)", FORBIDDEN_PATTERNS["serde_json::from_str"])


if __name__ == "__main__":
    unittest.main()
