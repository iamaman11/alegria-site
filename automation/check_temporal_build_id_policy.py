#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
MAIN = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "main.rs"
COMPOSE = ROOT / "docker-compose.yml"
BUILD_MODES = ROOT / "docs" / "OPS_TEMPORAL_BUILD_MODES.md"
RUNBOOK = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"


def must_contain(path: Path, needle: str) -> str | None:
    if not path.exists():
        return f"missing file: {path.relative_to(ROOT)}"
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures = [
        must_contain(MAIN, "WORKER_BUILD_ID"),
        must_contain(WORKFLOWS, "build_worker_options"),
        must_contain(WORKFLOWS, "WORKER_BUILD_ID"),
        must_contain(COMPOSE, "WORKER_BUILD_ID"),
        must_contain(BUILD_MODES, "Когда обязателен новый `workflow type`"),
        must_contain(BUILD_MODES, "Rollback / drain policy"),
        must_contain(RUNBOOK, "Rollback/drain policy"),
    ]
    failures = [f for f in failures if f]
    if failures:
        print("TEMPORAL_BUILD_ID_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("TEMPORAL_BUILD_ID_POLICY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
