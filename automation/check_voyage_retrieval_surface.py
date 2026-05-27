#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
POLICY = ROOT / "docs" / "V6_Voyage_Retrieval_Policy.md"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    starter = read(STARTER)
    registry = read(REGISTRY)
    ops = read(OPS)
    plan = read(PLAN)
    policy = read(POLICY)
    failures: list[str] = []

    for needle in [
        "apply_qdrant: bool",
        "VoyageClient",
        "VoyageEmbeddingOptions",
        "VOYAGE_CONTEXT_MODEL",
        "VOYAGE_RERANK_MODEL",
        "connect_qdrant",
        "raw_chunks_4",
        "raw_chunks_ctx",
        "kb.qdrant_points",
        "qdrant_point_id_v1(\"ontology\", \"concept\", &concept_key)",
        "\"ontology_voyage@1\"",
    ]:
        if needle not in starter:
            failures.append(f"temporal_starter missing `{needle}`")

    for needle in [
        "Voyage/Qdrant ontology materialization",
        "apply_qdrant",
        "V6_Voyage_Retrieval_Policy.md",
        "raw_chunks_4",
        "raw_chunks_ctx",
    ]:
        if needle not in ops:
            failures.append(f"OPS runbook missing `{needle}`")

    if "Voyage/Qdrant ontology materialization are executable now" not in registry:
        failures.append("support registry does not claim live ontology retrieval materialization")

    if "`Voyage/Qdrant` ontology retrieval projection" not in plan:
        failures.append("working plan missing promoted ontology retrieval projection wording")

    for needle in [
        "voyage-4-large",
        "voyage-context-3",
        "rerank-2.5",
        "raw_chunks_4",
        "raw_chunks_ctx",
        "whole_page_advisory_prototypes",
    ]:
        if needle not in policy:
            failures.append(f"Voyage retrieval policy missing `{needle}`")

    if failures:
        print("VOYAGE_RETRIEVAL_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VOYAGE_RETRIEVAL_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
