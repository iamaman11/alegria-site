#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "docs" / "STEP_CATALOG_CONTRACT.md"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
RUNTIME_PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"

REQUIRED_CONTRACT_FIELDS = [
    "retry_class",
    "registry_version",
    "executor_version",
    "idempotency_key",
]

LEDGER_BACKED_STEPS = [
    "extract_facts",
    "verify_rules",
    "persist_and_emit",
    "generate_content",
    "validate_blocks",
    "load_seo_site_build_input",
    "serp_ingest",
    "serp_normalize",
    "opportunity_build",
    "ia_build",
    "link_recommend",
    "draft_assemble",
    "draft_qa",
    "cms_publish",
    "rebuild_detect",
]


def main() -> int:
    contract = CONTRACT.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    proto = RUNTIME_PROTO.read_text(encoding="utf-8")
    failures: list[str] = []

    for field in REQUIRED_CONTRACT_FIELDS:
        if field not in contract:
            failures.append(f"STEP_CATALOG_CONTRACT missing `{field}`")
        if field not in proto:
            failures.append(f"StepContractMeta missing `{field}`")
    for step in LEDGER_BACKED_STEPS:
        if f"\"{step}\"" not in activities:
            failures.append(f"activity surface missing ledger step `{step}`")
    if "execute_step(" not in activities:
        failures.append("activity surface is not using execute_step")

    if failures:
        print("STEP_CATALOG_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("STEP_CATALOG_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
