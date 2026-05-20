#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
V6_OWNER = ROOT / "docs" / "V6_Expert_Truth_Graph_Runtime.md"
V6_PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
INDEX = ROOT / "docs" / "INDEX.md"

PROCESSES = [
    "freshness_monitor",
    "rebuild_dispatcher",
    "ontology_backfill_reindex",
    "projection_reconcile_and_reclaim",
    "post_publish_feedback_loop",
    "release_and_restore_gate",
]

CONTRACT_FIELDS = [
    "Purpose",
    "Trigger",
    "Inputs",
    "Outputs",
    "Owner",
    "Blocking relation to main flow",
    "Required evidence artifact",
    "Automation checks",
]


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []

    if not REGISTRY.exists():
        print("SUPPORT_PROCESS_REGISTRY: FAILED")
        print("- missing docs/V6_Support_Process_Registry.md")
        return 1

    registry = read(REGISTRY)
    owner = read(V6_OWNER)
    plan = read(V6_PLAN)
    ops = read(OPS)
    index = read(INDEX)

    for process in PROCESSES:
        if process not in registry:
            failures.append(f"registry missing `{process}`")

    for field in CONTRACT_FIELDS:
        if field not in registry:
            failures.append(f"registry missing contract field `{field}`")

    for needle in [
        "current support-plane registry",
        "Owner-Doc Boundaries",
        "Activation Criteria",
        "code-present, runtime-inactive",
        "V5 Status",
    ]:
        if needle not in registry:
            failures.append(f"registry missing `{needle}` section")

    if "V6_Support_Process_Registry.md" not in owner:
        failures.append("V6 owner doc does not reference support process registry")
    if "code-present, runtime-inactive" not in owner:
        failures.append("V6 owner doc does not define code-present/runtime-inactive rule")
    if "Support-process registry boundary" not in plan:
        failures.append("V6 working plan missing support-process boundary section")
    if "V6_Support_Process_Registry.md" not in ops:
        failures.append("OPS runtime runbook does not reference support process registry")
    if "V6_Support_Process_Registry.md" not in index:
        failures.append("docs index does not classify support process registry")

    if failures:
        print("SUPPORT_PROCESS_REGISTRY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "SUPPORT_PROCESS_REGISTRY: OK "
        f"processes={len(PROCESSES)} fields={len(CONTRACT_FIELDS)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
