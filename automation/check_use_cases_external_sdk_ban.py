#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
USE_CASES = ROOT / "app" / "rust" / "crates" / "use_cases" / "src"

BANNED = {
    "neo4rs import": re.compile(r"\buse\s+neo4rs::"),
    "reqwest import": re.compile(r"\buse\s+reqwest::"),
    "qdrant-client import": re.compile(r"\buse\s+qdrant_client::"),
    "temporal sdk import": re.compile(r"\buse\s+temporalio"),
    "tonic import": re.compile(r"\buse\s+tonic::"),
    "hyper import": re.compile(r"\buse\s+hyper::"),
    "tower import": re.compile(r"\buse\s+tower::"),
    "tokio-postgres import": re.compile(r"\buse\s+tokio_postgres::"),
    "rig import": re.compile(r"\buse\s+rig"),
    "graph-flow import": re.compile(r"\buse\s+graph_flow"),
    "playwright-rs import": re.compile(r"\buse\s+playwright"),
}


def main() -> int:
    violations: list[str] = []
    for path in sorted(USE_CASES.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for label, pattern in BANNED.items():
            if pattern.search(text):
                violations.append(f"{path.relative_to(ROOT)}: forbidden {label}")

    if violations:
        print("USE_CASES_EXTERNAL_SDK_BAN: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("USE_CASES_EXTERNAL_SDK_BAN: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
