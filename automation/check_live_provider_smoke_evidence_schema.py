#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/live_provider_minimal_scope_evidence.json"

REQUIRED_TOP_LEVEL = {
    "artifact_id": str,
    "step": str,
    "evidence_status": str,
    "status_reason": str,
    "failure_class": str,
    "smoke_command": str,
    "scope": dict,
    "provider_requirements": dict,
    "execution": dict,
    "observed": dict,
    "required_for_pass": list,
    "updated_at": str,
}

ALLOWED_STATUS = {"PASS", "PENDING_CREDENTIALS", "FAIL"}
ALLOWED_FAILURE_CLASSES = {
    "none",
    "credential_issue",
    "rate_or_transport_issue",
    "provider_contract_issue",
    "extraction_provider_unavailable",
    "crawl_policy_failure",
    "extraction_mismatch",
}


def main() -> int:
    if not ARTIFACT.exists():
        print("LIVE_PROVIDER_SMOKE_EVIDENCE_SCHEMA: FAILED")
        print(f"- missing {ARTIFACT.relative_to(ROOT)}")
        return 1

    try:
        payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("LIVE_PROVIDER_SMOKE_EVIDENCE_SCHEMA: FAILED")
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

    if payload.get("artifact_id") != "step5_live_provider_minimal_scope":
        failures.append("artifact_id must equal `step5_live_provider_minimal_scope`")
    if payload.get("step") != "R1.Step5":
        failures.append("step must equal `R1.Step5`")
    if payload.get("evidence_status") not in ALLOWED_STATUS:
        failures.append(f"evidence_status must be one of {sorted(ALLOWED_STATUS)}")
    if payload.get("failure_class") not in ALLOWED_FAILURE_CLASSES:
        failures.append(
            f"failure_class must be one of {sorted(ALLOWED_FAILURE_CLASSES)}"
        )

    scope = payload.get("scope", {})
    if isinstance(scope, dict):
        for key in [
            "market",
            "locale",
            "country_code",
            "visa_type",
            "citizenship_code",
            "applicant_profile",
            "query",
        ]:
            if key not in scope:
                failures.append(f"scope missing `{key}`")

    requirements = payload.get("provider_requirements", {})
    if isinstance(requirements, dict):
        for key in [
            "dataforseo_login_present",
            "dataforseo_password_present",
            "psql_present",
            "truth_extraction_provider",
        ]:
            if key not in requirements:
                failures.append(f"provider_requirements missing `{key}`")

    execution = payload.get("execution", {})
    if isinstance(execution, dict) and payload.get("evidence_status") != "PENDING_CREDENTIALS":
        for key in [
            "run_id",
            "query_batch_key",
            "command_exit_code",
            "stdout_excerpt",
            "stderr_excerpt",
            "result",
            "phase_count",
        ]:
            if key not in execution:
                failures.append(f"execution missing `{key}`")

    observed = payload.get("observed", {})
    if isinstance(observed, dict) and payload.get("evidence_status") != "PENDING_CREDENTIALS":
        for key in [
            "serp_phase_status",
            "serp_persisted_snapshot_count",
            "crawl_phase_status",
            "crawl_crawled_count",
            "crawl_failed_count",
            "raw_knowledge_phase_status",
            "raw_knowledge_changed_truth_keys",
        ]:
            if key not in observed:
                failures.append(f"observed missing `{key}`")

    if failures:
        print("LIVE_PROVIDER_SMOKE_EVIDENCE_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "LIVE_PROVIDER_SMOKE_EVIDENCE_SCHEMA: OK "
        f"status={payload['evidence_status']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
