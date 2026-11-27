#!/usr/bin/env python3
"""Fail-closed source boundary check for legacy protobuf and typed DAG v2.

This check does NOT generate a second operation catalog or declare that 52
provisional operations are accepted. It verifies where each *existing* wire
format stops, so a future refactor cannot silently turn the pure domain
protocol into an unvalidated JSON/protobuf transport.

  .proto -> prost-generated legacy Temporal V1 payloads (fixed replay schema)
  operation_protocol -> owner-neutral typed Rust DAG V2 (canonical model)
  JSON -> checked authoring/serialized artifact envelope at named adapters

The formats are not interchangeable; proof of a native Temporal E2E or
independent owner authorization is separate from these source checks.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCES = {
    "temporal_v1_proto": "app/contracts/proto/temporal_payloads.proto",
    "prost_build": "app/rust/crates/contracts/build.rs",
    "prost_generated_module": "app/rust/crates/contracts/src/lib.rs",
    "operation_protocol": "app/rust/crates/operation_protocol/src/lib.rs",
    "operation_protocol_cargo": "app/rust/crates/operation_protocol/Cargo.toml",
    "typed_pure_adapter": "app/rust/crates/seo_application/src/operation_adapter.rs",
    "site_lab_authoring": "app/rust/crates/seo_application/src/site_lab/parameters_json.rs",
    "typed_json_boundary": "app/rust/crates/seo_application/src/typed_json.rs",
}


def source_texts() -> dict[str, str]:
    return {key: (ROOT / name).read_text(encoding="utf-8") for key, name in SOURCES.items()}


def require(code: str, pattern: str, label: str, failures: list[str]) -> None:
    if not re.search(pattern, code, re.MULTILINE | re.DOTALL):
        failures.append(label)


def check_sources(sources: dict[str, str]) -> list[str]:
    """Inspect exact source boundaries; callers can mutate in-memory fixtures.

    A passing check proves *static source-shape* only. Rust compilation, DTO
    semantics, immutable owner catalog, auth and Temporal replay still require
    their existing independent integration/e2e evidence.
    """
    failures: list[str] = []
    missing = set(SOURCES) - sources.keys()
    if missing:
        return [f"missing audited source: {name}" for name in sorted(missing)]

    proto = sources["temporal_v1_proto"]
    require(proto, r'^syntax\s*=\s*"proto3"\s*;', "legacy V1 proto3 syntax", failures)
    require(proto, r"^package\s+alegria\.temporal\.v1\s*;", "legacy V1 package", failures)
    for message in ("StepContractMeta", "StepEnvelope", "SeoSiteBuildInputPayload", "SerpIngestInputPayload"):
        require(proto, rf"^message\s+{message}\s*\{{", f"legacy V1 {message} message", failures)
    require(proto, r"message\s+StepEnvelope\s*\{[^}]*\bbytes\s+payload_bytes\s*=\s*4\s*;",
            "legacy V1 StepEnvelope bytes payload field", failures)
    require(proto, r"message\s+StepContractMeta\s*\{[^}]*\buint32\s+schema_version\s*=\s*3\s*;",
            "legacy V1 StepContractMeta schema version", failures)

    build = sources["prost_build"]
    require(build, r"prost_build::Config::new\(\)", "Rust protobuf generated via prost-build", failures)
    require(build, r'join\("temporal_payloads\.proto"\)', "prost compiles canonical V1 source", failures)
    require(build, r"\.compile_protos\(", "prost compile_protos invocation", failures)
    require(sources["prost_generated_module"], r'include!\(concat!\(env!\("OUT_DIR"\),\s*"/alegria\.temporal\.v1\.rs"\)\)',
            "typed prost module import", failures)

    protocol = sources["operation_protocol"]
    require(protocol, r"pub\s+trait\s+Operation\s*\{[^}]*type\s+Input\s*:\s*Serialize\s*\+\s*DeserializeOwned",
            "operation_protocol typed domain Input", failures)
    require(protocol, r"pub\s+trait\s+Operation\s*\{[^}]*type\s+Output\s*:\s*Serialize\s*\+\s*DeserializeOwned",
            "operation_protocol typed domain Output", failures)
    require(protocol, r"pub\s+trait\s+Operation\s*\{[^}]*type\s+Failure\s*:\s*Error",
            "operation_protocol typed domain Failure", failures)
    require(protocol, r"pub\s+struct\s+OperationContract\s*\{[^}]*owner:\s*OperationOwnerId",
            "operation_protocol domain owner", failures)
    require(protocol, r"pub\s+struct\s+OperationContract\s*\{[^}]*inputs:\s*BTreeMap<PortId,\s*ArtifactSchema>",
            "operation_protocol typed input ports", failures)
    require(protocol, r"pub\s+struct\s+OperationContract\s*\{[^}]*outputs:\s*BTreeMap<PortId,\s*ArtifactSchema>",
            "operation_protocol typed output ports", failures)
    require(protocol, r"pub\s+const\s+PLAN_SCHEMA_REVISION:\s*u16\s*=\s*2\s*;",
            "operation_protocol current DAG revision pin", failures)
    require(protocol, r"pub\s+struct\s+ExecutionPlan\s*\{[^}]*pinned_contracts:\s*Vec<PinnedContract>",
            "operation_protocol immutable contract pins", failures)
    require(protocol, r"pub\s+fn\s+fingerprint\(&self\)",
            "operation_protocol owner contract fingerprint", failures)
    require(protocol, r"pub\s+struct\s+OperationContract\s*\{",
            "operation_protocol owner contract", failures)
    if "serde_json::Value" in protocol or "contracts::" in protocol or "prost::" in protocol:
        failures.append("domain operation_protocol must not depend on dynamic JSON or legacy protobuf DTOs")
    cargo = sources["operation_protocol_cargo"]
    if re.search(r"^\s*(?:contracts|prost|prost-build)\s*=", cargo, re.MULTILINE):
        failures.append("provider-neutral operation_protocol cannot depend on protobuf transport crates")

    adapter = sources["typed_pure_adapter"]
    require(adapter, r"let\s+typed:\s*O::Input\s*=\s*serde_json::from_slice\(bytes\)",
            "pure adapter decodes exactly owner Input", failures)
    require(adapter, r"let\s+output:\s*O::Output\s*=\s*O::evaluate\(&typed\)",
            "pure adapter executes typed owner Output", failures)
    require(adapter, r"registry\.register_pure::<",
            "pure executor installs trusted concrete Rust operations", failures)
    require(adapter, r"typed_json::parse_unique_json_bytes\(bytes\)",
            "pure adapter rejects duplicate authored JSON keys", failures)
    require(adapter, r"typed_json::normalized_wire_json_value\(&typed\)",
            "pure adapter compares actual typed wire serialization", failures)
    require(adapter, r"typed_json::preserves_authored_fields\(&authored,\s*&normalized\)",
            "pure adapter rejects unknown or discarded owner fields", failures)
    if "serde_json::Value" in adapter:
        failures.append("pure owner executor cannot interpret arbitrary JSON Value")
    if "crate::site_lab::" in adapter:
        failures.append("runtime owner adapter cannot depend on Site Lab authoring UI")

    authoring = sources["site_lab_authoring"]
    require(authoring, r"struct\s+RecipeParameters\(Value\)",
            "Site Lab JSON is authoring-only, not a domain operation DTO", failures)
    require(authoring, r"preserves_authored_fields\(value,\s*&normalized\)",
            "Site Lab rejects silent parameter loss", failures)
    require(authoring, r"UniqueJsonValue::deserialize\(deserializer\)",
            "Site Lab rejects duplicate raw parameter keys", failures)
    require(authoring, r"let\s+decoded:\s*T\s*=\s*serde_json::from_value",
            "Site Lab locally validates owner concrete DTO", failures)
    require(authoring, r"fn\s+register_local<T>",
            "Site Lab local owner typed registration", failures)

    boundary = sources["typed_json_boundary"]
    require(boundary, r"fn\s+preserves_authored_fields\(",
            "shared typed JSON boundary checks authored field preservation", failures)
    require(boundary, r"fn\s+parse_unique_json_bytes\(",
            "shared typed JSON boundary rejects duplicate raw keys", failures)
    require(boundary, r"fn\s+normalized_wire_json_value<T:\s*Serialize>",
            "shared typed JSON boundary normalizes concrete DTO", failures)
    require(boundary, r"parse_unique_json_bytes\(&wire\)",
            "shared typed JSON boundary rejects duplicate serialized owner keys", failures)

    return failures


def main() -> int:
    failures = check_sources(source_texts())
    if failures:
        print("OPERATION_WIRE_BOUNDARY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("OPERATION_WIRE_BOUNDARY: PASS legacy_protobuf_v1=prost_generated "
          "domain_dag=rust_typed_v2 json=checked_adapter_envelope only_static_source_proof=true")
    return 0


if __name__ == "__main__":
    sys.exit(main())
