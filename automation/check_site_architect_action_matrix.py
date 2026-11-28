#!/usr/bin/env python3
"""Check #94's accepted 52-row action plan against the source-derived ledger.

This checks a *planning document*, never an installed operation registry. In
particular, source declarations cannot confer Temporal admission or readiness.
"""

from __future__ import annotations

import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import audit_operation_inventory as ledger

ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "app/rust/crates/seo_application/src/site_architect/site_architect_action_matrix_v1.json"
ACTION_ID = re.compile(r"^[a-z][a-z0-9]*(?:[._][a-z][a-z0-9_]*)+@[1-9][0-9]*$")
DECISIONS = {"KEEP": 29, "SPLIT": 12, "INTERNAL": 11}
RESOLVED_LEGACY_PHASES = {
    "load_verified_support_bundle": (["KN-02c"], "reuse_proposed_action", "knowledge.verified_rules.load@1"),
    "raw_knowledge_ingestion": (["ACQ-07", "EX-04", "TR-05", "TR-06", "TR-07"], "decompose_legacy_macro", None),
    "load_verified_support_bundle.refresh": (["KN-02c"], "reuse_proposed_action", "knowledge.verified_rules.load@1"),
}
PENDING_INVENTORY_GAPS = (
    "public PDF download and file import",
    "Expand provider inventory",
)


