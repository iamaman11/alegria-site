#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app" / "db" / "schema.sql"


REQUIRED_TABLES = [
    "site.keyword_clusters",
    "site.page_nodes",
    "site.page_blueprints",
    "site.page_briefs",
    "site.page_drafts",
    "site.cms_pages",
    "site.cms_page_revisions",
    "site.cms_publish_events",
    "site.seo_hitl_tasks",
    "site.link_recommendations",
    "site.cannibalization_conflicts",
    "site.content_gaps",
    "site.search_features",
    "site.section_templates",
    "serp.query_batches",
    "serp.serp_patterns",
    "serp.serp_pattern_observations",
    "serp.competitor_pages",
    "serp.competitor_section_patterns",
    "serp.opportunity_candidates",
    "monitoring.seo_metric_snapshots",
    "monitoring.seo_freshness_alerts",
    "monitoring.seo_rebuild_backlog",
    "monitoring.seo_quality_failures",
]

REQUIRED_NEEDLES = [
    "scope_signature",
    "UNIQUE (scope_signature, dominant_intent, canonical_url_path)",
    "qa_verdict IN",
    "publish_ready",
    "rebuild_required",
    "CHECK (source_page_key <> target_page_key)",
    "REFERENCES site.page_nodes(page_node_key)",
]


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    failures: list[str] = []
    for table in REQUIRED_TABLES:
        if f"CREATE TABLE IF NOT EXISTS {table}" not in schema:
            failures.append(f"missing SEO table `{table}`")
    for needle in REQUIRED_NEEDLES:
        if needle not in schema:
            failures.append(f"missing SEO schema contract `{needle}`")
    if failures:
        print("SEO_SCHEMA_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_SCHEMA_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
