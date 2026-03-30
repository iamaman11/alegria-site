#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RECONCILE_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_reconcile_adapter.rs"
RECONCILE_MAIN = ROOT / "app" / "rust" / "services" / "reconcile" / "src" / "main.rs"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(RECONCILE_ADAPTER, "status = 'processing'"),
            must_contain(RECONCILE_ADAPTER, "locked_until IS NULL OR locked_until <= now()"),
            must_contain(RECONCILE_ADAPTER, "status = 'pending'"),
            must_contain(RECONCILE_ADAPTER, "reset stale processing lease"),
            must_contain(RECONCILE_ADAPTER, "status = 'failed'"),
            must_contain(RECONCILE_ADAPTER, "requeue failed reconcile candidates"),
            must_contain(RECONCILE_MAIN, "append_reconcile_target_action("),
        ]
        if x is not None
    ]

    if failures:
        print("SMOKE_STALE_OUTBOX_RECLAIM: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SMOKE_STALE_OUTBOX_RECLAIM: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
