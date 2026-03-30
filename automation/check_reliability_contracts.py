#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
CONTRACTS_BUILD = ROOT / "app" / "rust" / "crates" / "contracts" / "build.rs"
CONTRACTS_LIB = ROOT / "app" / "rust" / "crates" / "contracts" / "src" / "lib.rs"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
TEMPORAL_MAIN = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "main.rs"
METRICS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "metrics.rs"
OUTBOX_BUILDER = ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "outbox_builder.rs"
PIPELINE_RUNTIME_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_pipeline_runtime_adapter.rs"
PROTO_RUNTIME_PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"
GATE = ROOT / "automation" / "temporal_production_gate.sh"


def must_contain(path: Path, needle: str) -> str | None:
    if not path.exists():
        return f"missing file: {path.relative_to(ROOT)}"
    text = path.read_text(encoding="utf-8")
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
            must_contain(GATE, "encode-fact-input"),
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

    if failures:
        print("RELIABILITY_CONTRACTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RELIABILITY_CONTRACTS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
