#!/usr/bin/env python3
"""Opt-in *actual rustc* A/B: Qdrant changes cannot relink generic Temporal.

This is an expensive, exact previously green SHA, disposable GitHub Actions
study. It is NEVER a mandatory PR job or a local source-edit experiment. Both
feature variants are compiled in ONE clean isolated Cargo target without
sccache/rustc wrappers; Qdrant V1 History remains explicitly replayable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from compare_physical_build_options import (
    ORCHESTRATOR, ROOT, RUST, EvidenceError, binary_snapshot,
    compiler_artifacts, generic_parent_default_feature_boundary,
    workspace_graph, cargo_manifest,
)
from measure_owner_rebuild import git_source_is_clean, probe_marker, verified_study_source


SOURCE = "app/rust/crates/qdrant_contracts/src/lib.rs"
DEFAULT_CMD = [
    "cargo", "build", "--locked", "--no-default-features", "-p", ORCHESTRATOR,
    "--bin", ORCHESTRATOR, "--message-format=json-render-diagnostics",
]
LEGACY_CMD = [
    "cargo", "build", "--locked", "--no-default-features", "-p", ORCHESTRATOR,
    "--features", "legacy-qdrant-v1", "--bin", ORCHESTRATOR,
    "--message-format=json-render-diagnostics",
]


def compiled_targets(result: dict) -> set[str]:
    return {unit["target_name"] for unit in result["non_fresh_units"]}


def assert_expected_rebuilds(
    generic_before: dict, generic_after: dict, legacy_after: dict,
    sha_before: str, sha_after: str,
) -> None:
    if "temporal_orchestrator" in compiled_targets(generic_after):
        raise EvidenceError("Qdrant-only source edit caused default generic Temporal rustc rebuild")
    if "qdrant_contracts" in compiled_targets(generic_after):
        raise EvidenceError("default Temporal unexpectedly compiled optional Qdrant contract")
    if sha_before != sha_after:
        raise EvidenceError("Qdrant-only source edit changed default Temporal binary bytes")
    if not {"qdrant_contracts", "temporal_orchestrator"} <= compiled_targets(legacy_after):
        raise EvidenceError("legacy-enabled Temporal did not actually rebuild after Qdrant edit")


def run_study() -> dict:
    source_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    verified_study_source(source_sha, os.environ)
    if not git_source_is_clean([SOURCE]):
        raise EvidenceError("refuse to edit a dirty Qdrant V1 contract source")
    graph, paths = workspace_graph()
    boundary = generic_parent_default_feature_boundary(graph, cargo_manifest(paths[ORCHESTRATOR]))
    if (not boundary["default_legacy_dependency_gated"]
            or boundary["legacy_qdrant_contracts_in_default_closure"]):
        raise EvidenceError("default provider-free feature boundary is absent")
    source = ROOT / SOURCE
    original = source.read_bytes()
    if not original.endswith(b"\n"):
        raise EvidenceError("Qdrant contract source lacks terminal newline")
    marker = probe_marker("qdrant_contracts", os.environ["GITHUB_RUN_ID"])
    report = {
        "schema_version": 1, "source_sha": source_sha,
        "source_git_tree_sha": subprocess.check_output(
            ["git", "rev-parse", "HEAD^{tree}"], cwd=ROOT, text=True,
        ).strip(),
        "baseline_source_sha256": hashlib.sha256(original).hexdigest(),
        "changed_source_sha256": hashlib.sha256(original + marker).hexdigest(),
        "feature_boundary": boundary,
        "rustc_evidence": "in_progress",
    }
    try:
        with tempfile.TemporaryDirectory(prefix="alegria-dag-parent-ab-") as directory:
            target = Path(directory) / "target"
            # Build legacy first, then generic: the latter must be the actual
            # output on disk immediately before source perturbation. Otherwise
            # Cargo switching features would induce a false relink.
            report["legacy_baseline"] = compiler_artifacts(
                LEGACY_CMD, RUST, isolated_target=True, target_dir=target,
            )
            report["generic_baseline"] = compiler_artifacts(
                DEFAULT_CMD, RUST, isolated_target=True, target_dir=target,
            )
            generic_before = binary_snapshot(report["generic_baseline"], (ORCHESTRATOR,))
            source.write_bytes(original + marker)
            report["generic_after_qdrant_edit"] = compiler_artifacts(
                DEFAULT_CMD, RUST, isolated_target=True, target_dir=target,
            )
            generic_after = binary_snapshot(report["generic_after_qdrant_edit"], (ORCHESTRATOR,))
            if generic_before[ORCHESTRATOR]["mtime_ns"] != generic_after[ORCHESTRATOR]["mtime_ns"]:
                raise EvidenceError("default Temporal binary was relinked after Qdrant edit")
            report["legacy_after_qdrant_edit"] = compiler_artifacts(
                LEGACY_CMD, RUST, isolated_target=True, target_dir=target,
            )
            assert_expected_rebuilds(
                report["generic_baseline"], report["generic_after_qdrant_edit"],
                report["legacy_after_qdrant_edit"],
                generic_before[ORCHESTRATOR]["sha256"], generic_after[ORCHESTRATOR]["sha256"],
            )
    finally:
        if source.read_bytes() != original:
            source.write_bytes(original)
        if not git_source_is_clean([SOURCE]):
            raise EvidenceError("Qdrant contract source was not restored byte-for-byte")
    report["rustc_evidence"] = "actual_wrapper_free_AB_pass"
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "automation/reports/generic_parent_rebuild_ab.json")
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        result = run_study()
    except (OSError, RuntimeError, ValueError) as error:
        args.output.write_text(json.dumps({
            "status": "FAIL", "error": str(error),
        }, indent=2) + "\n")
        print(f"GENERIC_PARENT_RUSTC_AB=FAIL {error}", file=sys.stderr)
        return 1
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print("GENERIC_PARENT_RUSTC_AB=PASS actual_wrapper_free_AB_pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
