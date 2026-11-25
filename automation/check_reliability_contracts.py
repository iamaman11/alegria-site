#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
CONTRACTS_BUILD = ROOT / "app" / "rust" / "crates" / "contracts" / "build.rs"
CONTRACTS_LIB = ROOT / "app" / "rust" / "crates" / "contracts" / "src" / "lib.rs"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
REVISION_MIGRATION = ROOT / "app" / "db" / "migrations" / "20261124_0001_verified_rule_projection_revision.sql"
TEMPORAL_MAIN = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "main.rs"
METRICS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "metrics.rs"
OUTBOX_BUILDER = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "outbox_builder.rs"
PIPELINE_RUNTIME_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_pipeline_runtime_adapter.rs"
PROTO_RUNTIME_PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"
GATE = ROOT / "automation" / "temporal_production_gate.sh"
POSTGRES_OUTBOX = ROOT / "app" / "rust" / "crates" / "postgres_store" / "src" / "sqlx_outbox_adapter.rs"
POSTGRES_RECONCILE = ROOT / "app" / "rust" / "crates" / "postgres_store" / "src" / "sqlx_reconcile_adapter.rs"
PROJECTION_DISPATCH = ROOT / "app" / "rust" / "crates" / "runtime_composition" / "src" / "adapters" / "projection_materialize_adapter.rs"
OUTBOX_MATERIALIZE = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "materialize.rs"
OUTBOX_WORKER = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "worker.rs"
TEMPORAL_RUNTIME_SUPPORT = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations" / "runtime_support_ops.rs"
LEGACY_SOURCE_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_source_projection_adapter.rs"
LEGACY_RAW_CRAWL_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "raw_crawl_adapter" / "truth_candidate_mapping.rs"
CANONICAL_VERIFIED_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "verified_rule_projection_adapter.rs"
SQLX_VERIFIED_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter" / "projection_runtime_store.rs"
GENERIC_OUTBOX_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_runtime_outbox_adapter.rs"
CANONICAL_TRUTH_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_truth_write_adapter.rs"
TRUTH_PREFLIGHT = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_truth_write_adapter" / "preflight.rs"
NEO4J_SCHEMA_OWNER = ROOT / "app" / "rust" / "crates" / "graph_neo4j" / "src" / "schema.rs"
NEO4J_CONNECTION_OWNER = ROOT / "app" / "rust" / "crates" / "graph_neo4j" / "src" / "lib.rs"
NEO4J_REAL_HARNESS = ROOT / "app" / "rust" / "crates" / "integration_harness" / "src" / "containers.rs"
NEO4J_REAL_E2E = ROOT / "app" / "rust" / "crates" / "integration_harness" / "src" / "tests" / "provider_runtime_surface_tests.rs"
NEO4J_OPERATOR_TYPES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter" / "types.rs"
NEO4J_OPERATOR_DISPATCH = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter" / "main_dispatch.rs"
NEO4J_LOCAL_GRAPH_GATE = ROOT / "automation" / "run_graph_contract_gate.sh"
OPERATION_PROTOCOL_OWNER = ROOT / "app" / "rust" / "crates" / "operation_protocol" / "src" / "lib.rs"
OPERATION_PROTOCOL_TESTS = ROOT / "app" / "rust" / "crates" / "operation_protocol" / "src" / "tests.rs"
SEMANTIC_EXTRACTION_OPERATIONS = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src" / "extraction" / "operations.rs"
SEMANTIC_OPERATION_RUNNER_TESTS = ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "operation_adapter" / "tests.rs"
CI_IMPACT_PLANNER = ROOT / "automation" / "ci_impact.py"
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
UNCERTAIN_MARKER = "projection_outcome_uncertain:v1"


def match_arm_body(source: str, arm_end: int) -> str | None:
    """Find a braced Rust match arm; fail closed on a missing/unterminated arm."""
    cursor = arm_end
    while cursor < len(source) and source[cursor].isspace():
        cursor += 1
    if cursor >= len(source) or source[cursor] != "{":
        return None
    start, depth = cursor, 0
    for cursor in range(start, len(source)):
        if source[cursor] == "{":
            depth += 1
        elif source[cursor] == "}":
            depth -= 1
            if depth == 0:
                return source[start + 1:cursor]
    return None


