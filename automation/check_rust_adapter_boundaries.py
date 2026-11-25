#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST_ROOT = ROOT / "app" / "rust"
INFRA_SRC = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src"
ADAPTERS_DIR = INFRA_SRC / "adapters"
POSTGRES_STORE_SRC = RUST_ROOT / "crates" / "postgres_store" / "src"
INTEGRATION_HARNESS_DIR = ROOT / "app" / "rust" / "crates" / "integration_harness"
# This is an opt-in, test-only binary owner. It needs the native Temporal
# client to validate actual History/Signal replay, but it is not a production
# adapter or a back door for unrelated provider SDKs.
PG_E2E_DIR = RUST_ROOT / "crates" / "integration_pg_v3_e2e"
PG_E2E_SRC = PG_E2E_DIR / "src"
PG_E2E_TEMPORAL_SDKS = frozenset({
    "temporalio-client", "temporalio-common", "temporalio-sdk-core",
})
TEMPORAL_GATEWAY_SRC = RUST_ROOT / "crates" / "temporal_gateway" / "src"
TEMPORAL_WORKER_SRC = RUST_ROOT / "services" / "temporal" / "src"
TEMPORAL_ORCHESTRATOR_SRC = RUST_ROOT / "services" / "temporal_orchestrator" / "src"
TEMPORAL_PG_EFFECTS_SRC = RUST_ROOT / "services" / "temporal_pg_effects" / "src"
INFRA_MANIFEST = RUST_ROOT / "crates" / "infrastructure" / "Cargo.toml"
COMPOSITION_MANIFEST = RUST_ROOT / "crates" / "runtime_composition" / "Cargo.toml"
# SDKs may be used by these dedicated adapter crates, never by domain code.
DEDICATED_ADAPTER_DIRS = {
    (RUST_ROOT / "crates" / "graph_neo4j").resolve(),
    (RUST_ROOT / "crates" / "http_client").resolve(),
    (RUST_ROOT / "crates" / "vector_qdrant").resolve(),
}
# A standalone gRPC service owns its transport-facing request/response types.
GRPC_SERVICE_SRCS = {
    (RUST_ROOT / "services" / name / "src").resolve()
    for name in ("analytics_svc", "primitives_svc")
}
ANALYTICS_CLIENT_SRC = (RUST_ROOT / "services" / "cli_tools" / "src" / "cli" / "analytics_client.rs").resolve()

ALLOWED_DIRECT_SDK_MANIFESTS = {
    (ROOT / "app" / "rust" / "Cargo.toml").resolve(),
    (ROOT / "app" / "rust" / "crates" / "infrastructure" / "Cargo.toml").resolve(),
    *(directory / "Cargo.toml" for directory in DEDICATED_ADAPTER_DIRS),
}

MANIFEST_DEP_EXCEPTIONS = {
    (RUST_ROOT / "crates" / "temporal_gateway" / "Cargo.toml").resolve(): {
        "temporalio-client", "temporalio-common", "temporalio-sdk-core",
    },
    (ROOT / "app" / "rust" / "services" / "temporal" / "Cargo.toml").resolve(): {
        "temporalio-sdk",
        "temporalio-sdk-core",
        "temporalio-client",
        "temporalio-common",
        "temporalio-macros",
    },
    # Independent native Temporal runtime owners are legitimate SDK consumers.
    # These are exact manifest exceptions, not permission for domain crates or
    # unrelated provider dependencies to import a Temporal SDK.
    (RUST_ROOT / "services" / "temporal_orchestrator" / "Cargo.toml").resolve(): {
        "temporalio-sdk",
        "temporalio-client",
        "temporalio-common",
        "temporalio-macros",
    },
    (RUST_ROOT / "services" / "temporal_pg_effects" / "Cargo.toml").resolve(): {
        "temporalio-sdk",
        "temporalio-sdk-core",
        "temporalio-client",
        "temporalio-common",
        "temporalio-macros",
    },
    (INTEGRATION_HARNESS_DIR / "Cargo.toml").resolve(): {
        "neo4rs",
        "qdrant-client",
        "reqwest",
        "sqlx",
        "temporalio-client",
        "temporalio-common",
        "temporalio-sdk-core",
    },
    (PG_E2E_DIR / "Cargo.toml").resolve(): PG_E2E_TEMPORAL_SDKS,
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
    "tokio_postgres_adapter.rs",
    "hyper_adapter.rs",
    "tower_adapter.rs",
    "playwright_rs_adapter.rs",
    "voyage_api_adapter.rs",
    "sqlx_outbox_adapter.rs",
    "tracing_adapter.rs",
    "reqwest_adapter.rs",
}

