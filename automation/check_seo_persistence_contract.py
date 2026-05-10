#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
ADAPTER_MOD = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "mod.rs"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
SERP_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_serp_adapter.rs"

PERSISTENCE_FUNCTIONS = [
    "persist_serp_ingest_output",
    "persist_serp_normalize_output",
    "persist_opportunity_build_output",
    "persist_ia_build_output",
    "persist_link_recommend_output",
    "persist_draft_assemble_output",
    "persist_draft_qa_output",
    "persist_rebuild_detect_output",
]

REQUIRED_TABLE_TOUCHES = [
    "serp.query_batches",
    "serp.raw_snapshots",
    "serp.serp_patterns",
    "serp.opportunity_candidates",
    "site.keyword_clusters",
    "site.page_blueprints",
    "site.page_nodes",
    "site.link_recommendations",
    "site.page_briefs",
    "site.page_drafts",
    "monitoring.seo_rebuild_backlog",
    "monitoring.seo_quality_failures",
]


def main() -> int:
    adapter = ADAPTER.read_text(encoding="utf-8") if ADAPTER.exists() else ""
    adapter_mod = ADAPTER_MOD.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    serp_adapter = SERP_ADAPTER.read_text(encoding="utf-8")
    failures: list[str] = []

    if "pub mod sqlx_seo_adapter;" not in adapter_mod:
        failures.append("infrastructure adapter module does not export sqlx_seo_adapter")

    for function in PERSISTENCE_FUNCTIONS:
        if f"pub async fn {function}" not in adapter:
            failures.append(f"missing SEO persistence function `{function}`")
        if f"sqlx_seo_adapter::{function}" not in activities:
            failures.append(f"activity surface does not call `{function}`")

    for table in REQUIRED_TABLE_TOUCHES:
        if table not in adapter:
            failures.append(f"SEO persistence adapter does not touch `{table}`")

    if (
        "payload_json" in serp_adapter
        or "payload_hash" in serp_adapter
        or "serp.crawl_queue.priority" in serp_adapter
        or "(url_norm, priority)" in serp_adapter
    ):
        failures.append("sqlx_serp_adapter still references removed raw/crawl schema fields")
    if "raw_result" not in serp_adapter or "serp.crawl_queue" not in serp_adapter:
        failures.append("sqlx_serp_adapter is not aligned with current SERP schema")

    if failures:
        print("SEO_PERSISTENCE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_PERSISTENCE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
