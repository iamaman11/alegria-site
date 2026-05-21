#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS_MOD = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
EXPERT_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "expert_extraction.rs"
EXPERT_DECOMPOSED_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "expert_decomposed_extraction.rs"
EXPERT_PROJECTION_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "expert_projection.rs"
EXPERT_SEMANTIC_SLICE_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "expert_semantic_slice.rs"
RECONCILE_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "projection_reconcile.rs"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
OPERATIONS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations.rs"
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []

    workflows_mod = read(WORKFLOWS_MOD)
    expert = read(EXPERT_WORKFLOW)
    expert_decomposed = read(EXPERT_DECOMPOSED_WORKFLOW)
    expert_projection = read(EXPERT_PROJECTION_WORKFLOW)
    expert_semantic_slice = read(EXPERT_SEMANTIC_SLICE_WORKFLOW)
    reconcile = read(RECONCILE_WORKFLOW)
    activities = read(ACTIVITIES)
    operations = read(OPERATIONS)
    starter = read(STARTER)
    proto = read(PROTO)
    payload_store = read(PAYLOAD_STORE)

    for needle in [
        "mod expert_extraction;",
        "mod expert_decomposed_extraction;",
        "mod expert_projection;",
        "mod expert_semantic_slice;",
        "mod projection_reconcile;",
        "expert_extraction::register(&mut opts);",
        "expert_decomposed_extraction::register(&mut opts);",
        "expert_projection::register(&mut opts);",
        "expert_semantic_slice::register(&mut opts);",
        "projection_reconcile::register(&mut opts);",
    ]:
        if needle not in workflows_mod:
            failures.append(f"workflow registry missing `{needle}`")

    for needle in [
        "struct ExpertExtractionWorkflow",
        "load_verified_support_bundle.initial",
        "raw_knowledge_ingestion",
        "done:expert_extraction",
    ]:
        if needle not in expert:
            failures.append(f"expert workflow missing `{needle}`")

    for needle in [
        "struct ExpertDecomposedExtractionWorkflow",
        "load_semantic_section_sample",
        "page_utility_classifier",
        "dom_block_relevance_filter",
        "layer_router",
        "entity_span_detection",
        "canonical_mapping",
        "procedural_extraction",
        "completeness_judge",
        "triple_builder",
        "contradiction_gate",
        "hitl_decision",
        "raw_knowledge_ingestion",
        "done:expert_decomposed_extraction",
    ]:
        if needle not in expert_decomposed:
            failures.append(f"expert decomposed workflow missing `{needle}`")

    for needle in [
        "struct ExpertProjectionWorkflow",
        "graph_admissibility_gate",
        "neo4j_sync",
        "retrieval_admissibility_gate",
        "voyage_qdrant_sync",
        "done:expert_projection",
    ]:
        if needle not in expert_projection:
            failures.append(f"expert projection workflow missing `{needle}`")

    for needle in [
        "struct ExpertSemanticSliceWorkflow",
        "load_semantic_section_sample",
        "page_utility_classifier",
        "dom_block_relevance_filter",
        "entity_span_detection",
        "canonical_mapping",
        "procedural_extraction",
        "completeness_judge",
        "triple_builder",
        "contradiction_gate",
        "hitl_decision",
        "done:expert_semantic_slice",
    ]:
        if needle not in expert_semantic_slice:
            failures.append(f"expert semantic slice workflow missing `{needle}`")

    for needle in [
        "struct ProjectionReconcileWorkflow",
        "projection_reconcile.neo4j",
        "projection_reconcile.qdrant",
        "done:projection_reconcile",
    ]:
        if needle not in reconcile:
            failures.append(f"projection reconcile workflow missing `{needle}`")

    for needle in [
        "run_projection_reconcile_step",
        "run_projection_barrier_audit_step",
        "load_semantic_section_sample_step",
        "run_page_utility_classifier_step",
        "run_dom_block_relevance_step",
        "run_entity_span_detection_step",
        "run_canonical_mapping_step",
        "run_procedural_extraction_step",
        "run_completeness_judge_step",
        "ProjectionBarrierAuditInputPayload",
        "ReconcileTargetInputPayload",
        "projection_reconcile",
    ]:
        if needle not in activities:
            failures.append(f"activities missing `{needle}`")

    if "projection_reconcile_impl(" not in operations:
        failures.append("operations missing `projection_reconcile_impl`")

    for needle in [
        "ExpertDecomposedExtraction",
        "ExpertExtraction",
        "ExpertProjection",
        "ExpertSemanticSlice",
        "ProjectionReconcile",
        "ExpertDecomposedExtractionWorkflow",
        "ExpertExtractionWorkflow",
        "ExpertProjectionWorkflow",
        "ExpertSemanticSliceWorkflow",
        "ProjectionReconcileWorkflow",
    ]:
        if needle not in starter:
            failures.append(f"temporal starter missing `{needle}`")

    if "message ReconcileTargetInputPayload" not in proto:
        failures.append("proto missing `ReconcileTargetInputPayload`")
    if "message ProjectionBarrierAuditInputPayload" not in proto:
        failures.append("proto missing `ProjectionBarrierAuditInputPayload`")
    if "message ProjectionBarrierAuditOutputPayload" not in proto:
        failures.append("proto missing `ProjectionBarrierAuditOutputPayload`")
    if "impl RuntimeProtoPayload for ReconcileTargetInputPayload" not in payload_store:
        failures.append("payload store missing RuntimeProtoPayload impl for ReconcileTargetInputPayload")
    if "impl RuntimeProtoPayload for ProjectionBarrierAuditInputPayload" not in payload_store:
        failures.append("payload store missing RuntimeProtoPayload impl for ProjectionBarrierAuditInputPayload")
    if "impl RuntimeProtoPayload for ProjectionBarrierAuditOutputPayload" not in payload_store:
        failures.append("payload store missing RuntimeProtoPayload impl for ProjectionBarrierAuditOutputPayload")

    if failures:
        print("TEMPORAL_WORKFLOW_CATALOG_SUPPORT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TEMPORAL_WORKFLOW_CATALOG_SUPPORT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
