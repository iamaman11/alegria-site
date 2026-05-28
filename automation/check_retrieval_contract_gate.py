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
    "retrieval_capability_required": bool,
    "canonical_vector_retrieval_required": bool,
    "contextual_raw_chunk_retrieval_required": bool,
    "voyage_rerank_required": bool,
    "voyage_embeddings_ready": bool,
    "voyage_contextualized_ready": bool,
    "voyage_rerank_ready": bool,
    "qdrant_ready": bool,
    "qdrant_collection_contract_ready": bool,
    "projection_blocked": bool,
    "required_collections": list,
}

ALLOWED_STATUS = {
    "pass",
    "warn",
    "blocked_retrieval_contract",
    "blocked_missing_collection",
    "blocked_stale_collection",
    "blocked_projection_incomplete",
    "blocked_provider_capability",
}


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_retrieval_contract_gate.py <report_json>")
        return 2
    report = Path(sys.argv[1])
    if not report.exists():
        print("RETRIEVAL_CONTRACT_GATE_SCHEMA: FAILED")
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
    if payload.get("status") not in ALLOWED_STATUS:
        failures.append(f"status must be one of {sorted(ALLOWED_STATUS)}")

    required_collections = payload.get("required_collections", [])
    if isinstance(required_collections, list):
        expected_names = {
            "raw_chunks_4",
            "raw_chunks_ctx",
            "kb_canonical_4",
            "verified_rules_voyage4",
            "editorial_topics_voyage4",
            "seo_keyword_clusters_voyage4",
            "whole_page_advisory_prototypes",
        }
        names = {
            item.get("collection_name")
            for item in required_collections
            if isinstance(item, dict)
        }
        missing = sorted(expected_names - names)
        if missing:
            failures.append(f"required_collections missing: {', '.join(missing)}")

    if failures:
        print("RETRIEVAL_CONTRACT_GATE_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(f"RETRIEVAL_CONTRACT_GATE_SCHEMA: OK status={payload['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
