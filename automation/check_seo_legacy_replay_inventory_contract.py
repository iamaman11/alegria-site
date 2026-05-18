#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"


def main() -> int:
    payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    failures: list[str] = []

    history_source = payload.get("history_source")
    if not isinstance(history_source, dict):
        failures.append("history_source must be object")
    else:
        inventory = history_source.get("inventory")
        if not isinstance(inventory, dict):
            failures.append("history_source.inventory must be object")
        else:
            for key in ["source", "captured_at", "total_runs", "open_runs", "done_runs", "sample_run_ids", "last_error"]:
                if key not in inventory:
                    failures.append(f"history_source.inventory missing `{key}`")
        environment_probe = history_source.get("environment_probe")
        if not isinstance(environment_probe, dict):
            failures.append("history_source.environment_probe must be object")
        else:
            if "captured_at" not in environment_probe:
                failures.append("history_source.environment_probe missing `captured_at`")
            candidates = environment_probe.get("candidates")
            if not isinstance(candidates, list) or not candidates:
                failures.append("history_source.environment_probe.candidates must be non-empty list")
            else:
                required_candidate_keys = {
                    "database_url",
                    "host",
                    "port",
                    "tcp_status",
                    "inventory_status",
                    "last_error",
                }
                for idx, candidate in enumerate(candidates):
                    if not isinstance(candidate, dict):
                        failures.append(
                            f"history_source.environment_probe.candidates[{idx}] must be object"
                        )
                        continue
                    missing = sorted(required_candidate_keys - candidate.keys())
                    for key in missing:
                        failures.append(
                            f"history_source.environment_probe.candidates[{idx}] missing `{key}`"
                        )

    if payload.get("evidence_status") not in {"PENDING_LIVE_REPLAY", "PASS", "FAIL"}:
        failures.append("evidence_status has invalid value")

    if failures:
        print("SEO_LEGACY_REPLAY_INVENTORY_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "SEO_LEGACY_REPLAY_INVENTORY_CONTRACT: OK "
        f"status={payload.get('evidence_status')}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
