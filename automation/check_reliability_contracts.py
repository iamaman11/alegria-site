#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
CONTRACTS_BUILD = ROOT / "app" / "rust" / "crates" / "contracts" / "build.rs"
CONTRACTS_LIB = ROOT / "app" / "rust" / "crates" / "contracts" / "src" / "lib.rs"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
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

    # A fenced call owns terminal acknowledgement itself. Both consumers must
    # branch on quarantine BEFORE their generic retry/dead-letter branches.
    if not re.search(
        r"\bdispatch_leased_event\s*\(\s*pool\s*,\s*event\s*,?\s*\)",
        worker_materialize,
    ):
        failures.append("outbox materialize wrapper must invoke the leased projection dispatcher")
    if "dispatch_event(pool, &event)" not in worker:
        failures.append("outbox worker must pass its claimed event/PG pool to materialize wrapper")

    for name, consumer in (("outbox-worker", worker), ("temporal-runtime", temporal)):
        fenced_call = consumer.find("dispatch_event(pool, &event)") if name == "outbox-worker" else consumer.find("dispatch_leased_event(")
        if fenced_call == -1:
            failures.append(f"{name} bypasses the leased projection dispatcher")
        if "AcknowledgedByFence" not in consumer:
            failures.append(f"{name} has no fenced acknowledgement disposition")
        fenced_branch = consumer[fenced_call:] if fenced_call != -1 else ""
        quarantine_arm = re.search(
            r"Err\s*\(\s*(\w+)\s*\)\s+if\s+\1\.stage\s*==\s*(?:\w+::)*DispatchStage::Quarantine\s*=>",
            fenced_branch,
        )
        generic_error = re.search(r"\bErr\s*\(\s*\w+\s*\)\s*=>", fenced_branch)
        if not quarantine_arm or not generic_error or quarantine_arm.start() > generic_error.start():
            failures.append(f"{name} must quarantine unknown provider results before ordinary retry")
        elif (body := match_arm_body(fenced_branch, quarantine_arm.end())) is None:
            failures.append(f"{name} quarantine arm has no inspectable guarded body")
        elif re.search(r"\bmark_(?:done|retry|failed_with_dead_letter)\s*\(", body):
            failures.append(f"{name} must not mutate Outbox status in the quarantine arm")
    return failures


def truth_writer_fence_failures(files: dict[Path, str], postgres_outbox: str) -> list[str]:
    """Protect the full set of known producer paths, not just RPC consumers."""
    failures: list[str] = []
    if "pub async fn lock_and_reject_projected_legacy_truth_write_in_tx" not in postgres_outbox:
        failures.append("PG owner must reject projected non-atomic legacy Truth mutation")
    if "NonAtomicLegacyProjection" not in postgres_outbox:
        failures.append("PG owner must classify non-atomic legacy projected writes")
    expected = {
        LEGACY_SOURCE_WRITER: ("lock_and_reject_projected_legacy_truth_write_in_tx", 1),
        LEGACY_RAW_CRAWL_WRITER: ("lock_and_reject_projected_legacy_truth_write_in_tx", 2),
        CANONICAL_VERIFIED_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 1),
        SQLX_VERIFIED_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 3),
        GENERIC_OUTBOX_WRITER: ("lock_and_ensure_verified_rules_writable_in_tx", 1),
    }
    for path, (call, minimum) in expected.items():
        if files[path].count(call) < minimum:
            failures.append(
                f"{path.relative_to(ROOT)} is missing a mandatory verified-rule Truth fence"
            )
    if "verified-rule Qdrant Outbox events require atomic Truth staging" not in files[GENERIC_OUTBOX_WRITER]:
        failures.append("standalone verified-rule Outbox writes must be prohibited")
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

    if failures:
        print("RELIABILITY_CONTRACTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RELIABILITY_CONTRACTS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
