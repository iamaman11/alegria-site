#!/usr/bin/env python3
"""Fail closed when Alegria Compose publishes a service beyond loopback."""

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
SERVICE = re.compile(r"^  ([a-z][a-z0-9_-]*):$")
BINDING = re.compile(r'^      - "127\.0\.0\.1:[0-9]+:[0-9]+"(?:\s+#.*)?$')


def check(text: str) -> tuple[int, list[str]]:
    service = "<unknown>"
    in_ports = False
    checked = 0
    failures: list[str] = []
    for number, line in enumerate(text.splitlines(), 1):
        match = SERVICE.fullmatch(line)
        if match:
            service = match.group(1)
            in_ports = False
        if line == "    ports:":
            in_ports = True
            continue
        if not in_ports:
            continue
        if not line.startswith("      "):
            in_ports = False
            continue
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not BINDING.fullmatch(line):
            failures.append(
                f"line {number}, service {service}: unsafe or unsupported port: {line.strip()}"
            )
        else:
            checked += 1
    return checked, failures



def check_temporal_graph_route(text: str) -> list[str]:
    """The Temporal worker must reach the Compose Neo4j service, not itself."""
    match = re.search(
        r"(?ms)^  temporal-worker:\n(.*?)(?=^  [a-z][a-z0-9_-]*:|\Z)",
        text,
    )
    if not match:
        return ["missing temporal-worker service"]
    worker = match.group(1)
    lines = worker.splitlines()
    missing = [
        setting
        for setting in (
            "      NEO4J_URI: neo4j:7687",
            "      NEO4J_USER: neo4j",
            "      NEO4J_PASSWORD: neo4j_password",
        )
        if setting not in lines
    ]
    dependencies = worker.split("    depends_on:\n", 1)
    if len(dependencies) != 2 or "      neo4j:" not in dependencies[1].split("    restart:", 1)[0]:
        missing.append("healthy Neo4j dependency")
    return [f"temporal-worker missing Compose Neo4j contract: {value.strip()}" for value in missing]


def check_outbox_voyage_route(text: str) -> list[str]:
    """The existing materializer needs the Voyage settings used by Qdrant."""
    match = re.search(
        r"(?ms)^  outbox-worker:\n(.*?)(?=^  [a-z][a-z0-9_-]*:|\Z)",
        text,
    )
    if not match:
        return ["missing outbox-worker service"]
    lines = match.group(1).splitlines()
    return [
        f"outbox-worker missing provider configuration: {value.strip()}"
        for value in (
            "      VOYAGE_API_KEY: ${VOYAGE_API_KEY:-}",
            "      VOYAGE_MODEL: ${VOYAGE_MODEL:-voyage-4-large}",
        )
        if value not in lines
    ]

def check_release_volume(text: str) -> list[str]:
    """Keep immutable releases in Docker storage, not a Windows bind mount."""
    required_mount = "      - static_site_releases:/app/app/rust/dist"
    volume_definition = "  static_site_releases:"
    worker = re.search(r"(?ms)^  temporal-worker:\n(.*?)(?=^  [a-z][a-z0-9_-]*:|\Z)", text)
    failures = []
    if not worker or required_mount not in worker.group(1).splitlines():
        failures.append("temporal-worker must persist dist using static_site_releases named volume")
    if volume_definition not in text.splitlines():
        failures.append("static_site_releases top-level volume must be declared")
    return failures

def main() -> int:
    example = 'services:\n  example:\n    ports:\n      - "127.0.0.1:5433:5432"\n'
    bad = 'services:\n  example:\n    ports:\n      - "5433:5432"\n'
    sample_graph_route = ("services:\n  temporal-worker:\n    environment:\n"
                          "      NEO4J_URI: neo4j:7687\n      NEO4J_USER: neo4j\n"
                          "      NEO4J_PASSWORD: neo4j_password\n    depends_on:\n"
                          "      neo4j:\n        condition: service_healthy\n")
    if (check_temporal_graph_route(sample_graph_route)
            or not check_temporal_graph_route(sample_graph_route.replace("neo4j:7687", "127.0.0.1:7687"))):
        print("COMPOSE_LOOPBACK_PORTS: FAILED Neo4j route self-test", file=sys.stderr)
        return 1
    voyage_example = ('services:\n  outbox-worker:\n    environment:\n'
                      '      VOYAGE_API_KEY: ${VOYAGE_API_KEY:-}\n'
                      '      VOYAGE_MODEL: ${VOYAGE_MODEL:-voyage-4-large}\n')
    if (check_outbox_voyage_route(voyage_example)
            or not check_outbox_voyage_route(voyage_example.replace('VOYAGE_API_KEY', 'MISSING_API_KEY'))):
        print("COMPOSE_LOOPBACK_PORTS: FAILED Voyage config self-test", file=sys.stderr)
        return 1
    release_example = ('services:\n  temporal-worker:\n    volumes:\n'
                       '      - static_site_releases:/app/app/rust/dist\n'
                       'volumes:\n  static_site_releases:\n')
    if (check_release_volume(release_example)
            or not check_release_volume(release_example.replace('static_site_releases:/app', './app/rust/dist:/app'))):
        print("COMPOSE_LOOPBACK_PORTS: FAILED release volume self-test", file=sys.stderr)
        return 1
    if check(example) != (1, []) or not check(bad)[1]:
        print("COMPOSE_LOOPBACK_PORTS: FAILED self-test", file=sys.stderr)
        return 1
    compose_text = (ROOT / "docker-compose.yml").read_text(encoding="utf-8")
    count, errors = check(compose_text)
    errors.extend(check_temporal_graph_route(compose_text))
    errors.extend(check_outbox_voyage_route(compose_text))
    errors.extend(check_release_volume(compose_text))
    if not count:
        errors.append("no explicit loopback port mappings found")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print("COMPOSE_LOOPBACK_PORTS: FAILED", file=sys.stderr)
        return 1
    print(f"COMPOSE_LOOPBACK_PORTS: PASS ({count} bindings)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
