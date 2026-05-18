#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"

REQUIRED_TOP_LEVEL = {
    "artifact_id": str,
    "workflow_type": str,
    "evidence_status": str,
    "status_reason": str,
    "worker_build_id_policy": str,
    "workflow_type_policy": str,
    "run_mode_legacy_default": str,
    "projection_barrier_contract": str,
    "history_source": dict,
    "checks_green": list,
    "required_for_pass": list,
    "updated_at": str,
}

ALLOWED_STATUS = {"PENDING_LIVE_REPLAY", "PASS", "FAIL"}


def main() -> int:
    if not ARTIFACT.exists():
        print("SEO_LEGACY_REPLAY_EVIDENCE_SCHEMA: FAILED")
        print(f"- missing {ARTIFACT.relative_to(ROOT)}")
        return 1

    try:
        payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("SEO_LEGACY_REPLAY_EVIDENCE_SCHEMA: FAILED")
        print(f"- invalid json: {exc}")
        return 1

    failures: list[str] = []
    for key, expected_type in REQUIRED_TOP_LEVEL.items():
        value = payload.get(key)
        if value is None:
            failures.append(f"missing `{key}`")
            continue
        if not isinstance(value, expected_type):
            failures.append(f"`{key}` must be {expected_type.__name__}")

    if payload.get("workflow_type") != "SeoSiteBuildWorkflow":
        failures.append("workflow_type must equal `SeoSiteBuildWorkflow`")
    if payload.get("evidence_status") not in ALLOWED_STATUS:
        failures.append(
            f"evidence_status must be one of {sorted(ALLOWED_STATUS)}"
        )
    if payload.get("run_mode_legacy_default") != "publish_with_hitl":
        failures.append("run_mode_legacy_default must equal `publish_with_hitl`")

    history_source = payload.get("history_source", {})
    if not isinstance(history_source, dict):
        failures.append("history_source must be object")
    else:
        for key in ["kind", "workflow_ids", "run_ids", "namespace", "inventory", "environment_probe"]:
            if key not in history_source:
                failures.append(f"history_source missing `{key}`")
        environment_probe = history_source.get("environment_probe")
        if not isinstance(environment_probe, dict):
            failures.append("history_source.environment_probe must be object")
        else:
            if "captured_at" not in environment_probe:
                failures.append("history_source.environment_probe missing `captured_at`")
            candidates = environment_probe.get("candidates")
            if not isinstance(candidates, list) or not candidates:
                failures.append("history_source.environment_probe.candidates must be non-empty list")

    if failures:
        print("SEO_LEGACY_REPLAY_EVIDENCE_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "SEO_LEGACY_REPLAY_EVIDENCE_SCHEMA: OK "
        f"status={payload['evidence_status']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
