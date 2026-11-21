#!/usr/bin/env python3
"""No-Cargo regression tests for streamed, fail-closed Rust build evidence."""

from __future__ import annotations

import io
import json
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

from cargo_feedback import run_cargo, workspace_artifact


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app/rust"


def artifact(name: str, *, fresh: bool, workspace: bool = True) -> str:
    source = (WORKSPACE if workspace else Path("/external/registry")) / name / "src/lib.rs"
    return json.dumps({
        "reason": "compiler-artifact", "fresh": fresh,
        "target": {"name": name, "src_path": str(source)},
    }) + "\n"


class FakeProcess:
    def __init__(self, lines: list[str], exit_code: int = 0) -> None:
        self.stdout = iter(lines)
        self.exit_code = exit_code

    def wait(self) -> int:
        return self.exit_code


class CargoFeedbackTests(unittest.TestCase):
    def test_streamed_json_counts_only_actual_workspace_artifacts(self) -> None:
        lines = [
            artifact("http_client", fresh=False),
            artifact("gsc_sync", fresh=True),
            artifact("serde", fresh=False, workspace=False),
            json.dumps({"reason": "compiler-message", "message": {"rendered": "warning: useful\n"}}) + "\n",
            json.dumps({"reason": "build-finished", "success": True}) + "\n",
            "test scope::behavior ... ok\n",
        ]
        output = io.StringIO()
        command = ["cargo", "test", "--locked", "--package", "http_client"]
        with patch("cargo_feedback.subprocess.Popen", return_value=FakeProcess(lines)) as popen:
            result = run_cargo(command, cwd=WORKSPACE, output=output)
        self.assertEqual(popen.call_args.args[0][2], "--message-format=json-render-diagnostics")
        self.assertEqual(result["command"], command)
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(result["rebuilt_workspace_targets"], ["http_client"])
        self.assertEqual(result["fresh_workspace_targets"], ["gsc_sync"])
        self.assertEqual(result["artifact_evidence"], "observed")
        self.assertIsNotNone(result["compiler_seconds"])
        self.assertIn("warning: useful", output.getvalue())
        self.assertIn("test scope::behavior ... ok", output.getvalue())
        self.assertIn("CARGO_FEEDBACK_JSON=", output.getvalue())

    def test_no_artifacts_is_unknown_not_a_claim_of_zero_rebuilds(self) -> None:
        with patch("cargo_feedback.subprocess.Popen", return_value=FakeProcess(["running 0 tests\n"])):
            result = run_cargo(["cargo", "check", "--locked"], cwd=WORKSPACE, output=io.StringIO())
        self.assertEqual(result["artifact_evidence"], "unavailable")
        self.assertEqual(result["rebuilt_workspace_targets"], [])
        self.assertIsNone(result["compiler_seconds"])

    def test_failure_is_logged_but_never_turns_into_success(self) -> None:
        output = io.StringIO()
        with patch("cargo_feedback.subprocess.Popen", return_value=FakeProcess([
            artifact("vector_qdrant", fresh=False),
            json.dumps({"reason": "build-finished", "success": False}) + "\n",
        ], exit_code=17)):
            with self.assertRaises(subprocess.CalledProcessError) as error:
                run_cargo(["cargo", "check", "--locked"], cwd=WORKSPACE, output=output)
        self.assertEqual(error.exception.returncode, 17)
        self.assertIn('"exit_code": 17', output.getvalue())

    def test_forbidden_command_cannot_be_modified_or_executed(self) -> None:
        with patch("cargo_feedback.subprocess.Popen") as popen:
            with self.assertRaises(ValueError):
                run_cargo(["cargo", "build", "--release"], cwd=WORKSPACE)
        popen.assert_not_called()

    def test_malformed_artifact_does_not_count_as_compiled(self) -> None:
        self.assertIsNone(workspace_artifact({"reason": "compiler-artifact", "target": {}}, WORKSPACE))
        self.assertIsNone(workspace_artifact({"reason": "compiler-artifact", "target": None}, WORKSPACE))


if __name__ == "__main__":
    unittest.main()