def temporal_projection_observer_failures(temporal: str) -> list[str]:
    """Prevent a second delivery authority and false Temporal `done`.

    The status predicate is extracted as executable Rust match branches, not
    accepted just because a matching explanation appears in a comment.
    """
    failures: list[str] = []
    activity = temporal.split("pub(crate) async fn projection_sync_impl(", 1)
    if len(activity) != 2:
        return ["Temporal projection_sync status observer is missing"]
    observer = activity[1].split("pub(crate) async fn ", 1)[0]
    for forbidden in (
        "claim_outbox_batch", "dispatch_leased_event(", "dispatch_event(",
        "mark_done(", "mark_retry(", "mark_failed_with_dead_letter(",
    ):
        if forbidden in observer:
            failures.append(f"Temporal must not deliver Outbox projections: `{forbidden}`")
    if "projection_backlog_counts(" not in observer:
        failures.append("Temporal must observe run/target-scoped durable projection backlog")
    if "projection_drain_state(&backlog" not in observer:
        failures.append("Temporal must derive durable status through the typed drain state")
    if "status: drain_state.as_status().to_string()" not in observer:
        failures.append("Temporal must not forge success outside the durable state predicate")

    helper = temporal.split("fn projection_drain_state(", 1)
    mapping = temporal.split("impl ProjectionDrainState {", 1)
    if len(helper) != 2 or len(mapping) != 2:
        return failures + ["Temporal typed projection drain state mapping is missing"]
    predicate = helper[1].split("pub(crate) async fn projection_sync_impl(", 1)[0]
    status_map = mapping[1].split("fn projection_drain_state(", 1)[0]
    if not re.search(
        r"else\s+if\s+backlog\.pending_events\s*==\s*0\s*&&\s*"
        r"backlog\.failed_events\s*==\s*0\s*\{\s*ProjectionDrainState::Drained",
        predicate,
    ):
        failures.append("Temporal Drained requires zero pending AND zero failed durable events")
    if not re.search(
        r"if\s+backlog\.failed_events\s*>\s*backlog\.uncertain_events\s*"
        r"\{\s*ProjectionDrainState::Blocked",
        predicate,
    ):
        failures.append("Temporal must block proven terminal delivery failures")
    if not re.search(
        r"else\s+if\s+backlog\.uncertain_events\s*>\s*0\s*"
        r"\{\s*ProjectionDrainState::Quarantined",
        predicate,
    ):
        failures.append("Temporal must fail closed on timed-out uncertain provider effects")
    if not re.search(r'Self::Drained\s*=>\s*"done"', status_map):
        failures.append("Only the typed Drained state may map to `done`")
    if re.search(r'Self::(?:Blocked|Quarantined|Stalled|AwaitingOwner)\s*=>\s*"done"', status_map):
        failures.append("Non-drained Temporal state must never return `done`")
    return failures


def fence_contract_failures(
    postgres_outbox: str, reconcile: str, dispatcher: str,
    worker_materialize: str, worker: str, temporal: str,
) -> list[str]:
    """Catch regressions in the executable PostgreSQL/Outbox safety boundary.

    This is a cheap source-level guard, not a substitute for concurrent real-PG
    kill/lease-reclaim tests. The two SQL queries are inspected separately:
    excluding quarantine only in the dry-run count would be unsafe.
    """
    failures: list[str] = []
    required_pg = (
        f'pub const VERIFIED_RULE_UNCERTAIN_MARKER: &str = "{UNCERTAIN_MARKER}"',
        "pub async fn dispatch_verified_rule_with_fence<",
        "VerifiedRuleDispatchError::BlockedUncertain",
        "VerifiedRuleDispatchError::UnknownOutcome",
        ".bind(VERIFIED_RULE_UNCERTAIN_MARKER)",
        "pg_advisory_xact_lock",
    )
    for needle in required_pg:
        if needle not in postgres_outbox:
            failures.append(f"PostgreSQL fenced delivery missing `{needle}`")

    # The durable uncertain marker must be written before the external effect;
    # a cancelled process may never have a chance to mark a result afterwards.
    ordered = (
        "SET status='failed', last_error=$4",
        ".bind(VERIFIED_RULE_UNCERTAIN_MARKER)",
        "effect()",
        "let acknowledged = sqlx::query(",
    )
    positions: list[int] = []
    next_offset = 0
    for needle in ordered:
        position = postgres_outbox.find(needle, next_offset)
        positions.append(position)
        if position != -1:
            next_offset = position + len(needle)
    if -1 in positions or positions != sorted(positions) or len(set(positions)) != len(positions):
        failures.append("PostgreSQL uncertainty marker must be committed before provider effect and ack")

    quarantine_filter = re.compile(
        rf"\bAND\s+last_error\s+IS\s+DISTINCT\s+FROM\s+'{re.escape(UNCERTAIN_MARKER)}'",
        re.IGNORECASE,
    )
    for variable in ("failed_candidates", "failed"):
        statement = re.search(
            rf"\blet\s+{variable}\s*=\s*sqlx::query\(\s*r#\"(.*?)\"#",
            reconcile,
            flags=re.DOTALL,
        )
        if not statement or not quarantine_filter.search(statement.group(1)):
            failures.append(f"reconcile `{variable}` must exclude uncertain Outbox events before requeue")

    for needle in (
        "pub async fn dispatch_leased_event(",
        "dispatch_verified_rule_with_fence(",
        "AcknowledgedByFence",
        "NeedsAck",
        "Quarantine",
        "verified-rule Qdrant RPC requires a PostgreSQL leased Outbox fence",
        "verified-rule Qdrant delete requires a PostgreSQL leased Outbox fence",
    ):
        if needle not in dispatcher:
            failures.append(f"projection dispatcher missing `{needle}`")

    # The dedicated Outbox Worker is the sole projection-delivery authority.
    # A fenced call owns terminal acknowledgement itself, and quarantine must
    # precede the generic retry/dead-letter branch in this single consumer.
    if not re.search(
        r"\bdispatch_leased_event\s*\(\s*pool\s*,\s*event\s*,?\s*\)",
        worker_materialize,
    ):
        failures.append("outbox materialize wrapper must invoke the leased projection dispatcher")
    if "dispatch_event(pool, &event)" not in worker:
        failures.append("outbox worker must pass its claimed event/PG pool to materialize wrapper")

    fenced_call = worker.find("dispatch_event(pool, &event)")
    if fenced_call == -1:
        failures.append("outbox-worker bypasses the leased projection dispatcher")
    if "AcknowledgedByFence" not in worker:
        failures.append("outbox-worker has no fenced acknowledgement disposition")
    fenced_branch = worker[fenced_call:] if fenced_call != -1 else ""
    quarantine_arm = re.search(
        r"Err\s*\(\s*(\w+)\s*\)\s+if\s+\1\.stage\s*==\s*(?:\w+::)*DispatchStage::Quarantine\s*=>",
        fenced_branch,
    )
    generic_error = re.search(r"\bErr\s*\(\s*\w+\s*\)\s*=>", fenced_branch)
    if not quarantine_arm or not generic_error or quarantine_arm.start() > generic_error.start():
        failures.append("outbox-worker must quarantine unknown provider results before ordinary retry")
    elif (body := match_arm_body(fenced_branch, quarantine_arm.end())) is None:
        failures.append("outbox-worker quarantine arm has no inspectable guarded body")
    elif re.search(r"\bmark_(?:done|retry|failed_with_dead_letter)\s*\(", body):
        failures.append("outbox-worker must not mutate Outbox status in the quarantine arm")

    # Temporal may observe completion; only the dedicated worker may deliver.
    failures.extend(temporal_projection_observer_failures(temporal))
    return failures


