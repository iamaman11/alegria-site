#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    result = subprocess.run(
        [
            "bash",
            "-lc",
            "cd app/rust && SQLX_OFFLINE=true cargo test -q -p seo_application scenario_report_surface_serializes_phase_reports && "
            "SQLX_OFFLINE=true cargo test -q -p cli_tools --no-run && "
            "SQLX_OFFLINE=true cargo test -q -p integration_harness --features e2e synthetic_full_scenario_persists_page_draft_on_full_harness",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
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
