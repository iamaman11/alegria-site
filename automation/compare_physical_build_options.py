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
import re
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
PG_EFFECT_OWNER = "temporal_pg_effects"
QDRANT_EFFECT_OWNER = "temporal_qdrant_effects"
QDRANT_LEGACY_PARENT_FEATURE = "legacy-qdrant-v1"
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
PG_ALLOWED_PROVIDER_OWNERS = frozenset({"postgres_store"})
QDRANT_ALLOWED_PROVIDER_OWNERS = frozenset({"vector_qdrant"})
# Cargo workspace names alone are insufficient: a new standalone binary could
# bypass a local provider crate by taking a vendor SDK directly. Match package
# names from the resolved locked Cargo graph (never substring-match arbitrary
# crate names). For the PG Activity owner only SQL drivers are admissible.
PROVIDER_TRANSPORT_PREFIXES = (
    "sqlx", "tokio-postgres", "postgres", "deadpool-postgres", "pg-",
    "neo4rs", "neo4j", "bolt-client", "qdrant", "redis", "mongodb",
)
GRAPH_VECTOR_TRANSPORT_PREFIXES = (
    "neo4rs", "neo4j", "bolt-client", "qdrant",
)
SOURCE_OWNERS = ("postgres_store", "graph_neo4j", "vector_qdrant", "temporal_orchestrator")
OWNER_SOURCE_PATHS = {
    "postgres_store": "app/rust/crates/postgres_store/src/lib.rs",
    "graph_neo4j": "app/rust/crates/graph_neo4j/src/lib.rs",
    "vector_qdrant": "app/rust/crates/vector_qdrant/src/lib.rs",
    "temporal_orchestrator": "app/rust/services/temporal_orchestrator/src/main.rs",
}
# A Cargo compiler-artifact with `fresh: false` is not proof that rustc ran
# while a compiler cache wrapper is enabled. Both environment and Cargo's
# config layering must be neutralized in empirical A/B probe subprocesses.
RUSTC_WRAPPER_ENV = frozenset({
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
})
CARGO_DISABLE_WRAPPER_CONFIG = (
    "--config", 'build.rustc-wrapper=""',
    "--config", 'build.rustc-workspace-wrapper=""',
)


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


def owner_must_relink_binary(owner: str, binary_package: str, dependencies: set[str]) -> bool:
    """The binary owner itself also rebuilds, not only its dependency crates.

    Manifest reports intentionally exclude the package itself from their
    dependency lists. Forgetting this case made a direct orchestrator edit
    look like an illegal relink in the opt-in physical experiment.
    """
    return owner == binary_package or owner in dependencies


