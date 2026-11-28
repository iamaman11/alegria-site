#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"

# The isolated PG V3 package is an opt-in integration-test target. These two
# exact files decode *captured Temporal test History* inside cfg(test, e2e),
# not application messages at a production use-case boundary. No broad crate
# prefix exemption: a future production module or new test file must justify
# its own JSON boundary explicitly.
PG_V3_E2E_HISTORY_TEST_BOUNDARIES = frozenset({
    "app/rust/crates/integration_pg_v3_e2e/src/lib.rs",
    "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_temporal_cross_queue_tests.rs",
    # Exact opt-in V5 E2E is the native Temporal wire boundary: it builds a
    # serialized parent Start payload and decodes the real History result.
    # This grants no JSON privileges to the production orchestration code.
    "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v5_ordered_children_tests.rs",
})

# The V3 operator's separately credentialed CLI decodes a strictly typed
# request file and writes a typed JSON receipt. Its opt-in real-PG E2E reads
# the CLI stdout and mutates a disposable request fixture. Permit only these
# two exact wire/test files; no broad grant to PG effect Activity code.
PG_V3_OPERATOR_JSON_BOUNDARIES = frozenset({
    "app/rust/services/temporal_pg_effects/src/operator.rs",
    # Exact V4 owner adapter reads server-sourced Temporal History plus its
    # separately credentialed operator CLI wire request/receipt, not domain JSON.
    "app/rust/services/temporal_pg_effects/src/operator_v4.rs",
    "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_operator_cli_tests.rs",
})

# A checked owner-effect DAG decodes an admitted typed artifact only in its
# application adapter. The versioned Truth contract's JSON negative test is
# physically separate, so its *production* domain declaration stays denied.
V3_TYPED_EFFECT_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/seo_application/src/operation_adapter/effectful_v1.rs",
    "app/rust/crates/seo_application/src/operation_adapter/effectful_v1/tests.rs",
    "app/rust/crates/truth_domain/src/review_effect_contract/tests.rs",
    # The actual PostgreSQL owner verifies the V4 child invocation's canonical
    # typed command and bound input bytes against its durable PG admission.
    "app/rust/crates/postgres_store/src/truth_review_effect_v3.rs",
})

# The A14 Outbox integration tests were physically moved out of the broad
# harness; their fixture JSON remains confined to this exact opt-in test file.
# The other two files are production provider wire adapters, moved verbatim
# from the previously permitted infrastructure/runtime adapter directories.
# Explicit file grants keep new modules in these crates deny-by-default.
EXTRACTED_OWNER_JSON_BOUNDARIES = frozenset({
    # SQLx adapter serializes captured DataForSEO rows to their canonical
    # JSONB wire representation. No other new PG port file gets a free grant.
    "app/rust/crates/pg_runtime_ports/src/planning_ports.rs",
    "app/rust/crates/integration_data_fixtures/src/lib.rs",
    "app/rust/crates/integration_outbox_e2e/src/tests/outbox_delivery_tests.rs",
    # Authentic Qdrant/PG test fixture JSON, moved from the broad harness.
    # No blanket JSON permission is granted to the new production-adjacent crate.
    "app/rust/crates/integration_qdrant_e2e/src/tests/knowledge_read_tests.rs",
    # Opt-in real two-container E2E inspects Qdrant REST/gRPC wire payloads
    # while proving the typed PostgreSQL admission/receipt fence end to end.
    "app/rust/crates/integration_qdrant_e2e/src/tests/admitted_qdrant_projection_tests.rs",
    # V7 genuine Temporal parent/child/PG/Qdrant test: only this exact opt-in
    # test file may inspect raw captured server History and RPC wire JSON.
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_qdrant_tests.rs",
    # Real PostgreSQL/HTTP fault-injection tests moved byte-for-byte from the
    # broad test harness, not permission to use JSON in production PG ports.
    "app/rust/crates/integration_pg_app_e2e/src/tests/qa_persistence_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/publication_delivery_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/truth_adjudication_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_review_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_publish_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_cross_scope_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_navigation_pg_tests.rs",
    "app/rust/crates/projection_delivery/src/projection_materialize_adapter.rs",
    # This single explicitly admitted operator wire adapter verifies
    # normalized Qdrant JSON at the external RPC boundary; no domain module
    # or other projection_delivery source gains blanket JSON privileges.
    "app/rust/crates/projection_delivery/src/admitted_qdrant_projection_v1.rs",
    # V7 extracted *single* Qdrant provider-wire executor; this formerly
    # permitted projection_delivery adapter moved into a narrow PG+Qdrant
    # owner crate. No other qdrant_effects_core module receives permission.
    "app/rust/crates/qdrant_effects_core/src/lib.rs",
    # Explicit typed V7 Temporal command payload adapters. Neither the pure
    # orchestrator workflow nor the Qdrant effect logic receives a crate-wide
    # JSON decode exemption; each wire adapter is reviewed independently.
    "app/rust/services/temporal_orchestrator/src/qdrant_child_wire_json.rs",
    "app/rust/services/temporal_qdrant_effects/src/operator.rs",
    "app/rust/crates/voyage_api/src/lib.rs",
    # Real Temporal/history and full-stack tests were moved verbatim out of
    # the broad harness. Grant exact test files, never an E2E crate prefix.
    "app/rust/crates/integration_temporal_e2e/src/tests/manual_llm_temporal_tests.rs",
    "app/rust/crates/integration_temporal_e2e/src/tests/pure_v3_temporal_tests.rs",
    "app/rust/crates/integration_temporal_e2e/src/tests/scenario_parity_tests.rs",
})

