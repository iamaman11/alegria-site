#!/usr/bin/env python3
"""Offline structural regression: independent services must not import infrastructure."""

from __future__ import annotations

import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"
SERVICE_ALLOWED = {
    "gsc_sync": {
        "anyhow", "oauth2", "serde_json", "tokio", "tracing",
        "tracing-subscriber", "http_client",
    },
    "analytics_svc": {
        "anyhow", "tokio", "tonic", "tonic-prost", "prost",
        "tracing", "tracing-subscriber", "graph_neo4j",
    },
}
ADAPTER_ALLOWED = {
    "graph_neo4j": {"anyhow", "neo4rs", "tokio"},
    "http_client": {"anyhow", "reqwest"},
    "vector_qdrant": {"anyhow", "qdrant-client", "reqwest", "serde_json"},
}
FORBIDDEN_TRANSITIVE = {
    "gsc_sync": {
        "infrastructure", "sqlx", "neo4rs", "qdrant-client", "temporalio-sdk",
        "playwright-rs", "rig-core", "rig-vertexai",
    },
    "analytics_svc": {
        "infrastructure", "sqlx", "qdrant-client", "temporalio-sdk",
        "playwright-rs", "rig-core", "rig-vertexai",
    },
}


def lock_dependencies(lock_packages: list[dict], root_name: str) -> set[str]:
    """Conservative closure: include every locked version of a package name."""
    index: dict[str, list[dict]] = {}
    for package in lock_packages:
        index.setdefault(package["name"], []).append(package)
    seen: set[str] = set()
    pending = [root_name]
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        if name not in index:
            raise ValueError(f"missing package in Cargo.lock: {name}")
        seen.add(name)
        for package in index[name]:
            for dependency in package.get("dependencies", []):
                pending.append(dependency.split(" ", 1)[0])
    return seen


def main() -> int:
    manifest = tomllib.loads((RUST / "Cargo.toml").read_text(encoding="utf-8"))
    members = set(manifest["workspace"]["members"])
    for independent, allowed in ADAPTER_ALLOWED.items():
        if f"crates/{independent}" not in members:
            raise ValueError(f"missing workspace member: {independent}")
        config = tomllib.loads(
            (RUST / "crates" / independent / "Cargo.toml").read_text(encoding="utf-8")
        )
        unexpected = set(config["dependencies"]) - allowed
        if unexpected:
            raise ValueError(f"{independent} has unexpected dependencies: {sorted(unexpected)}")

    for name, allowed in SERVICE_ALLOWED.items():
        folder = RUST / "services" / name
        config = tomllib.loads((folder / "Cargo.toml").read_text(encoding="utf-8"))
        dependencies = set(config["dependencies"])
        forbidden = dependencies - allowed
        if forbidden:
            raise ValueError(f"{name} has unexpected heavy dependencies: {sorted(forbidden)}")
        for source in (folder / "src").rglob("*.rs"):
            if "infrastructure::" in source.read_text(encoding="utf-8"):
                raise ValueError(f"{name} still imports infrastructure: {source}")

    lock = tomllib.loads((RUST / "Cargo.lock").read_text(encoding="utf-8"))
    for name, forbidden in FORBIDDEN_TRANSITIVE.items():
        actual = lock_dependencies(lock["package"], name)
        unwanted = actual & forbidden
        if unwanted:
            raise ValueError(f"{name} still transitively compiles heavy SDKs: {sorted(unwanted)}")
    print("INDEPENDENT_RUST_MODULE_BOUNDARIES=PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
