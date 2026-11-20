#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "docs" / "runs" / "truth_certification_local_ci_baseline.json"

COMPARE_FIELDS = [
    "verified_rule_set",
    "needs_hitl_set",
    "rejected_set",
    "contradiction_groups",
    "completeness_failures",
    "draft_verdict",
    "publish_verdict",
    "projection_verdict",
    "rebuild_impact_emitted",
]


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def fixture_index(report: dict) -> dict[str, dict]:
    return {fixture["fixture_id"]: fixture for fixture in report.get("fixtures", [])}


def monotonic_no_truth_projection_block(baseline: dict, current: dict) -> bool:
    """Only accept a stricter fail-closed projection policy, never a weaker one.

    Old immutable certification evidence may record graph projections as
    permitted even when truth was empty. The current canonical flow prevents
    graph/planning without admissible truth. This is an *allowed tightening*,
    not an approval to edit the historical report or change facts/HITL/drafts.
    """
    return (
        baseline.get("projection_verdict") == "allow"
        and current.get("projection_verdict") == "blocked"
        and baseline.get("verified_rule_set") == []
        and current.get("verified_rule_set") == []
        and current.get("draft_verdict") == "blocked"
        and current.get("publish_verdict") == "not_attempted"
        and current.get("rebuild_impact_emitted") is False
    )


def monotonic_competitor_candidate_review(fixture_id: str, baseline: dict, current: dict) -> bool:
    """Accept one explicitly reviewed tightening of candidate validation.

    An agency's service charge cannot establish a consular fee. The newer
    validator holds the claim for human review *before* source corroboration,
    whereas the historical baseline held it during source governance.
    Neither outcome may verify the fee, draft it, or publish it.
    """
    if fixture_id != "competitor_claim_non_verified":
        return False
    previous = baseline.get("needs_hitl_set")
    latest = current.get("needs_hitl_set")
    legacy_reason = (
        "|single_source_requires_corroboration; "
        "source_tiers=low_trust; authority_classes=agency"
    )
    if (
        not isinstance(previous, list)
        or len(previous) != 1
        or not isinstance(previous[0], str)
        or not previous[0].endswith(legacy_reason)
        or latest != [
            previous[0][: -len(legacy_reason)] + "|candidate_validation_requires_hitl"
        ]
    ):
        return False
    # The live fixture's source governance must still identify an agency.
    source_records = current.get("source_governance_snapshot", [])
    if not isinstance(source_records, list) or not any(
        isinstance(record, str) and "|niche_agency|agency|" in record
        for record in source_records
    ):
        return False
    return (
        baseline.get("verified_rule_set") == current.get("verified_rule_set") == []
        and baseline.get("rejected_set") == current.get("rejected_set") == []
        and current.get("draft_verdict") == "blocked"
        and current.get("publish_verdict") == "not_attempted"
        and current.get("projection_verdict") == "blocked"
        and current.get("rebuild_impact_emitted") is False
    )


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_truth_certification_regression.py <current-report-path>")
        return 2

    current_path = Path(sys.argv[1]).resolve()
    if not current_path.exists():
        print("TRUTH_CERTIFICATION_REGRESSION: FAILED")
        print(f"- current report missing: {current_path}")
        return 1
    if not BASELINE.exists():
        print("TRUTH_CERTIFICATION_REGRESSION: FAILED")
        print(f"- baseline missing: {BASELINE}")
        return 1

    current = load_json(current_path)
    baseline = load_json(BASELINE)
    current_by_id = fixture_index(current)
    baseline_by_id = fixture_index(baseline)

    failures: list[str] = []
    if sorted(current_by_id) != sorted(baseline_by_id):
        failures.append("fixture set differs from accepted baseline")

    for fixture_id in sorted(set(current_by_id) & set(baseline_by_id)):
        current_fixture = current_by_id[fixture_id]
        baseline_fixture = baseline_by_id[fixture_id]
        if not current_fixture.get("pass", False):
            failures.append(f"{fixture_id}: current certification report is not passing")
        if (
            not current_fixture.get("verified_rule_set")
            and current_fixture.get("projection_verdict") != "blocked"
        ):
            failures.append(f"{fixture_id}: zero admissible truth must block projection")
        for field in COMPARE_FIELDS:
            if current_fixture.get(field) == baseline_fixture.get(field):
                continue
            if field == "projection_verdict" and monotonic_no_truth_projection_block(
                baseline_fixture, current_fixture
            ):
                continue
            if field == "needs_hitl_set" and monotonic_competitor_candidate_review(
                fixture_id, baseline_fixture, current_fixture
            ):
                continue
            failures.append(f"{fixture_id}: {field} drifted from accepted baseline")

    if failures:
        print("TRUTH_CERTIFICATION_REGRESSION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_CERTIFICATION_REGRESSION: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
