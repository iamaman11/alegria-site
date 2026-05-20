#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    starter = read(STARTER)
    registry = read(REGISTRY)
    ops = read(OPS)
    plan = read(PLAN)
    failures: list[str] = []

    for needle in [
        "apply_qdrant: bool",
        "VoyageClient",
        "connect_qdrant",
        "ensure_default_dense_collection",
        "upsert_embedding_points",
        "kb.qdrant_points",
        "qdrant_point_id_v1(\"ontology\", \"concept\", &concept_key)",
        "\"ontology_voyage@1\"",
    ]:
        if needle not in starter:
            failures.append(f"temporal_starter missing `{needle}`")

    for needle in [
        "Voyage/Qdrant ontology materialization",
        "apply_qdrant",
    ]:
        if needle not in ops:
            failures.append(f"OPS runbook missing `{needle}`")

    if "Voyage/Qdrant ontology materialization are executable now" not in registry:
        failures.append("support registry does not claim live ontology retrieval materialization")

    if "`Voyage/Qdrant` ontology retrieval projection" not in plan:
        failures.append("working plan missing promoted ontology retrieval projection wording")

    if failures:
        print("VOYAGE_RETRIEVAL_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VOYAGE_RETRIEVAL_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
