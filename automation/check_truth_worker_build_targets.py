#!/usr/bin/env python3
"""Protect sequential, shared-target native Worker preparation and real E2E."""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "automation/run_truth_certification_gate.sh"
PREP = ROOT / "automation/run_truth_prepare.sh"
TRANSPORT = ROOT / "automation/run_qdrant_transport_gate.sh"
TRUTH_RUNTIME_TESTS = ROOT / "app/rust/crates/integration_harness/src/tests/truth_runtime_tests.rs"


def validate(source: str, prep: str, transport: str) -> None:
    required = (
        'SUITE_TARGET_DIR="${TRUTH_CERT_SUITE_TARGET_DIR:-$TARGET_ROOT}"',
        'TEMPORAL_BIN="$SUITE_TARGET_DIR/debug/temporal_worker"',
        'OUTBOX_BIN="$SUITE_TARGET_DIR/debug/outbox_worker"',
        'bash automation/run_truth_prepare.sh "$SUITE_TARGET_DIR"',
        'truth_suite cargo test --manifest-path app/rust/Cargo.toml',
        '--target-dir "$SUITE_TARGET_DIR"',
        'INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN"',
        'INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"',
        'if [[ ! -x "$worker_bin" ]]',
    )
    for marker in required:
        if marker not in source:
            raise ValueError(f"missing Truth shared-target or runtime contract: {marker}")
    if source.count('bash automation/run_truth_prepare.sh "$SUITE_TARGET_DIR"') != 1:
        raise ValueError("Truth preparation must run once")
    if source.index('bash automation/run_truth_prepare.sh') > source.index('run_suite() {'):
        raise ValueError("Workers must be ready before suite")
    prep_required = (
        'truth_workers_build cargo build --manifest-path app/rust/Cargo.toml',
        'bash automation/run_qdrant_transport_gate.sh "$TARGET_DIR"',
        'CI_TRUTH_TRANSPORT_REUSED=integration-e2e',
        'CI_TRUTH_PREP_SHARED_TARGET=PASS',
    )
    transport_required = (
        'truth_transport cargo test --manifest-path app/rust/Cargo.toml',
        'qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval',
    )
    for marker in prep_required:
        if marker not in prep:
            raise ValueError(f"missing real Worker or Qdrant preparation: {marker}")
    for marker in transport_required:
        if marker not in transport:
            raise ValueError(f"missing real Qdrant transport: {marker}")
    if prep.count('--target-dir "$TARGET_DIR"') != 1 or transport.count('--target-dir "$TARGET_DIR"') != 1:
        raise ValueError("all Cargo preparation must reuse the same Cargo target")
    if prep.count('--locked') != 1 or prep.count('--timings') != 1 or transport.count('--locked') != 1 or transport.count('--timings') != 1:
        raise ValueError("both phases must retain locked dependencies and timing")
    if prep.count('-p temporal_worker -p outbox_worker') != 1:
        raise ValueError("real Worker packages must build in one native Cargo invocation")
    if prep.count('--bin temporal_worker') != 1 or prep.count('--bin outbox_worker') != 1:
        raise ValueError("real Worker binary selection changed")
    if '--jobs' in prep or 'worker_build_pid' in prep or 'transport_pid' in prep:
        raise ValueError("parallel Cargo or arbitrary job budget must not return")


def validate_fixture_lifecycle(source: str) -> None:
    marker = "async fn truth_certification_suite_matches_fixture_expectations() {"
    if marker not in source:
        raise ValueError("missing full truth certification test")
    body = source.split(marker, 1)[1].split("\n    }", 1)[0]
    if "tokio::time::sleep(Duration::from_secs(3))" in body:
        raise ValueError("fixed per-fixture Worker startup sleep returned")
    required = (
        "for fixture in &fixtures {",
        "spawn_temporal_worker(&worker_env)",
        "spawn_outbox_worker(&worker_env)",
        "handle.get_result(WorkflowGetResultOptions::default())",
        "Duration::from_secs(180)",
        "collect_truth_certification_fixture_report(",
        "worker.child.try_wait().unwrap().is_none()",
        "outbox_worker.child.try_wait().unwrap().is_none()",
        "CI_TRUTH_FIXTURE_TIME source_sha={} fixture_id=",
        "workflow_started.duration_since(fixture_started)",
        "workflow_finished.duration_since(workflow_started)",
    )
    for item in required:
        if item not in body:
            raise ValueError(f"missing truth fixture lifecycle contract: {item}")
    if body.count("\n                worker.child.try_wait().unwrap().is_none()") != 2:
        raise ValueError("temporal worker must be checked before and after each workflow")
    if body.count("\n                outbox_worker.child.try_wait().unwrap().is_none()") != 2:
        raise ValueError("outbox worker must be checked before and after each workflow")


