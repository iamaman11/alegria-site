#!/usr/bin/env python3
"""Offline checks: diagnostic timing must not remove CI's real test commands."""

from __future__ import annotations

import contextlib
import io
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import smoke_manual_llm_queue
import smoke_run_report_completeness


class SmokeDiagnosticsContract(unittest.TestCase):
    def test_all_three_manual_inbox_tests_still_run(self) -> None:
        calls: list[list[str]] = []

        def run(args, **_kwargs):
            calls.append(args)
            return SimpleNamespace(returncode=0)

        with patch.object(smoke_manual_llm_queue.subprocess, "run", side_effect=run):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(smoke_manual_llm_queue.main(), 0)
        self.assertEqual(len(calls), 3)
        self.assertEqual([call[6] for call in calls], [
            "manual_llm_queue_persists_validates_replays_and_recovers",
            "manual_truth_inbox_refuses_fabricated_evidence_and_replays_atomically",
            "canonical_temporal_manual_editorial_resumes_after_worker_restart",
        ])
        self.assertEqual(calls[2][-2:], ["--", "--nocapture"])

    def test_manual_inbox_test_failure_propagates(self) -> None:
        with patch.object(
            smoke_manual_llm_queue.subprocess,
            "run",
            return_value=SimpleNamespace(returncode=17),
        ) as run:
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(smoke_manual_llm_queue.main(), 17)
        self.assertEqual(run.call_count, 1)

    def test_report_smoke_preserves_three_cargo_commands_and_sqlx_offline(self) -> None:
        calls: list[list[str]] = []

        def run(args, **kwargs):
            calls.append(args)
            self.assertEqual(kwargs["env"]["SQLX_OFFLINE"], "true")
            self.assertIn("PATH", kwargs["env"])
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        with patch.object(smoke_run_report_completeness.subprocess, "run", side_effect=run):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(smoke_run_report_completeness.main(), 0)
        self.assertEqual(len(calls), 3)
        self.assertEqual([call[0] for call in calls], ["cargo"] * 3)
        self.assertIn("scenario_report_surface_serializes_phase_reports", calls[0])
        self.assertEqual(calls[1][-1], "--no-run")
        self.assertIn("synthetic_full_scenario_persists_page_draft_on_full_harness", calls[2])

    def test_report_smoke_exits_on_first_failure(self) -> None:
        with patch.object(
            smoke_run_report_completeness.subprocess,
            "run",
            return_value=SimpleNamespace(returncode=23, stdout="", stderr="failure"),
        ) as run:
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(smoke_run_report_completeness.main(), 23)
        self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
