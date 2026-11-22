#!/usr/bin/env python3
"""Deterministic, fail-closed Rust change-impact report for the canonical workspace.

This report selects affected native Cargo packages and the relevant CI suites.
Unknown inputs expand to the full workspace; known support-only files do not
pretend to have modified every Rust crate.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from collections import deque
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
SCHEMA_VERSION = 2

# Includes changes to generated contracts, integration fixtures, build tooling,
# container contents and runtime configuration; they are NOT Rust crate-local.
GLOBAL_PREFIXES = (
    "app/db/",
    "contracts/",
    "analytics_lab/",
)
SUPPORT_ONLY_PREFIXES = (
    ".github/",
    "docs/",
    "infra/local/",
    "infra/cloudflare/",
    "automation/",
)
SUPPORT_ONLY_FILES = {"README.md", "docker-compose.yml", ".gitignore"}
# A changed test driver must still exercise the suites it directly controls.
DEEP_DRIVER_FILES = {
    "automation/ci_verify.sh",
    "automation/run_truth_certification_gate.sh",
    "automation/run_truth_prepare.sh",
    "automation/run_qdrant_transport_gate.sh",
    "automation/run_whole_page_semantic_gate.sh",
    "automation/smoke_manual_llm_queue.py",
}
# Core runtime changes must prove real external-service behavior before release.
DEEP_PACKAGES = {
    "temporal_worker", "outbox_worker", "integration_harness", "infrastructure",
    "seo_steps", "seo_application", "vector_qdrant", "graph_neo4j",
}
GLOBAL_FILES = {
    "app/rust/Cargo.toml",
    "app/rust/Cargo.lock",
    "app/rust/rust-toolchain.toml",
    "rust-toolchain.toml",
}


def parse_name_status(raw: bytes) -> list[str]:
    """Decode a NUL-delimited Git diff, keeping both rename paths and deletes."""
    fields = raw.decode("utf-8", errors="surrogateescape").split("\0")
    paths: set[str] = set()
    i = 0
    while i < len(fields):
        status = fields[i]
        i += 1
        if not status:
            continue
        if status[0] not in "ACDMRTUXB":
            raise ValueError(f"unsupported git diff status: {status!r}")
        count = 2 if status[0] in "RC" else 1
        if i + count > len(fields):
            raise ValueError("incomplete NUL-delimited git diff")
        for path in fields[i : i + count]:
            if not path or path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe changed path: {path!r}")
            paths.add(path)
        i += count
    return sorted(paths)


def changed_paths(base: str, head: str, *, root: Path = ROOT) -> list[str]:
    """Return BOTH names in renames/copies, including deleted paths."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, head, "--"],
        cwd=root,
    )
    return parse_name_status(raw)


def changed_worktree_paths(base: str, *, root: Path = ROOT) -> list[str]:
    """Include staged, unstaged, renamed, deleted and not-yet-tracked local files."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, "--"],
        cwd=root,
    )
    tracked = set(parse_name_status(raw))
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z", "--"],
        cwd=root,
    )
    for path in untracked.decode("utf-8", errors="surrogateescape").split("\0"):
        if path:
            if path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe untracked path: {path!r}")
            tracked.add(path)
    return sorted(tracked)


def load_cargo_metadata(*, workspace: Path = WORKSPACE) -> dict[str, Any]:
    output = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--no-deps",
            "--manifest-path",
            str(workspace / "Cargo.toml"),
        ],
        cwd=workspace,
        text=True,
    )
    return json.loads(output)


def workspace_graph(
    metadata: dict[str, Any], *, root: Path = ROOT
) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Map local manifest directories to package names and reverse edges.

    Conservatively include normal, build, dev and optional edges. A workspace
    feature can enable a dependency later; omitting it would be a false-negative.
    """
    members = set(metadata["workspace_members"])
    packages = [pkg for pkg in metadata["packages"] if pkg["id"] in members]
    if not packages or len(packages) != len(members):
        raise ValueError("incomplete cargo metadata workspace member set")

    manifest_owner: dict[str, str] = {}
    by_absolute_manifest: dict[Path, str] = {}
    for pkg in packages:
        manifest = Path(pkg["manifest_path"]).resolve()
        directory = manifest.parent.relative_to(root.resolve()).as_posix()
        if pkg["name"] in manifest_owner.values() or directory in manifest_owner:
            raise ValueError("ambiguous workspace package name or directory")
        manifest_owner[directory] = pkg["name"]
        by_absolute_manifest[manifest] = pkg["name"]

    reverse = {name: set() for name in manifest_owner.values()}
    for pkg in packages:
        for dep in pkg["dependencies"]:
            if not dep.get("path"):
                continue
            dependent = by_absolute_manifest.get(
                (Path(dep["path"]) / "Cargo.toml").resolve()
            )
            if dependent is not None:
                reverse[dependent].add(pkg["name"])
    return manifest_owner, reverse


