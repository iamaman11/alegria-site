#!/usr/bin/env python3
"""Offline deterministic contract for evidence emitted at the existing PR gate."""

from __future__ import annotations

from datetime import datetime, timezone
import unittest

from report_ci_critical_path import (
    COMPILER_STEPS, LIVE_E2E_STEPS, analyze, timestamp, to_summary,
)


def job(name: str, start: str, end: str, conclusion: str = "success",
        step: str | None = None) -> dict:
    return {
        "name": name, "started_at": start, "completed_at": end,
        "conclusion": conclusion,
        "run_id": 42, "run_attempt": 1, "head_sha": "a" * 40,
        "steps": ([{"name": step, "started_at": start, "completed_at": end,
                    "conclusion": conclusion}] if step else []),
    }


RUN = {
    "event": "pull_request", "id": 42, "head_sha": "a" * 40,
    "run_attempt": 1,
    "created_at": "2026-11-25T14:00:09Z",
}
NOW = datetime(2026, 11, 25, 14, 4, 40, tzinfo=timezone.utc)
JOBS = [
    job("impact", "2026-11-25T14:00:10Z", "2026-11-25T14:00:21Z"),
    job("rust-e2e-temporal-harness", "2026-11-25T14:00:23Z", "2026-11-25T14:02:20Z",
        step=COMPILER_STEPS["rust-e2e-temporal-harness"]),
    job("rust-e2e-legacy-binaries", "2026-11-25T14:00:25Z", "2026-11-25T14:03:15Z",
        step=COMPILER_STEPS["rust-e2e-legacy-binaries"]),
    job("rust-temporal-legacy-e2e (temporal_manual)",
        "2026-11-25T14:03:17Z", "2026-11-25T14:04:36Z",
        step=LIVE_E2E_STEPS["manual-temporal"]),
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
        self.assertEqual(path["real_e2e_step_seconds"], 79.0)
        self.assertEqual(path["compiler_steps"][1]["cargo_step_seconds"], 170.0)
        self.assertIn("not** an accepted ≤180s", to_summary(result))
        self.assertIn("Precise runner stages", to_summary(result))

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

    def test_missing_or_failed_live_step_cannot_claim_real_e2e_timing(self) -> None:
        missing = [dict(row, steps=[]) if row["name"].startswith("rust-temporal-legacy-e2e")
                   else row for row in JOBS]
        with self.assertRaisesRegex(ValueError, "missing successful source-verified step"):
            analyze(RUN, missing, NOW)
        wrong = [dict(row, steps=[dict(step, conclusion="failure") for step in row["steps"]])
                 if row["name"] == "rust-e2e-legacy-binaries" else row for row in JOBS]
        with self.assertRaisesRegex(ValueError, "missing successful source-verified step"):
            analyze(RUN, wrong, NOW)

    def test_job_run_and_attempt_provenance_fail_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "different workflow run"):
            analyze(RUN, [dict(row, run_id=666) if row["name"] == "impact" else row
                          for row in JOBS], NOW)
        with self.assertRaisesRegex(ValueError, "different workflow run attempt"):
            analyze(RUN, [dict(row, run_attempt=2) if row["name"] == "impact" else row
                          for row in JOBS], NOW, checkout_sha="f" * 40, expected_attempt=1)
        with self.assertRaisesRegex(ValueError, "unrelated source checkout"):
            analyze(RUN, [dict(row, head_sha="b" * 40) if row["name"] == "impact" else row
                          for row in JOBS], NOW, checkout_sha="f" * 40, expected_attempt=1)
        self.assertEqual(
            analyze(RUN, JOBS, NOW, checkout_sha="f" * 40, expected_attempt=1)["checkout_sha"],
            "f" * 40,
        )

    def test_source_bound_step_window_must_stay_inside_actual_job(self) -> None:
        invalid = [dict(row, steps=[dict(step, started_at="2026-11-25T14:00:15Z")
                                    for step in row["steps"]])
                   if row["name"] == "rust-e2e-legacy-binaries" else row for row in JOBS]
        with self.assertRaisesRegex(ValueError, "out-of-job timestamp"):
            analyze(RUN, invalid, NOW)

    def test_all_three_physical_runners_keep_every_compiler_and_real_step(self) -> None:
        complete = [
            *JOBS,
            job("rust-e2e-qdrant-temporal-harness", "2026-11-25T14:00:22Z",
                "2026-11-25T14:02:30Z", step=COMPILER_STEPS["rust-e2e-qdrant-temporal-harness"]),
            job("rust-e2e-qdrant-effects-binaries", "2026-11-25T14:00:23Z",
                "2026-11-25T14:02:35Z", step=COMPILER_STEPS["rust-e2e-qdrant-effects-binaries"]),
            job("rust-e2e-v3-binaries", "2026-11-25T14:00:25Z",
                "2026-11-25T14:02:33Z", step=COMPILER_STEPS["rust-e2e-v3-binaries"]),
            job("rust-qdrant-temporal-e2e (temporal_qdrant)", "2026-11-25T14:02:40Z",
                "2026-11-25T14:03:20Z", step=LIVE_E2E_STEPS["native-qdrant"]),
            job("rust-e2e-outbox-harness", "2026-11-25T14:00:22Z",
                "2026-11-25T14:02:31Z", step=COMPILER_STEPS["rust-e2e-outbox-harness"]),
            job("rust-outbox-e2e (outbox)", "2026-11-25T14:03:20Z",
                "2026-11-25T14:04:15Z", step=LIVE_E2E_STEPS["outbox"]),
        ]
        report = analyze(RUN, complete, NOW, checkout_sha="f" * 40, expected_attempt=1)
        routes = {path["route"]: path for path in report["measured_paths"]}
        self.assertEqual(set(routes), {"manual-temporal", "native-qdrant", "outbox"})
        self.assertEqual(len(routes["native-qdrant"]["compiler_steps"]), 3)
        self.assertEqual(routes["native-qdrant"]["last_compiler"],
                         "rust-e2e-qdrant-effects-binaries")
        self.assertEqual(routes["outbox"]["last_compiler"], "rust-e2e-legacy-binaries")
        self.assertFalse(report["event_to_green_180_confirmed"])


if __name__ == "__main__":
    unittest.main()