# Native V6's reviewed typed JSON ingress points. These are exact, reviewed
# wire/config adapters, not a grant to the provider-free DAG runner, domain
# contracts generally, owner Activities, or any future sibling module.
NATIVE_OWNER_V6_JSON_BOUNDARIES = frozenset({
    # One audited file loader shared by the generic parent and ChatGPT authoring.
    # Root-owned /etc/alegria path checks are mandatory before decoding routes.
    "app/rust/crates/trusted_owner_catalog/src/lib.rs",
    # The Qdrant-owned V2 contract parses the immutable typed command envelope
    # to verify its exact bytes before a provider Activity can be admitted.
    "app/rust/crates/qdrant_contracts/src/qdrant_effect_child_v2.rs",
    # The Temporal parent decodes trusted deployment route configuration,
    # never arbitrary caller-supplied routes or effect implementations.
    # Separately credentialed PG operator decodes strict CLI requests and
    # server-issued Temporal History when verifying one exact owner child.
    "app/rust/services/temporal_pg_effects/src/operator_owner_v1.rs",
    # Separately credentialed Qdrant operator performs the equivalent strict
    # typed CLI/History decoding for its V2 child and issuer admission.
    "app/rust/services/temporal_qdrant_effects/src/operator_v2.rs",
    # Shared V1 Qdrant owner adapter decodes a concrete externally admitted
    # keyword command and emits an explicitly typed receipt. It does not
    # authorize Produced dataflow or offer a general domain JSON entrypoint.
    "app/rust/crates/qdrant_contracts/src/qdrant_owner_generic_v1.rs",
    # This exact opt-in native multi-owner E2E serializes the immutable DAG
    # Start and checks real Temporal History, separate SQL receipt and Qdrant
    # readback. Its sibling or future test files remain deny-by-default.
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_owner_tests.rs",
    # The isolated platform governance operator parses strict typed locators,
    # operator-owned route catalogs and the actual Temporal parent Start.
    # It has no grant to decode arbitrary owner effects or use-case inputs.
    "app/rust/services/temporal_pg_effects/src/operator_dag_governance.rs",
    # The Qdrant V3 issuer binds its concrete typed command to reciprocal
    # parent/child server Histories and tenant-bound immutable SQL authority.
    # This grants no JSON parsing to the Qdrant Activity or other owners.
    "app/rust/services/temporal_qdrant_effects/src/operator_v3.rs",
})

# V7 Produced uses exact, audited serialization boundaries; domain crates,
# protocol internals and unrelated owner modules remain deny-by-default.
NATIVE_OWNER_V7_JSON_BOUNDARIES = frozenset({
    # PostgreSQL-owned adapter decodes its own trusted typed lookup and the
    # JSONB vector returned by the immutable SQL source admission function.
    "app/rust/crates/postgres_store/src/vector_projection_owner_v1.rs",
    # Qdrant contract adapter validates typed bytes received exclusively from
    # the Temporal Child Produced binding; no arbitrary domain JSON entrypoint.
    "app/rust/crates/qdrant_contracts/src/qdrant_produced_vector_v1.rs",
    # Separately privileged PG issuer verifies real Temporal server Histories
    # and its exact typed operator locator before publishing source authority.
    "app/rust/services/temporal_pg_effects/src/operator_vector_projection_v1.rs",
})

