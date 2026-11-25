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
from ci_impact import (
    ROOT, SUITE_OWNERS, WORKSPACE, e2e_shard_commands_for_report,
    suite_commands_by_ci_job, suite_environment_for_report,
)
from dev_rust_feedback import commands_for_report
from rust_ci_artifacts import package as package_ci_artifact


FULL_WORKSPACE_UNIT_TEST = ["cargo", "test", "--workspace", "--locked"]
VERIFY_E2E_COMMAND_PREFIX = (
    "cargo", "test", "-p", "integration_harness", "--features", "e2e", "--lib",
)
PG_E2E_PACKAGE = "integration_pg_v3_e2e"
PG_E2E_COMMAND_PREFIX = (
    "cargo", "test", "-p", PG_E2E_PACKAGE, "--features", "e2e", "--lib",
)
E2E_COMMAND_PREFIXES = (VERIFY_E2E_COMMAND_PREFIX, PG_E2E_COMMAND_PREFIX)
VERIFY_E2E_COMMAND_SUFFIX = ("--locked", "--", "--nocapture")
E2E_COMPILE = [
    *VERIFY_E2E_COMMAND_PREFIX, "--locked", "--no-run",
]
PG_E2E_COMPILE = [*PG_E2E_COMMAND_PREFIX, "--locked", "--no-run"]
EXACT_EXECUTABLE_NAMES = {
    "integration_harness": "integration_harness-ci-exact",
    PG_E2E_PACKAGE: "integration_pg_v3_e2e-ci-exact",
}
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
            or tuple(command[:7]) not in E2E_COMMAND_PREFIXES
            or tuple(command[8:]) != VERIFY_E2E_COMMAND_SUFFIX
            or not command[7] or command[7].startswith("-")):
        raise ValueError(f"parallel E2E requires an exact, locked owner-package filter: {command}")
    return command[7]


def e2e_package_for_command(command: list[str]) -> str:
    exact_verify_e2e_filter(command)
    return command[3]


def exact_e2e_executable(package: str) -> Path:
    if package not in EXACT_EXECUTABLE_NAMES:
        raise ValueError(f"unsupported exact-source E2E owner package {package}")
    return WORKSPACE / "target/debug/deps" / EXACT_EXECUTABLE_NAMES[package]


def precompiled_verify_executable(
    measurement: dict, *, package: str = "integration_harness",
) -> Path:
    """Bind execution to the actual Cargo-produced test artifact, not a glob."""
    if package not in EXACT_EXECUTABLE_NAMES:
        raise ValueError(f"unsupported E2E package {package}")
    paths = [row["path"] for row in measurement.get("test_executables", [])
             if row.get("target") == package]
    if len(paths) != 1:
        raise RuntimeError(f"expected one compiled {package} lib test executable, found {len(paths)}")
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
        and executable.name.startswith(package + "-")
    )
    build_root = debug / "build" / package
    special = False
    if executable.is_relative_to(build_root):
        parts = executable.relative_to(build_root).parts
        special = (
            len(parts) == 3
            and re.fullmatch(r"[0-9a-f]{16}", parts[0]) is not None
            and parts[1] == "out"
            and parts[2].startswith(package + "-")
        )
    if (not (normal or special) or not executable.is_file()
            or not os.access(executable, os.X_OK)):
        raise RuntimeError(f"Cargo returned an unusable {package} test binary: {executable}")
    return executable


