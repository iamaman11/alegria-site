#!/usr/bin/env python3
"""Execute the exact-source Cargo plan from the shared local/CI impact model.

This is a runner, not a second impact planner. It consumes the single
`ci_impact.py` report and calls `dev_rust_feedback.commands_for_report`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from cargo_feedback import run_cargo
from ci_impact import ROOT, WORKSPACE
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

    measurements = []
    for command in commands:
        print("CI_RUST_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

    required_a17 = report.get("publishing_a17_required")
    if not isinstance(required_a17, bool):
        raise ValueError("impact report omitted the fail-closed Publishing A17 decision")
    if required_a17:
        command = [
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "publishing_a17_postgres_outbox_http_faults_and_recovery",
            "--locked", "--", "--nocapture",
        ]
        print("CI_PUBLISHING_A17_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

    required_a14 = report.get("outbox_a14_required")
    if not isinstance(required_a14, bool):
        raise ValueError("impact report omitted the fail-closed Outbox A14 decision")
    if required_a14:
        command = [
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "outbox_a14_", "--locked", "--", "--nocapture",
        ]
        print("CI_OUTBOX_A14_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

    required_a13 = report.get("knowledge_a13_required")
    if not isinstance(required_a13, bool):
        raise ValueError("impact report omitted the fail-closed Knowledge A13 decision")
    if required_a13:
        command = [
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "knowledge_a13_", "--locked", "--", "--nocapture",
        ]
        print("CI_KNOWLEDGE_A13_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

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
