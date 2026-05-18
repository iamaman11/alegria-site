#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
from urllib.parse import urlparse
import socket
import shutil
import subprocess
import sys
from typing import Any

from local_env import inventory_probe_urls, resolve_database_url


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"

SQL_TOTAL = """
SELECT count(*)::bigint
FROM pipeline.execution_runs
WHERE workflow_type = 'seo_site_build'
"""

SQL_DONE = """
SELECT count(*)::bigint
FROM pipeline.execution_runs
WHERE workflow_type = 'seo_site_build'
  AND status = 'done'
"""

SQL_OPEN = """
SELECT count(*)::bigint
FROM pipeline.execution_runs
WHERE workflow_type = 'seo_site_build'
  AND status <> 'done'
"""

SQL_SAMPLE = """
SELECT run_id::text
FROM pipeline.execution_runs
WHERE workflow_type = 'seo_site_build'
ORDER BY created_at DESC
LIMIT 10
"""


def read_artifact() -> dict[str, Any]:
    return json.loads(ARTIFACT.read_text(encoding="utf-8"))


def write_artifact(payload: dict[str, Any]) -> None:
    ARTIFACT.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def utc_now() -> str:
    return subprocess.run(
        ["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], capture_output=True, text=True, check=True
    ).stdout.strip()


def tcp_probe(database_url: str) -> dict[str, Any]:
    parsed = urlparse(database_url)
    host = parsed.hostname or "localhost"
    port = parsed.port or 5432
    result: dict[str, Any] = {
        "database_url": database_url,
        "host": host,
        "port": port,
        "tcp_status": "unknown",
        "inventory_status": "not_attempted",
        "last_error": None,
    }
    try:
        with socket.create_connection((host, port), timeout=1.5):
            result["tcp_status"] = "listening"
    except OSError as exc:
        result["tcp_status"] = "connect_failed"
        result["last_error"] = str(exc)
    return result


def capture_environment_probe(database_url: str) -> dict[str, Any]:
    return {
        "captured_at": utc_now(),
        "candidates": [tcp_probe(candidate) for candidate in inventory_probe_urls(database_url)],
    }


def run_psql(database_url: str, sql: str) -> list[str]:
    cmd = [
        "psql",
        database_url,
        "-t",
        "-A",
        "-c",
        " ".join(line.strip() for line in sql.strip().splitlines()),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")
    return [line.strip() for line in proc.stdout.splitlines() if line.strip()]


def collect_inventory(database_url: str) -> dict[str, Any]:
    return {
        "total_runs": int(run_psql(database_url, SQL_TOTAL)[0]),
        "done_runs": int(run_psql(database_url, SQL_DONE)[0]),
        "open_runs": int(run_psql(database_url, SQL_OPEN)[0]),
        "sample_run_ids": run_psql(database_url, SQL_SAMPLE),
    }


def main() -> int:
    if shutil.which("psql") is None:
        print("SEO_LEGACY_REPLAY_INVENTORY: FAILED")
        print("- `psql` is not available")
        return 1

    database_url = resolve_database_url()

    artifact = read_artifact()
    inventory = artifact.setdefault("history_source", {}).setdefault("inventory", {})
    environment_probe = capture_environment_probe(database_url)
    artifact["history_source"]["environment_probe"] = environment_probe
    collected = None
    failures: list[str] = []
    for candidate in environment_probe["candidates"]:
        if candidate["tcp_status"] != "listening":
            continue
        try:
            collected = collect_inventory(candidate["database_url"])
            candidate["inventory_status"] = "query_succeeded"
            candidate["last_error"] = None
            break
        except Exception as exc:
            candidate["inventory_status"] = "query_failed"
            candidate["last_error"] = str(exc)
            failures.append(f"{candidate['database_url']}: {exc}")

    if collected is None:
        inventory["source"] = "pipeline.execution_runs"
        inventory["captured_at"] = utc_now()
        inventory["total_runs"] = None
        inventory["open_runs"] = None
        inventory["done_runs"] = None
        inventory["sample_run_ids"] = []
        inventory["last_error"] = " | ".join(failures) if failures else "no reachable project database endpoint"
        artifact["history_source"]["kind"] = "db_inventory_failed"
        artifact["status_reason"] = (
            "Legacy replay inventory capture failed before run classification; "
            "environment evidence is still missing."
        )
        write_artifact(artifact)
        print("SEO_LEGACY_REPLAY_INVENTORY: FAILED")
        print(f"- {inventory['last_error']}")
        return 1

    total_runs = collected["total_runs"]
    done_runs = collected["done_runs"]
    open_runs = collected["open_runs"]
    sample_run_ids = collected["sample_run_ids"]
    inventory["source"] = "pipeline.execution_runs"
    inventory["captured_at"] = utc_now()
    inventory["total_runs"] = total_runs
    inventory["open_runs"] = open_runs
    inventory["done_runs"] = done_runs
    inventory["sample_run_ids"] = sample_run_ids
    inventory["last_error"] = None
    artifact["history_source"]["kind"] = "db_inventory"
    artifact["history_source"]["run_ids"] = sample_run_ids
    artifact["history_source"]["workflow_ids"] = sample_run_ids
    if total_runs == 0:
        artifact["evidence_status"] = "PASS"
        artifact["status_reason"] = (
            "No historical `seo_site_build` executions found in pipeline.execution_runs; "
            "legacy replay is not required for this environment."
        )
    else:
        artifact["evidence_status"] = "PENDING_LIVE_REPLAY"
        artifact["status_reason"] = (
            "Historical `seo_site_build` executions exist; replay certification remains required."
        )
    write_artifact(artifact)

    print(
        "SEO_LEGACY_REPLAY_INVENTORY: OK "
        f"total_runs={total_runs} open_runs={open_runs} done_runs={done_runs}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
