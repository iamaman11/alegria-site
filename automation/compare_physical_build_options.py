#!/usr/bin/env python3
"""Fail-closed A/B evidence for a provider-free Temporal orchestrator.

The manifest comparison is an *upper bound on local workspace dependencies*,
not a rustc timing result. An optional compiler probe requires a real candidate
Cargo package and captures Cargo's actual compiler-artifact JSON. It never
pretends that a hypothetical package has been compiled.

Usage:
    python3 automation/compare_physical_build_options.py
    python3 automation/compare_physical_build_options.py --compiler
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"
MONOLITH = "temporal_worker"
ORCHESTRATOR = "temporal_orchestrator"
# This is a DESIGN hypothesis only. Once a candidate Cargo package exists,
# its *actual* manifest replaces this proposed set in all measurements.
PROPOSED_ORCHESTRATOR_DEPS = frozenset({
    "temporal_gateway", "seo_application", "operation_protocol", "contracts",
    "primitives", "seo_ports",
})
PROVIDER_OWNERS = frozenset({
    "graph_neo4j", "vector_qdrant", "postgres_store", "runtime_composition",
    "infrastructure",
})
SOURCE_OWNERS = ("postgres_store", "graph_neo4j", "vector_qdrant", "temporal_gateway")
OWNER_SOURCE_PATHS = {
    "postgres_store": "app/rust/crates/postgres_store/src/lib.rs",
    "graph_neo4j": "app/rust/crates/graph_neo4j/src/lib.rs",
    "vector_qdrant": "app/rust/crates/vector_qdrant/src/lib.rs",
    "temporal_gateway": "app/rust/crates/temporal_gateway/src/lib.rs",
}


class EvidenceError(RuntimeError):
    """Incomplete or inconsistent graph/compiler evidence, never a pass."""


def cargo_manifest(path: Path) -> dict[str, Any]:
    try:
        contents = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
        raise EvidenceError(f"cannot parse {path}: {exc}") from exc
    if not isinstance(contents, dict):
        raise EvidenceError(f"invalid Cargo manifest: {path}")
    return contents


def manifest_dependencies(manifest: dict[str, Any]) -> list[tuple[str, Any]]:
    """Conservative production edges; includes every platform and optional dep.

    Cargo feature selection needs `cargo metadata` for exact resolution. We
    deliberately overapproximate features here and label these edges as such.
    Dev dependencies are excluded from `cargo build --bin` closure only.
    """
    result: list[tuple[str, Any]] = []
    for section in ("dependencies", "build-dependencies"):
        result.extend(manifest.get(section, {}).items())
    for block in manifest.get("target", {}).values():
        if not isinstance(block, dict):
            raise EvidenceError("non-table target-specific dependency block")
        for section in ("dependencies", "build-dependencies"):
            result.extend(block.get(section, {}).items())
    return result


def workspace_graph(workspace: Path = RUST) -> tuple[dict[str, set[str]], dict[str, Path]]:
    top = cargo_manifest(workspace / "Cargo.toml")
    members = top.get("workspace", {}).get("members")
    if not isinstance(members, list) or not members:
        raise EvidenceError("workspace members are missing; cannot prove package closure")

    manifests: dict[str, Path] = {}
    for relative in members:
        if not isinstance(relative, str) or "*" in relative:
            raise EvidenceError(f"non-explicit workspace member {relative!r}")
        manifest_file = (workspace / relative / "Cargo.toml").resolve()
        if not manifest_file.is_relative_to(workspace.resolve()) or not manifest_file.is_file():
            raise EvidenceError(f"workspace member missing/outside repository: {relative}")
        name = cargo_manifest(manifest_file).get("package", {}).get("name")
        if not isinstance(name, str) or not name or name in manifests:
            raise EvidenceError(f"invalid or duplicate workspace package {name!r}")
        manifests[name] = manifest_file

    graph: dict[str, set[str]] = {package: set() for package in manifests}
    for package, manifest_file in manifests.items():
        manifest = cargo_manifest(manifest_file)
        for declared_name, setting in manifest_dependencies(manifest):
            if not isinstance(setting, dict) or "path" not in setting:
                continue
            resolved = (manifest_file.parent / setting["path"] / "Cargo.toml").resolve()
            if not resolved.is_file():
                raise EvidenceError(f"{package}: missing path dependency {declared_name}")
            actual = cargo_manifest(resolved).get("package", {}).get("name")
            if actual not in manifests or resolved != manifests[actual]:
                raise EvidenceError(f"{package}: path dependency {declared_name} outside members")
            if setting.get("package", declared_name) != actual:
                raise EvidenceError(f"{package}: path package mismatch {declared_name}->{actual}")
            graph[package].add(actual)

    if not graph.get(MONOLITH):
        raise EvidenceError("temporal_worker missing or empty; comparison is invalid")
    return graph, manifests


def closure(graph: dict[str, set[str]], roots: set[str]) -> set[str]:
    if unknown := roots - graph.keys():
        raise EvidenceError(f"unknown workspace packages: {sorted(unknown)}")
    discovered: set[str] = set()
    pending = list(roots)
    while pending:
        package = pending.pop()
        if package in discovered:
            continue
        discovered.add(package)
        pending.extend(graph[package] - discovered)
    return discovered


def reverse_closure(graph: dict[str, set[str]], owner: str) -> set[str]:
    reverse: dict[str, set[str]] = defaultdict(set)
    for package, dependencies in graph.items():
        for dependency in dependencies:
            reverse[dependency].add(package)
    return closure({name: reverse[name] for name in graph}, {owner})


def manifest_report(graph: dict[str, set[str]], paths: dict[str, Path]) -> dict[str, Any]:
    provenance = subprocess.run(
        ["git", "rev-parse", "HEAD", "HEAD^{tree}"],
        cwd=ROOT, text=True, capture_output=True,
    )
    revisions = provenance.stdout.splitlines()
    if provenance.returncode or len(revisions) != 2 or any(len(value) != 40 for value in revisions):
        raise EvidenceError("cannot establish Git source/tree provenance for A/B evidence")
    baseline = closure(graph, {MONOLITH})
    actual_candidate = ORCHESTRATOR in graph
    desired_deps = graph[ORCHESTRATOR] if actual_candidate else set(PROPOSED_ORCHESTRATOR_DEPS)
    candidate = closure(graph, desired_deps) | ({ORCHESTRATOR} if actual_candidate else set())
    candidate_providers = sorted(candidate & PROVIDER_OWNERS)
    monolith_providers = sorted(baseline & PROVIDER_OWNERS)
    if not monolith_providers:
        raise EvidenceError("baseline unexpectedly has no provider edges; assumptions changed")
    if candidate_providers:
        raise EvidenceError(f"candidate transitively links provider owners: {candidate_providers}")

    observed_source = ROOT / "app/rust/services/temporal/src/main.rs"
    observed_workflows = ROOT / "app/rust/services/temporal/src/workflows/mod.rs"
    observed_queue = ROOT / "app/rust/services/temporal/src/workflows/runtime.rs"
    observed_docker = ROOT / "app/rust/services/temporal/Dockerfile"
    source_checks = {
        "legacy_queue_registered": 'build_worker_options("alegria-pipeline", acts)' in observed_source.read_text(),
        "legacy_activities_registered": ".register_activities(acts)" in observed_workflows.read_text(),
        "legacy_options_inherit_queue": ".task_queue(" not in observed_queue.read_text(),
        "legacy_docker_cobuilds": "-p temporal_worker -p cli_tools" in observed_docker.read_text(),
    }
    if not all(source_checks.values()):
        failed = sorted(key for key, value in source_checks.items() if not value)
        raise EvidenceError(f"operational topology changed; review A/B assumptions: {failed}")

    return {
        "schema_version": 1,
        "source_git_sha": revisions[0],
        "source_git_tree_sha": revisions[1],
        "status": "MANIFEST_ONLY_UNCOMPILED" if not actual_candidate else "CANDIDATE_MANIFEST_PRESENT_UNCOMPILED",
        "confidence": "conservative_workspace_manifest_closure_not_rustc",
        "workspace_manifest_count": len(graph),
        "A_native_monolith": {
            "package": MONOLITH,
            "workspace_closure_excluding_self": sorted(baseline - {MONOLITH}),
            "workspace_dependency_count": len(baseline) - 1,
            "provider_dependencies": monolith_providers,
            "binary_manifest": str(paths[MONOLITH].relative_to(ROOT)),
        },
        "A_modular_monolith": {
            "package": MONOLITH,
            "same_cargo_package_deps": True,
            "workspace_dependency_count": len(baseline) - 1,
            "caveat": "moving Rust modules/bins inside same Cargo package does not sever Cargo dependency edges",
        },
        "B_separate_orchestrator": {
            "package": ORCHESTRATOR,
            "exists": actual_candidate,
            "hypothetical": not actual_candidate,
            "declared_workspace_deps": sorted(desired_deps),
            "workspace_closure_excluding_self": sorted(candidate - {ORCHESTRATOR}),
            "workspace_dependency_count": len(candidate) - (1 if actual_candidate else 0),
            "provider_dependencies": candidate_providers,
        },
        "provider_edits_reverse_closure": {
            owner: sorted(reverse_closure(graph, owner)) for owner in SOURCE_OWNERS
        },
        "current_operational_invariants": source_checks,
        "actual_rustc_evidence": False,
        "limitations": [
            "Optional/target-specific dependencies may be overapproximated without cargo metadata",
            "Hypothetical B cannot be counted as an actual Cargo package, compiler artifact or deployable service",
            "Existing monolith/outbox remain in workspace and Docker builds, so total CI time need not decrease",
            "Old Temporal history/queue must remain unchanged; new Task Queue is routing, not effect authorization",
        ],
    }


def compiler_artifacts(command: list[str], cwd: Path, *, isolated_target: bool = False) -> dict[str, Any]:
    started = time.monotonic()
    # An opt-in isolated source study must not reuse, invalidate or poison the
    # current checkout's target directory even if the caller configured one.
    environment = os.environ.copy()
    if isolated_target:
        environment["CARGO_TARGET_DIR"] = str(cwd / "target")
    process = subprocess.Popen(
        command, cwd=cwd, env=environment,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    stdout, stderr = process.communicate()
    units: list[dict[str, Any]] = []
    finished = False
    for line in stdout.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("reason") == "compiler-artifact":
            target = record.get("target", {})
            if not isinstance(record.get("fresh"), bool) or not isinstance(target.get("kind"), list):
                raise EvidenceError("incomplete Cargo compiler-artifact")
            units.append({
                "package_id": record.get("package_id"),
                "target_name": target.get("name"),
                "kind": target["kind"],
                "fresh": record["fresh"],
                "executable": record.get("executable"),
            })
        if record.get("reason") == "build-finished":
            finished = record.get("success") is True
    if process.returncode or not finished or not units:
        raise EvidenceError(f"compiler command failed/incomplete: {command}; stderr={stderr[-4000:]}")
    if not any("bin" in item["kind"] and item["executable"] for item in units):
        raise EvidenceError("Cargo emitted no executable binary artifact")
    return {
        "command": command,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "compiler_artifacts": units,
        "non_fresh_units": [u for u in units if not u["fresh"]],
    }


def binary_snapshot(build: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Retain executable mtime/hash, not just Cargo's `fresh` flag."""
    result = {}
    for unit in build["compiler_artifacts"]:
        if "bin" not in unit["kind"] or not unit["executable"]:
            continue
        path = Path(unit["executable"])
        if not path.is_file():
            raise EvidenceError(f"Cargo executable does not exist: {path}")
        digest = hashlib.sha256()
        with path.open("rb") as executable:
            for chunk in iter(lambda: executable.read(1024 * 1024), b""):
                digest.update(chunk)
        result[unit["target_name"]] = {
            "mtime_ns": path.stat().st_mtime_ns,
            "sha256": digest.hexdigest(),
        }
    if {MONOLITH, ORCHESTRATOR} - result.keys():
        raise EvidenceError("A/B compiler invocation omitted one binary")
    return result


