#!/usr/bin/env python3
"""Source-derived, read-only audit for Issue #94 architecture freeze.

This is NOT a second production operation catalog: it discovers existing
Rust declarations and call sites, checks all recorded inventory identities,
and emits evidence. It must not turn legacy phases into normative operations.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs"
WORKFLOWS = ROOT / "app/rust/services/temporal/src/workflows"
EXECUTION = ROOT / "app/rust/crates/seo_application/src/execution.rs"
RECIPE = ROOT / "app/rust/crates/seo_application/src/execution/recipe.rs"
PURE_OPS = ROOT / "app/rust/crates/seo_domain/src/extraction/operations.rs"
MATRIX = ROOT / "docs/ISSUE94_OPERATION_DECISION_MATRIX.md"

# Counts apply to the audited 851c3fa baseline. They are drift tripwires, not
# desired architecture: changing a count requires a new source inventory.
AUDITED_ACTIVITY_COUNT = 73
AUDITED_PHASE_COUNT = 22
AUDITED_CANDIDATE_COUNT = 52
AUDITED_PURE_OPERATION_COUNT = 5
ALLOWED_DECISIONS = {"KEEP", "MERGE", "SPLIT", "INTERNAL", "DELETE", "TBD"}

# This checked-in *audit classification* mirrors #94 §6.1, not a runtime
# operation catalog. Every activity must occur in precisely one provisional
# owner/effect family, or the inventory is stale and requires human review.
ACTIVITY_FAMILIES = {
    "acquisition": """
      run_page_utility_classifier_step run_dom_block_relevance_step
      run_whole_page_semantic_pass_step run_sectioning_step
      run_page_utility_sweep_step run_dom_block_relevance_sweep_step
      run_sectioning_contract_gate_step run_cas_gate_step run_raw_evidence_register_step
    """,
    "extraction": """
      run_layer_router_step run_entity_span_detection_step run_canonical_mapping_step
      run_procedural_extraction_step run_operational_extraction_step
      run_editorial_extraction_step run_layer_router_sweep_step
      run_subspan_layer_router_step run_entity_span_sweep_step
      run_canonical_mapping_sweep_step run_ontology_intake_gate_step
      run_procedural_extraction_sweep_step run_operational_extraction_sweep_step
      run_editorial_extraction_sweep_step run_seo_signal_extraction_step
      run_commercial_signal_extraction_step run_extraction_schema_validate_step
    """,
    "truth": """
      run_completeness_judge_step run_triple_builder_step
      run_contradiction_gate_step run_hitl_decision_step
      prepare_manual_truth_candidates load_manual_truth_status
      run_candidate_validation_step run_triple_builder_sweep_step
      run_completeness_judge_sweep_step run_resolution_loop_step
      run_contradiction_gate_sweep_step run_truth_adjudication_step
      run_verified_truth_write_step run_truth_admissibility_gate_step
    """,
    "projection": """
      run_projection_reconcile_step run_projection_sync_step
      run_projection_barrier_audit_step load_semantic_section_sample_step
      run_seo_preflight_step
    """,
    "planning": """
      load_seo_site_build_input load_verified_support_bundle
      run_serp_ingest_step run_crawl_sources_step run_raw_knowledge_ingestion_step
      run_serp_normalize_step run_opportunity_build_step run_ia_build_step
      run_link_recommend_step run_global_site_reconcile_step
    """,
    "editorial": """
      run_draft_assemble_step run_draft_normalize_step run_editorial_draft_generate
      load_manual_editorial_result run_content_contract_validate_step run_draft_qa_step
    """,
    "publishing": """
      run_cms_request_review_step run_human_approval_wait_step
      run_cms_publish_approved_step load_cms_approval_decision
      run_rebuild_detect_step run_publish_materialize_step
      run_render_preview_validate_step run_finalize_publish_step
    """,
    "control": """
      test_step_prepare test_step_finalize finalize_run check_data_freshness
    """,
}
FAMILY_EFFECT_CANDIDATES = {
    "acquisition": "PURE acquisition + PG evidence/HTTP adapter (per method TBD)",
    "extraction": "PURE domain + PG journal, optional Qdrant read (per method TBD)",
    "truth": "Truth/HITL policy + PG atomic store/manual queue (per method TBD)",
    "projection": "Outbox/Neo4j/Qdrant/PG readiness; source-specific fence TBD",
    "planning": "SERP/HTTP/PG + Graph read/projection (per method TBD)",
    "editorial": "PURE draft + external LLM/manual PG queue (per method TBD)",
    "publishing": "CMS/PG/FS/CDN effects, publish freshness fence required",
    "control": "Temporal state/journal/test, not business operation",
}
BASELINE_CANDIDATE_IDS = set("""
 ACQ-01 ACQ-02 ACQ-03 ACQ-04a ACQ-04b ACQ-05 ACQ-06 ACQ-07
 EX-01 EX-02 EX-03 EX-04 EX-05 EX-06 EX-07 EX-08a EX-08b EX-08c
 TR-01 TR-02 TR-03 TR-04a TR-04b TR-05 TR-06 TR-07 TR-08 TR-09
 KN-01a KN-01b KN-02a KN-02b KN-02c
 PL-01 PL-02 PL-03 PL-04 PL-05 PL-06
 ED-01 ED-02 ED-03 ED-04a ED-04b ED-05
 CMS-01a CMS-01b CMS-02 PUB-01 PUB-02 PUB-03 PUB-04
