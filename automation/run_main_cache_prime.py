#!/usr/bin/env python3
"""Warm reusable Rust library objects after a fully accepted PR.

The mandatory PR CI already compiled and ran the actual unit/integration
tests. Main must not spend minutes relinking those tests and worker binaries:
sccache cannot cache targets that invoke the system linker. A trusted-main
writer therefore warms only cacheable rlib dependency objects for the exact
impact graph, including the E2E feature graph when a live suite was selected.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from cargo_feedback import run_cargo
from ci_impact import ROOT, WORKSPACE, load_cargo_metadata, suite_commands_for_report


def prime_commands_for_report(report: dict, metadata: dict) -> list[list[str]]:
    """Compile affected library objects, not non-cacheable test/bin targets.

    Cargo's own metadata is authoritative for target kinds: do not infer a
    package is a library from its path/name, or silently skip an unknown crate.
    """
    selected_suites = suite_commands_for_report(report)
    affected = report.get("affected_crates")
    if not isinstance(affected, list) or len(set(affected)) != len(affected):
        raise ValueError("invalid affected crate selection for cache priming")
    members = set(metadata["workspace_members"])
    packages = [pkg for pkg in metadata["packages"] if pkg["id"] in members]
    if len(packages) != len(members):
        raise ValueError("incomplete Cargo metadata for cache priming")
    by_name = {pkg["name"]: pkg for pkg in packages}
    if len(by_name) != len(packages) or not set(affected).issubset(by_name):
        raise ValueError("unknown or ambiguous Cargo package in accepted-main impact")

    libraries = sorted(
        name for name in affected
        if any("lib" in target["kind"] or "rlib" in target["kind"]
               for target in by_name[name]["targets"])
    )
    commands: list[list[str]] = []
    if libraries:
        command = ["cargo", "build", "--locked", "--lib"]
        for name in libraries:
            command.extend(["--package", name])
        commands.append(command)

    # A live PR suite compiled a second E2E feature graph. Prime only its
    # cacheable libraries; the non-cacheable harness executable and worker
    # processes were already built and exercised by mandatory PR CI.
    if selected_suites:
        if "integration_harness" not in by_name or not any(
            "lib" in target["kind"] or "rlib" in target["kind"]
            for target in by_name["integration_harness"]["targets"]
        ):
            raise ValueError("E2E suite selected without integration_harness library")
        commands.append([
            "cargo", "build", "--locked", "--package", "integration_harness",
            "--features", "e2e", "--lib",
        ])
    return commands


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

    commands = prime_commands_for_report(report, load_cargo_metadata())
    if not commands:
        print("MAIN_CACHE_PRIME=SKIPPED no reusable library objects changed", flush=True)
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
    # Compiler-object telemetry only; the PR gate owns all test acceptance.
    print("MAIN_CACHE_PRIME=COMPILED measurements=" + str(len(measurements)), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