def build_both(workspace: Path) -> dict[str, Any]:
    result = compiler_artifacts([
        "cargo", "build", "--locked", "--timings", "--message-format=json-render-diagnostics",
        "--package", MONOLITH, "--bin", MONOLITH,
        "--package", ORCHESTRATOR, "--bin", ORCHESTRATOR,
    ], workspace, isolated_target=True)
    result["binaries"] = binary_snapshot(result)
    return result


def isolated_source_tree(destination: Path) -> str:
    """Export only tracked exact-HEAD source; never perturb the active checkout."""
    clean = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=no", "--", "app/rust"],
        cwd=ROOT, text=True, capture_output=True,
    )
    if clean.returncode or clean.stdout.strip():
        raise EvidenceError("tracked Rust sources are dirty; commit exact candidate before compiler probe")
    sha = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, capture_output=True,
    )
    if sha.returncode or len(sha.stdout.strip()) != 40:
        raise EvidenceError("cannot resolve immutable source SHA")
    archive = subprocess.Popen(
        ["git", "archive", "--format=tar", "HEAD"], cwd=ROOT,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    assert archive.stdout is not None
    extract = subprocess.run(
        ["tar", "-xf", "-", "-C", str(destination)],
        stdin=archive.stdout, capture_output=True, text=True,
    )
    archive.stdout.close()
    _, archive_err = archive.communicate()
    if archive.returncode or extract.returncode:
        raise EvidenceError(f"isolated exact-source export failed: {extract.stderr}; {archive_err!r}")
    return sha.stdout.strip()


def verify_actual_cargo_metadata(workspace: Path) -> dict[str, Any]:
    metadata = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=workspace, text=True, capture_output=True,
    )
    if metadata.returncode:
        raise EvidenceError(f"locked Cargo metadata failed: {metadata.stderr[-3000:]}")
    try:
        parsed = json.loads(metadata.stdout)
    except json.JSONDecodeError as exc:
        raise EvidenceError("cargo metadata emitted invalid JSON") from exc
    present = {p.get("name") for p in parsed.get("packages", [])}
    if {MONOLITH, ORCHESTRATOR} - present:
        raise EvidenceError("Cargo metadata missing one side of A/B")
    if not isinstance(parsed.get("resolve"), dict) or not parsed["resolve"].get("nodes"):
        raise EvidenceError("Cargo metadata has no resolved dependency graph")
    package_ids = {p["id"]: p["name"] for p in parsed["packages"]}
    package_names = {name: identifier for identifier, name in package_ids.items()}
    resolved_edges = {}
    for node in parsed["resolve"]["nodes"]:
        resolved_edges[node["id"]] = {
            item["pkg"] for item in node.get("deps", [])
            if isinstance(item, dict) and isinstance(item.get("pkg"), str)
        }
    results = {}
    for candidate in (MONOLITH, ORCHESTRATOR):
        visited = set()
        pending = [package_names[candidate]]
        while pending:
            identifier = pending.pop()
            if identifier in visited:
                continue
            if identifier not in resolved_edges:
                raise EvidenceError(f"unresolved metadata dependency node: {identifier}")
            visited.add(identifier)
            pending.extend(resolved_edges[identifier] - visited)
        owners = {package_ids[identifier] for identifier in visited}
        results[candidate] = {
            "resolved_all_dependency_package_count": len(visited) - 1,
            "resolved_provider_crates": sorted(owners & PROVIDER_OWNERS),
        }
    if results[ORCHESTRATOR]["resolved_provider_crates"]:
        raise EvidenceError("real Cargo metadata resolves forbidden providers in B")
    return results


