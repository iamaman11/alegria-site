#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app" / "db" / "schema.sql"
RUNTIME_HEALTH = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_runtime_health_adapter.rs"
STEP_LEDGER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_step_ledger_adapter.rs"
RECONCILE = ROOT / "app" / "rust" / "services" / "reconcile" / "src" / "main.rs"

PIPELINE_RUN_STATUSES = [
    "pending_hitl",
    "done",
    "failed",
]

STEP_STATUSES = [
    "running",
    "done",
    "failed",
    "pending_hitl",
    "poisoned",
    "dead_letter",
]


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    runtime_health = RUNTIME_HEALTH.read_text(encoding="utf-8")
    step_ledger = STEP_LEDGER.read_text(encoding="utf-8")
    reconcile = RECONCILE.read_text(encoding="utf-8")
    failures: list[str] = []

    for status in PIPELINE_RUN_STATUSES:
        if f"'{status}'" not in schema:
            failures.append(f"schema missing pipeline execution status `{status}`")
    for status in STEP_STATUSES:
        if f"'{status}'" not in schema:
            failures.append(f"schema missing step status `{status}`")
    for status in ["pending_hitl", "done", "failed"]:
        if status not in runtime_health:
            failures.append(f"runtime health adapter missing status `{status}`")
    for status in ["running", "done"]:
        if f"'{status}'" not in step_ledger and f"\"{status}\"" not in step_ledger:
            failures.append(f"step ledger adapter missing status `{status}`")
    if "update_step_execution_failed" not in step_ledger or "fail_step_execution" not in step_ledger:
        failures.append("step ledger adapter missing failed status transition")
    if "pending_hitl_runs" not in reconcile or "is_run_pending_hitl" not in reconcile:
        failures.append("reconcile service does not preserve pending_hitl semantics")

    if failures:
        print("STATUS_MODEL_PARITY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("STATUS_MODEL_PARITY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
