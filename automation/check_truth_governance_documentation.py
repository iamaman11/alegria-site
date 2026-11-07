#!/usr/bin/env python3
"""Legacy V6 prose parity gate. Retain until Issue #8 M2/M4 cutover evidence."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
POLICY_DOC = ROOT / "docs" / "V6_Truth_Governance_Policy.md"
WORKING_PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
OWNER_DOC = ROOT / "docs" / "V6_Expert_Truth_Graph_Runtime.md"
CI_VERIFY = ROOT / "automation" / "ci_verify.sh"


def main() -> int:
    failures: list[str] = []
    if not POLICY_DOC.exists():
        failures.append("missing V6_Truth_Governance_Policy.md")
    else:
        policy = POLICY_DOC.read_text(encoding="utf-8")
        for needle in [
            "source independence",
            "authority override",
            "freshness",
            "Regex is not allowed as final authority",
        ]:
            if needle not in policy:
                failures.append(f"policy doc missing marker: {needle}")

    # Preserve original cross-document assertions while prose is still active.
    working_plan = WORKING_PLAN.read_text(encoding="utf-8")
    if "V6_Truth_Governance_Policy.md" not in working_plan:
        failures.append("working plan does not reference truth governance policy")

    owner_doc = OWNER_DOC.read_text(encoding="utf-8")
    if "V6_Truth_Governance_Policy.md" not in owner_doc:
        failures.append("owner doc does not reference truth governance policy")

    ci_verify = CI_VERIFY.read_text(encoding="utf-8")
    if "python3 automation/check_truth_governance_documentation.py" not in ci_verify:
        failures.append("ci_verify.sh omits legacy truth documentation check")

    if failures:
        print("TRUTH_GOVERNANCE_DOCUMENTATION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_GOVERNANCE_DOCUMENTATION: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
