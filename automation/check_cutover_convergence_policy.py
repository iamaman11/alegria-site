#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "seo_cutover_convergence_policy_2026-05-22.json"
WORKING_PLAN = ROOT / "docs" / "V6_SeoSiteBuildWorkflow_Working_Plan.md"
OWNER = ROOT / "docs" / "V6_Expert_Truth_Graph_Runtime.md"
RUNBOOK = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"
STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
WORKFLOWS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
LEGACY = ROOT / "docs" / "runs" / "seo_site_build_legacy_replay_evidence.json"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "temporal_starter.rs":
        sub_dir = path.parent / "temporal_starter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []

    if not ARTIFACT.exists():
        failures.append(f"missing {ARTIFACT.relative_to(ROOT)}")
    else:
        payload = json.loads(read(ARTIFACT))
        for key in [
            "artifact_id",
            "status",
            "legacy_workflow_type",
            "forward_workflow_type",
            "compat_drain_window",
            "expert_workflow_policy",
            "workflow_naming_decision",
            "evidence_refs",
            "updated_at",
        ]:
            if key not in payload:
                failures.append(f"artifact missing `{key}`")

        if payload.get("legacy_workflow_type") != "SeoSiteBuildWorkflow":
            failures.append("legacy_workflow_type must equal `SeoSiteBuildWorkflow`")
        if payload.get("forward_workflow_type") != "SeoSiteBuildCanonicalCutoverWorkflow":
            failures.append(
                "forward_workflow_type must equal `SeoSiteBuildCanonicalCutoverWorkflow`"
            )
        if payload.get("status") not in {
            "WINDOW_OPEN",
            "LOCAL_COMPAT_DRAIN_CLOSED",
            "PRODUCTION_COMPAT_DRAIN_CLOSED",
        }:
            failures.append("artifact status has invalid value")

        expert_policy = payload.get("expert_workflow_policy", {})
        if expert_policy.get("classification") != "diagnostic-only":
            failures.append("expert_workflow_policy.classification must be `diagnostic-only`")
        if expert_policy.get("registration_gate") != "ALLOW_EXPERT_MIGRATION_WORKFLOWS=true":
            failures.append("expert workflow registration gate must equal explicit env opt-in")
        expected_retention = (
            "retain permanently as a narrow diagnostic toolset behind explicit opt-in; "
            "do not route new product logic or canonical execution through these workflows"
        )
        if expert_policy.get("retention_decision") != expected_retention:
            failures.append("expert workflow retention decision does not match the accepted permanent-diagnostic policy")
        if "deprecation_rule" not in expert_policy:
            failures.append("expert_workflow_policy missing `deprecation_rule`")

        naming = payload.get("workflow_naming_decision", {})
        expected = (
            "keep SeoSiteBuildCanonicalCutoverWorkflow as the runtime workflow type after drain; "
            "do not create a new workflow type purely to rename it"
        )
        if naming.get("decision") != expected:
            failures.append("workflow naming decision does not match canonical convergence rule")

    legacy_payload = json.loads(read(LEGACY))
    inventory = legacy_payload.get("history_source", {}).get("inventory", {})
    if legacy_payload.get("evidence_status") != "PASS":
        failures.append("legacy replay evidence must be PASS")
    if inventory.get("total_runs") != 0:
        failures.append("legacy replay inventory total_runs must equal 0 for local drain closure")
    if inventory.get("open_runs") != 0:
        failures.append("legacy replay inventory open_runs must equal 0 for local drain closure")

    for path, needles in [
        (
            WORKING_PLAN,
            [
                "compat/drain window",
                "diagnostic-only",
                "ALLOW_EXPERT_MIGRATION_WORKFLOWS=true",
                "do not create a new workflow type purely to rename it",
                "permanent narrow diagnostic toolset",
            ],
        ),
        (
            OWNER,
            [
                "Compat/drain workflow still present during the rollout window",
                "ALLOW_EXPERT_MIGRATION_WORKFLOWS=true",
                "active forward path",
                "retained as permanent narrow diagnostics",
            ],
        ),
        (
            RUNBOOK,
            [
                "compat/drain workflow only; retained for replay-safe legacy histories and controlled comparison",
                "disabled by default; requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`",
                "permanent narrow diagnostic surfaces",
            ],
        ),
        (
            STARTER,
            [
                "SeoSiteBuildLegacyCompat",
                "ALLOW_EXPERT_MIGRATION_WORKFLOWS",
                "Expert migration workflows are diagnostic-only.",
            ],
        ),
        (
            WORKFLOWS,
            [
                "ALLOW_EXPERT_MIGRATION_WORKFLOWS",
            ],
        ),
    ]:
        text = read_file_resolved(path)
        for needle in needles:
            if needle not in text:
                failures.append(f"{path.relative_to(ROOT)} missing `{needle}`")

    if failures:
        print("CUTOVER_CONVERGENCE_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CUTOVER_CONVERGENCE_POLICY: OK status=LOCAL_COMPAT_DRAIN_CLOSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
