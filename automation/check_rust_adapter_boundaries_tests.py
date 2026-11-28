#!/usr/bin/env python3
"""Regression tests for Temporal test SDK and production Outbox isolation."""

from __future__ import annotations

import re
import tomllib
import unittest

from check_rust_adapter_boundaries import (
    ACQUISITION_PURE_OWNER_MANIFEST,
    ACQUISITION_PURE_OWNER_SOURCE_FILES,
    ACQUISITION_PURE_OWNER_TEMPORAL_SDKS,
    DATAFORSEO_PAID_PROVIDER_SOURCE_FILES,
    DELIVERY_MANIFEST,
    DELIVERY_FORBIDDEN_LOCAL_OWNERS,
    MANIFEST_DEP_EXCEPTIONS,
    PURE_OWNER_RUNTIME_SOURCE_FILES,
    SEMANTIC_HISTORY_SOURCE,
    SEMANTIC_PURE_OWNER_MANIFEST,
    SEO_STEPS_PURE_OWNER_MANIFEST,
    TRUTH_PURE_OWNER_MANIFEST,
    WEB_FETCH_OWNER_MANIFEST,
    WEB_FETCH_OWNER_TRANSPORT_SOURCE_FILES,
    OUTBOX_MANIFEST,
    PG_E2E_DIR,
    PG_E2E_SRC,
    PG_E2E_TEMPORAL_SDKS,
    PG_RUNTIME_FORBIDDEN_LOCAL_OWNERS,
    PG_RUNTIME_MANIFEST,
    QDRANT_EFFECTS_MANIFEST,
    QDRANT_EFFECTS_RUNTIME_SDKS,
    QDRANT_EFFECTS_SDK_SOURCE_FILES,
    QDRANT_TEMPORAL_E2E_DIR,
    QDRANT_TEMPORAL_E2E_SRC,
    QDRANT_TEMPORAL_E2E_SDKS,
    RUST_ROOT,
    VOYAGE_MANIFEST,
    acquisition_pure_owner_direct_sdk_allowed,
    dataforseo_owner_direct_sdk_allowed,
    new_site_architect_owner_sdk_allowed,
    pg_e2e_direct_temporal_sdk_allowed,
    qdrant_e2e_direct_temporal_sdk_allowed,
    qdrant_runtime_direct_temporal_sdk_allowed,
    provider_delivery_closure_failures,
    pg_runtime_closure_failures,
    workspace_dependency_closure,
)


class NativeAcquisitionPureOwnerBoundaryTests(unittest.TestCase):
    def test_exact_native_sdk_surface_and_no_provider_escape(self) -> None:
        self.assertEqual({path.name for path in ACQUISITION_PURE_OWNER_SOURCE_FILES}, {"main.rs"})
        self.assertEqual(MANIFEST_DEP_EXCEPTIONS[ACQUISITION_PURE_OWNER_MANIFEST],
                         ACQUISITION_PURE_OWNER_TEMPORAL_SDKS)
        for source in ACQUISITION_PURE_OWNER_SOURCE_FILES:
            for sdk in ACQUISITION_PURE_OWNER_TEMPORAL_SDKS:
                self.assertTrue(acquisition_pure_owner_direct_sdk_allowed(
                    source, re.compile(rf"\buse\s+{sdk.replace('-', '_')}::")))
            for forbidden in ("reqwest", "sqlx", "neo4rs", "qdrant_client", "temporalio_sdk_core"):
                self.assertFalse(acquisition_pure_owner_direct_sdk_allowed(
                    source, re.compile(rf"\buse\s+{forbidden}::")))

    def test_other_owner_files_and_domain_are_denied(self) -> None:
        for source in (
            ACQUISITION_PURE_OWNER_MANIFEST.parent / "src" / "future_owner.rs",
            ACQUISITION_PURE_OWNER_MANIFEST.parent / "src" / "nested" / "main.rs",
            RUST_ROOT / "crates" / "acquisition" / "src" / "operations.rs",
        ):
            self.assertFalse(acquisition_pure_owner_direct_sdk_allowed(
                source, re.compile(r"\buse\s+temporalio_sdk::")))


