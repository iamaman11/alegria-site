#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
TRUTH_LLM = ROOT / "app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs"
PLAN = ROOT / "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md"


def main() -> int:
    truth_llm = TRUTH_LLM.read_text(encoding="utf-8")
    plan = PLAN.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "Return a JSON object with top-level key `candidates`.",
        "pub struct TruthRuleCandidate",
        "pub role: String",
        "pub concept_canonical_key: String",
        "pub raw_mention: String",
        "pub params: Value",
        "pub scope: Value",
        "pub severity: String",
        "pub derivation_type: String",
        "pub confidence: f64",
        "pub evidence_section_id: Option<i64>",
        "pub evidence_quote: String",
        "pub span_start: usize",
        "pub span_end: usize",
        "pub uncertainty_flags: Vec<String>",
        '"GEMINI_TRUTH_MODEL"',
        '"GEMINI_SEO_MODEL"',
        '"GEMINI_API_KEY"',
        '"GOOGLE_API_KEY"',
        "response_from_generated_text",
        "validate_candidate(",
        '.get("candidates")',
    ]:
        if needle not in truth_llm:
            failures.append(f"truth_extraction_llm_adapter missing `{needle}`")

    for forbidden in [
        "INSERT INTO verified.rule_instances",
        "publish_admissibility",
        "verification_method",
        "adjudication_reason",
    ]:
        if forbidden in truth_llm:
            failures.append(f"truth_extraction_llm_adapter must not contain `{forbidden}`")

    for needle in [
        "R3.4 Add LLM Extraction Contract",
        "strict JSON-only output",
        "candidate может стать только `structured`, `needs_hitl` или `rejected`, но не `verified`",
    ]:
        if needle not in plan:
            failures.append(f"plan missing `{needle}`")

    if failures:
        print("LLM_EXTRACTION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("LLM_EXTRACTION_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
