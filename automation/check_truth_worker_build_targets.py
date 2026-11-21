#!/usr/bin/env python3
"""Prove Truth Workers share compilation, not runtime/harness state."""
from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "automation" / "run_truth_certification_gate.sh"
TRUTH_RUNTIME_TESTS = ROOT / "app/rust/crates/integration_harness/src/tests/truth_runtime_tests.rs"


def validate(source: str) -> None:
    required = (
        'WORKERS_TARGET_DIR="$NESTED_TARGET_ROOT/workers"',
        'TEMPORAL_BIN="$WORKERS_TARGET_DIR/debug/temporal_worker"',
        'OUTBOX_BIN="$WORKERS_TARGET_DIR/debug/outbox_worker"',
        'truth_worker_build cargo build --manifest-path app/rust/Cargo.toml',
        'truth_outbox_build cargo build --manifest-path app/rust/Cargo.toml',
        'truth_transport cargo test --manifest-path app/rust/Cargo.toml',
        'truth_suite cargo test --manifest-path app/rust/Cargo.toml',
        'INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN"',
        'INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"',
        'if [[ ! -x "$worker_bin" ]]',
        '"$(realpath -m "$WORKERS_TARGET_DIR")" == "$(realpath -m "$SUITE_TARGET_DIR")"',
    )
    for marker in required:
        if marker not in source:
            raise ValueError(f"missing Truth runtime/build isolation: {marker}")
    if source.count('--target-dir "$WORKERS_TARGET_DIR"') != 2:
        raise ValueError("both Worker builds must use the same Cargo target")
    if source.count('--target-dir "$SUITE_TARGET_DIR"') != 2:
        raise ValueError("transport and full suite must use their isolated Cargo target")
    if source.count('  --bin temporal_worker') != 1 or source.count('  --bin outbox_worker') != 1:
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
        validate(GATE.read_text(encoding="utf-8"))

    def test_separate_worker_target_regression_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = source.replace('--target-dir "$WORKERS_TARGET_DIR"', '--target-dir "$NESTED_TARGET_ROOT/outbox"', 1)
        with self.assertRaisesRegex(ValueError, "same Cargo target"):
            validate(changed)

    def test_shared_integration_harness_target_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = source.replace('--target-dir "$SUITE_TARGET_DIR"', '--target-dir "$WORKERS_TARGET_DIR"', 1)
        with self.assertRaisesRegex(ValueError, "(same|isolated) Cargo target"):
            validate(changed)

    def test_missing_worker_binary_override_is_rejected(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        changed = source.replace('INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN"', 'INTEGRATION_OUTBOX_WORKER_BIN=""')
        with self.assertRaisesRegex(ValueError, "runtime/build isolation"):
            validate(changed)

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
