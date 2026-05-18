#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_run_report_metrics_surface.json"
SCENARIO = ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "scenario.rs"
CLI = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"
METRICS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "metrics.rs"
GATE = ROOT / "automation" / "temporal_production_gate.sh"
HARNESS = ROOT / "app" / "rust" / "crates" / "integration_harness" / "src" / "lib.rs"


def main() -> int:
    failures: list[str] = []

    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("RUN_REPORT_COMPLETENESS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("RUN_REPORT_COMPLETENESS: FAILED")
        print(f"- invalid json: {exc}")
        return 1

    for key in [
        "artifact_id",
        "step",
        "evidence_status",
        "failure_class",
        "status_reason",
        "smoke_command",
        "contract_checks",
        "updated_at",
    ]:
        if key not in artifact:
            failures.append(f"artifact missing `{key}`")

    if artifact.get("artifact_id") != "r4_run_report_metrics_surface":
        failures.append("artifact_id must equal `r4_run_report_metrics_surface`")
    if artifact.get("step") != "R4.Step5":
        failures.append("step must equal `R4.Step5`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    scenario = SCENARIO.read_text(encoding="utf-8")
    cli = CLI.read_text(encoding="utf-8")
    metrics = METRICS.read_text(encoding="utf-8")
    gate = GATE.read_text(encoding="utf-8")
    harness = HARNESS.read_text(encoding="utf-8")

    for needle in [
        "pub struct SeoPhaseReport",
        "pub struct SeoScenarioResult",
        "Serialize, Deserialize",
        "phase_reports: Vec<SeoPhaseReport>",
        "scenario_report_surface_serializes_phase_reports",
    ]:
        if needle not in scenario:
            failures.append(f"scenario.rs missing `{needle}`")

    for needle in [
        "SEO_RUN_RESULT scenario=",
        "SEO_RUN_PHASE phase=",
        "result.phase_reports",
    ]:
        if needle not in cli:
            failures.append(f"cli_tools run report surface missing `{needle}`")

    for needle in [
        "workflow_starts_total",
        "workflow_completions_total",
        "support_bundle_events_total",
        "publish_build_scope_total",
        "/metrics",
    ]:
        if needle not in metrics:
            failures.append(f"metrics.rs missing `{needle}`")

    for needle in [
        "checking metrics endpoint",
        "step_execution_reused_total",
    ]:
        if needle not in gate:
            failures.append(f"production gate missing `{needle}`")

    for needle in [
        "synthetic_full_scenario_persists_page_draft_on_full_harness",
        "phase_reports.iter().any",
    ]:
        if needle not in harness:
            failures.append(f"integration harness missing `{needle}`")

    if failures:
        print("RUN_REPORT_COMPLETENESS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RUN_REPORT_COMPLETENESS: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
