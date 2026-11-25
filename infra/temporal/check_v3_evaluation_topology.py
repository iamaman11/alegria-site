#!/usr/bin/env python3
"""Fail closed on accidental V3/V4 promotion into default Compose runtime.

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
GATEWAY_RUNTIME = ROOT / "app/rust/crates/temporal_gateway/src/lib.rs"
PARENT_RUNTIME = ROOT / "app/rust/services/temporal_orchestrator/src/main.rs"
PARENT_WORKFLOWS = ROOT / "app/rust/services/temporal_orchestrator/src/workflow.rs"
PG_RUNTIME = ROOT / "app/rust/services/temporal_pg_effects/src/main.rs"
PG_OPERATOR_V4 = ROOT / "app/rust/services/temporal_pg_effects/src/operator_v4.rs"


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
    effect_parent = service_body(compose, "temporal-effect-dag-v4")
    pg = service_body(compose, "temporal-pg-effects-v3")
    if "    profiles:" in legacy:
        failures.append("legacy production service must not silently switch profiles")
    for name, body, stage, owner, context in [
        ("temporal-orchestrator-v3", pure, "pure", "temporal_orchestrator", "V3_PURE_BUILD_CONTEXT"),
        ("temporal-pg-effects-v3", pg, "pg", "temporal_pg_effects", "V3_PG_BUILD_CONTEXT"),
    ]:
        for line in [
            f"      context: ${{{context}:-.}}",
            "      dockerfile: ./infra/temporal/Dockerfile.v3",
            f"      target: {stage}",
            f"        OWNER_PACKAGE: {owner}",
            "      temporal-server:",
            "        condition: service_healthy",
        ]:
            if line not in body.splitlines():
                failures.append(f"{name} missing {line.strip()}")
        # The one PostgreSQL child binary deliberately serves three opt-in
        # parents. Require all three explicit profiles; V3 must not disappear
        # when V4/V5 are added, and no unexpected auto-start profile is safe.
        expected_profiles = (
            '    profiles: ["v3-evaluation"]' if stage == "pure" else
            '    profiles: ["v3-evaluation", "v4-effect-evaluation", "v5-effect-evaluation"]'
        )
        if expected_profiles not in body.splitlines():
            failures.append(f"{name} missing profiles: {expected_profiles}")
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

    # One binary can poll exactly one of these queues per process. Registering
    # the V4 parent on the V3 queue would invalidate operator History proof.
    for line in [
        '    profiles: ["v4-effect-evaluation"]',
        "      context: ${V3_PURE_BUILD_CONTEXT:-.}",
        "      dockerfile: ./infra/temporal/Dockerfile.v3",
        "      target: pure",
        "        OWNER_PACKAGE: temporal_orchestrator",
        "      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4",
        '      ALLOW_PURE_DAG_ORCHESTRATOR: "false"',
        "      ALLOW_EFFECT_DAG_ORCHESTRATOR_V4: ${ALLOW_EFFECT_DAG_ORCHESTRATOR_V4:-false}",
        "      WORKER_BUILD_ID: ${WORKER_BUILD_ID:-}",
        "      temporal-server:",
        "        condition: service_healthy",
    ]:
        if line not in effect_parent.splitlines():
            failures.append(f"V4 parent missing fail-closed {line.strip()}")
    if "    ports:" in effect_parent:
        failures.append("V4 parent must not publish a network port")
    for forbidden in [
        "      DATABASE_URL:", "      PG_EFFECT_WORKER_DATABASE_URL:",
        "      PG_EFFECT_WORKER_DATABASE_URL_FILE:", "      NEO4J_", "      QDRANT_",
        "      OUTBOX_", "    secrets:",
    ]:
        if forbidden in effect_parent:
            failures.append(f"provider-free V4 parent unexpectedly receives {forbidden.strip()}")
    if "      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4" in pure:
        failures.append("V3 pure queue and V4 effect queue must remain disjoint")

    for line in [
        "      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn",
        "      ALEGRIA_ENABLE_PG_EFFECT_V3: ${ALEGRIA_ENABLE_PG_EFFECT_V3:-false}",
        "    secrets:",
        "      - source: pg_effect_worker_dsn",
        "        target: pg_effect_worker_dsn",
        '        uid: "65532"',
        '        gid: "65532"',
        "        mode: 0400",
    ]:
        if line not in pg.splitlines():
            failures.append(f"PG owner missing fail-closed {line.strip()}")
    if not re.search(r"(?m)^secrets:\n  pg_effect_worker_dsn:\n    external: true$", compose):
        failures.append("PG effect must use one externally provisioned Compose secret")
    for forbidden in ["      DATABASE_URL:", "      PG_EFFECT_WORKER_DATABASE_URL:", "      NEO4J_", "      QDRANT_", "      OUTBOX_"]:
        if forbidden in pg:
            failures.append(f"PG-only worker unexpectedly receives {forbidden.strip()}")

    for required in [
        "ARG OWNER_PACKAGE", "COPY dependency-skeleton/app ./app", "COPY app ./app",
        'temporal_orchestrator|temporal_pg_effects) ;;',
        '-p "$OWNER_PACKAGE" --bin "$OWNER_PACKAGE"',
    ]:
        if required not in dockerfile:
            failures.append(f"V3 Docker build missing source-scoped owner contract: {required}")
    if dockerfile.find("COPY dependency-skeleton/app ./app") > dockerfile.find("COPY app ./app"):
        failures.append("actual source copied before stable Cargo dependency layer")
    for target, executable in [
        ("pure", "temporal_orchestrator"),
        ("pg", "temporal_pg_effects"),
    ]:
        if f"FROM runtime AS {target}" not in dockerfile:
            failures.append(f"missing isolated final Docker target {target}")
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


def check_v4_cross_owner_queue_contract(
    gateway: str, parent: str, workflow: str, pg: str, operator: str,
) -> list[str]:
    """Prevent an apparently valid Compose route that no real worker polls.

    The PG admission operator attests server Histories with literal queues,
    while the parent worker gets its queue from temporal_gateway. Checking
    both source owners caught a prior integration mismatch at the boundary.
    """
    expected = [
        (gateway, 'pub const PURE_DAG_ORCHESTRATOR_QUEUE: &str = "alegria-pure-dag-v3";', "pure gateway queue"),
        (gateway, 'pub const EFFECT_DAG_ORCHESTRATOR_V4_QUEUE: &str = "alegria-effect-dag-v4";', "V4 gateway queue"),
        (parent, "Self::PureV3 => PURE_DAG_ORCHESTRATOR_QUEUE", "exclusive V3 mode"),
        (parent, "Self::AdmittedEffectV4 => EFFECT_DAG_ORCHESTRATOR_V4_QUEUE", "exclusive V4 mode"),
        (parent, "OrchestratorMode::AdmittedEffectV4 => workflow::register_v4(&mut options)?", "V4 registration"),
        (workflow, 'pub(crate) const PG_V4_CHILD_COMMAND_QUEUE: &str = "alegria-pg-command-v3";', "parent child queue"),
        (workflow, "opts.register_workflow::<AlegriaAdmittedEffectDagV2Workflow>()?;", "parent V4 workflow"),
        (pg, 'const COMMAND_TASK_QUEUE: &str = "alegria-pg-command-v3";', "PG command queue"),
        (pg, "command_options.register_workflow::<PgTruthReviewReconcileV4Workflow>()?;", "PG V4 child"),
        (operator, 'const PARENT_QUEUE: &str = "alegria-effect-dag-v4";', "operator parent history queue"),
        (operator, 'const CHILD_QUEUE: &str = "alegria-pg-command-v3";', "operator child history queue"),
    ]
    return [f"V4 cross-owner runtime mismatch: {label}" for source, needle, label in expected if needle not in source]


def main() -> int:
    compose = COMPOSE.read_text(encoding="utf-8")
    image = IMAGE.read_text(encoding="utf-8")
    pure_cargo = PURE_CARGO.read_text(encoding="utf-8")
    pg_cargo = PG_CARGO.read_text(encoding="utf-8")
    failures = check(compose, image, pure_cargo, pg_cargo)
    runtime_sources = tuple(path.read_text(encoding="utf-8") for path in [
        GATEWAY_RUNTIME, PARENT_RUNTIME, PARENT_WORKFLOWS, PG_RUNTIME, PG_OPERATOR_V4,
    ])
    failures.extend(check_v4_cross_owner_queue_contract(*runtime_sources))
    if not failures:
        for broken in [
            compose.replace('    profiles: ["v3-evaluation"]', "    profiles: []", 1),
            compose.replace(
                '    profiles: ["v3-evaluation", "v4-effect-evaluation", "v5-effect-evaluation"]',
                '    profiles: ["v4-effect-evaluation", "v5-effect-evaluation"]', 1,
            ),
            compose.replace("      ALLOW_PURE_DAG_ORCHESTRATOR: ${ALLOW_PURE_DAG_ORCHESTRATOR:-false}", ""),
            compose.replace('    profiles: ["v4-effect-evaluation"]', '    profiles: ["v3-evaluation"]'),
            compose.replace("      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4", "      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-pure-dag-v3"),
            compose.replace("      ALLOW_EFFECT_DAG_ORCHESTRATOR_V4: ${ALLOW_EFFECT_DAG_ORCHESTRATOR_V4:-false}", "      ALLOW_EFFECT_DAG_ORCHESTRATOR_V4: true"),
            compose.replace("      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn", ""),
            compose.replace("        mode: 0400", "        mode: 0444"),
            compose.replace('        uid: "65532"', '        uid: "0"'),
            compose.replace("    external: true", "    external: false", 1),
        ]:
            if not check(broken, image, pure_cargo, pg_cargo):
                failures.append("negative V3 topology contract self-test failed")
        if not check(compose, image.replace("COPY dependency-skeleton/app ./app", ""), pure_cargo, pg_cargo):
            failures.append("negative Docker dependency skeleton self-test failed")
        if not check(compose, image, pure_cargo + "\ngraph_neo4j = { path = 'bad' }", pg_cargo):
            failures.append("negative provider dependency contract self-test failed")
        for index, old in [
            (0, '"alegria-effect-dag-v4"'),
            (1, "Self::AdmittedEffectV4 => EFFECT_DAG_ORCHESTRATOR_V4_QUEUE"),
            (2, '"alegria-pg-command-v3"'),
            (3, "command_options.register_workflow::<PgTruthReviewReconcileV4Workflow>()?;"),
            (4, 'const PARENT_QUEUE: &str = "alegria-effect-dag-v4";'),
        ]:
            broken = list(runtime_sources)
            broken[index] = broken[index].replace(old, "BROKEN_QUEUE_CONTRACT", 1)
            if not check_v4_cross_owner_queue_contract(*broken):
                failures.append("negative V4 cross-owner queue contract self-test failed")
    if failures:
        for failure in failures:
            print(f"V3_TOPOLOGY: FAIL {failure}")
        return 1
    print("V3_TOPOLOGY: PASS (V3/V4/V5 opt-in only; not a production cutover)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