""".split())

ACTIVITY_SIGNATURE = re.compile(
    r"#\[activity\]\s*pub async fn\s+(\w+)\s*\((.*?)\)\s*"
    r"->\s*Result<\s*(.*?)\s*,\s*ActivityError\s*>",
    re.DOTALL,
)
ENUM = re.compile(r"pub enum SeoPhaseKey\s*\{(?P<body>[^}]+)\}", re.DOTALL)
FROM_STABLE_ID = re.compile(r'"([^"\n]+)"\s*=>\s*Self::(\w+)')
PURE_OPERATION = re.compile(r"impl Operation for\s+(\w+)\s*\{")
CANDIDATE_ROW = re.compile(
    r"^\|\s*(?P<id>[A-Z]{2,3}-\d{2}[a-z]?)\s*\|\s*"
    r"(?P<decision>KEEP|MERGE|SPLIT|INTERNAL|DELETE|TBD)\s*\|"
    r"\s*(?P<owner>[^|]*)\|\s*(?P<effects>[^|]*)\|"
    r"\s*(?P<reachability>[^|]*)\|\s*(?P<source>[^|]*)\|",
)

ROUTE_PREFIXES = (
    ("seo_site_build_canonical_cutover", "canonical_v1"),
    ("seo_site_build_composable_v2", "source_only_v2"),
    ("pure_operation_dag", "pure_dag"),
    ("expert_", "expert_opt_in"),
    ("test_hitl", "test_workflow"),
    ("projection_reconcile", "projection_runtime"),
    ("freshness", "freshness_runtime"),
)


def route_for(source: Path) -> str:
    path = source.relative_to(WORKFLOWS).as_posix()
    for prefix, route in ROUTE_PREFIXES:
        if path.startswith(prefix):
            return route
    return "other_workflow"


def activity_registry() -> tuple[list[dict], list[str]]:
    source = REGISTRY.read_text(encoding="utf-8")
    errors: list[str] = []
    blocks = list(ACTIVITY_SIGNATURE.finditer(source))
    declared = source.count("#[activity]")
    if declared != len(blocks):
        errors.append(
            f"Activity signature parser matched {len(blocks)}/{declared} declarations"
        )
    seen = Counter(match.group(1) for match in blocks)
    if duplicates := sorted(name for name, count in seen.items() if count != 1):
        errors.append(f"duplicate Activity methods: {duplicates}")
    group_members = {
        name: group for group, members in ACTIVITY_FAMILIES.items()
        for name in members.split()
    }
    all_classified = [name for members in ACTIVITY_FAMILIES.values() for name in members.split()]
    if len(all_classified) != len(set(all_classified)):
        errors.append("Activity family ledger has duplicate methods")
    if set(seen) != set(group_members):
        errors.append(
            f"Activity family ledger mismatch: missing={sorted(set(seen)-set(group_members))}, "
            f"removed={sorted(set(group_members)-set(seen))}"
        )
    refs: dict[str, set[str]] = {name: set() for name in seen}
    reference_sources: dict[str, set[str]] = {name: set() for name in seen}
    for file in WORKFLOWS.rglob("*.rs"):
        code = file.read_text(encoding="utf-8")
        for name in seen:
            if re.search(rf"\bAlegriaActivities::{re.escape(name)}\b", code):
                refs[name].add(route_for(file))
                reference_sources[name].add(file.relative_to(ROOT).as_posix())
    activities: list[dict] = []
    for match in blocks:
        name, params, output = match.group(1, 2, 3)
        arg = re.search(
            r"_ctx\s*:\s*ActivityContext\s*,\s*(\w+)\s*:\s*(.*?)\s*,?\s*$", params, re.DOTALL
        )
        if not arg:
            errors.append(f"Activity {name} has no parsed input argument")
            input_name, input_type = "?", "TBD"
        else:
            input_name, input_type = arg.group(1), " ".join(arg.group(2).strip().split())
            input_type = input_type.rstrip(",").strip()
        start = match.end()
        next_activity = source.find("#[activity]", start)
        body = source[start : next_activity if next_activity >= 0 else len(source)]
        activities.append(
            {
                "name": name,
                "input_parameter": input_name,
                "input_type": input_type,
                "output_type": " ".join(output.strip().split()),
                "line": source.count("\n", 0, match.start()) + 1,
                "audit_family": group_members.get(name, "TBD"),
                "provisional_owner_effects": FAMILY_EFFECT_CANDIDATES.get(
                    group_members.get(name, ""), "TBD"
                ),
                "referenced_by": sorted(refs[name]),
                "reference_files": sorted(reference_sources[name]),
                "journal_wrapper_present": "self.execute_step(" in body,
                "effect_contract": "TBD: call-graph and owner admission required",
            }
        )
        if not refs[name]:
            # Source registration does not establish production reachability.
            # A method without a static workflow call must be explicitly
            # reviewed, not silently called a business operation.
            errors.append(f"registered Activity has no static workflow reference: {name}")
    if len(activities) != AUDITED_ACTIVITY_COUNT:
        errors.append(f"Activity count drift: {len(activities)} != {AUDITED_ACTIVITY_COUNT}")
    canonical = sum("canonical_v1" in item["referenced_by"] for item in activities)
    expert_only = sum(
        "expert_opt_in" in item["referenced_by"]
        and "canonical_v1" not in item["referenced_by"]
        and "projection_runtime" not in item["referenced_by"]
        for item in activities
    )
    runtime_other = sum(
        "projection_runtime" in item["referenced_by"]
        or "freshness_runtime" in item["referenced_by"]
        for item in activities
    )
    test_only = sum(item["referenced_by"] == ["test_workflow"] for item in activities)
    v2 = sum("source_only_v2" in item["referenced_by"] for item in activities)
    if (canonical, expert_only, runtime_other, test_only, v2) != (55, 14, 2, 2, 5):
        errors.append(
            f"workflow reachability drift: {(canonical, expert_only, runtime_other, test_only, v2)}"
            " != (55 canonical V1, 14 expert-only, 2 other, 2 test-only, 5 reused in V2)"
        )
    return activities, errors


def legacy_phases() -> tuple[list[dict], list[str]]:
    source = EXECUTION.read_text(encoding="utf-8")
    recipe = RECIPE.read_text(encoding="utf-8")
    errors: list[str] = []
    enum = ENUM.search(source)
    if enum is None:
        return [], ["SeoPhaseKey enum not found"]
    variants = re.findall(r"(?m)^\s*([A-Z]\w+),\s*$", enum.group("body"))
    ids = FROM_STABLE_ID.findall(recipe.split("pub fn from_stable_id", 1)[-1].split("pub fn contract", 1)[0])
    counts = Counter(variant for _, variant in ids)
    if len(variants) != AUDITED_PHASE_COUNT or len(ids) != AUDITED_PHASE_COUNT:
        errors.append(
            f"legacy phase drift: enum={len(variants)}, stable-IDs={len(ids)}, expected={AUDITED_PHASE_COUNT}"
        )
    if sorted(variants) != sorted(counts):
        errors.append(f"SeoPhaseKey enum/stable-ID mismatch: {sorted(set(variants)^set(counts))}")
    if any(count != 1 for count in counts.values()):
        errors.append("duplicate SeoPhaseKey stable-ID variant")
    if len({name for name, _ in ids}) != len(ids):
        errors.append("duplicate SeoPhaseKey stable ID")
    phase_label_body = source.split("pub fn phase_label", 1)[-1].split("\n}\n", 1)[0]
    phase_labels = {
        variant: stable_id
        for variant, stable_id in re.findall(
            r'SeoPhaseKey::(\w+)\s*=>\s*"([^"]+)"', phase_label_body
        )
    }
    if phase_labels != {variant: stable_id for stable_id, variant in ids}:
        errors.append("SeoPhaseKey phase_label/stable-ID divergence")
    return [
        {"variant": variant, "legacy_stable_id": stable_id, "normative_operation": False}
        for stable_id, variant in ids
    ], errors


def pure_operations() -> tuple[list[str], list[str]]:
    source = PURE_OPS.read_text(encoding="utf-8")
    names = PURE_OPERATION.findall(source)
    errors: list[str] = []
    if len(names) != AUDITED_PURE_OPERATION_COUNT or len(set(names)) != len(names):
        errors.append(f"pure implementation drift: {len(names)} instead of {AUDITED_PURE_OPERATION_COUNT}")
    for name in names:
        if not re.search(rf"impl PureExtractionOperation for\s+{re.escape(name)}\b", source):
            errors.append(f"Operation has no PureExtractionOperation implementation: {name}")
    return names, errors


def candidates() -> tuple[list[dict], list[str]]:
    code = MATRIX.read_text(encoding="utf-8")
    rows: list[dict] = []
    errors: list[str] = []
    for line in code.splitlines():
        match = CANDIDATE_ROW.match(line)
        if not match:
            continue
        item = {key: value.strip() for key, value in match.groupdict().items()}
        rows.append(item)
        if not item["owner"] or not item["effects"] or not item["reachability"] or not item["source"]:
            errors.append(f"missing audited dimension for {item['id']}")
        for relative_path in item["source"].split(";"):
            path = (ROOT / relative_path.strip()).resolve()
            if not path.is_relative_to(ROOT) or not path.is_file():
                errors.append(f"candidate {item['id']} has invalid source path: {relative_path}")
    if len(rows) != AUDITED_CANDIDATE_COUNT:
        errors.append(f"candidate decision rows drift: {len(rows)} != {AUDITED_CANDIDATE_COUNT}")
    if len({row["id"] for row in rows}) != len(rows):
        errors.append("duplicate candidate IDs")
    if {row["id"] for row in rows} != BASELINE_CANDIDATE_IDS:
        errors.append(
            "candidate identity drift: missing="
            + str(sorted(BASELINE_CANDIDATE_IDS - {row["id"] for row in rows}))
            + " unexpected="
            + str(sorted({row["id"] for row in rows} - BASELINE_CANDIDATE_IDS))
        )
    return rows, errors


def audit() -> dict:
    activities, activity_errors = activity_registry()
    phases, phase_errors = legacy_phases()
    pure, pure_errors = pure_operations()
    matrix, matrix_errors = candidates()
    errors = activity_errors + phase_errors + pure_errors + matrix_errors
    return {
        "source_ref": "851c3fa54fd80c6ad7eb90c8f24d97568baeb45b (baseline; current files read live)",
        "counts": {
            "registered_temporal_activities": len(activities),
            "legacy_seo_phase_keys": len(phases),
            "candidate_operation_decisions": len(matrix),
            "generic_pure_operation_implementations": len(pure),
        },
        "activities": activities,
        "legacy_phases": phases,
        "candidate_decisions": matrix,
        "generic_pure_operations": pure,
        # Even an explicitly classified effect does not prove a versioned
        # domain Input/Output/Error contract or tested effect admission.
        "remaining_unknown_contracts": [row["id"] for row in matrix],
        "explicit_tbd_fields": [
            row["id"] for row in matrix
            if "TBD" in row["owner"] + row["effects"] + row["reachability"]
        ],
        "validation_errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--format", choices=("json", "summary"), default="summary")
    args = parser.parse_args()
    report = audit()
    if args.format == "json":
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print("ISSUE94_INVENTORY " + json.dumps(report["counts"], sort_keys=True))
        print("unresolved_candidate_contracts=" + str(len(report["remaining_unknown_contracts"])))
        for error in report["validation_errors"]:
            print("ERROR: " + error, file=sys.stderr)
    return 1 if report["validation_errors"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
