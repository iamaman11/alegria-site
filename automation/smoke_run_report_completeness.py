#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from pathlib import Path
import sys
import os
import time


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    # Identical real Cargo checks, now timed individually. Shell-chaining used
    # to hide which of these three checks/compiles consumed several minutes.
    steps = (
        ("report_surface", ["cargo", "test", "-q", "-p", "seo_application", "scenario_report_surface_serializes_phase_reports"]),
        ("cli_tools_compile", ["cargo", "test", "-q", "-p", "cli_tools", "--no-run"]),
        ("full_harness", ["cargo", "test", "-q", "-p", "integration_harness", "--features", "e2e", "draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness"]),
    )
    for name, command in steps:
        started = time.perf_counter_ns()
        result = subprocess.run(
            command,
            cwd=ROOT / "app" / "rust",
            env={**os.environ, "SQLX_OFFLINE": "true"},
            check=False,
            text=True,
            capture_output=True,
        )
        print(
            f"RUN_REPORT_STAGE_TIME stage={name} elapsed_ms={(time.perf_counter_ns() - started) // 1_000_000} exit_code={result.returncode}",
            flush=True,
        )
        if result.returncode != 0:
            print("RUN_REPORT_COMPLETENESS_SMOKE: FAILED")
            if result.stdout.strip():
                print(result.stdout.rstrip())
            if result.stderr.strip():
                print(result.stderr.rstrip())
            return result.returncode

    print("RUN_REPORT_COMPLETENESS_SMOKE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
