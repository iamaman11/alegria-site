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
import tempfile

from compare_physical_build_options import (
    EvidenceError, compiler_artifacts, isolated_source_tree,
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
    return ["cargo", "build", "--locked", "--timings", *(flag
        for package in packages for flag in ("-p", package)),
        *(flag for package in packages for flag in ("--bin", package))]


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
    result = {}
    for variant, packages in VARIANTS.items():
        # Avoid target cache sharing: each variant receives a new empty target.
        # Each directory is deleted immediately after the measured build.
        with tempfile.TemporaryDirectory(prefix=f"alegria-{variant}-target-", dir=scratch) as temp:
            probe = compiler_artifacts(
                build_command(packages), source / "app" / "rust",
                isolated_target=True, target_dir=Path(temp) / "target",
                measure_resources=True,
            )
        produced = {
            unit["target_name"] for unit in probe["compiler_artifacts"]
            if "bin" in unit["kind"] and unit["executable"]
        }
        if not set(packages).issubset(produced):
            raise EvidenceError(f"{variant} did not compile its real Worker binaries: {produced}")
        result[variant] = {
            "packages": list(packages),
            "measurement": probe,
            "observed_executable_names": sorted(produced),
        }
    return {"workspace_graph": analysis, "separate_cold_targets": result,
            "interpretation": (
                "Independent sequential cold variants on one exact source SHA; "
                "these times are NOT PR READ_ONLY sccache data, parallel-job "
                "wall time, or proof of <=180-second full CI. Both Workers "
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
