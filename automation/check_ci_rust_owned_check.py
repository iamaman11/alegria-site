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
            result = invoke("architecture-and-rust", command=command)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("CI_RUST_CHECK_DELEGATED owner=architecture-and-rust", result.stdout)
            self.assertIn(f"source_sha={SHA}", result.stdout)

    def test_local_delegation_is_denied(self) -> None:
        result = invoke("architecture-and-rust", actions="false")
        self.assertEqual(result.returncode, 2)

    def test_wrong_ci_job_is_denied(self) -> None:
        result = invoke("architecture-and-rust", job="product-truth")
        self.assertEqual(result.returncode, 2)

    def test_invalid_source_is_denied(self) -> None:
        result = invoke("architecture-and-rust", sha="not-a-commit")
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

    def test_arbitrary_or_extended_check_is_denied(self) -> None:
        for command in (("python3", "-c", "pass"), (*COMMAND, "extra")):
            result = invoke("architecture-and-rust", command=command)
            self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
