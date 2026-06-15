#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys

from local_env import inventory_probe_urls, resolve_database_url


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"


def normalize_dsn(url: str) -> str:
    if "@" in url:
        prefix, suffix = url.split("@", 1)
        if "//" in prefix:
            proto, _ = prefix.split("//", 1)
            return f"{proto}//*@{suffix}"
    return url


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
            by_url[normalize_dsn(candidate["database_url"])] = candidate

    required_normalized = {normalize_dsn(url): url for url in required_urls}

    missing = sorted(required_normalized.keys() - by_url.keys())
    for dsn in missing:
        failures.append(f"missing canonical candidate `{required_normalized[dsn]}`")

    for dsn in sorted(required_normalized.keys() & by_url.keys()):
        original_url = required_normalized[dsn]
        candidate = by_url[dsn]
        tcp_status = candidate.get("tcp_status")
        inventory_status = candidate.get("inventory_status")

        if tcp_status not in {"connect_failed", "listening"}:
            failures.append(
                f"`{original_url}` has invalid tcp_status `{tcp_status}`"
            )
        if inventory_status not in {"not_attempted", "query_failed", "query_succeeded"}:
            failures.append(
                f"`{original_url}` has invalid inventory_status `{inventory_status}`"
            )
        if dsn == normalize_dsn(resolved_url) and tcp_status == "listening" and inventory_status == "not_attempted":
            failures.append(
                f"`{original_url}` is the configured inventory DSN and must be query-attempted"
            )
        if tcp_status == "connect_failed" and inventory_status != "not_attempted":
            failures.append(
                f"`{original_url}` cannot be query-attempted when tcp_status is connect_failed"
            )

    if failures:
        print("SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    statuses = ", ".join(
        f"{original_url.rsplit('@', 1)[-1]}:{by_url[dsn]['tcp_status']}/{by_url[dsn]['inventory_status']}"
        for dsn, original_url in sorted(required_normalized.items())
    )
    print(f"SEO_LEGACY_REPLAY_ENVIRONMENT_PROBE: OK {statuses}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