class SiteArchitectNewOwnerSdkBoundaryTests(unittest.TestCase):
    def test_pure_workers_get_only_exact_native_temporal_sdk_access(self) -> None:
        manifests = (SEO_STEPS_PURE_OWNER_MANIFEST, TRUTH_PURE_OWNER_MANIFEST,
                     SEMANTIC_PURE_OWNER_MANIFEST)
        self.assertEqual(
            PURE_OWNER_RUNTIME_SOURCE_FILES,
            {manifest.parent / "src" / "main.rs" for manifest in manifests},
        )
        for manifest in manifests:
            self.assertEqual(MANIFEST_DEP_EXCEPTIONS[manifest],
                             ACQUISITION_PURE_OWNER_TEMPORAL_SDKS)
        for source in PURE_OWNER_RUNTIME_SOURCE_FILES:
            self.assertEqual(source.name, "main.rs")
            for sdk in ACQUISITION_PURE_OWNER_TEMPORAL_SDKS:
                self.assertTrue(new_site_architect_owner_sdk_allowed(
                    source, re.compile(rf"\buse\s+{sdk.replace('-', '_')}::")))
            self.assertFalse(new_site_architect_owner_sdk_allowed(
                source, re.compile(r"\buse\s+reqwest::")))
        for path in (
            SEO_STEPS_PURE_OWNER_MANIFEST.parent / "src" / "other.rs",
            SEMANTIC_PURE_OWNER_MANIFEST.parent / "src" / "other.rs",
            RUST_ROOT / "crates" / "seo_steps" / "src" / "owner_pure_operations.rs",
        ):
            self.assertFalse(new_site_architect_owner_sdk_allowed(
                path, re.compile(r"\btemporalio_sdk::")))

    def test_semantic_history_reads_only_through_temporal_client_and_protos(self) -> None:
        for sdk in ("temporalio_client", "temporalio_common"):
            self.assertTrue(new_site_architect_owner_sdk_allowed(
                SEMANTIC_HISTORY_SOURCE, re.compile(rf"\b{sdk}::")))
        for sdk in ("temporalio_sdk", "temporalio_macros", "reqwest", "sqlx"):
            self.assertFalse(new_site_architect_owner_sdk_allowed(
                SEMANTIC_HISTORY_SOURCE, re.compile(rf"\b{sdk}::")))

    def test_web_transport_and_paid_client_do_not_grant_sibling_sdk_access(self) -> None:
        self.assertEqual(MANIFEST_DEP_EXCEPTIONS[WEB_FETCH_OWNER_MANIFEST], {"reqwest"})
        self.assertEqual(len(WEB_FETCH_OWNER_TRANSPORT_SOURCE_FILES), 1)
        self.assertEqual(len(DATAFORSEO_PAID_PROVIDER_SOURCE_FILES), 1)
        for source in WEB_FETCH_OWNER_TRANSPORT_SOURCE_FILES:
            self.assertTrue(new_site_architect_owner_sdk_allowed(
                source, re.compile(r"\buse\s+reqwest::")))
            self.assertFalse(new_site_architect_owner_sdk_allowed(
                source, re.compile(r"\btemporalio_sdk::")))
        for source in DATAFORSEO_PAID_PROVIDER_SOURCE_FILES:
            self.assertTrue(dataforseo_owner_direct_sdk_allowed(
                source, re.compile(r"\buse\s+reqwest::")))
            self.assertFalse(dataforseo_owner_direct_sdk_allowed(
                source, re.compile(r"\btemporalio_client::")))
        self.assertFalse(new_site_architect_owner_sdk_allowed(
            WEB_FETCH_OWNER_MANIFEST.parent / "src" / "archive.rs",
            re.compile(r"\buse\s+reqwest::")))


