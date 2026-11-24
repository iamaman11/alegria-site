#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs" / "runs" / "r3_deterministic_fallback_publish_blocking.json"
CMS = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "cms_publish_step.rs"
CMS_POLICY = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src" / "cms_revision.rs"
QA = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "draft_qa_step.rs"


def main() -> int:
    failures: list[str] = []
    if not ARTIFACT.exists():
        failures.append(f"missing artifact `{ARTIFACT.relative_to(ROOT)}`")
        print("DETERMINISTIC_FALLBACK_PUBLISH_BLOCKING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    try:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    except Exception as exc:
        print("DETERMINISTIC_FALLBACK_PUBLISH_BLOCKING: FAILED")
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

    if artifact.get("artifact_id") != "r3_deterministic_fallback_publish_blocking":
        failures.append("artifact_id must equal `r3_deterministic_fallback_publish_blocking`")
    if artifact.get("step") != "R3.Step6":
        failures.append("step must equal `R3.Step6`")
    if artifact.get("evidence_status") != "PASS":
        failures.append("evidence_status must equal `PASS`")
    if artifact.get("failure_class") != "none":
        failures.append("failure_class must equal `none`")

    cms = CMS.read_text(encoding="utf-8")
    cms_policy = CMS_POLICY.read_text(encoding="utf-8")
    if not QA.exists():
        failures.append(f"missing QA step `{QA.relative_to(ROOT)}`")

    # The domain owns the publication rule. The Temporal adapter must build a
    # typed candidate, invoke that owner, and expose its canonical reason code.
    for needle in [
        'candidate.llm_provider == "deterministic_fallback"',
        "CmsPublishGateViolation::DeterministicFallbackPublishBlocked",
        "deterministic_fallback_publish_blocked",
    ]:
        if needle not in cms_policy:
            failures.append(f"cms domain policy missing `{needle}`")
    for needle in [
        "cms_publication_blockers(",
        "CmsPublicationGate {",
        "llm_provider: &draft.llm_provider_key",
        "CmsRevisionState::decide",
        "fn deterministic_fallback_blocks_publish()",
        'contains(&"deterministic_fallback_publish_blocked".to_string())',
    ]:
        if needle not in cms:
            failures.append(f"cms wire adapter missing policy invocation/test `{needle}`")
    if 'llm_provider_key == "deterministic_fallback"' in cms.split("#[cfg(test)]", 1)[0]:
        failures.append("cms wire adapter must not duplicate deterministic-fallback policy")

    if failures:
        print("DETERMINISTIC_FALLBACK_PUBLISH_BLOCKING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("DETERMINISTIC_FALLBACK_PUBLISH_BLOCKING: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
