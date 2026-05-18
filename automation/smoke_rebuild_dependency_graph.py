#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
LOCAL_ENV = ROOT / "infra" / "local" / "dev_db.env"
MIGRATION = ROOT / "app" / "db" / "migrations" / "20260515_0004_rebuild_dependency_graph_contract.sql"


def load_env() -> dict[str, str]:
    env = os.environ.copy()
    for line in LOCAL_ENV.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        env[key] = value
    return env


def main() -> int:
    env = load_env()
    database_url = env.get("DATABASE_URL")
    if not database_url:
        print("REBUILD_DEPENDENCY_GRAPH_SMOKE: FAILED")
        print("- DATABASE_URL missing in local env")
        return 1

    migrate = subprocess.run(
        ["psql", database_url, "-v", "ON_ERROR_STOP=1", "-f", str(MIGRATION)],
        cwd=ROOT,
        env=env,
        check=False,
        text=True,
        capture_output=True,
    )
    if migrate.returncode != 0:
        print("REBUILD_DEPENDENCY_GRAPH_SMOKE: FAILED")
        if migrate.stdout.strip():
            print(migrate.stdout.rstrip())
        if migrate.stderr.strip():
            print(migrate.stderr.rstrip())
        return migrate.returncode

    tests = subprocess.run(
        [
            "bash",
            "-lc",
            "cd app/rust && SQLX_OFFLINE=true cargo test -q -p infrastructure --lib",
        ],
        cwd=ROOT,
        env=env,
        check=False,
        text=True,
        capture_output=True,
    )
    if tests.returncode != 0:
        print("REBUILD_DEPENDENCY_GRAPH_SMOKE: FAILED")
        if tests.stdout.strip():
            print(tests.stdout.rstrip())
        if tests.stderr.strip():
            print(tests.stderr.rstrip())
        return tests.returncode

    print("REBUILD_DEPENDENCY_GRAPH_SMOKE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
