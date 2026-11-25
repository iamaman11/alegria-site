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
    "app/rust/crates/voyage_api/src/lib.rs",
    # Real Temporal/history and full-stack tests were moved verbatim out of
    # the broad harness. Grant exact test files, never an E2E crate prefix.
    "app/rust/crates/integration_temporal_e2e/src/tests/manual_llm_temporal_tests.rs",
    "app/rust/crates/integration_temporal_e2e/src/tests/pure_v3_temporal_tests.rs",
    "app/rust/crates/integration_temporal_e2e/src/tests/scenario_parity_tests.rs",
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
               | V3_TYPED_EFFECT_JSON_BOUNDARIES | EXTRACTED_OWNER_JSON_BOUNDARIES):
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
