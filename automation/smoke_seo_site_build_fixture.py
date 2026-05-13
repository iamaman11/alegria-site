#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SEO_TEST = ROOT / "app/rust/crates/seo_steps/src/seo_flow_tests.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"


def main() -> int:
    test = SEO_TEST.read_text(encoding="utf-8")
    workflow = WORKFLOW.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "SerpIngestInputPayload",
        "SeoVerifiedFactSupportState",
        "verified_support",
        "traceability_entries",
        "missing_required_internal_links",
    ]:
        if needle not in test:
            failures.append(f"SEO acceptance fixture missing `{needle}`")

    for needle in [
        "load_seo_site_build_input",
        "run_serp_ingest_step",
        "verified_support",
        "publish_mode: \"request_review\"",
        "for (page_index, page_node)",
        "done:published_pages=",
    ]:
        if needle not in workflow:
            failures.append(f"SEO production workflow missing `{needle}`")

    if "ia.page_nodes.first()" in workflow:
        failures.append("SEO production workflow still shortcuts to the first IA page node")

    if failures:
        print("SMOKE_SEO_SITE_BUILD_FIXTURE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SMOKE_SEO_SITE_BUILD_FIXTURE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
