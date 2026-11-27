#!/usr/bin/env python3
"""Negative drift regressions for the Protobuf V1 / typed DAG V2 seam."""

from __future__ import annotations

import unittest

from check_operation_wire_boundary import (
    check_sources,
    proto_declaration_fingerprint,
    source_texts,
    temporal_workflow_impl,
    v2_operation_contract_declaration_fingerprint,
)


class OperationWireBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.baseline = source_texts()

    def mutated(self, source: str, old: str, new: str) -> list[str]:
        text = self.baseline[source]
        self.assertIn(old, text, "fixture drifted; re-review this check before accepting")
        changed = dict(self.baseline)
        changed[source] = text.replace(old, new, 1)
        return check_sources(changed)

    def test_current_wire_boundaries_pass_source_audit(self) -> None:
        self.assertEqual(check_sources(self.baseline), [])

    def test_v1_proto_package_must_not_silently_rewrite_history(self) -> None:
        failures = self.mutated("temporal_v1_proto", "package alegria.temporal.v1;", "package alegria.temporal.v2;")
        self.assertIn("legacy V1 package", failures)

    def test_v1_proto_payload_bytes_field_is_pinned(self) -> None:
        failures = self.mutated("temporal_v1_proto", "bytes payload_bytes = 4;", "string payload_bytes = 4;")
        self.assertIn("legacy V1 StepEnvelope bytes payload field", failures)

    def test_legacy_proto_unrelated_field_type_cannot_drift_silently(self) -> None:
        failures = self.mutated(
            "temporal_v1_proto", "string status = 5;", "bool status = 5;"
        )
        self.assertIn("legacy V1 Protobuf declaration drift: temporal_v1_proto", failures)

    def test_imported_condition_proto_is_part_of_frozen_v1_wire(self) -> None:
        failures = self.mutated(
            "temporal_v1_condition_import", "string field = 3;", "bytes field = 3;"
        )
        self.assertIn(
            "legacy V1 Protobuf declaration drift: temporal_v1_condition_import", failures
        )

    def test_v1_proto_comment_or_format_change_does_not_force_replay_review(self) -> None:
        source = self.baseline["temporal_v1_proto"]
        modified = "// documentation only\n" + source.replace(
            "syntax = \"proto3\";", "syntax  =  \"proto3\" ;", 1
        )
        self.assertEqual(
            proto_declaration_fingerprint(source),
            proto_declaration_fingerprint(modified),
        )
        changed = dict(self.baseline)
        changed["temporal_v1_proto"] = modified
        self.assertEqual(check_sources(changed), [])

    def test_v1_schema_must_keep_condition_import(self) -> None:
        failures = self.mutated(
            "temporal_v1_proto", 'import "condition.proto";', 'import "rules.proto";'
        )
        self.assertIn("legacy V1 condition import remains explicit", failures)

    def test_prost_build_must_compile_v1_source(self) -> None:
        failures = self.mutated("prost_build", 'join("temporal_payloads.proto")', 'join("obsolete.proto")')
        self.assertIn("prost compiles canonical V1 source", failures)

    def test_domain_operation_does_not_fall_back_to_untyped_input(self) -> None:
        failures = self.mutated("operation_protocol", "type Input: Serialize + DeserializeOwned", "type Input: Send + Sync")
        self.assertIn("operation_protocol typed domain Input", failures)

    def test_domain_owner_contract_cannot_lose_its_version_pin(self) -> None:
        failures = self.mutated("operation_protocol", "pinned_contracts: Vec<PinnedContract>", "pinned_contracts: Vec<String>")
        self.assertIn("operation_protocol immutable contract pins", failures)

    def test_v2_owner_contract_cannot_mutate_wire_layout_in_place(self) -> None:
        failures = self.mutated(
            "operation_protocol",
            "pub idempotency: IdempotencyPolicy,",
            "pub idempotency: IdempotencyPolicy,\n    pub dto_structural_hash: String,",
        )
        self.assertIn("historical V2 OperationContract serialized layout drift", failures)

    def test_v2_owner_contract_must_retain_denied_unknown_fields(self) -> None:
        failures = self.mutated(
            "operation_protocol",
            "#[serde(deny_unknown_fields)]\npub struct OperationContract",
            "pub struct OperationContract",
        )
        self.assertIn(
            "historical V2 owner contract rejects unknown serialized fields", failures
        )

    def test_v2_contract_docs_do_not_change_the_structural_pin(self) -> None:
        original = self.baseline["operation_protocol"]
        changed = original.replace(
            "pub struct OperationContract {",
            "pub struct OperationContract {\n    /// documentation-only, not serialized",
            1,
        )
        self.assertEqual(
            v2_operation_contract_declaration_fingerprint(original),
            v2_operation_contract_declaration_fingerprint(changed),
        )
        sources = dict(self.baseline)
        sources["operation_protocol"] = changed
        self.assertEqual(check_sources(sources), [])

    def test_v2_fingerprint_algorithm_cannot_be_silently_replaced(self) -> None:
        failures = self.mutated(
            "operation_protocol", "blake3::hash(&encoded)", "blake3::hash(b\"untyped\")"
        )
        self.assertIn(
            "historical V2 fingerprint keeps Blake3 of serialized contract", failures
        )

    def test_v3_owner_schema_is_additive_and_keeps_its_own_revision(self) -> None:
        failures = self.mutated(
            "owner_schema_v3",
            "OWNER_SCHEMA_ATTESTATION_REVISION_V3: u16 = 3;",
            "OWNER_SCHEMA_ATTESTATION_REVISION_V3: u16 = 2;",
        )
        self.assertIn("additive owner schema V3 keeps independent revision", failures)

    def test_v3_owner_schema_cannot_drop_failure_descriptor(self) -> None:
        failures = self.mutated(
            "owner_schema_v3", "pub failure: OwnerDtoDescriptorV3,", "pub failure: String,"
        )
        self.assertIn("V3 attestation requires owner Failure descriptor", failures)

    def test_v3_owner_schema_cannot_be_self_attested_by_caller(self) -> None:
        failures = self.mutated(
            "owner_schema_v3",
            "self.fingerprint != trusted.fingerprint",
            "self.fingerprint != self.fingerprint",
        )
        self.assertIn("V3 compares a separately installed trusted owner pin", failures)

    def test_v3_json_schema_must_validate_real_descriptor_bytes(self) -> None:
        failures = self.mutated(
            "owner_schema_v3",
            "owner_schema_json::validate_json_schema_2020_12(&self.descriptor_bytes)",
            "owner_schema_json::validate_json_schema_2020_12(b\"false\")",
        )
        self.assertIn("V3 validates the actual JSON Schema descriptor bytes", failures)

    def test_owner_descriptor_parser_does_not_reintroduce_last_wins_json(self) -> None:
        failures = self.mutated(
            "owner_schema_json",
            "values.insert(key, value).is_some()",
            "values.insert(key, value).is_none()",
        )
        self.assertIn("owner JSON descriptor parser rejects recursively duplicate keys", failures)

    def test_v3_protobuf_schema_must_decode_real_descriptor_bytes(self) -> None:
        failures = self.mutated(
            "owner_schema_v3",
            "FileDescriptorSet::decode(self.descriptor_bytes.as_slice())",
            "FileDescriptorSet::decode(&[][..])",
        )
        self.assertIn("V3 parses the actual Protobuf descriptor set bytes", failures)

    def test_trusted_schema_registry_cannot_drop_the_owner_pin(self) -> None:
        failures = self.mutated(
            "trusted_owner_catalog",
            "owner_schemas: BTreeMap<OperationRef, PinnedOwnerOperationSchemaV3>",
            "owner_schemas: BTreeMap<OperationRef, String>",
        )
        self.assertIn("trusted owner bundle stores pinned owner structural schemas", failures)

    def test_caller_cannot_self_seal_and_replace_trusted_owner_schema(self) -> None:
        failures = self.mutated(
            "trusted_owner_catalog",
            "verify_against_trusted_owner(",
            "validate(",
        )
        self.assertIn("catalog checks untrusted pin against separately installed owner pin", failures)

    def test_owner_input_schema_must_be_generated_from_real_dto(self) -> None:
        failures = self.mutated(
            "owner_generated_dto_schema",
            "descriptor_from_dto::<O::Input>(input_artifact, DtoWireRole::InputDeserialize)",
            "descriptor_from_dto::<O::Output>(input_artifact, DtoWireRole::InputDeserialize)",
        )
        self.assertIn("owner Input descriptor is generated from concrete Rust DTO", failures)

    def test_owner_output_schema_must_be_generated_from_real_dto(self) -> None:
        failures = self.mutated(
            "owner_generated_dto_schema",
            "descriptor_from_dto::<O::Output>(output_artifact, DtoWireRole::OutputSerialize)",
            "descriptor_from_dto::<O::Output>(output_artifact, DtoWireRole::InputDeserialize)",
        )
        self.assertIn("owner Output descriptor is generated from concrete Rust DTO", failures)

    def test_owner_schema_generator_cannot_be_replaced_with_handwritten_json(self) -> None:
        failures = self.mutated(
            "owner_generated_dto_schema",
            "into_generator().into_root_schema_for::<T>()",
            'into_generator().into_root_schema_for::<String>()',
        )
        self.assertIn("owner descriptor generator derives schema from Rust DTO", failures)

    def test_owner_input_wire_direction_cannot_switch_to_serialize(self) -> None:
        failures = self.mutated(
            "owner_generated_dto_schema",
            "DtoWireRole::InputDeserialize => settings.for_deserialize()",
            "DtoWireRole::InputDeserialize => settings.for_serialize()",
        )
        self.assertIn("owner Input schema models actual Deserialize wire", failures)

    def test_owner_output_wire_direction_cannot_switch_to_deserialize(self) -> None:
        failures = self.mutated(
            "owner_generated_dto_schema",
            "DtoWireRole::OutputSerialize => settings.for_serialize()",
            "DtoWireRole::OutputSerialize => settings.for_deserialize()",
        )
        self.assertIn("owner Output schema models actual Serialize wire", failures)

    def test_domain_protocol_must_not_import_arbitrary_json_value(self) -> None:
        changed = dict(self.baseline)
        changed["operation_protocol"] += "\nuse serde_json::Value;\n"
        self.assertIn(
            "domain operation_protocol must not depend on dynamic JSON or legacy protobuf DTOs",
            check_sources(changed),
        )

    def test_local_pure_adapter_must_decode_concrete_input(self) -> None:
        failures = self.mutated(
            "typed_pure_adapter", "let typed: O::Input =", "let typed: serde_json::Value ="
        )
        self.assertIn("pure adapter decodes exactly owner Input", failures)

    def test_site_lab_cannot_silently_drop_provider_parameters(self) -> None:
        failures = self.mutated("site_lab_authoring", "preserves_authored_fields(value, &normalized)", "true")
        self.assertIn("Site Lab rejects silent parameter loss", failures)

    def test_pure_adapter_cannot_skip_strict_raw_input_check(self) -> None:
        failures = self.mutated("typed_pure_adapter", "typed_json::parse_unique_json_bytes(bytes)", "serde_json::from_slice(bytes)")
        self.assertIn("pure adapter rejects duplicate authored JSON keys", failures)

    def test_shared_boundary_cannot_restore_last_wins_dto_serialization(self) -> None:
        failures = self.mutated("typed_json_boundary", "parse_unique_json_bytes(&wire)", "serde_json::from_slice(&wire)")
        self.assertIn("shared typed JSON boundary rejects duplicate serialized owner keys", failures)

    def test_v3_temporal_replay_cannot_switch_to_new_strict_json_gate(self) -> None:
        failures = self.mutated(
            "temporal_pure_workflows",
            "execute_pure_dag_legacy_v3(&workflow_id, &input)",
            "execute_pure_dag(&workflow_id, &input)",
        )
        self.assertIn(
            "historical Temporal V3 pure replay retains legacy Serde ingress", failures
        )

    def test_v4_temporal_workflow_cannot_fall_back_to_legacy_ingress(self) -> None:
        workflows = self.baseline["temporal_pure_workflows"]
        original_impl = temporal_workflow_impl(
            workflows, "AlegriaPureOperationDagV2Workflow"
        )
        self.assertIsNotNone(original_impl)
        self.assertIn("execute_pure_dag(&workflow_id, &input)", original_impl)
        failures = self.mutated(
            "temporal_pure_workflows",
            "execute_pure_dag(&workflow_id, &input).map_err(workflow_failure)",
            "execute_pure_dag_legacy_v3(&workflow_id, &input).map_err(workflow_failure)",
        )
        self.assertIn("new Temporal V4 pure workflow uses strict ingress", failures)

    def test_v4_temporal_result_must_stay_typed(self) -> None:
        failures = self.mutated(
            "temporal_pure_workflows",
            "WorkflowResult<PureDagReceipt>",
            "WorkflowResult<String>",
        )
        self.assertIn("new Temporal V4 pure workflow uses typed receipt ABI", failures)

    def test_v4_queue_cannot_alias_existing_v3_replay_queue(self) -> None:
        failures = self.mutated(
            "temporal_pure_launcher",
            'PURE_DAG_ORCHESTRATOR_V4_QUEUE: &str = "alegria-pure-dag-v4"',
            'PURE_DAG_ORCHESTRATOR_V4_QUEUE: &str = "alegria-pure-dag-v3"',
        )
        self.assertIn("new Temporal V4 queue is independently versioned", failures)

    def test_v4_deployment_requires_separate_opt_in(self) -> None:
        failures = self.mutated(
            "temporal_pure_launcher",
            'PURE_V4_LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR_V4"',
            'PURE_V4_LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR"',
        )
        self.assertIn("new Temporal V4 has independent opt-in", failures)

    def test_v5_history_input_must_contain_structural_owner_schema_pins(self) -> None:
        failures = self.mutated(
            "temporal_pure_workflows",
            "pub owner_schemas: Vec<PinnedOwnerOperationSchemaV3>",
            "pub owner_schemas: Vec<String>",
        )
        self.assertIn("V5 Temporal Start History includes full immutable owner structural pins", failures)

    def test_v5_must_compare_pins_with_installed_owner_before_execution(self) -> None:
        failures = self.mutated(
            "temporal_pure_workflows",
            "verify_owner_generated_pins_v5(&input).map_err(workflow_failure)?;",
            "let _ = &input.owner_schemas;",
        )
        self.assertIn("V5 verifies full structural pins before typed pure DAG execution", failures)

    def test_v5_must_verify_exact_owner_generated_schema_not_caller_copy(self) -> None:
        failures = self.mutated(
            "temporal_pure_workflows",
            "pin.verify_against_trusted_owner(expected, contract)?",
            "pin.validate(contract)?",
        )
        self.assertIn("V5 compares each caller pin with concrete installed owner identity", failures)

    def test_v5_queue_cannot_reuse_v4_queue_and_history(self) -> None:
        failures = self.mutated(
            "temporal_pure_launcher",
            'PURE_DAG_ORCHESTRATOR_V5_QUEUE: &str = "alegria-pure-dag-v5"',
            'PURE_DAG_ORCHESTRATOR_V5_QUEUE: &str = "alegria-pure-dag-v4"',
        )
        self.assertIn("V5 Temporal has separate Task Queue", failures)

    def test_v5_deployment_requires_independent_opt_in(self) -> None:
        failures = self.mutated(
            "temporal_pure_launcher",
            'PURE_V5_LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR_V5"',
            'PURE_V5_LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR_V4"',
        )
        self.assertIn("V5 Temporal requires independent opt-in", failures)


if __name__ == "__main__":
    unittest.main()
