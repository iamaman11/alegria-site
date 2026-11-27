#!/usr/bin/env python3
"""Fail-closed source check for legacy Proto, typed DAG, and additive owner V3.

This check does NOT generate a second operation catalog or declare that 52
provisional operations are accepted. It verifies where each *existing* wire
format stops, so a future refactor cannot silently turn the pure domain
protocol into an unvalidated JSON/protobuf transport.

  .proto -> prost-generated legacy Temporal V1 payloads (fixed replay schema)
  operation_protocol -> owner-neutral typed Rust DAG V2 (canonical model)
  owner_schema_v3 -> additive descriptor pin, never an in-place V2 history edit
  JSON -> checked authoring/serialized artifact envelope at named adapters
  Temporal pure V3 -> historical Serde replay; new V4 -> opt-in strict ingress

The formats are not interchangeable; proof of a native Temporal E2E or
independent owner authorization is separate from these source checks.
"""

from __future__ import annotations

import hashlib
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCES = {
    "temporal_v1_proto": "app/contracts/proto/temporal_payloads.proto",
    "temporal_v1_condition_import": "app/contracts/proto/condition.proto",
    "prost_build": "app/rust/crates/contracts/build.rs",
    "prost_generated_module": "app/rust/crates/contracts/src/lib.rs",
    "operation_protocol": "app/rust/crates/operation_protocol/src/lib.rs",
    "operation_protocol_cargo": "app/rust/crates/operation_protocol/Cargo.toml",
    "owner_schema_v3": "app/rust/crates/operation_protocol/src/owner_schema_v3.rs",
    "typed_pure_adapter": "app/rust/crates/seo_application/src/operation_adapter.rs",
    "site_lab_authoring": "app/rust/crates/seo_application/src/site_lab/parameters_json.rs",
    "typed_json_boundary": "app/rust/crates/seo_application/src/typed_json.rs",
    "temporal_pure_workflows": "app/rust/services/temporal_orchestrator/src/workflow.rs",
    "temporal_pure_launcher": "app/rust/services/temporal_orchestrator/src/main.rs",
}

# These are *declaration* fingerprints of the existing V1 Protobuf IDL,
# not a second schema registry. They exclude whitespace and // comments.
# A deliberate V1 wire change must be separately reviewed for historical
# Temporal replay and explicitly update this pin. New protocols belong in
# separately versioned contracts, not by editing the V1 declaration in place.
LEGACY_V1_PROTO_DECLARATION_PINS = {
    "temporal_v1_proto": "eb2e2fa04e9e5f6304479b493732e75a2c542330f3cdf8d52716557c18e00f4f",
    "temporal_v1_condition_import": "1dfc096b5118b292e9faa04d57ce8186363d56bb5d12bc8dddb6dfc842b0455c",
}

# The existing V2 ExecutionPlan serializes OperationContract and pins its
# Blake3 fingerprint in Temporal History. An additive owner descriptor must
# be a NEW versioned type, not a field added in-place to OperationContract.
V2_OPERATION_CONTRACT_DECLARATION_PIN = "a835a633172d572225d536faa630d47b354c3a7dccb7bc0a96d1a831853d085d"


def proto_declaration_fingerprint(source: str) -> str:
    """Hash V1 declaration tokens, ignoring only comments and formatting.

    This is a source drift gate. It does not prove Protobuf compatibility,
    presence of stored Histories, owner DTO shape, or runtime replay.
    """
    declarations = "".join(line.partition("//")[0] for line in source.splitlines())
    tokens = re.sub(r"\s+", "", declarations)
    return hashlib.sha256(tokens.encode("utf-8")).hexdigest()


def v2_operation_contract_declaration_fingerprint(protocol: str) -> str | None:
    """Hash the original V2 serde struct declaration, ignoring only comments.

    This protects the *historical serialized layout*, not the structure of
    independently compiled owner Input/Output/Failure DTOs.
    """
    match = re.search(r"pub\s+struct\s+OperationContract\s*\{([^}]*)\}", protocol, re.DOTALL)
    if match is None:
        return None
    declarations = "".join(line.partition("//")[0] for line in match.group(1).splitlines())
    tokens = re.sub(r"\s+", "", declarations)
    return hashlib.sha256(tokens.encode("utf-8")).hexdigest()


