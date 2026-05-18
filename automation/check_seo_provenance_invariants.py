#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SOURCE_PROJECTION = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_source_projection_adapter.rs"
SEO_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
RAW_CRAWL = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"


def main() -> int:
    source_projection = SOURCE_PROJECTION.read_text(encoding="utf-8")
    seo_adapter = SEO_ADAPTER.read_text(encoding="utf-8")
    raw_crawl = RAW_CRAWL.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        "if status == \"verified\" && source_key.is_none()",
        "missing source provenance",
        "source_key.as_deref()",
    ]:
        if needle not in source_projection:
            failures.append(f"source projection missing `{needle}`")

    for needle in [
        "AND r.source_key IS NOT NULL",
        "AND r.source_key <> ''",
        "source_tier = match (source_type.as_str(), trust_level)",
        "source_tier: source_tier.to_string()",
    ]:
        if needle not in seo_adapter:
            failures.append(f"seo adapter missing `{needle}`")

    for needle in [
        "truth_extraction_llm_adapter::extract_rule_candidates",
        "INSERT INTO extracted.rule_candidates",
        "forum",
        "low_trust",
    ]:
        if needle not in raw_crawl:
            failures.append(f"raw crawl provenance missing `{needle}`")
    for forbidden in [
        "source_allows_auto_verify",
        "persist_from_pipeline_state",
        "ensure_extracted_concepts",
    ]:
        if forbidden in raw_crawl:
            failures.append(f"raw crawl provenance still contains legacy `{forbidden}`")

    if failures:
        print("SEO_PROVENANCE_INVARIANTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_PROVENANCE_INVARIANTS: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
