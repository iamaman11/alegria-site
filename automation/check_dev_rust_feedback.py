#!/usr/bin/env python3
"""Offline, no-Cargo tests for the native incremental developer feedback path."""

from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

from ci_impact import SCHEMA_VERSION, analyze, changed_worktree_paths
from dev_rust_feedback import commands_for_report


OWNERS = {
    "app/rust/crates/http_client": "http_client",
    "app/rust/crates/vector_qdrant": "vector_qdrant",
    "app/rust/services/gsc_sync": "gsc_sync",
    "app/rust/services/temporal": "temporal_worker",
}
REVERSE = {
    "http_client": {"gsc_sync"},
    "vector_qdrant": {"temporal_worker"},
    "gsc_sync": set(),
    "temporal_worker": set(),
}


def report(*paths: str) -> dict:
    return analyze(list(paths), OWNERS, REVERSE, head="HEAD", base="BASE")


class DevRustFeedbackTests(unittest.TestCase):
    def test_changed_worktree_includes_staged_unstaged_deleted_renamed_and_untracked(self) -> None:
        with patch("subprocess.check_output", side_effect=[
            b"M\0app/rust/crates/http_client/src/lib.rs\0"
            b"D\0app/rust/services/gsc_sync/src/removed.rs\0"
            b"R100\0app/rust/crates/vector_qdrant/src/old.rs\0"
            b"app/rust/crates/vector_qdrant/src/new.rs\0",
            b"app/rust/services/temporal/src/new.rs\0",
        ]) as call:
            paths = changed_worktree_paths("HEAD", root=Path("/synthetic/repo"))
        self.assertEqual(len(paths), 5)
        self.assertIn("app/rust/crates/vector_qdrant/src/old.rs", paths)
        self.assertIn("app/rust/services/temporal/src/new.rs", paths)
        self.assertEqual(call.call_count, 2)

    def test_untracked_unknown_file_forces_full_scope(self) -> None:
        with patch("subprocess.check_output", side_effect=[b"", b"unexpected.py\0"]):
            paths = changed_worktree_paths("HEAD", root=Path("/synthetic/repo"))
        self.assertEqual(report(*paths)["analysis"], "full")

    def test_untracked_unsafe_path_is_rejected(self) -> None:
        with patch("subprocess.check_output", side_effect=[b"", b"../escape.rs\0"]):
            with self.assertRaises(ValueError):
                changed_worktree_paths("HEAD", root=Path("/synthetic/repo"))

    def test_http_change_only_checks_own_dependency_closure_and_tests_owner(self) -> None:
        commands = commands_for_report(
            report("app/rust/crates/http_client/src/lib.rs"), check_only=False
        )
        self.assertEqual(commands, [
            ["cargo", "check", "--locked", "--package", "gsc_sync", "--package", "http_client"],
            ["cargo", "test", "--locked", "--package", "http_client"],
        ])

    def test_check_only_skips_test_execution(self) -> None:
        commands = commands_for_report(
            report("app/rust/crates/vector_qdrant/src/lib.rs"), check_only=True
        )
        self.assertEqual(commands, [[
            "cargo", "check", "--locked", "--package", "temporal_worker", "--package", "vector_qdrant"
        ]])

    def test_global_input_falls_back_to_whole_workspace(self) -> None:
        commands = commands_for_report(report("app/db/schema.sql"), check_only=False)
        self.assertEqual(commands, [["cargo", "test", "--workspace", "--locked"]])
        self.assertEqual(
            commands_for_report(report("app/db/schema.sql"), check_only=True),
            [["cargo", "check", "--workspace", "--locked"]],
        )

    def test_clean_worktree_only_needs_formatting(self) -> None:
        self.assertEqual(commands_for_report(report(), check_only=False), [])

    def test_missing_gate_or_invalid_package_fails_closed(self) -> None:
        unsafe = report("app/rust/crates/http_client/src/lib.rs")
        unsafe["mandatory_full_ci_gate"] = False
        with self.assertRaises(ValueError):
            commands_for_report(unsafe, check_only=False)
        unsafe["mandatory_full_ci_gate"] = True
        unsafe["schema_version"] = SCHEMA_VERSION - 1
        with self.assertRaises(ValueError):
            commands_for_report(unsafe, check_only=True)
        unsafe["schema_version"] = SCHEMA_VERSION
        unsafe["affected_crates"] = ["--workspace"]
        with self.assertRaises(ValueError):
            commands_for_report(unsafe, check_only=True)


if __name__ == "__main__":
    unittest.main()
