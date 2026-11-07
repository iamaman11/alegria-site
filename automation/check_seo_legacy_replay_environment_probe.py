#!/usr/bin/env python3
"""Verify the *recorded* legacy replay inventory environment.

Historical run evidence is immutable.  The current CI DATABASE_URL belongs to
a disposable test database and must never be required inside a past capture.
A current staging inventory is a separate live operation, not this contract.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys
from urllib.parse import urlsplit

from local_env import LEGACY_DEV_DATABASE_URLS


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"


def endpoint(url: str) -> tuple[str, int] | None:
    try:
        parsed = urlsplit(url)
        if parsed.scheme not in {"postgres", "postgresql"} or not parsed.hostname:
            return None
        return parsed.hostname, parsed.port or 5432
    except (TypeError, ValueError):
        return None


def validate_snapshot(payload: dict) -> list[str]:
    failures: list[str] = []
    history = payload.get("history_source")
    if not isinstance(history, dict):
        return ["missing history_source"]

    probe = history.get("environment_probe")
    if not isinstance(probe, dict):
        return ["missing history_source.environment_probe"]
    if not probe.get("captured_at"):
        failures.append("historical environment probe has no capture timestamp")

    candidates = probe.get("candidates")
    if not isinstance(candidates, list) or not candidates:
        return failures + ["historical candidates must be a non-empty list"]

    observed: set[tuple[str, int]] = set()
    successful_queries = 0
    for index, candidate in enumerate(candidates):
        if not isinstance(candidate, dict):
            failures.append(f"candidate {index} must be an object")
            continue

        recorded = endpoint(candidate.get("database_url"))
        if recorded is None:
            failures.append(f"candidate {index} has invalid recorded database endpoint")
            continue
        observed.add(recorded)
        if (candidate.get("host"), candidate.get("port")) != recorded:
            failures.append(f"candidate {index} recorded host/port disagrees with URL")

        tcp = candidate.get("tcp_status")
        inventory = candidate.get("inventory_status")
        if tcp not in {"connect_failed", "listening"}:
            failures.append(f"candidate {index} has invalid tcp_status")
        if inventory not in {"not_attempted", "query_failed", "query_succeeded"}:
            failures.append(f"candidate {index} has invalid inventory_status")
        if tcp == "connect_failed" and inventory != "not_attempted":
            failures.append(f"candidate {index} queried an unreachable endpoint")
        if inventory in {"query_failed", "query_succeeded"} and tcp != "listening":
            failures.append(f"candidate {index} query status contradicts TCP status")
        if inventory == "query_succeeded":
            successful_queries += 1

    # The historical capture tool always includes the three known local
    # endpoints, even when its primary configured URL was different.  Compare
    # endpoint identities, not credentials or today's CI connection string.
    expected = {endpoint(url) for url in LEGACY_DEV_DATABASE_URLS}
    for host, port in sorted(expected - observed):
        failures.append(f"missing recorded legacy endpoint {host}:{port}")

    inventory = history.get("inventory")
    if history.get("kind") == "db_inventory":
        if not isinstance(inventory, dict):
            failures.append("successful historical inventory is missing")
        else:
            counts = [inventory.get(key) for key in ("total_runs", "open_runs", "done_runs")]
            if any(type(value) is not int or value < 0 for value in counts):
                failures.append("historical inventory counts must be nonnegative integers")
            elif counts[0] != counts[1] + counts[2]:
                failures.append("historical inventory counts contradict each other")
            if inventory.get("last_error") is not None:
                failures.append("successful inventory cannot report a query error")
        if not successful_queries:
            failures.append("successful historical inventory has no successful query")

    if payload.get("evidence_status") == "PASS" and not successful_queries:
        failures.append("PASS cannot be supported without an inventory query")
    return failures


def main() -> int:
    payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    failures = validate_snapshot(payload)
    if failures:
        print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: OK historical_snapshot_only")
    return 0


if __name__ == "__main__":
    sys.exit(main())