def owner_rebuild_probe(report: dict[str, Any]) -> dict[str, Any]:
    """Compile all four owner-only deltas inside an ephemeral exact-SHA export.

    This does NOT modify even temporarily a byte of the current worktree.
    It keeps one target directory within the ephemeral export so `fresh` and
    binary relink evidence is meaningful between owner perturbations.
    """
    if not report["B_separate_orchestrator"]["exists"]:
        raise EvidenceError("four-owner compiler A/B requires a real temporal_orchestrator package")
    for executable in ("cargo", "rustc", "git", "tar"):
        if not shutil.which(executable):
            raise EvidenceError(f"{executable} unavailable; cannot measure compiler-owner A/B")
    with tempfile.TemporaryDirectory(prefix="alegria-physical-owner-ab-") as folder:
        exported = Path(folder) / "source"
        exported.mkdir()
        sha = isolated_source_tree(exported)
        workspace = exported / "app" / "rust"
        actual_resolve = verify_actual_cargo_metadata(workspace)
        baseline = build_both(workspace)
        baseline_binaries = baseline["binaries"]
        a_closure = set(report["A_native_monolith"]["workspace_closure_excluding_self"])
        b_closure = set(report["B_separate_orchestrator"]["workspace_closure_excluding_self"])
        experiments = []
        for index, (owner, source_path) in enumerate(OWNER_SOURCE_PATHS.items()):
            source = exported / source_path
            original = source.read_bytes()
            if not original.endswith(b"\n"):
                raise EvidenceError(f"owner source missing terminal newline: {owner}")
            probe_bytes = (f"#[allow(dead_code)]\n"
                           f"const PHYSICAL_AB_OWNER_PROBE: &str = \"{sha}_{index}\";\n").encode()
            try:
                source.write_bytes(original + probe_bytes)
                build = build_both(workspace)
            finally:
                source.write_bytes(original)
            if source.read_bytes() != original:
                raise EvidenceError(f"owner source restore failed in isolated export: {owner}")
            rebuilt = [unit for unit in build["non_fresh_units"]]
            if not any(unit["target_name"] == owner for unit in rebuilt):
                raise EvidenceError(f"{owner}: no actual non-fresh owner rustc unit")
            relinked = sorted({
                unit["target_name"] for unit in rebuilt if "bin" in unit["kind"]
            } & {MONOLITH, ORCHESTRATOR})
            if (MONOLITH in relinked) != (owner in a_closure):
                raise EvidenceError(f"{owner}: monolith relink inconsistent with Cargo closure")
            touched = sorted(
                name for name, image in build["binaries"].items()
                if image["mtime_ns"] != baseline_binaries[name]["mtime_ns"]
            )
            changed = sorted(
                name for name, image in build["binaries"].items()
                if image["sha256"] != baseline_binaries[name]["sha256"]
            )
            # A shared temporal_gateway is today's positive control that
            # detects a false-positive 'always fresh B' test harness.
            expected_b_relink = owner in b_closure
            if (ORCHESTRATOR in relinked) != expected_b_relink:
                raise EvidenceError(f"{owner}: orchestrator binary relink unexpectedly {relinked}")
            if ORCHESTRATOR in touched and not expected_b_relink:
                raise EvidenceError(f"{owner}: unrelated owner wrote orchestrator binary")
            probe_result = {
                "owner": owner,
                "modified_only_in_temporary_export": source_path,
                "compiler_rebuilt_units": rebuilt,
                "compiler_relinked_bins": relinked,
                "binary_files_written": touched,
                "binary_hash_changed": changed,
                "elapsed_seconds": build["elapsed_seconds"],
            }
            restored = build_both(workspace)
            baseline_binaries = restored["binaries"]
            probe_result["baseline_restore_elapsed_seconds"] = restored["elapsed_seconds"]
            experiments.append(probe_result)
        result = dict(report)
        result["source_sha"] = sha
        result["compiler"] = {
            "isolated_git_archive": True,
            "source_tree_mutated": False,
            "cargo_resolved_closure": actual_resolve,
            "baseline": baseline,
            "four_owner_probes": experiments,
        }
        result["actual_rustc_evidence"] = True
        result["status"] = "FOUR_OWNER_ACTUAL_RUSTC_AB_CAPTURED"
        return result