# V2 keeps JSON at the exact operator/owner transport adapters. These files
# decode strongly typed operator locators, *server* Temporal Histories, and
# provider-owned ValidateOnly command DTOs. No blanket PG/Qdrant crate,
# generic DAG workflow, domain, or future sibling exemption is permitted.
NATIVE_OWNER_V2_JSON_BOUNDARIES = frozenset({
    # Separate least-privilege business-root issuer parses a private typed
    # operator intent and issues only its own tenant-scoped authority.
    "app/rust/services/temporal_pg_effects/src/operator_business_run_v1.rs",
    "app/rust/services/temporal_pg_effects/src/operator_dag_governance_v2.rs",
    "app/rust/services/temporal_pg_effects/src/operator_vector_projection_v2.rs",
    "app/rust/services/temporal_qdrant_effects/src/owner_generic_v1.rs",
    # Operator-only single-step Temporal Start accepts a bounded private
    # typed request only after an independent tenant-bound SQL business root
    # check. Generic DAG orchestration remains provider/SQL-free.
    "app/rust/services/temporal_pg_effects/src/operator_single_step_start_v2.rs",
    # Tenant-bound, read-only V2 diagnostics decode server History only after
    # SQL business/governance lookup; never raw provider payloads in responses.
    "app/rust/services/temporal_pg_effects/src/operator_dag_read_v2.rs",
})

# siteArchitect accepts untrusted *authoring* JSON, not runtime owner commands.
# This one adapter decodes owner-authored parameter DTOs and hashes their
# canonical bytes without performing an effect. Its exact test fixture file
# exercises strict types and schema pins; no blanket seo_application waiver.
SITE_ARCHITECT_AUTHORING_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/seo_application/src/site_architect/parameters_json.rs",
    "app/rust/crates/seo_application/src/site_architect/tests.rs",
    # Only this exact fixture test constructs parameter values; the production
    # per-operation inspector uses the existing typed catalog and DTO gate.
    "app/rust/crates/seo_application/src/site_architect/operation_inspection/tests.rs",
})

# Exact Serde wire normalizer shared by siteArchitect parameter authoring and the
# pure DAG executor. It only rejects ambiguous or silently changed fields;
# domain Operation DTOs, the provider-neutral protocol, and future sibling
# modules do not inherit permission to interpret raw JSON.
TYPED_OPERATION_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/seo_application/src/typed_json.rs",
    # Owner-local DTO descriptor serialization and its schema regression
    # fixtures, never a generic provider payload executor or DAG authoring
    # permission. Only real concrete Rust DTOs are accepted via schemars.
    "app/rust/crates/seo_application/src/operation_adapter/owner_dto_json.rs",
    # One bounded, fail-closed JSON Schema 2020-12 descriptor parser. The
    # provider-neutral operation_protocol remains denied for arbitrary JSON
    # payloads, including owner_schema_v3.rs and all sibling modules.
    "app/rust/crates/operation_protocol/src/owner_schema_json.rs",
})

# The future one-use Start broker accepts only a single opaque UUID locator.
# Keep its exact JSON boundary distinct from the three frozen owner-schema
# adapters so new product authority cannot widen their permission set.
ONE_STEP_REQUEST_GRANT_JSON_BOUNDARIES = frozenset({
    # Exact one-field opaque Start-request wire decoder. It has no provider,
    # SQL issuer, Temporal Start or authorization authority; all policy and
    # one-use checks remain JSON-free in one_step_request_grant_v1.rs.
    "app/rust/crates/operation_protocol/src/one_step_request_grant_json.rs",
    # Exact bounded canonical OAuth MCP HMAC envelope at the private native
    # admission boundary. This verifier has no SQL authority or Temporal
    # execution access; no sibling operation_protocol module gains JSON rights.
    "app/rust/crates/operation_protocol/src/signed_mcp_admission_v1.rs",
    # The single opt-in, loopback-only operator HTTP adapter decodes a bounded
    # signed MCP proof and serializes a sealed reply; it cannot issue grants.
    # No other temporal_pg_effects module inherits JSON ingress permission.
    "app/rust/services/temporal_pg_effects/src/operator_signed_ingress_v1.rs",
    # Only the independently credentialed issuer's strict private request
    # and protected catalog decoding; no JSON ingress in its SQL/ACL main.
    "app/rust/services/one_step_request_issuer/src/issue_json.rs",
    "app/rust/services/one_step_request_issuer/src/catalog.rs",
    # Exact native physical E2E fixtures construct typed SQL/Temporal wires.
    # The production operator and generic DAG receive no broad exemption.
    "app/rust/crates/integration_pg_app_e2e/src/tests/one_step_request_grant_pg_tests.rs",
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_one_step_grant_tests.rs",
})

