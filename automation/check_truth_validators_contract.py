#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PRIMITIVES = ROOT / "app/rust/crates/primitives/src/truth_candidates.rs"
RAW_CRAWL = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "raw_crawl_adapter.rs":
        sub_dir = path.parent / "raw_crawl_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    primitives = PRIMITIVES.read_text(encoding="utf-8")
    raw_crawl = read_file_resolved(RAW_CRAWL)
    failures: list[str] = []

    for needle in [
        "pub fn validate_truth_candidate(",
        '"structured"',
        '"needs_hitl"',
        '"rejected"',
        '"invalid_role"',
        '"missing_evidence_quote"',
        '"invalid_evidence_span"',
        '"missing_numeric_params"',
        '"missing_range_bounds"',
        '"freshness_or_temporality_ambiguous"',
        "role_specific_completeness_checks(",
        "FEE_ITEM",
        "TIMELINE_ITEM",
        "WHERE_TO_APPLY",
    ]:
        if needle not in primitives:
            failures.append(f"truth_candidates validator missing `{needle}`")

    for needle in [
        "validate_truth_candidate(",
        "validator:",
        "epistemic_status = EXCLUDED.epistemic_status",
        ".bind(&validation.epistemic_status)",
    ]:
        if needle not in raw_crawl:
            failures.append(f"raw_crawl_adapter missing validator integration `{needle}`")

    if "VALUES (" in raw_crawl and "'candidate'" in raw_crawl:
        failures.append("raw_crawl_adapter still hardcodes candidate epistemic_status in insert values")

    if failures:
        print("TRUTH_VALIDATORS_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_VALIDATORS_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
