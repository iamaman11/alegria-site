#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r3_hitl_operator_surface.json"
STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "hitl_operator_surface_step.rs"


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("HITL_OPERATOR_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("HITL_OPERATOR_SURFACE: FAILED")
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

    if artifact.get("artifact_id") != "r3_hitl_operator_surface":
        failures.append("artifact_id must equal `r3_hitl_operator_surface`")
    if artifact.get("step") != "R3.Step7":
        failures.append("step must equal `R3.Step7`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    text = STEP.read_text(encoding="utf-8")
    for needle in [
        "list",
        "show",
        "approve",
        "reject",
        "request_changes",
        "registry_extension",
        "registry_extension_allowed",
    ]:
        if needle not in text:
            failures.append(f"hitl surface missing `{needle}`")

    if failures:
        print("HITL_OPERATOR_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("HITL_OPERATOR_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
