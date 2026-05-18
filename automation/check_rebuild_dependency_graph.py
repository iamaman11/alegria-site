#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_rebuild_dependency_graph.json"
SCHEMA = ROOT / "app" / "db" / "schema.sql"
MIGRATION = ROOT / "app" / "db" / "migrations" / "20260515_0004_rebuild_dependency_graph_contract.sql"
ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
DOMAIN = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src" / "rebuild.rs"


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("REBUILD_DEPENDENCY_GRAPH: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("REBUILD_DEPENDENCY_GRAPH: FAILED")
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

    if artifact.get("artifact_id") != "r4_rebuild_dependency_graph":
        failures.append("artifact_id must equal `r4_rebuild_dependency_graph`")
    if artifact.get("step") != "R4.Step1":
        failures.append("step must equal `R4.Step1`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    schema = SCHEMA.read_text(encoding="utf-8")
    migration = MIGRATION.read_text(encoding="utf-8")
    adapter = ADAPTER.read_text(encoding="utf-8")
    domain = DOMAIN.read_text(encoding="utf-8")

    for needle in [
        "'truth_support'",
        "'blueprint'",
        "'section_template'",
        "'keyword_cluster'",
        "'serp_query'",
        "'required_link'",
        "'source_provenance'",
        "'navigation_state'",
    ]:
        if needle not in schema:
            failures.append(f"schema missing dependency type `{needle}`")
        if needle not in migration:
            failures.append(f"migration missing dependency type `{needle}`")

    for needle in [
        "source_provenance",
        "navigation_state",
        "SELECT source_key",
        "dependency_type = 'source_provenance'",
        "dependency_type = 'navigation_state'",
        "matched_dependencies",
        "seed_reason_package",
    ]:
        if needle not in adapter:
            failures.append(f"adapter missing `{needle}`")

    for needle in [
        "classify_trigger",
        "canonical_reason_package",
        "lifecycle_state_for_trigger",
    ]:
        if needle not in domain:
            failures.append(f"rebuild domain missing `{needle}`")

    if failures:
        print("REBUILD_DEPENDENCY_GRAPH: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("REBUILD_DEPENDENCY_GRAPH: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
