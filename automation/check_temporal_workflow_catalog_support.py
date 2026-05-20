#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS_MOD = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
EXPERT_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "expert_extraction.rs"
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
    reconcile = read(RECONCILE_WORKFLOW)
    activities = read(ACTIVITIES)
    operations = read(OPERATIONS)
    starter = read(STARTER)
    proto = read(PROTO)
    payload_store = read(PAYLOAD_STORE)

    for needle in [
        "mod expert_extraction;",
        "mod projection_reconcile;",
        "expert_extraction::register(&mut opts);",
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
        "struct ProjectionReconcileWorkflow",
        "projection_reconcile.neo4j",
        "projection_reconcile.qdrant",
        "done:projection_reconcile",
    ]:
        if needle not in reconcile:
            failures.append(f"projection reconcile workflow missing `{needle}`")

    for needle in [
        "run_projection_reconcile_step",
        "ReconcileTargetInputPayload",
        "projection_reconcile",
    ]:
        if needle not in activities:
            failures.append(f"activities missing `{needle}`")

    if "projection_reconcile_impl(" not in operations:
        failures.append("operations missing `projection_reconcile_impl`")

    for needle in [
        "ExpertExtraction",
        "ProjectionReconcile",
        "ExpertExtractionWorkflow",
        "ProjectionReconcileWorkflow",
    ]:
        if needle not in starter:
            failures.append(f"temporal starter missing `{needle}`")

    if "message ReconcileTargetInputPayload" not in proto:
        failures.append("proto missing `ReconcileTargetInputPayload`")
    if "impl RuntimeProtoPayload for ReconcileTargetInputPayload" not in payload_store:
        failures.append("payload store missing RuntimeProtoPayload impl for ReconcileTargetInputPayload")

    if failures:
        print("TEMPORAL_WORKFLOW_CATALOG_SUPPORT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TEMPORAL_WORKFLOW_CATALOG_SUPPORT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
