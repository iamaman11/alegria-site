#!/usr/bin/env python3
"""Regression tests for Temporal test SDK and production Outbox isolation."""

from __future__ import annotations

import re
import unittest

from check_rust_adapter_boundaries import (
    DELIVERY_MANIFEST,
    DELIVERY_FORBIDDEN_LOCAL_OWNERS,
    MANIFEST_DEP_EXCEPTIONS,
    OUTBOX_MANIFEST,
    PG_E2E_DIR,
    PG_E2E_SRC,
    PG_E2E_TEMPORAL_SDKS,
    PG_RUNTIME_FORBIDDEN_LOCAL_OWNERS,
    PG_RUNTIME_MANIFEST,
    RUST_ROOT,
    VOYAGE_MANIFEST,
    pg_e2e_direct_temporal_sdk_allowed,
    provider_delivery_closure_failures,
    pg_runtime_closure_failures,
    workspace_dependency_closure,
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


class PgRuntimeOwnerBoundaryTests(unittest.TestCase):
    def test_pg_repository_is_a_real_provider_free_cargo_owner(self) -> None:
        closure, direct = workspace_dependency_closure(PG_RUNTIME_MANIFEST)
        self.assertEqual(pg_runtime_closure_failures(closure, direct), [])
        self.assertTrue({"pg_runtime_ports", "postgres_store", "seo_ports"} <= closure)

    def test_any_provider_or_temporal_reconnection_fails_closed(self) -> None:
        closure, direct = workspace_dependency_closure(PG_RUNTIME_MANIFEST)
        for forbidden in PG_RUNTIME_FORBIDDEN_LOCAL_OWNERS:
            with self.subTest(owner=forbidden):
                self.assertIn(forbidden, " ".join(
                    pg_runtime_closure_failures(closure | {forbidden}, direct)
                ))


class OutboxProviderDeliveryBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.delivery_closure, self.delivery_direct = workspace_dependency_closure(DELIVERY_MANIFEST)
        self.voyage_closure, _ = workspace_dependency_closure(VOYAGE_MANIFEST)
        self.outbox_closure, self.outbox_direct = workspace_dependency_closure(OUTBOX_MANIFEST)

    def failures(self, *, outbox_closure=None, outbox_direct=None,
                 delivery_closure=None, delivery_direct=None, voyage_closure=None):
        return provider_delivery_closure_failures(
            self.outbox_closure if outbox_closure is None else outbox_closure,
            self.outbox_direct if outbox_direct is None else outbox_direct,
            self.delivery_closure if delivery_closure is None else delivery_closure,
            self.delivery_direct if delivery_direct is None else delivery_direct,
            self.voyage_closure if voyage_closure is None else voyage_closure,
        )

    def test_production_outbox_physical_cargo_graph_is_narrow(self) -> None:
        self.assertEqual(self.failures(), [])
        self.assertTrue({"projection_delivery", "postgres_store", "voyage_api"} <= self.outbox_closure)
        self.assertEqual(DELIVERY_FORBIDDEN_LOCAL_OWNERS & self.outbox_closure, set())

    def test_all_heavy_owners_fail_closed_at_every_delivery_layer(self) -> None:
        for owner in DELIVERY_FORBIDDEN_LOCAL_OWNERS:
            for layer in ("outbox_closure", "delivery_closure", "voyage_closure"):
                with self.subTest(owner=owner, layer=layer):
                    closure = getattr(self, layer) | {owner}
                    self.assertIn(owner, " ".join(self.failures(**{layer: closure})))

    def test_forwarding_shim_and_missing_provider_edge_are_rejected(self) -> None:
        self.assertIn("must directly depend", " ".join(self.failures(outbox_direct=set())))
        self.assertIn("must directly depend", " ".join(self.failures(delivery_direct=set())))


if __name__ == "__main__":
    unittest.main()