def analyze(
    paths: list[str],
    owners: dict[str, str],
    reverse: dict[str, set[str]],
    *,
    head: str,
    base: str,
) -> dict[str, Any]:
    """Build an auditable, no-false-negative conservative affected closure."""
    reason: dict[str, set[str]] = {name: set() for name in reverse}
    direct: set[str] = set()
    broad_reasons: list[str] = []

    for path in sorted(set(paths)):
        if path in GLOBAL_FILES or path in DEEP_DRIVER_FILES or path.startswith(GLOBAL_PREFIXES):
            broad_reasons.append(f"global input: {path}")
            continue
        if path in SUPPORT_ONLY_FILES or path.startswith(SUPPORT_ONLY_PREFIXES):
            continue
        matches = [(prefix, name) for prefix, name in owners.items() if path.startswith(prefix + "/")]
        if len(matches) != 1:
            broad_reasons.append(f"unmapped or ambiguous input: {path}")
            continue
        _, name = matches[0]
        direct.add(name)
        reason[name].add(f"changed: {path}")

    if broad_reasons:
        affected = set(reverse)
        for name in affected:
            reason[name].add("fail-closed: common/unmapped input")
    else:
        affected = set(direct)
        queue = deque(sorted(direct))
        while queue:
            dependency = queue.popleft()
            for dependent in sorted(reverse[dependency]):
                reason[dependent].add(f"depends on: {dependency}")
                if dependent not in affected:
                    affected.add(dependent)
                    queue.append(dependent)

    services = sorted(
        name for directory, name in owners.items()
        if directory.startswith("app/rust/services/") and name in affected
    )
    deep_required = bool(broad_reasons or DEEP_PACKAGES.intersection(affected))
    return {
        "schema_version": SCHEMA_VERSION,
        "base": base,
        "source_sha": head,
        "changed_paths": sorted(set(paths)),
        "analysis": "full" if broad_reasons else "scoped",
        "full_scope_reasons": sorted(broad_reasons),
        "direct_crates": sorted(direct),
        "affected_crates": sorted(affected),
        "affected_services": services,
        "deep_required": deep_required,
        "reasons": {pkg: sorted(reason[pkg]) for pkg in sorted(affected)},
        # The exact-source CI gate always remains mandatory, even when all
        # expensive runtime suites are irrelevant to the changed files.
        "mandatory_full_ci_gate": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Fetched PR base commit SHA")
    parser.add_argument("--head", required=True, help="Exact CI commit SHA")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()

    metadata = load_cargo_metadata()
    owners, reverse = workspace_graph(metadata)
    paths = changed_paths(args.base, args.head)
    report = analyze(paths, owners, reverse, base=args.base, head=args.head)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        f"RUST_IMPACT={report['analysis']} "
        f"direct={','.join(report['direct_crates']) or '-'} "
        f"affected={','.join(report['affected_crates']) or '-'} "
        f"deep_required={str(report['deep_required']).lower()} "
        f"report={args.report}"
    )
    import os
    if os.environ.get("GITHUB_OUTPUT"):
        # Package names are validated by Cargo metadata, never by PR text.
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as outputs:
            outputs.write(f"deep_required={str(report['deep_required']).lower()}\n")
            outputs.write(f"affected_packages={json.dumps(report['affected_crates'])}\n")
    for why in report["full_scope_reasons"]:
        print(f"  FULL_SCOPE: {why}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