def run_precompiled_e2e_suite(
    name: str, command: list[str], executable: Path,
    runtime_env: Mapping[str, str], output_lock: threading.Lock,
    *, test_filter: str | None = None, expected_tests: int | None = None,
) -> dict:
    """Execute one already-selected test filter in its own process/env scope."""
    selected_filter = exact_verify_e2e_filter(command)
    if test_filter is None:
        test_filter = selected_filter
    elif (not isinstance(test_filter, str) or not test_filter.startswith(selected_filter)
          or test_filter.startswith("-") or not test_filter):
        raise ValueError(f"shard filter must narrow its selected source suite: {test_filter!r}")
    if expected_tests is not None and (type(expected_tests) is not int or expected_tests <= 0):
        raise ValueError("shard must require a positive exact test count")
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
        "required_exact_tests": expected_tests,
        "elapsed_seconds": round(time.monotonic() - start, 3),
        "compiler_seconds": None,
        "post_compiler_seconds": None,
        "rebuilt_workspace_targets": [],
        "fresh_workspace_targets": [],
        "artifact_evidence": "unavailable",
    }
    with output_lock:
        print("CARGO_FEEDBACK_JSON=" + json.dumps(measurement, sort_keys=True), flush=True)
    if exit_code or matched_count == 0 or (expected_tests is not None and matched_count != expected_tests):
        raise RuntimeError(
            f"E2E suite {name} failed (exit={exit_code}, matched_tests={matched_count}, "
            f"expected_tests={expected_tests})"
        )
    return measurement


def run_exact_precompiled_e2e_shard(
    report: dict, shard: str, *, suite_job: str, prebuilt_executable: Path,
    prebuilt_pg_executable: Path | None = None,
) -> list[dict]:
    """Execute one nonempty, registry-owned E2E shard on its *own* CI runner.

    The planner is the single source of suite selection and exact test-filter
    assignments. A shard is intentionally process-serial; separate GitHub
    runners, not contending Docker containers, provide parallelism. Empty or
    narrowed fragments never produce an artificial successful E2E result.
    """
    if report.get("analysis") != "full":
        raise ValueError("sharded precompiled E2E requires exact-source full CI")
    plan = e2e_shard_commands_for_report(report)
    if shard not in plan:
        raise ValueError(f"unrecognized immutable E2E shard {shard!r}")
    selected = plan[shard]
    if not selected:
        raise ValueError(f"scheduled E2E shard {shard!r} has no selected real test filters")
    if suite_job not in ("architecture-and-rust", "verify"):
        raise ValueError("unknown sharded E2E owner")
    owner_commands = dict(suite_commands_by_ci_job(report)[suite_job])
    filters: set[str] = set()
    for name, command, filter_, count in selected:
        if (name not in owner_commands or command != owner_commands[name]
                or command != list(SUITE_OWNERS[name]["command"])):
            raise ValueError(f"shard {shard} selected a non-owner or rewritten Cargo command: {name}")
        source_filter = exact_verify_e2e_filter(command)
        if (not isinstance(filter_, str) or not filter_.startswith(source_filter)
                or filter_ in filters or type(count) is not int or count <= 0):
            raise ValueError(f"shard {shard} has an invalid or duplicate real test filter: {filter_!r}")
        filters.add(filter_)

    executables = {
        "integration_harness": prebuilt_executable,
        PG_E2E_PACKAGE: prebuilt_pg_executable,
    }
    for name, command, _, _ in selected:
        package = e2e_package_for_command(command)
        executable = executables[package]
        expected = exact_e2e_executable(package).resolve()
        if (executable is None or executable.resolve() != expected
                or not expected.is_file() or not os.access(expected, os.X_OK)):
            raise ValueError(
                f"E2E shard {shard} must execute only installed verified {package} test binary"
            )
    print(f"CI_E2E_SHARD_SOURCE=PASS shard={shard} owner={suite_job} filters={len(selected)}", flush=True)
    results: list[dict] = []
    output_lock = threading.Lock()
    for name, command, filter_, count in selected:
        require_clean_truth_filter_environment(name)
        runtime_env = suite_runtime_environment(report, name)
        print(f"CI_E2E_SHARD_FILTER={shard} suite={name} filter={filter_} expected_tests={count}", flush=True)
        results.append(run_precompiled_e2e_suite(
            name, command, exact_e2e_executable(e2e_package_for_command(command)).resolve(),
            runtime_env, output_lock,
            test_filter=filter_, expected_tests=count,
        ))
    actual = sum(row["matched_tests"] for row in results)
    required = sum(count for _, _, _, count in selected)
    if actual != required:
        raise RuntimeError(f"E2E shard {shard} mismatched required coverage {actual}!={required}")
    print(f"CI_E2E_SHARD=PASS shard={shard} filters={len(selected)} tests={actual}", flush=True)
    return results


