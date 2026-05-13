#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EXEC_PLAN = ROOT / "docs" / "SEO_SUPERSITE_10_10_EXECUTION_PLAN.md"
RUNTIME_SPEC = ROOT / "docs" / "V5_SEO_Runtime_Step_Contracts_Spec.md"
IDENTITY_PLAN = ROOT / "docs" / "V5_SEO_Identity_And_Applicability_Hardening_Plan.md"


def main() -> int:
    failures: list[str] = []

    exec_plan = EXEC_PLAN.read_text(encoding="utf-8")
    runtime_spec = RUNTIME_SPEC.read_text(encoding="utf-8")
    identity_plan = IDENTITY_PLAN.read_text(encoding="utf-8")

    forbidden = "Starting `SeoSiteBuildWorkflow` for a fresh `country_code + visa_type + applicant_profile` can create/resolve context."
    if forbidden in exec_plan:
        failures.append(
            "SEO_SUPERSITE_10_10_EXECUTION_PLAN.md still conflates applicant_profile with truth identity"
        )

    for needle in [
        "truth identity",
        "publishing identity",
        "scope_signature",
    ]:
        if needle not in exec_plan:
            failures.append(f"SEO execution plan missing `{needle}` wording")

    for needle in [
        "profile_applicability_change",
        "scope_change",
        "locale_change",
        "structured per-page trigger metadata",
    ]:
        if needle not in runtime_spec:
            failures.append(f"runtime step spec missing rebuild_detect rule `{needle}`")

    for needle in [
        "conditional",
        "replace_value",
        "add_requirement",
        "applicant_profile",
    ]:
        if needle not in identity_plan:
            failures.append(f"identity hardening plan missing `{needle}`")

    if failures:
        print("SEO_IDENTITY_DOCS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_IDENTITY_DOCS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
