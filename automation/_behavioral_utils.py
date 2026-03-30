from __future__ import annotations

import os
import subprocess
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
POSTGRES_CONTAINER = os.environ.get("ALEGRIA_POSTGRES_CONTAINER", "alegria_postgres")
DATABASE_URL = os.environ.get(
    "ALEGRIA_DATABASE_URL",
    "postgresql://postgres:postgres_password@localhost:5433/alegria",
)
TEMPORAL_URL = os.environ.get("ALEGRIA_TEMPORAL_URL", "http://localhost:7233")


def run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> str:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    completed = subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        env=merged_env,
        check=True,
        text=True,
        capture_output=True,
    )
    return completed.stdout.strip()


def psql(sql: str) -> str:
    return run(
        [
            "docker",
            "exec",
            POSTGRES_CONTAINER,
            "psql",
            "-U",
            "postgres",
            "-d",
            "alegria",
            "-v",
            "ON_ERROR_STOP=1",
            "-At",
            "-c",
            sql,
        ]
    )


def cargo_run(manifest_path: str, *, env: dict[str, str]) -> str:
    return run(
        ["cargo", "run", "-q", "--manifest-path", manifest_path],
        cwd=ROOT / "app" / "rust",
        env=env,
    )


def random_key(prefix: str) -> str:
    return f"{prefix}_{uuid.uuid4().hex}"
