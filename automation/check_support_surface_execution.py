#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
TEMPORAL_STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
CLI_MAIN = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"
FRESHNESS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "freshness.rs"
RECONCILE = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "projection_reconcile.rs"
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
PROD_GATE = ROOT / "automation" / "temporal_production_gate.sh"
RESTORE = ROOT / "infra" / "backups" / "restore_drill.sh"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []

    starter = read(TEMPORAL_STARTER)
    cli = read(CLI_MAIN)
    registry = read(REGISTRY)
    ops = read(OPS)

    for needle in [
        "RebuildDispatch",
        "run_rebuild_dispatch(",
        "OntologyBackfillPlan",
        "run_ontology_backfill_plan(",
        "RebuildDispatchWorkflowKind",
        "report_json: Option<String>",
        "write_report(",
    ]:
        if needle not in starter:
            failures.append(f"temporal_starter missing `{needle}`")

    for needle in [
        "SeoPostPublishFeedbackProbe",
        "seo_post_publish_feedback_probe(",
        "AnalyticsClient::connect",
        "new_default_client",
        "report_json: Option<String>",
        "write_report(",
    ]:
        if needle not in cli:
            failures.append(f"cli_tools missing `{needle}`")

    for needle in [
        "SeoReleaseRestoreGate",
        "seo_release_restore_gate(",
        "run_gate_command(",
        "automation/check_temporal_build_id_policy.py",
        "automation/check_seo_rollout_compat_contract.py",
        "automation/check_backup_restore_layout.py",
        "automation/ci_verify.sh",
        "automation/temporal_production_gate.sh",
        "infra/backups/restore_drill.sh",
    ]:
        if needle not in cli:
            failures.append(f"cli_tools release gate surface missing `{needle}`")

    if "struct FreshnessCheckWorkflow" not in read(FRESHNESS):
        failures.append("freshness workflow surface missing")
    if "struct ProjectionReconcileWorkflow" not in read(RECONCILE):
        failures.append("projection reconcile workflow surface missing")

    if not PROD_GATE.exists():
        failures.append("missing automation/temporal_production_gate.sh")
    if not RESTORE.exists():
        failures.append("missing infra/backups/restore_drill.sh")

    for needle in [
        "FreshnessCheckWorkflow",
        "ProjectionReconcileWorkflow",
        "RebuildDispatch",
        "OntologyBackfillPlan",
        "SeoPostPublishFeedbackProbe",
        "SeoReleaseRestoreGate",
        "temporal_production_gate.sh",
        "restore_drill.sh",
    ]:
        if needle not in registry:
            failures.append(f"support registry missing executable surface `{needle}`")
        if needle not in ops:
            failures.append(f"ops runbook missing executable surface `{needle}`")

    if failures:
        print("SUPPORT_SURFACE_EXECUTION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SUPPORT_SURFACE_EXECUTION: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
