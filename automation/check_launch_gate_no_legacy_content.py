#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "automation/temporal_production_gate.sh"
RUNBOOK = ROOT / "docs/OPS_RUNTIME_RUNBOOK.md"


def main() -> int:
    gate = GATE.read_text(encoding="utf-8")
    runbook = RUNBOOK.read_text(encoding="utf-8")
    failures: list[str] = []

    if "run_content_workflow" in gate:
        failures.append("temporal gate still references run_content_workflow")
    if "content-generation" in gate:
        failures.append("temporal gate still starts legacy content-generation workflow")
    if "run_seo_site_build_workflow" not in gate:
        failures.append("temporal gate missing run_seo_site_build_workflow")
    if "legacy-only" not in runbook:
        failures.append("runbook does not mark ContentGenerationWorkflow as legacy-only")

    if failures:
        print("LAUNCH_GATE_NO_LEGACY_CONTENT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("LAUNCH_GATE_NO_LEGACY_CONTENT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
