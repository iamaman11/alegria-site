#!/usr/bin/env python3
"""Behavioral regressions for immutable historical replay environment evidence."""
from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
import unittest

from check_seo_legacy_replay_environment_probe import (
    ARTIFACT,
    validate_snapshot,
)


class LegacyReplayEnvironmentSnapshotTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.recorded = json.loads(Path(ARTIFACT).read_text(encoding="utf-8"))

    def snapshot(self) -> dict:
        return deepcopy(self.recorded)

    def test_historical_capture_is_valid_independent_of_ci_database_url(self) -> None:
        # Snapshot from May must not depend on the November disposable CI DB.
        self.assertEqual(validate_snapshot(self.snapshot()), [])

    def test_missing_canonical_endpoint_is_blocked(self) -> None:
        payload = self.snapshot()
        candidates = payload["history_source"]["environment_probe"]["candidates"]
        payload["history_source"]["environment_probe"]["candidates"] = [
            item for item in candidates if item["port"] != 5433
        ]
        self.assertIn("missing recorded legacy endpoint localhost:5433", validate_snapshot(payload))

    def test_host_port_tampering_is_blocked(self) -> None:
        payload = self.snapshot()
        payload["history_source"]["environment_probe"]["candidates"][0]["port"] = 9999
        self.assertTrue(
            any("disagrees with URL" in problem for problem in validate_snapshot(payload))
        )

    def test_unreachable_database_cannot_claim_a_query(self) -> None:
        payload = self.snapshot()
        candidate = payload["history_source"]["environment_probe"]["candidates"][1]
        candidate["tcp_status"] = "connect_failed"
        candidate["inventory_status"] = "query_succeeded"
        self.assertTrue(
            any("queried an unreachable endpoint" in problem for problem in validate_snapshot(payload))
        )

    def test_pass_without_any_successful_query_is_blocked(self) -> None:
        payload = self.snapshot()
        for item in payload["history_source"]["environment_probe"]["candidates"]:
            if item["inventory_status"] == "query_succeeded":
                item["inventory_status"] = "not_attempted"
        self.assertTrue(
            any("no successful query" in problem for problem in validate_snapshot(payload))
        )

    def test_corrupt_inventory_counts_are_blocked(self) -> None:
        payload = self.snapshot()
        payload["history_source"]["inventory"]["total_runs"] = 7
        self.assertTrue(
            any("counts contradict" in problem for problem in validate_snapshot(payload))
        )


if __name__ == "__main__":
    unittest.main()
