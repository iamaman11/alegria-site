#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


REQUIRED_TOP_LEVEL = {
    "artifact_id": str,
    "status": str,
    "context_key": str,
    "normalized_profile": str,
    "graph_capability_required": bool,
    "neo4j_sync_required": bool,
    "graph_query_required": bool,
    "graph_gds_required": bool,
    "neo4j_ready": bool,
    "graph_query_ready": bool,
    "graph_gds_ready": bool,
    "graph_projection_contract_ready": bool,
    "graph_contract_status": str,
    "required_graph_projections": list,
}

ALLOWED_STATUS = {
    "pass",
    "warn",
    "blocked_provider_capability",
    "blocked_missing_projection",
    "blocked_stale_projection",
    "blocked_projection_incomplete",
    "blocked_graph_contract",
}


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_graph_contract_gate.py <report_json>")
        return 2
    report = Path(sys.argv[1])
    if not report.exists():
        print("GRAPH_CONTRACT_GATE_SCHEMA: FAILED")
        print(f"- missing {report}")
        return 1

    payload = json.loads(report.read_text(encoding="utf-8"))
    failures: list[str] = []

    for key, expected_type in REQUIRED_TOP_LEVEL.items():
        value = payload.get(key)
        if value is None:
            failures.append(f"missing `{key}`")
            continue
        if not isinstance(value, expected_type):
            failures.append(f"`{key}` must be {expected_type.__name__}")

    if payload.get("artifact_id") != "seo_preflight":
        failures.append("artifact_id must equal `seo_preflight`")
    if payload.get("graph_contract_status") not in ALLOWED_STATUS:
        failures.append(f"graph_contract_status must be one of {sorted(ALLOWED_STATUS)}")
    if payload.get("status") not in ALLOWED_STATUS.union(
        {
            "blocked_retrieval_contract",
            "blocked_missing_collection",
            "blocked_stale_collection",
            "blocked_provider_capability",
        }
    ):
        failures.append("status has unsupported value for graph gate")

    expected_names = {
        "keyword_cluster",
        "serp_pattern",
        "page_blueprint",
        "page_node",
        "content_gap",
        "link_recommendation",
        "page_brief",
    }
    required_graph_projections = payload.get("required_graph_projections", [])
    if isinstance(required_graph_projections, list):
        names = {
            item.get("projection_name")
            for item in required_graph_projections
            if isinstance(item, dict)
        }
        missing = sorted(expected_names - names)
        if missing:
            failures.append(
                f"required_graph_projections missing: {', '.join(missing)}"
            )

    if failures:
        print("GRAPH_CONTRACT_GATE_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "GRAPH_CONTRACT_GATE_SCHEMA: OK "
        f"graph_contract_status={payload['graph_contract_status']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
