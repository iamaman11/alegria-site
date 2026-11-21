#!/usr/bin/env python3
"""Fail-closed regression cases for the retained Rust-backed CI checks."""
from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "automation" / "run_rust_owned_check.sh"
SHA = "a" * 40
COMMAND = ("python3", "automation/smoke_crawler_robots_policy.py")
PRODUCT_COMMANDS = (
    ("python3", "automation/smoke_rebuild_dependency_graph.py"),
    ("python3", "automation/smoke_extraction_coverage.py"),
    ("python3", "automation/smoke_rebuild_queue_execution_path.py"),
)
WORKFLOW = ROOT / ".github/workflows/ci.yml"
PRODUCT_GATE = ROOT / "automation/ci_verify.sh"


def invoke(owner: str, *, actions: str = "true", job: str = "verify", sha: str = SHA,
           command: tuple[str, ...] = COMMAND,
           extra_env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(WRAPPER), *command],
        cwd=ROOT,
        env={**os.environ, "CI_RUST_CHECK_OWNER": owner, "GITHUB_ACTIONS": actions,
             "GITHUB_JOB": job, "CI_SOURCE_SHA": sha, **(extra_env or {})},
        text=True,
        capture_output=True,
        check=False,
    )


class RustOwnedCheckContract(unittest.TestCase):
    def test_exact_source_verify_delegates_to_required_owner(self) -> None:
        for command in (
            ("python3", "automation/smoke_crawler_robots_policy.py"),
            ("bash", "automation/contract_verify.sh"),
            ("python3", "automation/smoke_run_report_completeness.py"),
        ):
            result = invoke("rust-contract-smokes", command=command)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("CI_RUST_CHECK_DELEGATED owner=rust-contract-smokes", result.stdout)
            self.assertIn(f"source_sha={SHA}", result.stdout)

    def test_exact_source_product_delegates_only_retained_rust_smokes(self) -> None:
        for command in PRODUCT_COMMANDS:
            result = invoke("architecture-and-rust", job="product-truth", command=command)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("CI_RUST_CHECK_DELEGATED owner=architecture-and-rust", result.stdout)
            self.assertIn(f"check={command[1]}", result.stdout)
            self.assertIn(f"source_sha={SHA}", result.stdout)

    def test_original_smoke_remains_runnable_in_explicit_deep_gate(self) -> None:
        source = PRODUCT_GATE.read_text(encoding="utf-8")
        workflow = WORKFLOW.read_text(encoding="utf-8")
        deep = (ROOT / ".github/workflows/ci-deep.yml").read_text(encoding="utf-8")
        owner_section = workflow.split("  architecture-and-rust:", 1)[1].split("  product-truth:", 1)[0]
        product_section = workflow.split("  product-truth:", 1)[1].split("  rust-security:", 1)[0]
        for command in PRODUCT_COMMANDS:
            self.assertIn(f"bash automation/run_rust_owned_check.sh {' '.join(command)}", source)
            self.assertNotIn(' '.join(command), owner_section)
            self.assertNotIn(' '.join(command), product_section)
        self.assertIn("bash automation/ci_verify.sh all", deep)
        self.assertNotIn('CI_RUST_CHECK_OWNER: "architecture-and-rust"', product_section)

    def test_local_delegation_is_denied(self) -> None:
        result = invoke("rust-contract-smokes", actions="false")
        self.assertEqual(result.returncode, 2)

    def test_verify_smoke_cannot_be_delegated_without_exact_ci_source(self) -> None:
        for actions, sha in (("false", SHA), ("true", "invalid")):
            result = invoke("rust-contract-smokes", actions=actions, sha=sha)
            self.assertEqual(result.returncode, 2)

    def test_wrong_ci_job_is_denied(self) -> None:
        result = invoke("rust-contract-smokes", job="product-truth")
        self.assertEqual(result.returncode, 2)

    def test_wrong_owner_for_verify_is_denied(self) -> None:
        result = invoke("architecture-and-rust", job="verify")
        self.assertEqual(result.returncode, 2)

    def test_wrong_owner_for_product_is_denied(self) -> None:
        result = invoke("rust-contract-smokes", job="product-truth",
                        command=PRODUCT_COMMANDS[0])
        self.assertEqual(result.returncode, 2)

    def test_product_smoke_cannot_be_delegated_from_verify(self) -> None:
        for command in PRODUCT_COMMANDS:
            result = invoke("architecture-and-rust", job="verify", command=command)
            self.assertEqual(result.returncode, 2)

    def test_product_smoke_cannot_be_delegated_without_exact_ci(self) -> None:
        for command in PRODUCT_COMMANDS:
            for actions, sha in (("false", SHA), ("true", "invalid")):
                result = invoke("architecture-and-rust", actions=actions,
                                job="product-truth", sha=sha, command=command)
                self.assertEqual(result.returncode, 2)

    def test_invalid_source_is_denied(self) -> None:
        result = invoke("rust-contract-smokes", sha="not-a-commit")
        self.assertEqual(result.returncode, 2)

    def test_unknown_owner_is_denied(self) -> None:
        result = invoke("other-owner")
        self.assertEqual(result.returncode, 2)

    def test_local_owner_executes_and_propagates_real_command_failure(self) -> None:
        # Stub only the executable in the test's disposable PATH. The wrapper
        # still dispatches the exact allowed script name without running Cargo.
        with tempfile.TemporaryDirectory() as temp:
            fake_python = Path(temp) / "python3"
            fake_python.write_text("#!/bin/sh\nprintf 'LOCAL_RUST_CHECK_EXECUTED\\n'\nexit 17\n")
            fake_python.chmod(0o755)
            result = invoke("verify", actions="false", extra_env={
                "PATH": f"{temp}:{os.environ.get('PATH', '')}",
            })
            self.assertEqual(result.returncode, 17)
            self.assertIn("LOCAL_RUST_CHECK_EXECUTED", result.stdout)

    def test_local_product_runs_original_command_with_failure_propagation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake_python = Path(temp) / "python3"
            fake_python.write_text("#!/bin/sh\nprintf 'LOCAL_PRODUCT_SMOKE_EXECUTED\\n'\nexit 19\n")
            fake_python.chmod(0o755)
            for command in PRODUCT_COMMANDS:
                result = invoke("verify", actions="false", command=command, extra_env={
                    "PATH": f"{temp}:{os.environ.get('PATH', '')}",
                })
                self.assertEqual(result.returncode, 19)
                self.assertIn("LOCAL_PRODUCT_SMOKE_EXECUTED", result.stdout)

    def test_arbitrary_or_extended_check_is_denied(self) -> None:
        for command in (("python3", "-c", "pass"), (*COMMAND, "extra")):
            result = invoke("rust-contract-smokes", command=command)
            self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
