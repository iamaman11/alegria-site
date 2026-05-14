#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build.rs"

REQUIRED_TOKENS = [
    "build_execution_plan(",
    "initial_phase_keys()",
    "planning_phase_keys(",
    "page_phase_keys(",
    "final_phase_keys(",
    "next_phase(",
]

FORBIDDEN_PHASE_ASSIGNMENTS = [
    's.phase = "serp_ingest".to_string()',
    's.phase = "crawl_sources".to_string()',
    's.phase = "raw_knowledge_ingestion".to_string()',
    's.phase = "serp_normalize".to_string()',
    's.phase = "opportunity_build".to_string()',
    's.phase = "ia_build".to_string()',
    's.phase = "link_recommend".to_string()',
    's.phase = "global_site_reconcile".to_string()',
]


def main() -> int:
    text = WORKFLOW.read_text(encoding="utf-8")
    failures: list[str] = []
    for token in REQUIRED_TOKENS:
        if token not in text:
            failures.append(f"missing workflow execution-plan token `{token}`")
    for token in FORBIDDEN_PHASE_ASSIGNMENTS:
        if token in text:
            failures.append(f"workflow still hard-codes phase ordering token `{token}`")
    if failures:
        print("TEMPORAL_WORKFLOW_EXECUTION_PLAN: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("TEMPORAL_WORKFLOW_EXECUTION_PLAN: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
