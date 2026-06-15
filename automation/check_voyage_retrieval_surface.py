#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter"
OPS_RUNTIME = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations"
SEO_STEPS_CANONICAL = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "canonical_mapping_step.rs"
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
POLICY = ROOT / "docs" / "V6_Voyage_Retrieval_Policy.md"
PROD_GATE = ROOT / "automation" / "temporal_production_gate.sh"
RETRIEVAL_GATE = ROOT / "automation" / "run_retrieval_contract_gate.sh"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def read_surface(path: Path) -> str:
    if path.is_dir():
        return "\n".join(
            child.read_text(encoding="utf-8")
            for child in sorted(path.rglob("*.rs"))
        )
    return read(path)


def main() -> int:
    starter = read_surface(STARTER)
    ops_runtime = read_surface(OPS_RUNTIME)
    seo_steps_canonical = read(SEO_STEPS_CANONICAL)
    registry = read(REGISTRY)
    ops = read(OPS)
    plan = read(PLAN)
    policy = read(POLICY)
    prod_gate = read(PROD_GATE)
    retrieval_gate = read(RETRIEVAL_GATE)
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
        "RETRIEVAL_CAPABILITY_REQUIRED",
    ]:
        if needle not in policy:
            failures.append(f"Voyage retrieval policy missing `{needle}`")

    for needle in [
        "run_retrieval_contract_gate",
        "RETRIEVAL_CAPABILITY_REQUIRED=true",
        "CANONICAL_VECTOR_RETRIEVAL_REQUIRED=true",
        "CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED=true",
        "VOYAGE_RERANK_REQUIRED=true",
    ]:
        if needle not in prod_gate:
            failures.append(f"temporal_production_gate missing `{needle}`")

    for needle in [
        "RETRIEVAL_CAPABILITY_REQUIRED",
        "CANONICAL_VECTOR_RETRIEVAL_REQUIRED",
        "CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED",
        "VOYAGE_RERANK_REQUIRED",
        "seo-preflight",
    ]:
        if needle not in retrieval_gate:
            failures.append(f"retrieval contract gate missing `{needle}`")

    for needle in [
        "resolve_canonical_vector_mappings(output.mappings).await?;",
        "CANONICAL_VECTOR_RETRIEVAL_REQUIRED",
        "VOYAGE_RERANK_REQUIRED",
        "\"kb_canonical_4\"",
    ]:
        if needle not in ops_runtime:
            failures.append(f"operations canonical mapping path missing `{needle}`")

    if "resolve_canonical_vector_fallbacks(output.mappings).await" in ops_runtime:
        failures.append("operations still reference legacy canonical vector fallback resolver")

    for needle in [
        "matching_stage: MatchingStage::VectorQdrant",
        "mapping_type: \"vector_required\".to_string()",
        "match_method: \"vector_qdrant_required\".to_string()",
    ]:
        if needle not in seo_steps_canonical:
            failures.append(f"seo_steps canonical mapping missing `{needle}`")

    if "pseudo_qdrant_score(" in seo_steps_canonical:
        failures.append("seo_steps canonical mapping still contains pseudo_qdrant_score")

    if failures:
        print("VOYAGE_RETRIEVAL_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VOYAGE_RETRIEVAL_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
