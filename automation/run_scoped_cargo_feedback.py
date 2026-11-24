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
from typing import Mapping

from cargo_feedback import run_cargo
from ci_impact import ROOT, SUITE_OWNERS, WORKSPACE, suite_commands_by_ci_job, suite_environment_for_report
from dev_rust_feedback import commands_for_report


FULL_WORKSPACE_UNIT_TEST = ["cargo", "test", "--workspace", "--locked"]


def cargo_commands_for_ci(
    report: dict, *, delegate_workspace_unit_tests_to_verify: bool,
) -> tuple[list[list[str]], bool]:
    """Run all scoped tests here; delegate only full workspace tests to the gated verify job.

    The impact planner already owns the suite and Cargo package selection.
    This runner may move the single full-workspace test command to the
    existing parallel CI owner, but may not drop any narrower Cargo command.
    """
    commands = commands_for_report(report, check_only=False)
    if not delegate_workspace_unit_tests_to_verify or report.get("analysis") != "full":
        return commands, False

    owners = report.get("selected_ci_jobs")
    if (
        commands != [FULL_WORKSPACE_UNIT_TEST]
        or not isinstance(owners, list)
        or not all(isinstance(owner, str) for owner in owners)
        or not {"verify", "architecture-and-rust"}.issubset(owners)
    ):
        raise ValueError("full workspace unit test delegation requires both selected CI owners")
    return [], True


def require_clean_truth_filter_environment(
    name: str, *, environment: Mapping[str, str] = os.environ,
) -> None:
    if name == "truth_certification" and (
        "TRUTH_CERT_FIXTURE_IDS" in environment or "TRUTH_CERT_LIMIT" in environment
    ):
        raise ValueError("externally supplied Truth fixture filters cannot shrink exact-source CI")


def selected_preparations(
    integration_commands: list[tuple[str, list[str]]],
) -> list[tuple[str, tuple[str, ...], bool]]:
    """Order selected binary builds before E2E changes the Cargo feature graph.

    The impact planner owns the suite selection and its exact commands. Build
    each selected runtime binary command once, even if several suites share it;
    do not build workers at all when no owner selected their preparation.
    """
    prepared: set[tuple[str, ...]] = set()
    schedule: list[tuple[str, tuple[str, ...], bool]] = []
    for name, _ in integration_commands:
        require_clean_truth_filter_environment(name)
        preparation = SUITE_OWNERS[name].get("prepare")
        if preparation:
            reused = preparation in prepared
            schedule.append((name, preparation, reused))
            prepared.add(preparation)
    return schedule


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument(
        "--delegate-workspace-unit-tests-to-verify", action="store_true",
        help="For full impact only, require the parallel verify job to run cargo test --workspace",
    )
    parser.add_argument(
        "--suite-job", choices=("architecture-and-rust", "verify"),
        default="architecture-and-rust",
        help="Existing CI owner executing this exact-source suite partition",
    )
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    actual_sha = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if report.get("source_sha") != actual_sha:
        raise ValueError("Rust impact report does not match the checked-out source SHA")
    if args.suite_job == "verify":
        if args.delegate_workspace_unit_tests_to_verify or report.get("analysis") != "full":
            raise ValueError("verify executes delegated E2E suites only for full impact")
        # The same verify job has already executed its mandatory workspace
        # unit tests. Never re-run those Cargo commands through this runner.
        commands, delegated = [], False
    else:
        commands, delegated = cargo_commands_for_ci(
            report,
            delegate_workspace_unit_tests_to_verify=args.delegate_workspace_unit_tests_to_verify,
        )
    integration_commands = suite_commands_by_ci_job(report)[args.suite_job]
    preparations = selected_preparations(integration_commands)

    measurements = []
    if delegated:
        print(f"CI_WORKSPACE_UNIT_TESTS=DELEGATED_TO_VERIFY source_sha={actual_sha}", flush=True)
    for command in commands:
        print("CI_RUST_COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))

    # In the measured full CI, compiling --features e2e before a late Worker
    # build invalidated the shared default-feature Cargo graph and rebuilt the
    # same crates. Keep the exact prebuilt binaries in the ordinary target,
    # immediately after default-feature tests and before any E2E compilation.
    for name, preparation, reused in preparations:
        if reused:
            print(f"CI_SUITE_PREPARE_REUSED={name}", flush=True)
        else:
            print(f"CI_SUITE_PREPARE={name} COMMAND=" + " ".join(preparation), flush=True)
            measurements.append(run_cargo(list(preparation), cwd=WORKSPACE))

    # All real integration commands and owners come from ci_impact.py; the
    # runner has no suite-specific compilation or filter-selection branches.
    for name, command in integration_commands:
        owner = SUITE_OWNERS[name]
        # The prebuilt-binary hook is generic. A runtime suite can use its
        # existing override variables without recompiling the same Rust graph
        # under a second Cargo target directory inside the test process.
        target = Path(os.environ.get("CARGO_TARGET_DIR", WORKSPACE / "target"))
        if not target.is_absolute():
            target = WORKSPACE / target
        runtime_env = suite_environment_for_report(report, name)
        if name == "truth_certification":
            print("CI_SUITE_FIXTURE_IDS=" + runtime_env.get("TRUTH_CERT_FIXTURE_IDS", "ALL"), flush=True)
        for variable, binary in owner.get("runtime_binaries", {}).items():
            path = target / "debug" / (binary + (".exe" if os.name == "nt" else ""))
            if not path.is_file() or not os.access(path, os.X_OK):
                raise FileNotFoundError(f"selected suite {name} lacks prebuilt runtime binary: {path}")
            runtime_env[variable] = str(path)
            print(f"CI_SUITE_PREBUILT={name} {variable}={path}", flush=True)
        print(f"CI_SUITE={name} COMMAND=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE, env=runtime_env or None))

    output = ROOT / (
        "automation/reports/scoped_cargo_feedback.json"
        if args.suite_job == "architecture-and-rust"
        else "automation/reports/scoped_cargo_feedback_verify.json"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {"source_sha": actual_sha, "impact": report["analysis"],
             "suite_job": args.suite_job,
             "selected_suites": [name for name, _ in integration_commands],
             "delegated_workspace_unit_tests_to": "verify" if delegated else None,
             "measurements": measurements}, indent=2
        ) + "\n",
        encoding="utf-8",
    )
    print(f"CI_RUST_FEEDBACK=PASS commands={len(measurements)}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
