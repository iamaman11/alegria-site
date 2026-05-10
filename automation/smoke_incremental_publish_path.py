#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CMS = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs"
STATIC = ROOT / "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs"
SCHEMA = ROOT / "app/db/schema.sql"


def main() -> int:
    failures: list[str] = []
    cms = CMS.read_text(encoding="utf-8")
    static = STATIC.read_text(encoding="utf-8")
    schema = SCHEMA.read_text(encoding="utf-8")

    for needle in [
        "build_static_site_incremental",
        "SEO_PUBLISH_ALLOW_FULL_REBUILD_FALLBACK",
        '"build_scope"',
        "site.publish_artifact_entries",
    ]:
        if needle not in cms:
            failures.append(f"cms adapter missing `{needle}`")
    for needle in [
        "pub async fn build_static_site_incremental",
        "write_partial_artifacts",
    ]:
        if needle not in static:
            failures.append(f"static builder missing `{needle}`")
    for needle in [
        "CREATE TABLE IF NOT EXISTS site.publish_artifact_entries",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    if failures:
        print("INCREMENTAL_PUBLISH_PATH: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("INCREMENTAL_PUBLISH_PATH: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
