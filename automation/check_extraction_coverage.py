#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r3_required_rule_type_coverage.json"
FLOW_TESTS = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "seo_flow_tests.rs"
CONTRACT_VALIDATE = (
    ROOT
    / "app"
    / "rust"
    / "crates"
    / "seo_steps"
    / "src"
    / "content_contract_validate_step.rs"
)
QA_STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "draft_qa_step.rs"
SEO_BLOCKS = ROOT / "app" / "rust" / "crates" / "runtime_models" / "src" / "seo_blocks.rs"

REQUIRED_RULE_TYPES = {
    "document_required": "documents",
    "fee_item": "fees",
    "timeline_item": "timing",
    "where_to_apply": "where_to_apply",
    "eligibility_rule": "who_fits",
    "step": "process",
}

ALLOWED_STATUS = {"covered", "blocking_gap", "non_blocking_gap"}
ALLOWED_GAP_CLASSIFICATION = {"none", "blocking", "non_blocking"}


def load_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []

    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("EXTRACTION_COVERAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(load_text(ARTIFACT))
    except Exception as exc:
        print("EXTRACTION_COVERAGE: FAILED")
        print(f"- invalid json: {exc}")
        return 1

    for key in [
        "artifact_id",
        "step",
        "evidence_status",
        "failure_class",
        "status_reason",
        "canonical_smoke",
        "coverage_summary",
        "coverage",
        "contract_checks",
        "updated_at",
    ]:
        if key not in artifact:
            failures.append(f"artifact missing `{key}`")

    if artifact.get("artifact_id") != "r3_required_rule_type_coverage":
        failures.append("artifact_id must equal `r3_required_rule_type_coverage`")
    if artifact.get("step") != "R3.Step2":
        failures.append("step must equal `R3.Step2`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    summary = artifact.get("coverage_summary", {})
    if isinstance(summary, dict):
        for key in [
            "required_rule_types",
            "covered_rule_types",
            "blocking_gaps",
            "non_blocking_gaps",
        ]:
            if key not in summary:
                failures.append(f"coverage_summary missing `{key}`")
    else:
        failures.append("coverage_summary must be an object")

    coverage = artifact.get("coverage", [])
    if not isinstance(coverage, list):
        failures.append("coverage must be a list")
        coverage = []

    seen_rule_types: set[str] = set()
    for entry in coverage:
        if not isinstance(entry, dict):
            failures.append("coverage entries must be objects")
            continue
        for key in [
            "rule_type",
            "section_role",
            "status",
            "gap_classification",
            "smoke_ref",
            "notes",
        ]:
            if key not in entry:
                failures.append(f"coverage entry missing `{key}`")
        rule_type = entry.get("rule_type")
        section_role = entry.get("section_role")
        status = entry.get("status")
        gap_classification = entry.get("gap_classification")
        if rule_type not in REQUIRED_RULE_TYPES:
            failures.append(f"unexpected rule_type `{rule_type}`")
            continue
        seen_rule_types.add(rule_type)
        if REQUIRED_RULE_TYPES[rule_type] != section_role:
            failures.append(
                f"rule_type `{rule_type}` must map to section_role "
                f"`{REQUIRED_RULE_TYPES[rule_type]}`"
            )
        if status not in ALLOWED_STATUS:
            failures.append(
                f"rule_type `{rule_type}` has invalid status `{status}`"
            )
        if gap_classification not in ALLOWED_GAP_CLASSIFICATION:
            failures.append(
                f"rule_type `{rule_type}` has invalid gap_classification "
                f"`{gap_classification}`"
            )
        if status == "covered" and gap_classification != "none":
            failures.append(
                f"covered rule_type `{rule_type}` must use `none` gap_classification"
            )
        if status != "covered" and gap_classification == "none":
            failures.append(
                f"gap rule_type `{rule_type}` must classify the gap as blocking or non_blocking"
            )

    missing = sorted(set(REQUIRED_RULE_TYPES) - seen_rule_types)
    if missing:
        failures.append("coverage missing required rule_types: " + ", ".join(missing))

    flow_tests = load_text(FLOW_TESTS)
    contract_validate = load_text(CONTRACT_VALIDATE)
    qa_step = load_text(QA_STEP)
    seo_blocks = load_text(SEO_BLOCKS)

    for rule_type, section_role in REQUIRED_RULE_TYPES.items():
        if f'"{rule_type}"' not in flow_tests and f"'{rule_type}'" not in flow_tests:
            failures.append(
                f"canonical smoke missing rule_type `{rule_type}`"
            )

    for needle in [
        "missing_required_content_block",
        "unsupported_factual_content_block",
        "missing_required_internal_links",
    ]:
        if needle not in contract_validate:
            failures.append(f"content contract validate missing `{needle}`")

    for needle in [
        "unsupported_factual_fragment",
        "unsupported_claim_ledger_entry",
        "forbidden_serp_as_fact_usage",
        "missing_required_sections",
        "missing_required_metadata",
        "missing_traceability_manifest",
        "missing_claim_ledger",
    ]:
        if needle not in qa_step:
            failures.append(f"draft qa missing `{needle}`")

    for needle in [
        "document_required",
        "fee_item",
        "timeline_item",
        "where_to_apply",
        "eligibility_rule",
        "step",
    ]:
        if needle not in seo_blocks:
            failures.append(f"seo blocks mapping missing `{needle}`")

    if failures:
        print("EXTRACTION_COVERAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "EXTRACTION_COVERAGE: OK "
        f"covered={len(coverage)} required={len(REQUIRED_RULE_TYPES)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
