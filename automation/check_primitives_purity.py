#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PRIMITIVES = ROOT / "app" / "rust" / "crates" / "primitives" / "src"

FORBIDDEN_PATTERNS = {
    "contracts import": re.compile(r"\buse\s+contracts::"),
    "sqlx import": re.compile(r"\buse\s+sqlx::"),
    "reqwest import": re.compile(r"\buse\s+reqwest::"),
    "neo4rs import": re.compile(r"\buse\s+neo4rs::"),
    "qdrant-client import": re.compile(r"\buse\s+qdrant_client::"),
    "temporal import": re.compile(r"\buse\s+temporalio_"),
    "tokio-postgres import": re.compile(r"\buse\s+tokio_postgres::"),
    "tonic import": re.compile(r"\buse\s+tonic::"),
    "hyper import": re.compile(r"\buse\s+hyper::"),
    "tower import": re.compile(r"\buse\s+tower::"),
    "rig import": re.compile(r"\buse\s+rig(?:::|_)"),
    "graph-flow import": re.compile(r"\buse\s+graph_flow::"),
    "playwright-rs import": re.compile(r"\buse\s+playwright_rs::"),
    "serde_json::Value": re.compile(r"\bserde_json::Value\b"),
    "json! macro": re.compile(r"\bjson!\s*\("),
    "serde_json::to_value": re.compile(r"\bserde_json::to_value\b"),
    "serde_json::from_value": re.compile(r"\bserde_json::from_value\b"),
    "serde_json::from_slice": re.compile(r"\bserde_json::from_slice\b"),
    "serde_json::from_str": re.compile(r"\bserde_json::from_str\b"),
}


def is_json_wrapper(path: Path) -> bool:
    return path.name.endswith("_json.rs")


def main() -> int:
    violations: list[str] = []
    for file_path in sorted(PRIMITIVES.rglob("*.rs")):
        text = file_path.read_text(encoding="utf-8")
        for label, pattern in FORBIDDEN_PATTERNS.items():
            if is_json_wrapper(file_path) and label in {
                "serde_json::Value",
                "json! macro",
                "serde_json::to_value",
                "serde_json::from_value",
                "serde_json::from_slice",
                "serde_json::from_str",
            }:
                continue
            if pattern.search(text):
                violations.append(f"{file_path.relative_to(ROOT)}: forbidden {label}")

    if violations:
        print("PRIMITIVES_PURITY: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("PRIMITIVES_PURITY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
