#!/usr/bin/env python3
"""Record comparable per-owner Cargo cache footprints for issue #37.

Action restore/save wall times are taken from the GitHub job step timestamps;
this report records the cache hit and the bytes each owner would transfer.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OWNERS = ("architecture-and-rust", "product-truth", "verify")


def directory_bytes(path: Path) -> int:
    if not path.exists():
        return 0
    result = subprocess.run(
        ["du", "-sb", "--", str(path)],
        capture_output=True,
        text=True,
        check=True,
    )
    return int(result.stdout.split(maxsplit=1)[0])


def record(
    owner: str,
    cache_hit: str,
    source_sha: str,
    cargo_home: Path,
    target_dir: Path,
) -> dict[str, object]:
    if owner not in OWNERS:
        raise ValueError("unknown CI Cargo cache owner")
    if cache_hit not in ("true", "false", ""):
        raise ValueError("invalid actions/cache hit output")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("missing exact GitHub source SHA")
    return {
        "schema_version": 1,
        "source_sha": source_sha,
        "owner": owner,
        "strategy": "registry-one-writer-sccache-no-target",
        "registry_cache_hit": cache_hit == "true",
        "registry_cache_hit_known": cache_hit != "",
        "registry_bytes": directory_bytes(cargo_home / "registry"),
        "git_bytes": directory_bytes(cargo_home / "git"),
        "target_bytes_uncached": directory_bytes(target_dir),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", required=True, choices=OWNERS)
    parser.add_argument("--registry-cache-hit", required=True)
    args = parser.parse_args()
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    report = record(
        args.job,
        args.registry_cache_hit,
        os.environ.get("CI_SOURCE_SHA", ""),
        cargo_home,
        ROOT / "app" / "rust" / "target",
    )
    destination = ROOT / "automation" / "reports" / f"cargo_cache_{args.job}.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, sort_keys=True) + "\n", encoding="utf-8")
    print("CI_CARGO_CACHE_METRIC " + json.dumps(report, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
