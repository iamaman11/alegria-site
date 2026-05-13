#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
STEP_CATALOG = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "step_catalog.rs"

FORBIDDEN_TOKENS = [
    "fn run_serp_ingest(",
    "fn run_serp_normalize(",
    "fn run_opportunity_build(",
    "fn run_ia_build(",
    "fn run_link_recommend(",
    "fn run_global_site_reconcile(",
    "fn run_draft_assemble(",
    "fn run_draft_normalize(",
    "fn run_content_contract_validate(",
    "fn run_draft_qa(",
    "fn run_cms_publish(",
    "fn run_rebuild_detect(",
    "fn run_publish_materialize(",
    "fn run_render_preview_validate(",
    "fn run_finalize_publish(",
]


def main() -> int:
    text = STEP_CATALOG.read_text(encoding="utf-8")
    errors = [token for token in FORBIDDEN_TOKENS if token in text]
    if errors:
        for token in errors:
            print(f"FAIL migrated SEO wrapper still present in step_catalog.rs: {token}")
        return 1
    print("OK temporal step_catalog SEO surface")
    return 0


if __name__ == "__main__":
    sys.exit(main())