def compiled_report(report: dict[str, Any]) -> dict[str, Any]:
    if not report["B_separate_orchestrator"]["exists"]:
        raise EvidenceError("compiler comparison requires a real temporal_orchestrator Cargo package")
    for executable in ("cargo", "rustc", "git", "tar"):
        if not shutil.which(executable):
            raise EvidenceError(f"{executable} unavailable; compiler evidence cannot be inferred from manifests")
    with tempfile.TemporaryDirectory(prefix="alegria-physical-ab-") as folder:
        exported = Path(folder) / "source"
        exported.mkdir()
        sha = isolated_source_tree(exported)
        workspace = exported / "app" / "rust"
        actual_resolve = verify_actual_cargo_metadata(workspace)
        runs = {}
        for label, package in (("A", MONOLITH), ("B", ORCHESTRATOR)):
            command = [
                "cargo", "build", "--locked", "--timings", "--message-format=json-render-diagnostics",
                "--package", package, "--bin", package,
            ]
            runs[label] = compiler_artifacts(command, workspace, isolated_target=True)
    report = dict(report)
    report["source_sha"] = sha
    report["compiler"] = runs
    report["cargo_resolved_closure"] = actual_resolve
    report["actual_rustc_evidence"] = True
    report["status"] = "ACTUAL_CARGO_BUILDS_CAPTURED_NOT_ISOLATION_PROVEN"
    report["limitations"] = [
        *report["limitations"],
        "A/B sequential builds share target cache; record cold/warm and owner-perturbation measurements separately",
        "Compiler success alone does not prove cross-queue semantics, effect fences, runtime parity or CI SLO",
    ]
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", action="store_true", help="require real A/B Cargo metadata and compiler artifacts")
    parser.add_argument("--compiler-owners", action="store_true", help="measure four owner-only edits in isolated exact-HEAD export")
    arguments = parser.parse_args(argv)
    if arguments.compiler and arguments.compiler_owners:
        parser.error("choose --compiler or --compiler-owners")
    try:
        graph, manifests = workspace_graph()
        report = manifest_report(graph, manifests)
        if arguments.compiler:
            report = compiled_report(report)
        elif arguments.compiler_owners:
            report = owner_rebuild_probe(report)
    except EvidenceError as exc:
        print(json.dumps({"status": "EVIDENCE_BLOCKED", "error": str(exc), "actual_rustc_evidence": False}, indent=2))
        return 2
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