def truth_writer_fence_failures(files: dict[Path, str], postgres_outbox: str) -> list[str]:
    """Protect the full set of known producer paths, not just RPC consumers."""
    failures: list[str] = []
    if "pub async fn lock_and_reject_projected_legacy_truth_write_in_tx" not in postgres_outbox:
        failures.append("PG owner must reject projected non-atomic legacy Truth mutation")
    if "NonAtomicLegacyProjection" not in postgres_outbox:
        failures.append("PG owner must classify non-atomic legacy projected writes")
    for needle in ("SELECT aggregate_key FROM system.sync_outbox",
                   "SELECT aggregate_key FROM system.verified_rule_projection_heads"):
        if needle not in postgres_outbox:
            failures.append(f"legacy Truth fence must include tombstoned projection history: `{needle}`")
    expected = {
        LEGACY_SOURCE_WRITER: ("lock_and_reject_projected_legacy_truth_write_in_tx", 1),
        LEGACY_RAW_CRAWL_WRITER: ("lock_and_reject_projected_legacy_truth_write_in_tx", 2),
        CANONICAL_VERIFIED_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 1),
        SQLX_VERIFIED_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 3),
        GENERIC_OUTBOX_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 1),
        CANONICAL_TRUTH_WRITER: ("lock_and_reject_projected_legacy_truth_write_in_tx", 1),
    }
    for path, (call, minimum) in expected.items():
        if files[path].count(call) < minimum:
            failures.append(
                f"{path.relative_to(ROOT)} is missing a mandatory verified-rule Truth fence"
            )
    if "verified-rule Qdrant Outbox events require atomic Truth staging" not in files[GENERIC_OUTBOX_WRITER]:
        failures.append("standalone verified-rule Outbox writes must be prohibited")
    if "unstaged_global_verified_rule_ids(" not in files[CANONICAL_TRUTH_WRITER]:
        failures.append("canonical Truth writer must detect verified rules missing an atomic Qdrant intent")
    if "pub(super) fn unstaged_global_verified_rule_ids(" not in files[TRUTH_PREFLIGHT]:
        failures.append("Truth preflight must own the missing-projection-intent classification")
    return failures


