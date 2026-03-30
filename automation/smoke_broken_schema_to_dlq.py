#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "runtime.rs"
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
            must_contain(RUNTIME, "ErrorClass::ContractViolation"),
            must_contain(RUNTIME, "ErrorClass::ValidationFailure"),
            must_contain(RUNTIME, "ErrorClass::ForeignKeyViolation"),
            must_contain(RUNTIME, "ErrorClass::ConflictViolation"),
            must_contain(RUNTIME, "write_dead_letter_typed("),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS system.dead_letter_queue"),
            must_contain(SCHEMA, "payload_type      TEXT"),
            must_contain(SCHEMA, "payload_bytes     BYTEA"),
            must_contain(SCHEMA, "payload_hash      TEXT"),
        ]
        if x is not None
    ]

    if failures:
        print("SMOKE_BROKEN_SCHEMA_TO_DLQ: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SMOKE_BROKEN_SCHEMA_TO_DLQ: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
