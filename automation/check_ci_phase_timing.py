#!/usr/bin/env python3
"""Exercise the actual CI timing wrapper without running Rust or providers."""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "automation" / "measure_ci_phase.sh"
SOURCE_SHA = "f" * 40


class PhaseTimingContract(unittest.TestCase):
    def test_success_is_recorded_with_exact_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "evidence.jsonl"
            result = self.invoke(report, "contracts", "exit 0")
            self.assertEqual(result.returncode, 0, result.stderr)
            entry = json.loads(report.read_text())
            self.assertEqual(entry["phase"], "contracts")
            self.assertEqual(entry["source_sha"], SOURCE_SHA)
            self.assertEqual(entry["exit_code"], 0)
            self.assertGreaterEqual(entry["elapsed_ms"], 0)

    def test_failure_is_never_masked_by_reporting(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "evidence.jsonl"
            result = self.invoke(report, "product", "exit 17")
            self.assertEqual(result.returncode, 17)
            entry = json.loads(report.read_text())
            self.assertEqual(entry["exit_code"], 17)
            self.assertEqual(entry["phase"], "product")

    def test_report_appends_independent_phases(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "evidence.jsonl"
            self.assertEqual(self.invoke(report, "contracts", "exit 0").returncode, 0)
            self.assertEqual(self.invoke(report, "integration", "exit 0").returncode, 0)
            entries = [json.loads(line) for line in report.read_text().splitlines()]
            self.assertEqual([entry["phase"] for entry in entries], ["contracts", "integration"])

    def test_rust_subphases_are_measured_by_the_same_wrapper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "evidence.jsonl"
            for phase in ("truth_workers_build", "truth_transport", "truth_suite", "semantic_fixture"):
                self.assertEqual(self.invoke(report, phase, "exit 0").returncode, 0)
            entries = [json.loads(line) for line in report.read_text().splitlines()]
            self.assertEqual(len(entries), 4)
            self.assertTrue(all(entry["exit_code"] == 0 for entry in entries))

    def test_unknown_phase_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "evidence.jsonl"
            result = self.invoke(report, "bypass", "exit 0")
            self.assertEqual(result.returncode, 2)
            self.assertFalse(report.exists())

    def test_wall_clock_jump_backwards_cannot_make_elapsed_negative(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_date = root / "date"
            fake_date.write_text(
                "#!/bin/sh\n"
                'if [ -e "$FAKE_DATE_STATE" ]; then printf "0\\n"; '
                'else touch "$FAKE_DATE_STATE"; printf "999999\\n"; fi\n',
                encoding="utf-8",
            )
            fake_date.chmod(0o755)
            result = self.invoke(
                root / "evidence.jsonl", "contracts", "exit 0",
                env_overrides={"PATH": f"{root}:{os.environ['PATH']}",
                               "FAKE_DATE_STATE": str(root / "date_called")},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertGreaterEqual(json.loads((root / "evidence.jsonl").read_text())["elapsed_ms"], 0)
            self.assertFalse((root / "date_called").exists(), "phase timer must not read wall clock")

    @staticmethod
    def invoke(
        report: Path, phase: str, command: str,
        env_overrides: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(WRAPPER), phase, "bash", "-c", command],
            cwd=ROOT,
            env={
                **os.environ,
                "CI_TIMING_REPORT_PATH": str(report),
                "CI_SOURCE_SHA": SOURCE_SHA,
                **(env_overrides or {}),
            },
            capture_output=True,
            text=True,
            check=False,
        )


if __name__ == "__main__":
    unittest.main()
