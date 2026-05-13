#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "app" / "rust" / "Cargo.toml"

WORKSPACE_RULES = {
    "seo_application": {"forbidden": {"infrastructure"}},
    "seo_ports": {"forbidden": {"infrastructure"}},
    "seo_steps": {"forbidden": {"infrastructure", "seo_ports", "seo_application"}},
}

FORBIDDEN_DIRECT_EXTERNAL = {
    "seo_domain": {
        "sqlx",
        "reqwest",
        "neo4rs",
        "qdrant-client",
        "temporalio-sdk",
        "temporalio-client",
        "temporalio-common",
        "temporalio-sdk-core",
        "temporalio-macros",
    }
}


def main() -> int:
    raw = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            str(MANIFEST),
        ],
        cwd=ROOT,
        text=True,
    )
    meta = json.loads(raw)
    packages = {pkg["name"]: pkg for pkg in meta["packages"]}
    errors: list[str] = []

    for package_name, rule in WORKSPACE_RULES.items():
        deps = {dep["name"] for dep in packages[package_name]["dependencies"]}
        forbidden = deps & rule["forbidden"]
        for dep in sorted(forbidden):
            errors.append(f"{package_name} must not depend on {dep}")

    for package_name, forbidden_names in FORBIDDEN_DIRECT_EXTERNAL.items():
        deps = {dep["name"] for dep in packages[package_name]["dependencies"]}
        forbidden = deps & forbidden_names
        for dep in sorted(forbidden):
            errors.append(f"{package_name} must not depend on external crate `{dep}`")

    if errors:
        print("SEO_CRATE_GRAPH: FAILED")
        for error in errors:
            print(f"- {error}")
        return 1

    print("SEO_CRATE_GRAPH: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
