#!/usr/bin/env python3
"""Exercise the actual manual LLM inbox against a disposable PostgreSQL 18 container."""
from __future__ import annotations

import subprocess
import time
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
        command = [
            "cargo", "test", "-p", "integration_harness", "--features", "e2e", test_name,
        ]
        if test_name == "canonical_temporal_manual_editorial_resumes_after_worker_restart":
            # Show bounded stage timings without hiding the real Temporal run.
            command.extend(["--", "--nocapture"])
        started = time.perf_counter_ns()
        result = subprocess.run(
            command,
            cwd=ROOT / "app" / "rust",
            check=False,
        )
        print(
            f"MANUAL_LLM_E2E_TIME test={test_name} elapsed_ms={(time.perf_counter_ns() - started) // 1_000_000} exit_code={result.returncode}",
            flush=True,
        )
        if result.returncode:
            print(f"MANUAL_LLM_E2E: FAILED test={test_name}", flush=True)
            return result.returncode

    print("MANUAL_LLM_E2E: OK", flush=True)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
