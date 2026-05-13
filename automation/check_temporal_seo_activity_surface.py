#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"

FORBIDDEN_TOKENS = [
    "step_catalog::run_serp_ingest(",
    "step_catalog::run_serp_normalize(",
    "step_catalog::run_opportunity_build(",
    "step_catalog::run_ia_build(",
    "step_catalog::run_link_recommend(",
    "step_catalog::run_global_site_reconcile(",
    "sqlx_seo_adapter::persist_serp_ingest_output(",
    "sqlx_seo_adapter::persist_serp_normalize_output(",
    "sqlx_seo_adapter::persist_opportunity_build_output(",
    "sqlx_seo_adapter::persist_ia_build_output(",
    "sqlx_seo_adapter::persist_link_recommend_output(",
    "sqlx_seo_adapter::persist_global_navigation_from_active_pages(",
    "sqlx_serp_adapter::",
    "dataforseo_serp_adapter",
    "semantic_search_adapter",
    "step_catalog::run_draft_assemble(",
    "step_catalog::run_draft_normalize(",
    "step_catalog::run_content_contract_validate(",
    "step_catalog::run_draft_qa(",
    "sqlx_seo_adapter::load_section_templates(",
    "sqlx_seo_adapter::persist_draft_assemble_output(",
    "sqlx_seo_adapter::persist_draft_normalize_output(",
    "sqlx_seo_adapter::persist_content_contract_validate_output(",
    "sqlx_seo_adapter::persist_draft_qa_output(",
    "editorial_llm_adapter",
    "step_catalog::run_cms_publish(",
    "step_catalog::run_publish_materialize(",
    "step_catalog::run_render_preview_validate(",
    "step_catalog::run_finalize_publish(",
    "sqlx_seo_cms_adapter::persist_cms_publish_output(",
    "sqlx_seo_cms_adapter::load_latest_approval_decision(",
    "sqlx_seo_cms_adapter::persist_publish_materialize_output(",
    "sqlx_seo_cms_adapter::persist_finalize_publish_output(",
]


def main() -> int:
    text = ACTIVITIES.read_text(encoding="utf-8")
    errors = [token for token in FORBIDDEN_TOKENS if token in text]
    if errors:
        for token in errors:
            print(f"FAIL temporal SEO planning cluster leaked back into activities/mod.rs: {token}")
        return 1
    print("OK temporal SEO planning cluster activity surface")
    return 0


if __name__ == "__main__":
    sys.exit(main())