REQUIRED_POSTGRES_OWNER_FILES = {
    "lib.rs",
    "sqlx_adapter.rs",
    "sqlx_error.rs",
    "sqlx_reconcile_adapter.rs",
    "sqlx_reconcile_runtime_adapter.rs",
    "sqlx_runtime_health_adapter.rs",
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


def workspace_dependency_closure(manifest: Path) -> tuple[set[str], set[str]]:
    """Walk real local path dependencies, including target/dev/build edges.

    Cargo's compilation graph may enable feature-dependent edges; checking
    their declared union prevents a future feature from silently reconnecting
    the PostgreSQL owner to Neo4j or Qdrant. No Rust toolchain is required.
    """
    visited: set[Path] = set()
    packages: set[str] = set()
    direct: set[str] = set()

    def visit(current: Path, *, root: bool = False) -> None:
        current = current.resolve()
        if current in visited:
            return
        visited.add(current)
        package = tomllib.loads(current.read_text(encoding="utf-8"))
        name = package.get("package", {}).get("name")
        if not isinstance(name, str):
            raise ValueError(f"missing package.name in {current}")
        packages.add(name)
        tables = [package]
        tables.extend(
            target for target in package.get("target", {}).values()
            if isinstance(target, dict)
        )
        for table in tables:
            for key in ("dependencies", "dev-dependencies", "build-dependencies"):
                for dependency in table.get(key, {}).values():
                    if not isinstance(dependency, dict) or "path" not in dependency:
                        continue
                    linked = (current.parent / dependency["path"] / "Cargo.toml").resolve()
                    if not linked.is_relative_to(RUST_ROOT.resolve()):
                        raise ValueError(f"workspace dependency escapes root: {linked}")
                    if root:
                        linked_package = tomllib.loads(linked.read_text(encoding="utf-8"))
                        direct.add(linked_package["package"]["name"])
                    visit(linked)

    visit(manifest, root=True)
    return packages, direct


def pg_e2e_direct_temporal_sdk_allowed(file_path: Path, pattern: re.Pattern[str]) -> bool:
    """Except precisely three native Temporal SDKs in the isolated E2E src.

    Unlike the older all-provider integration harness, this owner must still
    fail if Graph/Qdrant, reqwest or an additional Temporal SDK appears.
    """
    if PG_E2E_SRC not in file_path.parents:
        return False
    return any(
        f"{dependency.replace('-', '_')}::" in pattern.pattern
        for dependency in PG_E2E_TEMPORAL_SDKS
    )


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
        if INTEGRATION_HARNESS_DIR in file_path.parents:
            continue
        if any(directory in file_path.parents for directory in DEDICATED_ADAPTER_DIRS):
            continue
        text = file_path.read_text(encoding="utf-8")
        for pat in FORBIDDEN_DIRECT_IN_USE_CASES:
            if pg_e2e_direct_temporal_sdk_allowed(file_path, pat):
                continue
            # Temporal's Workflow/Activity SDK belongs to its runtime, and
            # client-only SDKs belong to the independent Temporal gateway.
            # Other external SDKs remain forbidden in those owner modules.
            if ("temporalio_" in pat.pattern and any(
                directory in file_path.parents
                for directory in (
                    TEMPORAL_WORKER_SRC,
                    TEMPORAL_GATEWAY_SRC,
                    TEMPORAL_ORCHESTRATOR_SRC,
                    TEMPORAL_PG_EFFECTS_SRC,
                )
            )):
                continue
            # A gRPC server must use tonic request/response types directly,
            # but still must not import unrelated external SDKs.
            if "tonic" in pat.pattern and (
                any(directory in file_path.parents for directory in GRPC_SERVICE_SRCS)
                or file_path.resolve() == ANALYTICS_CLIENT_SRC
            ):
                continue
            if pat.search(text):
                violations.append(f"{rel}: forbidden direct external SDK usage matched `{pat.pattern}`")

    if not ADAPTERS_DIR.exists():
        violations.append("app/rust/crates/infrastructure/src/adapters: missing")
    else:
        present = {p.name for p in ADAPTERS_DIR.glob("*.rs")}
        for req in sorted(REQUIRED_ADAPTERS):
            if req not in present:
                violations.append(f"missing adapter module: {ADAPTERS_DIR / req}")

    present_postgres = {p.name for p in POSTGRES_STORE_SRC.glob("*.rs")}
    for required in sorted(REQUIRED_POSTGRES_OWNER_FILES):
        if required not in present_postgres:
            violations.append(f"missing PostgreSQL owner module: {POSTGRES_STORE_SRC / required}")

    dep_patterns = {k: re.compile(rf"^\s*{re.escape(k)}\s*=", re.MULTILINE) for k in ENFORCED_DEP_KEYS}

    # SDK exceptions here are *only* test feature dependencies. Otherwise a
    # default workspace build could silently turn the E2E package into an
    # extra Temporal runtime owner with unconditional SDK linkage.
    try:
        pg_doc = tomllib.loads((PG_E2E_DIR / "Cargo.toml").read_text(encoding="utf-8"))
        pg_deps = pg_doc["dependencies"]
        pg_e2e_features = set(pg_doc["features"]["e2e"])
        for sdk in sorted(PG_E2E_TEMPORAL_SDKS):
            declared = pg_deps.get(sdk)
            if (not isinstance(declared, dict) or declared.get("optional") is not True
                    or f"dep:{sdk}" not in pg_e2e_features):
                violations.append(
                    f"integration_pg_v3_e2e must gate native test-only SDK `{sdk}` behind `e2e`"
                )
    except (OSError, KeyError, TypeError, ValueError, tomllib.TOMLDecodeError) as error:
        violations.append(f"cannot verify isolated PG E2E SDK feature boundary: {error}")

    for manifest in sorted(RUST_ROOT.rglob("Cargo.toml")):
        manifest = manifest.resolve()
        if manifest in ALLOWED_DIRECT_SDK_MANIFESTS:
            # Storage adapters must not accidentally recover independent
            # provider SDK dependencies removed by the owner split.
            if manifest == INFRA_MANIFEST.resolve():
                for dep_key in ("neo4rs", "qdrant-client", "temporalio-sdk", "temporalio-sdk-core",
                                "temporalio-client", "temporalio-common", "temporalio-macros"):
                    if dep_patterns[dep_key].search(manifest.read_text(encoding="utf-8")):
                        violations.append(f"{manifest.relative_to(ROOT)}: provider SDK `{dep_key}` belongs to its dedicated owner")
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

    # Enforce the architectural dependency direction, not just source path or
    # direct SDK imports: infra/PG must never compile graph/vector providers.
    try:
        infra_closure, infra_direct = workspace_dependency_closure(INFRA_MANIFEST)
        composition_closure, composition_direct = workspace_dependency_closure(COMPOSITION_MANIFEST)
        for forbidden in ("graph_neo4j", "vector_qdrant", "runtime_composition"):
            if forbidden in infra_closure:
                violations.append(
                    f"infrastructure Cargo dependency closure imports provider/composer `{forbidden}`"
                )
        required_composition = {"infrastructure", "graph_neo4j", "vector_qdrant"}
        for required in sorted(required_composition - composition_direct):
            violations.append(
                f"runtime_composition must directly depend on owner `{required}`"
            )
        if "runtime_composition" in infra_direct:
            violations.append("infrastructure -> runtime_composition would create an owner cycle")
        for owner in ("graph_neo4j", "vector_qdrant"):
            owner_manifest = RUST_ROOT / "crates" / owner / "Cargo.toml"
            owner_closure, _ = workspace_dependency_closure(owner_manifest)
            for forbidden in ("infrastructure", "runtime_composition"):
                if forbidden in owner_closure:
                    violations.append(f"{owner} depends on higher-level `{forbidden}`")
        if not required_composition.issubset(composition_closure):
            violations.append("runtime_composition Cargo closure lacks one or more provider owners")
    except (OSError, ValueError, KeyError, tomllib.TOMLDecodeError) as error:
        violations.append(f"cannot verify provider Cargo dependency closure: {error}")

    if violations:
        print("RUST_ADAPTER_BOUNDARY: FAILED")
        for v in violations:
            print(f"- {v}")
        return 1

    print("RUST_ADAPTER_BOUNDARY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
