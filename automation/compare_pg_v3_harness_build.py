#!/usr/bin/env python3
"""Opt-in, *physical* Cargo A/B: legacy E2E lib vs isolated PG V3 E2E lib.

This is deliberately not a manifest-size estimate, a CI timing prediction, or
an assertion about CPU/RAM. Both real test executables must be produced from
one immutable Git archive, with separate initially empty Cargo target dirs,
incremental compilation off, and rustc wrappers (including sccache) disabled.

The full measurement is expensive. Run explicitly on the PR's actual runner:

    python3 automation/compare_pg_v3_harness_build.py \
        --output automation/reports/pg_v3_harness_physical_ab.json

Optionally add --measure-resources when GNU time is available. This reports
process-tree user/system CPU *seconds* and the largest individual process RSS;
it cannot measure aggregate simultaneous RAM or the whole Issue #94 CI DAG.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any

import compare_physical_build_options as shared


ROOT = Path(__file__).resolve().parents[1]
PACKAGES = (("A_legacy_monolith", "integration_harness"),
            ("B_pg_owner", "integration_pg_v3_e2e"))
FORBIDDEN_PG_PACKAGES = frozenset({
    "integration_harness", "runtime_composition", "infrastructure",
    "graph_neo4j", "vector_qdrant", "outbox_worker", "temporal_worker",
    "analytics_svc", "reconcile", "neo4rs", "qdrant-client",
})
WRAPPER_ENV = shared.RUSTC_WRAPPER_ENV
DISABLE_WRAPPER_CONFIG = shared.CARGO_DISABLE_WRAPPER_CONFIG
SCHEMA_VERSION = 1


class EvidenceError(RuntimeError):
    """Insufficient provenance or Rust compiler evidence; never report PASS."""


def command_for(package: str) -> list[str]:
    if package not in {candidate for _, candidate in PACKAGES}:
        raise EvidenceError(f"unexpected E2E package {package!r}")
    return [
        "cargo", *DISABLE_WRAPPER_CONFIG, "test", "--package", package,
        "--features", "e2e", "--lib", "--no-run", "--locked",
        "--timings", "--message-format=json-render-diagnostics",
    ]


def wrapper_free_environment(target_dir: Path) -> dict[str, str]:
    rustc = shutil.which("rustc")
    if not rustc:
        raise EvidenceError("rustc unavailable; no physical compiler evidence")
    if target_dir.exists() and any(target_dir.iterdir()):
        raise EvidenceError(f"Cargo A/B target is not initially empty: {target_dir}")
    environment = os.environ.copy()
    for key in tuple(environment):
        if key in WRAPPER_ENV or key.startswith("SCCACHE_"):
            environment.pop(key)
    environment.update({
        "RUSTC": rustc,
        "CARGO_BUILD_RUSTC": rustc,
        "CARGO_INCREMENTAL": "0",
        "CARGO_TARGET_DIR": str(target_dir),
    })
    return environment


def checked_toolchain(workspace: Path) -> dict[str, str]:
    results: dict[str, str] = {}
    for executable, args in (("rustc", ["-vV"]), ("cargo", ["--version"])):
        if not shutil.which(executable):
            raise EvidenceError(f"{executable} missing; no Rust compiler evidence")
        result = subprocess.run([executable, *args], cwd=workspace, text=True,
                                capture_output=True, check=False)
        if result.returncode or not result.stdout.strip():
            raise EvidenceError(f"{executable} version query failed: {result.stderr[-2000:]}")
        results[executable] = result.stdout.strip()
    host = re.findall(r"^host: ([A-Za-z0-9_\-]+)$", results["rustc"], re.MULTILINE)
    if len(host) != 1:
        raise EvidenceError("rustc -vV did not identify a unique host target")
    results["host_target"] = host[0]
    return results


def resolved_package_closures(workspace: Path, host: str) -> tuple[dict[str, Any], dict[str, str]]:
    """Locked Cargo metadata is an upper-bound graph, *not* rustc unit evidence.

    Workspace-wide feature unification can overapproximate each target. The
    actual compiler-artifact stream below is authoritative for compiled units.
    """
    cmd = [
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--filter-platform", host,
        "--features", ",".join(f"{name}/e2e" for _, name in PACKAGES),
    ]
    process = subprocess.run(cmd, cwd=workspace, text=True,
                             capture_output=True, check=False)
    if process.returncode:
        raise EvidenceError(f"locked Cargo metadata failed: {process.stderr[-3000:]}")
    try:
        metadata = json.loads(process.stdout)
    except json.JSONDecodeError as exc:
        raise EvidenceError("Cargo metadata did not return valid JSON") from exc
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        raise EvidenceError("Cargo metadata omitted packages")
    packages = metadata["packages"]
    for item in packages:
        if not isinstance(item, dict):
            raise EvidenceError("Cargo metadata package is not a JSON object")
        # Local path dependencies outside the Git archive would make the
        # physical build depend on bytes not captured by source provenance.
        if item.get("source") is None:
            manifest_path = item.get("manifest_path")
            if (not isinstance(manifest_path, str)
                    or not Path(manifest_path).resolve().is_relative_to(workspace.resolve())):
                raise EvidenceError(f"Cargo resolved local source outside exact Git archive: {item.get('name')}")
    id_to_name = {
        item["id"]: item["name"] for item in packages
        if isinstance(item, dict) and isinstance(item.get("id"), str)
        and isinstance(item.get("name"), str)
    }
    if len(id_to_name) != len(packages):
        raise EvidenceError("Cargo metadata has incomplete or duplicate package IDs")
    workspace_members = metadata.get("workspace_members")
    if not isinstance(workspace_members, list):
        raise EvidenceError("Cargo metadata omitted workspace members")
    by_name = {id_to_name[item]: item for item in workspace_members if item in id_to_name}
    if len(by_name) != len(workspace_members):
        raise EvidenceError("Cargo workspace members have ambiguous names")
    if missing := {package for _, package in PACKAGES} - by_name.keys():
        raise EvidenceError(f"Cargo metadata missing workspace targets {sorted(missing)}")
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("nodes"), list):
        raise EvidenceError("Cargo metadata omitted the resolved dependency graph")
    edges: dict[str, set[str]] = {}
    for node in resolve["nodes"]:
        if not isinstance(node, dict) or node.get("id") not in id_to_name:
            raise EvidenceError("Cargo metadata contains an unknown resolved node")
        dependencies: set[str] = set()
        if not isinstance(node.get("deps"), list):
            raise EvidenceError("Cargo metadata omitted dependency kinds")
        for dependency in node["deps"]:
            if not isinstance(dependency, dict) or dependency.get("pkg") not in id_to_name:
                raise EvidenceError("Cargo metadata has an unknown dependency package")
            kinds = dependency.get("dep_kinds")
            if not isinstance(kinds, list) or not kinds or any(
                not isinstance(kind, dict) or kind.get("kind") not in (None, "build", "dev")
                for kind in kinds
            ):
                raise EvidenceError("Cargo metadata has ambiguous dependency kinds")
            # For `cargo test --lib` normal, build AND dev dependencies matter.
            dependencies.add(dependency["pkg"])
        edges[node["id"]] = dependencies
    closures: dict[str, Any] = {}
    for _, package in PACKAGES:
        visited: set[str] = set()
        pending = [by_name[package]]
        while pending:
            identifier = pending.pop()
            if identifier in visited:
                continue
            if identifier not in edges:
                raise EvidenceError(f"Cargo resolve node missing for {identifier}")
            visited.add(identifier)
            pending.extend(edges[identifier] - visited)
        names = {id_to_name[identifier] for identifier in visited}
        closures[package] = {
            "resolved_dependency_package_count_upper_bound": len(visited) - 1,
            "resolved_workspace_packages_upper_bound": sorted(
                id_to_name[identifier] for identifier in visited
                if identifier in workspace_members
            ),
            "resolved_package_names_upper_bound": sorted(names),
        }
    check_pg_owner_closure(set(closures["integration_pg_v3_e2e"]["resolved_package_names_upper_bound"]))
    return closures, id_to_name


def check_pg_owner_closure(names: set[str]) -> None:
    if forbidden := sorted(names & FORBIDDEN_PG_PACKAGES):
        raise EvidenceError(f"PG owner harness resolves forbidden legacy/provider crates: {forbidden}")


def decode_cargo_artifacts(output: str, package: str, package_ids: dict[str, str]) -> dict[str, Any]:
    units: list[dict[str, Any]] = []
    finished: list[bool] = []
    for line in output.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue  # Cargo may emit non-JSON diagnostics on stdout.
        if not isinstance(event, dict):
            raise EvidenceError("Cargo JSON stream contains a non-object record")
        reason = event.get("reason")
        if reason == "build-finished":
            if not isinstance(event.get("success"), bool):
                raise EvidenceError("Cargo build-finished omitted success")
            finished.append(event["success"])
        elif reason == "compiler-artifact":
            target, profile = event.get("target"), event.get("profile")
            if (not isinstance(target, dict) or not isinstance(profile, dict)
                    or not isinstance(target.get("name"), str)
                    or not isinstance(target.get("kind"), list)
                    or not isinstance(profile.get("test"), bool)
                    or not isinstance(event.get("fresh"), bool)
                    or not isinstance(event.get("features"), list)
                    or event.get("package_id") not in package_ids):
                raise EvidenceError("Cargo compiler-artifact omitted required rustc fields")
            unit = {
                "package": package_ids[event["package_id"]],
                "package_id": event["package_id"],
                "target": target["name"],
                "target_kinds": target["kind"],
                "test_profile": profile["test"],
                "features": event["features"],
                "fresh": event["fresh"],
                "executable": event.get("executable"),
            }
            units.append(unit)
    if finished != [True] or not units:
        raise EvidenceError("Cargo JSON stream has no single successful build-finished and compiler-artifacts")
    if not any(not unit["fresh"] for unit in units):
        raise EvidenceError("no non-fresh Cargo compiler-artifacts; target was not cold")
    tests = [unit for unit in units if unit["package"] == package
             and unit["target"] == package and unit["test_profile"]
             and "e2e" in unit["features"] and unit["executable"]]
    if len(tests) != 1 or tests[0]["fresh"]:
        raise EvidenceError(f"no unique freshly compiled e2e lib test executable for {package}")
    if package == "integration_pg_v3_e2e":
        check_pg_owner_closure({unit["package"] for unit in units})
    return {
        "compiler_artifacts": units,
        "non_fresh_compiler_artifacts": [unit for unit in units if not unit["fresh"]],
        "test_executable_units": tests,
    }


def file_digest(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise EvidenceError(f"Cargo artifact does not exist: {path}")
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return {"size_bytes": path.stat().st_size, "sha256": hasher.hexdigest()}


def recorded_test_executable(unit: dict[str, Any], target_dir: Path) -> dict[str, Any]:
    path = Path(unit["executable"]).resolve()
    if not path.is_relative_to(target_dir.resolve()):
        raise EvidenceError("test executable escaped its isolated Cargo target")
    return {"target": unit["target"], "relative_path": str(path.relative_to(target_dir)),
            **file_digest(path)}


def collect_cargo_timings(target_dir: Path) -> list[dict[str, Any]]:
    timing_dir = target_dir / "cargo-timings"
    output = []
    for html in sorted(timing_dir.glob("*.html")):
        digest = file_digest(html)
        if digest["size_bytes"]:
            output.append({"file": html.name, **digest})
    if not output:
        raise EvidenceError("Cargo --timings did not produce any nonempty HTML timing artifact")
    return output


def is_gnu_time_version(output: str) -> bool:
    """Match GNU time's actual version banner, not BSD time or incidental text.

    GNU time prints `time (GNU Time) 1.9` on Linux; older fixtures may
    print `GNU time 1.9`. The product name is case-insensitive but must
    occupy the beginning of the first line and include a numeric version.
    """
    first_line = output.splitlines()[0].strip() if output else ""
    return re.fullmatch(
        r"(?:time\s*\(\s*GNU\s+Time\s*\)|GNU\s+Time)\s+\d+(?:\.\d+)*",
        first_line,
        flags=re.IGNORECASE,
    ) is not None


def measured_cargo_run(workspace: Path, target_dir: Path, package: str,
                       package_ids: dict[str, str], measure_resources: bool) -> dict[str, Any]:
    environment = wrapper_free_environment(target_dir)
    command = command_for(package)
    resource_path = target_dir.parent / f"resource-{package}.txt"
    if measure_resources:
        timing = shutil.which("time")
        if not timing:
            raise EvidenceError("GNU time not found; cannot claim CPU/RAM measurement")
        version = subprocess.run([timing, "--version"], capture_output=True, text=True)
        if version.returncode or not is_gnu_time_version(version.stdout):
            raise EvidenceError("--measure-resources requires GNU time")
        invocation = [timing, "-f", "PHYSICAL_AB_RESOURCE\t%e\t%U\t%S\t%M\t%x",
                      "-o", str(resource_path), *command]
    else:
        invocation = command
    start = time.monotonic()
    process = subprocess.Popen(invocation, cwd=workspace, env=environment,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    stdout, stderr = process.communicate()
    seconds = time.monotonic() - start
    if process.returncode:
        raise EvidenceError(f"{package}: physical Cargo compile failed ({process.returncode}): {stderr[-5000:]}")
    parsed = decode_cargo_artifacts(stdout, package, package_ids)
    executable = recorded_test_executable(parsed["test_executable_units"][0], target_dir)
    return {
        "package": package,
        "command": command,
        "cargo_wall_seconds_observed": round(seconds, 3),
        "compiler_artifacts": parsed["compiler_artifacts"],
        "non_fresh_compiler_artifacts": parsed["non_fresh_compiler_artifacts"],
        "compiler_artifact_count": len(parsed["compiler_artifacts"]),
        "non_fresh_compiler_artifact_count": len(parsed["non_fresh_compiler_artifacts"]),
        "lib_test_executable": executable,
        "cargo_timings_html": collect_cargo_timings(target_dir),
        "process_tree_resources": (
            shared.parse_gnu_time_report(resource_path.read_text(encoding="utf-8"))
            if measure_resources else None
        ),
    }


def physical_ab_report(*, measure_resources: bool = False) -> dict[str, Any]:
    for executable in ("git", "tar", "cargo", "rustc"):
        if not shutil.which(executable):
            raise EvidenceError(f"{executable} unavailable; no physical Cargo A/B")
    try:
        provenance = shared.verify_pr_source_provenance()
    except shared.EvidenceError as exc:
        raise EvidenceError(str(exc)) from exc
    with tempfile.TemporaryDirectory(prefix="alegria-pg-v3-harness-physical-ab-") as dirname:
        temporary = Path(dirname)
        source = temporary / "source"
        source.mkdir()
        try:
            exported_commit = shared.isolated_source_tree(source)
        except shared.EvidenceError as exc:
            raise EvidenceError(str(exc)) from exc
        if exported_commit != provenance["checkout_commit_sha"]:
            raise EvidenceError("Git HEAD changed between source provenance and archive")
        workspace = source / "app" / "rust"
        toolchain = checked_toolchain(workspace)
        closures, ids = resolved_package_closures(workspace, toolchain["host_target"])
        results: dict[str, Any] = {}
        for option, package in PACKAGES:
            results[option] = measured_cargo_run(
                workspace, temporary / f"target-{option}", package, ids, measure_resources,
            )
        baseline = results["A_legacy_monolith"]["cargo_wall_seconds_observed"]
        candidate = results["B_pg_owner"]["cargo_wall_seconds_observed"]
        return {
            "schema_version": SCHEMA_VERSION,
            "status": "ACTUAL_PG_V3_HARNESS_COLD_CARGO_AB_CAPTURED",
            "actual_rustc_evidence": True,
            "source_provenance": provenance,
            "source_export_commit_sha": exported_commit,
            "source_export_tree_sha": provenance["checkout_tree_sha"],
            "cargo_lock_sha256": file_digest(workspace / "Cargo.lock")["sha256"],
            "toolchain": toolchain,
            "runner_context": {
                key: os.environ.get(key) for key in (
                    "GITHUB_REPOSITORY", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT",
                    "GITHUB_JOB", "GITHUB_EVENT_NAME", "GITHUB_REF", "RUNNER_OS",
                    "RUNNER_ARCH", "RUNNER_NAME",
                )
            },
            "cargo_resolved_closure_upper_bound": closures,
            "cargo_metadata_scope": (
                "Cargo --locked, host-filtered, both /e2e features selected; "
                "workspace-wide feature unification can overapproximate independent target closures"
            ),
            "compiler_cache_regime": {
                "separate_initially_empty_CARGO_TARGET_DIR": True,
                "RUSTC_WRAPPER_disabled": True,
                "RUSTC_WORKSPACE_WRAPPER_disabled": True,
                "Cargo_config_wrapper_overridden": True,
                "SCCACHE_environment_removed": True,
                "CARGO_INCREMENTAL": "0",
                "cargo_registry_download_cache_shared": True,
                "source_tree_same_for_both_options": True,
            },
            **results,
            "observed_cargo_wall_delta_B_minus_A_seconds": round(candidate - baseline, 3),
            "observed_cargo_wall_ratio_B_to_A": round(candidate / baseline, 4) if baseline > 0 else None,
            "resource_measurement": (
                "GNU time: each whole Cargo invocation and waited child processes; CPU seconds "
                "and maximum individual process RSS KiB, not aggregate concurrent RAM"
                if measure_resources else "NOT_MEASURED_NO_CPU_OR_RAM_CLAIM"
            ),
            "measurement_scope": (
                "Two --lib --no-run e2e Rust test executables only, same archived PR source "
                "and runner; not E2E test runtime, not other CI jobs, not overall <=180s SLA"
            ),
            "limitations": [
                "Cold independent target dirs compare actual Cargo work; registry/source cache and runner load are shared",
                "Cargo --timings HTML and JSON compiler artifacts identify compilation units, not CI end-to-end duration",
                "Actual rustc compilation alone does not prove tests pass, behavior parity, or owner-effect isolation",
                "Cross-run comparison cannot isolate unrelated runner load without repetitions and environment control",
            ],
        }


def atomic_write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                     prefix=f".{path.name}.", delete=False) as outfile:
        temporary = Path(outfile.name)
        json.dump(report, outfile, ensure_ascii=False, indent=2, sort_keys=True, allow_nan=False)
        outfile.write("\n")
    os.replace(temporary, path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="JSON report path (on failure writes EVIDENCE_BLOCKED)")
    parser.add_argument("--measure-resources", action="store_true",
                        help="require genuine GNU time CPU/RSS resource accounting")
    args = parser.parse_args(argv)
    blocked = {"schema_version": SCHEMA_VERSION, "status": "EVIDENCE_BLOCKED",
               "actual_rustc_evidence": False,
               "reason": "No successful isolated Cargo A/B was recorded"}
    if args.output:
        atomic_write_report(args.output, blocked)
    try:
        report = physical_ab_report(measure_resources=args.measure_resources)
    except (EvidenceError, shared.EvidenceError, OSError, subprocess.SubprocessError,
            UnicodeError, ValueError) as exc:
        blocked["reason"] = str(exc)
        if args.output:
            atomic_write_report(args.output, blocked)
        print(json.dumps(blocked, ensure_ascii=False, indent=2), file=sys.stderr)
        return 2
    if args.output:
        atomic_write_report(args.output, report)
        print(f"Physical Cargo A/B recorded: {args.output}")
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True, allow_nan=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
