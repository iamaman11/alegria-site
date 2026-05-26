#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/local_operational_evidence_bundle.json"

REQUIRED_TOP_LEVEL = {
    "artifact_id": str,
    "environment_key": str,
    "evidence_status": str,
    "status_reason": str,
    "blocking_reasons": list,
    "external_blockers": list,
    "checks": dict,
    "artifacts": dict,
    "required_for_pass": list,
    "updated_at": str,
}

ALLOWED_STATUS = {"PASS", "BLOCKED_ON_LIVE_PROVIDER", "FAIL"}


def main() -> int:
    if not ARTIFACT.exists():
        print("LOCAL_OPERATIONAL_EVIDENCE_SCHEMA: FAILED")
        print(f"- missing {ARTIFACT.relative_to(ROOT)}")
        return 1

    try:
        payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("LOCAL_OPERATIONAL_EVIDENCE_SCHEMA: FAILED")
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

    if payload.get("artifact_id") != "local_operational_evidence_bundle":
        failures.append("artifact_id must equal `local_operational_evidence_bundle`")
    if payload.get("evidence_status") not in ALLOWED_STATUS:
        failures.append(f"evidence_status must be one of {sorted(ALLOWED_STATUS)}")

    checks = payload.get("checks", {})
    if isinstance(checks, dict):
        for key in [
            "clean_acceptance_bundle",
            "release_restore_gate",
            "legacy_replay_evidence",
            "live_provider_minimal_scope",
        ]:
            if key not in checks:
                failures.append(f"checks missing `{key}`")

    artifacts = payload.get("artifacts", {})
    if isinstance(artifacts, dict):
        for key in [
            "clean_acceptance_bundle",
            "release_restore_gate",
            "legacy_replay_evidence",
            "live_provider_minimal_scope",
        ]:
            if key not in artifacts:
                failures.append(f"artifacts missing `{key}`")

    if failures:
        print("LOCAL_OPERATIONAL_EVIDENCE_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "LOCAL_OPERATIONAL_EVIDENCE_SCHEMA: OK "
        f"status={payload['evidence_status']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
