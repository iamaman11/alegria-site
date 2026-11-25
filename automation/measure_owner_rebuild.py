#!/usr/bin/env python3
"""Optional, fail-closed owner-only Cargo rebuild experiment in the existing PR job.

This command deliberately runs only on an ephemeral GitHub Actions checkout.
It never commits: every source perturbation is limited to one known Rust lib.rs
and is restored byte-for-byte in a finally block. Normal PR CI does not run it.

The probe measures actual Cargo compiler-artifact units and binary relinks, not
merely the reverse dependency closure inferred from Cargo.toml. These transient
source edits are architectural *diagnostics*, not accepted-SHA proof of a final
runtime topology; a selected architecture must be measured again on real SHAs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from typing import Any, Mapping


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
REPORT = ROOT / "automation" / "reports" / "owner_rebuild_probe.json"
OWNER_SOURCES = {
    "postgres_store": "app/rust/crates/postgres_store/src/lib.rs",
    "graph_neo4j": "app/rust/crates/graph_neo4j/src/lib.rs",
    "vector_qdrant": "app/rust/crates/vector_qdrant/src/lib.rs",
    # Measure the actual provider-free orchestration process, not the shared
    # Temporal client helper used by legacy/provider-linked consumers.
    "temporal_orchestrator": "app/rust/services/temporal_orchestrator/src/main.rs",
}
EXECUTABLE_PACKAGES = (
    "temporal_worker", "temporal_orchestrator", "temporal_pg_effects",
    "temporal_qdrant_effects", "outbox_worker", "cli_tools", "analytics_svc",
    "reconcile",
)
BUILD_COMMAND = (
    "cargo", "build", "--locked", "--bins", "--timings",
    *(flag for package in EXECUTABLE_PACKAGES for flag in ("-p", package)),
)
LATEST_REPORT: dict[str, Any] | None = None


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def checked_output(command: list[str], *, cwd: Path = ROOT) -> str:
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def owner_of_artifact(record: dict[str, Any]) -> str | None:
    target = record.get("target")
    if not isinstance(target, dict) or not isinstance(target.get("src_path"), str):
        return None
    source = Path(target["src_path"]).resolve()
    for owner, relative_path in OWNER_SOURCES.items():
        package_dir = (ROOT / relative_path).parent.parent
        if source.is_relative_to(package_dir):
            return owner
    return None


def compiler_unit(record: dict[str, Any]) -> dict[str, Any] | None:
    """Retain enough information to disambiguate real rustc and binary units."""
    if record.get("reason") != "compiler-artifact":
        return None
    target = record.get("target")
    if not isinstance(target, dict) or not isinstance(target.get("name"), str):
        return None
    if not isinstance(record.get("package_id"), str) or not isinstance(record.get("fresh"), bool):
        return None
    if not isinstance(target.get("kind"), list) or not isinstance(target.get("src_path"), str):
        return None
    profile = record.get("profile")
    if not isinstance(profile, dict):
        return None
    executable = record.get("executable")
    return {
        "package_id": record["package_id"],
        "name": target["name"],
        "kind": target["kind"],
        "src_path": target["src_path"],
        "profile": profile,
        "fresh": record["fresh"],
        "owner": owner_of_artifact(record),
        "executable": executable if isinstance(executable, str) else None,
    }


def target_dir() -> Path:
    configured = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    return configured if configured.is_absolute() else WORKSPACE / configured


def timing_snapshot() -> dict[str, tuple[int, int]]:
    folder = target_dir() / "cargo-timings"
    return {
        path.name: (path.stat().st_mtime_ns, path.stat().st_size)
        for path in folder.glob("cargo-timing*.html") if path.is_file()
    }


def changed_timings(before: dict[str, tuple[int, int]]) -> list[dict[str, Any]]:
    folder = target_dir() / "cargo-timings"
    result = []
    for name, metadata in sorted(timing_snapshot().items()):
        if before.get(name) != metadata:
            path = folder / name
            result.append({"path": str(path.relative_to(ROOT)), "sha256": sha256_file(path)})
    return result


def binary_snapshot(units: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    result = {}
    for unit in units:
        if "bin" not in unit["kind"] or not unit["executable"]:
            continue
        path = Path(unit["executable"])
        if not path.is_file():
            raise RuntimeError(f"Cargo emitted missing executable: {path}")
        stat = path.stat()
        key = f"{unit['package_id']}::{unit['name']}"
        result[key] = {
            "path": str(path), "sha256": sha256_file(path),
            "mtime_ns": stat.st_mtime_ns, "size": stat.st_size,
        }
    return result


def sccache_stats() -> dict[str, Any]:
    """Require machine-readable compiler-cache evidence for each probe."""
    result = subprocess.run(
        ["sccache", "--show-stats", "--stats-format", "json"],
        cwd=WORKSPACE, text=True, capture_output=True, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"sccache JSON statistics unavailable: {result.stderr.strip()}")
    try:
        stats = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("sccache returned non-JSON statistics") from error
    if not isinstance(stats, dict) or not stats:
        raise RuntimeError("sccache statistics are empty")
    return stats


def numeric_stats_delta(before: Any, after: Any, prefix: str = "") -> dict[str, int]:
    """Preserve cache counter deltas without assuming an sccache JSON revision."""
    if isinstance(before, dict) and isinstance(after, dict):
        result: dict[str, int] = {}
        for key in before.keys() & after.keys():
            result.update(numeric_stats_delta(before[key], after[key], f"{prefix}.{key}" if prefix else str(key)))
        return result
    if type(before) is int and type(after) is int:
        return {prefix: after - before}
    return {}


def run_cargo(label: str) -> dict[str, Any]:
    """Execute the same locked binary build and capture unaggregated rustc units."""
    before_timings = timing_snapshot()
    started = time.monotonic()
    command = [*BUILD_COMMAND[:2], "--message-format=json-render-diagnostics", *BUILD_COMMAND[2:]]
    print(f"OWNER_BUILD_PROBE={label} command={' '.join(command)}", flush=True)
    process = subprocess.Popen(
        command, cwd=WORKSPACE, stdout=subprocess.PIPE, stderr=None,
        text=True, errors="replace", bufsize=1,
    )
    assert process.stdout is not None
    units = []
    finished = False
    for line in process.stdout:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            print(line, end="", flush=True)
            continue
        if not isinstance(record, dict):
            continue
        unit = compiler_unit(record)
        if unit is not None:
            units.append(unit)
        elif record.get("reason") == "compiler-message":
            message = record.get("message")
            if isinstance(message, dict) and isinstance(message.get("rendered"), str):
                print(message["rendered"], end="", flush=True)
        elif record.get("reason") == "build-finished":
            finished = bool(record.get("success"))
    exit_code = process.wait()
    elapsed = round(time.monotonic() - started, 3)
    if exit_code or not finished or not units:
        raise RuntimeError(f"{label}: cargo failed or gave incomplete artifact evidence (exit={exit_code})")
    binaries = binary_snapshot(units)
    # All selected runtime packages must produce real binaries, not merely an
    # unrelated library build or a synthetic no-op confirmation.
    package_bins = set()
    for unit in units:
        if "bin" not in unit["kind"] or not unit["executable"]:
            continue
        path = Path(unit["src_path"]).resolve()
        for package in EXECUTABLE_PACKAGES:
            package_dir = WORKSPACE / "services" / ("temporal" if package == "temporal_worker" else package)
            if path.is_relative_to(package_dir):
                package_bins.add(package)
    if package_bins != set(EXECUTABLE_PACKAGES):
        raise RuntimeError(f"{label}: missing binaries for {sorted(set(EXECUTABLE_PACKAGES) - package_bins)}")
    timing_files = changed_timings(before_timings)
    if not timing_files:
        raise RuntimeError(f"{label}: cargo --timings produced no updated HTML evidence")
    return {
        "label": label, "command": command, "elapsed_seconds": elapsed,
        "compiler_units": units, "rebuilt_units": [unit for unit in units if not unit["fresh"]],
        "fresh_units": [unit for unit in units if unit["fresh"]],
        "binaries": binaries, "timing_files": timing_files,
        "sccache": sccache_stats(),
    }


def probe_marker(owner: str, run_id: str) -> bytes:
    identifier = re.sub(r"[^A-Z0-9_]", "_", owner.upper())
    nonce = re.sub(r"[^A-Za-z0-9_-]", "_", run_id)
    return f'\n#[allow(dead_code)]\nconst OWNER_BUILD_PROBE_{identifier}: &str = "{nonce}_{owner}";\n'.encode()


def git_source_is_clean(paths: list[str]) -> bool:
    # Compare against HEAD, not just the index, so staged edits are protected.
    result = subprocess.run(["git", "diff", "--quiet", "HEAD", "--", *paths], cwd=ROOT, check=False)
    return result.returncode == 0


def verified_study_source(commit_sha: str, environment: Mapping[str, str]) -> str:
    """Only explicitly dispatched exact-head studies may perturb local source.

    GitHub's workflow_dispatch GITHUB_SHA can be the moving branch ref; the
    caller selects an immutable previously green PR SHA independently. Never
    substitute a synthetic PR merge or an unverified branch tip for that SHA.
    """
    if (environment.get("GITHUB_ACTIONS") != "true"
            or environment.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
            or not environment.get("GITHUB_RUN_ID")):
        raise RuntimeError("owner rebuild study requires an explicit GitHub Actions dispatch")
    expected = environment.get("CI_SOURCE_SHA", "")
    if not re.fullmatch(r"[a-f0-9]{40}", expected) or commit_sha != expected:
        raise RuntimeError("owner rebuild study source differs from exact selected PR SHA")
    return expected


def run_study() -> dict[str, Any]:
    global LATEST_REPORT
    commit_sha = checked_output(["git", "rev-parse", "HEAD"])
    verified_study_source(commit_sha, os.environ)
    paths = list(OWNER_SOURCES.values())
    if not git_source_is_clean(paths):
        raise RuntimeError("owner probe refuses to modify already-dirty provider sources")
    originals = {owner: (ROOT / path).read_bytes() for owner, path in OWNER_SOURCES.items()}
    if any(not body.endswith(b"\n") for body in originals.values()):
        raise RuntimeError("owner source lacks terminal newline; refusing marker append")
    report: dict[str, Any] = {
        "schema_version": 1,
        "status": "IN_PROGRESS",
        "source_sha": commit_sha,
        "pr_head_sha": os.environ.get("CI_SOURCE_SHA"),
        "git_tree_sha": checked_output(["git", "rev-parse", "HEAD^{tree}"]),
        "experiment": "optional-owner-rebuild-A-baseline",
        "toolchain": {
            "cargo": checked_output(["cargo", "--version"], cwd=WORKSPACE),
            "rustc": checked_output(["rustc", "--version"], cwd=WORKSPACE),
        },
        "build_env": {key: os.environ.get(key) for key in (
            "CARGO_INCREMENTAL", "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_TEST_DEBUG",
            "CARGO_TARGET_DIR", "RUSTC_WRAPPER", "SCCACHE_GHA_RW_MODE", "SQLX_OFFLINE",
        )},
        "owner_sources": {owner: {"path": path, "sha256": sha256(originals[owner])}
                          for owner, path in OWNER_SOURCES.items()},
        "probes": [],
    }
    LATEST_REPORT = report
    try:
        baseline_cache_before = sccache_stats()
        report["baseline"] = run_cargo("baseline")
        report["baseline"]["sccache_delta"] = numeric_stats_delta(
            baseline_cache_before, report["baseline"]["sccache"]
        )
        baseline_binaries = report["baseline"]["binaries"]
        cache_before = report["baseline"]["sccache"]
        for owner, relative_path in OWNER_SOURCES.items():
            file = ROOT / relative_path
            changed = originals[owner] + probe_marker(owner, os.environ["GITHUB_RUN_ID"])
            file.write_bytes(changed)
            try:
                result = run_cargo(owner)
                if not any(unit["owner"] == owner for unit in result["rebuilt_units"]):
                    raise RuntimeError(f"{owner}: Cargo did not rebuild its changed owner library")
                binaries = result["binaries"]
                relinked = sorted({
                    f"{unit['package_id']}::{unit['name']}"
                    for unit in result["rebuilt_units"] if "bin" in unit["kind"]
                })
                written_binaries = sorted(
                    key for key, snapshot in binaries.items()
                    if key not in baseline_binaries
                    or snapshot["mtime_ns"] != baseline_binaries[key]["mtime_ns"]
                )
                changed_binary_hashes = sorted(
                    key for key, snapshot in binaries.items()
                    if key not in baseline_binaries or snapshot["sha256"] != baseline_binaries[key]["sha256"]
                )
                report["probes"].append({
                    "owner": owner, "transient_source_sha256": sha256(changed),
                    "build": result,
                    "cargo_rebuilt_bin_targets": relinked,
                    "relinked_binaries": sorted(set(relinked) & set(written_binaries)),
                    "binary_files_written": written_binaries,
                    "changed_binary_hashes_vs_baseline": changed_binary_hashes,
                    "sccache_delta": numeric_stats_delta(cache_before, result["sccache"]),
                })
            finally:
                file.write_bytes(originals[owner])
            if not git_source_is_clean(paths):
                raise RuntimeError(f"{owner}: source restoration failed")
            # Re-establish each identical baseline at the same workspace path;
            # otherwise edits to earlier owners contaminate later experiments.
            report["probes"][-1]["baseline_restore"] = run_cargo(f"restore-{owner}")
            baseline_binaries = report["probes"][-1]["baseline_restore"]["binaries"]
            cache_before = report["probes"][-1]["baseline_restore"]["sccache"]
    finally:
        for owner, relative_path in OWNER_SOURCES.items():
            file = ROOT / relative_path
            if file.read_bytes() != originals[owner]:
                file.write_bytes(originals[owner])
        if not git_source_is_clean(paths):
            raise RuntimeError("source bytes were not restored; owner rebuild experiment invalid")
    if len(report["probes"]) != len(OWNER_SOURCES):
        raise RuntimeError("incomplete four-owner rebuild experiment")
    # A successful probe proves evidence capture, not architectural isolation.
    # The baseline may legitimately show unwanted owner-to-binary rebuilds.
    report["status"] = "EVIDENCE_CAPTURED"
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, default=REPORT)
    args = parser.parse_args()
    args.report.parent.mkdir(parents=True, exist_ok=True)
    try:
        result = run_study()
    except Exception as error:
        evidence = dict(LATEST_REPORT or {})
        evidence.update({
            "status": "FAIL", "error_class": type(error).__name__,
            "error": str(error), "completed_probes": len((LATEST_REPORT or {}).get("probes", [])),
            "expected_probes": len(OWNER_SOURCES),
        })
        args.report.write_text(json.dumps(evidence, indent=2, sort_keys=True), encoding="utf-8")
        print(f"OWNER_BUILD_PROBE=FAIL reason={type(error).__name__}: {error}", file=sys.stderr, flush=True)
        return 1
    args.report.write_text(json.dumps(result, indent=2, sort_keys=True), encoding="utf-8")
    print(f"OWNER_BUILD_PROBE=EVIDENCE_CAPTURED owners={len(result['probes'])} report={args.report}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
