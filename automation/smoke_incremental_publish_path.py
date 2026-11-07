#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CMS = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs"
STATIC = ROOT / "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs"
SCHEMA = ROOT / "app/db/schema.sql"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "sqlx_seo_cms_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_cms_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    cms = read_file_resolved(CMS)
    static = STATIC.read_text(encoding="utf-8")
    schema = SCHEMA.read_text(encoding="utf-8")

    # CMS materialization must build the explicitly approved candidate revision.
    # The older public-snapshot incremental builder is not the publish entrypoint.
    for needle in [
        "static_site_builder_adapter::build_static_site_candidate_incremental(",
        '"build_scope": "candidate_page"',
        '"candidate_isolated_from_public_output": true',
        '"public_snapshot_fallback_allowed": false',
        "site.publish_artifact_entries",
    ]:
        if needle not in cms:
            failures.append(f"cms adapter missing `{needle}`")
    if "static_site_builder_adapter::build_static_site(" in cms:
        failures.append("cms adapter must not fall back to a public full build")
    for needle in [
        "pub async fn build_static_site_candidate_incremental",
        "load_static_site_candidate_snapshot(",
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