class TruthWorkerTargetContract(unittest.TestCase):
    def test_actual_shared_target_contract(self) -> None:
        validate(GATE.read_text(), PREP.read_text(), TRANSPORT.read_text())

    def test_separate_targets_are_rejected(self) -> None:
        changed = PREP.read_text().replace('--target-dir "$TARGET_DIR"', '--target-dir "$OTHER_TARGET"', 1)
        with self.assertRaisesRegex(ValueError, "same Cargo target"):
            validate(GATE.read_text(), changed, TRANSPORT.read_text())

    def test_missing_worker_binary_override_is_rejected(self) -> None:
        changed = GATE.read_text().replace('INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"', 'INTEGRATION_OUTBOX_WORKER_BIN=""')
        with self.assertRaisesRegex(ValueError, "runtime contract"):
            validate(changed, PREP.read_text(), TRANSPORT.read_text())

    def test_missing_cargo_timing_or_unlocked_dependency_is_rejected(self) -> None:
        prep = PREP.read_text()
        for changed in (prep.replace('--timings', '', 1), prep.replace('--locked', '', 1)):
            with self.assertRaisesRegex(ValueError, "timing"):
                validate(GATE.read_text(), changed, TRANSPORT.read_text())
        transport = TRANSPORT.read_text()
        for changed in (transport.replace('--timings', '', 1), transport.replace('--locked', '', 1)):
            with self.assertRaisesRegex(ValueError, "timing"):
                validate(GATE.read_text(), prep, changed)

    @staticmethod
    def invoke_stubbed_prep(root: Path, fail: str = "") -> subprocess.CompletedProcess[str]:
        cargo = root / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import os, sys, time\n"
            "args = sys.argv[1:]\n"
            "name = 'workers' if args[0] == 'build' else 'integration_harness'\n"
            "if '--locked' not in args or '--timings' not in args: sys.exit(87)\n"
            "if name == 'workers' and not all(x in args for x in ('temporal_worker','outbox_worker')): sys.exit(86)\n"
            "if args[args.index('--target-dir') + 1] != os.environ['TRUTH_TEST_TARGET']: sys.exit(88)\n"
            "with open(os.environ['TRUTH_TEST_EVENTS'], 'a') as f: f.write(f'{name} start\\n')\n"
            "time.sleep(0.005)\n"
            "with open(os.environ['TRUTH_TEST_EVENTS'], 'a') as f: f.write(f'{name} end\\n')\n"
            "sys.exit(23 if os.environ.get('TRUTH_TEST_FAIL') == name else 0)\n"
        )
        cargo.chmod(0o755)
        target = str(root / "shared-target")
        return subprocess.run(
            ["bash", str(PREP), target], cwd=ROOT,
            env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                 "TRUTH_TEST_EVENTS": str(root / "events.txt"),
                 "TRUTH_TEST_TARGET": target,
                 "TRUTH_TEST_FAIL": fail,
                 "CI_QDRANT_TRANSPORT_IN_INTEGRATION": "false",
                 "CI_SOURCE_SHA": "a" * 40,
                 "CI_TIMING_REPORT_PATH": str(root / "timings.jsonl")},
            capture_output=True, text=True, check=False,
        )

    def test_both_cargo_invocations_are_sequential_with_shared_target(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = self.invoke_stubbed_prep(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("CI_TRUTH_PREP_SHARED_TARGET=PASS", result.stdout)
            self.assertEqual((root / "events.txt").read_text().splitlines(), [
                "workers start", "workers end",
                "integration_harness start", "integration_harness end",
            ])
            timings = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
            self.assertEqual([item["phase"] for item in timings], [
                "truth_workers_build", "truth_transport",
            ])
            self.assertTrue(all(item["exit_code"] == 0 for item in timings))

    def test_each_failure_blocks_remaining_commands(self) -> None:
        stages = ("workers", "integration_harness")
        for stage in stages:
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                result = self.invoke_stubbed_prep(root, fail=stage)
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertNotIn("CI_TRUTH_PREP_SHARED_TARGET=PASS", result.stdout)
                observed = (root / "events.txt").read_text().splitlines()
                self.assertEqual([line for line in observed if line.endswith('start')], [
                    f"{name} start" for name in stages[:stages.index(stage) + 1]
                ])
                timings = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
                self.assertEqual(timings[-1]["exit_code"], 23)

    def test_ci_truth_reuses_real_transport_from_required_integration_owner(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cargo = root / "cargo"
            cargo.write_text("#!/bin/sh\nif [ \"$1\" = build ]; then exit 0; fi\nexit 19\n")
            cargo.chmod(0o755)
            result = subprocess.run(
                ["bash", str(PREP), str(root / "shared-target")], cwd=ROOT,
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                     "CI_QDRANT_TRANSPORT_IN_INTEGRATION": "true", "CI_SOURCE_SHA": "a" * 40,
                     "CI_TIMING_REPORT_PATH": str(root / "timings.jsonl")},
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("CI_TRUTH_TRANSPORT_REUSED=integration-e2e", result.stdout)
            entries = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
            self.assertEqual([entry["phase"] for entry in entries], ["truth_workers_build"])

    def test_real_transport_owned_by_integration_propagates_cargo_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cargo = root / "cargo"
            cargo.write_text(
                "#!/bin/sh\n"
                'test "$1" = test || exit 89\n'
                'test "$2" = --manifest-path || exit 88\n'
                'printf "%s\\n" "$@" > "$TRUTH_TEST_EVENTS"\n'
                "exit 19\n"
            )
            cargo.chmod(0o755)
            result = subprocess.run(
                ["bash", str(TRANSPORT), str(root / "shared-target")], cwd=ROOT,
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                     "TRUTH_TEST_EVENTS": str(root / "args.txt"), "CI_SOURCE_SHA": "a" * 40,
                     "CI_TIMING_REPORT_PATH": str(root / "timings.jsonl")},
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 19, result.stderr)
            self.assertIn("CI_PHASE_TIME phase=truth_transport", result.stdout)
            arguments = (root / "args.txt").read_text().splitlines()
            self.assertIn("qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval", arguments)
            self.assertIn(str(root / "shared-target"), arguments)
            record = json.loads((root / "timings.jsonl").read_text())
            self.assertEqual((record["phase"], record["exit_code"]), ("truth_transport", 19))


class TruthFixtureLifecycleContract(unittest.TestCase):
    def test_actual_fixture_lifecycle_has_timing_and_liveness(self) -> None:
        validate_fixture_lifecycle(TRUTH_RUNTIME_TESTS.read_text())

    def test_reintroduced_fixed_sleep_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text()
        changed = source.replace(
            "let mut worker = spawn_temporal_worker(&worker_env);",
            "let mut worker = spawn_temporal_worker(&worker_env);\n"
            "            tokio::time::sleep(Duration::from_secs(3)).await;", 1,
        )
        with self.assertRaisesRegex(ValueError, "fixed per-fixture"):
            validate_fixture_lifecycle(changed)

    def test_missing_post_workflow_liveness_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text()
        changed = source.replace(
            'outbox_worker.child.try_wait().unwrap().is_none(),\n                "outbox_worker exited during fixture',
            'outbox_worker.child.try_wait().unwrap().is_some(),\n                "outbox_worker exited during fixture', 1,
        )
        with self.assertRaisesRegex(ValueError, "before and after"):
            validate_fixture_lifecycle(changed)

    def test_missing_per_fixture_timing_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text()
        changed = source.replace("CI_TRUTH_FIXTURE_TIME source_sha={} fixture_id=", "CI_TRUTH_TIME removed=", 1)
        with self.assertRaisesRegex(ValueError, "missing truth fixture lifecycle"):
            validate_fixture_lifecycle(changed)


if __name__ == "__main__":
    unittest.main()
