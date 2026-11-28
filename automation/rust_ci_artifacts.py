#!/usr/bin/env python3
"""Exact-source, content-verified Linux Rust executables for one CI workflow.

The compiler runs once per executable owner. E2E jobs install its immutable
binary artifact and execute exactly the suites chosen by ``ci_impact.py``.
Artifacts are a transport, never an alternative source of executable authority.
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
import time

from ci_impact import ROOT, WORKSPACE


FORMAT = "alegria-rust-ci-artifact@1"
KINDS = {
    "harness": ("integration_harness_test",),
    "pg_harness": ("integration_pg_v3_e2e_test",),
    "outbox_harness": ("integration_outbox_e2e_test",),
    # Each physically separate E2E owner must transport a distinct compiled
    # executable. Never alias these to the old monolithic harness artifact.
    "graph_harness": ("integration_graph_e2e_test",),
    "qdrant_harness": ("integration_qdrant_e2e_test",),
    "qdrant_temporal_harness": ("integration_qdrant_temporal_e2e_test",),
    "temporal_harness": ("integration_temporal_e2e_test",),
    "pg_app_harness": ("integration_pg_app_e2e_test",),
    "legacy": ("temporal_worker", "outbox_worker"),
    "v3": ("temporal_orchestrator", "temporal_pg_effects"),
    # The historical Qdrant V1 parent is a separately built *feature-enabled*
    # Temporal binary. Keep its exact-source executable physically distinct
    # from the provider-free default generic DAG parent, while preserving the
    # frozen Workflow Type and replay logic inside the legacy feature.
    "legacy_qdrant_parent": ("temporal_orchestrator_legacy_qdrant_v1",),
    "qdrant_effects": ("temporal_qdrant_effects",),
    # Separately attested opt-in stub-only DataForSEO worker executable.
    # Never alias provider code to the provider-neutral parent bundle.
    "dataforseo_effects": ("temporal_dataforseo",),
    # Two independently privileged Acquisition executables built once in the
    # existing architecture owner job: pure Temporal Child and source-only SQL
    # issuer. Each filename is pinned and hash-verified separately at install.
    "acquisition_pure": ("temporal_acquisition_pure", "acquisition_source_issuer"),
    # Independently built native pure owners. Each has its own strictly
    # content-verified artifact, Task Queue and trusted V3 route, never a
    # synthetic path through the generic parent binary.
    "seo_steps_pure": ("temporal_seo_steps_pure",),
    "truth_pure": ("temporal_truth_pure",),
    "one_step_request_issuer": ("one_step_request_issuer",),
}
SHA = re.compile(r"[a-f0-9]{40}\Z")
TARGET = WORKSPACE / "target" / "debug"


def assert_source_sha(source_sha: str) -> None:
    if not SHA.fullmatch(source_sha):
        raise ValueError("invalid exact source commit SHA")
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if actual != source_sha:
        raise ValueError(f"artifact source mismatch: checked-out {actual} != {source_sha}")


def digest(path: Path) -> tuple[str, int]:
    hash_ = hashlib.sha256()
    length = 0
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            hash_.update(data)
            length += len(data)
    return hash_.hexdigest(), length


def source_binaries(kind: str, *, harness: Path | None = None) -> dict[str, Path]:
    if kind not in KINDS:
        raise ValueError(f"unknown Rust CI artifact kind {kind}")
    if kind in ("harness", "pg_harness", "outbox_harness", "graph_harness", "qdrant_harness",
                "qdrant_temporal_harness", "temporal_harness", "pg_app_harness"):
        if harness is None:
            raise ValueError("the harness bundle needs the compiler-reported test executable")
        return {KINDS[kind][0]: harness.resolve()}
    if harness is not None:
        raise ValueError("runtime bundles cannot disguise arbitrary harness executables")
    if kind == "legacy_qdrant_parent":
        # This kind is only packaged immediately after the explicit
        # `cargo build --features legacy-qdrant-v1` command. Cargo always
        # calls the produced executable `temporal_orchestrator`; the bundle
        # gives it a distinct immutable, content-verified installed name.
        return {KINDS[kind][0]: TARGET / "temporal_orchestrator"}
    return {name: TARGET / name for name in KINDS[kind]}


def package(kind: str, source_sha: str, output: Path, *, harness: Path | None = None) -> dict:
    began = time.monotonic()
    assert_source_sha(source_sha)
    if output.exists() and any(output.iterdir()):
        raise ValueError("refuse to overwrite an existing CI artifact bundle")
    output.mkdir(parents=True, exist_ok=True)
    binaries = source_binaries(kind, harness=harness)
    metadata: dict[str, dict[str, str | int]] = {}
    for name, source in binaries.items():
        if not source.is_file() or source.is_symlink() or not os.access(source, os.X_OK):
            raise ValueError(f"missing real executable for {name}: {source}")
        destination = output / name
        shutil.copyfile(source, destination)
        # The bundle is also executed in the producer CI job to prove the
        # provider-free and legacy feature modes. copyfile does not preserve
        # executable mode; the exact-source binary must remain executable
        # before any later install step (which also grants 0755).
        destination.chmod(0o755)
        sha256, size = digest(destination)
        if not size:
            raise ValueError(f"empty compiled executable {name}")
        metadata[name] = {"sha256": sha256, "size": size}
    total_bytes = sum(entry["size"] for entry in metadata.values())
    manifest = {"format": FORMAT, "source_sha": source_sha, "kind": kind,
                "total_bytes": total_bytes, "files": metadata}
    (output / "manifest.json").write_text(json.dumps(manifest, sort_keys=True) + "\n", encoding="utf-8")
    print(f"RUST_CI_ARTIFACT_PACK=PASS kind={kind} source_sha={source_sha} "
          f"binaries={len(metadata)} bytes={total_bytes} "
          f"elapsed_seconds={time.monotonic()-began:.3f}", flush=True)
    return manifest


def install(kind: str, source_sha: str, bundle: Path, *, target: Path = TARGET) -> dict:
    began = time.monotonic()
    assert_source_sha(source_sha)
    if kind not in KINDS or not bundle.is_dir() or bundle.is_symlink():
        raise ValueError("invalid Rust CI artifact owner or bundle directory")
    manifest_file = bundle / "manifest.json"
    if not manifest_file.is_file() or manifest_file.is_symlink():
        raise ValueError("missing trusted Rust CI artifact manifest")
    manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    if (not isinstance(manifest, dict) or manifest.get("format") != FORMAT
            or manifest.get("source_sha") != source_sha or manifest.get("kind") != kind
            or set(manifest.get("files", {})) != set(KINDS[kind])
            or set(path.name for path in bundle.iterdir()) != {"manifest.json", *KINDS[kind]}):
        raise ValueError("artifact manifest does not match selected exact-source owner")
    try:
        total_bytes = sum(entry["size"] for entry in manifest["files"].values())
    except (TypeError, KeyError) as error:
        raise ValueError("invalid byte counts in artifact manifest") from error
    if manifest.get("total_bytes") != total_bytes:
        raise ValueError("artifact total byte count mismatch")
    # All paths come from the typed owner map, never from artifact JSON.
    target.mkdir(parents=True, exist_ok=True)
    for name in KINDS[kind]:
        entry = manifest["files"][name]
        source = bundle / name
        if source.is_symlink() or not source.is_file() or not isinstance(entry, dict):
            raise ValueError(f"invalid executable artifact for {name}")
        sha256, size = digest(source)
        if (entry != {"sha256": sha256, "size": size}
                or sha256 == "" or size == 0):
            raise ValueError(f"artifact digest or size mismatch for {name}")
        exact_tests = {
            "harness": "deps/integration_harness-ci-exact",
            "pg_harness": "deps/integration_pg_v3_e2e-ci-exact",
            "outbox_harness": "deps/integration_outbox_e2e-ci-exact",
            "graph_harness": "deps/integration_graph_e2e-ci-exact",
            "qdrant_harness": "deps/integration_qdrant_e2e-ci-exact",
            "qdrant_temporal_harness": "deps/integration_qdrant_temporal_e2e-ci-exact",
            "temporal_harness": "deps/integration_temporal_e2e-ci-exact",
            "pg_app_harness": "deps/integration_pg_app_e2e-ci-exact",
        }
        destination = target / exact_tests.get(kind, name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists() or destination.is_symlink():
            raise ValueError(f"refuse to overwrite a pre-existing CI executable: {destination}")
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
        if digest(destination) != (sha256, size):
            raise ValueError(f"installed Rust CI executable changed during copy: {name}")
    print(f"RUST_CI_ARTIFACT_INSTALL=PASS kind={kind} source_sha={source_sha} "
          f"bytes={total_bytes} elapsed_seconds={time.monotonic()-began:.3f}", flush=True)
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("pack", "install"))
    parser.add_argument("--kind", choices=tuple(KINDS), required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--harness", type=Path)
    args = parser.parse_args(argv)
    if args.operation == "pack":
        package(args.kind, args.source_sha, args.directory, harness=args.harness)
    else:
        if args.harness:
            raise ValueError("install cannot override the compiler-owned executable path")
        install(args.kind, args.source_sha, args.directory)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"RUST_CI_ARTIFACT=FAIL {error}", file=sys.stderr)
        raise SystemExit(1) from error
