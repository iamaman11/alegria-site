#!/usr/bin/env python3
"""Exercise the actual manual LLM inbox against a disposable PostgreSQL 18 container."""
from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    # Real disposable PostgreSQL state transitions and negative submission cases.
    # Then exercise the actual canonical Temporal workflow, not a mock runner.
    for test_name in (
        "manual_llm_queue_persists_validates_replays_and_recovers",
        "manual_truth_inbox_refuses_fabricated_evidence_and_replays_atomically",
        "canonical_temporal_manual_editorial_resumes_after_worker_restart",
    ):
        result = subprocess.run(
            [
                "cargo",
                "test",
                "-p",
                "integration_harness",
                "--features",
                "e2e",
                test_name,
            ],
            cwd=ROOT / "app" / "rust",
            check=False,
        )
        if result.returncode:
            print(f"MANUAL_LLM_E2E: FAILED test={test_name}", flush=True)
            return result.returncode

    print("MANUAL_LLM_E2E: OK", flush=True)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
