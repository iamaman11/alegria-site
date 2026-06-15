#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
INFRA_CARGO = ROOT / "app" / "rust" / "crates" / "infrastructure" / "Cargo.toml"
EXPERT_CORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "expert_extraction_core.rs"
RAW_CRAWL = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "raw_crawl_adapter.rs"
SEO_STEPS_LIB = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "lib.rs"

REQUIRED_EXPORTS = [
    "pub mod canonical_mapping_step;",
    "pub mod completeness_judge_step;",
    "pub mod entity_span_detection_step;",
    "pub mod procedural_extraction_step;",
]

REQUIRED_STAGE_NAMES = [
    "whole_page_semantic_pass",
    "page_utility_classifier",
    "dom_block_relevance_filter",
    "sectioning_contract_gate",
    "cas_gate",
    "layer_router",
    "subspan_layer_router",
    "entity_span_detection",
    "canonical_mapping",
    "ontology_intake_gate",
    "procedural_extraction",
    "operational_extraction",
    "editorial_extraction",
    "extraction_schema_validate",
    "candidate_validation",
    "triple_builder",
    "completeness_judge",
    "resolution_loop",
    "contradiction_gate",
    "truth_adjudication",
    "verified_truth_write",
]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "expert_extraction_core.rs":
        sub_dir = path.parent / "expert_extraction_core"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "raw_crawl_adapter.rs":
        sub_dir = path.parent / "raw_crawl_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []

    cargo = read(INFRA_CARGO)
    expert_core = read_file_resolved(EXPERT_CORE)
    raw_crawl = read_file_resolved(RAW_CRAWL)
    seo_steps_lib = read(SEO_STEPS_LIB)

    if 'seo_steps = { path = "../seo_steps" }' not in cargo:
        failures.append("infrastructure crate does not depend on seo_steps")

    for export in REQUIRED_EXPORTS:
        if export not in seo_steps_lib:
            failures.append(f"seo_steps/lib.rs missing export `{export}`")

    for needle in [
        "pub fn run_expert_extraction_core(",
        "ExpertStageRecord",
        "idempotency_key",
        "input_hash",
        "output_hash",
        "ExpertStageStatus",
        "run_expert_extraction_core(",
        "mixed_layer_section_produces_rich_stage_records",
        "unresolved_mapping_and_loss_require_hitl",
        "conflicting_high_confidence_values_are_rejected",
    ]:
        if needle not in expert_core:
            failures.append(f"expert core missing `{needle}`")

    for stage_name in REQUIRED_STAGE_NAMES:
        if f"\"{stage_name}\"" not in expert_core:
            failures.append(f"expert core missing stage `{stage_name}`")

    for needle in [
        "run_expert_extraction_core(",
        "expert_blocked_section_count",
        "expert_needs_hitl_section_count",
        "expert_verified_ready_section_count",
        "expert_triple_count",
    ]:
        if needle not in raw_crawl:
            failures.append(f"raw_crawl_adapter missing `{needle}` integration")

    if failures:
        print("EXPERT_CORE_ACTIVATION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "EXPERT_CORE_ACTIVATION_CONTRACT: OK "
        f"stages={len(REQUIRED_STAGE_NAMES)} exports={len(REQUIRED_EXPORTS)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
