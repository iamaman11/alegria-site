#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
MIGRATIONS = ROOT / "app/db/migrations"

REQUIRED_FIELDS = [
    "input_payload",
    "verified_support_bundle",
    "extracted_payload",
    "generation_result",
    "content_block_plan",
    "draft_normalize_output",
    "content_contract_validation",
    "publish_materialize_output",
    "render_preview_validation",
    "finalize_publish_output",
    "verify_report",
    "persist_report",
    "errors",
]


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    combined = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(MIGRATIONS.glob("*.sql"))
    )
    failures: list[str] = []

    for field_name in REQUIRED_FIELDS:
        schema_needle = f"'{field_name}'"
        if schema_needle not in schema:
            failures.append(f"schema.sql missing execution_run_blobs field `{field_name}`")
        if schema_needle not in combined:
            failures.append(f"migrations missing execution_run_blobs field `{field_name}`")

    if "execution_run_blobs_field_name_check" not in combined:
        failures.append("migrations missing execution_run_blobs_field_name_check contract")

    if failures:
        print("EXECUTION_RUN_BLOB_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("EXECUTION_RUN_BLOB_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
