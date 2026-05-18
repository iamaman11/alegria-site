from __future__ import annotations

import subprocess
from dataclasses import dataclass


BASELINE_TABLES: list[tuple[str, str]] = [
    ("pipeline", "execution_run_blobs"),
    ("site", "page_nodes"),
    ("site", "publish_artifacts"),
]


@dataclass(frozen=True)
class BaselineCheckResult:
    database_url: str
    missing_tables: tuple[str, ...]

    @property
    def status(self) -> str:
        return "OK" if not self.missing_tables else "DRIFTED"

    @property
    def is_ready(self) -> bool:
        return not self.missing_tables

    def reason(self) -> str:
        if self.is_ready:
            return "baseline-ready local DB"
        return (
            "local DB is older than the SEO runtime migration baseline; missing tables: "
            + ", ".join(self.missing_tables)
        )


def _psql(database_url: str, sql: str) -> str:
    proc = subprocess.run(
        [
            "psql",
            database_url,
            "-t",
            "-A",
            "-c",
            " ".join(line.strip() for line in sql.strip().splitlines()),
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")
    return proc.stdout.strip()


def table_exists(database_url: str, schema: str, table: str) -> bool:
    return (
        _psql(
            database_url,
            f"""
            SELECT EXISTS (
                SELECT 1
                FROM information_schema.tables
                WHERE table_schema = '{schema}'
                  AND table_name = '{table}'
            );
            """,
        )
        == "t"
    )


def check_database_baseline(
    database_url: str, required_tables: list[tuple[str, str]] | None = None
) -> BaselineCheckResult:
    missing_tables = tuple(
        f"{schema}.{table}"
        for schema, table in required_tables or BASELINE_TABLES
        if not table_exists(database_url, schema, table)
    )
    return BaselineCheckResult(database_url=database_url, missing_tables=missing_tables)


def require_database_baseline(
    database_url: str, required_tables: list[tuple[str, str]] | None = None
) -> BaselineCheckResult:
    result = check_database_baseline(database_url, required_tables=required_tables)
    if not result.is_ready:
        raise RuntimeError(result.reason())
    return result
