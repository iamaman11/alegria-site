#!/usr/bin/env python3
"""Negative drift regressions for the Protobuf V1 / typed DAG V2 seam."""

from __future__ import annotations

import unittest

from check_operation_wire_boundary import check_sources, source_texts


class OperationWireBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.baseline = source_texts()

    def mutated(self, source: str, old: str, new: str) -> list[str]:
        text = self.baseline[source]
        self.assertIn(old, text, f"fixture drifted; re-review this check before accepting")
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

    def test_prost_build_must_compile_v1_source(self) -> None:
        failures = self.mutated("prost_build", 'join("temporal_payloads.proto")', 'join("obsolete.proto")')
        self.assertIn("prost compiles canonical V1 source", failures)

    def test_domain_operation_does_not_fall_back_to_untyped_input(self) -> None:
        failures = self.mutated("operation_protocol", "type Input: Serialize + DeserializeOwned", "type Input: Send + Sync")
        self.assertIn("operation_protocol typed domain Input", failures)

    def test_domain_owner_contract_cannot_lose_its_version_pin(self) -> None:
        failures = self.mutated("operation_protocol", "pinned_contracts: Vec<PinnedContract>", "pinned_contracts: Vec<String>")
        self.assertIn("operation_protocol immutable contract pins", failures)

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
        failures = self.mutated("site_lab_authoring", "fn preserves_authored_fields(", "fn forget_authored_fields(")
        self.assertIn("Site Lab rejects silent parameter loss", failures)


if __name__ == "__main__":
    unittest.main()
