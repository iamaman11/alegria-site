#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    proc = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "seo_steps",
            "seo_flow_tests::seo_steps_form_publish_ready_pipeline",
            "--",
            "--exact",
            "--nocapture",
        ],
        cwd=ROOT / "app/rust",
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print("SMOKE_SERP_TOP10_PATTERN_PIPELINE: FAILED")
        if proc.stdout.strip():
            print(proc.stdout.strip())
        if proc.stderr.strip():
            print(proc.stderr.strip())
        return proc.returncode

    print("SMOKE_SERP_TOP10_PATTERN_PIPELINE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
