#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
ACTIVITIES_RUNTIME = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "runtime.rs"
METRICS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "metrics.rs"
STEP_CHECK = ROOT / "automation" / "check_step_execution_contract.py"
RELIABILITY = ROOT / "automation" / "check_reliability_contracts.py"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        x
        for x in [
            must_contain(PROTO, "message StepContractMeta"),
            must_contain(PROTO, "message StepEnvelope"),
            must_contain(ACTIVITIES_RUNTIME, "pub(crate) async fn execute_step<I, O, F, Fut>("),
            must_contain(ACTIVITIES_RUNTIME, "derive_step_keys("),
            must_contain(ACTIVITIES_RUNTIME, "load_completed_step_result::<O>("),
            must_contain(ACTIVITIES_RUNTIME, "complete_step_execution_typed("),
            must_contain(ACTIVITIES_RUNTIME, "write_dead_letter_typed("),
            must_contain(METRICS, "step_execution_reused_total"),
            must_contain(METRICS, "activity_failures_total"),
            must_contain(STEP_CHECK, "STEP_EXECUTION_CONTRACT: OK"),
            must_contain(RELIABILITY, "RELIABILITY_CONTRACTS: OK"),
        ]
        if x is not None
    ]

    if failures:
        print("STEP_CONTRACT_COMPLETENESS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("STEP_CONTRACT_COMPLETENESS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
