#!/usr/bin/env python3
"""No-Cargo regression tests for streamed, fail-closed Rust build evidence."""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from cargo_feedback import run_cargo, workspace_artifact
from ci_impact import (
    SCHEMA_VERSION, SUITE_OWNERS, WORKER_E2E_PREPARE, OUTBOX_E2E_PREPARE,
    suite_commands_by_ci_job, suite_commands_for_report,
)
from run_scoped_cargo_feedback import (
    E2E_COMPILE, PG_E2E_COMPILE, OUTBOX_E2E_COMPILE,
    NEW_E2E_ARTIFACT_OWNERS, NEW_E2E_COMPILES,
    cargo_commands_for_ci, exact_verify_e2e_filter,
    main as run_scoped_cargo_feedback, precompiled_verify_executable,
    run_exact_precompiled_e2e_shard, run_precompiled_e2e_suites,
    run_precompiled_e2e_suite, selected_preparations, suite_runtime_environment,
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

    def test_no_run_cargo_artifact_records_the_exact_library_test_binary(self) -> None:
        executable = str(WORKSPACE / "target/debug/deps/integration_harness-exact-hash")
        record = json.dumps({
            "reason": "compiler-artifact", "fresh": False,
            "target": {
                "name": "integration_harness", "kind": ["lib"],
                "src_path": str(WORKSPACE / "crates/integration_harness/src/lib.rs"),
            },
            "executable": executable,
        }) + "\n"
        with patch("cargo_feedback.subprocess.Popen", return_value=FakeProcess([
            record, json.dumps({"reason": "build-finished", "success": True}) + "\n",
        ])):
            result = run_cargo(E2E_COMPILE, cwd=WORKSPACE, output=io.StringIO())
        self.assertEqual(result["test_executables"], [
            {"target": "integration_harness", "path": executable},
        ])


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
        self, suites: list[str], *, delegated: bool = False,
        suite_job: str = "architecture-and-rust", scope: str = "full",
    ) -> tuple[list[tuple[list[str], dict]], str, dict]:
        report = self.report(suites)
        report["analysis"] = scope
        executions: list[tuple[list[str], dict]] = []

        def record(command: list[str], **kwargs: object) -> dict:
            executions.append((command, kwargs))
            return {"command": command, "exit_code": 0}

        def precompiled(_report: dict, assigned: list[tuple[str, list[str]]], *,
                        suite_job: str) -> list[dict]:
            executions.append((["precompiled-e2e-execution"], {
                "suite_job": suite_job, "selected": [name for name, _ in assigned],
            }))
            return [{"compiled_once": True, "selected": [name for name, _ in assigned]}]

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
                patch("run_scoped_cargo_feedback.run_precompiled_e2e_suites", side_effect=precompiled),
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
            ["precompiled-e2e-execution"],
        ]
        self.assertEqual([command for command, _ in executions], expected)
        self.assertEqual(len(measurements["measurements"]), len(expected))
        self.assertEqual(output.count("CI_SUITE_PREPARE=truth_certification COMMAND="), 1)
        self.assertIn("CI_SUITE_PREPARE_REUSED=temporal_restart", output)
        self.assertEqual(executions[-1][1], {
            "suite_job": "architecture-and-rust",
            "selected": [name for name, _ in commands],
        })
        self.assertTrue(all(kwargs.get("env") is None for _, kwargs in executions[:2]))

    def test_e2e_without_runtime_owner_does_not_build_workers(self) -> None:
        suites = ["cms_review_revision", "knowledge_a13", "publishing_a17"]
        executions, output, _ = self.run_plan(suites)
        self.assertEqual([command for command, _ in executions], [
            ["cargo", "test", "--workspace", "--locked"],
            *(command for _, command in suite_commands_by_ci_job(self.report(suites))["architecture-and-rust"]),
        ])
        self.assertNotIn("CI_SUITE_PREPARE=", output)
        self.assertNotIn("CI_SUITE_PREPARE_REUSED=", output)

    def test_pure_dag_v3_prepares_only_the_new_worker_in_exact_source_target(self) -> None:
        from ci_impact import PURE_V3_E2E_PREPARE

        suites = ["pure_dag_v3"]
        executions, output, _ = self.run_plan(suites, scope="scoped")
        self.assertEqual([command for command, _ in executions], [
            ["cargo", "test", "--workspace", "--locked"],
            list(PURE_V3_E2E_PREPARE),
            list(SUITE_OWNERS["pure_dag_v3"]["command"]),
        ])
        self.assertIn("CI_SUITE_PREPARE=pure_dag_v3 COMMAND=", output)
        self.assertIn("CI_SUITE_PREBUILT=pure_dag_v3", output)
        self.assertNotIn("INTEGRATION_TEMPORAL_WORKER_BIN", output)
        self.assertNotIn("INTEGRATION_OUTBOX_WORKER_BIN", output)

    def test_pg_v4_parent_child_operator_cli_requires_both_v3_owner_binaries_no_monolith(self) -> None:
        suites = ["pg_effect_v3"]
        executions, output, measurements = self.run_plan(suites, scope="scoped")
        self.assertEqual([command for command, _ in executions], [
            ["cargo", "test", "--workspace", "--locked"],
            list(SUITE_OWNERS["pg_effect_v3"]["prepare"]),
            list(SUITE_OWNERS["pg_effect_v3"]["command"]),
        ])
        self.assertEqual(len(measurements["measurements"]), 3)
        self.assertIn("pg_v3_", output)
        self.assertIn("CI_SUITE_PREPARE=pg_effect_v3 COMMAND=", output)
        self.assertIn("CI_SUITE_PREBUILT=pg_effect_v3 INTEGRATION_PG_EFFECT_WORKER_BIN=", output)
        self.assertIn("CI_SUITE_PREBUILT=pg_effect_v3 INTEGRATION_EFFECT_DAG_WORKER_BIN=", output)
        self.assertNotIn("INTEGRATION_TEMPORAL_WORKER_BIN", output)
        self.assertNotIn("INTEGRATION_OUTBOX_WORKER_BIN", output)

    def test_live_v4_pg_shard_fails_closed_when_parent_binary_was_not_installed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "target"
            runtime = target / "debug"
            runtime.mkdir(parents=True)
            pg = runtime / "temporal_pg_effects"
            pg.write_bytes(b"exact PG child runtime")
            pg.chmod(0o755)
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}):
                with self.assertRaisesRegex(FileNotFoundError, "temporal_orchestrator"):
                    suite_runtime_environment(self.report(["pg_effect_v3"]), "pg_effect_v3")
                pure = runtime / "temporal_orchestrator"
                pure.write_bytes(b"exact parent runtime")
                pure.chmod(0o755)
                with patch("sys.stdout", new_callable=io.StringIO):
                    env = suite_runtime_environment(self.report(["pg_effect_v3"]), "pg_effect_v3")
                self.assertEqual(env["INTEGRATION_PG_EFFECT_WORKER_BIN"], str(pg))
                self.assertEqual(env["INTEGRATION_EFFECT_DAG_WORKER_BIN"], str(pure))

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
        self.assertEqual([command for command, _ in executions], [
            list(OUTBOX_E2E_PREPARE), *(command for _, command in assigned),
        ])
        self.assertEqual({name for name, _ in assigned}, {
            "cms_review_revision", "draft_qa_scenario", "knowledge_a13", "outbox_a14",
            "planning_atomicity", "publishing_a17", "truth_adjudication_integrity",
        })
        self.assertIn("CI_SUITE_PREPARE=outbox_a14", output)
        self.assertIn("CI_SUITE_PREBUILT=outbox_a14 INTEGRATION_OUTBOX_WORKER_BIN=", output)
        self.assertNotIn("CI_WORKSPACE_UNIT_TESTS=", output)
        self.assertEqual(evidence["suite_job"], "verify")
        self.assertEqual(evidence["selected_suites"], [name for name, _ in assigned])
        self.assertEqual(len(evidence["measurements"]), len(assigned) + 1)

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

    def test_parallel_verify_uses_only_exact_selected_suite_filters(self) -> None:
        assigned = suite_commands_by_ci_job(self.report([
            "cms_review_revision", "draft_qa_scenario", "knowledge_a13",
        ]))["verify"]
        self.assertEqual([exact_verify_e2e_filter(command) for _, command in assigned], [
            "cms_revision_", "draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness",
            "knowledge_a13_",
        ])
        with self.assertRaisesRegex(ValueError, "exact, locked"):
            exact_verify_e2e_filter([
                "cargo", "test", "-p", "integration_harness", "--lib", "cms_revision_",
            ])

    def test_parallel_verify_fails_closed_on_missing_or_outside_binary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target"
            binary = target / "debug/deps/integration_harness-correct"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"compiled test binary")
            binary.chmod(0o755)
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}):
                self.assertEqual(precompiled_verify_executable({
                    "test_executables": [{"target": "integration_harness", "path": str(binary)}],
                }), binary)
                # This is the exact alternative layout observed in GitHub CI:
                # Running unittests src/lib.rs (target/debug/build/integration_harness/<hash>/out/<bin>)
                cargo_build_binary = (target / "debug/build/integration_harness"
                                      / "b5e3e249052aa1c9/out/integration_harness-correct")
                cargo_build_binary.parent.mkdir(parents=True)
                cargo_build_binary.write_bytes(b"compiled test binary")
                cargo_build_binary.chmod(0o755)
                self.assertEqual(precompiled_verify_executable({
                    "test_executables": [{"target": "integration_harness", "path": str(cargo_build_binary)}],
                }), cargo_build_binary)
                with self.assertRaisesRegex(RuntimeError, "expected one"):
                    precompiled_verify_executable({"test_executables": []})
                with self.assertRaisesRegex(RuntimeError, "unusable"):
                    precompiled_verify_executable({"test_executables": [
                        {"target": "integration_harness", "path": "/tmp/foreign-test"},
                    ]})
                wrong_crate = (target / "debug/build/outbox_worker"
                               / "b5e3e249052aa1c9/out/integration_harness-correct")
                wrong_crate.parent.mkdir(parents=True)
                wrong_crate.write_bytes(b"not integration harness")
                wrong_crate.chmod(0o755)
                with self.assertRaisesRegex(RuntimeError, "unusable"):
                    precompiled_verify_executable({"test_executables": [
                        {"target": "integration_harness", "path": str(wrong_crate)},
                    ]})

    def test_pg_v3_owner_cargo_command_and_compiler_report_are_exactly_typed(self) -> None:
        self.assertEqual(PG_E2E_COMPILE, [
            "cargo", "test", "-p", "integration_pg_v3_e2e", "--features", "e2e",
            "--lib", "--locked", "--no-run",
        ])
        for name, selected_filter in (("pg_effect_v3", "pg_v3_"),
                                      ("pg_temporal_v3", "temporal_effect_v3_")):
            command = list(SUITE_OWNERS[name]["command"])
            self.assertEqual(command[3], "integration_pg_v3_e2e")
            self.assertEqual(exact_verify_e2e_filter(command), selected_filter)
        with self.assertRaisesRegex(ValueError, "exact, locked"):
            exact_verify_e2e_filter([
                "cargo", "test", "-p", "integration_pg_v3_e2e", "--lib",
                "pg_v3_", "--locked", "--", "--nocapture",
            ])
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "target"
            binary = target / "debug/deps/integration_pg_v3_e2e-testhash"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"PG V3 isolated test binary")
            binary.chmod(0o755)
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}):
                self.assertEqual(precompiled_verify_executable({"test_executables": [
                    {"target": "integration_pg_v3_e2e", "path": str(binary)},
                ]}, package="integration_pg_v3_e2e"), binary)
                with self.assertRaisesRegex(RuntimeError, "expected one"):
                    precompiled_verify_executable({"test_executables": [
                        {"target": "integration_pg_v3_e2e", "path": str(binary)},
                    ]}, package="integration_harness")
                with self.assertRaisesRegex(ValueError, "unsupported"):
                    precompiled_verify_executable({}, package="unreviewed_test_owner")

    def test_new_physical_owner_compilers_have_distinct_locked_test_targets(self) -> None:
        self.assertEqual(NEW_E2E_ARTIFACT_OWNERS, {
            "graph_harness": "integration_graph_e2e",
            "qdrant_harness": "integration_qdrant_e2e",
            "qdrant_temporal_harness": "integration_qdrant_temporal_e2e",
            "temporal_harness": "integration_temporal_e2e",
            "pg_app_harness": "integration_pg_app_e2e",
        })
        for package in NEW_E2E_ARTIFACT_OWNERS.values():
            with self.subTest(package=package):
                self.assertEqual(NEW_E2E_COMPILES[package], [
                    "cargo", "test", "-p", package, "--features", "e2e",
                    "--lib", "--locked", "--no-run",
                ])
                filter_command = [
                    "cargo", "test", "-p", package, "--features", "e2e",
                    "--lib", "true_real_owner_", "--locked", "--", "--nocapture",
                ]
                self.assertEqual(exact_verify_e2e_filter(filter_command), "true_real_owner_")
                for corrupt in (
                    filter_command[:7] + ["", *filter_command[8:]],
                    filter_command[:7] + ["--skip", *filter_command[8:]],
                    filter_command[:9] + ["--ignored", "--nocapture"],
                ):
                    with self.assertRaisesRegex(ValueError, "exact, locked"):
                        exact_verify_e2e_filter(corrupt)

    def test_outbox_a14_compiler_is_independent_and_keeps_nine_exact_tests(self) -> None:
        self.assertEqual(OUTBOX_E2E_COMPILE, [
            "cargo", "test", "-p", "integration_outbox_e2e", "--features", "e2e",
            "--lib", "--locked", "--no-run",
        ])
        command = list(SUITE_OWNERS["outbox_a14"]["command"])
        self.assertEqual(exact_verify_e2e_filter(command), "outbox_a14_")
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "target"
            binary = target / "debug/deps/integration_outbox_e2e-testhash"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"real independent A14 test binary")
            binary.chmod(0o755)
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}):
                self.assertEqual(precompiled_verify_executable({"test_executables": [
                    {"target": "integration_outbox_e2e", "path": str(binary)},
                ]}, package="integration_outbox_e2e"), binary)
                with self.assertRaisesRegex(RuntimeError, "expected one"):
                    precompiled_verify_executable({"test_executables": [
                        {"target": "integration_outbox_e2e", "path": str(binary)},
                    ]}, package="integration_harness")

    def test_outbox_shard_cannot_execute_broad_harness_or_accept_short_test_count(self) -> None:
        report = self.report(["outbox_a14"])
        report["analysis"] = "full"
        command = list(SUITE_OWNERS["outbox_a14"]["command"])
        selected = [("outbox_a14", command, "outbox_a14_", 9)]
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "app/rust"
            broad = workspace / "target/debug/deps/integration_harness-ci-exact"
            outbox = workspace / "target/debug/deps/integration_outbox_e2e-ci-exact"
            outbox.parent.mkdir(parents=True)
            broad.write_bytes(b"wrong broad harness")
            outbox.write_bytes(b"real A14 harness")
            broad.chmod(0o755)
            outbox.chmod(0o755)
            with (
                patch("run_scoped_cargo_feedback.WORKSPACE", workspace),
                patch("run_scoped_cargo_feedback.e2e_shard_commands_for_report",
                      return_value={"outbox": selected}),
                patch("run_scoped_cargo_feedback.suite_runtime_environment", return_value={}),
                patch("run_scoped_cargo_feedback.run_precompiled_e2e_suite",
                      return_value={"matched_tests": 9}) as run,
            ):
                with self.assertRaisesRegex(ValueError, "only installed verified"):
                    run_exact_precompiled_e2e_shard(
                        report, "outbox", suite_job="verify",
                        prebuilt_executable=broad,
                        prebuilt_outbox_executable=broad,
                    )
                run.assert_not_called()
                result = run_exact_precompiled_e2e_shard(
                    report, "outbox", suite_job="verify",
                    prebuilt_executable=broad,
                    prebuilt_outbox_executable=outbox,
                )
                self.assertEqual(result[0]["matched_tests"], 9)
                self.assertEqual(run.call_args.kwargs["expected_tests"], 9)
                self.assertEqual(run.call_args.args[2], outbox)
                run.return_value = {"matched_tests": 8}
                with self.assertRaisesRegex(RuntimeError, "mismatched required coverage"):
                    run_exact_precompiled_e2e_shard(
                        report, "outbox", suite_job="verify",
                        prebuilt_executable=broad,
                        prebuilt_outbox_executable=outbox,
                    )

    def test_precompiled_filter_is_process_isolated_and_cannot_pass_zero_tests(self) -> None:
        command = list(SUITE_OWNERS["cms_review_revision"]["command"])
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "integration_harness-exact"
            binary.write_bytes(b"compiled test binary")
            binary.chmod(0o755)
            captured: list[tuple[list[str], dict]] = []

            def run(command: list[str], **kwargs: object) -> FakeProcess:
                captured.append((command, kwargs))
                return FakeProcess(["running 4 tests\n", "test result: ok. 4 passed; 0 failed; finished\n"])

            with patch("run_scoped_cargo_feedback.subprocess.Popen", side_effect=run):
                result = run_precompiled_e2e_suite(
                    "cms_review_revision", command, binary,
                    {"TEST_ONLY_SUITE_ENV": "unique"}, threading.Lock(),
                )
            self.assertEqual(result["matched_tests"], 4)
            self.assertEqual(result["selected_cargo_command"], command)
            self.assertEqual(captured[0][0], [str(binary), "cms_revision_", "--nocapture"])
            self.assertEqual(captured[0][1]["env"]["TEST_ONLY_SUITE_ENV"], "unique")
            self.assertIn(str(binary.parent), captured[0][1]["env"]["LD_LIBRARY_PATH"])

            with patch("run_scoped_cargo_feedback.subprocess.Popen", return_value=FakeProcess([
                "running 0 tests\n", "test result: ok. 0 passed; 0 failed; finished\n",
            ])):
                with self.assertRaisesRegex(RuntimeError, "matched_tests=0"):
                    run_precompiled_e2e_suite(
                        "cms_review_revision", command, binary, {}, threading.Lock(),
                    )
            with patch("run_scoped_cargo_feedback.subprocess.Popen", return_value=FakeProcess([
                "running 4 tests\n", "test result: FAILED. 3 passed; 1 failed\n",
            ], exit_code=101)):
                with self.assertRaisesRegex(RuntimeError, "exit=101"):
                    run_precompiled_e2e_suite(
                        "cms_review_revision", command, binary, {}, threading.Lock(),
                    )

            # Full shards require the *exact* expected tests, not merely one
            # matching test. This catches an inadvertently narrowed prefix or
            # a newly registered test that a handwritten shard omitted.
            with patch("run_scoped_cargo_feedback.subprocess.Popen", return_value=FakeProcess([
                "running 4 tests\n", "test result: ok. 4 passed; 0 failed; finished\n",
            ])):
                with self.assertRaisesRegex(RuntimeError, "expected_tests=5"):
                    run_precompiled_e2e_suite(
                        "cms_review_revision", command, binary, {}, threading.Lock(),
                        test_filter="cms_revision_", expected_tests=5,
                    )
            with patch("run_scoped_cargo_feedback.subprocess.Popen") as forbidden:
                with self.assertRaisesRegex(ValueError, "must narrow"):
                    run_precompiled_e2e_suite(
                        "cms_review_revision", command, binary, {}, threading.Lock(),
                        test_filter="canonical_temporal_", expected_tests=1,
                    )
            forbidden.assert_not_called()

    def test_full_shard_executes_only_registry_owned_exact_filters_and_counts(self) -> None:
        report = self.report(["temporal_restart"])
        temporal = list(SUITE_OWNERS["temporal_restart"]["command"])
        selected = [
            ("temporal_restart", temporal,
             "canonical_temporal_manual_editorial_resumes_after_worker_restart", 1),
            ("temporal_restart", temporal,
             "canonical_temporal_activity_history_wire_shape", 1),
        ]
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "app/rust"
            binary = workspace / "target/debug/deps/integration_temporal_e2e-ci-exact"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"exact-source test bundle")
            binary.chmod(0o755)
            calls: list[tuple[str, str, int]] = []

            def run(name: str, command: list[str], *_args: object, **kwargs: object) -> dict:
                self.assertEqual(command, temporal)
                calls.append((name, kwargs["test_filter"], kwargs["expected_tests"]))
                return {"suite": name, "matched_tests": kwargs["expected_tests"]}

            with (
                patch("run_scoped_cargo_feedback.WORKSPACE", workspace),
                patch("run_scoped_cargo_feedback.e2e_shard_commands_for_report",
                      return_value={"temporal_manual": selected, "graph": []}),
                patch("run_scoped_cargo_feedback.suite_runtime_environment", return_value={}),
                patch("run_scoped_cargo_feedback.run_precompiled_e2e_suite", side_effect=run),
                patch("sys.stdout", new_callable=io.StringIO) as output,
            ):
                result = run_exact_precompiled_e2e_shard(
                    report, "temporal_manual", suite_job="architecture-and-rust",
                    prebuilt_executable=binary,
                    extra_prebuilt_executables={"integration_temporal_e2e": binary},
                )
                self.assertEqual(sum(row["matched_tests"] for row in result), 2)
                self.assertIn("CI_E2E_SHARD=PASS shard=temporal_manual filters=2 tests=2",
                              output.getvalue())
                with self.assertRaisesRegex(ValueError, "no selected real test"):
                    run_exact_precompiled_e2e_shard(
                        report, "graph", suite_job="architecture-and-rust",
                        prebuilt_executable=binary,
                    )
                with self.assertRaisesRegex(ValueError, "unrecognized immutable"):
                    run_exact_precompiled_e2e_shard(
                        report, "unknown", suite_job="architecture-and-rust",
                        prebuilt_executable=binary,
                    )
            self.assertEqual(calls, [(row[0], row[2], row[3]) for row in selected])

    def test_full_shard_rejects_duplicate_filters_and_wrong_owner_before_execution(self) -> None:
        report = self.report(["temporal_restart", "cms_review_revision"])
        temporal = list(SUITE_OWNERS["temporal_restart"]["command"])
        one = ("temporal_restart", temporal,
               "canonical_temporal_manual_editorial_resumes_after_worker_restart", 1)
        for chosen, expected_error in [
            ([one, one], "duplicate real test filter"),
            ([("cms_review_revision", list(SUITE_OWNERS["cms_review_revision"]["command"]),
               "cms_revision_", 4)], "non-owner or rewritten Cargo command"),
        ]:
            with (
                patch("run_scoped_cargo_feedback.e2e_shard_commands_for_report",
                      return_value={"temporal_manual": chosen}),
                patch("run_scoped_cargo_feedback.run_precompiled_e2e_suite") as run,
            ):
                with self.assertRaisesRegex(ValueError, expected_error):
                    run_exact_precompiled_e2e_shard(
                        report, "temporal_manual", suite_job="architecture-and-rust",
                        prebuilt_executable=Path("/tmp/never-reached"),
                    )
                run.assert_not_called()

    def test_pg_v3_full_shards_execute_exact_owner_binary_without_monolith_fallback(self) -> None:
        report = self.report(["pg_effect_v3", "pure_dag_v3", "pg_temporal_v3"])
        pg_command = list(SUITE_OWNERS["pg_effect_v3"]["command"])
        pure_command = list(SUITE_OWNERS["pure_dag_v3"]["command"])
        temporal_command = list(SUITE_OWNERS["pg_temporal_v3"]["command"])
        shards = {
            "verify_d": [("pg_effect_v3", pg_command, "pg_v3_", 10)],
            "pg_temporal": [("pg_temporal_v3", temporal_command, "temporal_effect_v3_", 1)],
            "temporal_pure": [("pure_dag_v3", pure_command, "pure_v3_", 1)],
        }
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "app/rust"
            debug = workspace / "target/debug/deps"
            debug.mkdir(parents=True)
            harness = debug / "integration_harness-ci-exact"
            temporal = debug / "integration_temporal_e2e-ci-exact"
            pg = debug / "integration_pg_v3_e2e-ci-exact"
            # Critically the PG-only job installs no monolithic test binary.
            # Both PG shards must pass before such an artifact even exists.
            pg.write_bytes(b"exact-source verified artifact")
            pg.chmod(0o755)
            actual: list[tuple[str, Path, str, int]] = []

            def execute(name: str, _command: list[str], executable: Path,
                        *_args: object, **kwargs: object) -> dict:
                actual.append((name, executable, kwargs["test_filter"], kwargs["expected_tests"]))
                return {"suite": name, "matched_tests": kwargs["expected_tests"]}

            with (
                patch("run_scoped_cargo_feedback.WORKSPACE", workspace),
                patch("run_scoped_cargo_feedback.e2e_shard_commands_for_report", return_value=shards),
                patch("run_scoped_cargo_feedback.suite_runtime_environment", return_value={}),
                patch("run_scoped_cargo_feedback.run_precompiled_e2e_suite", side_effect=execute),
                patch("sys.stdout", new_callable=io.StringIO),
            ):
                with self.assertRaisesRegex(ValueError, "verified integration_pg_v3_e2e"):
                    run_exact_precompiled_e2e_shard(
                        report, "verify_d", suite_job="verify", prebuilt_executable=harness,
                    )
                self.assertFalse(actual)
                verify = run_exact_precompiled_e2e_shard(
                    report, "verify_d", suite_job="verify", prebuilt_executable=harness,
                    prebuilt_pg_executable=pg,
                )
                self.assertEqual(sum(row["matched_tests"] for row in verify), 10)
                architecture = run_exact_precompiled_e2e_shard(
                    report, "pg_temporal", suite_job="architecture-and-rust",
                    prebuilt_executable=harness, prebuilt_pg_executable=pg,
                )
                self.assertEqual(sum(row["matched_tests"] for row in architecture), 1)
                self.assertFalse(harness.exists(), "PG owner must not consume broad harness")
                temporal.write_bytes(b"exact-source verified artifact")
                temporal.chmod(0o755)
                pure = run_exact_precompiled_e2e_shard(
                    report, "temporal_pure", suite_job="verify", prebuilt_executable=harness,
                    prebuilt_pg_executable=pg,
                    extra_prebuilt_executables={"integration_temporal_e2e": temporal},
                )
                self.assertEqual(sum(row["matched_tests"] for row in pure), 1)
                with self.assertRaisesRegex(ValueError, "verified integration_pg_v3_e2e"):
                    run_exact_precompiled_e2e_shard(
                        report, "pg_temporal", suite_job="architecture-and-rust",
                        prebuilt_executable=harness, prebuilt_pg_executable=harness,
                    )
                with self.assertRaisesRegex(ValueError, "mixed E2E owner binaries"):
                    run_precompiled_e2e_suites(
                        report, suite_commands_by_ci_job(report)["verify"], suite_job="verify",
                    )
            self.assertEqual(actual, [
                ("pg_effect_v3", pg, "pg_v3_", 10),
                ("pg_temporal_v3", pg, "temporal_effect_v3_", 1),
                ("pure_dag_v3", temporal, "pure_v3_", 1),
            ])

    def test_legacy_bulk_verify_rejects_mixed_physical_package_compilation(self) -> None:
        selected = ["cms_review_revision", "draft_qa_scenario", "knowledge_a13"]
        suites = suite_commands_by_ci_job(self.report(selected))["verify"]
        with patch("run_scoped_cargo_feedback.run_cargo") as compiler:
            with self.assertRaisesRegex(ValueError, "mixed E2E owner binaries"):
                run_precompiled_e2e_suites(
                    self.report(selected), suites, suite_job="verify",
                )
        compiler.assert_not_called()

    def test_legacy_bulk_architecture_rejects_mixed_physical_packages(self) -> None:
        selected = ["graph_query", "vector_projection", "temporal_restart"]
        suites = suite_commands_by_ci_job(self.report(selected))["architecture-and-rust"]
        with patch("run_scoped_cargo_feedback.run_cargo") as compiler:
            with self.assertRaisesRegex(ValueError, "mixed E2E owner binaries"):
                run_precompiled_e2e_suites(
                    self.report(selected), suites, suite_job="architecture-and-rust",
                )
        compiler.assert_not_called()

    def test_legacy_bulk_cannot_bypass_test_source_owner_validation(self) -> None:
        selected = ["graph_query", "temporal_restart"]
        suites = suite_commands_by_ci_job(self.report(selected))["architecture-and-rust"]
        with patch("run_scoped_cargo_feedback.run_cargo") as compiler:
            with self.assertRaisesRegex(ValueError, "mixed E2E owner binaries"):
                run_precompiled_e2e_suites(
                    self.report(selected), suites[:1] + [(
                        "vector_projection", list(SUITE_OWNERS["vector_projection"]["command"]),
                    )], suite_job="architecture-and-rust",
                )
        compiler.assert_not_called()

    def test_legacy_bulk_verify_cannot_turn_failed_or_duplicate_process_green(self) -> None:
        for selected, suites in (
            (["cms_review_revision", "draft_qa_scenario"], None),
            (["cms_review_revision"], [
                ("cms_review_revision", list(SUITE_OWNERS["cms_review_revision"]["command"])),
            ] * 2),
        ):
            with self.subTest(selected=selected):
                report = self.report(selected)
                if suites is None:
                    suites = suite_commands_by_ci_job(report)["verify"]
                with patch("run_scoped_cargo_feedback.run_cargo") as compiler:
                    with self.assertRaisesRegex(ValueError, "mixed E2E owner binaries"):
                        run_precompiled_e2e_suites(
                            report, suites, suite_job="verify",
                        )
                compiler.assert_not_called()


if __name__ == "__main__":
    unittest.main()