def causal_revision_contract_failures(
    schema: str, migration: str, postgres_outbox: str,
    generic_writer: str, canonical_writer: str, sqlx_verified_writer: str,
) -> list[str]:
    """Fail closed on loss of the PostgreSQL-owned causal Outbox ordering.

    This is an inexpensive source invariant, not proof of serializability or
    provider delivery: the real concurrent PostgreSQL/Outbox E2E is mandatory.
    An old UUIDv4/created_at ordering fallback must never become authoritative
    when historical events have no recoverable revision.
    """
    failures: list[str] = []
    for name, source in (("schema", schema), ("migration", migration)):
        for needle in (
            "ADD COLUMN IF NOT EXISTS projection_revision BIGINT",
            "CREATE TABLE IF NOT EXISTS system.verified_rule_projection_heads",
            "aggregate_key TEXT PRIMARY KEY",
            "current_revision BIGINT NOT NULL",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_outbox_verified_rule_revision",
            "sync_outbox_verified_rule_revision_check",
            "OR (projection_revision IS NOT NULL AND projection_revision > 0)) NOT VALID",
        ):
            if needle not in source:
                failures.append(f"causal revision {name} missing `{needle}`")
        if re.search(r"\bUPDATE\s+system\.sync_outbox\s+SET\s+projection_revision\b", source, re.I):
            failures.append(f"causal revision {name} must not fabricate legacy ordering")

    def require(source: str, owner: str, needles: tuple[str, ...]) -> None:
        for needle in needles:
            if needle not in source:
                failures.append(f"{owner} missing causal revision `{needle}`")

    producer_start = postgres_outbox.find("pub async fn insert_verified_rule_outbox_in_tx(")
    producer_end = postgres_outbox.find("pub async fn lock_and_reject_projected_legacy_truth_write_in_tx(")
    dispatch_start = postgres_outbox.find("pub async fn dispatch_verified_rule_with_fence<")
    dispatch_end = postgres_outbox.find("pub struct SyncStatusSnapshot", dispatch_start)
    if producer_start == -1 or producer_end <= producer_start:
        failures.append("PostgreSQL revisioned enqueue owner is missing")
        enqueue = ""
    else:
        enqueue = postgres_outbox[producer_start:producer_end]
    if dispatch_start == -1 or dispatch_end <= dispatch_start:
        failures.append("PostgreSQL revision-aware dispatcher is missing")
        dispatch = ""
    else:
        dispatch = postgres_outbox[dispatch_start:dispatch_end]

    require(enqueue, "PostgreSQL revisioned enqueue", (
        "lock_and_ensure_verified_rules_writable_in_tx(tx,",
        "SELECT MAX(projection_revision) FROM system.sync_outbox",
        "if head != persisted_max",
        "ON CONFLICT (aggregate_key) DO UPDATE",
        "current_revision=system.verified_rule_projection_heads.current_revision + 1",
        "payload_hash, projection_revision, created_at",
        ".bind(revision)",
        ".execute(&mut **tx)",
        "IdempotencyConflict",
    ))
    # Revision reservation must precede the new event insert and both must use
    # the caller's transaction; a separate PG connection would break causality.
    reservation = enqueue.find("INSERT INTO system.verified_rule_projection_heads")
    event_insert = enqueue.find("INSERT INTO system.sync_outbox")
    if reservation == -1 or event_insert <= reservation:
        failures.append("PostgreSQL must reserve the revision before inserting its Outbox event")
    require(postgres_outbox, "PostgreSQL revision contract", (
        "projection_revision: Option<i64>",
        "projection_revision: r.get(\"projection_revision\")",
        "projection_revision IS NULL",
        "LegacyUnordered",
    ))
    require(dispatch, "PostgreSQL causal dispatch", (
        "projection_revision=$8",
        ".bind(event.projection_revision)",
        "projection_revision IS NULL",
        "ORDER BY projection_revision DESC LIMIT 1",
        "SELECT current_revision FROM system.verified_rule_projection_heads",
        "if latest.as_ref().map(|(_, _, revision)| *revision) != head",
        "Some(*revision) == event.projection_revision",
    ))
    if "ORDER BY created_at DESC" in dispatch or "latest_intent_timestamp_unambiguous" in dispatch:
        failures.append("PostgreSQL causal dispatch must not fall back to timestamps/UUIDv4")
    if "matches!(event.projection_revision, Some(revision) if revision > 0)" not in postgres_outbox:
        failures.append("verified-rule lease must reject absent/non-positive producer revision")

    transactional_start = generic_writer.find("pub async fn outbox_emit_many_in_tx(")
    transactional_end = generic_writer.find("pub async fn outbox_emit_one(")
    if transactional_start == -1 or transactional_end <= transactional_start:
        failures.append("generic Outbox transaction writer is missing")
        transactional = ""
    else:
        transactional = generic_writer[transactional_start:transactional_end]
    require(transactional, "generic Outbox writer", (
        'event.target_system == "qdrant" && event.aggregate_type == "rule_instance"',
        "postgres_store::sqlx_outbox_adapter::insert_verified_rule_outbox_in_tx(",
        "continue;",
    ))
    require(generic_writer, "standalone Outbox writer", (
        "verified-rule Qdrant Outbox events require atomic Truth staging",
    ))
    if generic_writer.count("verified-rule Qdrant Outbox events require atomic Truth staging") < 2:
        failures.append("both standalone Outbox emitters must reject verified-rule writes")
    if canonical_writer.count("sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(tx, &events)") < 2:
        failures.append("both canonical verified-rule effects must stage via revisioned Outbox transaction")
    if sqlx_verified_writer.count("lock_and_reject_projected_legacy_truth_write_in_tx(") < 2:
        failures.append("standalone verified-rule Truth upsert and demotion must reject projected identities")
    return failures


# These are physical Neo4j projection MERGE identities, not a fixed website
# recipe, business-step count or a promise that all future operations are known.
# A new MERGE identity must be added to the owner schema and this guard.
NEO4J_UNIQUE_IDENTITIES = frozenset({
    ("Node", "VisaContext", "context_key"),
    ("Node", "Concept", "concept_key"),
    ("Node", "RuleInstance", "rule_instance_id"),
    ("Node", "Page", "url_path"),
    ("Node", "KeywordCluster", "cluster_key"),
    ("Node", "Intent", "intent_key"),
    ("Node", "SERPPattern", "serp_pattern_key"),
    ("Node", "PageBlueprint", "blueprint_key"),
    ("Node", "PageNode", "page_node_key"),
    ("Node", "ContentGap", "content_gap_key"),
    ("Node", "PageBrief", "page_brief_key"),
    ("Relationship", "RECOMMENDS_LINK_TO", "link_recommendation_key"),
    ("Relationship", "CONFLICTS_WITH", "conflict_key"),
})