# Only the acquisition V1 replay-compatibility *unit fixture* may serialize
# the new strict owner-only DTOs. Production `acquisition::operations` and
# legacy `source_relevance` stay JSON-boundary-free; no crate-wide waiver.
ACQUISITION_OWNER_JSON_TEST_BOUNDARIES = frozenset({
    "app/rust/crates/acquisition/src/operations_json.rs",
})

# This one separate, SQL-restricted Acquisition issuer decodes its operator-
# private immutable source request and emits a redacted issuance diagnostic.
# The HTML transformer, pure owner and generic Temporal operator remain denied
# raw JSON parsing; no broad service or crate permission is introduced.
ACQUISITION_SOURCE_ISSUER_JSON_BOUNDARIES = frozenset({
    "app/rust/services/acquisition_source_issuer/src/issue_json.rs",
})

# Newly isolated owner-local *pure* DTO wire and test boundaries. These exact
# files may decode/inspect JSON for typed Input/Output and structural schema
# tests, but neither a domain policy file nor the generic DAG gains a JSON
# exemption. They do not start Temporal or grant external owner admission.
PURE_OWNER_LOCAL_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/acquisition/src/operation_owner_schema_json.rs",
    "app/rust/services/temporal_acquisition_pure/src/main.rs",
    "app/rust/crates/truth_domain/src/operation_owner_schema_json.rs",
    "app/rust/crates/truth_domain/src/operations_json.rs",
    # Exact opt-in native Temporal parent/Acquisition Child E2E fixture.
    # Its JSON wires are real server History and typed owner artifact bytes.
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_acquisition_tests.rs",
})

# A single opt-in cross-owner integration fixture constructs protected catalog
# JSON and typed Temporal plan wire values for a real PG/Qdrant contract test.
# No production owner, DAG runner, or sibling E2E file inherits this exception.
SITE_ARCHITECT_TRUSTED_OWNER_E2E_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/trusted_site_architect_owner_catalog_tests.rs",
})

# The new native DataForSEO owner has two *exact* typed wire boundaries:
# its contract verifies immutable External/ValidateOnly command bytes, while
# the separately deployed worker exchanges bounded loopback-only stub HTTP
# JSON and serializes a typed owner receipt. Its separately credentialed
# operator reads bounded exact Temporal History and operator intent JSON; that
# boundary cannot mint provider data or authorize public provider HTTP. No
# paid HTTP adapter, sibling domain module or future file inherits this grant.
DATAFORSEO_NATIVE_OWNER_JSON_BOUNDARIES = frozenset({
    "app/rust/crates/dataforseo_contracts/src/lib.rs",
    "app/rust/services/temporal_dataforseo/src/main.rs",
    "app/rust/services/temporal_dataforseo/src/operator.rs",
    # This single opt-in physical E2E decodes captured server History and
    # the immutable SQL receipt, and constructs only loopback HTTP fixtures.
    "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_dataforseo_tests.rs",
})

FORBIDDEN_PATTERNS = {
    "serde_json::Value": re.compile(r"\bserde_json::Value\b"),
    "json! macro": re.compile(r"\bjson!\s*\("),
    "serde_json::to_value": re.compile(r"\bserde_json::to_value\b"),
    "serde_json::from_value": re.compile(r"\bserde_json::from_value\b"),
    "serde_json::from_slice": re.compile(r"\bserde_json::from_slice\b"),
    "serde_json::from_str": re.compile(r"\bserde_json::from_str\b"),
}


