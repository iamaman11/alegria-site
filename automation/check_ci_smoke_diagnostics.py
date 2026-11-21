#!/usr/bin/env python3
"""Offline checks: diagnostic timing must not remove CI's real test commands."""

from __future__ import annotations

import contextlib
import io
import os
from pathlib import Path
import tempfile
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

        with patch.dict(os.environ, {
            "INTEGRATION_TEMPORAL_WORKER_BIN": "/dummy/temporal_worker",
            "INTEGRATION_OUTBOX_WORKER_BIN": "/dummy/outbox_worker",
        }):
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
        with patch.dict(os.environ, {
            "INTEGRATION_TEMPORAL_WORKER_BIN": "/dummy/temporal_worker",
            "INTEGRATION_OUTBOX_WORKER_BIN": "/dummy/outbox_worker",
        }):
            with patch.object(
                smoke_manual_llm_queue.subprocess,
                "run",
                return_value=SimpleNamespace(returncode=17),
            ) as run:
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(smoke_manual_llm_queue.main(), 17)
        self.assertEqual(run.call_count, 1)

    def test_shared_worker_build_precedes_all_three_real_tests(self) -> None:
        calls: list[list[str]] = []
        envs: list[dict[str, str]] = []
        with tempfile.TemporaryDirectory() as tmp:
            def run(args, **kwargs):
                calls.append(args)
                if args[:2] == ["cargo", "build"]:
                    binaries = Path(tmp) / "debug"
                    binaries.mkdir(parents=True)
                    suffix = ".exe" if os.name == "nt" else ""
                    for name in ("temporal_worker", "outbox_worker"):
                        (binaries / f"{name}{suffix}").write_text("test-binary", encoding="utf-8")
                else:
                    envs.append(kwargs["env"])
                return SimpleNamespace(returncode=0)

            with patch.dict(os.environ, {"CARGO_TARGET_DIR": tmp}, clear=True):
                with patch.object(smoke_manual_llm_queue.subprocess, "run", side_effect=run):
                    with contextlib.redirect_stdout(io.StringIO()):
                        self.assertEqual(smoke_manual_llm_queue.main(), 0)

        self.assertEqual(len(calls), 4)
        self.assertEqual(calls[0][:3], ["cargo", "build", "--locked"])
        self.assertIn("temporal_worker", calls[0])
        self.assertIn("outbox_worker", calls[0])
        self.assertEqual(len(envs), 3)
        suffix = ".exe" if os.name == "nt" else ""
        for env in envs:
            temporal = Path(env["INTEGRATION_TEMPORAL_WORKER_BIN"])
            outbox = Path(env["INTEGRATION_OUTBOX_WORKER_BIN"])
            self.assertEqual((temporal.parent.name, temporal.name), ("debug", f"temporal_worker{suffix}"))
            self.assertEqual((outbox.parent.name, outbox.name), ("debug", f"outbox_worker{suffix}"))

    def test_failed_prebuild_never_claims_manual_tests_passed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": tmp}, clear=True):
                with patch.object(
                    smoke_manual_llm_queue.subprocess,
                    "run",
                    return_value=SimpleNamespace(returncode=23),
                ) as run:
                    with contextlib.redirect_stdout(io.StringIO()):
                        self.assertEqual(smoke_manual_llm_queue.main(), 23)
        self.assertEqual(run.call_count, 1)

    def test_missing_prebuilt_binaries_is_error_not_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": tmp}, clear=True):
                with patch.object(
                    smoke_manual_llm_queue.subprocess,
                    "run",
                    return_value=SimpleNamespace(returncode=0),
                ) as run:
                    with contextlib.redirect_stdout(io.StringIO()):
                        with contextlib.redirect_stderr(io.StringIO()):
                            self.assertEqual(smoke_manual_llm_queue.main(), 1)
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