def temporal_workflow_impl(source: str, rust_type: str) -> str | None:
    """Read one `#[workflow_methods]` impl, excluding later Workflow types.

    This avoids a false positive where the correct call appears only in the
    next Workflow after an old Workflow accidentally changes its behaviour.
    """
    for section in re.split(r"(?m)^#\[workflow_methods\]\s*$", source):
        if re.search(rf"(?m)^impl\s+{re.escape(rust_type)}\s*\{{", section):
            return section
    return None


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
    for source_name, expected in LEGACY_V1_PROTO_DECLARATION_PINS.items():
        actual = proto_declaration_fingerprint(sources[source_name])
        if actual != expected:
            failures.append(f"legacy V1 Protobuf declaration drift: {source_name}")
    require(proto, r'^syntax\s*=\s*"proto3"\s*;', "legacy V1 proto3 syntax", failures)
    require(proto, r"^package\s+alegria\.temporal\.v1\s*;", "legacy V1 package", failures)
    require(proto, r'^import\s+"condition\.proto"\s*;',
            "legacy V1 condition import remains explicit", failures)
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
    require(protocol, r"#\[serde\(deny_unknown_fields\)\]\s*pub\s+struct\s+OperationContract",
            "historical V2 owner contract rejects unknown serialized fields", failures)
    if v2_operation_contract_declaration_fingerprint(protocol) != V2_OPERATION_CONTRACT_DECLARATION_PIN:
        failures.append("historical V2 OperationContract serialized layout drift")
    require(protocol, r"fn\s+fingerprint\(&self\)[^}]*serde_json::to_vec\(self\)",
            "historical V2 fingerprint keeps exact serialized contract", failures)
    require(protocol, r"fn\s+fingerprint\(&self\)[^}]*blake3::hash\(&encoded\)",
            "historical V2 fingerprint keeps Blake3 of serialized contract", failures)
    if "serde_json::Value" in protocol or "contracts::" in protocol or "prost::" in protocol:
        failures.append("domain operation_protocol must not depend on dynamic JSON or legacy protobuf DTOs")
    cargo = sources["operation_protocol_cargo"]
    if re.search(r"^\s*(?:contracts|prost|prost-build)\s*=", cargo, re.MULTILINE):
        failures.append("provider-neutral operation_protocol cannot depend on protobuf transport crates")

    owner_schema = sources["owner_schema_v3"]
    require(protocol, r"pub\s+use\s+owner_schema_v3::\{",
            "additive V3 owner schema is exported separately from V2", failures)
    require(owner_schema, r"pub\s+const\s+OWNER_SCHEMA_ATTESTATION_REVISION_V3:\s*u16\s*=\s*3\s*;",
            "additive owner schema V3 keeps independent revision", failures)
    require(owner_schema, r"pub\s+struct\s+OwnerDtoDescriptorV3\s*\{[^}]*descriptor_bytes:\s*Vec<u8>",
            "V3 owner DTO descriptor pins exact bytes", failures)
    require(owner_schema, r"pub\s+struct\s+OwnerOperationSchemaV3\s*\{[^}]*v2_contract_fingerprint:\s*ContractHash",
            "V3 attestation pins existing V2 contract identity", failures)
    require(owner_schema, r"pub\s+struct\s+OwnerOperationSchemaV3\s*\{[^}]*inputs:\s*BTreeMap<PortId,\s*OwnerDtoDescriptorV3>",
            "V3 attestation pins owner Input descriptor", failures)
    require(owner_schema, r"pub\s+struct\s+OwnerOperationSchemaV3\s*\{[^}]*outputs:\s*BTreeMap<PortId,\s*OwnerDtoDescriptorV3>",
            "V3 attestation pins owner Output descriptor", failures)
    require(owner_schema, r"pub\s+struct\s+OwnerOperationSchemaV3\s*\{[^}]*failure:\s*OwnerDtoDescriptorV3",
            "V3 attestation requires owner Failure descriptor", failures)
    require(owner_schema, r"pub\s+fn\s+verify_against_trusted_owner\(",
            "V3 cannot self-attest caller-controlled schema", failures)
    require(owner_schema, r"self\.fingerprint\s*!=\s*trusted\.fingerprint",
            "V3 compares a separately installed trusted owner pin", failures)
    if "serde_json::Value" in owner_schema or "serde_json::from_slice" in owner_schema or "prost::" in owner_schema:
        failures.append("provider-neutral V3 descriptor cannot decode provider JSON/protobuf DTOs")

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

    workflows = sources["temporal_pure_workflows"]
    v3 = temporal_workflow_impl(workflows, "AlegriaPureOperationDagV1Workflow") or ""
    v4 = temporal_workflow_impl(workflows, "AlegriaPureOperationDagV2Workflow") or ""
    require(v3, r"execute_pure_dag_legacy_v3\(&workflow_id,\s*&input\)",
            "historical Temporal V3 pure replay retains legacy Serde ingress", failures)
    require(v3, r"WorkflowResult<String>",
            "historical Temporal V3 pure replay retains String result ABI", failures)
    require(v4, r"execute_pure_dag\(&workflow_id,\s*&input\)",
            "new Temporal V4 pure workflow uses strict ingress", failures)
    require(v4, r"WorkflowResult<PureDagReceipt>",
            "new Temporal V4 pure workflow uses typed receipt ABI", failures)
    require(workflows, r"register_workflow::<AlegriaPureOperationDagV2Workflow>\(\)",
            "new Temporal V4 pure workflow is separately registered", failures)

    launcher = sources["temporal_pure_launcher"]
    require(launcher, r'const\s+PURE_DAG_ORCHESTRATOR_QUEUE:\s*&str\s*=\s*"alegria-pure-dag-v3";',
            "historical Temporal V3 queue stays unchanged", failures)
    require(launcher, r'const\s+PURE_DAG_ORCHESTRATOR_V4_QUEUE:\s*&str\s*=\s*"alegria-pure-dag-v4";',
            "new Temporal V4 queue is independently versioned", failures)
    require(launcher, r'const\s+PURE_V4_LIVE_OPT_IN:\s*&str\s*=\s*"ALLOW_PURE_DAG_ORCHESTRATOR_V4";',
            "new Temporal V4 has independent opt-in", failures)
    require(launcher, r"Self::PureV4\s*=>\s*PURE_DAG_ORCHESTRATOR_V4_QUEUE",
            "new Temporal V4 mode binds its own queue", failures)

    return failures


def main() -> int:
    failures = check_sources(source_texts())
    if failures:
        print("OPERATION_WIRE_BOUNDARY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("OPERATION_WIRE_BOUNDARY: PASS legacy_protobuf_v1=prost_generated "
          "domain_dag=rust_typed_v2 owner_schema=additive_v3 "
          "temporal_pure=v3_legacy_v4_strict json=checked_adapter_envelope "
          "only_static_source_proof=true")
    return 0


if __name__ == "__main__":
    sys.exit(main())
