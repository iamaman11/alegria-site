#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / "app/rust/crates/seo_steps/src/draft_qa_step.rs"
PUBLISH = ROOT / "app/rust/crates/seo_steps/src/cms_publish_step.rs"
PAGE_PUBLISH = (
    ROOT
    / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/page_publish.rs"
)


def main() -> int:
    qa = QA.read_text(encoding="utf-8")
    publish = PUBLISH.read_text(encoding="utf-8")
    page_publish = PAGE_PUBLISH.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "unsupported_factual_fragment",
        "forbidden_serp_as_fact_usage",
        "missing_required_internal_links",
        "SeoPublishBlockerState",
        "required_next_action",
    ]:
        if needle not in qa:
            failures.append(f"draft_qa_step missing publish gate `{needle}`")

    for needle in [
        "human_approval_required",
        "review_requested",
    ]:
        if needle not in publish:
            failures.append(f"cms_publish_step missing human gate `{needle}`")

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
        "PagePublishLoopOutcome::Blocked(format!(",
        "published_before_block",
        "blocked_publish_gate_status()",
    ]:
        if needle not in page_publish:
            failures.append(f"canonical page publish loop missing blocker propagation `{needle}`")

    if "if page_blocked {\n            continue;" in page_publish:
        failures.append(
            "canonical page publish loop still skips a blocked page and can report normal completion"
        )

    if failures:
        print("SEO_PUBLISH_GATES: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_PUBLISH_GATES: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
