#!/usr/bin/env python3
"""Fail closed on accidental V3 promotion into the default Compose runtime.

The opt-in services are build/runtime experiments, not a release path. Keep
this independent of optional PyYAML, Docker and a running Temporal cluster.
"""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[2]
COMPOSE = ROOT / "docker-compose.yml"
IMAGE = ROOT / "infra/temporal/Dockerfile.v3"
PURE_CARGO = ROOT / "app/rust/services/temporal_orchestrator/Cargo.toml"
PG_CARGO = ROOT / "app/rust/services/temporal_pg_effects/Cargo.toml"


def service_body(document: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [a-z][a-z0-9_-]*:|^volumes:|\Z)",
        document,
    )
    if not match:
        raise ValueError(f"missing Compose service {name}")
    return match.group(1)


def check(compose: str, dockerfile: str, pure_cargo: str, pg_cargo: str) -> list[str]:
    failures: list[str] = []
    legacy = service_body(compose, "temporal-worker")
    pure = service_body(compose, "temporal-orchestrator-v3")
    pg = service_body(compose, "temporal-pg-effects-v3")
    if "    profiles:" in legacy:
        failures.append("legacy production service must not silently switch profiles")
    for name, body, stage in [
        ("temporal-orchestrator-v3", pure, "pure"),
        ("temporal-pg-effects-v3", pg, "pg"),
    ]:
        for line in [
            '    profiles: ["v3-evaluation"]',
            "      dockerfile: ./infra/temporal/Dockerfile.v3",
            f"      target: {stage}",
            "      temporal-server:",
            "        condition: service_healthy",
        ]:
            if line not in body.splitlines():
                failures.append(f"{name} missing {line.strip()}")
        if "    ports:" in body:
            failures.append(f"{name} must not publish a network port")

    for line in [
        "      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-pure-dag-v3",
        "      WORKER_BUILD_ID: ${WORKER_BUILD_ID:-}",
        "      ALLOW_PURE_DAG_ORCHESTRATOR: ${ALLOW_PURE_DAG_ORCHESTRATOR:-false}",
    ]:
        if line not in pure.splitlines():
            failures.append(f"pure orchestrator missing fail-closed {line.strip()}")
    for forbidden in ["DATABASE_URL:", "PG_EFFECT_WORKER_DATABASE_URL:", "NEO4J_", "QDRANT_"]:
        if forbidden in pure:
            failures.append(f"provider-free orchestrator unexpectedly receives {forbidden}")

    for line in [
        "      PG_EFFECT_WORKER_DATABASE_URL: ${PG_EFFECT_WORKER_DATABASE_URL:-}",
        "      ALEGRIA_ENABLE_PG_EFFECT_V3: ${ALEGRIA_ENABLE_PG_EFFECT_V3:-false}",
    ]:
        if line not in pg.splitlines():
            failures.append(f"PG owner missing fail-closed {line.strip()}")
    for forbidden in ["      DATABASE_URL:", "      NEO4J_", "      QDRANT_", "      OUTBOX_"]:
        if forbidden in pg:
            failures.append(f"PG-only worker unexpectedly receives {forbidden.strip()}")

    for target, executable in [
        ("pure", "temporal_orchestrator"),
        ("pg", "temporal_pg_effects"),
    ]:
        if f"FROM runtime AS {target}" not in dockerfile:
            failures.append(f"missing isolated final Docker target {target}")
        if f"-p {executable} --bin {executable}" not in dockerfile:
            failures.append(f"missing exact single-owner Cargo build {executable}")
        if f'ENTRYPOINT ["/usr/local/bin/{executable}"]' not in dockerfile:
            failures.append(f"missing isolated entrypoint for {executable}")
    if "-p temporal_worker" in dockerfile or 'ENTRYPOINT ["/usr/local/bin/temporal_worker"]' in dockerfile:
        failures.append("V3 images must not compile or start legacy worker")
    # No provider SDK or the legacy runtime may re-enter either manifest,
    # even if the Docker target still looks narrow.
    forbidden_manifest_dependencies = [
        "graph_neo4j", "vector_qdrant", "infrastructure", "runtime_composition",
        "projection_delivery", "temporal_worker",
    ]
    for owner, manifest in [("pure", pure_cargo), ("pg", pg_cargo)]:
        for dependency in forbidden_manifest_dependencies:
            if re.search(rf"(?m)^{dependency}\s*=", manifest):
                failures.append(f"{owner} binary reintroduced dependency {dependency}")
    return failures


def main() -> int:
    compose = COMPOSE.read_text(encoding="utf-8")
    image = IMAGE.read_text(encoding="utf-8")
    pure_cargo = PURE_CARGO.read_text(encoding="utf-8")
    pg_cargo = PG_CARGO.read_text(encoding="utf-8")
    failures = check(compose, image, pure_cargo, pg_cargo)
    if not failures:
        for broken in [
            compose.replace('    profiles: ["v3-evaluation"]', "    profiles: []", 1),
            compose.replace("      ALLOW_PURE_DAG_ORCHESTRATOR: ${ALLOW_PURE_DAG_ORCHESTRATOR:-false}", ""),
            compose.replace("      PG_EFFECT_WORKER_DATABASE_URL: ${PG_EFFECT_WORKER_DATABASE_URL:-}", ""),
        ]:
            if not check(broken, image, pure_cargo, pg_cargo):
                failures.append("negative V3 topology contract self-test failed")
        if not check(compose, image, pure_cargo + "\ngraph_neo4j = { path = 'bad' }", pg_cargo):
            failures.append("negative provider dependency contract self-test failed")
    if failures:
        for failure in failures:
            print(f"V3_TOPOLOGY: FAIL {failure}")
        return 1
    print("V3_TOPOLOGY: PASS (opt-in only; not a production cutover)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
