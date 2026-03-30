#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES_DIR = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities"
ACTIVITIES_MOD = ACTIVITIES_DIR / "mod.rs"
ACTIVITIES_RUNTIME = ACTIVITIES_DIR / "runtime.rs"
ACTIVITIES_FACT = ACTIVITIES_DIR / "fact_extraction.rs"
USE_CASES = [
    ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "assemble_context_bundle.rs",
    ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "hitl_queue.rs",
    ROOT / "app" / "rust" / "crates" / "use_cases" / "src" / "neo4j_backwrite_step.rs",
]


def main() -> int:
    failures: list[str] = []

    runtime_text = ACTIVITIES_RUNTIME.read_text(encoding="utf-8")
    required_runtime = [
        "use primitives::errors::{DomainError, ErrorClass};",
        "pub(crate) fn activity_error_from_domain(err: &DomainError) -> ActivityError",
        "Fut: Future<Output = Result<O, DomainError>>",
    ]
    for needle in required_runtime:
        if needle not in runtime_text:
            failures.append(
                f"missing `{needle}` in {ACTIVITIES_RUNTIME.relative_to(ROOT)}"
            )

    fact_text = ACTIVITIES_FACT.read_text(encoding="utf-8")
    required_fact = [
        "DomainError::ValidationFailure",
        "DomainError::ContractViolation",
    ]
    for needle in required_fact:
        if needle not in fact_text:
            failures.append(f"missing `{needle}` in {ACTIVITIES_FACT.relative_to(ROOT)}")

    mod_text = ACTIVITIES_MOD.read_text(encoding="utf-8")
    required_mod = [
        "Self::activity_error_from_domain(&err)",
        "self.execute_step(",
    ]
    for needle in required_mod:
        if needle not in mod_text:
            failures.append(f"missing `{needle}` in {ACTIVITIES_MOD.relative_to(ROOT)}")

    forbidden = [
        "Output = anyhow::Result",
        "anyhow!(",
        "ActivityError::NonRetryable(\n                anyhow",
    ]
    for needle in forbidden:
        if needle in runtime_text:
            failures.append(
                f"forbidden `{needle}` in {ACTIVITIES_RUNTIME.relative_to(ROOT)}"
            )
        if needle in mod_text:
            failures.append(
                f"forbidden `{needle}` in {ACTIVITIES_MOD.relative_to(ROOT)}"
            )

    if failures:
        print("DOMAIN_ERROR_USAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    required_uc_needles = [
        "DomainError",
        "std::result::Result<",
    ]
    forbidden_uc_needles = [
        "use anyhow::Result;",
    ]
    for path in USE_CASES:
        text = path.read_text(encoding="utf-8")
        for needle in required_uc_needles:
            if needle not in text:
                failures.append(f"missing `{needle}` in {path.relative_to(ROOT)}")
        for needle in forbidden_uc_needles:
            if needle in text:
                failures.append(f"forbidden `{needle}` in {path.relative_to(ROOT)}")

    if failures:
        print("DOMAIN_ERROR_USAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("DOMAIN_ERROR_USAGE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
