#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app" / "db" / "schema.sql"
ACTIVITIES_RUNTIME = (
    ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "runtime.rs"
)
PIPELINE_STORAGE = ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "pipeline_runtime.rs"
PIPELINE_STORAGE_BOUNDARY_FILES = [
    ROOT
    / "app"
    / "rust"
    / "crates"
    / "infrastructure"
    / "src"
    / "adapters"
    / "sqlx_pipeline_runtime_adapter.rs",
    ROOT
    / "app"
    / "rust"
    / "crates"
    / "infrastructure"
    / "src"
    / "adapters"
    / "sqlx_step_ledger_adapter.rs",
]


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def must_contain_any(paths: list[Path], needle: str) -> str | None:
    text = "\n".join(path.read_text(encoding="utf-8") for path in paths if path.exists())
    if needle not in text:
        joined = ", ".join(str(path.relative_to(ROOT)) for path in paths)
        return f"missing `{needle}` in [{joined}]"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_executions"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_attempts"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.execution_run_blobs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.step_payload_blobs"),
            must_contain(SCHEMA, "CREATE TABLE IF NOT EXISTS pipeline.hitl_decisions"),
            must_contain(SCHEMA, "ALTER TABLE pipeline.step_executions DROP COLUMN IF EXISTS result_payload"),
            must_contain(SCHEMA, "UNIQUE (step_name, idempotency_key)"),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn begin_step_execution("),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn begin_step_attempt("),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn finish_step_attempt("),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn write_step_payload_blob_typed"),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn load_completed_step_result"),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn complete_step_execution_typed"),
            must_contain_any(PIPELINE_STORAGE_BOUNDARY_FILES, "pub async fn fail_step_execution("),
            must_contain(ACTIVITIES_RUNTIME, "pub(crate) async fn execute_step<I, O, F, Fut>("),
            must_contain(ACTIVITIES_RUNTIME, "pipeline_storage::begin_step_execution("),
            must_contain(
                ACTIVITIES_RUNTIME,
                "pipeline_storage::load_completed_step_result::<O>(",
            ),
            must_contain(
                ACTIVITIES_RUNTIME,
                "pipeline_storage::complete_step_execution_typed(",
            ),
        ]
        if x is not None
    ]

    if failures:
        print("STEP_EXECUTION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("STEP_EXECUTION_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
