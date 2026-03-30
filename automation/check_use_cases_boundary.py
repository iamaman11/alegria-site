#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
USE_CASES = ROOT / "app" / "rust" / "crates" / "use_cases" / "src"

BANNED_PATTERNS = [
    r"\buse\s+sqlx::",
    r"\bsqlx::PgPool\b",
    r"\bstd::env\b",
    r"\benv::var\(",
    r"\bconnect_pg\(",
    r"\bconnect_neo4j\(",
    r"\bconnect_qdrant\(",
    r"\bVoyageClient::new\(",
    r"\bneo4rs_adapter::\{?connect_neo4j",
    r"\bsqlx_adapter::connect_pg\b",
    r"\buse\s+neo4rs::",
    r"\buse\s+reqwest::",
    r"\buse\s+qdrant_client::",
    r"\buse\s+tokio_postgres::",
    r"\buse\s+tonic::",
    r"\buse\s+hyper::",
    r"\buse\s+tower::",
    r"\buse\s+rig",
    r"\buse\s+graph_flow",
]


def main() -> int:
    violations: list[str] = []
    for path in sorted(USE_CASES.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for pattern in BANNED_PATTERNS:
            if re.search(pattern, text):
                violations.append(
                    f"{path.relative_to(ROOT)}: forbidden boundary leak `{pattern}`"
                )

    if violations:
        print("USE_CASES_BOUNDARY: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("USE_CASES_BOUNDARY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
