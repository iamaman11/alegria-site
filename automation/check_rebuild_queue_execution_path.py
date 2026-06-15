#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_rebuild_queue_execution_path.json"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
APPLICATION = ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "rebuild_detect.rs"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "sqlx_seo_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("REBUILD_QUEUE_EXECUTION_PATH: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("REBUILD_QUEUE_EXECUTION_PATH: FAILED")
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

    if artifact.get("artifact_id") != "r4_rebuild_queue_execution_path":
        failures.append("artifact_id must equal `r4_rebuild_queue_execution_path`")
    if artifact.get("step") != "R4.Step2":
        failures.append("step must equal `R4.Step2`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    schema = SCHEMA.read_text(encoding="utf-8")
    adapter = read_file_resolved(ADAPTER)
    application = APPLICATION.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    workflow = WORKFLOW.read_text(encoding="utf-8")

    for needle in [
        "CREATE TABLE IF NOT EXISTS site.global_rebuild_plan",
        "CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_backlog",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "INSERT INTO site.global_rebuild_plan",
        "INSERT INTO monitoring.seo_rebuild_backlog",
        "global_rebuild_plan_key",
        "rebuild_request_key",
        "rebuild_plan_count",
    ]:
        if needle not in adapter:
            failures.append(f"adapter missing `{needle}`")

    for needle in [
        "execute_rebuild_detect",
        "persist_rebuild_detect_output(input, &",
        "narrowed_input.page_nodes",
    ]:
        if needle not in application:
            failures.append(f"rebuild application missing `{needle}`")

    for needle in [
        "run_rebuild_detect_step",
        "run_rebuild_detect_step(",
    ]:
        if needle not in activities:
            failures.append(f"activities missing `{needle}`")
    if "run_rebuild_detect_step" not in workflow:
        failures.append("workflow missing run_rebuild_detect_step")

    if failures:
        print("REBUILD_QUEUE_EXECUTION_PATH: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("REBUILD_QUEUE_EXECUTION_PATH: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
