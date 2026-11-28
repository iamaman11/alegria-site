#!/usr/bin/env python3
"""Measure a possible physical legacy Worker/Outbox build split on one exact SHA.

This is an optional, never-gating experiment. Each variant builds the identical
Git-archived immutable source in a DIFFERENT disposable Cargo target, with
rustc/sccache wrappers and incremental compilation disabled. It measures cold
compilation and actual executable artifacts; it cannot establish a warm GHA
cache improvement or justify removing either real E2E runtime dependency.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from compare_physical_build_options import (
    EvidenceError, binary_snapshot, compiler_artifacts, isolated_source_tree,
    verify_pr_source_provenance, workspace_graph,
)


VARIANTS = {
    "combined": ("temporal_worker", "outbox_worker"),
    "temporal_only": ("temporal_worker",),
    "outbox_only": ("outbox_worker",),
}


def transitive_workspace_closure(graph: dict[str, set[str]], package: str) -> set[str]:
    if package not in graph:
        raise EvidenceError(f"unknown immutable Cargo package {package}")
    visited: set[str] = set()
    pending = [package]
    while pending:
        current = pending.pop()
        if current not in visited:
            visited.add(current)
            pending.extend(graph[current] - visited)
    return visited


def build_command(packages: tuple[str, ...]) -> list[str]:
    if not packages or any(package not in VARIANTS["combined"] for package in packages):
        raise EvidenceError("worker probe must build known actual runtime package(s)")
    # The same source targets, features and profile as the mandatory CI build.
    # --timings produces auditable Cargo unit timing files, but never alters
    # which binaries or tests the accepted pipeline builds.
    return ["cargo", "build", "--locked", "--timings",
        "--message-format=json-render-diagnostics", *(flag
        for package in packages for flag in ("-p", package)),
        *(flag for package in packages for flag in ("--bin", package))]


def resolved_locked_runtime_graph(workspace: Path) -> dict:
    """Observe normal/build dependencies in the target-filtered locked Cargo graph.

    This is not a compiler-unit graph: Cargo metadata may conservatively
    unify workspace-wide features. The separate compiler-artifact stream below
    records which units actually participated in the measured invocations.
    """
    version = subprocess.run(
        ["rustc", "-vV"], cwd=workspace, capture_output=True, text=True,
    )
    targets = re.findall(r"^host: ([A-Za-z0-9_-]+)$", version.stdout, re.MULTILINE)
    if version.returncode or len(targets) != 1:
        raise EvidenceError("rustc host target unavailable for locked Worker dependency graph")
    metadata = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1",
         "--filter-platform", targets[0]],
        cwd=workspace, capture_output=True, text=True,
    )
    if metadata.returncode:
        raise EvidenceError(f"locked Cargo metadata failed: {metadata.stderr[-2000:]}")
    try:
        parsed = json.loads(metadata.stdout)
        package_map = {item["id"]: item for item in parsed["packages"]}
        workspace_ids = set(parsed["workspace_members"])
        nodes = {item["id"]: item for item in parsed["resolve"]["nodes"]}
    except (KeyError, TypeError, ValueError) as exc:
        raise EvidenceError("incomplete locked Cargo metadata result") from exc
    if len(nodes) != len(parsed["resolve"]["nodes"]) or not workspace_ids <= package_map.keys():
        raise EvidenceError("inconsistent locked Cargo dependency identities")
    if any(identifier not in workspace_ids and package["source"] is None
           for identifier, package in package_map.items()):
        raise EvidenceError("untracked path package in locked Worker dependency graph")
    roots = {
        package_map[identifier]["name"]: identifier for identifier in workspace_ids
    }
    if len(roots) != len(workspace_ids) or set(VARIANTS["combined"]) - roots.keys():
        raise EvidenceError("runtime package identities absent or ambiguous in Cargo metadata")

    def closure(package: str) -> set[str]:
        seen: set[str] = set()
        pending = [roots[package]]
        while pending:
            identifier = pending.pop()
            if identifier in seen:
                continue
            node = nodes.get(identifier)
            if node is None:
                raise EvidenceError(f"missing resolved Cargo node for {identifier}")
            seen.add(identifier)
            for dependency in node.get("deps", []):
                dep_id = dependency.get("pkg")
                kinds = dependency.get("dep_kinds")
                if dep_id not in package_map or not isinstance(kinds, list) or not kinds:
                    raise EvidenceError("incomplete Cargo dependency kind or identity")
                if any(isinstance(kind, dict) and kind.get("kind") in (None, "build")
                       for kind in kinds):
                    pending.append(dep_id)
        return seen

    temporal, outbox = (closure(package) for package in VARIANTS["combined"])
    overlap = temporal & outbox

    def package_names(ids: set[str]) -> list[str]:
        return sorted(package_map[identifier]["name"] for identifier in ids)

    def owner_names(ids: set[str]) -> list[str]:
        return package_names(ids & workspace_ids)

    return {
        "method": "cargo metadata --locked --filter-platform: resolved normal/build dependency edges",
        "host_target": targets[0],
        "feature_caveat": "Cargo metadata may conservatively unify default features across workspace members",
        "temporal_worker": {
            "workspace_packages": owner_names(temporal),
            "all_resolved_packages": package_names(temporal),
            "resolved_package_count": len(temporal),
        },
        "outbox_worker": {
            "workspace_packages": owner_names(outbox),
            "all_resolved_packages": package_names(outbox),
            "resolved_package_count": len(outbox),
        },
        "shared_resolved_packages": package_names(overlap),
        "shared_workspace_packages": owner_names(overlap),
        "shared_resolved_package_count": len(overlap),
    }


def summarize_build(build: dict, packages: tuple[str, ...]) -> dict:
    """Require real executable rustc units, inspect their bytes before target cleanup."""
    produced = {
        unit["target_name"] for unit in build["compiler_artifacts"]
        if "bin" in unit["kind"] and unit["executable"]
    }
    compiled = {
        unit["target_name"] for unit in build["non_fresh_units"]
        if "bin" in unit["kind"] and unit["executable"]
    }
    if set(packages) - produced:
        raise EvidenceError(f"Worker split omitted executable targets {sorted(set(packages) - produced)}")
    return {
        "actual_compiled_binary_targets": sorted(compiled),
        "actual_cargo_units": len(build["compiler_artifacts"]),
        "actual_non_fresh_units": len(build["non_fresh_units"]),
        "binary_snapshots": binary_snapshot(build, packages),
    }


def measure(source: Path, scratch: Path) -> dict:
    graph, _ = workspace_graph(source / "app" / "rust")
    temporal = transitive_workspace_closure(graph, "temporal_worker")
    outbox = transitive_workspace_closure(graph, "outbox_worker")
    # This hypothesis must be explicit; it is NOT proof that two Cargo build
    # jobs have smaller elapsed wall-clock time on GitHub-hosted CI runners.
    analysis = {
        "method": "conservative workspace path dependency closure (includes optional/target deps)",
        "temporal_worker_packages": sorted(temporal),
        "outbox_worker_packages": sorted(outbox),
        "overlap_packages": sorted(temporal & outbox),
        "combined_package_count": len(temporal | outbox),
    }
    resolved_graph = resolved_locked_runtime_graph(source / "app" / "rust")
    result = {}
    for variant, packages in VARIANTS.items():
        # Avoid target cache sharing: each variant receives a new empty target.
        # Each directory is deleted immediately after the measured build.
        with tempfile.TemporaryDirectory(prefix=f"alegria-{variant}-target-", dir=scratch) as temp:
            cold = compiler_artifacts(
                build_command(packages), source / "app" / "rust",
                isolated_target=True, target_dir=Path(temp) / "target",
                measure_resources=True,
            )
            cold_observed = summarize_build(cold, packages)
            if set(packages) - set(cold_observed["actual_compiled_binary_targets"]):
                raise EvidenceError(f"{variant} cold probe did not compile every real Worker binary")
            warm = compiler_artifacts(
                build_command(packages), source / "app" / "rust",
                isolated_target=True, target_dir=Path(temp) / "target",
                measure_resources=True,
            )
            warm_observed = summarize_build(warm, packages)
            if cold_observed["binary_snapshots"] != warm_observed["binary_snapshots"]:
                raise EvidenceError(f"{variant} unchanged warm source unexpectedly altered Worker executables")
        result[variant] = {
            "packages": list(packages),
            "cold_measurement": cold,
            "warm_measurement": warm,
            "cold_observed": cold_observed,
            "warm_observed": warm_observed,
        }
    return {"workspace_graph": analysis, "locked_resolved_cargo_graph": resolved_graph,
            "separate_cold_targets": result,
            "interpretation": (
                "Independent sequential cold/warm variants on one exact source SHA; "
                "these times are NOT PR READ_ONLY sccache data, parallel-job "
                "wall time, or proof of <=180-second full CI. Cargo metadata "
                "may overapproximate default feature unification. Both Workers "
                "remain mandatory in the live Temporal manual E2E."
            )}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    provenance = verify_pr_source_provenance()
    expected = os.environ.get("CI_SOURCE_SHA", "")
    if not expected or provenance["checkout_commit_sha"] != expected or not provenance["trees_equal"]:
        raise EvidenceError("worker split study requires exact verified canonical source SHA/tree")
    with tempfile.TemporaryDirectory(prefix="alegria-legacy-split-study-") as temp:
        source = Path(temp) / "source"
        source.mkdir()
        observed = isolated_source_tree(source)
        if observed != expected:
            raise EvidenceError("isolated worker study source SHA does not match green canonical SHA")
        result = measure(source, Path(temp))
    result["schema_version"] = 1
    result["source_sha"] = expected
    result["source_tree_sha"] = provenance["checkout_tree_sha"]
    result["source_provenance"] = provenance
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"LEGACY_WORKER_SPLIT_PHYSICAL=PASS sha={expected} report={args.output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (EvidenceError, OSError, ValueError) as error:
        raise SystemExit(f"LEGACY_WORKER_SPLIT_PHYSICAL=FAIL {error}") from error
