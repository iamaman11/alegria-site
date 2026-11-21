#!/usr/bin/env python3
"""Prove Truth Workers share compilation, not runtime/harness state."""
from __future__ import annotations

import os
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "automation" / "run_truth_certification_gate.sh"
PREP = ROOT / "automation" / "run_truth_build_transport_parallel.sh"
TRUTH_RUNTIME_TESTS = ROOT / "app/rust/crates/integration_harness/src/tests/truth_runtime_tests.rs"


def validate(source: str, prep: str) -> None:
    required = (
        'WORKERS_TARGET_DIR="$NESTED_TARGET_ROOT/workers"',
        'TEMPORAL_BIN="$WORKERS_TARGET_DIR/debug/temporal_worker"',
        'OUTBOX_BIN="$WORKERS_TARGET_DIR/debug/outbox_worker"',
        'bash automation/run_truth_build_transport_parallel.sh "$WORKERS_TARGET_DIR" "$SUITE_TARGET_DIR"',
        'truth_suite cargo test --manifest-path app/rust/Cargo.toml',
        'INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN"',
        'INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"',
        'if [[ ! -x "$worker_bin" ]]',
        '"$(realpath -m "$WORKERS_TARGET_DIR")" == "$(realpath -m "$SUITE_TARGET_DIR")"',
    )
    for marker in required:
        if marker not in source:
            raise ValueError(f"missing Truth runtime/build isolation: {marker}")
    if source.count('bash automation/run_truth_build_transport_parallel.sh "$WORKERS_TARGET_DIR" "$SUITE_TARGET_DIR"') != 1:
        raise ValueError("parallel Truth preparation must run exactly once")
    if source.index('bash automation/run_truth_build_transport_parallel.sh') > source.index('run_suite() {'):
        raise ValueError("transport and Worker preparation must finish before Truth suite")
    prep_required = (
        'truth_worker_build cargo build --manifest-path app/rust/Cargo.toml',
        'truth_outbox_build cargo build --manifest-path app/rust/Cargo.toml',
        'truth_transport cargo test --manifest-path app/rust/Cargo.toml',
        'qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval',
        'worker_build_pid=$!',
        'transport_pid=$!',
        'wait "$worker_build_pid"',
        'wait "$transport_pid"',
        'if [[ "$worker_rc" -ne 0 ]]; then exit "$worker_rc"; fi',
        'if [[ "$transport_rc" -ne 0 ]]; then exit "$transport_rc"; fi',
        'CI_TRUTH_PREP_PARALLEL',
    )
    for marker in prep_required:
        if marker not in prep:
            raise ValueError(f"missing parallel Truth preparation/failure gate: {marker}")
    if prep.count('--target-dir "$WORKERS_TARGET_DIR"') != 2:
        raise ValueError("both Worker builds must use the same Cargo target")
    if prep.count('--target-dir "$SUITE_TARGET_DIR"') != 1 or source.count('--target-dir "$SUITE_TARGET_DIR"') != 1:
        raise ValueError("transport and full suite must use their isolated Cargo target")
    if prep.count('  --bin temporal_worker') != 1 or prep.count('  --bin outbox_worker') != 1:
        raise ValueError("real Worker binary selection was changed")


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
    def test_actual_workflow_keeps_binaries_and_harness_isolated(self) -> None:
        validate(GATE.read_text(encoding="utf-8"), PREP.read_text(encoding="utf-8"))

    def test_separate_worker_target_regression_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = PREP.read_text(encoding="utf-8").replace(
            '--target-dir "$WORKERS_TARGET_DIR"', '--target-dir "$NESTED_TARGET_ROOT/outbox"', 1
        )
        with self.assertRaisesRegex(ValueError, "same Cargo target"):
            validate(source, changed)

    def test_shared_integration_harness_target_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = PREP.read_text(encoding="utf-8").replace(
            '--target-dir "$SUITE_TARGET_DIR"', '--target-dir "$WORKERS_TARGET_DIR"', 1
        )
        with self.assertRaisesRegex(ValueError, "(same|isolated) Cargo target"):
            validate(source, changed)

    def test_missing_worker_binary_override_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = source.replace('INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"', 'INTEGRATION_OUTBOX_WORKER_BIN=""')
        with self.assertRaisesRegex(ValueError, "runtime/build isolation"):
            validate(changed, PREP.read_text(encoding="utf-8"))

    @staticmethod
    def invoke_stubbed_parallel_prep(root: Path, *, fail: str = "") -> subprocess.CompletedProcess[str]:
        fake_cargo = root / "cargo"
        fake_cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import os, sys, time\n"
            "args = sys.argv\n"
            "name = args[args.index('-p') + 1]\n"
            "events = os.environ['TRUTH_TEST_EVENTS']\n"
            "with open(events, 'a') as f: f.write(f'{name} start {time.monotonic_ns()}\\n')\n"
            "time.sleep({'temporal_worker': 0.18, 'outbox_worker': 0.04, 'integration_harness': 0.23}[name])\n"
            "with open(events, 'a') as f: f.write(f'{name} end {time.monotonic_ns()}\\n')\n"
            "sys.exit(23 if os.environ.get('TRUTH_TEST_FAIL') == name else 0)\n"
        )
        fake_cargo.chmod(0o755)
        return subprocess.run(
            ["bash", str(PREP), str(root / "worker-target"), str(root / "suite-target")],
            cwd=ROOT,
            env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                 "TRUTH_TEST_EVENTS": str(root / "events.txt"),
                 "TRUTH_TEST_FAIL": fail, "CI_SOURCE_SHA": "a" * 40,
                 "CI_TIMING_REPORT_PATH": str(root / "timings.jsonl")},
            capture_output=True, text=True, check=False,
        )

    def test_parallel_cargo_targets_overlap_and_all_three_phases_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = self.invoke_stubbed_parallel_prep(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("worker_exit=0 transport_exit=0", result.stdout)
            timings = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
            self.assertEqual({item["phase"] for item in timings}, {
                "truth_worker_build", "truth_outbox_build", "truth_transport"
            })
            self.assertTrue(all(item["exit_code"] == 0 for item in timings))
            events = {
                (name, event): int(when)
                for name, event, when in (line.split() for line in (root / "events.txt").read_text().splitlines())
            }
            self.assertLess(events[("integration_harness", "start")], events[("temporal_worker", "end")])
            self.assertLess(events[("outbox_worker", "start")], events[("integration_harness", "end")])

    def test_failed_transport_blocks_truth_gate_even_when_workers_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = self.invoke_stubbed_parallel_prep(root, fail="integration_harness")
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertIn("worker_exit=0 transport_exit=23", result.stdout)
            timings = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
            self.assertEqual(next(t["exit_code"] for t in timings if t["phase"] == "truth_transport"), 23)

    def test_failed_temporal_worker_blocks_outbox_and_truth_suite(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = self.invoke_stubbed_parallel_prep(root, fail="temporal_worker")
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertIn("worker_exit=23 transport_exit=0", result.stdout)
            events = (root / "events.txt").read_text()
            self.assertNotIn("outbox_worker start", events)

    def test_failed_outbox_worker_cannot_be_hidden_by_transport_success(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = self.invoke_stubbed_parallel_prep(root, fail="outbox_worker")
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertIn("worker_exit=23 transport_exit=0", result.stdout)
            timings = [json.loads(line) for line in (root / "timings.jsonl").read_text().splitlines()]
            self.assertEqual(next(t["exit_code"] for t in timings if t["phase"] == "truth_outbox_build"), 23)

    def test_aliasing_targets_fails_before_cargo_or_side_effects(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            nested = Path(temp) / "nested"
            result = subprocess.run(
                ["bash", str(GATE)],
                cwd=ROOT,
                env={**os.environ, "TRUTH_CERT_NESTED_TARGET_ROOT": str(nested),
                     "TRUTH_CERT_SUITE_TARGET_DIR": str(nested / "workers"),
                     "TRUTH_CERT_REPORT_PATH": str(Path(temp) / "report.json")},
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("must differ", result.stderr)
            self.assertFalse((Path(temp) / "report.json").exists())


class TruthFixtureLifecycleContract(unittest.TestCase):
    def test_actual_fixture_lifecycle_has_timing_and_liveness(self) -> None:
        validate_fixture_lifecycle(TRUTH_RUNTIME_TESTS.read_text(encoding="utf-8"))

    def test_reintroduced_fixed_sleep_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text(encoding="utf-8")
        changed = source.replace(
            "let mut worker = spawn_temporal_worker(&worker_env);",
            "let mut worker = spawn_temporal_worker(&worker_env);\n"
            "            tokio::time::sleep(Duration::from_secs(3)).await;",
            1,
        )
        with self.assertRaisesRegex(ValueError, "fixed per-fixture"):
            validate_fixture_lifecycle(changed)

    def test_missing_post_workflow_liveness_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text(encoding="utf-8")
        changed = source.replace(
            'outbox_worker.child.try_wait().unwrap().is_none(),\n                "outbox_worker exited during fixture',
            'outbox_worker.child.try_wait().unwrap().is_some(),\n                "outbox_worker exited during fixture',
            1,
        )
        with self.assertRaisesRegex(ValueError, "before and after"):
            validate_fixture_lifecycle(changed)

    def test_missing_per_fixture_timing_is_rejected(self) -> None:
        source = TRUTH_RUNTIME_TESTS.read_text(encoding="utf-8")
        changed = source.replace("CI_TRUTH_FIXTURE_TIME source_sha={} fixture_id=", "CI_TRUTH_TIME removed=", 1)
        with self.assertRaisesRegex(ValueError, "missing truth fixture lifecycle"):
            validate_fixture_lifecycle(changed)


if __name__ == "__main__":
    unittest.main()
