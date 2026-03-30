#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS_LIB = ROOT / "app" / "rust" / "crates" / "contracts" / "src" / "lib.rs"
RUNTIME_DOMAIN = ROOT / "app" / "rust" / "crates" / "runtime_models" / "src" / "lib.rs"
RUNTIME_STORAGE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "runtime_storage.rs"
PIPELINE_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_pipeline_runtime_adapter.rs"
PROTO_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"
PIPELINE_RUNTIME = ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "pipeline_runtime.rs"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def must_not_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle in text:
        return f"unexpected `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(CONTRACTS_LIB, "pub mod wire"),
            must_contain(CONTRACTS_LIB, "pub mod condition"),
            must_contain(CONTRACTS_LIB, "pub mod sync"),
            must_contain(CONTRACTS_LIB, "pub mod temporal"),
            must_contain(RUNTIME_DOMAIN, "pub struct ExecutionRun"),
            must_contain(RUNTIME_DOMAIN, "pub struct PersistPipelineState"),
            must_contain(RUNTIME_DOMAIN, "pub struct ExtractedPayload"),
            must_contain(RUNTIME_DOMAIN, "pub struct ReconcileSummary"),
            must_contain(RUNTIME_STORAGE, "pub struct StepExecution"),
            must_contain(RUNTIME_STORAGE, "pub struct StepAttempt"),
            must_contain(PIPELINE_ADAPTER, "pub use runtime_models::"),
            must_contain(PIPELINE_ADAPTER, "pub use super::runtime_storage::"),
            must_contain(PROTO_STORE, "impl RuntimeProtoPayload for ExtractedPayload"),
            must_contain(PROTO_STORE, "impl RuntimeProtoPayload for ReconcileSummary"),
            must_not_contain(PROTO_STORE, "pub struct ExecutionRun {"),
            must_not_contain(PROTO_STORE, "pub struct StepExecution {"),
            must_not_contain(PIPELINE_RUNTIME, "serde_json::Value"),
            must_not_contain(PIPELINE_RUNTIME, "sqlx::"),
        ]
        if x is not None
    ]

    if failures:
        print("RUNTIME_TYPE_FAMILIES: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RUNTIME_TYPE_FAMILIES: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
