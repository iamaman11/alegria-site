#!/usr/bin/env python3
"""Proof that draft-only R5 scenarios never certify a published site."""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "automation/check_e2e_product_verdict.py"


def check(
    temp: Path, *, publish: bool, published_pages: int,
    mode: str = "temporal_durable", expected_success: bool,
) -> None:
    report = temp / "scenario.md"
    payload = {
        "scenario_id": "fresh_scope",
        "evidence_status": "PASS",
        "publish_requested": publish,
        "workflow_id": "9c337079-ae0b-4cb4-b052-bb3a0ca623c1",
        "result": {
            "workflow_id": "9c337079-ae0b-4cb4-b052-bb3a0ca623c1",
            "workflow_type": "SeoSiteBuildCanonicalCutoverWorkflow",
            "mode": mode,
            "truth_verified": 3,
            "status": "published",
            "page_total": 2,
            "published_pages": published_pages,
        },
    }
    report.write_text("## Scenario\n\n```json\n" + json.dumps(payload) + "\n```\n", encoding="utf-8")
    manifest = temp / "manifest.json"
    manifest.write_text(
        json.dumps({"evidence_status": "PASS", "scenarios": [
            {"scenario_id": "fresh_scope", "report_path": str(report)}
        ]}), encoding="utf-8",
    )
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(manifest)],
        text=True, capture_output=True, check=False,
    )
    if (result.returncode == 0) != expected_success:
        raise AssertionError(result.stdout + result.stderr)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="alegria-e2e-verdict-") as temp:
        root = Path(temp)
        check(root, publish=False, published_pages=0, expected_success=False)
        check(root, publish=True, published_pages=0, expected_success=False)
        check(root, publish=True, published_pages=2, mode="semi_auto_operator", expected_success=False)
        check(root, publish=True, published_pages=2, expected_success=True)
        # A scenario label alone is not sufficient to claim a factual rebuild.
        import importlib.util
        spec = importlib.util.spec_from_file_location("e2e_verdict", CHECKER)
        assert spec and spec.loader
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        payload = {
            "scenario_id": "factual_change_rebuild",
            "evidence_status": "PASS",
            "publish_requested": True,
            "workflow_id": "id-1",
            "result": {"mode": "temporal_durable", "workflow_id": "id-1",
                       "workflow_type": "SeoSiteBuildCanonicalCutoverWorkflow",
                       "truth_verified": 3, "status": "published",
                       "page_total": 2, "published_pages": 2},
        }
        assert module.validate_scenario(payload)
        payload["factual_change_evidence"] = {
            "status": "PASS", "verified_existing_rule_changes": 1,
            "modified_existing_html_pages": 1,
        }
        assert not module.validate_scenario(payload)
    print("E2E_PUBLICATION_VERDICT_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
