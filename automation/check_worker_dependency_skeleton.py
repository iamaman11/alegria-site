#!/usr/bin/env python3
"""Verify the Docker dependency skeleton resolves the exact Cargo graph.

This is a metadata-only PR gate; the actual Worker binaries still compile on
the accepted main SHA in the one signed-image release job.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


DEPENDENCY_FIELDS = (
    "name", "source", "req", "kind", "rename", "optional",
    "uses_default_features", "features", "target", "registry",
)


def graph_signature(metadata: dict) -> dict:
    """Ignore relocated absolute paths but compare every Cargo dependency edge."""
    return {
        package["name"]: {
            "version": package["version"],
            "features": package["features"],
            "dependencies": sorted(
                (tuple(json.dumps(dep.get(key), sort_keys=True) for key in DEPENDENCY_FIELDS)
                 for dep in package["dependencies"])
            ),
            "targets": sorted(
                (target["name"], tuple(sorted(target["kind"])))
                for target in package["targets"]
                if set(target["kind"]) & {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin", "custom-build"}
            ),
        }
        for package in metadata["packages"]
    }


def cargo_metadata(manifest: Path) -> dict:
    output = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version=1", "--no-deps", "--manifest-path", str(manifest)],
        text=True,
    )
    return json.loads(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("actual_manifest", type=Path)
    parser.add_argument("skeleton_manifest", type=Path)
    args = parser.parse_args()
    actual = graph_signature(cargo_metadata(args.actual_manifest))
    skeleton = graph_signature(cargo_metadata(args.skeleton_manifest))
    if actual != skeleton:
        missing = sorted(set(actual) ^ set(skeleton))
        changed = sorted(name for name in set(actual) & set(skeleton) if actual[name] != skeleton[name])
        raise SystemExit(f"WORKER_DEPENDENCY_GRAPH=FAIL missing={missing} changed={changed}")
    print(f"WORKER_DEPENDENCY_GRAPH=PASS packages={len(actual)}")


if __name__ == "__main__":
    main()
