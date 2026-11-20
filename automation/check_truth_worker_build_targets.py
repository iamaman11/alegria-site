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


if __name__ == "__main__":
    unittest.main()
