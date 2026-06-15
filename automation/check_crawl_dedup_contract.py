#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
RAW = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"


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

    failures: list[str] = []

    for needle in [
        "CREATE TABLE IF NOT EXISTS raw.page_content_aliases",
        "alias_page_id     BIGINT PRIMARY KEY",
        "canonical_page_id BIGINT NOT NULL",
        "content_hash      TEXT NOT NULL",
        "idx_raw_page_content_aliases_canonical",
        "idx_raw_page_content_aliases_hash",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "pub async fn record_content_hash_alias_linkage(",
        "FROM raw.pages",
        "WHERE content_hash = $1",
        "ON CONFLICT (alias_page_id) DO UPDATE",
        "raw.page_content_aliases",
        "source_url",
        "final_url",
        "let _ = record_content_hash_alias_linkage(",
        "source_url: &str",
        "final_url: &str",
        "content_hash: &str",
    ]:
        if needle not in raw:
            failures.append(f"raw_crawl_adapter missing `{needle}`")

    if failures:
        print("CRAWL_DEDUP_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CRAWL_DEDUP_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
