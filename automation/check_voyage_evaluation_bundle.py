#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "voyage_evaluation_bundle.json"


def main() -> int:
    payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    failures: list[str] = []

    if payload.get("status") not in {"PASS", "PENDING_CREDENTIALS"}:
        failures.append("status must be PASS or PENDING_CREDENTIALS")

    provider = payload.get("provider", {})
    for field, expected in [
        ("embeddings_model", "voyage-4-large"),
        ("context_model", "voyage-context-3"),
        ("rerank_model", "rerank-2.5"),
    ]:
        if provider.get(field) != expected:
            failures.append(f"provider.{field} must be `{expected}`")

    expected_collections = {
        "raw_chunks_4",
        "raw_chunks_ctx",
        "kb_canonical_4",
        "verified_rules_voyage4",
        "editorial_topics_voyage4",
        "seo_keyword_clusters_voyage4",
        "whole_page_advisory_prototypes",
    }
    collections = set(payload.get("collections", []))
    missing = sorted(expected_collections - collections)
    if missing:
        failures.append(f"missing collections: {', '.join(missing)}")

    samples = payload.get("samples", {})
    for field in [
        "russian_retrieval",
        "contextualized_chunk_recall",
        "canonical_mapping_fallback",
        "draft_support_rerank",
        "planning_cluster_merge",
    ]:
        if field not in samples:
            failures.append(f"missing sample field `{field}`")

    if failures:
        print("VOYAGE_EVALUATION_BUNDLE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VOYAGE_EVALUATION_BUNDLE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
