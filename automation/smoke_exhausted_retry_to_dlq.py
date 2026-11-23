#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "worker.rs"
OUTBOX_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_outbox_adapter.rs"
OUTBOX_E2E = ROOT / "app" / "rust" / "crates" / "integration_harness" / "src" / "tests" / "outbox_delivery_tests.rs"


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
            must_contain(WORKER, "mark_failed_with_dead_letter("),
            must_contain(WORKER, "if result == LeaseUpdate::Applied"),
            must_contain(OUTBOX_ADAPTER, "pub async fn mark_failed_with_dead_letter("),
            must_contain(OUTBOX_ADAPTER, "let mut tx = pool.begin().await?;"),
            must_contain(OUTBOX_ADAPTER, "INSERT " + "INTO system.dead_letter_queue"),
            must_contain(OUTBOX_ADAPTER, "tx.commit().await?;"),
            must_contain(OUTBOX_E2E, "outbox_a14_terminal_failure_and_dead_letter_are_atomic"),
        ]
        if x is not None
    ]

    worker_text = WORKER.read_text(encoding="utf-8")
    if "mark_failed(pool, event_id, &msg).await?;" in worker_text:
        failures.append("worker still uses legacy non-atomic terminal failure update")

    if failures:
        print("SMOKE_EXHAUSTED_RETRY_TO_DLQ: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SMOKE_EXHAUSTED_RETRY_TO_DLQ: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
