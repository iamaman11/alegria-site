#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST_ROOT = ROOT / "app" / "rust"
INFRA_SRC = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src"
ADAPTERS_DIR = INFRA_SRC / "adapters"

ALLOWED_DIRECT_SDK_MANIFESTS = {
    (ROOT / "app" / "rust" / "Cargo.toml").resolve(),
    (ROOT / "app" / "rust" / "crates" / "infrastructure" / "Cargo.toml").resolve(),
}

MANIFEST_DEP_EXCEPTIONS = {
    (ROOT / "app" / "rust" / "services" / "temporal" / "Cargo.toml").resolve(): {
        "temporalio-sdk",
        "temporalio-sdk-core",
        "temporalio-client",
        "temporalio-common",
        "temporalio-macros",
    },
}

ENFORCED_DEP_KEYS = [
    "neo4rs",
    "qdrant-client",
    "reqwest",
    "tokio-postgres",
    "hyper",
    "tower",
    "playwright-rs",
    "graph-flow",
    "rig-core",
    "rig-vertexai",
    "temporalio-sdk",
    "temporalio-sdk-core",
    "temporalio-client",
    "temporalio-common",
    "temporalio-macros",
]

FORBIDDEN_DIRECT_IN_USE_CASES = [
    re.compile(r"\buse\s+neo4rs::"),
    re.compile(r"\buse\s+qdrant_client::"),
    re.compile(r"\buse\s+reqwest::"),
    re.compile(r"\buse\s+tonic::"),
    re.compile(r"\buse\s+hyper::"),
    re.compile(r"\buse\s+tower::"),
    re.compile(r"\buse\s+tokio_postgres::"),
    re.compile(r"\buse\s+rig::"),
    re.compile(r"\buse\s+rig_vertexai::"),
    re.compile(r"\buse\s+playwright_rs::"),
    re.compile(r"\buse\s+graph_flow::"),
    re.compile(r"\buse\s+temporalio_sdk::"),
    re.compile(r"\buse\s+temporalio_sdk_core::"),
    re.compile(r"\buse\s+temporalio_client::"),
    re.compile(r"\buse\s+temporalio_common::"),
    re.compile(r"\buse\s+temporalio_macros::"),
    re.compile(r"\bneo4rs::"),
    re.compile(r"\bqdrant_client::"),
    re.compile(r"\breqwest::"),
    re.compile(r"\btonic::"),
    re.compile(r"\bhyper::"),
    re.compile(r"\btower::"),
    re.compile(r"\btokio_postgres::"),
    re.compile(r"\brig::"),
    re.compile(r"\brig_vertexai::"),
    re.compile(r"\bplaywright_rs::"),
    re.compile(r"\bgraph_flow::"),
    re.compile(r"\btemporalio_sdk::"),
    re.compile(r"\btemporalio_sdk_core::"),
    re.compile(r"\btemporalio_client::"),
    re.compile(r"\btemporalio_common::"),
    re.compile(r"\btemporalio_macros::"),
]

REQUIRED_ADAPTERS = {
    "mod.rs",
    "sqlx_adapter.rs",
    "tokio_postgres_adapter.rs",
    "neo4rs_adapter.rs",
    "temporalio_sdk_adapter.rs",
    "hyper_adapter.rs",
    "tower_adapter.rs",
    "playwright_rs_adapter.rs",
    "graph_flow_adapter.rs",
    "rig_core_adapter.rs",
    "rig_vertexai_adapter.rs",
    "qdrant_client_adapter.rs",
    "voyage_api_adapter.rs",
    "tonic_adapter.rs",
    "sqlx_outbox_adapter.rs",
    "tracing_adapter.rs",
    "reqwest_adapter.rs",
}

FORBIDDEN_INFRA_ROOT_FILES = {
    "postgres.rs",
    "neo4j.rs",
    "qdrant.rs",
    "voyage_client.rs",
    "analytics_client.rs",
    "outbox_store.rs",
    "telemetry.rs",
}


def main() -> int:
    violations: list[str] = []

    if not RUST_ROOT.exists():
        print(f"RUST_ADAPTER_BOUNDARY: FAILED\n- missing path: {RUST_ROOT}")
        return 1
    if not INFRA_SRC.exists():
        print(f"RUST_ADAPTER_BOUNDARY: FAILED\n- missing path: {INFRA_SRC}")
        return 1

    for file_path in sorted(RUST_ROOT.rglob("*.rs")):
        rel = file_path.relative_to(ROOT)
        if "target" in file_path.parts:
            continue
        if ADAPTERS_DIR in file_path.parents:
            continue
        text = file_path.read_text(encoding="utf-8")
        for pat in FORBIDDEN_DIRECT_IN_USE_CASES:
            if pat.search(text):
                violations.append(f"{rel}: forbidden direct external SDK usage matched `{pat.pattern}`")

    if not ADAPTERS_DIR.exists():
        violations.append("app/rust/crates/infrastructure/src/adapters: missing")
    else:
        present = {p.name for p in ADAPTERS_DIR.glob("*.rs")}
        for req in sorted(REQUIRED_ADAPTERS):
            if req not in present:
                violations.append(f"missing adapter module: {ADAPTERS_DIR / req}")

    dep_patterns = {k: re.compile(rf"^\s*{re.escape(k)}\s*=", re.MULTILINE) for k in ENFORCED_DEP_KEYS}
    for manifest in sorted(RUST_ROOT.rglob("Cargo.toml")):
        manifest = manifest.resolve()
        if manifest in ALLOWED_DIRECT_SDK_MANIFESTS:
            continue
        text = manifest.read_text(encoding="utf-8")
        allowed_for_manifest = MANIFEST_DEP_EXCEPTIONS.get(manifest, set())
        for dep_key, pat in dep_patterns.items():
            if dep_key in allowed_for_manifest:
                continue
            if pat.search(text):
                rel = manifest.relative_to(ROOT)
                violations.append(
                    f"{rel}: direct dependency `{dep_key}` is forbidden outside infrastructure adapters"
                )

    for name in sorted(FORBIDDEN_INFRA_ROOT_FILES):
        p = INFRA_SRC / name
        if p.exists():
            violations.append(f"legacy infrastructure module must be removed: {p}")

    if violations:
        print("RUST_ADAPTER_BOUNDARY: FAILED")
        for v in violations:
            print(f"- {v}")
        return 1

    print("RUST_ADAPTER_BOUNDARY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
