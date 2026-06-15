#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "app/rust/services/cli_tools/src/main.rs"
CMS = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs"
SCHEMA = ROOT / "app/db/schema.sql"
STATIC = ROOT / "app/rust/services/cli_tools/src/main.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "main.rs" and "cli_tools" in path.parts:
        sub_dir = path.parent / "cli"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "sqlx_seo_cms_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_cms_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    cli = read_file_resolved(CLI)
    cms = read_file_resolved(CMS)
    schema = SCHEMA.read_text(encoding="utf-8")
    static = read_file_resolved(STATIC)
    for needle in [
        "CmsReviewList",
        "CmsReviewShow",
        "CmsApprovePublish",
        "CmsBlock",
        "CmsReopen",
        "cms_review_decision",
        "build_static_site(database_url, &output_dir, &base_url)",
    ]:
        if needle not in cli:
            failures.append(f"cli_tools missing `{needle}`")
    for needle in [
        "CREATE TABLE IF NOT EXISTS site.cms_approval_decisions",
        "CREATE TABLE IF NOT EXISTS site.publish_artifacts",
        "CREATE TABLE IF NOT EXISTS site.expertise_signals",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")
    for needle in [
        "missing_persisted_human_approval",
        "seo_page_published",
        "published_at",
        "PublishArtifact",
        "site.publish_artifacts",
    ]:
        if needle not in cms:
            failures.append(f"CMS adapter missing `{needle}`")
    for needle in [
        "render_content_blocks",
        "content-block--",
        "content_blocks",
    ]:
        if needle not in static:
            failures.append(f"static/headless renderer missing `{needle}`")

    if failures:
        print("HEADLESS_CMS_PUBLISH_FLOW_SMOKE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("HEADLESS_CMS_PUBLISH_FLOW_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
