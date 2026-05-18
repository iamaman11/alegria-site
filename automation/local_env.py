from __future__ import annotations

import os
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LOCAL_ENV_FILES = [
    ROOT / "infra" / "local" / "dev_db.env",
    ROOT / "infra" / "local" / "dev_runtime.env",
]
LEGACY_DEV_DATABASE_URLS = [
    "postgresql://postgres:postgres_password@localhost:5433/alegria",
    "postgresql://postgres:postgres_password@localhost:6432/alegria",
    "postgresql://postgres:postgres_password@localhost:5432/alegria",
]


def read_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


def load_local_env(paths: list[Path] | None = None) -> dict[str, str]:
    merged: dict[str, str] = {}
    for path in paths or DEFAULT_LOCAL_ENV_FILES:
        values = read_env_file(path)
        merged.update(values)
    for key, value in merged.items():
        os.environ.setdefault(key, value)
    return merged


def resolve_database_url(default: str | None = None) -> str:
    values = load_local_env()
    return (
        os.environ.get("DATABASE_URL")
        or os.environ.get("ALEGRIA_DATABASE_URL")
        or values.get("DATABASE_URL")
        or values.get("ALEGRIA_DATABASE_URL")
        or default
        or LEGACY_DEV_DATABASE_URLS[0]
    )


def inventory_probe_urls(database_url: str) -> list[str]:
    ordered: list[str] = []
    for candidate in [database_url, *LEGACY_DEV_DATABASE_URLS]:
        if candidate not in ordered:
            ordered.append(candidate)
    return ordered
