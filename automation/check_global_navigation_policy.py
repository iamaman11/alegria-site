#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_global_navigation_policy.json"
STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "global_site_reconcile_step.rs"
IA = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "ia_build_step.rs"
ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
SCHEMA = ROOT / "app" / "db" / "schema.sql"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "sqlx_seo_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("GLOBAL_NAVIGATION_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("GLOBAL_NAVIGATION_POLICY: FAILED")
        print(f"- invalid json: {exc}")
        return 1

    for key in [
        "artifact_id",
        "step",
        "evidence_status",
        "failure_class",
        "status_reason",
        "smoke_command",
        "contract_checks",
        "updated_at",
    ]:
        if key not in artifact:
            failures.append(f"artifact missing `{key}`")

    if artifact.get("artifact_id") != "r4_global_navigation_policy":
        failures.append("artifact_id must equal `r4_global_navigation_policy`")
    if artifact.get("step") != "R4.Step3":
        failures.append("step must equal `R4.Step3`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    step = STEP.read_text(encoding="utf-8")
    ia = IA.read_text(encoding="utf-8")
    adapter = read_file_resolved(ADAPTER)
    schema = SCHEMA.read_text(encoding="utf-8")

    for needle in [
        "menu_group",
        "breadcrumb_policy",
        "canonical_url_family",
        "path_segments",
        "visa_country_silo",
        "visa_type_silo",
        "visa_type_leaf",
        "global_site_reconcile@1",
    ]:
        if needle not in step:
            failures.append(f"global site reconcile missing `{needle}`")

    for needle in [
        "menu_group: format!",
        "breadcrumb_policy: \"path_segments\"",
        "canonical_url_family: \"visa_country_silo\"",
        "canonical_url_family: \"visa_type_silo\"",
        "canonical_url_family: \"visa_type_leaf\"",
    ]:
        if needle not in ia:
            failures.append(f"ia build missing `{needle}`")

    for needle in [
        "country_hub_to_visa_hub",
        "hub_to_child",
        "child_to_hub",
    ]:
        if needle not in step:
            failures.append(f"global navigation step missing `{needle}`")

    for needle in [
        "INSERT INTO site.navigation_trees",
        "INSERT INTO site.navigation_items",
        "global_rebuild_plan",
        "navigation_reconciled",
        "site.navigation_items",
        "site.navigation_trees",
    ]:
        if needle not in adapter:
            failures.append(f"navigation adapter missing `{needle}`")

    for needle in [
        "CREATE TABLE IF NOT EXISTS site.navigation_trees",
        "CREATE TABLE IF NOT EXISTS site.navigation_items",
        "CREATE TABLE IF NOT EXISTS site.global_rebuild_plan",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    if failures:
        print("GLOBAL_NAVIGATION_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("GLOBAL_NAVIGATION_POLICY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
