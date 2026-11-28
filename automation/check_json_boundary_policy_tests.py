#!/usr/bin/env python3
"""Regression tests for the isolated PG V3 Temporal History JSON boundary."""

from __future__ import annotations

import unittest

from check_json_boundary_policy import (
    ACQUISITION_OWNER_JSON_TEST_BOUNDARIES,
    ACQUISITION_SOURCE_ISSUER_JSON_BOUNDARIES,
    EXTRACTED_OWNER_JSON_BOUNDARIES,
    FORBIDDEN_PATTERNS,
    NATIVE_OWNER_V2_JSON_BOUNDARIES,
    NATIVE_OWNER_V6_JSON_BOUNDARIES,
    ONE_STEP_REQUEST_GRANT_JSON_BOUNDARIES,
    PG_V3_E2E_HISTORY_TEST_BOUNDARIES,
    PG_V3_OPERATOR_JSON_BOUNDARIES,
    PURE_OWNER_LOCAL_JSON_BOUNDARIES,
    SITE_ARCHITECT_TRUSTED_OWNER_E2E_JSON_BOUNDARIES,
    TYPED_OPERATION_JSON_BOUNDARIES,
    V3_TYPED_EFFECT_JSON_BOUNDARIES,
    ROOT,
    is_allowed_json_boundary,
)