class PgE2eSdkBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.test_source = PG_E2E_SRC / "tests" / "future_pg_effect_test.rs"

    def test_only_two_native_temporal_client_libraries_are_allowed(self) -> None:
        self.assertEqual(PG_E2E_TEMPORAL_SDKS, {
            "temporalio-client", "temporalio-common",
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
            "neo4rs", "qdrant_client", "reqwest", "temporalio_sdk", "temporalio_sdk_core",
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


class NativeQdrantTemporalSdkBoundaryTests(unittest.TestCase):
    def test_runtime_exception_is_exact_six_reviewed_files_and_four_sdks(self) -> None:
        self.assertEqual({path.name for path in QDRANT_EFFECTS_SDK_SOURCE_FILES},
                         {"main.rs", "operator.rs", "operator_v2.rs", "owner_v2.rs",
                          "operator_v3.rs", "owner_generic_v1.rs"})
        self.assertEqual(QDRANT_EFFECTS_RUNTIME_SDKS, {
            "temporalio-sdk", "temporalio-client",
            "temporalio-common", "temporalio-macros",
        })
        for path in QDRANT_EFFECTS_SDK_SOURCE_FILES:
            for sdk in QDRANT_EFFECTS_RUNTIME_SDKS:
                with self.subTest(path=path, sdk=sdk):
                    self.assertTrue(qdrant_runtime_direct_temporal_sdk_allowed(
                        path, re.compile(rf"\buse\s+{sdk.replace('-', '_')}::")))
            for forbidden in ("temporalio_sdk_core", "qdrant_client", "neo4rs", "reqwest"):
                with self.subTest(path=path, forbidden=forbidden):
                    self.assertFalse(qdrant_runtime_direct_temporal_sdk_allowed(
                        path, re.compile(rf"\buse\s+{forbidden}::")))
        self.assertEqual(MANIFEST_DEP_EXCEPTIONS[QDRANT_EFFECTS_MANIFEST.resolve()],
                         QDRANT_EFFECTS_RUNTIME_SDKS)

    def test_future_runtime_modules_and_domain_crates_cannot_import_sdk(self) -> None:
        for path in (
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "future_effect.rs",
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "operator_v4.rs",
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "owner_v3.rs",
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "nested" / "operator_v2.rs",
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "nested" / "operator_v3.rs",
            QDRANT_EFFECTS_MANIFEST.parent / "src" / "nested" / "owner_generic_v1.rs",
            RUST_ROOT / "crates" / "qdrant_effects_core" / "src" / "lib.rs",
            RUST_ROOT / "crates" / "operation_protocol" / "src" / "lib.rs",
            RUST_ROOT / "services" / "temporal_pg_effects" / "src" / "operator.rs",
            RUST_ROOT / "services" / "temporal_orchestrator" / "src" / "owner_v2.rs",
        ):
            for sdk in QDRANT_EFFECTS_RUNTIME_SDKS:
                with self.subTest(path=path, sdk=sdk):
                    self.assertFalse(qdrant_runtime_direct_temporal_sdk_allowed(
                        path, re.compile(rf"\buse\s+{sdk.replace('-', '_')}::")))

    def test_new_generic_owner_files_do_not_allow_other_provider_or_sdk_imports(self) -> None:
        for filename in ("operator_v3.rs", "owner_generic_v1.rs"):
            path = QDRANT_EFFECTS_MANIFEST.parent / "src" / filename
            for forbidden in ("qdrant_client", "neo4rs", "reqwest", "temporalio_sdk_core",
                              "tokio_postgres", "hyper", "tonic"):
                with self.subTest(file=filename, forbidden=forbidden):
                    self.assertFalse(qdrant_runtime_direct_temporal_sdk_allowed(
                        path, re.compile(rf"\buse\s+{forbidden}::")))

    def test_e2e_is_exact_optional_native_temporal_sdk_set(self) -> None:
        self.assertEqual(QDRANT_TEMPORAL_E2E_SDKS, {
            "temporalio-client", "temporalio-common",
        })
        manifest = tomllib.loads((QDRANT_TEMPORAL_E2E_DIR / "Cargo.toml").read_text())
        for sdk in QDRANT_TEMPORAL_E2E_SDKS:
            with self.subTest(sdk=sdk):
                self.assertTrue(manifest["dependencies"][sdk]["optional"])
                self.assertIn(f"dep:{sdk}", manifest["features"]["e2e"])
                self.assertTrue(qdrant_e2e_direct_temporal_sdk_allowed(
                    QDRANT_TEMPORAL_E2E_SRC / "lib.rs",
                    re.compile(rf"\buse\s+{sdk.replace('-', '_')}::")))
        for forbidden in ("temporalio_sdk", "temporalio_sdk_core", "temporalio_macros", "neo4rs", "qdrant_client"):
            self.assertFalse(qdrant_e2e_direct_temporal_sdk_allowed(
                QDRANT_TEMPORAL_E2E_SRC / "tests" / "unexpected.rs",
                re.compile(rf"\buse\s+{forbidden}::")))
        self.assertFalse(qdrant_e2e_direct_temporal_sdk_allowed(
            QDRANT_TEMPORAL_E2E_DIR / "build.rs",
            re.compile(r"\btemporalio_client::")))
        self.assertEqual(
            MANIFEST_DEP_EXCEPTIONS[(QDRANT_TEMPORAL_E2E_DIR / "Cargo.toml").resolve()],
            QDRANT_TEMPORAL_E2E_SDKS,
        )


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
