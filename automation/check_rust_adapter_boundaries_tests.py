#!/usr/bin/env python3
"""Regression tests for the exact-source E2E Temporal SDK exception."""

from __future__ import annotations

import re
import unittest

from check_rust_adapter_boundaries import (
    MANIFEST_DEP_EXCEPTIONS,
    PG_E2E_DIR,
    PG_E2E_SRC,
    PG_E2E_TEMPORAL_SDKS,
    RUST_ROOT,
    pg_e2e_direct_temporal_sdk_allowed,
)


class PgE2eSdkBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.test_source = PG_E2E_SRC / "tests" / "future_pg_effect_test.rs"

    def test_only_three_native_temporal_sdks_are_allowed(self) -> None:
        self.assertEqual(PG_E2E_TEMPORAL_SDKS, {
            "temporalio-client", "temporalio-common", "temporalio-sdk-core",
        })
        for sdk in PG_E2E_TEMPORAL_SDKS:
            module = sdk.replace("-", "_")
            with self.subTest(sdk=sdk):
                self.assertTrue(pg_e2e_direct_temporal_sdk_allowed(
                    self.test_source, re.compile(rf"\buse\s+{module}::")))
                self.assertTrue(pg_e2e_direct_temporal_sdk_allowed(
                    self.test_source, re.compile(rf"\b{module}::")))

    def test_pg_tests_still_reject_provider_and_unapproved_sdks(self) -> None:
        for module in (
            "neo4rs", "qdrant_client", "reqwest", "temporalio_sdk",
            "temporalio_macros", "tonic", "hyper", "sqlx",
        ):
            with self.subTest(module=module):
                self.assertFalse(pg_e2e_direct_temporal_sdk_allowed(
                    self.test_source, re.compile(rf"\buse\s+{module}::")))

    def test_production_and_other_e2e_packages_get_no_exception(self) -> None:
        for path in (
            RUST_ROOT / "crates" / "postgres_store" / "src" / "future_effect.rs",
            RUST_ROOT / "crates" / "operation_protocol" / "src" / "lib.rs",
            RUST_ROOT / "crates" / "integration_harness" / "src" / "lib.rs",
            PG_E2E_DIR / "build.rs",
        ):
            for sdk in PG_E2E_TEMPORAL_SDKS:
                with self.subTest(path=path, sdk=sdk):
                    self.assertFalse(pg_e2e_direct_temporal_sdk_allowed(
                        path, re.compile(rf"\b{sdk.replace('-', '_')}::")))

    def test_exception_manifest_is_exact(self) -> None:
        self.assertEqual(
            MANIFEST_DEP_EXCEPTIONS[(PG_E2E_DIR / "Cargo.toml").resolve()],
            PG_E2E_TEMPORAL_SDKS,
        )


if __name__ == "__main__":
    unittest.main()
