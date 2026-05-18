#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r3_licensing_gate.json"
LICENSING = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "licensing_gate_step.rs"
ASSEMBLE = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "draft_assemble_step.rs"
QA = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "draft_qa_step.rs"


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("LICENSING_GATE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("LICENSING_GATE: FAILED")
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

    if artifact.get("artifact_id") != "r3_licensing_gate":
        failures.append("artifact_id must equal `r3_licensing_gate`")
    if artifact.get("step") != "R3.Step5":
        failures.append("step must equal `R3.Step5`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    licensing = LICENSING.read_text(encoding="utf-8")
    assemble = ASSEMBLE.read_text(encoding="utf-8")
    qa = QA.read_text(encoding="utf-8")

    for needle in [
        "restricted_redistribution_source",
        "non_redistributable_source",
        "validation_verdict == \"blocked\"",
        "publish_gate_blocker",
    ]:
        if needle not in licensing:
            failures.append(f"licensing gate missing `{needle}`")

    for needle in [
        "redistribution_allowed=false",
        "non_redistributable",
        "no redistribution",
        "restricted_redistribution_source",
    ]:
        if needle not in assemble:
            failures.append(f"draft assemble missing `{needle}`")

    for needle in [
        "licensing_gate_step::evaluate",
    ]:
        if needle not in qa:
            failures.append(f"draft qa missing `{needle}`")

    if failures:
        print("LICENSING_GATE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("LICENSING_GATE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
