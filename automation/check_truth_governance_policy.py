#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
POLICY_DOC = ROOT / "docs" / "V6_Truth_Governance_Policy.md"
WORKING_PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
OWNER_DOC = ROOT / "docs" / "V6_Expert_Truth_Graph_Runtime.md"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
POLICY_ENGINE = ROOT / "app" / "rust" / "crates" / "policies" / "src" / "truth_governance.rs"
RAW_CRAWL = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "raw_crawl_adapter.rs"
TEMPORAL_OPS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations.rs"
CI_VERIFY = ROOT / "automation" / "ci_verify.sh"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []
    if not POLICY_DOC.exists():
        failures.append("missing V6_Truth_Governance_Policy.md")
    else:
        policy = read(POLICY_DOC)
        for needle in [
            "source independence",
            "authority override",
            "freshness",
            "Regex is not allowed as final authority",
        ]:
            if needle not in policy:
                failures.append(f"policy doc missing marker: {needle}")

    working_plan = read(WORKING_PLAN)
    if "V6_Truth_Governance_Policy.md" not in working_plan:
        failures.append("working plan does not reference truth governance policy")

    owner_doc = read(OWNER_DOC)
    if "V6_Truth_Governance_Policy.md" not in owner_doc:
        failures.append("owner doc does not reference truth governance policy")

    schema = read(SCHEMA)
    for needle in [
        "authority_class",
        "independence_group_key",
        "freshness_ttl_days",
        "override_eligible",
    ]:
        if needle not in schema:
            failures.append(f"schema missing governance field: {needle}")

    if not POLICY_ENGINE.exists():
        failures.append("missing truth_governance policy engine")
    else:
        engine = read(POLICY_ENGINE)
        for needle in [
            "non_independent_corroboration",
            "weak_source_corroboration",
            "authority_override@1",
            "override_not_allowed_for_authority_class",
            "freshness_block; freshness_class=",
        ]:
            if needle not in engine:
                failures.append(f"policy engine missing marker: {needle}")

    for path, label in [
        (RAW_CRAWL, "raw crawl adapter"),
        (TEMPORAL_OPS, "temporal truth adjudication"),
    ]:
        text = read(path)
        if "adjudicate_truth_candidates_with_governance" not in text:
            failures.append(f"{label} is not wired to governance adjudication")

    ci_verify = read(CI_VERIFY)
    if "python3 automation/check_truth_governance_policy.py" not in ci_verify:
        failures.append("ci_verify.sh does not include truth governance policy check")

    if failures:
        print("TRUTH_GOVERNANCE_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_GOVERNANCE_POLICY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
