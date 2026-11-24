#!/usr/bin/env python3
"""Regression tests for fail-closed physical Rust build A/B evidence."""

from __future__ import annotations

import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compare_physical_build_options as ab


class ManifestEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.graph, cls.paths = ab.workspace_graph()

    def test_current_native_worker_cannot_isolate_providers_inside_same_package(self) -> None:
        report = ab.manifest_report(self.graph, self.paths)
        self.assertEqual(report["A_native_monolith"]["workspace_dependency_count"], 19)
        self.assertEqual(report["A_modular_monolith"]["workspace_dependency_count"], 19)
        self.assertEqual(report["A_native_monolith"]["provider_dependencies"], sorted(ab.PROVIDER_OWNERS))

    def test_hypothetical_orchestrator_is_explicitly_uncompiled(self) -> None:
        report = ab.manifest_report(self.graph, self.paths)
        self.assertFalse(report["B_separate_orchestrator"]["exists"])
        self.assertTrue(report["B_separate_orchestrator"]["hypothetical"])
        self.assertEqual(report["B_separate_orchestrator"]["workspace_dependency_count"], 9)
        self.assertFalse(report["actual_rustc_evidence"])
        self.assertEqual(report["status"], "MANIFEST_ONLY_UNCOMPILED")

    def test_four_owner_edits_have_known_current_reverse_consumers(self) -> None:
        report = ab.manifest_report(self.graph, self.paths)
        actual = report["provider_edits_reverse_closure"]
        self.assertEqual(set(actual["graph_neo4j"]), {
            "graph_neo4j", "analytics_svc", "integration_harness", "outbox_worker",
            "runtime_composition", "temporal_worker",
        })
        self.assertEqual(set(actual["vector_qdrant"]), {
            "vector_qdrant", "integration_harness", "outbox_worker",
            "runtime_composition", "temporal_worker",
        })
        self.assertEqual(set(actual["postgres_store"]), {
            "postgres_store", "cli_tools", "infrastructure", "integration_harness",
            "outbox_worker", "reconcile", "runtime_composition", "temporal_worker",
        })
        self.assertEqual(set(actual["temporal_gateway"]), {
            "temporal_gateway", "cli_tools", "integration_harness", "reconcile", "temporal_worker",
        })

    def test_candidate_with_direct_or_transitive_provider_rejected(self) -> None:
        unsafe = {package: dependencies.copy() for package, dependencies in self.graph.items()}
        unsafe["temporal_orchestrator"] = {"seo_application", "runtime_composition"}
        with self.assertRaisesRegex(ab.EvidenceError, "transitively links provider owners"):
            ab.manifest_report(unsafe, self.paths)

    def test_unknown_package_fails_instead_of_shorter_fabricated_graph(self) -> None:
        with self.assertRaisesRegex(ab.EvidenceError, "unknown workspace packages"):
            ab.closure(self.graph, {"does_not_exist"})

    def test_cycles_are_finite_and_closed(self) -> None:
        self.assertEqual(ab.closure({"one": {"two"}, "two": {"one"}}, {"one"}), {"one", "two"})

    def test_optional_and_target_specific_dependencies_are_conservative(self) -> None:
        dependencies = ab.manifest_dependencies({
            "dependencies": {"normal": {"path": "../normal"}},
            "dev-dependencies": {"excluded_for_build": {"path": "../test"}},
            "target": {"cfg(windows)": {
                "dependencies": {"windows_only": {"path": "../windows"}},
                "build-dependencies": {"builder": {"path": "../builder"}},
            }},
        })
        self.assertEqual({name for name, _ in dependencies}, {"normal", "windows_only", "builder"})

    def test_duplicate_workspace_package_name_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            workspace = Path(temp)
            (workspace / "Cargo.toml").write_text('[workspace]\nmembers = ["a", "b"]\n')
            for name in ("a", "b"):
                folder = workspace / name
                folder.mkdir()
                (folder / "Cargo.toml").write_text('[package]\nname = "same"\nversion = "0.1.0"\n')
            with self.assertRaisesRegex(ab.EvidenceError, "duplicate workspace"):
                ab.workspace_graph(workspace)


