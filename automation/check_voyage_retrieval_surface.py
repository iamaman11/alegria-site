#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter"
OPS_RUNTIME = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations"
KNOWLEDGE_USE_CASE = ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "knowledge.rs"
KNOWLEDGE_PORT = ROOT / "app" / "rust" / "crates" / "seo_ports" / "src" / "lib.rs"
VECTOR_ADAPTER = ROOT / "app" / "rust" / "crates" / "runtime_composition" / "src" / "adapters" / "canonical_vector_search_adapter.rs"
SEO_DOMAIN_CANONICAL = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src" / "canonical_mapping.rs"
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
    knowledge_use_case = read(KNOWLEDGE_USE_CASE)
    knowledge_port = read(KNOWLEDGE_PORT)
    vector_adapter = read(VECTOR_ADAPTER)
    seo_domain_canonical = read(SEO_DOMAIN_CANONICAL)
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
        "seo_application::knowledge::resolve_canonical_vector_mappings(",
        "&runtime_composition::adapters::canonical_vector_search_adapter::CanonicalVectorSearchAdapter",
        "CANONICAL_VECTOR_RETRIEVAL_REQUIRED",
        "VOYAGE_RERANK_REQUIRED",
        "\"kb_canonical_4\"",
    ]:
        if needle not in ops_runtime:
            failures.append(f"operations canonical mapping path missing `{needle}`")

    if "resolve_canonical_vector_fallbacks(output.mappings).await" in ops_runtime:
        failures.append("operations still reference legacy canonical vector fallback resolver")

    # The canonical matching decision is Knowledge-owned. Temporal wires the
    # typed port to the infrastructure adapter; it must not own vector policy.
    for needle in [
        "pub async fn resolve_canonical_vector_mappings<S: CanonicalVectorSearchPort + ?Sized>(",
        "search.search_canonical_concepts(&mapping.raw_text).await?",
        "CanonicalVectorLookup::Unavailable",
        "canonical_mapping::resolve_vector_candidates(&mapping, &candidates)",
    ]:
        if needle not in knowledge_use_case:
            failures.append(f"Knowledge canonical vector use case missing `{needle}`")

    for needle in [
        "pub trait CanonicalVectorSearchPort: Send + Sync",
        "async fn search_canonical_concepts(",
    ]:
        if needle not in knowledge_port:
            failures.append(f"canonical vector port missing `{needle}`")

    for needle in [
        "impl CanonicalVectorSearchPort for CanonicalVectorSearchAdapter",
        "semantic_search_adapter::search_by_text_with_surface(",
        "semantic_search_adapter::rerank_records(",
        "CANONICAL_VECTOR_RETRIEVAL_REQUIRED",
        "VOYAGE_RERANK_REQUIRED",
        "\"kb_canonical_4\"",
    ]:
        if needle not in vector_adapter:
            failures.append(f"canonical vector adapter missing `{needle}`")

    for needle in [
        "matching_stage: MatchingStage::VectorQdrant",
        "mapping_type: \"vector_required\".to_string()",
        "match_method: \"vector_qdrant_required\".to_string()",
    ]:
        if needle not in seo_domain_canonical:
            failures.append(f"seo_domain canonical mapping missing `{needle}`")

    if "pseudo_qdrant_score(" in seo_domain_canonical:
        failures.append("seo_domain canonical mapping still contains pseudo_qdrant_score")

    if failures:
        print("VOYAGE_RETRIEVAL_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VOYAGE_RETRIEVAL_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
