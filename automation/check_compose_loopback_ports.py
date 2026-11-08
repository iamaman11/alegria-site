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


def main() -> int:
    example = 'services:\n  example:\n    ports:\n      - "127.0.0.1:5433:5432"\n'
    bad = 'services:\n  example:\n    ports:\n      - "5433:5432"\n'
    if check(example) != (1, []) or not check(bad)[1]:
        print("COMPOSE_LOOPBACK_PORTS: FAILED self-test", file=sys.stderr)
        return 1
    count, errors = check((ROOT / "docker-compose.yml").read_text(encoding="utf-8"))
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