class CompilerEvidenceTests(unittest.TestCase):
    def test_isolated_probe_invokes_real_rustc_without_wrapper_or_shared_target(self) -> None:
        fake_cached = {
            "RUSTC_WRAPPER": "sccache",
            "RUSTC_WORKSPACE_WRAPPER": "custom-sccache",
            "CARGO_BUILD_RUSTC_WRAPPER": "cached",
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "cached-workspace",
            "SCCACHE_GHA_ENABLED": "on",
            "SCCACHE_GHA_RW_MODE": "READ_ONLY",
            "CARGO_TARGET_DIR": "/shared/poisoned-target",
            "CARGO_INCREMENTAL": "1",
            "RUSTC": "/some/wrapper-disguised-as-rustc",
            "CARGO_BUILD_RUSTC": "/some/cached-rustc",
        }
        with patch.dict(os.environ, fake_cached), patch.object(ab.shutil, "which", return_value="/usr/bin/rustc"):
            with patch.object(ab.subprocess, "Popen") as popen:
                process = popen.return_value
                process.communicate.return_value = (
                    json.dumps({
                        "reason": "compiler-artifact", "package_id": "package-1",
                        "target": {"kind": ["bin"], "name": "temporal_worker"},
                        "fresh": False, "executable": "/fake/temporal_worker",
                    }) + "\n" + json.dumps({"reason": "build-finished", "success": True}),
                    "",
                )
                process.returncode = 0
                build = ab.compiler_artifacts(
                    ["cargo", "build", "--locked"], Path("/tmp/isolated-source/app/rust"),
                    isolated_target=True,
                )
                passed = popen.call_args.kwargs["env"]
                for name in ab.RUSTC_WRAPPER_ENV:
                    self.assertNotIn(name, passed)
                self.assertNotIn("SCCACHE_GHA_ENABLED", passed)
                self.assertNotIn("SCCACHE_GHA_RW_MODE", passed)
                self.assertEqual(passed["RUSTC"], "/usr/bin/rustc")
                self.assertEqual(passed["CARGO_BUILD_RUSTC"], "/usr/bin/rustc")
                self.assertEqual(passed["CARGO_INCREMENTAL"], "0")
                self.assertEqual(passed["CARGO_TARGET_DIR"], "/tmp/isolated-source/app/rust/target")
                self.assertNotEqual(passed["CARGO_TARGET_DIR"], fake_cached["CARGO_TARGET_DIR"])
                command = popen.call_args.args[0]
                self.assertEqual(command[0], "cargo")
                self.assertIn('build.rustc-wrapper=""', command)
                self.assertIn('build.rustc-workspace-wrapper=""', command)
                self.assertTrue(build["compiler_execution"]["rustc_wrapper_disabled"])
                self.assertEqual(len(build["non_fresh_units"]), 1)

    def test_exact_head_export_is_isolated_from_current_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            exported = Path(folder)
            sha = ab.isolated_source_tree(exported)
            self.assertEqual(len(sha), 40)
            self.assertTrue((exported / "app/rust/Cargo.toml").is_file())
            self.assertFalse((exported / ".git").exists())
            # A source-only export must not be mistaken for a compiler result.
            self.assertFalse((exported / "app/rust/target").exists())

    def test_compiler_requested_without_candidate_fails_closed(self) -> None:
        for argument in ("--compiler", "--compiler-owners"):
            with self.subTest(argument=argument):
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    code = ab.main([argument])
                report = json.loads(output.getvalue())
                self.assertEqual(code, 2)
                self.assertEqual(report["status"], "EVIDENCE_BLOCKED")
                self.assertFalse(report["actual_rustc_evidence"])
                self.assertIn("real temporal_orchestrator", report["error"])

    def test_compiler_artifact_incomplete_is_not_counted(self) -> None:
        with patch.object(ab.subprocess, "Popen") as popen:
            process = popen.return_value
            process.communicate.return_value = (
                json.dumps({
                    "reason": "compiler-artifact", "package_id": "example",
                    "target": {"kind": ["bin"], "name": "temporal_worker"},
                    "fresh": "false", "executable": "/fake/bin",
                }) + "\n" + json.dumps({"reason": "build-finished", "success": True}),
                "",
            )
            process.returncode = 0
            with self.assertRaisesRegex(ab.EvidenceError, "incomplete Cargo compiler-artifact"):
                ab.compiler_artifacts(["cargo", "build"], ab.RUST)

    def test_empty_or_failed_cargo_artifact_stream_fails_closed(self) -> None:
        with patch.object(ab.subprocess, "Popen") as popen:
            process = popen.return_value
            process.communicate.return_value = ('{"reason":"build-finished","success":true}', "")
            process.returncode = 0
            with self.assertRaisesRegex(ab.EvidenceError, "failed/incomplete"):
                ab.compiler_artifacts(["cargo", "build"], ab.RUST)

    def test_snapshot_requires_both_real_binaries(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            a = Path(folder) / "temporal_worker"
            a.write_bytes(b"real-a")
            records = {"compiler_artifacts": [{
                "kind": ["bin"], "executable": str(a), "target_name": "temporal_worker",
            }]}
            with self.assertRaisesRegex(ab.EvidenceError, "omitted one binary"):
                ab.binary_snapshot(records)

    def test_compiler_json_with_real_binary_paths_is_recorded_not_inferred(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            binaries = {}
            for name in ("temporal_worker", "temporal_orchestrator"):
                path = Path(folder) / name
                path.write_bytes(name.encode())
                binaries[name] = path
            observed = {"compiler_artifacts": [
                {"kind": ["bin"], "executable": str(path), "target_name": name}
                for name, path in binaries.items()
            ]}
            snapshot = ab.binary_snapshot(observed)
            self.assertEqual(set(snapshot), set(binaries))
            self.assertNotEqual(snapshot["temporal_worker"]["sha256"],
                                snapshot["temporal_orchestrator"]["sha256"])

    def test_no_compiler_tools_is_a_blocker_even_with_forged_candidate(self) -> None:
        report = {"B_separate_orchestrator": {"exists": True}}
        with patch.object(ab.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ab.EvidenceError, "unavailable"):
                ab.owner_rebuild_probe(report)


if __name__ == "__main__":
    unittest.main()
