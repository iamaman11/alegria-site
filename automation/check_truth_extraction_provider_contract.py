#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
TRUTH_LLM = ROOT / "app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs"
SMOKE = ROOT / "automation/smoke_real_provider_minimal_scope.py"
RUNBOOK = ROOT / "docs/OPS_RUNTIME_RUNBOOK.md"
PROD_GATE = ROOT / "docs/OPS_TEMPORAL_PRODUCTION_GATE.md"


def main() -> int:
    truth_llm = TRUTH_LLM.read_text(encoding="utf-8")
    smoke = SMOKE.read_text(encoding="utf-8")
    runbook = RUNBOOK.read_text(encoding="utf-8")
    prod_gate = PROD_GATE.read_text(encoding="utf-8")

    failures: list[str] = []

    truth_needles = [
        "pub fn configured_truth_provider_summary()",
        'std::env::var("SEO_TRUTH_LLM_LOCAL_ENDPOINT")',
        'std::env::var("SEO_LLM_LOCAL_ENDPOINT")',
        'std::env::var("OPENAI_API_KEY")',
        'std::env::var("ANTHROPIC_API_KEY")',
        'std::env::var("GEMINI_API_KEY")',
        'std::env::var("GOOGLE_API_KEY")',
        'std::env::var("OPENAI_SEO_MODEL")',
        'std::env::var("ANTHROPIC_SEO_MODEL")',
        'std::env::var("GEMINI_SEO_MODEL")',
        'std::env::var("SEO_LLM_LOCAL_MODEL")',
    ]
    for needle in truth_needles:
        if needle not in truth_llm:
            failures.append(f"truth_extraction_llm_adapter missing `{needle}`")

    for needle in [
        '"truth_extraction_provider"',
        '"extraction_provider_unavailable"',
        '"provider": "openai"',
        '"provider": "anthropic"',
        '"provider": "gemini"',
        '"provider": "local_compatible"',
    ]:
        if needle not in smoke:
            failures.append(f"smoke_real_provider_minimal_scope.py missing `{needle}`")

    for text, label in [
        (runbook, "OPS_RUNTIME_RUNBOOK.md"),
        (prod_gate, "OPS_TEMPORAL_PRODUCTION_GATE.md"),
    ]:
        for needle in [
            "check_truth_extraction_provider_ready.py",
            "SEO_TRUTH_LLM_LOCAL_ENDPOINT",
            "SEO_LLM_LOCAL_ENDPOINT",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
        ]:
            if needle not in text:
                failures.append(f"{label} missing `{needle}`")

    if failures:
        print("TRUTH_EXTRACTION_PROVIDER_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_EXTRACTION_PROVIDER_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