class PgHistoryJsonBoundaryTests(unittest.TestCase):
    def test_one_step_grant_and_acquisition_issuer_have_only_reviewed_wire_boundaries(self) -> None:
        self.assertEqual(ONE_STEP_REQUEST_GRANT_JSON_BOUNDARIES, {
            "app/rust/crates/operation_protocol/src/one_step_request_grant_json.rs",
            "app/rust/crates/operation_protocol/src/signed_mcp_admission_v1.rs",
            "app/rust/services/temporal_pg_effects/src/operator_signed_ingress_v1.rs",
            "app/rust/services/one_step_request_issuer/src/issue_json.rs",
            "app/rust/services/one_step_request_issuer/src/catalog.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/one_step_request_grant_pg_tests.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_one_step_grant_tests.rs",
        })
        self.assertEqual(ACQUISITION_SOURCE_ISSUER_JSON_BOUNDARIES, {
            "app/rust/services/acquisition_source_issuer/src/issue_json.rs",
        })
        for relative in ONE_STEP_REQUEST_GRANT_JSON_BOUNDARIES | ACQUISITION_SOURCE_ISSUER_JSON_BOUNDARIES:
            self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/crates/operation_protocol/src/one_step_request_grant_v1.rs",
            "app/rust/crates/operation_protocol/src/unreviewed_grant_json.rs",
            "app/rust/crates/operation_protocol/src/unreviewed_signed_mcp_admission_json.rs",
            "app/rust/services/acquisition_source_issuer/src/main.rs",
            "app/rust/services/acquisition_source_issuer/src/unreviewed_json.rs",
            "app/rust/services/one_step_request_issuer/src/main.rs",
            "app/rust/services/one_step_request_issuer/src/unreviewed_json.rs",
            "app/rust/services/temporal_pg_effects/src/unreviewed_http_ingress.rs",
        ):
            self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_new_pure_owner_json_is_exact_and_does_not_exempt_domain_modules(self) -> None:
        self.assertEqual(PURE_OWNER_LOCAL_JSON_BOUNDARIES, {
            "app/rust/crates/acquisition/src/operation_owner_schema_json.rs",
            "app/rust/services/temporal_acquisition_pure/src/main.rs",
            "app/rust/crates/seo_steps/src/operation_owner_schema_json.rs",
            "app/rust/services/temporal_seo_steps_pure/src/main.rs",
            "app/rust/crates/truth_domain/src/operation_owner_schema_json.rs",
            "app/rust/services/temporal_truth_pure/src/main.rs",
            "app/rust/crates/truth_domain/src/operations_json.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_acquisition_tests.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_truth_tests.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_seo_steps_tests.rs",
        })
        for relative in PURE_OWNER_LOCAL_JSON_BOUNDARIES:
            self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/crates/acquisition/src/operations.rs",
            "app/rust/crates/acquisition/src/new_owner_json.rs",
            "app/rust/services/temporal_acquisition_pure/src/new_owner_json.rs",
            "app/rust/crates/truth_domain/src/operations.rs",
            "app/rust/crates/truth_domain/src/new_owner_json.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/new_acquisition_json.rs",
        ):
            self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_owner_structural_json_permissions_are_exact_descriptor_files(self) -> None:
        self.assertEqual(TYPED_OPERATION_JSON_BOUNDARIES, {
            "app/rust/crates/seo_application/src/typed_json.rs",
            "app/rust/crates/seo_application/src/operation_adapter/owner_dto_json.rs",
            "app/rust/crates/operation_protocol/src/owner_schema_json.rs",
        })
        for relative in TYPED_OPERATION_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/crates/operation_protocol/src/owner_schema_v3.rs",
            "app/rust/crates/operation_protocol/src/owner_schema_other_json.rs",
            "app/rust/crates/operation_protocol/src/owner_dispatch.rs",
            "app/rust/crates/seo_application/src/operation_adapter/owner_dto_schema.rs",
            "app/rust/crates/seo_application/src/operation_adapter/future_owner_json.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_acquisition_owner_json_is_one_replay_fixture_not_a_domain_exemption(self) -> None:
        exact = "app/rust/crates/acquisition/src/operations_json.rs"
        self.assertEqual(ACQUISITION_OWNER_JSON_TEST_BOUNDARIES, {exact})
        self.assertTrue(is_allowed_json_boundary(ROOT / exact))
        for relative in (
            "app/rust/crates/acquisition/src/operations.rs",
            "app/rust/crates/acquisition/src/source_relevance.rs",
            "app/rust/crates/acquisition/src/new_owner_json.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_site_architect_trusted_owner_e2e_json_is_two_exact_test_files(self) -> None:
        self.assertEqual(SITE_ARCHITECT_TRUSTED_OWNER_E2E_JSON_BOUNDARIES, {
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/trusted_site_architect_owner_catalog_tests.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_pure_composition_tests.rs",
        })
        for relative in SITE_ARCHITECT_TRUSTED_OWNER_E2E_JSON_BOUNDARIES:
            self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        self.assertFalse(is_allowed_json_boundary(ROOT / "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/new_site_architect_json_test.rs"))

    def test_native_v6_json_exceptions_are_only_eight_typed_wire_boundaries(self) -> None:
        self.assertEqual(NATIVE_OWNER_V6_JSON_BOUNDARIES, {
            "app/rust/crates/qdrant_contracts/src/qdrant_effect_child_v2.rs",
            "app/rust/crates/trusted_owner_catalog/src/lib.rs",
            "app/rust/services/temporal_pg_effects/src/operator_owner_v1.rs",
            "app/rust/services/temporal_qdrant_effects/src/operator_v2.rs",
            "app/rust/crates/qdrant_contracts/src/qdrant_owner_generic_v1.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_owner_tests.rs",
            "app/rust/services/temporal_pg_effects/src/operator_dag_governance.rs",
            "app/rust/services/temporal_qdrant_effects/src/operator_v3.rs",
        })
        for relative in NATIVE_OWNER_V6_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))

        for relative in (
            "app/rust/crates/qdrant_contracts/src/lib.rs",
            "app/rust/crates/qdrant_contracts/src/qdrant_effect_child_v1.rs",
            "app/rust/crates/qdrant_contracts/src/future_json.rs",
            "app/rust/crates/qdrant_contracts/src/qdrant_owner_generic_v2.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/future_mixed_owner_json_tests.rs",
            "app/rust/crates/operation_protocol/src/owner_dispatch.rs",
            "app/rust/crates/seo_application/src/operation_adapter/generic_dag_v1.rs",
            "app/rust/services/temporal_orchestrator/src/workflow.rs",
            "app/rust/services/temporal_orchestrator/src/owner_catalog.rs",
            "app/rust/services/temporal_orchestrator/src/owner_catalog_v2.rs",
            "app/rust/services/temporal_pg_effects/src/main.rs",
            "app/rust/services/temporal_pg_effects/src/owner_v2.rs",
            "app/rust/services/temporal_qdrant_effects/src/owner_v2.rs",
            "app/rust/services/temporal_qdrant_effects/src/operator_v4.rs",
            "app/rust/services/temporal_qdrant_effects/src/nested/operator_v3.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_native_v2_json_exceptions_are_exact_owner_transport_adapters(self) -> None:
        self.assertEqual(NATIVE_OWNER_V2_JSON_BOUNDARIES, {
            "app/rust/services/temporal_pg_effects/src/operator_business_run_v1.rs",
            "app/rust/services/temporal_pg_effects/src/operator_dag_governance_v2.rs",
            "app/rust/services/temporal_pg_effects/src/operator_vector_projection_v2.rs",
            "app/rust/services/temporal_qdrant_effects/src/owner_generic_v1.rs",
            # The only newly reviewed operator JSON seams: a bounded,
            # private, typed one-node Start intent and a tenant-scoped,
            # read-only server History inspection request.
            "app/rust/services/temporal_pg_effects/src/operator_single_step_start_v2.rs",
            "app/rust/services/temporal_pg_effects/src/operator_dag_read_v2.rs",
        })
        for relative in NATIVE_OWNER_V2_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/services/temporal_pg_effects/src/operator_dag_governance_v3.rs",
            "app/rust/services/temporal_pg_effects/src/operator_vector_projection_v3.rs",
            "app/rust/services/temporal_pg_effects/src/operator_single_step_start_v3.rs",
            "app/rust/services/temporal_pg_effects/src/operator_dag_read_v3.rs",
            "app/rust/services/temporal_pg_effects/src/operator_single_step_start_v2/owner_rpc.rs",
            "app/rust/services/temporal_qdrant_effects/src/owner_generic_v2.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_v2_single_step_operator_json_requires_strict_typed_private_ingress(self) -> None:
        # The narrow per-file exceptions are justified only while Start and
        # Read use their own strict DTO, operator-owned SQL identity, and
        # immutable Temporal History. They are not general-purpose JSON APIs.
        start = (ROOT / "app/rust/services/temporal_pg_effects/src/operator_single_step_start_v2.rs").read_text()
        reader = (ROOT / "app/rust/services/temporal_pg_effects/src/operator_dag_read_v2.rs").read_text()
        for code in (start, reader):
            self.assertIn("#[serde(deny_unknown_fields)]", code)
            self.assertNotIn("serde_json::Value", code)
            self.assertNotIn("serde_json::from_value", code)
            self.assertIn("read_private_operator_file(", code)
        self.assertIn("preflight_unadmitted_single_operation_v2_start(", start)
        self.assertIn("assert_active_independent_business_root(", start)
        self.assertIn("tenant_read_only_pool(", start)
        self.assertIn("WorkflowIdReusePolicy::RejectDuplicate", start)
        self.assertIn("run_unavailable_or_untrusted", reader)
        self.assertIn("owner_receipt_attestation: \"not_independently_verified\"", reader)
        self.assertIn("attest_one_owner_child(", reader)

    def test_extracted_owner_json_grants_are_exact_files(self) -> None:
        self.assertEqual(EXTRACTED_OWNER_JSON_BOUNDARIES, {
            "app/rust/crates/pg_runtime_ports/src/planning_ports.rs",
            "app/rust/crates/integration_data_fixtures/src/lib.rs",
            "app/rust/crates/integration_outbox_e2e/src/tests/outbox_delivery_tests.rs",
            "app/rust/crates/integration_qdrant_e2e/src/tests/knowledge_read_tests.rs",
            "app/rust/crates/integration_qdrant_e2e/src/tests/admitted_qdrant_projection_tests.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_qdrant_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/qa_persistence_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/publication_delivery_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/truth_adjudication_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/cms_review_pg_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/cms_publish_pg_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/cms_cross_scope_pg_tests.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/cms_navigation_pg_tests.rs",
            "app/rust/crates/projection_delivery/src/projection_materialize_adapter.rs",
            "app/rust/crates/projection_delivery/src/admitted_qdrant_projection_v1.rs",
            "app/rust/crates/qdrant_effects_core/src/lib.rs",
            "app/rust/services/temporal_orchestrator/src/qdrant_child_wire_json.rs",
            "app/rust/services/temporal_qdrant_effects/src/operator.rs",
            "app/rust/crates/voyage_api/src/lib.rs",
            "app/rust/services/temporal_web_fetch/src/archive.rs",
            "app/rust/crates/integration_temporal_e2e/src/tests/manual_llm_temporal_tests.rs",
            "app/rust/crates/integration_temporal_e2e/src/tests/pure_v3_temporal_tests.rs",
            "app/rust/crates/integration_temporal_e2e/src/tests/scenario_parity_tests.rs",
        })
        for relative in EXTRACTED_OWNER_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))

        for relative in (
            "app/rust/crates/integration_outbox_e2e/src/lib.rs",
            "app/rust/crates/integration_outbox_e2e/src/tests/new_json_test.rs",
            "app/rust/crates/integration_qdrant_e2e/src/lib.rs",
            "app/rust/crates/integration_qdrant_e2e/src/tests/new_json_test.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/new_json_test.rs",
            "app/rust/services/temporal_qdrant_effects/src/workflow.rs",
            "app/rust/services/temporal_orchestrator/src/workflow.rs",
            "app/rust/crates/integration_pg_app_e2e/src/tests/new_json_test.rs",
            "app/rust/crates/projection_delivery/src/new_provider.rs",
            "app/rust/crates/qdrant_effects_core/src/new_provider.rs",
            "app/rust/crates/voyage_api/src/operation.rs",
            "app/rust/crates/pg_runtime_ports/src/new_port.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_only_three_actual_temporal_wire_test_files_are_exempt(self) -> None:
        self.assertEqual(PG_V3_E2E_HISTORY_TEST_BOUNDARIES, {
            "app/rust/crates/integration_pg_v3_e2e/src/lib.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_temporal_cross_queue_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v5_ordered_children_tests.rs",
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

    def test_operator_json_is_confined_to_one_cli_adapter_and_one_e2e_fixture(self) -> None:
        self.assertEqual(PG_V3_OPERATOR_JSON_BOUNDARIES, {
            "app/rust/services/temporal_pg_effects/src/operator.rs",
            "app/rust/services/temporal_pg_effects/src/operator_v4.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_operator_cli_tests.rs",
        })
        for relative in PG_V3_OPERATOR_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/services/temporal_pg_effects/src/main.rs",
            "app/rust/services/temporal_pg_effects/src/new_operator_boundary.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/new_operator_json_test.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_typed_effect_json_is_adapter_or_explicit_unit_test_only(self) -> None:
        self.assertEqual(V3_TYPED_EFFECT_JSON_BOUNDARIES, {
            "app/rust/crates/seo_application/src/operation_adapter/effectful_v1.rs",
            "app/rust/crates/seo_application/src/operation_adapter/effectful_v1/tests.rs",
            "app/rust/crates/truth_domain/src/review_effect_contract/tests.rs",
            "app/rust/crates/postgres_store/src/truth_review_effect_v3.rs",
        })
        for relative in V3_TYPED_EFFECT_JSON_BOUNDARIES:
            with self.subTest(relative=relative):
                self.assertTrue(is_allowed_json_boundary(ROOT / relative))
        for relative in (
            "app/rust/crates/seo_application/src/operation_adapter/effectful_new.rs",
            "app/rust/crates/truth_domain/src/review_effect_contract.rs",
            "app/rust/crates/truth_domain/src/review_effect_contract/new_json.rs",
        ):
            with self.subTest(relative=relative):
                self.assertFalse(is_allowed_json_boundary(ROOT / relative))

    def test_production_owner_code_remains_deny_by_default(self) -> None:
        for relative in (
            "app/rust/crates/postgres_store/src/lib.rs",
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
