#!/usr/bin/env python3
"""Execute the exact-source Cargo plan from the shared local/CI impact model.

This is a runner, not a second impact planner. It consumes the single
`ci_impact.py` report and calls `dev_rust_feedback.commands_for_report`.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path

from cargo_feedback import run_cargo
from ci_impact import ROOT, SUITE_OWNERS, WORKSPACE, suite_commands_for_report
from dev_rust_feedback import commands_for_report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    actual_sha = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if report.get("source_sha") != actual_sha:
        raise ValueError("Rust impact report does not match the checked-out source SHA")
    commands = commands_for_report(report, check_only=False)
    integration_commands = suite_commands_for_report(report)

    measurements = []
    for command in commands:
        print("CI_RUST_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

    # All real integration commands and their owners come from ci_impact.py.
    # The runner deliberately has no suite-specific branch or path logic.
    for name, command in integration_commands:
        owner = SUITE_OWNERS[name]
        preparation = owner.get("prepare")
        if preparation:
            print(f"CI_SUITE_PREPARE={name} COMMAND=" + " ".join(preparation), flush=True)
            measurements.append(run_cargo(list(preparation), cwd=WORKSPACE))

        # The prebuilt-binary hook is generic. A runtime suite can use its
        # existing override variables without recompiling the same Rust graph
        # under a second Cargo target directory inside the test process.
        target = Path(os.environ.get("CARGO_TARGET_DIR", WORKSPACE / "target"))
        if not target.is_absolute():
            target = WORKSPACE / target
        runtime_env = {}
        for variable, binary in owner.get("runtime_binaries", {}).items():
            path = target / "debug" / (binary + (".exe" if os.name == "nt" else ""))
            if not path.is_file() or not os.access(path, os.X_OK):
                raise FileNotFoundError(f"selected suite {name} lacks prebuilt runtime binary: {path}")
            runtime_env[variable] = str(path)
            print(f"CI_SUITE_PREBUILT={name} {variable}={path}", flush=True)
        print(f"CI_SUITE={name} COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE, env=runtime_env or None))

    output = ROOT / "automation/reports/scoped_cargo_feedback.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {"source_sha": actual_sha, "impact": report["analysis"],
             "measurements": measurements}, indent=2
        ) + "\n",
        encoding="utf-8",
    )
    print(f"CI_RUST_FEEDBACK=PASS commands={len(measurements)}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
