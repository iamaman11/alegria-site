#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / "app/rust/crates/seo_steps/src/draft_qa_step.rs"
QA_POLICY = ROOT / "app/rust/crates/seo_domain/src/draft_quality.rs"
CONTRACT_ADAPTER = ROOT / "app/rust/crates/seo_steps/src/content_contract_validate_step.rs"
CONTRACT_POLICY = ROOT / "app/rust/crates/seo_domain/src/content_contract.rs"
PUBLISH = ROOT / "app/rust/crates/seo_steps/src/cms_publish_step.rs"
REVISION = ROOT / "app/rust/crates/seo_domain/src/cms_revision.rs"
RENDER_VALIDATE = ROOT / "app/rust/crates/seo_steps/src/render_preview_validate_step.rs"
RENDER_POLICY = ROOT / "app/rust/crates/seo_domain/src/render_preview.rs"
PAGE_PUBLISH = (
    ROOT
    / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/page_publish.rs"
)


def main() -> int:
    qa = QA.read_text(encoding="utf-8")
    qa_policy = QA_POLICY.read_text(encoding="utf-8")
    contract_adapter = CONTRACT_ADAPTER.read_text(encoding="utf-8")
    contract_policy = CONTRACT_POLICY.read_text(encoding="utf-8")
    publish = PUBLISH.read_text(encoding="utf-8")
    revision = REVISION.read_text(encoding="utf-8")
    render_validate = RENDER_VALIDATE.read_text(encoding="utf-8")
    render_adapter = render_validate.split("#[cfg(test)]", 1)[0]
    render_policy = RENDER_POLICY.read_text(encoding="utf-8")
    page_publish = PAGE_PUBLISH.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "unsupported_factual_fragment",
        "forbidden_serp_as_fact_usage",
        "pub fn evaluate(",
        "required_next_action",
    ]:
        if needle not in qa_policy:
            failures.append(f"draft_quality domain missing publish gate `{needle}`")
    for needle in ["SeoPublishBlockerState", "draft_quality::evaluate(&DraftQuality {", "violation.reason_code()", "violation.required_next_action()"]:
        if needle not in qa:
            failures.append(f"draft QA wire adapter missing policy invocation/mapping `{needle}`")
    for needle in ["unsupported_factual_fragment", "forbidden_serp_as_fact_usage"]:
        if needle in qa.split("#[cfg(test)]", 1)[0]:
            failures.append(f"draft QA wire adapter duplicates domain gate `{needle}`")

    if "validate_for_publish(input)" not in qa:
        failures.append("draft QA must invoke the shared content-contract gate")
    if "seo_domain::content_contract" not in contract_adapter:
        failures.append("content-contract adapter must map typed domain policy")
    if "missing_required_internal_links" not in contract_policy:
        failures.append("content-contract domain must own the required-link publish gate")
    if "missing_required_internal_links" in qa.split("#[cfg(test)]", 1)[0]:
        failures.append("draft QA must not duplicate required-link blocker policy")

    for needle in ["CmsRevisionState::decide", "CmsPublishIntent::parse", "cms_publication_blockers(", "CmsPublicationGate {"]:
        if needle not in publish:
            failures.append(f"cms publish adapter missing domain invocation `{needle}`")
    for needle in ["human_approval_required", "CmsPublishGateViolation::HumanApprovalRequired", "review_requested", "publish_blocked", "seo_page_publish_blocked"]:
        if needle not in revision:
            failures.append(f"cms_revision domain missing publication policy/transition outcome `{needle}`")
    if "human_approval_required" in publish.split("#[cfg(test)]", 1)[0]:
        failures.append("cms publish adapter must not duplicate human-approval policy")

    forbidden = [
        'publish_status: "approved"',
        'publish_status = "approved"',
        'publish_status: "published"',
        'publish_status = "published"',
    ]
    for needle in forbidden:
        if needle in publish:
            failures.append(f"cms_publish_step can silently publish via `{needle}`")

    for needle in [
        "PagePublishLoopOutcome::Blocked {",
        "published_before_block",
        "published_pages,",
        "blocked_publish_gate_status()",
        "PageAdmissibility::Blocked(detail)",
        "first_blocked = Some(reason)",
        "if let Some(reason) = first_blocked",
    ]:
        if needle not in page_publish:
            failures.append(f"canonical page publish loop missing blocker propagation `{needle}`")

    # Check both the pre-editorial rejection and the aggregate return; a
    # rejected current page cannot be mistaken for retired history.
    pre_editorial = page_publish.split("PageAdmissibility::Blocked(detail)", 1)[-1].split(
        "PageAdmissibility::Eligible", 1
    )[0]
    aggregate = page_publish.split("if let Some(reason) = first_blocked", 1)[-1]
    if "first_blocked = Some(reason)" not in pre_editorial or "continue;" not in pre_editorial:
        failures.append("pre-editorial blockers must survive skipped page work")
    if "PagePublishLoopOutcome::Blocked {" not in aggregate or "published_pages," not in aggregate:
        failures.append("aggregate outcome must preserve blockers and actual publication count")

    if "if page_blocked {\n            continue;" in page_publish:
        failures.append(
            "canonical page publish loop still skips a blocked page and can report normal completion"
        )

    for needle in [
        "page.rendered_link_count < page.required_link_count",
        '"unsafe_active_content_marker"',
        '"unsafe_script_structure"',
        '"<link rel=\\\"canonical\\\" href=\\\""',
        '"<meta name=\\\"description\\\" content=\\\""',
        '"<html lang=\\\""',
    ]:
        if needle not in render_policy:
            failures.append(f"render_preview domain missing security/product gate `{needle}`")
    for needle in [
        "validate_render_preview(&pages)",
        "required_next_action()",
        "rejects_active_content_markers",
        "rejects_non_json_ld_script_elements",
        "maps_domain_findings_to_stable_temporal_blockers_once",
    ]:
        if needle not in render_validate:
            failures.append(f"render_preview wire adapter missing typed mapping/test `{needle}`")
    for duplicate in [
        "page.rendered_link_count < page.required_link_count",
        '"unsafe_active_content_marker"',
        '"unsafe_script_structure"',
        '"<html lang=\\\""',
    ]:
        if duplicate in render_adapter:
            failures.append(f"render_preview wire adapter duplicates domain policy `{duplicate}`")

    if failures:
        print("SEO_PUBLISH_GATES: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_PUBLISH_GATES: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
