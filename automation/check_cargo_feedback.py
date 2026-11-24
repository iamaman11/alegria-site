#!/usr/bin/env python3
"""No-Cargo regression tests for streamed, fail-closed Rust build evidence."""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from cargo_feedback import run_cargo, workspace_artifact
from ci_impact import (
    SCHEMA_VERSION, SUITE_OWNERS, WORKER_E2E_PREPARE,
    suite_commands_by_ci_job, suite_commands_for_report,
)
from run_scoped_cargo_feedback import (
    cargo_commands_for_ci, main as run_scoped_cargo_feedback, selected_preparations,
)


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

    def test_locked_debug_worker_build_and_explicit_runtime_env_are_measured(self) -> None:
        command = ["cargo", "build", "--locked", "-p", "temporal_worker", "--bin", "temporal_worker"]
        with patch("cargo_feedback.subprocess.Popen", return_value=FakeProcess([
            artifact("temporal_worker", fresh=False),
            json.dumps({"reason": "build-finished", "success": True}) + "\n",
        ])) as popen:
            got = run_cargo(command, cwd=WORKSPACE, output=io.StringIO(),
                            env={"INTEGRATION_TEMPORAL_WORKER_BIN": "/fake/prebuilt/temporal_worker"})
        self.assertEqual(got["command"], command)
        self.assertIn("temporal_worker", got["rebuilt_workspace_targets"])
        self.assertEqual(popen.call_args.kwargs["env"]["INTEGRATION_TEMPORAL_WORKER_BIN"],
                         "/fake/prebuilt/temporal_worker")

        with patch("cargo_feedback.subprocess.Popen") as forbidden:
            with self.assertRaises(ValueError):
                run_cargo(["cargo", "build", "-p", "temporal_worker"], cwd=WORKSPACE)
        forbidden.assert_not_called()

    def test_malformed_artifact_does_not_count_as_compiled(self) -> None:
        self.assertIsNone(workspace_artifact({"reason": "compiler-artifact", "target": {}}, WORKSPACE))
        self.assertIsNone(workspace_artifact({"reason": "compiler-artifact", "target": None}, WORKSPACE))


