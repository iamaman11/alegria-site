#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
PORTS_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs"
APPLICATION = ROOT / "app/rust/crates/seo_application/src/rebuild_detect.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"


def main() -> int:
    failures: list[str] = []
    schema = SCHEMA.read_text(encoding="utf-8")
    adapter = ADAPTER.read_text(encoding="utf-8")
    ports_adapter = PORTS_ADAPTER.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")

    for needle in [
        "CREATE TABLE IF NOT EXISTS site.page_support_bindings",
        "CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_dependencies",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")
    for needle in [
        "pub async fn resolve_rebuild_impacts(",
        "site.page_support_bindings",
        "monitoring.seo_rebuild_dependencies",
    ]:
        if needle not in adapter:
            failures.append(f"adapter missing `{needle}`")
    for needle in [
        "async fn narrow_rebuild_impacts(",
        "sqlx_seo_adapter::resolve_rebuild_impacts",
        "async fn persist_rebuild_detect_output(",
        "sqlx_seo_adapter::persist_rebuild_detect_output",
    ]:
        if needle not in ports_adapter:
            failures.append(f"seo_ports adapter missing `{needle}`")
    for needle in [
        "narrow_rebuild_impacts(&input.changed_truth_keys)",
        "narrowed_input.page_nodes",
        "persist_rebuild_detect_output(input, &output)",
    ]:
        if needle not in application:
            failures.append(f"seo_application rebuild detect missing `{needle}`")
    for needle in [
        "seo_application::rebuild_detect::execute",
        "run_rebuild_detect_step",
    ]:
        if needle not in activities:
            failures.append(f"activities missing `{needle}`")

    if failures:
        print("DEPENDENCY_AWARE_REBUILD: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("DEPENDENCY_AWARE_REBUILD: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
