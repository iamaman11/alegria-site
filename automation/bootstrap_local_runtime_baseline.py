#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import ParseResult, urlparse, urlunparse

from local_db_baseline import require_database_baseline


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_SQL = ROOT / "app" / "db" / "schema.sql"
MIGRATIONS_DIR = ROOT / "app" / "db" / "migrations"


def psql(database_url: str, *args: str, input_text: str | None = None) -> None:
    proc = subprocess.run(
        ["psql", database_url, *args],
        input=input_text,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")


def psql_scalar(database_url: str, sql: str) -> str:
    proc = subprocess.run(
        ["psql", database_url, "-t", "-A", "-c", sql],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")
    return proc.stdout.strip()


def parse_target(database_url: str) -> tuple[ParseResult, str]:
    parsed = urlparse(database_url)
    db_name = parsed.path.lstrip("/")
    if not db_name:
        raise RuntimeError("target DATABASE_URL must include database name")
    return parsed, db_name


def admin_url(parsed: ParseResult, admin_db: str) -> str:
    return urlunparse(parsed._replace(path=f"/{admin_db}"))


def quote_ident(value: str) -> str:
    return '"' + value.replace('"', '""') + '"'


def quote_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def database_exists(admin_database_url: str, db_name: str) -> bool:
    return (
        psql_scalar(
            admin_database_url,
            f"SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = {quote_literal(db_name)});",
        )
        == "t"
    )


def drop_database(admin_database_url: str, db_name: str) -> None:
    psql(
        admin_database_url,
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        (
            "SELECT pg_terminate_backend(pid) "
            "FROM pg_stat_activity "
            f"WHERE datname = {quote_literal(db_name)} AND pid <> pg_backend_pid();"
        ),
    )
    psql(
        admin_database_url,
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        f"DROP DATABASE IF EXISTS {quote_ident(db_name)};",
    )


def create_database(admin_database_url: str, db_name: str) -> None:
    psql(
        admin_database_url,
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        f"CREATE DATABASE {quote_ident(db_name)};",
    )


def apply_schema_and_migrations(database_url: str) -> None:
    psql(database_url, "-v", "ON_ERROR_STOP=1", "-f", str(SCHEMA_SQL))
    for path in sorted(MIGRATIONS_DIR.glob("*.sql")):
        psql(database_url, "-v", "ON_ERROR_STOP=1", "-f", str(path))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database-url", required=True)
    parser.add_argument("--admin-database", default="postgres")
    parser.add_argument("--recreate", action="store_true")
    args = parser.parse_args()

    if shutil.which("psql") is None:
        print("BOOTSTRAP_LOCAL_RUNTIME_BASELINE: FAILED")
        print("- `psql` is not available")
        return 1

    parsed, db_name = parse_target(args.database_url)
    admin_database_url = admin_url(parsed, args.admin_database)

    try:
        exists = database_exists(admin_database_url, db_name)
        if exists and not args.recreate:
            raise RuntimeError(
                f"target database `{db_name}` already exists; rerun with --recreate or choose a new name"
            )
        if exists:
            drop_database(admin_database_url, db_name)
        create_database(admin_database_url, db_name)
        apply_schema_and_migrations(args.database_url)
        result = require_database_baseline(args.database_url)
    except Exception as exc:
        print("BOOTSTRAP_LOCAL_RUNTIME_BASELINE: FAILED")
        print(f"- {exc}")
        return 1

    print("BOOTSTRAP_LOCAL_RUNTIME_BASELINE: OK")
    print(f"- database_url={result.database_url}")
    print(f"- status={result.status}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