def generic_parent_default_feature_boundary(
    graph: dict[str, set[str]], orchestrator_manifest: dict[str, Any],
) -> dict[str, Any]:
    """Classify native generic parent without confusing optional with default.

    The existing workspace_graph deliberately overapproximates all feature
    edges. For this *one reviewed legacy dependency*, inspect the real Cargo
    feature graph and remove it only if the feature is not default and an
    explicit `dep:qdrant_contracts` dependency gate exists. Physical rustc
    isolation still needs an actual --locked no-default compile/relink probe.
    """
    dependencies = orchestrator_manifest.get("dependencies", {})
    qdrant = dependencies.get("qdrant_contracts")
    features = orchestrator_manifest.get("features", {})
    feature_separated = (
        isinstance(qdrant, dict)
        and qdrant.get("optional") is True
        and features.get(QDRANT_LEGACY_PARENT_FEATURE) == ["dep:qdrant_contracts"]
    )
    is_gated = feature_separated and features.get("default", []) == []
    conservative = closure(graph, {ORCHESTRATOR})
    if feature_separated:
        default_graph = {package: deps.copy() for package, deps in graph.items()}
        default_graph[ORCHESTRATOR].discard("qdrant_contracts")
        no_default = closure(default_graph, {ORCHESTRATOR})
    else:
        no_default = conservative
    computed = no_default if is_gated else conservative
    return {
        "legacy_feature": QDRANT_LEGACY_PARENT_FEATURE,
        "default_legacy_dependency_gated": is_gated,
        "explicit_no_default_feature_isolation_declared": feature_separated,
        "no_default_workspace_closure_excluding_self": sorted(no_default - {ORCHESTRATOR}),
        "legacy_qdrant_contracts_in_no_default_closure": "qdrant_contracts" in no_default,
        "default_workspace_closure_excluding_self": sorted(computed - {ORCHESTRATOR}),
        "legacy_qdrant_contracts_in_default_closure": "qdrant_contracts" in computed,
        "feature_union_workspace_closure_excluding_self": sorted(conservative - {ORCHESTRATOR}),
        "confidence": "manifest_feature_boundary_only_not_physical_rustc",
    }


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
    pg_exists = PG_EFFECT_OWNER in graph
    pg_closure = closure(graph, {PG_EFFECT_OWNER}) if pg_exists else set()
    illegal_pg = sorted((pg_closure & PROVIDER_OWNERS) - PG_ALLOWED_PROVIDER_OWNERS)
    if illegal_pg:
        raise EvidenceError(f"PG Activity owner transitively links forbidden providers: {illegal_pg}")
    qdrant_exists = QDRANT_EFFECT_OWNER in graph
    qdrant_closure = closure(graph, {QDRANT_EFFECT_OWNER}) if qdrant_exists else set()
    illegal_qdrant = sorted((qdrant_closure & PROVIDER_OWNERS) - QDRANT_ALLOWED_PROVIDER_OWNERS)
    if illegal_qdrant:
        raise EvidenceError(f"Qdrant Activity owner transitively links forbidden providers: {illegal_qdrant}")
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
        "B_generic_parent_default_feature_boundary": (
            generic_parent_default_feature_boundary(graph, cargo_manifest(paths[ORCHESTRATOR]))
            if actual_candidate else None
        ),
        "C_postgres_activity_owner": {
            "package": PG_EFFECT_OWNER,
            "exists": pg_exists,
            "workspace_closure_excluding_self": sorted(pg_closure - {PG_EFFECT_OWNER}),
            "workspace_dependency_count": len(pg_closure) - (1 if pg_exists else 0),
            "provider_dependencies": sorted(pg_closure & PROVIDER_OWNERS),
            "allowed_provider_dependencies": sorted(PG_ALLOWED_PROVIDER_OWNERS),
        },
        "D_qdrant_activity_owner": {
            "package": QDRANT_EFFECT_OWNER,
            "exists": qdrant_exists,
            "workspace_closure_excluding_self": sorted(qdrant_closure - {QDRANT_EFFECT_OWNER}),
            "workspace_dependency_count": len(qdrant_closure) - (1 if qdrant_exists else 0),
            "provider_dependencies": sorted(qdrant_closure & PROVIDER_OWNERS),
            "allowed_provider_dependencies": sorted(QDRANT_ALLOWED_PROVIDER_OWNERS),
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


def parse_gnu_time_report(raw: str) -> dict[str, float | int]:
    """Accept only complete resource accounting from GNU time's child tree."""
    fields = raw.strip().split("\t")
    if len(fields) != 6 or fields[0] != "PHYSICAL_AB_RESOURCE":
        raise EvidenceError("GNU time resource output is absent or malformed")
    try:
        elapsed, user, system = (float(value) for value in fields[1:4])
        rss = int(fields[4])
        status = int(fields[5])
    except ValueError as exc:
        raise EvidenceError("GNU time resource fields are not numeric") from exc
    if (status or min(elapsed, user, system) < 0 or rss <= 0
            or any(value != value or value == float("inf") for value in (elapsed, user, system))):
        raise EvidenceError("GNU time recorded failed or invalid resource accounting")
    return {
        "wall_seconds": elapsed,
        "user_cpu_seconds": user,
        "system_cpu_seconds": system,
        "peak_resident_kib": rss,
    }


def compiler_artifacts(
    command: list[str], cwd: Path, *, isolated_target: bool = False,
    target_dir: Path | None = None, measure_resources: bool = False,
) -> dict[str, Any]:
    started = time.monotonic()
    environment = os.environ.copy()
    if measure_resources and not isolated_target:
        raise EvidenceError("CPU/RAM rustc evidence requires wrapper-free isolated compilation")
    if isolated_target:
        # The source study must never reuse/poison the checkout's target, any
        # sccache object, or rustc incremental compilation artifacts. Cargo
        # can otherwise emit fresh:false for a wrapper cache hit rather than
        # executing rustc, invalidating this empirical comparison.
        rustc = shutil.which("rustc")
        if not rustc:
            raise EvidenceError("rustc unavailable for wrapper-free compiler experiment")
        for name in RUSTC_WRAPPER_ENV:
            environment.pop(name, None)
        for name in tuple(environment):
            if name.startswith("SCCACHE_"):
                environment.pop(name)
        environment["RUSTC"] = rustc
        environment["CARGO_BUILD_RUSTC"] = rustc
        environment["CARGO_INCREMENTAL"] = "0"
        environment["CARGO_TARGET_DIR"] = str(target_dir or cwd / "target")
        if not command or command[0] != "cargo":
            raise EvidenceError("isolated rustc probe requires direct Cargo invocation")
        # Explicit CLI config outranks Cargo home/workspace config; otherwise
        # an unnoticed [build] rustc-wrapper can survive environment cleanup.
        command = [command[0], *CARGO_DISABLE_WRAPPER_CONFIG, *command[1:]]
    if measure_resources and not shutil.which("time"):
        raise EvidenceError("GNU time unavailable; cannot claim CPU/RAM resource evidence")
    # Per-Cargo GNU time tracks waited rustc/linker descendants, and its
    # max-RSS is scoped to this invocation (unlike cumulative RUSAGE_CHILDREN).
    # It does not measure summed simultaneous resident bytes across processes.
    with tempfile.TemporaryDirectory(prefix="alegria-cargo-resource-") as resources:
        timing_file = Path(resources) / "gnu-time.txt"
        invocation = (
            ["time", "-f", "PHYSICAL_AB_RESOURCE\t%e\t%U\t%S\t%M\t%x",
             "-o", str(timing_file), *command]
            if measure_resources else command
        )
        process = subprocess.Popen(
            invocation, cwd=cwd, env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        stdout, stderr = process.communicate()
        if process.returncode:
            raise EvidenceError(f"compiler command failed: {command}; stderr={stderr[-4000:]}")
        resources_observed = (
            parse_gnu_time_report(timing_file.read_text(encoding="utf-8"))
            if measure_resources else None
        )
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
        "compiler_execution": {
            "sccache_disabled": isolated_target,
            "rustc_wrapper_disabled": isolated_target,
            "rustc_incremental_disabled": isolated_target,
            "isolated_cargo_target": isolated_target,
        },
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "process_tree_resources": resources_observed,
        "resource_measurement": (
            "GNU time: Cargo plus waited rustc/linker children; per-process peak RSS, not total concurrent RSS"
            if measure_resources else "NOT_MEASURED"
        ),
        "compiler_artifacts": units,
        "non_fresh_units": [u for u in units if not u["fresh"]],
    }


def binary_snapshot(
    build: dict[str, Any], required_binaries: tuple[str, ...] = (MONOLITH, ORCHESTRATOR),
) -> dict[str, dict[str, Any]]:
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
    if set(required_binaries) - result.keys():
        raise EvidenceError(f"compiler invocation omitted binaries: {sorted(set(required_binaries) - result.keys())}")
    return result


def build_physical_targets(workspace: Path, *, pg_exists: bool) -> dict[str, Any]:
    packages = (MONOLITH, ORCHESTRATOR, PG_EFFECT_OWNER) if pg_exists else (MONOLITH, ORCHESTRATOR)
    result = compiler_artifacts([
        "cargo", "build", "--locked", "--timings", "--message-format=json-render-diagnostics",
        *(argument for package in packages for argument in ("--package", package, "--bin", package)),
    ], workspace, isolated_target=True, measure_resources=True)
    result["binaries"] = binary_snapshot(result, packages)
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


def verify_pr_source_provenance() -> dict[str, Any]:
    """Pin a physical study to the canonical PR HEAD *tree*, not the merge SHA.

    GitHub's pull_request checkout is its synthetic merge commit; the event's
    CI_SOURCE_SHA names the PR head commit. They may have different commit IDs
    even when they contain exactly the same bytes. No physical isolation claim
    may survive a mismatched tree or an unavailable canonical commit object.
    """

    def git_revision(revision: str) -> str:
        result = subprocess.run(
            ["git", "rev-parse", "--verify", revision],
            cwd=ROOT, text=True, capture_output=True,
        )
        value = result.stdout.strip()
        if result.returncode or not re.fullmatch(r"[0-9a-f]{40}", value):
            raise EvidenceError(
                f"cannot resolve exact Git object {revision}; "
                "fetch the canonical CI_SOURCE_SHA before the physical probe"
            )
        return value

    checkout_sha = git_revision("HEAD^{commit}")
    checkout_tree = git_revision("HEAD^{tree}")
    canonical_sha = os.environ.get("CI_SOURCE_SHA", "").strip()
    in_pr_ci = (os.environ.get("GITHUB_ACTIONS") == "true"
                and os.environ.get("GITHUB_EVENT_NAME") == "pull_request")
    if not canonical_sha:
        if in_pr_ci:
            raise EvidenceError("pull_request physical probe requires canonical CI_SOURCE_SHA")
        return {
            "checkout_commit_sha": checkout_sha,
            "checkout_tree_sha": checkout_tree,
            "canonical_pr_head_commit_sha": None,
            "canonical_pr_head_tree_sha": None,
            "trees_equal": None,
            "scope": "local_checkout_only_no_pr_head_claim",
        }
    if not re.fullmatch(r"[0-9a-f]{40}", canonical_sha):
        raise EvidenceError("CI_SOURCE_SHA is not an exact lower-case Git commit SHA")
    resolved_head = git_revision(f"{canonical_sha}^{{commit}}")
    if resolved_head != canonical_sha:
        raise EvidenceError("CI_SOURCE_SHA does not resolve to its exact Git commit")
    canonical_tree = git_revision(f"{canonical_sha}^{{tree}}")
    if checkout_tree != canonical_tree:
        raise EvidenceError(
            "synthetic PR merge source tree differs from canonical CI_SOURCE_SHA tree; "
            "no physical A/B evidence may be attributed to the PR head"
        )
    return {
        "checkout_commit_sha": checkout_sha,
        "checkout_tree_sha": checkout_tree,
        "canonical_pr_head_commit_sha": canonical_sha,
        "canonical_pr_head_tree_sha": canonical_tree,
        "trees_equal": True,
        "scope": "canonical_pr_head_tree_verified",
    }


def verify_actual_cargo_metadata(workspace: Path) -> dict[str, Any]:
    rustc_version = subprocess.run(
        ["rustc", "-vV"], cwd=workspace, text=True, capture_output=True,
    )
    hosts = re.findall(r"^host: ([A-Za-z0-9_\-]+)$", rustc_version.stdout, flags=re.MULTILINE)
    if rustc_version.returncode or len(hosts) != 1:
        raise EvidenceError("cannot pin Cargo metadata to rustc host target")
    host_target = hosts[0]
    metadata = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1",
         "--filter-platform", host_target],
        cwd=workspace, text=True, capture_output=True,
    )
    if metadata.returncode:
        raise EvidenceError(f"locked Cargo metadata failed: {metadata.stderr[-3000:]}")
    try:
        parsed = json.loads(metadata.stdout)
    except json.JSONDecodeError as exc:
        raise EvidenceError("cargo metadata emitted invalid JSON") from exc
    packages = parsed.get("packages", [])
    present = {p.get("name") for p in packages}
    if {MONOLITH, ORCHESTRATOR} - present:
        raise EvidenceError("Cargo metadata missing one side of A/B")
    if not isinstance(parsed.get("resolve"), dict) or not parsed["resolve"].get("nodes"):
        raise EvidenceError("Cargo metadata has no resolved dependency graph")
    package_ids = {p["id"]: p["name"] for p in packages}
    ids_to_packages = {p["id"]: p for p in packages}
    workspace_ids = set(parsed.get("workspace_members", []))
    if any(
        identifier not in workspace_ids and p.get("source") is None
        for identifier, p in ids_to_packages.items()
    ):
        raise EvidenceError("Cargo resolved undeclared external path package")
    package_names = {
        package_ids[identifier]: identifier
        for identifier in workspace_ids
        if identifier in package_ids
    }
    if {MONOLITH, ORCHESTRATOR} - package_names.keys():
        raise EvidenceError("Cargo metadata missing one workspace-owned A/B target")
    resolved_edges = {}
    for node in parsed["resolve"]["nodes"]:
        dependencies: set[str] = set()
        for item in node.get("deps", []):
            if not isinstance(item, dict) or not isinstance(item.get("pkg"), str):
                raise EvidenceError("Cargo metadata contains an unknown dependency edge")
            kinds = item.get("dep_kinds")
            if not isinstance(kinds, list) or not kinds:
                raise EvidenceError("Cargo metadata lacks dependency edge kind (dev/build/normal)")
            if any(
                isinstance(kind, dict) and kind.get("kind") in (None, "build")
                for kind in kinds
            ):
                dependencies.add(item["pkg"])
        resolved_edges[node["id"]] = dependencies
    results = {}
    candidates = (MONOLITH, ORCHESTRATOR, PG_EFFECT_OWNER)
    for candidate in candidates:
        if candidate not in package_names:
            if candidate == PG_EFFECT_OWNER:
                continue
            raise EvidenceError(f"missing workspace target {candidate}")
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
        vendor_names = {
            package_ids[identifier]
            for identifier in visited
            if ids_to_packages[identifier].get("source") is not None
        }
        forbidden_local = (
            (owners & PROVIDER_OWNERS)
            - (PG_ALLOWED_PROVIDER_OWNERS if candidate == PG_EFFECT_OWNER else set())
            if candidate != MONOLITH else set()
        )
        forbidden_vendor_prefixes = (
            GRAPH_VECTOR_TRANSPORT_PREFIXES
            if candidate == PG_EFFECT_OWNER else PROVIDER_TRANSPORT_PREFIXES
        )
        forbidden_vendor = sorted(
            name for name in vendor_names
            if any(name == prefix.rstrip("-") or name.startswith(prefix) for prefix in forbidden_vendor_prefixes)
        ) if candidate != MONOLITH else []
        if forbidden_local or forbidden_vendor:
            raise EvidenceError(
                f"{candidate}: forbidden resolved provider links: "
                f"workspace={sorted(forbidden_local)}, vendor={forbidden_vendor}"
            )
        results[candidate] = {
            "host_build_target": host_target,
            "feature_scope": "locked default-feature Cargo build; dev edges excluded; metadata may overapproximate workspace-wide feature unification",
            "resolved_all_dependency_package_count": len(visited) - 1,
            "resolved_provider_crates": sorted(owners & PROVIDER_OWNERS),
            "resolved_workspace_packages": sorted(owners & set(package_names)),
            "resolved_provider_transport_crates": sorted(
                name for name in vendor_names
                if any(name == prefix.rstrip("-") or name.startswith(prefix)
                       for prefix in PROVIDER_TRANSPORT_PREFIXES)
            ),
        }
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
    source_provenance = verify_pr_source_provenance()
    with tempfile.TemporaryDirectory(prefix="alegria-physical-owner-ab-") as folder:
        exported = Path(folder) / "source"
        exported.mkdir()
        sha = isolated_source_tree(exported)
        if sha != source_provenance["checkout_commit_sha"]:
            raise EvidenceError("checkout HEAD changed between identity check and git archive")
        workspace = exported / "app" / "rust"
        actual_resolve = verify_actual_cargo_metadata(workspace)
        pg_exists = report["C_postgres_activity_owner"]["exists"]
        packages = (MONOLITH, ORCHESTRATOR, PG_EFFECT_OWNER) if pg_exists else (MONOLITH, ORCHESTRATOR)
        baseline = build_physical_targets(workspace, pg_exists=pg_exists)
        baseline_binaries = baseline["binaries"]
        a_closure = set(report["A_native_monolith"]["workspace_closure_excluding_self"])
        b_closure = set(report["B_separate_orchestrator"]["workspace_closure_excluding_self"])
        c_closure = set(report["C_postgres_activity_owner"]["workspace_closure_excluding_self"])
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
                build = build_physical_targets(workspace, pg_exists=pg_exists)
            finally:
                source.write_bytes(original)
            if source.read_bytes() != original:
                raise EvidenceError(f"owner source restore failed in isolated export: {owner}")
            rebuilt = [unit for unit in build["non_fresh_units"]]
            if not any(unit["target_name"] == owner for unit in rebuilt):
                raise EvidenceError(f"{owner}: no actual non-fresh owner rustc unit")
            relinked = sorted({
                unit["target_name"] for unit in rebuilt if "bin" in unit["kind"]
            } & set(packages))
            expected_a_relink = owner_must_relink_binary(owner, MONOLITH, a_closure)
            if (MONOLITH in relinked) != expected_a_relink:
                raise EvidenceError(f"{owner}: monolith relink inconsistent with Cargo closure")
            touched = sorted(
                name for name, image in build["binaries"].items()
                if image["mtime_ns"] != baseline_binaries[name]["mtime_ns"]
            )
            changed = sorted(
                name for name, image in build["binaries"].items()
                if image["sha256"] != baseline_binaries[name]["sha256"]
            )
            if (MONOLITH in touched) != expected_a_relink:
                raise EvidenceError(f"{owner}: monolith executable write inconsistent with Cargo closure")
            # A direct orchestrator implementation edit must recompile and
            # relink the orchestrator even though its dependency *closure*
            # intentionally excludes the binary package itself.
            expected_b_relink = owner_must_relink_binary(owner, ORCHESTRATOR, b_closure)
            if (ORCHESTRATOR in relinked) != expected_b_relink:
                raise EvidenceError(f"{owner}: orchestrator binary relink unexpectedly {relinked}")
            if (ORCHESTRATOR in touched) != expected_b_relink:
                raise EvidenceError(f"{owner}: orchestrator executable write inconsistent with Cargo closure")
            if pg_exists:
                expected_c_relink = owner_must_relink_binary(owner, PG_EFFECT_OWNER, c_closure)
                if (PG_EFFECT_OWNER in relinked) != expected_c_relink:
                    raise EvidenceError(f"{owner}: PG Activity worker relink unexpectedly {relinked}")
                if (PG_EFFECT_OWNER in touched) != expected_c_relink:
                    raise EvidenceError(f"{owner}: PG Activity executable write inconsistent with Cargo closure")
            probe_result = {
                "owner": owner,
                "modified_only_in_temporary_export": source_path,
                "compiler_rebuilt_units": rebuilt,
                "cargo_non_fresh_bin_targets": relinked,
                "binary_files_written": touched,
                "binary_hash_changed": changed,
                "elapsed_seconds": build["elapsed_seconds"],
                "process_tree_resources": build["process_tree_resources"],
            }
            restored = build_physical_targets(workspace, pg_exists=pg_exists)
            baseline_binaries = restored["binaries"]
            probe_result["baseline_restore_elapsed_seconds"] = restored["elapsed_seconds"]
            probe_result["baseline_restore_resources"] = restored["process_tree_resources"]
            experiments.append(probe_result)
        result = dict(report)
        result["source_sha"] = sha
        result["source_tree_sha"] = source_provenance["checkout_tree_sha"]
        result["source_provenance"] = source_provenance
        result["compiler"] = {
            "isolated_git_archive": True,
            "source_tree_mutated": False,
            "cargo_resolved_closure": actual_resolve,
            "baseline": baseline,
            "compiled_packages": packages,
            "candidate_pg_activity_owner_included": pg_exists,
            "owner_probe_cache_regime": "one isolated Cargo target, sequential edits and restored source between probes",
            "four_owner_probes": experiments,
        }
        result["actual_rustc_evidence"] = True
        result["status"] = "FOUR_OWNER_ACTUAL_RUSTC_AB_CAPTURED"
        result["measurement_scope"] = (
            f"{len(packages)} Cargo binaries only: {', '.join(packages)}; "
            "not outbox_worker, CLI or entire Issue #94 owner topology"
        )
        return result


