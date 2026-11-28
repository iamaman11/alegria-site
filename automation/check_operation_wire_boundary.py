#!/usr/bin/env python3
"""Fail-closed source check for legacy Proto, typed DAG, and additive owner V3.

This check does NOT generate a second operation catalog or declare that 52
provisional operations are accepted. It verifies where each *existing* wire
format stops, so a future refactor cannot silently turn the pure domain
protocol into an unvalidated JSON/protobuf transport.

  .proto -> prost-generated legacy Temporal V1 payloads (fixed replay schema)
  operation_protocol -> owner-neutral typed Rust DAG V2 (canonical model)
  owner_schema_v3 -> additive opaque bounded descriptor pin, never V2 edit
  trusted_owner_catalog -> optional Protobuf FileDescriptorSet syntax check
  JSON -> checked authoring/serialized artifact envelope at named adapters
  Temporal pure V3 -> historical Serde replay; V4 -> opt-in strict ingress
  Temporal pure V5 -> new schema-pinned input, independent queue and replay

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
    "owner_schema_json": "app/rust/crates/operation_protocol/src/owner_schema_json.rs",
    "trusted_owner_catalog": "app/rust/crates/trusted_owner_catalog/src/lib.rs",
    "owner_generated_dto_schema": "app/rust/crates/seo_application/src/operation_adapter/owner_dto_schema.rs",
    "typed_pure_adapter": "app/rust/crates/seo_application/src/operation_adapter.rs",
    "site_architect_authoring": "app/rust/crates/seo_application/src/site_architect/parameters_json.rs",
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
    # Even prost_types is a forbidden transitive dependency: any protobuf
    # decoder in operation_protocol would pull it into every pure domain
    # owner (including truth_domain) through the generic DAG protocol.
    if re.search(r"^\s*(?:contracts|prost|prost-types|prost-build)\s*=", cargo, re.MULTILINE):
        failures.append("provider-neutral operation_protocol must not depend on protobuf crates")

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
    require(owner_schema, r"owner_schema_json::validate_json_schema_2020_12\(&self\.descriptor_bytes\)",
            "V3 validates the actual JSON Schema descriptor bytes", failures)
    descriptor_parser = sources["owner_schema_json"]
    require(descriptor_parser, r"serde_json::from_slice\(bytes\)",
            "owner JSON descriptor parser decodes supplied schema bytes", failures)
    require(descriptor_parser, r"values\.insert\(key,\s*value\)\.is_some\(\)",
            "owner JSON descriptor parser rejects recursively duplicate keys", failures)
    if re.search(r"\b(?:prost|prost_types)::|FileDescriptorSet::decode\(",
                 protocol + owner_schema):
        failures.append("protobuf descriptor decoding must remain outside provider-neutral operation_protocol")
    require(owner_schema, r"JsonSchema2020_12\s*=>\s*\{",
            "V3 schema validation checks the declared format", failures)
    require(descriptor_parser, r"Value::Bool\(false\)",
            "V3 allows a provably uninhabited Failure payload", failures)

    # The trusted schema may not be minted from untrusted caller manifest
    # bytes. A schema installed by the native owner is a distinct trust root
    # from a client's optional requested pin or parameter schema.
    trusted_catalog = sources["trusted_owner_catalog"]
    require(trusted_catalog, r"FileDescriptorSet::decode\(",
            "trusted owner catalog parses Protobuf descriptors at installation boundary", failures)
    require(trusted_catalog, r"fn\s+verify_installed_protobuf_descriptors\(",
            "Protobuf syntax validation is an explicit trusted installation step", failures)
    if trusted_catalog.count("verify_installed_protobuf_descriptors(") < 3:
        failures.append("both effectful and pure trusted owner schema installations validate Protobuf descriptors")
    require(owner_schema, r"OwnerDtoDescriptorFormatV3::ProtobufFileDescriptorSet\s*=>\s*\{\s*\}",
            "provider-neutral V3 keeps Protobuf descriptor opaque after bounded validation", failures)
    require(trusted_catalog, r"owner_schemas:\s*BTreeMap<OperationRef,\s*PinnedOwnerOperationSchemaV3>",
            "trusted owner bundle stores pinned owner structural schemas", failures)
    require(trusted_catalog, r"fn\s+verify_caller_schema\(",
            "trusted owner catalog verifies caller pin against installed schema", failures)
    require(trusted_catalog, r"verify_against_trusted_owner\(",
            "catalog checks untrusted pin against separately installed owner pin", failures)
    require(trusted_catalog, r"owner_schema:\s*Option<PinnedOwnerOperationSchemaV3>",
            "trusted catalog V3 entry requires a typed owner schema field", failures)

    # The owner-local schema generator is the only place that knows which
    # *real Rust DTO type* inhabits a semantic Input/Output wire port. This
    # source gate is cheap; Rust tests must additionally show that changing
    # a field, enum variant or Option type changes the resulting descriptor.
    owner_schema_source = sources["owner_generated_dto_schema"]
    require(owner_schema_source, r"SchemaSettings::draft2020_12\(\)",
            "owner generator pins JSON Schema 2020-12 dialect", failures)
    require(owner_schema_source, r"DtoWireRole::InputDeserialize\s*=>\s*settings\.for_deserialize\(\)",
            "owner Input schema models actual Deserialize wire", failures)
    require(owner_schema_source, r"DtoWireRole::OutputSerialize\s*=>\s*settings\.for_serialize\(\)",
            "owner Output schema models actual Serialize wire", failures)
    require(owner_schema_source, r"into_generator\(\)\.into_root_schema_for::<T>\(\)",
            "owner descriptor generator derives schema from Rust DTO", failures)
    require(owner_schema_source,
            r"descriptor_from_dto::<O::Input>\(input_artifact,\s*DtoWireRole::InputDeserialize\)",
            "owner Input descriptor is generated from concrete Rust DTO", failures)
    require(owner_schema_source,
            r"descriptor_from_dto::<O::Output>\(output_artifact,\s*DtoWireRole::OutputSerialize\)",
            "owner Output descriptor is generated from concrete Rust DTO", failures)
    require(owner_schema_source, r"fn\s+semantic_extraction_owner_schemas\(",
            "semantic owner exposes generated operation schema pins", failures)

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
    if "crate::site_architect::" in adapter:
        failures.append("runtime owner adapter cannot depend on siteArchitect authoring UI")

    authoring = sources["site_architect_authoring"]
    require(authoring, r"struct\s+RecipeParameters\(Value\)",
            "siteArchitect JSON is authoring-only, not a domain operation DTO", failures)
    require(authoring, r"preserves_authored_fields\(value,\s*&normalized\)",
            "siteArchitect rejects silent parameter loss", failures)
    require(authoring, r"UniqueJsonValue::deserialize\(deserializer\)",
            "siteArchitect rejects duplicate raw parameter keys", failures)
    require(authoring, r"let\s+decoded:\s*T\s*=\s*serde_json::from_value",
            "siteArchitect locally validates owner concrete DTO", failures)
    require(authoring, r"fn\s+register_local<T>",
            "siteArchitect local owner typed registration", failures)

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
    v5 = temporal_workflow_impl(workflows, "AlegriaPureOperationDagV3Workflow") or ""
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
    require(workflows,
            r"struct\s+PureDagRunSchemaPinnedV5\s*\{[^}]*owner_schemas:\s*Vec<PinnedOwnerOperationSchemaV3>",
            "V5 Temporal Start History includes full immutable owner structural pins", failures)
    require(workflows, r"fn\s+verify_owner_generated_pins_v5\(",
            "V5 checks owner structural attestation before pure DAG execution", failures)
    require(workflows,
            r"semantic_extraction_owner_schemas\(\)",
            "V5 verifies against owner-generated rather than caller-issued schemas", failures)
    require(workflows, r"pin\.verify_against_trusted_owner\(expected,\s*contract\)",
            "V5 compares each caller pin with concrete installed owner identity", failures)
    require(v5,
            r"verify_owner_generated_pins_v5\(&input\)\s*\.map_err\(workflow_failure\)\?\s*;[\s\S]*execute_pure_dag\(&workflow_id,\s*&input\.run\)",
            "V5 verifies full structural pins before typed pure DAG execution", failures)
    require(v5, r"WorkflowResult<PureDagReceipt>",
            "V5 preserves typed output receipt ABI", failures)

    launcher = sources["temporal_pure_launcher"]
    require(launcher, r'const\s+PURE_DAG_ORCHESTRATOR_QUEUE:\s*&str\s*=\s*"alegria-pure-dag-v3";',
            "historical Temporal V3 queue stays unchanged", failures)
    require(launcher, r'const\s+PURE_DAG_ORCHESTRATOR_V4_QUEUE:\s*&str\s*=\s*"alegria-pure-dag-v4";',
            "new Temporal V4 queue is independently versioned", failures)
    require(launcher, r'const\s+PURE_V4_LIVE_OPT_IN:\s*&str\s*=\s*"ALLOW_PURE_DAG_ORCHESTRATOR_V4";',
            "new Temporal V4 has independent opt-in", failures)
    require(launcher, r"Self::PureV4\s*=>\s*PURE_DAG_ORCHESTRATOR_V4_QUEUE",
            "new Temporal V4 mode binds its own queue", failures)
    require(launcher, r'const\s+PURE_DAG_ORCHESTRATOR_V5_QUEUE:\s*&str\s*=\s*"alegria-pure-dag-v5";',
            "V5 Temporal has separate Task Queue", failures)
    require(launcher, r'const\s+PURE_V5_LIVE_OPT_IN:\s*&str\s*=\s*"ALLOW_PURE_DAG_ORCHESTRATOR_V5";',
            "V5 Temporal requires independent opt-in", failures)
    require(launcher, r"Self::PureV5SchemaPinned\s*=>\s*PURE_DAG_ORCHESTRATOR_V5_QUEUE",
            "V5 Temporal mode maps to schema-pinned queue", failures)

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
          "temporal_pure=v3_legacy_v4_strict_v5_schema_pinned json=checked_adapter_envelope "
          "only_static_source_proof=true")
    return 0


if __name__ == "__main__":
    sys.exit(main())
