#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"

LAYER_RULES = {
    "contracts": {
        "path": RUST / "crates" / "contracts" / "src",
        "forbidden": [r"\buse\s+(primitives|policies|seo_steps|infrastructure|runtime_models)::"],
    },
    "runtime_models": {
        "path": RUST / "crates" / "runtime_models" / "src",
        "forbidden": [r"\buse\s+(primitives|policies|seo_steps|infrastructure)::"],
    },
    "primitives": {
        "path": RUST / "crates" / "primitives" / "src",
        "forbidden": [r"\buse\s+(contracts|policies|seo_steps|infrastructure|runtime_models)::"],
    },
    "policies": {
        "path": RUST / "crates" / "policies" / "src",
        "forbidden": [r"\buse\s+(seo_steps|infrastructure|runtime_models)::"],
    },
    "seo_steps": {
        "path": RUST / "crates" / "seo_steps" / "src",
        "forbidden": [
            r"\buse\s+temporalio_",
            r"\buse\s+sqlx::",
            r"\bsqlx::PgPool\b",
            r"\bstd::env\b",
            r"\benv::var\(",
            r"\bconnect_pg\(",
            r"\bconnect_neo4j\(",
            r"\bconnect_qdrant\(",
            r"\bVoyageClient::new\(",
            r"\buse\s+neo4rs::",
            r"\buse\s+reqwest::",
            r"\buse\s+qdrant_client::",
            r"\buse\s+tokio_postgres::",
            r"\buse\s+tonic::",
            r"\buse\s+hyper::",
            r"\buse\s+tower::",
            r"\buse\s+rig",
            r"\buse\s+graph_flow",
        ],
    },
    "services_temporal": {
        "path": RUST / "services" / "temporal" / "src",
        "forbidden": [r"\buse\s+sqlx::"],
    },
}


def main() -> int:
    violations: list[str] = []
    for layer_name, cfg in LAYER_RULES.items():
        base = cfg["path"]
        for file_path in sorted(base.rglob("*.rs")):
            if layer_name == "services_temporal" and "bin" in file_path.parts:
                continue
            text = file_path.read_text(encoding="utf-8")
            for pattern in cfg["forbidden"]:
                if re.search(pattern, text):
                    violations.append(
                        f"{file_path.relative_to(ROOT)}: layer `{layer_name}` violates `{pattern}`"
                    )

    if violations:
        print("LAYER_DEPENDENCY_MATRIX: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("LAYER_DEPENDENCY_MATRIX: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
