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
            "cd app/rust && SQLX_OFFLINE=true cargo test -q -p infrastructure --lib",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        print("REBUILD_QUEUE_EXECUTION_PATH_SMOKE: FAILED")
        if result.stdout.strip():
            print(result.stdout.rstrip())
        if result.stderr.strip():
            print(result.stderr.rstrip())
        return result.returncode

    print("REBUILD_QUEUE_EXECUTION_PATH_SMOKE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
