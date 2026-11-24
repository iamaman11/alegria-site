#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_rebuild_queue_execution_path.json"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
REBUILD_WRITER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter" / "draft_publish_rebuild_persistence.rs"
APPLICATION = ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "rebuild_detect.rs"
DOMAIN = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src" / "rebuild.rs"
PORTS = ROOT / "app" / "rust" / "crates" / "seo_ports" / "src" / "lib.rs"
OBSOLETE_STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "rebuild_detect_step.rs"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build_canonical_cutover.rs"
REBUILD_DISPATCH = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter" / "rebuild_dispatch.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
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
    rebuild_writer = REBUILD_WRITER.read_text(encoding="utf-8")
    rebuild_dispatch = REBUILD_DISPATCH.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    domain = DOMAIN.read_text(encoding="utf-8")
    ports = PORTS.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    workflow = WORKFLOW.read_text(encoding="utf-8")

    for needle in [
        "CREATE TABLE IF NOT EXISTS site.global_rebuild_plan",
        "CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_backlog",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    # Real rebuild detection still writes both durable queues. The former
    # navigation-only `rebuild_plan_count` report was an unused projection
    # metric and is intentionally gone; it was never a dispatch contract.
    # Scope checks to the production writer, not arbitrary sibling modules.
    for needle in [
        "INSERT INTO site.global_rebuild_plan",
        "INSERT INTO monitoring.seo_rebuild_backlog",
        "global_rebuild_plan_key",
        "rebuild_request_key",
        "outbox_emit_many_in_tx(&mut tx, &projection_events)",
        "tx.commit()",
    ]:
        if needle not in rebuild_writer:
            failures.append(f"rebuild writer missing `{needle}`")

    # The dispatcher's operational source of work is monitoring backlog,
    # not the read-only navigation projection or a `rebuild_plan_count` DTO.
    for needle in [
        "WorkflowKind::SeoSiteBuildCanonicalCutover.workflow_type()",
        "FROM monitoring.seo_rebuild_backlog b",
        "WHERE b.status = 'queued'",
        "repo.register_site_build_input(",
        ".start_workflow(",
        "UPDATE monitoring.seo_rebuild_backlog",
        "SET status = 'running'",
    ]:
        if needle not in rebuild_dispatch:
            failures.append(f"rebuild dispatcher missing `{needle}`")

    for needle in [
        "rebuild::detect_rebuild(&narrowed_input)",
        "persist_rebuild_detect_output(input, &",
        "narrowed_input.page_node_keys",
    ]:
        if needle not in application:
            failures.append(f"rebuild application missing `{needle}`")

    for needle in [
        "pub fn detect_rebuild(input: &RebuildDetectRequest)",
        "pub struct RebuildDetectResult",
        "pub enum RebuildVerdict",
    ]:
        if needle not in domain:
            failures.append(f"rebuild domain missing `{needle}`")
    for needle in [
        "input: &RebuildDetectRequest",
        "output: &RebuildDetectResult",
        "page_node_keys: &[String]",
    ]:
        if needle not in ports:
            failures.append(f"rebuild port missing `{needle}`")
    if OBSOLETE_STEP.exists():
        failures.append("obsolete duplicate rebuild detection step still exists")

    for needle in [
        "run_rebuild_detect_step",
        "run_rebuild_detect_step(",
        "seo_application::rebuild_detect::execute(&repo, &request)",
        "reason_package_json: impact.reason_package.to_string()",
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
