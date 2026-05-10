#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RECONCILE_MAIN = ROOT / "app" / "rust" / "services" / "reconcile" / "src" / "main.rs"
HEALTH = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_runtime_health_adapter.rs"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(HEALTH, "WHERE status = 'pending_hitl'"),
            must_contain(HEALTH, "status <> 'pending_hitl'"),
            must_contain(RECONCILE_MAIN, "is_run_pending_hitl"),
            must_contain(RECONCILE_MAIN, "if !is_pending_hitl"),
            must_contain(RECONCILE_MAIN, "pending_hitl_runs"),
        ]
        if x is not None
    ]

    if failures:
        print("SMOKE_PENDING_HITL_NOT_FAILURE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SMOKE_PENDING_HITL_NOT_FAILURE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
