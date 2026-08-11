#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "docs" / "runs" / "r5_supersite_certification_manifest.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_report_payload(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    matches = re.findall(r"```json\s*(\{.*?\})\s*```", text, flags=re.DOTALL)
    if not matches:
        raise ValueError(f"missing JSON evidence block in {path}")
    return json.loads(matches[-1])


def validate_scenario(payload: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    scenario = str(payload.get("scenario_id", "unknown"))
    evidence_status = str(payload.get("evidence_status", ""))
    result = payload.get("result") if isinstance(payload.get("result"), dict) else {}
    result_status = str(result.get("status", ""))
    page_total = int(result.get("page_total", 0) or 0)
    published_pages = int(result.get("published_pages", 0) or 0)
    publish_requested = bool(payload.get("publish_requested", False))

    if evidence_status != "PASS":
        failures.append(f"{scenario}: evidence_status={evidence_status!r}, expected PASS")
    if not result:
        failures.append(f"{scenario}: missing parsed SEO_RUN_RESULT")
        return failures
    if not result_status or result_status.startswith("empty:"):
        failures.append(f"{scenario}: product result is not usable: status={result_status!r}")
    if page_total <= 0:
        failures.append(f"{scenario}: page_total={page_total}, expected > 0")
    if publish_requested and published_pages <= 0:
        failures.append(
            f"{scenario}: publish was requested but published_pages={published_pages}"
        )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", nargs="?", default=str(DEFAULT_MANIFEST))
    args = parser.parse_args()

    manifest_path = Path(args.manifest)
    if not manifest_path.is_absolute():
        manifest_path = ROOT / manifest_path
    if not manifest_path.exists():
        print("E2E_PRODUCT_VERDICT: FAILED")
        print(f"- missing certification manifest: {manifest_path}")
        return 1

    manifest = load_json(manifest_path)
    failures: list[str] = []
    if manifest.get("evidence_status") != "PASS":
        failures.append(
            f"manifest evidence_status={manifest.get('evidence_status')!r}, expected PASS"
        )

    scenarios = manifest.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        failures.append("manifest contains no certification scenarios")
    else:
        for scenario in scenarios:
            if not isinstance(scenario, dict):
                failures.append("manifest contains malformed scenario entry")
                continue
            report_path_raw = scenario.get("report_path")
            if not report_path_raw:
                failures.append(
                    f"{scenario.get('scenario_id', 'unknown')}: missing report_path"
                )
                continue
            report_path = ROOT / str(report_path_raw)
            if not report_path.exists():
                failures.append(f"missing scenario report: {report_path.relative_to(ROOT)}")
                continue
            try:
                payload = load_report_payload(report_path)
            except (ValueError, json.JSONDecodeError) as exc:
                failures.append(str(exc))
                continue
            failures.extend(validate_scenario(payload))

    if failures:
        print("E2E_PRODUCT_VERDICT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("E2E_PRODUCT_VERDICT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
