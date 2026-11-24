#!/usr/bin/env python3
"""Guard WholePage policy ownership and both production runtime consumers."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"
DOMAIN = RUST / "crates" / "seo_domain" / "src" / "whole_page"
TEMPORAL = RUST / "services" / "temporal" / "src" / "activities" / "operations"
EXPERT = RUST / "crates" / "infrastructure" / "src" / "adapters"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> None:
    domain = "\n".join(path.read_text() for path in DOMAIN.glob("*.rs"))
    temporal = (TEMPORAL / "acquisition_section_ops.rs").read_text()
    temporal_adapter = (TEMPORAL / "whole_page_semantic_ops.rs").read_text()
    expert = (EXPERT / "expert_extraction_core" / "core_runner.rs").read_text()
    expert_adapter = (EXPERT / "expert_extraction_core.rs").read_text()

    require("pub mod whole_page;" in (DOMAIN.parent / "lib.rs").read_text(), "WholePage domain is not exported")
    require("enum LayerScoringProfile" in domain and "ExpertSection" in domain and "TemporalPage" in domain, "missing explicit semantic profiles")
    for forbidden in ("RawSectionRecord", "whole_page_advisory_adapter", "std::env::var", "sqlx::", "qdrant::"):
        require(forbidden not in domain, f"domain leaked runtime dependency {forbidden}")
    for method in ("whole_page::deterministic_whole_page_snapshot(", "whole_page::build_page_sketch(", "whole_page::fuse_with_advisory_retrieval("):
        require(method in temporal, f"Temporal runtime not using domain {method}")
    require("to_whole_page_sections(&sections)" in temporal and "to_whole_page_advisory_hits(&advisory_hits)" in temporal, "Temporal adapters missing typed projections")
    require("whole_page::expert_section_semantics(" in expert, "expert runtime not using domain semantic policy")
    require("WholePageSemanticPassOutput {" in expert, "expert stage changed persisted DTO")
    require("stage_record(\n            \"whole_page_semantic_pass\"" in expert, "expert stage registration changed")
    require("expert_extraction_core/stage_record.rs" in expert_adapter, "expert persistence metadata missing")
    require("semantic_scoring.rs" not in expert_adapter and not (EXPERT / "expert_extraction_core" / "semantic_scoring.rs").exists(), "legacy expert semantic policy remains")
    for stale in ("scoring.rs", "advisory.rs", "context_profile.rs"):
        require(not (TEMPORAL / "whole_page_semantic_ops" / stale).exists(), f"old Temporal semantic owner remains: {stale}")
    require('"alegria.runtime.json.WholePageSemanticPassOutput"' in (TEMPORAL / "contracts" / "runtime_payload_impls.rs").read_text(), "Temporal payload identity changed")
    require("WHOLE_PAGE_ADVISORY_LIMIT" in temporal_adapter and "falling back to deterministic semantics" in temporal, "advisory fallback changed")
    print("WHOLE_PAGE_OWNER_CONTRACT=PASS (Temporal + Expert + payload identity)")


if __name__ == "__main__":
    main()
