#!/usr/bin/env python3
"""Offline deterministic contract for evidence emitted at the existing PR gate."""

from __future__ import annotations

from datetime import datetime, timezone
import unittest

from report_ci_critical_path import analyze, timestamp, to_summary


def job(name: str, start: str, end: str, conclusion: str = "success") -> dict:
    return {
        "name": name, "started_at": start, "completed_at": end,
        "conclusion": conclusion,
    }


RUN = {
    "event": "pull_request", "id": 42, "head_sha": "a" * 40,
    "created_at": "2026-11-25T14:00:09Z",
}
NOW = datetime(2026, 11, 25, 14, 4, 40, tzinfo=timezone.utc)
JOBS = [
    job("impact", "2026-11-25T14:00:10Z", "2026-11-25T14:00:21Z"),
    job("rust-e2e-temporal-harness", "2026-11-25T14:00:23Z", "2026-11-25T14:02:20Z"),
    job("rust-e2e-legacy-binaries", "2026-11-25T14:00:25Z", "2026-11-25T14:03:15Z"),
    job("rust-temporal-legacy-e2e (temporal_manual)",
        "2026-11-25T14:03:17Z", "2026-11-25T14:04:36Z"),
    job("CI", "2026-11-25T14:04:38Z", None, ""),
]


class CriticalPathEvidence(unittest.TestCase):
    def test_real_after_compiler_chain_and_no_false_sla(self) -> None:
        result = analyze(RUN, JOBS, NOW)
        self.assertEqual(result["observed_event_to_current_step_seconds"], 271.0)
        self.assertFalse(result["event_to_green_180_confirmed"])
        self.assertEqual(len(result["measured_paths"]), 1)
        path = result["measured_paths"][0]
        self.assertEqual(path["last_compiler"], "rust-e2e-legacy-binaries")
        self.assertEqual(path["compiler_ready_seconds"], 186.0)
        self.assertEqual(path["runner_gap_seconds"], 2.0)
        self.assertEqual(path["e2e_job_seconds"], 79.0)
        self.assertEqual(path["event_to_e2e_finish_seconds"], 267.0)
        self.assertIn("not** an accepted ≤180s", to_summary(result))

    def test_both_compilers_are_independently_required(self) -> None:
        missing = [row for row in JOBS if row["name"] != "rust-e2e-legacy-binaries"]
        with self.assertRaisesRegex(ValueError, "successful prerequisite"):
            analyze(RUN, missing, NOW)

    def test_skip_is_not_success(self) -> None:
        skipped = [dict(row, conclusion="skipped") if row["name"] == "rust-e2e-legacy-binaries" else row
                   for row in JOBS]
        with self.assertRaisesRegex(ValueError, "successful prerequisite"):
            analyze(RUN, skipped, NOW)

    def test_cannot_invert_runner_dependency(self) -> None:
        invalid = [dict(row, started_at="2026-11-25T14:03:13Z")
                   if row["name"] == "rust-temporal-legacy-e2e (temporal_manual)" else row
                   for row in JOBS]
        with self.assertRaisesRegex(ValueError, "started before"):
            analyze(RUN, invalid, NOW)

    def test_missing_timezone_or_future_created_at_rejected(self) -> None:
        with self.assertRaises(ValueError):
            timestamp("2026-11-25T14:00:09")
        with self.assertRaisesRegex(ValueError, "future"):
            analyze(dict(RUN, created_at="2026-11-25T14:50:09Z"), JOBS, NOW)

    def test_non_pr_not_pretended_as_evidence(self) -> None:
        with self.assertRaisesRegex(ValueError, "PR-only"):
            analyze(dict(RUN, event="push"), JOBS, NOW)


if __name__ == "__main__":
    unittest.main()
