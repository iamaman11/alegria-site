#!/usr/bin/env python3
from __future__ import annotations

import os
import subprocess
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    # Ready PR acceptance requires rust-regressions to run the full workspace.
    # Both policies and seo_steps quality_policy tests are already owned there.
    # Keep the full native Cargo smoke for standalone/local audits.
    if os.environ.get("CI_WORKSPACE_TESTED") == "true":
        print("QUALITY_POLICY_REGISTRY_SMOKE: REUSED=rust-regressions/workspace")
        return 0

    result = subprocess.run(
        [
            "bash",
            "-lc",
            "cd app/rust && SQLX_OFFLINE=true cargo test -q -p policies quality_policy && "
            "SQLX_OFFLINE=true cargo test -q -p seo_steps quality_policy_step",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        print("QUALITY_POLICY_REGISTRY_SMOKE: FAILED")
        if result.stdout.strip():
            print(result.stdout.rstrip())
        if result.stderr.strip():
            print(result.stderr.rstrip())
        return result.returncode

    print("QUALITY_POLICY_REGISTRY_SMOKE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
