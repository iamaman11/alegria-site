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

    if failures:
        print("CRAWL_PROVENANCE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CRAWL_PROVENANCE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
