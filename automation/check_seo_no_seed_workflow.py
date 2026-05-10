#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


FORBIDDEN = [
    "default_scope",
    "seed_queries",
    "seed_factual_fragments",
    "Spain tourist visa",
    "spain tourist visa requirements",
]


def main() -> int:
    workflow = WORKFLOW.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    starter = STARTER.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in FORBIDDEN:
        if needle in workflow:
            failures.append(f"SeoSiteBuildWorkflow still contains seed/demo fallback `{needle}`")

    for needle in [
        "load_seo_site_build_input",
        "SeoSiteBuildInputPayload",
        "SerpIngestInputPayload",
    ]:
        if needle not in workflow:
            failures.append(f"SeoSiteBuildWorkflow missing production input needle `{needle}`")

    if "load_seo_site_build_input" not in activities:
        failures.append("Temporal activities missing load_seo_site_build_input activity")
    if "SeoSiteBuildInputPayload" not in starter or "input_payload" not in starter:
        failures.append("temporal_starter does not create typed SEO input blob")

    if failures:
        print("SEO_NO_SEED_WORKFLOW: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_NO_SEED_WORKFLOW: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
