#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RECONCILE = ROOT / "app" / "rust" / "services" / "reconcile" / "src" / "main.rs"
SCHEMA = ROOT / "app" / "db" / "schema.sql"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS system.dead_letter_queue"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.reconcile_runs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.reconcile_actions"),
            must_contain(SCHEMA, "'pending_hitl','done','failed'"),
            must_contain(RECONCILE, "open_dlq"),
            must_contain(RECONCILE, "stale_runs"),
            must_contain(RECONCILE, "pending_hitl_runs"),
            must_contain(RECONCILE, "stuck_steps"),
            must_contain(RECONCILE, "begin_reconcile_run"),
            must_contain(RECONCILE, "append_reconcile_target_action"),
            must_contain(RECONCILE, "append_reconcile_summary_action"),
            must_contain(RECONCILE, "finish_reconcile_run"),
            must_contain(RECONCILE, "runtime fail-safe snapshot"),
        ]
        if x is not None
    ]

    if failures:
        print("RECONCILE_FAILSAFE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RECONCILE_FAILSAFE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
