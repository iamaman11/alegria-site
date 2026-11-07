#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CMS = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs"
STATIC = ROOT / "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs"
SNAPSHOT = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_static_site_adapter.rs"
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
    snapshot = SNAPSHOT.read_text(encoding="utf-8")
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
    # A new isolated release directory cannot contain only the changed page:
    # its manifest and sitemap include previously published dependencies.
    candidate_body = static.split("pub async fn build_static_site_candidate_incremental(", 1)
    if len(candidate_body) != 2:
        failures.append("candidate release builder is absent")
    else:
        candidate_body = candidate_body[1].split("#[cfg(test)]", 1)[0]
        if "write_static_artifacts(output_dir, &artifacts)?" not in candidate_body:
            failures.append("candidate release does not materialize a complete site tree")
        if "build_incremental_from_snapshot(" in candidate_body:
            failures.append("isolated candidate release cannot use partial-only output")
        if "build_static_artifacts(&snapshot.pages, &snapshot.links, base_url)?" not in candidate_body:
            failures.append("candidate release must include published dependencies")
    if "AND p.scope_signature = (" not in snapshot or "WHERE target.page_node_key = $1" not in snapshot:
        failures.append("candidate release includes published pages from foreign scopes")
    # Replaying an older published revision can change its updated_at. The
    # public snapshot and candidate dependencies must follow the committed
    # CMS pointer, never choose the most recently touched revision.
    if snapshot.count("r.revision_id = p.current_revision_id") != 2:
        failures.append("public snapshot and candidate dependencies must use committed revision pointers")
    if "ORDER BY published_revision.updated_at" in snapshot:
        failures.append("public revision must not be selected by mutable updated_at")
    if "r.revision_status = 'published'" not in snapshot:
        failures.append("public snapshots must exclude non-published revisions")
    if "outbox_emit_many_in_tx(" not in cms or "rollback_revision_id" not in cms:
        failures.append("publishing must commit revision pointer and outbox in one transaction")
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
