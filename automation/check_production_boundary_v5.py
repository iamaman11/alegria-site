#!/usr/bin/env python3
"""Read-only production-owner boundary evidence; NEVER promotes a release.

Normal reporting distinguishes opt-in experiments from production routing and
returns zero so that a pending cutover does not invalidate an unrelated green
PR gate. ``--require-production`` is a strict acceptance check: until the
default Compose topology actually isolates the Temporal Workflow owner from
PostgreSQL, graph, vector and provider credentials it exits nonzero.

This is deliberately a small Compose *shape* checker, not an arbitrary YAML
parser, a live network attestation, or an authorization to start containers.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN_PARENT_ENV = frozenset({
    "DATABASE_URL", "PG_EFFECT_WORKER_DATABASE_URL_FILE", "DB_HOST", "DB_USER",
    "DB_PASSWORD", "NEO4J_URI", "NEO4J_USER", "NEO4J_PASSWORD", "QDRANT_URL",
    "VOYAGE_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY",
    "GOOGLE_API_KEY", "DATAFORSEO_PASSWORD", "SEO_LLM_PROVIDER",
    "SEO_TRUTH_LLM_PROVIDER", "CLOUDFLARE_API_TOKEN",
})
FORBIDDEN_PG_ENV = frozenset({
    "DATABASE_URL", "NEO4J_URI", "QDRANT_URL", "VOYAGE_API_KEY",
    "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY",
    "GOOGLE_API_KEY", "CLOUDFLARE_API_TOKEN",
})
PURE_CONTROL_NETWORK = "temporal_pure_control"
PG_EFFECT_DATA_NETWORK = "temporal_pg_effect_data"
V7_PURE_CANDIDATE = "temporal-pure-release-candidate"
V7_PURE_OVERLAY = ROOT / "infra/temporal/compose.v7-isolated.yml"
V7_PURE_DOCKERFILE = ROOT / "infra/temporal/Dockerfile.v7-pure"


def service_sections(compose: str) -> dict[str, str]:
    """Extract only services; never confuse root volumes/secrets with them."""
    lines = compose.splitlines(keepends=True)
    try:
        start = next(i + 1 for i, line in enumerate(lines) if line.rstrip() == "services:")
    except StopIteration:
        return {}
    end = len(lines)
    for index in range(start, len(lines)):
        line = lines[index]
        if line.strip() and not line.startswith((" ", "\t", "#")):
            end = index
            break
    section = "".join(lines[start:end])
    marker = list(re.finditer(r"(?m)^  ([a-z][a-z0-9_-]*):\s*(?:#.*)?$", section))
    return {
        match.group(1): section[match.end(): marker[i + 1].start() if i + 1 < len(marker) else len(section)]
        for i, match in enumerate(marker)
    }


def environment(body: str) -> set[str]:
    """Recognize only service environment mapping keys, never comment text."""
    return set(re.findall(r"(?m)^      ([A-Z][A-Z0-9_]*):", body))


def is_opt_in(body: str) -> bool:
    return bool(re.search(r"(?m)^    profiles:\s*\S", body))


def service_networks(body: str) -> list[str]:
    match = re.search(r"(?ms)^    networks:\n(.*?)(?=^    \S|\Z)", body)
    if not match:
        return ["default"]
    return re.findall(r"(?m)^      - ([a-z][a-z0-9_-]*)\s*$", match.group(1))


def service_dependencies(body: str) -> list[str]:
    match = re.search(r"(?ms)^    depends_on:\n(.*?)(?=^    \S|\Z)", body)
    if not match:
        return []
    return re.findall(r"(?m)^      ([a-z][a-z0-9_-]*):\s*$", match.group(1))


def unsafe_wiring(body: str) -> list[str]:
    forbidden = []
    for setting in ("env_file:", "privileged: true", "network_mode:"):
        if re.search(rf"(?m)^    {re.escape(setting)}", body):
            forbidden.append(setting)
    return forbidden


def isolated_native_image(body: str, target: str, package: str) -> bool:
    """A default release may not silently link the provider-heavy legacy bin."""
    required = (
        "    build:",
        "      dockerfile: ./infra/temporal/Dockerfile.v3",
        f"      target: {target}",
        f"        OWNER_PACKAGE: {package}",
    )
    return all(line in body.splitlines() for line in required)


def check_v7_pure_candidate(overlay: str, dockerfile: str) -> dict[str, Any]:
    """Fail-closed opt-in candidate evidence, never a release authorization.

    Unlike the base Compose, this file is not loaded at all by default. Even
    when explicitly merged, its own profile and runtime gate must be enabled.
    The exact-SHA isolated Cargo context is required at Compose parse time.
    """
    issues: list[str] = []
    services = service_sections(overlay)
    if set(services) != {V7_PURE_CANDIDATE}:
        issues.append("V7 overlay must define only its isolated pure candidate service")
    for forbidden_root in ("networks:", "secrets:", "volumes:"):
        if re.search(rf"(?m)^{re.escape(forbidden_root)}", overlay):
            issues.append(f"V7 overlay must not change default root {forbidden_root}")
    body = services.get(V7_PURE_CANDIDATE, "")
    required = (
        '    profiles: ["v7-production-isolation-evaluation"]',
        "    build:",
        "      context: ${V7_PURE_BUILD_CONTEXT:?exact-SHA pure build context required}",
        "      dockerfile: ./infra/temporal/Dockerfile.v7-pure",
        '    restart: "no"',
        "    read_only: true",
        '    cap_drop: ["ALL"]',
        "    security_opt:",
        "      - no-new-privileges:true",
        "      TEMPORAL_URL: http://temporal-server:7233",
        "      TEMPORAL_NAMESPACE: default",
        "      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v5",
        "      WORKER_BUILD_ID: ${V7_SOURCE_SHA:?exact committed 40-hex Git SHA required}",
        '      TEMPORAL_WORKER_VERSIONING: "false"',
        '      ALLOW_PURE_DAG_ORCHESTRATOR: "false"',
        '      ALLOW_EFFECT_DAG_ORCHESTRATOR_V4: "false"',
        "      ALLOW_EFFECT_DAG_ORCHESTRATOR_V5: ${V7_ENABLE_PURE_CANDIDATE:-false}",
    )
    for line in required:
        if line not in body.splitlines():
            issues.append(f"V7 pure candidate missing mandatory isolation/control: {line.strip()}")
    if service_networks(body) != [PURE_CONTROL_NETWORK]:
        issues.append("V7 pure candidate has non-Temporal or default network access")
    if service_dependencies(body) != ["temporal-server"]:
        issues.append("V7 pure candidate depends on a provider or non-Temporal service")
    expected_environment = {
        "TEMPORAL_URL", "TEMPORAL_NAMESPACE", "TEMPORAL_ORCHESTRATOR_TASK_QUEUE",
        "WORKER_BUILD_ID", "TEMPORAL_WORKER_VERSIONING",
        "ALLOW_PURE_DAG_ORCHESTRATOR", "ALLOW_EFFECT_DAG_ORCHESTRATOR_V4",
        "ALLOW_EFFECT_DAG_ORCHESTRATOR_V5", "RUST_LOG",
    }
    if environment(body) != expected_environment:
        issues.append("V7 pure candidate environment differs from exact provider-free allowlist")
    for forbidden in (
        "    secrets:", "    volumes:", "    ports:", "    devices:",
        "    extra_hosts:", "    links:", "    pid:", "    ipc:",
    ):
        if forbidden in body.splitlines():
            issues.append(f"V7 pure candidate has forbidden host/secret wiring: {forbidden.strip()}")
    for entry in unsafe_wiring(body):
        issues.append(f"V7 pure candidate has unsafe wiring: {entry}")

    dockerfile_requirements = (
        "FROM rust:1.100.0-slim-bookworm AS pure-builder",
        "COPY dependency-skeleton/app ./app",
        "COPY app ./app",
        "    -p temporal_orchestrator --bin temporal_orchestrator --features legacy-workflows",
        "USER 65532:65532",
        "COPY --from=pure-builder /app/app/rust/target/release/temporal_orchestrator \\",
        'ENTRYPOINT ["/usr/local/bin/temporal_orchestrator"]',
    )
    for line in dockerfile_requirements:
        if line not in dockerfile.splitlines():
            issues.append(f"V7 native image lacks pure owner build/runtime contract: {line.strip()}")
    if dockerfile.count("-p temporal_orchestrator --bin temporal_orchestrator --features legacy-workflows") != 2:
        issues.append("both V7 native build layers must enable frozen V5 Workflow History")
    for foreign in ("temporal_pg_effects", "temporal_worker", "vector_qdrant", "graph_neo4j"):
        if foreign in dockerfile:
            issues.append(f"V7 pure image mentions forbidden provider/legacy executable: {foreign}")
    return {
        "boundary": "PASS" if not issues else "FAIL",
        "production_release_authorized": False,
        "evidence_scope": "opt-in overlay, exact source-scoped native image shape only; no live deploy or History attestation",
        "issues": issues,
    }


def check(compose: str) -> dict[str, Any]:
    services = service_sections(compose)
    issues: list[str] = []
    experiments: list[str] = []

    if not re.search(
        rf"(?m)^networks:\n  {PURE_CONTROL_NETWORK}:\n    internal: true\s*$", compose
    ):
        issues.append("experimental isolated Temporal control network is missing or not internal")
    if not re.search(
        rf"(?m)^  {PG_EFFECT_DATA_NETWORK}:\n    internal: true\s*$", compose
    ):
        issues.append("experimental isolated PostgreSQL Activity data network is missing or not internal")
    temporal_server = services.get("temporal-server", "")
    if set(service_networks(temporal_server)) != {"default", PURE_CONTROL_NETWORK, PG_EFFECT_DATA_NETWORK}:
        issues.append("Temporal server must bridge default, pure control and PG Activity networks")

    for name in ("temporal-effect-dag-v4", "temporal-effect-dag-v5", "temporal-orchestrator-v3"):
        body = services.get(name)
        if body is None:
            issues.append(f"missing optional provider-free experiment: {name}")
            continue
        if not is_opt_in(body):
            issues.append(f"experimental owner accidentally routes in default Compose: {name}")
        leaked = sorted(environment(body) & FORBIDDEN_PARENT_ENV)
        if leaked:
            issues.append(f"experimental Temporal owner has forbidden credentials {name}: {leaked}")
        if service_networks(body) != [PURE_CONTROL_NETWORK]:
            issues.append(f"experimental Temporal owner can reach a provider network: {name}")
        if service_dependencies(body) != ["temporal-server"]:
            issues.append(f"experimental Temporal owner depends on non-Temporal services: {name}")
        unsafe = unsafe_wiring(body)
        if unsafe:
            issues.append(f"experimental Temporal owner has unsafe host/environment wiring {name}: {unsafe}")
        experiments.append(name)

    pg_experiment = services.get("temporal-pg-effects-v3")
    if pg_experiment is None:
        issues.append("missing isolated opt-in PostgreSQL Activity owner")
    else:
        if not is_opt_in(pg_experiment):
            issues.append("experimental PostgreSQL owner is unexpectedly default-enabled")
        if "PG_EFFECT_WORKER_DATABASE_URL_FILE" not in environment(pg_experiment):
            issues.append("experimental PostgreSQL owner has no file-scoped Worker DSN")
        leaked = sorted(environment(pg_experiment) & FORBIDDEN_PG_ENV)
        if leaked:
            issues.append(f"experimental PostgreSQL owner has forbidden credentials: {leaked}")
        if PURE_CONTROL_NETWORK in service_networks(pg_experiment):
            issues.append("experimental PostgreSQL Activity owner can join isolated Temporal parent network")
        if service_networks(pg_experiment) != [PG_EFFECT_DATA_NETWORK]:
            issues.append("experimental PostgreSQL Activity owner must have only its internal PG data network")
        experiments.append("temporal-pg-effects-v3")

    for name in ("postgres", "pgbouncer"):
        body = services.get(name)
        if body is not None and set(service_networks(body)) != {"default", PG_EFFECT_DATA_NETWORK}:
            issues.append(f"experimental PostgreSQL endpoint has wrong network isolation: {name}")

    for name in ("postgres", "pgbouncer", "neo4j", "qdrant", "outbox-worker", "temporal-worker"):
        body = services.get(name)
        if body is not None and PURE_CONTROL_NETWORK in service_networks(body):
            issues.append(f"experimental provider store/legacy Worker has pure control network access: {name}")
    for name in ("neo4j", "qdrant", "outbox-worker", "temporal-worker"):
        body = services.get(name)
        if body is not None and PG_EFFECT_DATA_NETWORK in service_networks(body):
            issues.append(f"experimental foreign provider/legacy Worker can reach PG Activity network: {name}")

    default_parents = [
        (name, body)
        for name, body in services.items()
        if name.startswith("temporal-")
        and name not in {
            "temporal-server", "temporal-ui", "temporal-admin-tools",
            "temporal-schema", "temporal-namespace",
        }
        and "pg-effects" not in name
        and not is_opt_in(body)
    ]
    for name, body in default_parents:
        if not isolated_native_image(body, "pure", "temporal_orchestrator"):
            issues.append(f"production Temporal parent is not a source-scoped pure native image: {name}")
        leaked = sorted(environment(body) & FORBIDDEN_PARENT_ENV)
        if leaked:
            issues.append(f"production Temporal process retains owner/provider credentials {name}: {leaked}")
        if service_networks(body) != [PURE_CONTROL_NETWORK]:
            issues.append(f"production Temporal process has a provider/default network path: {name}")
        if service_dependencies(body) != ["temporal-server"]:
            issues.append(f"production Temporal process depends on non-Temporal services: {name}")
        unsafe = unsafe_wiring(body)
        if unsafe:
            issues.append(f"production Temporal process has unsafe host/environment wiring {name}: {unsafe}")
    has_dag_parent = any(
        "TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v" in body
        for _name, body in default_parents
    )
    if not has_dag_parent:
        issues.append("no default-enabled provider-free effect DAG Temporal parent")

    default_pg = [
        (name, body) for name, body in services.items()
        if "pg-effects" in name and not is_opt_in(body)
    ]
    if not default_pg:
        issues.append("no default-enabled separate PostgreSQL effect Activity owner")
    for name, body in default_pg:
        if not isolated_native_image(body, "pg", "temporal_pg_effects"):
            issues.append(f"production PostgreSQL Activity owner is not a source-scoped PG native image: {name}")
        if "PG_EFFECT_WORKER_DATABASE_URL_FILE" not in environment(body):
            issues.append(f"production PostgreSQL owner lacks file-scoped least-privilege DSN: {name}")
        if service_networks(body) != [PG_EFFECT_DATA_NETWORK]:
            issues.append(f"production PostgreSQL owner has a provider/default network path: {name}")
        if "    secrets:" not in body or "      - source: pg_effect_worker_dsn" not in body:
            issues.append(f"production PostgreSQL owner lacks external secret mount: {name}")
        leaked = sorted(environment(body) & FORBIDDEN_PG_ENV)
        if leaked:
            issues.append(f"production PostgreSQL owner retains foreign credentials {name}: {leaked}")
        unsafe = unsafe_wiring(body)
        if unsafe:
            issues.append(f"production PostgreSQL owner has unsafe host/environment wiring {name}: {unsafe}")

    experiment_issues = [issue for issue in issues if issue.startswith((
        "missing optional", "experimental ", "missing isolated opt-in", "Temporal server"
    )) or "pure control network access" in issue]
    production_issues = [issue for issue in issues if issue not in experiment_issues]
    return {
        "schema_version": 1,
        "experimental_boundary": "PASS" if not experiment_issues else "FAIL",
        "production_static_boundary": "PASS" if not production_issues else "NO_GO",
        "production_release_authorized": False,
        "evidence_scope": "Compose topology only; no server mTLS, historical drain, RLS or live attestation",
        "experimental_services": experiments,
        "isolated_pg_effect_network": PG_EFFECT_DATA_NETWORK,
        "default_temporal_processes": [name for name, _ in default_parents],
        "default_pg_owner_processes": [name for name, _ in default_pg],
        "issues": issues,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compose", type=Path, default=ROOT / "docker-compose.yml")
    parser.add_argument("--candidate-overlay", type=Path, default=V7_PURE_OVERLAY)
    parser.add_argument("--candidate-dockerfile", type=Path, default=V7_PURE_DOCKERFILE)
    parser.add_argument("--require-production", action="store_true")
    args = parser.parse_args()
    result = check(args.compose.read_text(encoding="utf-8"))
    candidate = check_v7_pure_candidate(
        args.candidate_overlay.read_text(encoding="utf-8"),
        args.candidate_dockerfile.read_text(encoding="utf-8"),
    )
    result["v7_pure_isolation_candidate"] = candidate
    print(json.dumps(result, indent=2, sort_keys=True))
    if result["experimental_boundary"] != "PASS":
        return 1
    if candidate["boundary"] != "PASS":
        return 1
    if args.require_production and result["production_static_boundary"] != "PASS":
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
