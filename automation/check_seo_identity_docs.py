#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EXEC_PLAN = ROOT / "docs" / "SEO_SUPERSITE_10_10_EXECUTION_PLAN.md"
RUNTIME_SPEC = ROOT / "docs" / "V5_SEO_Runtime_Step_Contracts_Spec.md"
IDENTITY_PLAN = ROOT / "docs" / "V5_SEO_Identity_And_Applicability_Hardening_Plan.md"
INDEX_DOC = ROOT / "docs" / "INDEX.md"
V6_OWNER = ROOT / "docs" / "V6_Expert_Truth_Graph_Runtime.md"
V6_PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
SUPERSEDED_PLAN = ROOT / "docs" / "SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md"
SUPPORT_REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"


def main() -> int:
    failures: list[str] = []

    exec_plan = EXEC_PLAN.read_text(encoding="utf-8")
    runtime_spec = RUNTIME_SPEC.read_text(encoding="utf-8")
    identity_plan = IDENTITY_PLAN.read_text(encoding="utf-8")
    index_doc = INDEX_DOC.read_text(encoding="utf-8")
    v6_owner = V6_OWNER.read_text(encoding="utf-8")
    v6_plan = V6_PLAN.read_text(encoding="utf-8")
    superseded_plan = SUPERSEDED_PLAN.read_text(encoding="utf-8")
    support_registry = SUPPORT_REGISTRY.read_text(encoding="utf-8")

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
        "SeoSiteBuildCanonicalCutoverWorkflow",
        "references below to `SeoSiteBuildWorkflow` describe the historical runtime shape",
    ]:
        if needle not in exec_plan:
            failures.append(
                f"SEO execution plan missing active-vs-historical workflow framing `{needle}`"
            )

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

    for needle in [
        "executed by `SeoSiteBuildCanonicalCutoverWorkflow`",
        "superseded-reference",
    ]:
        if needle not in index_doc:
            failures.append(f"INDEX missing workflow identity framing `{needle}`")

    for needle in [
        "accepted canonical site-build flow",
        "executed by `SeoSiteBuildCanonicalCutoverWorkflow`",
    ]:
        if needle not in v6_owner:
            failures.append(f"V6 owner doc missing workflow identity framing `{needle}`")

    if "canonical `SeoSiteBuildWorkflow` value stream" in v6_owner:
        failures.append("V6 owner doc still equates the canonical value stream with legacy `SeoSiteBuildWorkflow`")

    for needle in [
        "executed by `SeoSiteBuildCanonicalCutoverWorkflow`",
        "**Current version:** `6.",
    ]:
        if needle not in v6_plan:
            failures.append(f"V6 working plan missing updated workflow identity framing `{needle}`")

    if "one `SeoSiteBuildWorkflow` run" in v6_plan:
        failures.append("V6 working plan still describes the canonical 56-step flow as one `SeoSiteBuildWorkflow` run")

    for needle in [
        "Current forward-path note",
        "SeoSiteBuildCanonicalCutoverWorkflow",
    ]:
        if needle not in superseded_plan:
            failures.append(f"Superseded expert gap plan missing explicit historical framing `{needle}`")

    if "является production SEO orchestration path" in superseded_plan:
        failures.append("Superseded expert gap plan still states legacy `SeoSiteBuildWorkflow` as current production path")

    for needle in [
        "executed by `SeoSiteBuildCanonicalCutoverWorkflow`",
        "support processes",
    ]:
        if needle not in support_registry:
            failures.append(f"Support registry missing workflow identity framing `{needle}`")

    if failures:
        print("SEO_IDENTITY_DOCS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_IDENTITY_DOCS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
