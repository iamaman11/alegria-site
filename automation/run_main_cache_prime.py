#!/usr/bin/env python3
"""Warm only compiler units exercised by exact-source scoped PR Rust feedback.

This trusted-main-only runner consumes the same `ci_impact.py` report and
`dev_rust_feedback.py` Cargo plan as the PR gate. It *compiles* the selected
tests without executing them: live integration and full test acceptance stay
solely in the mandatory PR gate. No second Cargo dependency graph or hard-coded
set of packages is maintained here.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from cargo_feedback import run_cargo
from ci_impact import ROOT, SUITE_OWNERS, WORKSPACE, suite_commands_for_report
from dev_rust_feedback import commands_for_report


def prime_commands_for_report(report: dict) -> list[list[str]]:
    """Mirror PR compilation profiles while omitting test execution.

    Cargo's `check` and test profiles produce distinct compiler objects. Only
    check the PR's reverse consumers and compile the PR's direct package tests.
    The E2E feature profile is needed only if a real selected integration suite
    would run with `integration_harness --features e2e --lib` on that source.
    """
    commands = commands_for_report(report, check_only=False)
    selected_suites = suite_commands_for_report(report)
    prime: list[list[str]] = []
    for command in commands:
        if command[:2] == ["cargo", "test"]:
            if "--" in command or "--no-run" in command:
                raise ValueError("unexpected test runner flags in scoped Cargo plan")
            prime.append([*command, "--no-run"])
        elif command[:2] == ["cargo", "check"]:
            prime.append(command)
        else:
            raise ValueError("unexpected Cargo command in scoped feedback plan")

    # The real CI runner prepares these binaries before suites that spawn
    # them. Reuse their declared commands, never maintain a second binary list.
    # Multiple suites can share the same compilation and must prime it once.
    prepared: set[tuple[str, ...]] = set()
    for name, _ in selected_suites:
        preparation = SUITE_OWNERS[name].get("prepare")
        if preparation and preparation not in prepared:
            if (preparation[:2] != ("cargo", "build")
                    or "--locked" not in preparation or "--release" in preparation):
                raise ValueError("unsafe runtime preparation in integration owner")
            prime.append(list(preparation))
            prepared.add(preparation)

    if selected_suites:
        prime.append([
            "cargo", "test", "--locked", "--package", "integration_harness",
            "--features", "e2e", "--lib", "--no-run",
        ])
    return prime


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--source-sha", required=True)
    args = parser.parse_args()
    actual_sha = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
    ).strip()
    if actual_sha != args.source_sha:
        raise ValueError("MAIN_CACHE_PRIME=FAIL checked out source differs from accepted main")
    report = json.loads(args.report.read_text(encoding="utf-8"))
    if (report.get("source_sha") != actual_sha or report.get("head_sha") != actual_sha
            or report.get("base_sha") != args.base_sha or report.get("base") != args.base_sha):
        raise ValueError("MAIN_CACHE_PRIME=FAIL impact plan not bound to exact accepted main SHA")

    commands = prime_commands_for_report(report)
    if not commands:
        print("MAIN_CACHE_PRIME=SKIPPED planner found no affected compiler units", flush=True)
        return 0
    print(
        f"MAIN_CACHE_PRIME=SCOPED analysis={report['analysis']} "
        f"affected={','.join(report['affected_crates']) or '-'} "
        f"suites={','.join(report['selected_suites']) or '-'} commands={len(commands)}",
        flush=True,
    )
    measurements = []
    for command in commands:
        print("MAIN_CACHE_PRIME_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))
    # This is only compiler telemetry, not a test PASS or substitute for PR CI.
    print("MAIN_CACHE_PRIME=COMPILED measurements=" + str(len(measurements)), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
