#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r4_quality_policy_registry.json"
POLICY = ROOT / "app" / "rust" / "crates" / "policies" / "src" / "quality_policy.rs"
STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "quality_policy_step.rs"
PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
RULE_TYPES = ROOT / "app" / "rust" / "crates" / "runtime_models" / "src" / "lib.rs"


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("QUALITY_POLICY_REGISTRY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("QUALITY_POLICY_REGISTRY: FAILED")
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

    if artifact.get("artifact_id") != "r4_quality_policy_registry":
        failures.append("artifact_id must equal `r4_quality_policy_registry`")
    if artifact.get("step") != "R4.Step4":
        failures.append("step must equal `R4.Step4`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    policy = POLICY.read_text(encoding="utf-8")
    step = STEP.read_text(encoding="utf-8")
    proto = PROTO.read_text(encoding="utf-8")
    rules = RULE_TYPES.read_text(encoding="utf-8")

    for needle in [
        "pub struct QualityPolicy",
        "pub struct QualityPolicyScore",
        "resolve_quality_policy",
        "score_quality",
        "min_quality_score",
        "min_supported_claims",
        "max_missing_required_sections",
    ]:
        if needle not in policy:
            failures.append(f"policy registry missing `{needle}`")

    for needle in [
        "QualityPolicyEvaluationInputPayload",
        "QualityPolicyEvaluationOutputPayload",
        "resolve_quality_policy",
        "score_quality",
    ]:
        if needle not in step:
            failures.append(f"quality policy step missing `{needle}`")

    for needle in [
        "message QualityPolicyEvaluationInputPayload",
        "message QualityPolicyEvaluationOutputPayload",
    ]:
        if needle not in proto:
            failures.append(f"proto missing `{needle}`")

    if "quality_policy" in rules and "rule_types" in rules:
        failures.append("quality policy must not live in rule_types registry")

    if failures:
        print("QUALITY_POLICY_REGISTRY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("QUALITY_POLICY_REGISTRY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
