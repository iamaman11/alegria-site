#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PRIMITIVES = ROOT / "app/rust/crates/primitives/src/truth_candidates.rs"
RAW_CRAWL = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"
PLAN = ROOT / "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md"


def main() -> int:
    primitives = PRIMITIVES.read_text(encoding="utf-8")
    raw_crawl = RAW_CRAWL.read_text(encoding="utf-8")
    plan = PLAN.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "pub fn adjudicate_truth_candidates(",
        '"non_structured_input"',
        '"freshness_or_completeness_block"',
        '"contradictory_structured_candidates',
        '"single_source_requires_corroboration',
        '"cross_source_consensus@1"',
        '"verified"',
        '"needs_hitl"',
        '"not_admissible"',
        '"admissible"',
        'source_tiers_evaluated_as_signals_only=',
    ]:
        if needle not in primitives:
            failures.append(f"truth adjudication module missing `{needle}`")

    if 'candidate.epistemic_status != "structured"' not in primitives:
        failures.append("adjudication does not enforce structured-only input")

    for needle in [
        "adjudicate_persisted_rule_candidates(",
        "load_persisted_candidates_for_adjudication(",
        "upsert_verified_rule_instance_from_candidate(",
        "demote_verified_semantic_slot(",
        '"disputed"',
        '"needs_hitl"',
        "semantic_rule_instance_id(",
        "pick_canonical_candidate(",
    ]:
        if needle not in raw_crawl:
            failures.append(f"raw_crawl_adapter missing runtime adjudication wiring `{needle}`")

    for needle in [
        "R3.6 Add Truth Adjudication",
        "source-tier-is-signal-only",
        "contradiction handling",
    ]:
        if needle not in plan:
            failures.append(f"execution plan missing `{needle}`")

    if failures:
        print("TRUTH_ADJUDICATION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_ADJUDICATION_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
