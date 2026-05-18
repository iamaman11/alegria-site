#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
RAW = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"
PORTS = ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs"


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    raw = RAW.read_text(encoding="utf-8")
    ports = PORTS.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        "final_url       TEXT",
        "redirect_chain  JSONB",
        "robots_trace    JSONB",
        "source_observation JSONB",
        "source_domain     TEXT NOT NULL DEFAULT ''",
        "next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now()",
        "idx_serp_crawl_queue_domain_ready",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "pub struct RobotsDecisionTrace",
        "pub struct CrawlObservationTrace",
        "pub async fn evaluate_robots_policy(",
        "pub async fn fetch_html(",
        "pub async fn mark_crawl_retry(",
        "pub fn should_retry_http_status(",
        "pub fn should_retry_crawl_attempt(",
        "redirect_chain: Vec<String>",
        "robots_trace: &RobotsDecisionTrace",
        "source_observation = $9",
        "redirect_chain = $7",
        "robots_trace = $8",
        "source_observation_json",
        "redirect_chain_json",
        "robots_trace_json",
        "fetch_error: Option<String>",
        "matched_rule: Option<String>",
        "next_attempt_at = now() + make_interval(secs => $3::int)",
    ]:
        if needle not in raw:
            failures.append(f"raw_crawl_adapter missing `{needle}`")

    for needle in [
        "evaluate_robots_policy(&item.url).await",
        "save_crawled_html(",
        "&fetched.source_url",
        "&fetched.final_url",
        "&fetched.redirect_chain",
        "&robots_trace",
        "robots disallow:",
        "should_retry_http_status(fetched.status_code)",
        "should_retry_crawl_attempt(item.attempt_count)",
        "mark_crawl_retry(",
    ]:
        if needle not in ports:
            failures.append(f"crawl ingest path missing `{needle}`")

    if failures:
        print("CRAWLER_INVARIANTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CRAWLER_INVARIANTS: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