class ScopedCargoFeedbackTests(unittest.TestCase):
    """Exercise the production planner/runner ordering without installing Cargo."""

    def report(self, suites: list[str]) -> dict:
        return {
            "source_sha": "exact-test-source-sha",
            "schema_version": SCHEMA_VERSION,
            "mandatory_full_ci_gate": True,
            "selected_suites": suites,
            "suite_fixture_ids": {},
            "analysis": "full",
            "direct_crates": [],
            "affected_crates": [],
            "selected_ci_jobs": ["architecture-and-rust", "verify"],
        }

    def run_plan(
        self, suites: list[str], *, delegated: bool = False, suite_job: str = "architecture-and-rust",
    ) -> tuple[list[tuple[list[str], dict]], str, dict]:
        report = self.report(suites)
        executions: list[tuple[list[str], dict]] = []

        def record(command: list[str], **kwargs: object) -> dict:
            executions.append((command, kwargs))
            return {"command": command, "exit_code": 0}

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report_path = root / "impact.json"
            report_path.write_text(json.dumps(report), encoding="utf-8")
            target = root / "app/rust/target"
            # A real build produces executable files at these exact paths.
            # Make the stubs executable so production verification stays live.
            for owner in SUITE_OWNERS.values():
                for binary in owner.get("runtime_binaries", {}).values():
                    path = target / "debug" / binary
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"test prebuilt binary")
                    path.chmod(0o755)

            with (
                patch("run_scoped_cargo_feedback.ROOT", root),
                patch("run_scoped_cargo_feedback.WORKSPACE", root / "app/rust"),
                patch("run_scoped_cargo_feedback.subprocess.check_output",
                      return_value="exact-test-source-sha\n"),
                patch("run_scoped_cargo_feedback.commands_for_report",
                      return_value=[["cargo", "test", "--workspace", "--locked"]]),
                patch("run_scoped_cargo_feedback.run_cargo", side_effect=record),
                patch.object(sys, "argv", [
                    "runner", "--report", str(report_path),
                    *(["--delegate-workspace-unit-tests-to-verify"] if delegated else []),
                    *(["--suite-job", suite_job] if suite_job != "architecture-and-rust" else []),
                ]),
                patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}, clear=True),
                patch("sys.stdout", new_callable=io.StringIO) as output,
            ):
                self.assertEqual(run_scoped_cargo_feedback(), 0)
                lines = output.getvalue()
            measurements = json.loads(
                (root / (
                    "automation/reports/scoped_cargo_feedback.json"
                    if suite_job == "architecture-and-rust"
                    else "automation/reports/scoped_cargo_feedback_verify.json"
                )).read_text(encoding="utf-8")
            )
        return executions, lines, measurements

    def test_runtime_prebuild_runs_once_before_any_e2e_without_changing_tests(self) -> None:
        suites = ["cms_review_revision", "truth_certification", "knowledge_a13", "temporal_restart"]
        commands = suite_commands_by_ci_job(self.report(suites))["architecture-and-rust"]
        executions, output, measurements = self.run_plan(suites)
        expected = [
            ["cargo", "test", "--workspace", "--locked"],
            list(WORKER_E2E_PREPARE),
            *(command for _, command in commands),
        ]
        self.assertEqual([command for command, _ in executions], expected)
        self.assertEqual(len(measurements["measurements"]), len(expected))
        self.assertEqual(output.count("CI_SUITE_PREPARE=truth_certification COMMAND="), 1)
        self.assertIn("CI_SUITE_PREPARE_REUSED=temporal_restart", output)
        self.assertEqual(output.count("CI_SUITE="), len(commands))
        self.assertTrue(all(kwargs.get("env") is None for _, kwargs in executions[:2]))
        for (name, _), (_, kwargs) in zip(commands, executions[2:]):
            if name in {"truth_certification", "temporal_restart"}:
                self.assertIn("INTEGRATION_TEMPORAL_WORKER_BIN", kwargs["env"])
                self.assertIn("INTEGRATION_OUTBOX_WORKER_BIN", kwargs["env"])
            else:
                self.assertIsNone(kwargs["env"])

    def test_e2e_without_runtime_owner_does_not_build_workers(self) -> None:
        suites = ["cms_review_revision", "knowledge_a13", "publishing_a17"]
        executions, output, _ = self.run_plan(suites)
        self.assertEqual([command for command, _ in executions], [
            ["cargo", "test", "--workspace", "--locked"],
            *(command for _, command in suite_commands_by_ci_job(self.report(suites))["architecture-and-rust"]),
        ])
        self.assertNotIn("CI_SUITE_PREPARE=", output)
        self.assertNotIn("CI_SUITE_PREPARE_REUSED=", output)

    def test_full_workspace_tests_delegated_only_to_parallel_verify_owner(self) -> None:
        suites = ["cms_review_revision", "knowledge_a13", "temporal_restart"]
        executions, output, evidence = self.run_plan(suites, delegated=True)
        expected = [
            list(WORKER_E2E_PREPARE),
            *(command for _, command in suite_commands_by_ci_job(self.report(suites))["architecture-and-rust"]),
        ]
        self.assertEqual([command for command, _ in executions], expected)
        self.assertEqual(evidence["delegated_workspace_unit_tests_to"], "verify")
        self.assertIn(
            "CI_WORKSPACE_UNIT_TESTS=DELEGATED_TO_VERIFY source_sha=exact-test-source-sha", output,
        )
        self.assertEqual(len(evidence["measurements"]), len(expected))
        self.assertIn("CI_SUITE_PREBUILT=temporal_restart", output)

    def test_full_verify_runs_only_its_selected_real_e2e_after_workspace_units(self) -> None:
        suites = [
            "cms_review_revision", "draft_qa_scenario", "knowledge_a13",
            "outbox_a14", "planning_atomicity", "publishing_a17",
            "temporal_restart", "truth_adjudication_integrity",
        ]
        executions, output, evidence = self.run_plan(suites, suite_job="verify")
        assigned = suite_commands_by_ci_job(self.report(suites))["verify"]
        self.assertEqual([command for command, _ in executions], [command for _, command in assigned])
        self.assertEqual({name for name, _ in assigned}, {
            "cms_review_revision", "draft_qa_scenario", "knowledge_a13", "outbox_a14",
            "planning_atomicity", "publishing_a17", "truth_adjudication_integrity",
        })
        self.assertNotIn("CI_SUITE_PREPARE=", output)
        self.assertNotIn("CI_WORKSPACE_UNIT_TESTS=", output)
        self.assertEqual(evidence["suite_job"], "verify")
        self.assertEqual(evidence["selected_suites"], [name for name, _ in assigned])
        self.assertEqual(len(evidence["measurements"]), len(assigned))

    def test_delegation_rejects_ungated_verify_and_noncanonical_full_command(self) -> None:
        report = self.report([])
        report["selected_ci_jobs"] = ["architecture-and-rust"]
        with self.assertRaisesRegex(ValueError, "requires both selected CI owners"):
            cargo_commands_for_ci(report, delegate_workspace_unit_tests_to_verify=True)

        report["selected_ci_jobs"] = ["architecture-and-rust", "verify"]
        with patch("run_scoped_cargo_feedback.commands_for_report", return_value=[
            ["cargo", "check", "--workspace", "--locked"],
        ]):
            with self.assertRaisesRegex(ValueError, "requires both selected CI owners"):
                cargo_commands_for_ci(report, delegate_workspace_unit_tests_to_verify=True)

    def test_scoped_cargo_commands_cannot_be_delegated(self) -> None:
        report = self.report([])
        report.update({
            "analysis": "scoped", "direct_crates": ["cms_domain"],
            "affected_crates": ["cms_domain"], "selected_ci_jobs": ["architecture-and-rust"],
        })
        commands, delegated = cargo_commands_for_ci(
            report, delegate_workspace_unit_tests_to_verify=True,
        )
        self.assertEqual(commands, [
            ["cargo", "test", "--locked", "--package", "cms_domain"],
        ])
        self.assertFalse(delegated)

    def test_truth_filter_rejected_before_any_cargo_runs(self) -> None:
        selected = suite_commands_for_report(self.report(["truth_certification"]))
        with patch.dict(os.environ, {"TRUTH_CERT_LIMIT": "1"}):
            with self.assertRaisesRegex(ValueError, "cannot shrink exact-source CI"):
                selected_preparations(selected)


if __name__ == "__main__":
    unittest.main()
