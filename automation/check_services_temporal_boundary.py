#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEMPORAL = ROOT / "app" / "rust" / "services" / "temporal" / "src"

BANNED = {
    "serde_json::Value": re.compile(r"\bserde_json::Value\b"),
    "json! macro": re.compile(r"\bjson!\s*\("),
    "sqlx usage": re.compile(r"\bsqlx::(query|query_as|query_scalar|PgPool|Row)\b"),
    "neo4rs import": re.compile(r"\buse\s+neo4rs::"),
    "reqwest import": re.compile(r"\buse\s+reqwest::"),
    "qdrant-client import": re.compile(r"\buse\s+qdrant_client::"),
    "tokio-postgres import": re.compile(r"\buse\s+tokio_postgres::"),
    "tonic import": re.compile(r"\buse\s+tonic::"),
    "hyper import": re.compile(r"\buse\s+hyper::"),
    "tower import": re.compile(r"\buse\s+tower::"),
}


def main() -> int:
    violations: list[str] = []
    for path in sorted(TEMPORAL.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for label, pattern in BANNED.items():
            if pattern.search(text):
                violations.append(f"{path.relative_to(ROOT)}: forbidden {label}")

    if violations:
        print("SERVICES_TEMPORAL_BOUNDARY: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("SERVICES_TEMPORAL_BOUNDARY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
