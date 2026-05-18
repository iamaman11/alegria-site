#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def run_filter(filter_name: str) -> None:
    result = subprocess.run(
        [
            "bash",
            "-lc",
            "cd app/rust && SQLX_OFFLINE=true cargo test -q -p seo_steps "
            f"{filter_name}",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        print("CONTRADICTION_HANDLING_SMOKE: FAILED")
        if result.stdout.strip():
            print(result.stdout.rstrip())
        if result.stderr.strip():
            print(result.stderr.rstrip())
        raise SystemExit(result.returncode)


def main() -> int:
    run_filter("contradiction_gate_step")
    run_filter("hitl_decision_step")
    print("CONTRADICTION_HANDLING_SMOKE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
