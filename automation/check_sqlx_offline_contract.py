#!/usr/bin/env python3
"""Require SQLx offline cache and offline mode at the actual Cargo CI owner."""
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
WORKSPACE_CARGO = WORKSPACE / "Cargo.toml"
SQLX_CACHE = WORKSPACE / ".sqlx"
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
TEMPORAL_DOCKERFILE = ROOT / "app" / "rust" / "services" / "temporal" / "Dockerfile"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def cargo_step_failures(workflow: str) -> list[str]:
    """Inspect step-local env, not a removed shell command or job-wide keyword."""
    failures: list[str] = []
    if "  architecture-and-rust:\n" not in workflow or "\n  rust-security:\n" not in workflow:
        return ["missing canonical Rust owner in ci.yml"]
    rust = workflow.split("  architecture-and-rust:\n", 1)[1].split("\n  rust-security:\n", 1)[0]
    expected = (
        ("Rust workspace tests (includes compile)", "run: cargo test --workspace --locked"),
        ("Compile e2e-feature integration harness", "run: cargo test -p integration_harness --features e2e --no-run --locked"),
        ("Rust migration and fact-verifier parity", "cargo run -q --locked --manifest-path app/rust/services/cli_tools/Cargo.toml --"),
    )
    for name, command in expected:
        marker = f"      - name: {name}\n"
        if marker not in rust:
            failures.append(f"missing Rust owner step: {name}")
            continue
        step = rust.split(marker, 1)[1].split("\n      - name:", 1)[0]
        if 'SQLX_OFFLINE: "true"' not in step:
            failures.append(f"SQLX_OFFLINE not enabled in Rust step: {name}")
        if command not in step:
            failures.append(f"missing Cargo command in Rust step: {name}")
    return failures


def main() -> int:
    failures: list[str] = []
    if not SQLX_CACHE.exists():
        failures.append("missing app/rust/.sqlx cache directory")
    elif not list(SQLX_CACHE.glob("query-*.json")):
        failures.append("app/rust/.sqlx has no query cache files")

    for item in (
        must_contain(WORKSPACE_CARGO, '"macros"'),
        must_contain(TEMPORAL_DOCKERFILE, "ENV SQLX_OFFLINE=true"),
    ):
        if item is not None:
            failures.append(item)
    if not CI_WORKFLOW.is_file():
        failures.append("missing canonical .github/workflows/ci.yml")
    else:
        failures.extend(cargo_step_failures(CI_WORKFLOW.read_text(encoding="utf-8")))

    if failures:
        print("SQLX_OFFLINE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SQLX_OFFLINE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
