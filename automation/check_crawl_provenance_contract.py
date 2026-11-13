#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
RAW = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"
PORTS = ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs"


def read_with_includes(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8")
    import re
    parent = path.parent
    for match in re.finditer(r'include!\("([^"]+)"\);', text):
        include_path = parent / match.group(1)
        if include_path.exists():
            text += "\n" + read_with_includes(include_path)
    return text


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    raw = read_with_includes(RAW)
    ports = read_with_includes(PORTS)
    workflow = WORKFLOW.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        "final_url       TEXT",
        "redirect_chain  JSONB",
        "robots_trace    JSONB",
        "source_observation JSONB",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "pub struct FetchedHtml",
        "pub struct CrawlObservationTrace",
        "source_url: String",
        "final_url: String",
        "redirect_chain: Vec<String>",
        "robots: RobotsDecisionTrace",
        "pub async fn save_crawled_html(",
        "source_url: &str",
        "final_url: &str",
        "redirect_chain: &[String]",
        "robots_trace: &RobotsDecisionTrace",
        "source_observation = $9",
        "source_observation_json",
        "&fetched.source_url",
        "&fetched.final_url",
        "&fetched.redirect_chain",
        "&robots_trace",
        "forum",
        "low_trust",
    ]:
        if needle not in raw and needle not in ports:
            failures.append(f"provenance contract missing `{needle}`")

    # Regression: same-domain documents and >25 unique URLs must not disappear.
    crawl_claim = raw.split("pub async fn claim_pending_crawl_batch(", 1)[-1].split(
        "pub async fn crawl_queue_backlog(", 1
    )[0]
    if "DISTINCT ON" in crawl_claim or "per_domain" in crawl_claim:
        failures.append("crawl claim still discards URLs on domain boundaries")
    for needle in [
        "FOR UPDATE SKIP LOCKED",
        "pub async fn crawl_queue_backlog(",
        "status IN ('pending', 'processing')",
        '"more"',
        '"pending_retry"',
    ]:
        if needle not in (raw + ports):
            failures.append(f"crawl completeness missing `{needle}`")
    for needle in [
        'while crawled.status == "more"',
        "crawled.raw_page_ids.extend(next.raw_page_ids)",
        '"crawl_unresolved_pending_retry"',
    ]:
        if needle not in workflow:
            failures.append(f"durable crawl drain missing `{needle}`")
    phases = [
        '"triple_builder".to_string()',
        '"serp_normalize".to_string()',
        '"global_site_reconcile".to_string()',
        '"manual_truth_prepare".to_string()',
        '"truth_adjudication".to_string()',
        '"graph_admissibility_gate".to_string()',
        '"seo_materialization_gate".to_string()',
    ]
    positions = [workflow.find(phase) for phase in phases]
    if any(index < 0 for index in positions) or positions != sorted(positions):
        failures.append("non-publishable site planning must precede fact verification and publish gates")

    if failures:
        print("CRAWL_PROVENANCE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CRAWL_PROVENANCE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
