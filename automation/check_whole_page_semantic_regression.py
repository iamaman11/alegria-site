#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "docs" / "runs" / "whole_page_semantic_fixture_baseline.json"

COMPARE_FIELDS = [
    "page_mode_hint",
    "page_mode_confidence",
    "dominant_layers",
    "country_hints",
    "visa_type_hints",
    "authority_hints",
    "mixed_section_ids",
    "advisory_model_used",
    "advisory_consensus",
    "advisory_prototype_families",
    "uncertainty_flags",
    "reason_codes",
]


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def fixture_index(report: dict) -> dict[str, dict]:
    return {fixture["fixture_id"]: fixture for fixture in report.get("fixtures", [])}


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_whole_page_semantic_regression.py <current-report-path>")
        return 2

    current_path = Path(sys.argv[1]).resolve()
    if not current_path.exists():
        print("WHOLE_PAGE_SEMANTIC_REGRESSION: FAILED")
        print(f"- current report missing: {current_path}")
        return 1
    if not BASELINE.exists():
        print("WHOLE_PAGE_SEMANTIC_REGRESSION: FAILED")
        print(f"- baseline missing: {BASELINE}")
        return 1

    current = load_json(current_path)
    baseline = load_json(BASELINE)
    current_by_id = fixture_index(current)
    baseline_by_id = fixture_index(baseline)

    failures: list[str] = []
    if current.get("artifact_id") != "whole_page_semantic_fixture_report":
        failures.append("artifact_id mismatch in current report")
    if baseline.get("artifact_id") != "whole_page_semantic_fixture_report":
        failures.append("artifact_id mismatch in accepted baseline")
    if sorted(current_by_id) != sorted(baseline_by_id):
        failures.append("fixture set differs from accepted baseline")

    for fixture_id in sorted(set(current_by_id) & set(baseline_by_id)):
        current_fixture = current_by_id[fixture_id]
        baseline_fixture = baseline_by_id[fixture_id]
        if not current_fixture.get("pass", False):
            failures.append(f"{fixture_id}: current report is not passing")
        for field in COMPARE_FIELDS:
            if current_fixture.get(field) != baseline_fixture.get(field):
                failures.append(f"{fixture_id}: {field} drifted from accepted baseline")

    if failures:
        print("WHOLE_PAGE_SEMANTIC_REGRESSION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("WHOLE_PAGE_SEMANTIC_REGRESSION: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