def compiled_report(report: dict[str, Any]) -> dict[str, Any]:
    if not report["B_separate_orchestrator"]["exists"]:
        raise EvidenceError("compiler comparison requires a real temporal_orchestrator Cargo package")
    for executable in ("cargo", "rustc", "git", "tar"):
        if not shutil.which(executable):
            raise EvidenceError(f"{executable} unavailable; compiler evidence cannot be inferred from manifests")
    source_provenance = verify_pr_source_provenance()
    with tempfile.TemporaryDirectory(prefix="alegria-physical-ab-") as folder:
        exported = Path(folder) / "source"
        exported.mkdir()
        sha = isolated_source_tree(exported)
        if sha != source_provenance["checkout_commit_sha"]:
            raise EvidenceError("checkout HEAD changed between identity check and git archive")
        workspace = exported / "app" / "rust"
        actual_resolve = verify_actual_cargo_metadata(workspace)
        runs = {}
        candidates = [("A", MONOLITH), ("B", ORCHESTRATOR)]
        if report["C_postgres_activity_owner"]["exists"]:
            candidates.append(("C", PG_EFFECT_OWNER))
        for label, package in candidates:
            command = [
                "cargo", "build", "--locked", "--timings", "--message-format=json-render-diagnostics",
                "--package", package, "--bin", package,
            ]
            # Independent artifact directories prevent B from inheriting A's
            # already-compiled local rlibs. Rust dependency fetches still use
            # the same immutable Cargo registry source; report that explicitly.
            target_dir = Path(folder) / f"target-{label}"
            cold = compiler_artifacts(
                command, workspace, isolated_target=True,
                target_dir=target_dir, measure_resources=True,
            )
            warm = compiler_artifacts(
                command, workspace, isolated_target=True,
                target_dir=target_dir, measure_resources=True,
            )
            runs[label] = {
                "package": package,
                "target_directory_isolated_from_other_options": True,
                "cold": cold,
                "warm": warm,
            }
    report = dict(report)
    report["source_sha"] = sha
    report["source_tree_sha"] = source_provenance["checkout_tree_sha"]
    report["source_provenance"] = source_provenance
    report["compiler"] = runs
    report["compiler_cache_regime"] = {
        "A_B_C_cold_targets": "distinct initially empty Cargo targets, no sccache/incremental rustc",
        "each_warm": "same target, same source, same default Cargo build features",
        "external_cache_shared": "Cargo registry and compiler executable only",
        "not_an_end_to_end_ci_slo": True,
    }
    report["cargo_resolved_closure"] = actual_resolve
    report["actual_rustc_evidence"] = True
    report["status"] = "ACTUAL_CARGO_BUILDS_CAPTURED_NOT_ISOLATION_PROVEN"
    report["measurement_scope"] = (
        f"{len(runs)} Cargo binaries only: "
        f"{', '.join(item['package'] for item in runs.values())}; "
        "not outbox_worker, CLI or entire Issue #94 owner topology"
    )
    report["limitations"] = [
        *report["limitations"],
        "A/B/C cold builds use independent target directories; warm builds reuse only their own target",
        "GNU time peak RSS is the maximum individual process resident set, not peak aggregate concurrent RAM",
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