def is_allowed_json_boundary(path: Path) -> bool:
    rel = str(path.relative_to(ROOT))
    if rel in (PG_V3_E2E_HISTORY_TEST_BOUNDARIES | PG_V3_OPERATOR_JSON_BOUNDARIES
               | V3_TYPED_EFFECT_JSON_BOUNDARIES | EXTRACTED_OWNER_JSON_BOUNDARIES
               | NATIVE_OWNER_V6_JSON_BOUNDARIES | NATIVE_OWNER_V7_JSON_BOUNDARIES
               | NATIVE_OWNER_V2_JSON_BOUNDARIES | SITE_ARCHITECT_AUTHORING_JSON_BOUNDARIES
               | TYPED_OPERATION_JSON_BOUNDARIES
               | ONE_STEP_REQUEST_GRANT_JSON_BOUNDARIES
               | ACQUISITION_OWNER_JSON_TEST_BOUNDARIES
               | ACQUISITION_SOURCE_ISSUER_JSON_BOUNDARIES
               | PURE_OWNER_LOCAL_JSON_BOUNDARIES
               | SITE_ARCHITECT_TRUSTED_OWNER_E2E_JSON_BOUNDARIES
               | DATAFORSEO_NATIVE_OWNER_JSON_BOUNDARIES):
        return True
    if rel.startswith("app/rust/crates/runtime_models/src/"):
        return True
    if rel.startswith("app/rust/crates/infrastructure/src/adapters/"):
        return True
    # Cross-provider runtime wiring remains an adapter boundary after moving
    # from infrastructure; domains, steps and the crate root stay deny-by-default.
    if rel.startswith("app/rust/crates/runtime_composition/src/adapters/"):
        return True
    # Transport-only Qdrant boundary, extracted verbatim from infrastructure.
    # Wire JSON remains legal here, never in the SEO domain or step crates.
    if rel.startswith("app/rust/crates/vector_qdrant/src/"):
        return True
    if rel.startswith("app/rust/crates/integration_harness/src/"):
        return True
    if rel in {
        "app/rust/crates/seo_ports/src/lib.rs",
        # The PostgreSQL-owned Truth/HITL SQLx adapter serializes audited
        # JSONB decision and review payloads at the persistence boundary.
        # This file moved from infrastructure/adapters without changing its
        # transaction semantics; no domain-wide JSON exception is granted.
        "app/rust/crates/postgres_store/src/sqlx_hitl_adapter.rs",
        # The provider-free protocol's tests verify a strict serialized plan
        # boundary; production domain/operation modules remain deny-by-default.
        "app/rust/crates/operation_protocol/src/tests.rs",
        "app/rust/crates/seo_application/src/rebuild_detect.rs",
        # The checked, typed operation runner owns the in-process artifact
        # serialization boundary. Domain implementations never decode JSON.
        "app/rust/crates/seo_application/src/operation_adapter.rs",
        "app/rust/crates/seo_application/src/operation_adapter/tests.rs",
        "app/rust/crates/seo_domain/src/applicability.rs",
        "app/rust/crates/seo_domain/src/rebuild.rs",
        "app/rust/services/temporal/src/activities/mod.rs",
    }:
        return True
    if rel.startswith("app/rust/crates/primitives/src/") and path.name.endswith("_json.rs"):
        return True
    if rel.startswith("app/rust/crates/telemetry/src/"):
        return True
    if rel.startswith("app/rust/services/outbox_worker/src/"):
        return True
    if rel.startswith("app/rust/services/gsc_sync/src/"):
        return True
    if rel.startswith("app/rust/services/cli_tools/src/"):
        return True
    # The CLI is a wire boundary; other runtime code must use typed inputs.
    if rel.startswith("app/rust/services/temporal/src/bin/temporal_starter/"):
        return True
    if rel in {
        "app/rust/services/temporal/src/bin/whole_page_semantic_report.rs",
        # The preamble and registry decode registered external activity JSON.
        "app/rust/services/temporal/src/activities/operations/contracts/preamble.rs",
        "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs",
        # The activity compatibility tests exercise the registered wire form.
        "app/rust/services/temporal/src/activities/operations/truth_write_and_fixture_tests.rs",
        "app/rust/services/temporal/src/activities/operations/truth_projection_helpers.rs",
    }:
        return True
    return False


def main() -> int:
    violations: list[str] = []
    for rs in sorted(RUST.rglob("*.rs")):
        if "target" in rs.parts:
            continue
        text = rs.read_text(encoding="utf-8")
        hits = [label for label, pattern in FORBIDDEN_PATTERNS.items() if pattern.search(text)]
        if hits and not is_allowed_json_boundary(rs):
            violations.append(f"{rs.relative_to(ROOT)}: forbidden JSON boundary usage {', '.join(hits)}")

    if violations:
        print("JSON_BOUNDARY_POLICY: FAILED")
        for violation in violations:
            print(f"- {violation}")
        print("action: JSON boundary is deny-by-default; keep it only in adapters, *_json.rs, and explicit raw/debug service boundaries")
        return 1

    print("JSON_BOUNDARY_POLICY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
