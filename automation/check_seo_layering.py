#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def fail(message: str) -> int:
    print(f"FAIL {message}")
    return 1


def main() -> int:
    app_toml = read(RUST / "crates" / "seo_application" / "Cargo.toml")
    ports_toml = read(RUST / "crates" / "seo_ports" / "Cargo.toml")
    core_toml = read(RUST / "crates" / "seo_domain" / "Cargo.toml")
    steps_toml = read(RUST / "crates" / "seo_steps" / "Cargo.toml")
    steps_src = RUST / "crates" / "seo_steps" / "src"

    errors = []
    if 'infrastructure = { path = "../infrastructure" }' in app_toml:
        errors.append("seo_application must not depend on infrastructure")
    if 'infrastructure = { path = "../infrastructure" }' in ports_toml:
        errors.append("seo_ports must not depend on infrastructure")
    if 'seo_ports = { path = "../seo_ports" }' in steps_toml:
        errors.append("seo_steps must not depend on seo_ports")
    if 'seo_application = { path = "../seo_application" }' in steps_toml:
        errors.append("seo_steps must not depend on seo_application")
    if 'infrastructure = { path = "../infrastructure" }' in steps_toml:
        errors.append("seo_steps must not depend on infrastructure")

    forbidden_core = ["sqlx", "reqwest", "neo4rs", "qdrant-client", "temporalio-"]
    for token in forbidden_core:
        if token in core_toml:
            errors.append(f"seo_domain must not depend on `{token}`")

    for path in steps_src.glob("*.rs"):
        text = read(path)
        if "seo_ports::" in text:
            errors.append(f"seo_steps must not import seo_ports directly: {path.name}")
        if "seo_application::" in text:
            errors.append(f"seo_steps must not import seo_application directly: {path.name}")

    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1

    print("OK seo layering")
    return 0


if __name__ == "__main__":
    sys.exit(main())
