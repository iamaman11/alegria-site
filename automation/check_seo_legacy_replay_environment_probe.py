#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys

from local_env import inventory_probe_urls, resolve_database_url


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"


def main() -> int:
    payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
    failures: list[str] = []
    resolved_url = resolve_database_url()
    required_urls = set(inventory_probe_urls(resolved_url))

    probe = payload.get("history_source", {}).get("environment_probe")
    if not isinstance(probe, dict):
        print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: FAILED")
        print("- missing history_source.environment_probe")
        return 1

    candidates = probe.get("candidates")
    if not isinstance(candidates, list):
        print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: FAILED")
        print("- history_source.environment_probe.candidates must be list")
        return 1

    by_url = {}
    for candidate in candidates:
        if isinstance(candidate, dict) and "database_url" in candidate:
            by_url[candidate["database_url"]] = candidate

    missing = sorted(required_urls - by_url.keys())
    for database_url in missing:
        failures.append(f"missing canonical candidate `{database_url}`")

    for database_url in sorted(required_urls & by_url.keys()):
        candidate = by_url[database_url]
        tcp_status = candidate.get("tcp_status")
        inventory_status = candidate.get("inventory_status")

        if tcp_status not in {"connect_failed", "listening"}:
            failures.append(
                f"`{database_url}` has invalid tcp_status `{tcp_status}`"
            )
        if inventory_status not in {"not_attempted", "query_failed", "query_succeeded"}:
            failures.append(
                f"`{database_url}` has invalid inventory_status `{inventory_status}`"
            )
        if database_url == resolved_url and tcp_status == "listening" and inventory_status == "not_attempted":
            failures.append(
                f"`{database_url}` is the configured inventory DSN and must be query-attempted"
            )
        if tcp_status == "connect_failed" and inventory_status != "not_attempted":
            failures.append(
                f"`{database_url}` cannot be query-attempted when tcp_status is connect_failed"
            )

    if failures:
        print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    statuses = ", ".join(
        f"{url.rsplit('@', 1)[-1]}:{by_url[url]['tcp_status']}/{by_url[url]['inventory_status']}"
        for url in sorted(required_urls)
    )
    print(f"SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: OK {statuses}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
