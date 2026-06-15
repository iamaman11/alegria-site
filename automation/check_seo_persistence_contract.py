#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
ADAPTER_MOD = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "mod.rs"
PORTS_ADAPTER = (
    ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "seo_ports_sqlx_adapter.rs"
)
SEO_PORTS = ROOT / "app" / "rust" / "crates" / "seo_ports" / "src" / "lib.rs"
SEO_APPLICATION = ROOT / "app" / "rust" / "crates" / "seo_application" / "src"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
SERP_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_serp_adapter.rs"

PERSISTENCE_OWNERS = {
    "persist_serp_ingest_output": ("planning.rs", "run_serp_ingest"),
    "persist_serp_normalize_output": ("planning.rs", "run_serp_normalize"),
    "persist_opportunity_build_output": ("planning.rs", "run_opportunity_build"),
    "persist_ia_build_output": ("planning.rs", "run_ia_build"),
    "persist_link_recommend_output": ("planning.rs", "run_link_recommend"),
    "persist_draft_assemble_output": ("drafting.rs", "run_draft_assemble"),
    "persist_draft_qa_output": ("drafting.rs", "run_draft_qa"),
    "persist_rebuild_detect_output": ("rebuild_detect.rs", "pub async fn execute"),
}

ACTIVITY_APPLICATION_CALLS = {
    "run_serp_ingest_step": "seo_application::planning::run_serp_ingest",
    "run_serp_normalize_step": "seo_application::planning::run_serp_normalize",
    "run_opportunity_build_step": "seo_application::planning::run_opportunity_build",
    "run_ia_build_step": "seo_application::planning::run_ia_build",
    "run_link_recommend_step": "seo_application::planning::run_link_recommend",
    "run_draft_assemble_step": "seo_application::drafting::run_draft_assemble",
    "run_draft_qa_step": "seo_application::drafting::run_draft_qa",
    "run_rebuild_detect_step": "seo_application::rebuild_detect::execute",
}

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


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "sqlx_seo_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "seo_ports_sqlx_adapter.rs":
        sub_dir = path.parent / "seo_ports_sqlx_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "planning.rs" and "seo_application" in path.parts:
        sub_dir = path.parent / "planning"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    adapter = read_file_resolved(ADAPTER) if ADAPTER.exists() else ""
    adapter_mod = ADAPTER_MOD.read_text(encoding="utf-8")
    ports_adapter = read_file_resolved(PORTS_ADAPTER)
    seo_ports = SEO_PORTS.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    serp_adapter = SERP_ADAPTER.read_text(encoding="utf-8")
    failures: list[str] = []

    if "pub mod sqlx_seo_adapter;" not in adapter_mod:
        failures.append("infrastructure adapter module does not export sqlx_seo_adapter")
    if "pub mod seo_ports_sqlx_adapter;" not in adapter_mod:
        failures.append("infrastructure adapter module does not export seo_ports_sqlx_adapter")

    for function, (owner_module, owner_token) in PERSISTENCE_OWNERS.items():
        owner_path = SEO_APPLICATION / owner_module
        owner = read_file_resolved(owner_path) if owner_path.exists() else ""
        if f"pub async fn {function}" not in adapter:
            failures.append(f"missing SEO persistence function `{function}`")
        if f"async fn {function}" not in seo_ports:
            failures.append(f"seo_ports does not expose `{function}`")
        if f"async fn {function}" not in ports_adapter:
            failures.append(f"seo_ports_sqlx_adapter does not implement `{function}`")
        if f"sqlx_seo_adapter::{function}" not in ports_adapter:
            failures.append(f"seo_ports_sqlx_adapter does not delegate `{function}` to sqlx_seo_adapter")
        if owner_token not in owner or f"{function}(" not in owner:
            failures.append(
                f"seo_application/{owner_module} does not own `{function}` through `{owner_token}`"
            )

    for activity_fn, application_call in ACTIVITY_APPLICATION_CALLS.items():
        if activity_fn not in activities:
            failures.append(f"activity surface is missing `{activity_fn}`")
        if application_call not in activities:
            failures.append(f"activity surface does not call `{application_call}`")

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
