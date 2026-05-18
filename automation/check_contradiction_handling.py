#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r3_contradiction_handling.json"
CONTRADICTION = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "contradiction_gate_step.rs"
HITL = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "hitl_decision_step.rs"


def main() -> int:
    failures: list[str] = []

    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("CONTRADICTION_HANDLING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("CONTRADICTION_HANDLING: FAILED")
        print(f"- invalid json: {exc}")
        return 1

    for key in [
        "artifact_id",
        "step",
        "evidence_status",
        "failure_class",
        "status_reason",
        "contract_checks",
        "smoke_command",
        "updated_at",
    ]:
        if key not in artifact:
            failures.append(f"artifact missing `{key}`")

    if artifact.get("artifact_id") != "r3_contradiction_handling":
        failures.append("artifact_id must equal `r3_contradiction_handling`")
    if artifact.get("step") != "R3.Step3":
        failures.append("step must equal `R3.Step3`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    for path, needles in [
        (
            CONTRADICTION,
            ["block_publish", "hitl_review", "pass", "critical", "high", "medium"],
        ),
        (
            HITL,
            ["contradiction_blocked", "hard_block", "fact_conflict", "queue_hitl"],
        ),
    ]:
        text = path.read_text(encoding="utf-8")
        for needle in needles:
            if needle not in text:
                failures.append(f"{path.name} missing `{needle}`")

    if failures:
        print("CONTRADICTION_HANDLING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CONTRADICTION_HANDLING: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
