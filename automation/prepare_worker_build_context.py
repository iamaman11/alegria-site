#!/usr/bin/env python3
"""Prepare a dependency-scoped Docker context for the accepted Temporal image.

Cargo still resolves the original workspace manifests and Cargo.lock. A crate
outside the actual Worker/CLI dependency closure gets a metadata-only inert
target, so editing that crate's Rust source cannot invalidate the release
compilation layer. The owner registry comes exclusively from ci_impact.py.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tomllib
from collections import deque
from pathlib import Path

from ci_impact import (
    IMAGE_OWNERS, ROOT, WORKER_NONDELIVERY_FILES, WORKER_PACKAGING_FILES,
    load_cargo_metadata, workspace_graph,
)


SKELETON_ROOT = "dependency-skeleton"


def dependency_skeleton_targets(
    metadata: dict, owners: dict[str, str], release: set[str], *, root: Path,
) -> dict[str, str]:
    """Map actual Cargo library/binary entrypoints to inert dependency stubs.

    Cargo metadata, not a second hand-maintained package list, owns target
    discovery. The first Docker layer retains the original locked manifests,
    but has neither production Rust bodies nor active workspace build scripts.
    """
    packages = {package["name"]: package for package in metadata["packages"]}
    if not release <= packages.keys():
        raise ValueError("dependency skeleton is missing release packages")
    targets: dict[str, str] = {}
    for directory, package_name in sorted(owners.items()):
        if package_name not in release:
            continue
        found = False
        for target in packages[package_name]["targets"]:
            kind = set(target["kind"])
            if kind & {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}:
                content = "// Cargo dependency skeleton: real implementation copied in the next layer.\n"
            elif kind & {"bin", "custom-build"}:
                content = "fn main() {}\n"
            else:
                # Examples, benches and tests are not part of cargo build.
                continue
            target_source = Path(target["src_path"]).resolve()
            try:
                relpath = target_source.relative_to(root.resolve()).as_posix()
            except ValueError as exc:
                raise ValueError(f"Cargo target escaped the repository: {target_source}") from exc
            if not relpath.startswith(directory + "/"):
                raise ValueError(f"Cargo target escaped its owning crate: {relpath}")
            if relpath in targets:
                raise ValueError(f"duplicate Cargo target: {relpath}")
            targets[relpath] = content
            found = True
        if not found:
            raise ValueError(f"release crate has no buildable lib/bin: {package_name}")
    return targets


def write_dependency_skeleton(
    destination: Path, *, metadata: dict, owners: dict[str, str],
    release: set[str], root: Path,
) -> None:
    """Persist dependency objects as an ordinary Buildx-exportable Docker layer.

    BuildKit cache mounts are not included in `cache-to: type=gha`; a native
    Cargo compile against stable stub targets gives real reusable .rlib files
    in the layer instead. It is rebuilt when Cargo manifests/lock change, but
    not when Rust functions, protobuf schemas or workspace build.rs change.
    """
    actual = destination / "app"
    skeleton = destination / SKELETON_ROOT / "app"
    # Keep only exact Cargo resolution inputs. Dummy workspace build.rs files
    # compile their *declared build dependencies* without generating outputs;
    # the real build scripts/protos run after the second COPY of actual source.
    for source in sorted(actual.rglob("*")):
        if not source.is_file():
            continue
        relpath = source.relative_to(destination).as_posix()
        if relpath.startswith("app/rust/") and source.name in {"Cargo.toml", "Cargo.lock"}:
            target = skeleton / source.relative_to(actual)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)

    stubs = dependency_skeleton_targets(metadata, owners, release, root=root)
    for relpath, content in sorted(stubs.items()):
        if not (destination / relpath).is_file():
            raise ValueError(f"Cargo entrypoint missing from release context: {relpath}")
        target = destination / SKELETON_ROOT / relpath
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
    for directory, package_name in owners.items():
        if package_name not in release:
            # Preserve the same inert non-consumer placeholder as the actual
            # release context, to maintain workspace resolution parity.
            stub = destination / directory / "src/lib.rs"
            target = destination / SKELETON_ROOT / directory / "src/lib.rs"
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(stub, target)


def release_closure(owners: dict[str, str], reverse: dict[str, set[str]]) -> set[str]:
    """Follow the same verified Cargo graph in the dependency direction."""
    direct_dependencies = {name: set() for name in reverse}
    for dependency, consumers in reverse.items():
        for consumer in consumers:
            direct_dependencies[consumer].add(dependency)
    roots = set(IMAGE_OWNERS["temporal-worker"]["crates"])
    if not roots <= direct_dependencies.keys() or set(owners.values()) != set(reverse):
        raise ValueError("Temporal image roots or workspace graph are incomplete")
    result: set[str] = set()
    pending = deque(sorted(roots))
    while pending:
        package = pending.popleft()
        if package not in result:
            result.add(package)
            pending.extend(sorted(direct_dependencies[package] - result))
    return result


def context_files(tracked: set[str], owners: dict[str, str], release: set[str]) -> set[str]:
    """Whitelist Cargo resolution, compiled sources and delivered assets."""
    mandatory = {"app/rust/Cargo.toml", "app/rust/Cargo.lock"}
    mandatory.update(f"{directory}/Cargo.toml" for directory in owners)
    if not mandatory <= tracked:
        raise ValueError("missing tracked workspace manifest or Cargo.lock")
    paths = set(mandatory)
    for path in tracked:
        # The static release directory has one tracked explanation-only file.
        # It is never delivered by the Worker and must not change the image
        # layers or trigger a new attested production artifact on its own.
        if path in WORKER_NONDELIVERY_FILES:
            continue
        if path.startswith(("app/rust/.sqlx/", "app/contracts/proto/", "app/analytics_lab/proto/")):
            paths.add(path)
        if path.startswith("infra/cloudflare/seo-static/") or path in WORKER_PACKAGING_FILES:
            paths.add(path)
        for directory, package in owners.items():
            if package in release and path.startswith(directory + "/"):
                # The actual compilation closure owns its non-document inputs.
                # README and Dockerfile edits must not poison the Rust layer.
                if not path.endswith(".md") and "/Dockerfile" not in path:
                    paths.add(path)
                break
    required = {
        "app/contracts/proto/read_api.proto",
        "app/contracts/proto/rules.proto",
        "app/contracts/proto/condition.proto",
        "app/contracts/proto/sync.proto",
        "app/contracts/proto/temporal_payloads.proto",
        "app/analytics_lab/proto/analytics.proto",
    }
    if not required <= paths:
        raise ValueError("missing generated-protobuf input for release compilation")
    return paths


def tracked_files(*, root: Path = ROOT) -> set[str]:
    raw = subprocess.check_output(["git", "ls-files", "-z"], cwd=root)
    return {path for path in raw.decode("utf-8").split("\0") if path}


def build_context(destination: Path, source_sha: str, *, root: Path = ROOT) -> dict:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if len(source_sha) != 40 or head != source_sha:
        raise ValueError("release context must use the exact accepted source SHA")
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=root):
        raise ValueError("release source has uncommitted tracked changes")

    metadata = load_cargo_metadata(workspace=root / "app/rust")
    owners, reverse = workspace_graph(metadata, root=root)
    release = release_closure(owners, reverse)
    paths = context_files(tracked_files(root=root), owners, release)
    if destination.exists():
        raise ValueError("release context destination must not already exist")
    destination.mkdir(parents=True)
    for relpath in sorted(paths):
        source = root / relpath
        if not source.is_file() or source.is_symlink():
            raise ValueError(f"unsafe or missing tracked release input: {relpath}")
        target = destination / relpath
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)

    for directory, package in sorted(owners.items()):
        if package not in release:
            # Preserve every original manifest for exact Cargo.lock resolution,
            # but do not include independent implementation sources.
            manifest = tomllib.loads((root / directory / "Cargo.toml").read_text(encoding="utf-8"))
            if any(key in manifest for key in ("lib", "bin")) or "build" in manifest.get("package", {}):
                raise ValueError(f"unsupported non-consumer target declaration: {package}")
            placeholder = destination / directory / "src/lib.rs"
            placeholder.parent.mkdir(parents=True, exist_ok=True)
            placeholder.write_text("// Cargo workspace metadata placeholder; never compiled.\n", encoding="utf-8")

    write_dependency_skeleton(destination, metadata=metadata, owners=owners, release=release, root=root)

    result = {
        "source_sha": source_sha,
        "release_crates": sorted(release),
        "other_workspace_members": sorted(set(owners.values()) - release),
        "tracked_input_count": len(paths),
    }
    (destination / "worker-context.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8",
    )
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    result = build_context(args.destination, args.source_sha)
    print("WORKER_RELEASE_CONTEXT=PASS " + json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