def neo4j_schema_contract_failures(
    schema: str, connection: str, harness: str,
    e2e: str, impact: str, workflow: str,
    operator_types: str, operator_dispatch: str,
    local_graph_gate: str,
) -> list[str]:
    """Static regression fence for *real* Neo4j identity prerequisites.

    Named DDL, SHOW metadata verification, production connection and an
    existing real-container graph E2E are all required. This check itself
    never connects to Neo4j and never claims to have installed a constraint.
    """
    failures: list[str] = []
    # Parse reviewed data entries, not comments or random Cypher strings:
    # each identity needs its own typed specification and executable DDL.
    entries = re.findall(
        r'ProjectionConstraint\s*\{\s*name:\s*"([^"]+)",\s*'
        r'entity:\s*ConstraintEntity::(Node|Relationship),\s*'
        r'label_or_type:\s*"([^"]+)",\s*'
        r'property:\s*"([^"]+)",\s*'
        r'ddl:\s*"([^"]+)",\s*\}',
        schema,
    )
    found: dict[tuple[str, str, str], str] = {}
    names: set[str] = set()
    for name, entity, label, property_key, ddl in entries:
        identity = (entity, label, property_key)
        if identity in found:
            failures.append(f"Neo4j projection duplicates typed schema specification {identity}")
        found[identity] = name
        if name in names:
            failures.append(f"Neo4j projection reuses constraint name `{name}`")
        names.add(name)
        identifier = r"[A-Za-z_][A-Za-z_0-9]*"
        if entity == "Node":
            template = rf"CREATE CONSTRAINT ({identifier}) IF NOT EXISTS FOR \(n:({identifier})\) REQUIRE n\.({identifier}) IS UNIQUE"
        else:
            template = rf"CREATE CONSTRAINT ({identifier}) IF NOT EXISTS FOR \(\)-\[r:({identifier})\]-\(\) REQUIRE r\.({identifier}) IS UNIQUE"
        parsed_ddl = re.fullmatch(template, ddl)
        if not parsed_ddl or parsed_ddl.groups() != (name, label, property_key):
            failures.append(f"Neo4j `{name}` must install a matching named uniqueness DDL for {identity}")
    for missing in sorted(NEO4J_UNIQUE_IDENTITIES - set(found)):
        failures.append(f"Neo4j projection missing owner uniqueness constraint for {missing}")

    install = schema.split("pub async fn install_projection_schema", 1)
    verify = schema.split("pub async fn verify_projection_schema", 1)
    if len(install) != 2 or len(verify) != 2:
        failures.append("Neo4j owner must expose explicit install and read-only verify schema APIs")
    else:
        installer = install[1].split("pub async fn verify_projection_schema", 1)[0]
        verifier = verify[1].split("#[cfg(test)]", 1)[0]
        normalized_installer = re.sub(r"\s+", "", installer)
        for needle in (
            "for spec in PROJECTION_CONSTRAINTS",
            "graph.run(query(spec.ddl))",
            "verify_projection_schema(graph).await",
        ):
            if re.sub(r"\s+", "", needle) not in normalized_installer:
                failures.append(f"Neo4j schema installer missing `{needle}`")
        for needle in (
            "SHOW CONSTRAINTS", "entityType", "labelsOrTypes", "properties",
            "matches_constraint(spec, found)", "bail!", "PROJECTION_CONSTRAINTS",
        ):
            if needle not in verifier:
                failures.append(f"Neo4j schema verifier missing `{needle}`")
        if "CREATE CONSTRAINT" in verifier or "graph.run(query(spec.ddl))" in verifier:
            failures.append("Neo4j production schema verification must remain read-only")
        shape_required = (
            "found.entity == expected_entity",
            "expected_kinds.contains(&found.kind.as_str())",
        )
        label_exact = (
            "found.labels_or_types == [spec.label_or_type]" in schema or
            all(part in schema for part in (
                "found.labels_or_types.len() == 1",
                "found.labels_or_types[0] == spec.label_or_type",
            ))
        )
        property_exact = (
            "found.properties == [spec.property]" in schema or
            all(part in schema for part in (
                "found.properties.len() == 1",
                "found.properties[0] == spec.property",
            ))
        )
        if not (all(needle in schema for needle in shape_required) and label_exact and property_exact):
            failures.append("Neo4j schema verifier must reject wrong type/entity/label/property")

    connection_fn = connection.split("pub async fn connect_neo4j", 1)
    if len(connection_fn) != 2:
        failures.append("Neo4j production connection factory missing")
    else:
        body = connection_fn[1].split("pub async fn run_cypher", 1)[0]
        if not re.search(r"schema::verify_projection_schema\s*\(\s*&graph\s*\)\s*\.await\s*\?", body):
            failures.append("Neo4j production connect_neo4j must fail closed on absent constraints")
        if "schema::install_projection_schema" in body or "CREATE CONSTRAINT" in body:
            failures.append("Neo4j production connect_neo4j may verify, never auto-install schema")

    bootstrap = harness.split("impl Neo4jHarness", 1)
    if len(bootstrap) != 2:
        failures.append("real Neo4jHarness is missing")
    else:
        body = bootstrap[1].split("impl ", 1)[0]
        install_position = body.find("graph_neo4j::schema::install_projection_schema(&graph)")
        verify_position = body.find("graph_neo4j::schema::verify_projection_schema(&graph)")
        return_position = body.find("Ok(Self {")
        if install_position < 0 or verify_position <= install_position or return_position <= verify_position:
            failures.append("real Neo4jHarness must install and verify constraints before exposing graph writers")
        if 'GenericImage::new("neo4j"' not in body:
            failures.append("Neo4j uniqueness E2E must use the actual Neo4j container")

    test = e2e.split("async fn graph_capability_link_projection_rejects_cross_scope_and_missing_endpoints", 1)
    if len(test) != 2:
        failures.append("real graph_capability Neo4j E2E is missing")
    else:
        body = test[1].split("#[cfg(feature = \"e2e\")]", 1)[0]
        for needle in ("Neo4jHarness::start().await", "tokio::join!", "count_page_nodes", "count_links", "upsert_page_node", "upsert_link_recommendation"):
            if needle not in body:
                failures.append(f"real concurrent Neo4j E2E missing `{needle}`")
        if body.count("tokio::join!(") < 2:
            failures.append("real Neo4j E2E must race both PageNode and link identities")
        for check in (
            "assert_eq!(count_page_nodes(&graph, &concurrent_page_key).await, 1)",
            "assert_eq!(count_links(&graph, &concurrent_link_key).await, 1)",
            "graph_neo4j::schema::verify_projection_schema(&graph)",
            "graph_neo4j::schema::install_projection_schema(&graph)",
            '"DROP CONSTRAINT alegria_pagenode_page_node_key_unique IF EXISTS"',
        ):
            if check not in body:
                failures.append(f"real Neo4j race/historical-duplicate E2E missing `{check}`")

    # A real production deployment must have an executable operator command,
    # not just a test-only installer. The existing temporal_starter has the
    # graph_neo4j dependency and deliberately returns before connecting to
    # Temporal; no new service, job or schema-migrator binary is introduced.
    if not re.search(r"enum Command\s*\{[\s\S]*?\bNeo4jSchemaInstall\s*,", operator_types):
        failures.append("existing temporal_starter must expose neo4j-schema-install command")
    cli_bootstrap = operator_dispatch.split("let command = match command {", 1)
    if len(cli_bootstrap) != 2:
        failures.append("temporal_starter operator migration dispatch is missing")
    else:
        early_dispatch = cli_bootstrap[1].split("let identity =", 1)[0]
        for needle in (
            "Command::Neo4jSchemaInstall =>",
            'std::env::var("NEO4J_URI")',
            'std::env::var("NEO4J_USER")',
            'std::env::var("NEO4J_PASSWORD")',
            "neo4rs_adapter::Graph::new(&uri, &user, &password)?",
            "neo4rs_adapter::schema::install_projection_schema(&graph).await?",
            "neo4rs_adapter::schema::verify_projection_schema(&graph).await?",
            "return Ok(())",
        ):
            if needle not in early_dispatch:
                failures.append(f"operator-only Neo4j migration missing `{needle}` before Temporal connect")
        if "connect_client(" in early_dispatch or "neo4rs_adapter::connect_neo4j(" in early_dispatch:
            failures.append("operator schema installation must avoid Temporal and production verify-first factory")

    # This local smoke gate may provision its OWN disposable compose Neo4j.
    # With BOOTSTRAP_LOCAL_INFRA=0 (operator-supplied production or staging
    # endpoint) it must not silently install/edit live constraints.
    compose_bootstrap = re.search(
        r'if \[\[ "\$BOOTSTRAP_LOCAL_INFRA" == "1" \]\]; then\s*'
        r'if command -v docker[^\n]*; then\s*'
        r'[\s\S]*?compose_cmd up -d postgres neo4j[^\n]*\n'
        r'[\s\S]*?LOCAL_NEO4J_BOOTSTRAPPED=true\s*\n\s*fi\s*\nfi',
        local_graph_gate,
    )
    if not compose_bootstrap or 'LOCAL_NEO4J_BOOTSTRAPPED=false' not in local_graph_gate:
        failures.append("local graph gate may enable auto-DDL only after its own successful Docker compose bootstrap")
    local_gate = local_graph_gate.split('if [[ "$LOCAL_NEO4J_BOOTSTRAPPED" == "true" ]]; then', 1)
    if len(local_gate) != 2:
        failures.append("local graph gate must isolate one-shot Neo4j schema installation from external endpoints")
    else:
        installer = local_gate[1].split("\nfi\n", 1)[0]
        for needle in (
            'CARGO_TARGET_DIR="$CLI_TARGET_DIR"',
            'NEO4J_URI="127.0.0.1:7687"',
            'NEO4J_USER="neo4j"',
            'NEO4J_PASSWORD="neo4j_password"',
            'cargo run -q -p temporal_worker --bin temporal_starter -- neo4j-schema-install',
        ):
            if needle not in installer:
                failures.append(f"local Neo4j graph gate missing safe one-shot installer `{needle}`")
    if local_graph_gate.count("-- neo4j-schema-install") != 1:
        failures.append("local graph gate must have exactly one guarded schema migration")
    if local_graph_gate.find("-- neo4j-schema-install") > local_graph_gate.find("    seo-preflight") >= 0:
        failures.append("local graph gate must install Neo4j schema before preflight")

    graph_suite = impact.split('"graph_query": {', 1)
    if len(graph_suite) != 2:
        failures.append("existing graph_query impact suite is missing")
    else:
        suite = graph_suite[1].split('"vector_projection": {', 1)[0]
        if '"--lib", "graph_capability_", "--locked"' not in suite:
            failures.append("graph_query must execute both real Neo4j graph_capability tests")
        if '"app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs"' not in suite:
            failures.append("graph_query impact map must own concurrent replay test edits")
        if '"app/rust/crates/integration_harness/src/containers.rs"' not in suite:
            failures.append("graph_query impact map must own real Neo4j schema bootstrap edits")
        if '"app/rust/crates/graph_neo4j/"' not in suite:
            failures.append("graph_query must run on Neo4j schema owner edits")
    architecture = workflow.split("  architecture-and-rust:\n", 1)
    if len(architecture) != 2 or "python3 automation/check_reliability_contracts.py" not in architecture[1].split("  product-truth:\n", 1)[0]:
        failures.append("required architecture owner must run Neo4j schema source guard")
    return failures


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "proto_runtime_payload_store.rs":
        sub_dir = path.parent / "proto_runtime_payload_store"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def must_contain(path: Path, needle: str) -> str | None:
    if not path.exists():
        return f"missing file: {path.relative_to(ROOT)}"
    text = read_file_resolved(path)
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def operation_freshness_contract_failures(
    protocol: str, protocol_tests: str, semantic_owner: str,
    postgres_owner: str, runner_tests: str, workflow: str,
) -> list[str]:
    """Keep generic causal freshness in its owner and on the mandatory CI path.

    This is a mutation-sensitive source gate; the Rust tests are the actual
    behavioral proof. Neither a verified provenance string nor this graph
    check authorizes future effectful Temporal execution.
    """
    failures: list[str] = []
    for needle in (
        "pub const PLAN_SCHEMA_REVISION: u16 = 2",
        "pub invalidates: BTreeSet<ArtifactTypeId>",
        "serde_json::to_vec(self)",
        "verify_freshness_dependencies(",
        "UnorderedArtifactHazard",
        "StaleArtifact",
        "UnorderedInvalidations",
        "return Err(PlanError::UnorderedArtifactHazard {",
        "return Err(PlanError::StaleArtifact {",
        "return Err(PlanError::UnorderedInvalidations {",
    ):
        if needle not in protocol:
            failures.append(f"generic operation freshness owner missing `{needle}`")
    if protocol.count("verify_freshness_dependencies(") < 2:
        failures.append("ExecutionPlan::validate must invoke freshness dependency validation")
    if "contract.invalidates" not in protocol:
        failures.append("operation owner invalidations are not inspected during validation")
    if any(site_phase in protocol for site_phase in (
        "RawKnowledgeIngestion", "VerifiedSupport", "SeoPhaseKey",
    )):
        failures.append("generic freshness contract is coupled to legacy SEO site phases")

    for kind in ("StaleArtifact", "UnorderedArtifactHazard", "UnorderedInvalidations"):
        if f"PlanError::{kind}" not in protocol_tests:
            failures.append(f"generic operation freshness Rust tests missing `{kind}` rejection")
    if "invalidates: BTreeSet::new()" not in semantic_owner:
        failures.append("five real pure semantic operations must declare no Truth invalidation")
    if "invalidates: BTreeSet::new()" not in postgres_owner:
        failures.append("PostgreSQL outbox lease maintenance cannot claim Truth invalidation")
    for needle in (
        "real_pure_owners_declare_no_truth_invalidation_under_plan_revision_two",
        "assert!(contract.invalidates.is_empty())",
    ):
        if needle not in runner_tests:
            failures.append(f"real operation runner tests missing `{needle}`")
    architecture = workflow.split("  architecture-and-rust:\n", 1)
    if len(architecture) != 2 or "python3 automation/test_operation_freshness_contract.py" not in architecture[1].split("  product-truth:\n", 1)[0]:
        failures.append("mandatory architecture job must execute freshness mutation tests")
    return failures