def run_precompiled_e2e_suites(
    report: dict, integration_commands: list[tuple[str, list[str]]],
    *, suite_job: str, prebuilt_executable: Path | None = None,
) -> list[dict]:
    """Compile one harness per CI owner, then run every required E2E filter.

    Verify's independent PostgreSQL suites retain their proven two-process
    scheduling. The architecture owner's Temporal/Neo4j/Qdrant suites run
    strictly sequentially: GitHub CI #492 measured resource contention when
    they competed (Temporal infra startup ~58 s vs ~19-22 s serially).
    Process isolation did not provide runtime resource independence.
    """
    if len(integration_commands) < 2:
        raise ValueError("precompiled E2E needs at least two suites")
    if suite_job not in ("verify", "architecture-and-rust"):
        raise ValueError("unknown E2E CI owner")
    filters = [exact_verify_e2e_filter(command) for _, command in integration_commands]
    if any(e2e_package_for_command(command) != "integration_harness"
           for _, command in integration_commands):
        raise ValueError("mixed E2E owner binaries require the exact-source sharded executor")
    if len(filters) != len(set(filters)):
        raise ValueError("parallel E2E cannot execute duplicated filters")
    if integration_commands != suite_commands_by_ci_job(report)[suite_job]:
        raise ValueError("precompiled E2E must execute the exact suite owner partition")

    measurements: list[dict] = []
    if prebuilt_executable is None:
        print("CI_E2E_PRECOMPILE COMMAND=" + " ".join(E2E_COMPILE), flush=True)
        compilation = run_cargo(E2E_COMPILE, cwd=WORKSPACE)
        executable = precompiled_verify_executable(compilation)
        measurements.append(compilation)
    else:
        executable = prebuilt_executable.resolve()
        expected = (WORKSPACE / "target/debug/deps/integration_harness-ci-exact").resolve()
        if executable != expected or not executable.is_file() or not os.access(executable, os.X_OK):
            raise ValueError("prebuilt harness requires verified exact-source installed artifact")
        print(f"CI_E2E_PREBUILT=PASS executable={executable}", flush=True)
    environments: list[dict[str, str]] = []
    for name, command in integration_commands:
        environments.append(suite_runtime_environment(report, name))
        print(f"CI_SUITE={name} COMMAND=" + " ".join(command), flush=True)

    output_lock = threading.Lock()
    results: dict[str, dict] = {}
    failures: list[Exception] = []
    workers = E2E_MAX_PROCESSES if suite_job == "verify" else 1
    print(f"CI_E2E_PROCESSES={workers} owner={suite_job} suites={len(filters)}", flush=True)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        pending = {
            pool.submit(run_precompiled_e2e_suite, name, command, executable, environment, output_lock): name
            for (name, command), environment in zip(integration_commands, environments)
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
    return [*measurements, *(results[name] for name, _ in integration_commands)]


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
    parser.add_argument("--compile-harness-only", action="store_true",
                        help="For full impact, compile exact E2E harness once and publish content-verified bundle")
    parser.add_argument("--compile-pg-harness-only", action="store_true",
                        help="Compile the independent PostgreSQL V3 E2E owner test target once")
    parser.add_argument("--harness-bundle-dir", type=Path,
                        help="Fresh output directory for the single full-impact E2E compiler artifact")
    parser.add_argument("--pg-harness-bundle-dir", type=Path,
                        help="Fresh output directory for the independent PG V3 compiler artifact")
    parser.add_argument("--execute-precompiled-e2e", action="store_true",
                        help="For full impact, run exact owner suites from previously verified binary artifacts")
    parser.add_argument("--e2e-shard",
                        help="Exact nonempty shard name selected by CI impact; full precompiled E2E only")
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    actual_sha = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if report.get("source_sha") != actual_sha:
        raise ValueError("Rust impact report does not match the checked-out source SHA")
    compiling = args.compile_harness_only or args.compile_pg_harness_only
    if compiling or args.execute_precompiled_e2e:
        if (report.get("analysis") != "full"
                or compiling == args.execute_precompiled_e2e
                or (args.compile_harness_only and args.compile_pg_harness_only)
                or (compiling and args.e2e_shard is not None)
                or args.delegate_workspace_unit_tests_to_verify
                or (args.compile_harness_only != (args.harness_bundle_dir is not None))
                or (args.compile_pg_harness_only != (args.pg_harness_bundle_dir is not None))
                or (args.parallel_verify_e2e and args.suite_job != "verify")
                or not {"architecture-and-rust", "verify"}.issubset(report.get("selected_ci_jobs", []))):
            raise ValueError("full exact-source compiler/E2E stage must be exclusive and correctly gated")
        if args.compile_harness_only:
            print("CI_E2E_PRECOMPILE COMMAND=" + " ".join(E2E_COMPILE), flush=True)
            compilation = run_cargo(E2E_COMPILE, cwd=WORKSPACE)
            executable = precompiled_verify_executable(compilation)
            package_ci_artifact("harness", actual_sha, args.harness_bundle_dir, harness=executable)
            print("CI_E2E_HARNESS_BUILD=PASS", flush=True)
            return 0
        if args.compile_pg_harness_only:
            print("CI_PG_E2E_PRECOMPILE COMMAND=" + " ".join(PG_E2E_COMPILE), flush=True)
            compilation = run_cargo(PG_E2E_COMPILE, cwd=WORKSPACE)
            executable = precompiled_verify_executable(compilation, package=PG_E2E_PACKAGE)
            package_ci_artifact("pg_harness", actual_sha, args.pg_harness_bundle_dir, harness=executable)
            print("CI_PG_E2E_HARNESS_BUILD=PASS", flush=True)
            return 0
        if args.e2e_shard is not None:
            if args.compile_harness_only or args.parallel_verify_e2e:
                raise ValueError("a precompiled shard cannot compile or request old worker-level parallelism")
            results = run_exact_precompiled_e2e_shard(
                report, args.e2e_shard, suite_job=args.suite_job,
                prebuilt_executable=exact_e2e_executable("integration_harness"),
                prebuilt_pg_executable=exact_e2e_executable(PG_E2E_PACKAGE),
            )
            print(f"CI_E2E_PREBUILT_OWNER=PASS owner={args.suite_job} shard={args.e2e_shard} "
                  f"tests={sum(row['matched_tests'] for row in results)}", flush=True)
            return 0
        if args.suite_job == "verify" and not args.parallel_verify_e2e:
            raise ValueError("full verify suites must retain independent two-process E2E scheduling")
        suites = suite_commands_by_ci_job(report)[args.suite_job]
        results = run_precompiled_e2e_suites(
            report, suites, suite_job=args.suite_job,
            prebuilt_executable=WORKSPACE / "target/debug/deps/integration_harness-ci-exact",
        )
        print(f"CI_E2E_PREBUILT_OWNER=PASS owner={args.suite_job} suites={len(suites)} tests={sum(row['matched_tests'] for row in results)}", flush=True)
        return 0
    if (args.harness_bundle_dir is not None or args.pg_harness_bundle_dir is not None
            or args.e2e_shard is not None):
        raise ValueError("harness bundle is valid only for the dedicated full compiler stage")
    if args.suite_job == "verify":
        if args.delegate_workspace_unit_tests_to_verify or report.get("analysis") != "full":
            raise ValueError("verify executes delegated E2E suites only for full impact")
        # The same verify job has already executed its mandatory workspace
        # unit tests. Never re-run those Cargo commands through this runner.
        commands, delegated = [], False
        if args.parallel_verify_e2e and report.get("analysis") != "full":
            raise ValueError("parallel verify E2E requires exact-source full impact")
    else:
        if args.parallel_verify_e2e:
            raise ValueError("parallel verify E2E cannot run in architecture owner")
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
    if (args.parallel_verify_e2e or (
        args.suite_job == "architecture-and-rust" and report.get("analysis") == "full"
    )) and len(integration_commands) >= 2:
        measurements.extend(run_precompiled_e2e_suites(
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
