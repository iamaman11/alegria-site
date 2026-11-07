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
        check(root, publish=True, published_pages=2, expected_success=False)
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
        # A one-scenario manifest cannot certify full R5 even if its only
        # scenario contains genuine canonical publication evidence.
        singleton_report = root / "singleton.md"
        singleton_report.write_text(
            "## Test\n\n```json\n" + json.dumps(payload) + "\n```\n",
            encoding="utf-8",
        )
        singleton_manifest = root / "singleton.json"
        singleton_manifest.write_text(json.dumps({
            "evidence_status": "PASS",
            "scenarios": [{"scenario_id": "factual_change_rebuild",
                           "report_path": str(singleton_report)}],
        }), encoding="utf-8")
        single = subprocess.run(
            [sys.executable, str(CHECKER), str(singleton_manifest)],
            capture_output=True, text=True, check=False,
        )
        assert single.returncode != 0 and "three distinct" in single.stdout, single.stdout
        # Positive contract: all three distinct canonical runs, genuine
        # publication, and independent factual-change/HTML evidence.
        scenarios = []
        for scenario_id in ("fresh_scope", "scope_expansion", "factual_change_rebuild"):
            full = dict(payload, scenario_id=scenario_id)
            report_file = root / f"complete_{scenario_id}.md"
            report_file.write_text(
                "## Scenario\n\n```json\n" + json.dumps(full) + "\n```\n",
                encoding="utf-8",
            )
            scenarios.append({"scenario_id": scenario_id, "report_path": str(report_file)})
        complete_manifest = root / "complete.json"
        complete_manifest.write_text(
            json.dumps({"evidence_status": "PASS", "scenarios": scenarios}),
            encoding="utf-8",
        )
        complete = subprocess.run(
            [sys.executable, str(CHECKER), str(complete_manifest)],
            capture_output=True, text=True, check=False,
        )
        assert complete.returncode == 0, complete.stdout + complete.stderr
    print("E2E_PUBLICATION_VERDICT_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
