#!/usr/bin/env python3
"""Fast, non-acceptance Cargo feedback for directly changed Rust crates."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    if report.get("schema_version") != 2 or report.get("mandatory_full_ci_gate") is not True:
        raise ValueError("invalid or unsafe Rust impact report")

    changed = report["direct_crates"] if report["analysis"] == "scoped" else []
    if len(changed) > 3:
        print("SCOPED_FEEDBACK=DEFER_TO_FULL_WORKSPACE (broad direct crate change)")
        return 0
    for name in changed:
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            raise ValueError(f"invalid crate in Rust impact report: {name!r}")
    if not changed:
        print("SCOPED_FEEDBACK=NO_CRATE_LOCAL_CHANGE (full acceptance still mandatory)")
        return 0

    command = ["cargo", "check", "--locked"]
    for name in changed:
        command.extend(["--package", name])
    print(f"SCOPED_FEEDBACK_CHECK={','.join(changed)}", flush=True)
    subprocess.run(command, cwd=WORKSPACE, check=True)
    print("SCOPED_FEEDBACK=PASS (not a release/merge verdict)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
