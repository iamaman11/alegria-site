#!/usr/bin/env python3
"""Execute the exact-source Cargo plan from the shared local/CI impact model.

This is a runner, not a second impact planner. It consumes the single
`ci_impact.py` report and calls `dev_rust_feedback.commands_for_report`.
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import json
import os
import re
import subprocess
import threading
import time
from pathlib import Path
from typing import Mapping

from cargo_feedback import run_cargo
from ci_impact import ROOT, SUITE_OWNERS, WORKSPACE, suite_commands_by_ci_job, suite_environment_for_report
from dev_rust_feedback import commands_for_report


FULL_WORKSPACE_UNIT_TEST = ["cargo", "test", "--workspace", "--locked"]
VERIFY_E2E_COMMAND_PREFIX = (
    "cargo", "test", "-p", "integration_harness", "--features", "e2e", "--lib",
)
VERIFY_E2E_COMMAND_SUFFIX = ("--locked", "--", "--nocapture")
E2E_COMPILE = [
    *VERIFY_E2E_COMMAND_PREFIX, "--locked", "--no-run",
]
E2E_MAX_PROCESSES = 2
TEST_RESULT_COUNT = re.compile(r"test result: ok\. (\d+) passed;")


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


def suite_runtime_environment(report: dict, name: str) -> dict[str, str]:
    """Preserve all exact-source suite overrides for Cargo and precompiled runs."""
    owner = SUITE_OWNERS[name]
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
    return runtime_env


def exact_verify_e2e_filter(command: list[str]) -> str:
    """Only replace the transport of a known immutable Cargo test invocation.

    The suite owner still chooses and validates its filter. Any future Cargo
    command shape change must be reviewed before enabling direct execution.
    """
    if (len(command) != 11
            or tuple(command[:7]) != VERIFY_E2E_COMMAND_PREFIX
            or tuple(command[8:]) != VERIFY_E2E_COMMAND_SUFFIX
            or not command[7] or command[7].startswith("-")):
        raise ValueError(f"parallel E2E requires an exact, locked integration harness filter: {command}")
    return command[7]


def precompiled_verify_executable(measurement: dict) -> Path:
    """Bind execution to the actual Cargo-produced test artifact, not a glob."""
    paths = [row["path"] for row in measurement.get("test_executables", [])
             if row.get("target") == "integration_harness"]
    if len(paths) != 1:
        raise RuntimeError(f"expected one compiled integration_harness lib test executable, found {len(paths)}")
    target = Path(os.environ.get("CARGO_TARGET_DIR", WORKSPACE / "target"))
    if not target.is_absolute():
        target = WORKSPACE / target
    executable = Path(paths[0])
    if not executable.is_absolute():
        executable = WORKSPACE / executable
    executable = executable.resolve()
    # Cargo can emit the test binary either in debug/deps or in the package's
    # hashed build output (observed in the real CI E2E logs). Never accept a
    # binary under an unrelated crate, target directory or symlink escape.
    debug = target.resolve() / "debug"
    normal = (
        executable.is_relative_to(debug / "deps")
        and executable.name.startswith("integration_harness-")
    )
    build_root = debug / "build" / "integration_harness"
    special = False
    if executable.is_relative_to(build_root):
        parts = executable.relative_to(build_root).parts
        special = (
            len(parts) == 3
            and re.fullmatch(r"[0-9a-f]{16}", parts[0]) is not None
            and parts[1] == "out"
            and parts[2].startswith("integration_harness-")
        )
    if (not (normal or special) or not executable.is_file()
            or not os.access(executable, os.X_OK)):
        raise RuntimeError(f"Cargo returned an unusable integration_harness test binary: {executable}")
    return executable


def run_precompiled_e2e_suite(
    name: str, command: list[str], executable: Path,
    runtime_env: Mapping[str, str], output_lock: threading.Lock,
) -> dict:
    """Execute one already-selected test filter in its own process/env scope."""
    test_filter = exact_verify_e2e_filter(command)
    executed = [str(executable), test_filter, "--nocapture"]
    environment = {**os.environ, **runtime_env}
    # Cargo usually appends target/debug/deps to the dynamic-loader path.
    # Reproduce that execution contract for the already compiled test binary.
    dependency_dir = str(executable.parent)
    previous = environment.get("LD_LIBRARY_PATH")
    environment["LD_LIBRARY_PATH"] = dependency_dir + (os.pathsep + previous if previous else "")
    start = time.monotonic()
    matched_count = 0
    process = subprocess.Popen(
        executed, cwd=WORKSPACE, env=environment, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, errors="replace", bufsize=1,
    )
    assert process.stdout is not None
    for line in process.stdout:
        match = TEST_RESULT_COUNT.search(line)
        if match:
            matched_count = int(match.group(1))
        with output_lock:
            print(f"[{name}] {line}", end="", flush=True)
    exit_code = process.wait()
    measurement = {
        "command": executed,
        "selected_cargo_command": command,
        "runner": "precompiled-e2e-test-process",
        "suite": name,
        "exit_code": exit_code,
        "matched_tests": matched_count,
        "elapsed_seconds": round(time.monotonic() - start, 3),
        "compiler_seconds": None,
        "post_compiler_seconds": None,
        "rebuilt_workspace_targets": [],
        "fresh_workspace_targets": [],
        "artifact_evidence": "unavailable",
    }
    with output_lock:
        print("CARGO_FEEDBACK_JSON=" + json.dumps(measurement, sort_keys=True), flush=True)
    if exit_code or matched_count == 0:
        raise RuntimeError(
            f"E2E suite {name} failed (exit={exit_code}, matched_tests={matched_count})"
        )
    return measurement


def run_parallel_precompiled_e2e(
    report: dict, integration_commands: list[tuple[str, list[str]]],
    *, suite_job: str,
) -> list[dict]:
    """Compile one exact-source harness per CI owner and isolate each E2E filter.

    This changes *only* scheduling, not ownership or suite selection. In
    particular, Temporal/graph use their actual independent Testcontainers,
    own temporary PostgreSQL/Neo4j/Qdrant state, and have distinct process
    environments. The two processes are bounded to avoid saturating the CI
    runner with multiple heavyweight databases. All selected suites must pass.
    """
    if len(integration_commands) < 2:
        raise ValueError("parallel precompiled E2E needs at least two suites")
    if suite_job not in ("verify", "architecture-and-rust"):
        raise ValueError("unknown E2E CI owner")
    filters = [exact_verify_e2e_filter(command) for _, command in integration_commands]
    if len(filters) != len(set(filters)):
        raise ValueError("parallel E2E cannot execute duplicated filters")
    if integration_commands != suite_commands_by_ci_job(report)[suite_job]:
        raise ValueError("parallel E2E must execute the exact suite owner partition")

    print("CI_E2E_PRECOMPILE COMMAND=" + " ".join(E2E_COMPILE), flush=True)
    compilation = run_cargo(E2E_COMPILE, cwd=WORKSPACE)
    executable = precompiled_verify_executable(compilation)
    environments: list[dict[str, str]] = []
    for name, command in integration_commands:
        environments.append(suite_runtime_environment(report, name))
        print(f"CI_SUITE={name} COMMAND=" + " ".join(command), flush=True)

    output_lock = threading.Lock()
    results: dict[str, dict] = {}
    failures: list[Exception] = []
    print(f"CI_E2E_PARALLEL_PROCESSES={E2E_MAX_PROCESSES} owner={suite_job} suites={len(filters)}", flush=True)
    # On the architecture owner, schedule the observed longest integration
    # first (canonical Temporal); it overlaps the independent graph contract
    # suite without changing the deterministic result/report order. The
    # verify job keeps its existing exact-source suite order.
    queue = list(zip(integration_commands, environments))
    if suite_job == "architecture-and-rust":
        queue.sort(key=lambda item: item[0][0] != "temporal_restart")
    with ThreadPoolExecutor(max_workers=E2E_MAX_PROCESSES) as pool:
        pending = {
            pool.submit(run_precompiled_e2e_suite, name, command, executable, environment, output_lock): name
            for (name, command), environment in queue
        }
        for future in as_completed(pending):
            name = pending[future]
            try:
                results[name] = future.result()
            except Exception as error:
                failures.append(error)
                print(f"CI_E2E_SUITE_FAILURE={name} reason={error}", flush=True)
    if failures:
        raise RuntimeError(f"{len(failures)} mandatory E2E suites failed") from failures[0]
    return [compilation, *(results[name] for name, _ in integration_commands)]


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
    parser.add_argument(
        "--parallel-verify-e2e", action="store_true",
        help="Compile once and run mandatory verify E2E filters in two isolated processes",
    )
    parser.add_argument(
        "--parallel-architecture-e2e", action="store_true",
        help="For full impact, precompile architecture E2E once and run independent filters in two OS processes",
    )
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    actual_sha = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if report.get("source_sha") != actual_sha:
        raise ValueError("Rust impact report does not match the checked-out source SHA")
    if args.suite_job == "verify":
        if (args.delegate_workspace_unit_tests_to_verify
                or args.parallel_architecture_e2e or report.get("analysis") != "full"):
            raise ValueError("verify executes delegated E2E suites only for full impact")
        # The same verify job has already executed its mandatory workspace
        # unit tests. Never re-run those Cargo commands through this runner.
        commands, delegated = [], False
        if args.parallel_verify_e2e and report.get("analysis") != "full":
            raise ValueError("parallel verify E2E requires exact-source full impact")
    else:
        if args.parallel_verify_e2e:
            raise ValueError("parallel verify E2E cannot run in architecture owner")
        if args.parallel_architecture_e2e and report.get("analysis") != "full":
            raise ValueError("parallel architecture E2E requires full exact-source impact")
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
    if (args.parallel_verify_e2e or args.parallel_architecture_e2e) and len(integration_commands) >= 2:
        measurements.extend(run_parallel_precompiled_e2e(
            report, integration_commands, suite_job=args.suite_job,
        ))
    else:
        for name, command in integration_commands:
            # The prebuilt-binary hook is generic. A runtime suite can use its
            # existing override variables without recompiling the same Rust graph
            # under a second Cargo target directory inside the test process.
            runtime_env = suite_runtime_environment(report, name)
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