def main() -> int:
    failures: list[str] = []
    failures.extend(
        x
        for x in [
            must_contain(PROTO, "message StepContractMeta"),
            must_contain(PROTO, "message StepEnvelope"),
            must_contain(PROTO, "message StringPayload"),
            must_contain(PROTO, "message FactExtractionInputPayload"),
            must_contain(PROTO, "message ExtractedPayloadState"),
            must_contain(PROTO, "message GenerationResultState"),
            must_contain(PROTO, "message RuntimeErrorPayload"),
            must_contain(PROTO, "message ReconcileSummaryPayload"),
            must_contain(PROTO, "message ReconcileTargetReportPayload"),
            must_contain(PROTO, "message VerifyReport"),
            must_contain(PROTO, "message PersistReport"),
            must_contain(PROTO, "message HitlPauseInfo"),
            must_contain(PROTO, "message HitlDecision"),
            must_contain(PROTO, "message HitlTaskContext"),
            must_contain(PROTO, "message HitlResolutionInput"),
            must_contain(PROTO, "message FreshnessReport"),
            must_contain(CONTRACTS_BUILD, "temporal_payloads.proto"),
            must_contain(CONTRACTS_BUILD, "cargo:rerun-if-changed"),
            must_contain(CONTRACTS_LIB, "alegria.temporal.v1.rs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_executions"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_attempts"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.execution_run_blobs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_payload_blobs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.hitl_decisions"),
            must_contain(SCHEMA, "ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS input_payload"),
            must_contain(SCHEMA, "ALTER TABLE pipeline.step_executions DROP COLUMN IF EXISTS result_payload"),
            must_contain(SCHEMA, "UNIQUE (step_name, idempotency_key)"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS system.dead_letter_queue"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.reconcile_runs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.reconcile_actions"),
            must_contain(SCHEMA, "ADD COLUMN IF NOT EXISTS payload_type"),
            must_contain(SCHEMA, "ADD COLUMN IF NOT EXISTS schema_version"),
            must_contain(SCHEMA, "ADD COLUMN IF NOT EXISTS idempotency_key"),
            must_contain(SCHEMA, "idx_sync_outbox_idempotency"),
            must_contain(TEMPORAL_MAIN, "METRICS_PORT"),
            must_contain(METRICS, "activity_duration_seconds"),
            must_contain(METRICS, "workflow_failures_total"),
            must_contain(OUTBOX_BUILDER, "payload_type"),
            must_contain(OUTBOX_BUILDER, "schema_version"),
            must_contain(OUTBOX_BUILDER, "idempotency_key"),
            must_contain(PROTO_RUNTIME_PAYLOAD_STORE, "trait RuntimeProtoPayload"),
            must_contain(PROTO_RUNTIME_PAYLOAD_STORE, "fn encode_runtime_payload"),
            must_contain(PIPELINE_RUNTIME_ADAPTER, "pub use super::proto_runtime_payload_store::"),
            must_contain(GATE, "run_seo_site_build_workflow"),
            must_contain(GATE, "verified_support_bundle blob was not persisted"),
            must_contain(GATE, "seo_page_review_requested"),
            must_contain(GATE, "encode-validation-input"),
        ]
        if x is not None
    )

    if PROTO_RUNTIME_PAYLOAD_STORE.exists():
        adapter_text = PROTO_RUNTIME_PAYLOAD_STORE.read_text(encoding="utf-8")
        if "JsonPayload" in adapter_text:
            failures.append("runtime payload store still references JsonPayload")
    if PROTO.exists():
        proto_text = PROTO.read_text(encoding="utf-8")
        if "message JsonPayload" in proto_text:
            failures.append("temporal proto still defines deprecated JsonPayload")
    if GATE.exists():
        gate_text = GATE.read_text(encoding="utf-8")
        if "alegria.temporal.v1.JsonPayload" in gate_text or "json_payload_hex" in gate_text:
            failures.append("temporal gate still uses JsonPayload-based input path")

    fence_paths = (
        POSTGRES_OUTBOX, POSTGRES_RECONCILE, PROJECTION_DISPATCH,
        OUTBOX_MATERIALIZE, OUTBOX_WORKER, TEMPORAL_RUNTIME_SUPPORT,
    )
    absent = [path for path in fence_paths if not path.is_file()]
    if absent:
        failures.extend(f"missing fenced delivery owner: {path.relative_to(ROOT)}" for path in absent)
    else:
        failures.extend(fence_contract_failures(*(path.read_text(encoding="utf-8") for path in fence_paths)))

    truth_paths = (
        LEGACY_SOURCE_WRITER, LEGACY_RAW_CRAWL_WRITER, CANONICAL_VERIFIED_WRITER,
        SQLX_VERIFIED_WRITER, GENERIC_OUTBOX_WRITER,
        CANONICAL_TRUTH_WRITER, TRUTH_PREFLIGHT,
    )
    absent_truth = [path for path in truth_paths if not path.is_file()]
    if absent_truth:
        failures.extend(
            f"missing fenced Truth owner: {path.relative_to(ROOT)}" for path in absent_truth
        )
    elif POSTGRES_OUTBOX.is_file():
        truth_files = {path: path.read_text(encoding="utf-8") for path in truth_paths}
        failures.extend(truth_writer_fence_failures(
            truth_files, POSTGRES_OUTBOX.read_text(encoding="utf-8"),
        ))

    revision_paths = (
        SCHEMA, REVISION_MIGRATION, POSTGRES_OUTBOX,
        GENERIC_OUTBOX_WRITER, CANONICAL_VERIFIED_WRITER, SQLX_VERIFIED_WRITER,
    )
    absent_revision = [path for path in revision_paths if not path.is_file()]
    if absent_revision:
        failures.extend(
            f"missing causal revision owner: {path.relative_to(ROOT)}" for path in absent_revision
        )
    else:
        failures.extend(causal_revision_contract_failures(
            *(path.read_text(encoding="utf-8") for path in revision_paths)
        ))

    neo4j_paths = (
        NEO4J_SCHEMA_OWNER, NEO4J_CONNECTION_OWNER, NEO4J_REAL_HARNESS,
        NEO4J_REAL_E2E, CI_IMPACT_PLANNER, CI_WORKFLOW,
        NEO4J_OPERATOR_TYPES, NEO4J_OPERATOR_DISPATCH,
        NEO4J_LOCAL_GRAPH_GATE,
    )
    absent_neo4j = [path for path in neo4j_paths if not path.is_file()]
    if absent_neo4j:
        failures.extend(
            f"missing Neo4j uniqueness contract owner: {path.relative_to(ROOT)}"
            for path in absent_neo4j
        )
    else:
        failures.extend(neo4j_schema_contract_failures(
            *(path.read_text(encoding="utf-8") for path in neo4j_paths)
        ))

    freshness_paths = (
        OPERATION_PROTOCOL_OWNER, OPERATION_PROTOCOL_TESTS,
        SEMANTIC_EXTRACTION_OPERATIONS, POSTGRES_RECONCILE,
        SEMANTIC_OPERATION_RUNNER_TESTS, CI_WORKFLOW,
    )
    missing_freshness = [path for path in freshness_paths if not path.is_file()]
    if missing_freshness:
        failures.extend(
            f"missing generic operation freshness owner: {path.relative_to(ROOT)}"
            for path in missing_freshness
        )
    else:
        failures.extend(operation_freshness_contract_failures(
            *(path.read_text(encoding="utf-8") for path in freshness_paths)
        ))

    if failures:
        print("RELIABILITY_CONTRACTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RELIABILITY_CONTRACTS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
