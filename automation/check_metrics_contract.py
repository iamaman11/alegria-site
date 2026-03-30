#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
METRICS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "metrics.rs"
GATE = ROOT / "automation" / "temporal_production_gate.sh"


def main() -> int:
    metrics_text = METRICS.read_text(encoding="utf-8")
    gate_text = GATE.read_text(encoding="utf-8")

    required_metrics = [
        "workflow_starts_total",
        "workflow_completions_total",
        "workflow_failures_total",
        "activity_attempts_total",
        "activity_failures_total",
        "step_execution_reused_total",
        "activity_duration_seconds",
    ]

    failures: list[str] = []
    for metric in required_metrics:
        if metric not in metrics_text:
            failures.append(f"missing metric `{metric}` in {METRICS.relative_to(ROOT)}")

    if "/metrics" not in metrics_text:
        failures.append("missing `/metrics` endpoint in metrics.rs")

    if "checking metrics endpoint" not in gate_text:
        failures.append("temporal_production_gate.sh does not validate metrics endpoint")

    if "step_execution_reused_total" not in gate_text:
        failures.append("temporal_production_gate.sh does not assert step_execution_reused_total")

    if failures:
        print("METRICS_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("METRICS_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
