#!/usr/bin/env python3
"""The deep CI owner runs real Rust-backed checks; fast PR CI never fakes them."""
from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "automation" / "run_rust_owned_check.sh"
WORKFLOW = ROOT / ".github/workflows/ci.yml"
GATE = ROOT / "automation/ci_verify.sh"

COMMANDS = (
    ("python3", "automation/smoke_crawler_robots_policy.py"),
    ("bash", "automation/contract_verify.sh"),
    ("python3", "automation/smoke_run_report_completeness.py"),
    ("python3", "automation/smoke_rebuild_dependency_graph.py"),
    ("python3", "automation/smoke_rebuild_queue_execution_path.py"),
    ("python3", "automation/smoke_extraction_coverage.py"),
)


def invoke(command: tuple[str, ...], *, owner: str = "verify", actions: str = "false",
           extra_env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["/bin/bash", str(WRAPPER), *command], cwd=ROOT,
        env={**os.environ, "CI_RUST_CHECK_OWNER": owner, "GITHUB_ACTIONS": actions,
             "GITHUB_JOB": "verify", "CI_SOURCE_SHA": "a" * 40, **(extra_env or {})},
        text=True, capture_output=True, check=False,
    )


class RustOwnedCheckContract(unittest.TestCase):
    def test_all_six_real_checks_remain_in_explicit_deep_gate(self) -> None:
        source = GATE.read_text(encoding="utf-8")
        workflow = WORKFLOW.read_text(encoding="utf-8")
        deep = workflow.split("  comprehensive:\n", 1)[1].split("  rust-cache-prime:\n", 1)[0]
        self.assertIn("bash automation/ci_verify.sh all", deep)
        for command in COMMANDS:
            self.assertIn(f"bash automation/run_rust_owned_check.sh {' '.join(command)}", source)
        self.assertNotIn("  rust-contract-smokes:\n", workflow)

    def test_no_owner_can_skip_real_checks_even_with_spoofed_github_context(self) -> None:
        for command in COMMANDS:
            for owner in ("architecture-and-rust", "rust-contract-smokes", "product-truth"):
                with self.subTest(command=command, owner=owner):
                    result = invoke(command, owner=owner, actions="true")
                    self.assertEqual(result.returncode, 2)
                    self.assertIn("cannot be delegated", result.stderr)

    def test_local_checks_execute_and_propagate_their_real_status(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            for executable, code in (("python3", 17), ("bash", 19)):
                stub = Path(temp) / executable
                stub.write_text(f"#!/bin/sh\necho REAL_CHECK_EXECUTED\nexit {code}\n")
                stub.chmod(0o755)
            for command in COMMANDS:
                with self.subTest(command=command):
                    result = invoke(command, actions="true", extra_env={
                        "PATH": f"{temp}:{os.environ.get('PATH', '')}",
                    })
                    self.assertEqual(result.returncode, 17 if command[0] == "python3" else 19)
                    self.assertIn("REAL_CHECK_EXECUTED", result.stdout)

    def test_arbitrary_and_extended_commands_fail_closed(self) -> None:
        for command in (("python3", "-c", "pass"), (*COMMANDS[0], "extra")):
            result = invoke(command)
            self.assertEqual(result.returncode, 2)
            self.assertIn("Unsupported", result.stderr)


if __name__ == "__main__":
    unittest.main()