def validate(
    manifest: dict,
    source_rows: list[dict],
    legacy_phases: list[dict],
    root: Path = ROOT,
) -> list[str]:
    errors: list[str] = []
    if manifest.get("revision") != "site_architect_action_decisions.v1":
        errors.append("unexpected action decision manifest revision")
    if manifest.get("source_matrix") != "docs/ISSUE94_OPERATION_DECISION_MATRIX.md":
        errors.append("action manifest points to an unexpected source matrix")
    if manifest.get("candidate_readiness") != "not_accepted":
        errors.append("planning manifest must not claim deployed readiness")

    rows = manifest.get("candidates")
    if not isinstance(rows, list):
        return errors + ["candidates must be an array"]
    ids = [row.get("id") if isinstance(row, dict) else None for row in rows]
    if len(rows) != 52 or len(set(ids)) != 52:
        errors.append("expected 52 unique candidate IDs")
    source_by_id = {row["id"]: row for row in source_rows}
    if set(ids) != set(source_by_id):
        errors.append(
            f"candidate IDs differ from Rust-source ledger: "
            f"missing={sorted(set(source_by_id)-set(ids))}, "
            f"extra={sorted(str(i) for i in set(ids)-set(source_by_id))}"
        )
    observed = Counter(row.get("decision") for row in rows if isinstance(row, dict))
    if observed != DECISIONS:
        errors.append(f"decision totals drift: {observed} != {DECISIONS}")

    referenced: dict[str, list[str]] = defaultdict(list)
    for row in rows:
        if not isinstance(row, dict):
            errors.append("candidate row must be an object")
            continue
        cid, decision, ops = row.get("id"), row.get("decision"), row.get("operations")
        if set(row) != {"id", "decision", "operations"}:
            errors.append(f"{cid}: candidate fields must be id/decision/operations only")
        if cid in source_by_id and source_by_id[cid]["decision"] != decision:
            errors.append(f"{cid}: classification differs from source ledger")
        if not isinstance(ops, list) or not all(isinstance(op, str) for op in ops):
            errors.append(f"{cid}: operations must be a string array")
            continue
        if decision == "INTERNAL" and ops:
            errors.append(f"{cid}: INTERNAL cannot expose user operations")
        if decision == "KEEP" and len(ops) != 1:
            errors.append(f"{cid}: KEEP must identify exactly one action")
        if decision == "SPLIT" and len(ops) < 2:
            errors.append(f"{cid}: SPLIT must identify at least two distinct actions")
        if len(ops) != len(set(ops)):
            errors.append(f"{cid}: duplicate operation in one candidate")
        for op in ops:
            if not ACTION_ID.fullmatch(op):
                errors.append(f"{cid}: non-versioned action ID: {op}")
            referenced[op].append(cid)

    shared = manifest.get("shared_operation_ids")
    if not isinstance(shared, dict):
        errors.append("shared_operation_ids must be a map")
        shared = {}
    actual_shared = {op: set(ids) for op, ids in referenced.items() if len(ids) > 1}
    declared_shared: dict[str, set[str]] = {}
    for op, ids in shared.items():
        if not isinstance(ids, list) or not all(isinstance(cid, str) for cid in ids):
            errors.append(f"shared operation {op} must list candidate IDs")
            continue
        if len(ids) != len(set(ids)):
            errors.append(f"shared operation {op} has duplicate candidate IDs")
        declared_shared[op] = set(ids)
    if declared_shared != actual_shared:
        errors.append(f"shared action aliases drift: actual={actual_shared}, declared={declared_shared}")

    declared = manifest.get("declared_rust_contracts")
    if not isinstance(declared, dict):
        errors.append("declared_rust_contracts must be a map")
        declared = {}
    if set(declared) - set(referenced):
        errors.append(f"source contracts not selected: {sorted(set(declared)-set(referenced))}")
    proposed = manifest.get("proposed_ports")
    if not isinstance(proposed, dict) or set(proposed) != set(referenced) - set(declared):
        errors.append("each action awaiting its Rust contract needs its own proposed ports")
    for operation_id, source in declared.items():
        if not isinstance(source, str):
            errors.append(f"{operation_id}: declaration source is not a path")
            continue
        full = (root / source).resolve()
        if not full.is_relative_to(root.resolve()) or not full.is_file():
            errors.append(f"{operation_id}: Rust declaration source does not exist: {source}")
            continue
        name, revision = operation_id.rsplit("@", 1)
        if revision != "1" or f'"{name}"' not in full.read_text(encoding="utf-8"):
            errors.append(f"{operation_id}: no literal v1 contract ID at {source}")

    mappings = manifest.get("legacy_phase_mapping")
    if not isinstance(mappings, dict):
        errors.append("legacy_phase_mapping must be a map")
        mappings = {}
    actual_phases = {phase["legacy_stable_id"] for phase in legacy_phases}
    if set(mappings) != actual_phases:
        errors.append(
            f"22 legacy phases not fully reconciled: "
            f"missing={sorted(actual_phases-set(mappings))}, "
            f"extra={sorted(set(mappings)-actual_phases)}"
        )
    for phase, candidate_ids in mappings.items():
        if not isinstance(candidate_ids, list) or not all(isinstance(cid, str) for cid in candidate_ids):
            errors.append(f"{phase}: legacy mapping must list candidate IDs")
            continue
        if len(candidate_ids) != len(set(candidate_ids)):
            errors.append(f"{phase}: duplicate legacy mapping")
        if set(candidate_ids) - set(source_by_id):
            errors.append(f"{phase}: mapped to unknown candidates")
        if set(candidate_ids) & {r.get("id") for r in rows if isinstance(r, dict) and r.get("decision") == "INTERNAL"}:
            errors.append(f"{phase}: mapped to user-invisible INTERNAL candidate")
        if not candidate_ids:
            errors.append(f"{phase}: unexplained legacy phase gap")
        if phase in RESOLVED_LEGACY_PHASES and candidate_ids != RESOLVED_LEGACY_PHASES[phase][0]:
            errors.append(f"{phase}: source-reviewed legacy phase mapping drift")

    dispositions = manifest.get("legacy_phase_dispositions")
    if not isinstance(dispositions, dict) or set(dispositions) != set(RESOLVED_LEGACY_PHASES):
        errors.append("three source-reviewed legacy phase dispositions must be complete")
    else:
        for phase, (_, expected_kind, expected_action) in RESOLVED_LEGACY_PHASES.items():
            detail = dispositions[phase]
            if not isinstance(detail, dict):
                errors.append(f"{phase}: legacy phase disposition must be an object")
                continue
            if detail.get("kind") != expected_kind or detail.get("action") != expected_action:
                errors.append(f"{phase}: source-reviewed disposition kind/action drift")
            if not isinstance(detail.get("reason"), str) or len(detail["reason"].strip()) < 32:
                errors.append(f"{phase}: source-reviewed disposition requires a concrete reason")
            evidence = detail.get("evidence")
            if not isinstance(evidence, list) or not evidence or not all(isinstance(path, str) for path in evidence):
                errors.append(f"{phase}: missing exact Rust source evidence")
            else:
                for path in evidence:
                    full = (root / path).resolve()
                    if not full.is_relative_to(root.resolve()) or not full.is_file():
                        errors.append(f"{phase}: invalid source evidence path: {path}")
    gaps = manifest.get("open_source_gaps")
    if not isinstance(gaps, list) or not all(isinstance(gap, str) for gap in gaps):
        errors.append("open_source_gaps must be a list of pending external inventories")
    elif any(phase in gap for phase in RESOLVED_LEGACY_PHASES for gap in gaps):
        errors.append("reviewed legacy phase cannot remain an unresolved source gap")
    elif any(not any(marker in gap for gap in gaps) for marker in PENDING_INVENTORY_GAPS):
        errors.append("pending external provider inventory gap disappeared without source review")

    return errors


def main() -> int:
    source_rows, source_errors = ledger.candidates()
    phases, phase_errors = ledger.legacy_phases()
    manifest = json.loads(MATRIX.read_text(encoding="utf-8"))
    errors = source_errors + phase_errors + validate(manifest, source_rows, phases)
    if errors:
        for error in errors:
            print(f"FAIL {error}", file=sys.stderr)
        return 1
    declared = set(manifest["declared_rust_contracts"])
    selected = {action for row in manifest["candidates"] for action in row["operations"]}
    print(
        "SITE_ARCHITECT_ACTION_MATRIX PASS "
        f"52 candidates: 29 KEEP + 12 SPLIT + 11 INTERNAL; "
        f"{len(selected)} proposed unique actions, {len(declared)} source-declared "
        f"contracts, {len(selected - declared)} proposed-only, 22 legacy phases "
        f"({len(RESOLVED_LEGACY_PHASES)} source-reviewed dispositions, 0 unresolved legacy phases); 0 readiness claims"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
