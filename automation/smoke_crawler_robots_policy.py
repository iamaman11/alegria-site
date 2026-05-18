#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main() -> int:
    proc = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "infrastructure",
            "--lib",
            "adapters::raw_crawl_adapter::tests::robots_policy_and_redirect_trace_smoke",
            "--",
            "--exact",
            "--nocapture",
        ],
        capture_output=True,
        text=True,
        cwd=Path(__file__).resolve().parents[1] / "app/rust",
    )
    if proc.returncode != 0:
        print("SMOKE_CRAWLER_ROBOTS_POLICY: FAILED")
        if proc.stdout.strip():
            print(proc.stdout.strip())
        if proc.stderr.strip():
            print(proc.stderr.strip())
        return proc.returncode

    print("SMOKE_CRAWLER_ROBOTS_POLICY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
