#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "worker.rs"
DLQ = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_dead_letter_adapter.rs"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(WORKER, "DEFAULT_MAX_RETRIES"),
            must_contain(WORKER, "if event.retry_count + 1 >= DEFAULT_MAX_RETRIES"),
            must_contain(WORKER, "mark_failed(pool, event_id, &msg).await?;"),
            must_contain(WORKER, "write_external_dead_letter("),
            must_contain(DLQ, "pub async fn write_external_dead_letter("),
        ]
        if x is not None
    ]

    if failures:
        print("SMOKE_EXHAUSTED_RETRY_TO_DLQ: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SMOKE_EXHAUSTED_RETRY_TO_DLQ: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
