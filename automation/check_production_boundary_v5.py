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


def check(compose: str) -> dict[str, Any]:
    services = service_sections(compose)
    issues: list[str] = []
    experiments: list[str] = []

    if not re.search(
        rf"(?m)^networks:\n  {PURE_CONTROL_NETWORK}:\n    internal: true\s*$", compose
    ):
        issues.append("experimental isolated Temporal control network is missing or not internal")
    temporal_server = services.get("temporal-server", "")
    if set(service_networks(temporal_server)) != {"default", PURE_CONTROL_NETWORK}:
        issues.append("Temporal server must bridge default and isolated pure control networks")

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
        experiments.append("temporal-pg-effects-v3")

    for name in ("postgres", "pgbouncer", "neo4j", "qdrant", "outbox-worker", "temporal-worker"):
        body = services.get(name)
        if body is not None and PURE_CONTROL_NETWORK in service_networks(body):
            issues.append(f"experimental provider store/legacy Worker has pure control network access: {name}")

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
        if "PG_EFFECT_WORKER_DATABASE_URL_FILE" not in environment(body):
            issues.append(f"production PostgreSQL owner lacks file-scoped least-privilege DSN: {name}")
        leaked = sorted(environment(body) & FORBIDDEN_PG_ENV)
        if leaked:
            issues.append(f"production PostgreSQL owner retains foreign credentials {name}: {leaked}")

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
        "default_temporal_processes": [name for name, _ in default_parents],
        "default_pg_owner_processes": [name for name, _ in default_pg],
        "issues": issues,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compose", type=Path, default=ROOT / "docker-compose.yml")
    parser.add_argument("--require-production", action="store_true")
    args = parser.parse_args()
    result = check(args.compose.read_text(encoding="utf-8"))
    print(json.dumps(result, indent=2, sort_keys=True))
    if result["experimental_boundary"] != "PASS":
        return 1
    if args.require_production and result["production_static_boundary"] != "PASS":
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
