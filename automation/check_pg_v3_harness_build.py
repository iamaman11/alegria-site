#!/usr/bin/env python3
"""Offline regression tests and an independent physical PG V3 A/B JSON checker.

    python3 automation/check_pg_v3_harness_build.py
    python3 automation/check_pg_v3_harness_build.py --report path/to/actual.json

The default tests use synthetic Cargo JSON *only to test rejection/parsing*;
they never produce or claim a physical rustc measurement. `--report` accepts
only a successful real compiler run with provenance, timings and executables.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import compare_pg_v3_harness_build as ab


def verify_physical_report(report: dict) -> None:
    """Reject non-physical/status-only/static results and inconsistent evidence."""
    if (report.get("schema_version") != ab.SCHEMA_VERSION
            or report.get("status") != "ACTUAL_PG_V3_HARNESS_COLD_CARGO_AB_CAPTURED"
            or report.get("actual_rustc_evidence") is not True):
        raise ab.EvidenceError("report is not completed physical Rust compiler evidence")
    provenance = report.get("source_provenance")
    if not isinstance(provenance, dict):
        raise ab.EvidenceError("source provenance is missing")
    for key in ("checkout_commit_sha", "checkout_tree_sha"):
        if not re.fullmatch(r"[a-f0-9]{40}", str(provenance.get(key, ""))):
            raise ab.EvidenceError(f"source provenance missing exact {key}")
    if (report.get("source_export_commit_sha") != provenance["checkout_commit_sha"]
            or report.get("source_export_tree_sha") != provenance["checkout_tree_sha"]):
        raise ab.EvidenceError("Git archive did not match checkout provenance")
    if not re.fullmatch(r"[a-f0-9]{64}", str(report.get("cargo_lock_sha256", ""))):
        raise ab.EvidenceError("immutable Cargo.lock digest missing")
    runner_context = report.get("runner_context")
    if not isinstance(runner_context, dict):
        raise ab.EvidenceError("exact runner/CI context missing")
    if (runner_context.get("GITHUB_EVENT_NAME") == "pull_request"
            and provenance.get("canonical_pr_head_commit_sha") is None):
        raise ab.EvidenceError("PR runner lacks verified canonical PR head")
    if provenance.get("canonical_pr_head_commit_sha") is not None:
        if not (provenance.get("trees_equal") is True
                and provenance.get("canonical_pr_head_tree_sha") == provenance["checkout_tree_sha"]
                and re.fullmatch(r"[a-f0-9]{40}", provenance["canonical_pr_head_commit_sha"])):
            raise ab.EvidenceError("PR canonical HEAD tree provenance not established")
    elif provenance.get("scope") != "local_checkout_only_no_pr_head_claim":
        raise ab.EvidenceError("unknown source attribution scope")
    regime = report.get("compiler_cache_regime")
    if not isinstance(regime, dict) or not all(regime.get(key) is True for key in (
        "separate_initially_empty_CARGO_TARGET_DIR", "RUSTC_WRAPPER_disabled",
        "RUSTC_WORKSPACE_WRAPPER_disabled", "Cargo_config_wrapper_overridden",
        "SCCACHE_environment_removed", "source_tree_same_for_both_options",
    )) or regime.get("CARGO_INCREMENTAL") != "0":
        raise ab.EvidenceError("independent wrapper-free cold Cargo regime not evidenced")
    if not isinstance(report.get("toolchain"), dict) or not report["toolchain"].get("host_target"):
        raise ab.EvidenceError("toolchain provenance missing")
    closures = report.get("cargo_resolved_closure_upper_bound")
    if not isinstance(closures, dict):
        raise ab.EvidenceError("resolved feature-aware Cargo metadata missing")
    candidate_closure = closures.get("integration_pg_v3_e2e")
    if not isinstance(candidate_closure, dict):
        raise ab.EvidenceError("candidate Cargo closure missing")
    names = candidate_closure.get("resolved_package_names_upper_bound")
    if not isinstance(names, list):
        raise ab.EvidenceError("candidate resolved packages missing")
    ab.check_pg_owner_closure(set(names))
    observed = []
    for option, package in ab.PACKAGES:
        probe = report.get(option)
        if not isinstance(probe, dict) or probe.get("package") != package:
            raise ab.EvidenceError(f"missing actual {option} Rust test probe")
        if probe.get("command") != ab.command_for(package):
            raise ab.EvidenceError(f"{option} command not the expected locked feature/test build")
        elapsed = probe.get("cargo_wall_seconds_observed")
        if not isinstance(elapsed, (int, float)) or not 0 < elapsed < float("inf"):
            raise ab.EvidenceError(f"{option} has no actual measured Cargo wall duration")
        observed.append(elapsed)
        units = probe.get("compiler_artifacts")
        nonfresh = probe.get("non_fresh_compiler_artifacts")
        if (not isinstance(units, list) or not isinstance(nonfresh, list)
                or not units or not nonfresh
                or probe.get("compiler_artifact_count") != len(units)
                or probe.get("non_fresh_compiler_artifact_count") != len(nonfresh)
                or nonfresh != [unit for unit in units if unit.get("fresh") is False]):
            raise ab.EvidenceError(f"{option} lacks coherent real non-fresh rustc units")
        actual_test_units = [unit for unit in units if unit.get("package") == package
                             and unit.get("target") == package and unit.get("test_profile") is True
                             and "e2e" in unit.get("features", [])
                             and unit.get("fresh") is False and unit.get("executable")]
        if len(actual_test_units) != 1:
            raise ab.EvidenceError(f"{option} lacks a unique e2e lib test rustc unit")
        executable = probe.get("lib_test_executable")
        if not isinstance(executable, dict) or executable.get("target") != package:
            raise ab.EvidenceError(f"{option} lacks test binary identity")
        digests = [executable, *(probe.get("cargo_timings_html") or [])]
        if len(digests) < 2 or any(
            not isinstance(digest, dict)
            or not isinstance(digest.get("size_bytes"), int)
            or digest["size_bytes"] <= 0
            or not re.fullmatch(r"[a-f0-9]{64}", str(digest.get("sha256", "")))
            for digest in digests
        ):
            raise ab.EvidenceError(f"{option} lacks binary or real Cargo --timings HTML digest")
        if package == "integration_pg_v3_e2e":
            ab.check_pg_owner_closure({unit.get("package") for unit in units})
    delta = report.get("observed_cargo_wall_delta_B_minus_A_seconds")
    ratio = report.get("observed_cargo_wall_ratio_B_to_A")
    if (not isinstance(delta, (int, float)) or not isinstance(ratio, (int, float))
            or abs(delta - (observed[1] - observed[0])) > 0.005
            or abs(ratio - observed[1] / observed[0]) > 0.0001):
        raise ab.EvidenceError("reported A/B comparison differs from measured Cargo durations")
    resource_scope = report.get("resource_measurement")
    if resource_scope == "NOT_MEASURED_NO_CPU_OR_RAM_CLAIM":
        if any(report[option].get("process_tree_resources") is not None for option, _ in ab.PACKAGES):
            raise ab.EvidenceError("report attributes resource data to an unmeasured run")
    elif isinstance(resource_scope, str) and resource_scope.startswith("GNU time:"):
        for option, _ in ab.PACKAGES:
            resources = report[option].get("process_tree_resources")
            if not isinstance(resources, dict) or not all(isinstance(resources.get(key), (int, float))
                and resources[key] >= (1 if key == "peak_resident_kib" else 0)
                for key in ("wall_seconds", "user_cpu_seconds", "system_cpu_seconds", "peak_resident_kib")):
                raise ab.EvidenceError("GNU time resources claimed without measured numbers")
    else:
        raise ab.EvidenceError("resource scope is absent/unsupported")


def artifact_event(package: str = "integration_pg_v3_e2e", *, fresh: bool = False,
                   features: list[str] | None = None, package_id: str = "pkg-pg") -> dict:
    return {
        "reason": "compiler-artifact", "package_id": package_id,
        "target": {"name": package, "kind": ["lib"]},
        "profile": {"test": True}, "features": ["e2e"] if features is None else features,
        "fresh": fresh, "executable": f"/isolated/{package}-ab123",
    }


def cargo_json(events: list[dict]) -> str:
    return "\n".join(json.dumps(event) for event in events)


class PhysicalCargoEvidenceTests(unittest.TestCase):
    def test_cargo_command_is_locked_lib_test_and_disables_config_wrappers(self):
        for _, package in ab.PACKAGES:
            command = ab.command_for(package)
            self.assertIn("--no-run", command)
            self.assertIn("--locked", command)
            self.assertIn("--lib", command)
            self.assertIn("--timings", command)
            self.assertIn("--message-format=json-render-diagnostics", command)
            self.assertIn('build.rustc-wrapper=""', command)
            self.assertIn('build.rustc-workspace-wrapper=""', command)
        with self.assertRaises(ab.EvidenceError):
            ab.command_for("hypothetical_package")

    def test_cold_target_removes_sccache_and_rustc_wrappers(self):
        poisoned = {
            "RUSTC_WRAPPER": "sccache", "RUSTC_WORKSPACE_WRAPPER": "cached",
            "CARGO_BUILD_RUSTC_WRAPPER": "cached", "SCCACHE_GHA_ENABLED": "true",
            "CARGO_INCREMENTAL": "1", "CARGO_TARGET_DIR": "/shared/cargo-target",
            "RUSTC": "/wrapper/not-rustc", "CARGO_BUILD_RUSTC": "/wrapper/cached",
        }
        with tempfile.TemporaryDirectory() as dirname, patch.dict(os.environ, poisoned), \
             patch.object(ab.shutil, "which", return_value="/usr/bin/rustc"):
            env = ab.wrapper_free_environment(Path(dirname) / "target-B")
            for key in (*ab.WRAPPER_ENV, "SCCACHE_GHA_ENABLED"):
                self.assertNotIn(key, env)
            self.assertEqual(env["RUSTC"], "/usr/bin/rustc")
            self.assertEqual(env["CARGO_BUILD_RUSTC"], "/usr/bin/rustc")
            self.assertEqual(env["CARGO_INCREMENTAL"], "0")
            self.assertNotEqual(env["CARGO_TARGET_DIR"], poisoned["CARGO_TARGET_DIR"])
            stale = Path(dirname) / "target-old"
            stale.mkdir()
            (stale / "old.file").touch()
            with self.assertRaisesRegex(ab.EvidenceError, "not initially empty"):
                ab.wrapper_free_environment(stale)

    def test_complete_actual_test_executable_cargo_json_is_required(self):
        events = [artifact_event(), {"reason": "build-finished", "success": True}]
        parsed = ab.decode_cargo_artifacts(cargo_json(events), "integration_pg_v3_e2e",
                                           {"pkg-pg": "integration_pg_v3_e2e"})
        self.assertEqual(len(parsed["non_fresh_compiler_artifacts"]), 1)
        for bad in (
            [artifact_event(fresh=True), events[-1]],
            [artifact_event(features=[]), events[-1]],
            [artifact_event()],
            [artifact_event(), {"reason": "build-finished", "success": False}],
            [artifact_event(), events[-1], events[-1]],
        ):
            with self.subTest(bad=bad), self.assertRaises(ab.EvidenceError):
                ab.decode_cargo_artifacts(cargo_json(bad), "integration_pg_v3_e2e",
                                          {"pkg-pg": "integration_pg_v3_e2e"})

    def test_cargo_json_cannot_invent_unknown_packages_or_omit_fields(self):
        events = [artifact_event(), {"reason": "build-finished", "success": True}]
        with self.assertRaisesRegex(ab.EvidenceError, "required rustc fields"):
            ab.decode_cargo_artifacts(cargo_json(events), "integration_pg_v3_e2e", {})
        broken = artifact_event()
        del broken["profile"]
        with self.assertRaisesRegex(ab.EvidenceError, "required rustc fields"):
            ab.decode_cargo_artifacts(cargo_json([broken, events[-1]]),
                                      "integration_pg_v3_e2e", {"pkg-pg": "integration_pg_v3_e2e"})

    def test_pg_owner_rejects_legacy_and_provider_compiler_units(self):
        events = [artifact_event(), artifact_event("graph_neo4j", package_id="pkg-graph"),
                  {"reason": "build-finished", "success": True}]
        with self.assertRaisesRegex(ab.EvidenceError, "forbidden"):
            ab.decode_cargo_artifacts(cargo_json(events), "integration_pg_v3_e2e", {
                "pkg-pg": "integration_pg_v3_e2e", "pkg-graph": "graph_neo4j",
            })
        with self.assertRaisesRegex(ab.EvidenceError, "forbidden"):
            ab.check_pg_owner_closure({"postgres_store", "integration_harness"})

    def test_feature_aware_locked_cargo_metadata_tracks_dev_deps_and_rejects_unsafe(self):
        package_names = ["integration_harness", "integration_pg_v3_e2e", "postgres_store", "serde"]
        packages = [{"id": f"id-{name}", "name": name,
                     "manifest_path": f"/unused/{name}/Cargo.toml",
                     "source": "registry+test" if name == "serde" else None}
                    for name in package_names]
        nodes = [{"id": f"id-{name}", "deps": []} for name in package_names]
        nodes[1]["deps"] = [{"pkg": "id-postgres_store", "dep_kinds": [{"kind": "dev"}]},
                              {"pkg": "id-serde", "dep_kinds": [{"kind": None}]}]
        metadata = {"packages": packages, "workspace_members": [
            "id-integration_harness", "id-integration_pg_v3_e2e", "id-postgres_store"],
            "resolve": {"nodes": nodes}}

        def run_metadata(command, **_kwargs):
            self.assertIn("--locked", command)
            self.assertIn("--filter-platform", command)
            self.assertIn("integration_pg_v3_e2e/e2e", command[-1])
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps(metadata), stderr="")

        with patch.object(ab.subprocess, "run", side_effect=run_metadata):
            closures, names = ab.resolved_package_closures(Path("/unused"), "x86_64-unknown-linux-gnu")
            self.assertEqual(closures["integration_pg_v3_e2e"]["resolved_dependency_package_count_upper_bound"], 2)
            self.assertEqual(names["id-serde"], "serde")
            nodes[1]["deps"].append({"pkg": "id-integration_harness", "dep_kinds": [{"kind": "dev"}]})
            with self.assertRaisesRegex(ab.EvidenceError, "forbidden"):
                ab.resolved_package_closures(Path("/unused"), "x86_64-unknown-linux-gnu")
            nodes[1]["deps"].pop()
            packages[2]["manifest_path"] = "/other-untracked-source/Cargo.toml"
            with self.assertRaisesRegex(ab.EvidenceError, "outside exact Git archive"):
                ab.resolved_package_closures(Path("/unused"), "x86_64-unknown-linux-gnu")

    def test_actual_rustc_invocation_requires_executable_and_cargo_timings(self):
        with tempfile.TemporaryDirectory() as dirname:
            tmp = Path(dirname)
            target = tmp / "target-B"
            binary = target / "debug" / "deps" / "integration_pg_v3_e2e-aa"
            timing = target / "cargo-timings" / "cargo-timing.html"
            observed = {}

            class FakeProcess:
                returncode = 0

                def communicate(self):
                    return cargo_json([{
                        **artifact_event(), "executable": str(binary),
                    }, {"reason": "build-finished", "success": True}]), ""

            def run_cargo(command, **kwargs):
                observed["command"] = command
                observed["env"] = kwargs["env"]
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"real-test-executable-fixture")
                timing.parent.mkdir(parents=True, exist_ok=True)
                timing.write_text("<html>cargo timings fixture</html>")
                return FakeProcess()

            with patch.object(ab.shutil, "which", return_value="/usr/bin/rustc"), \
                 patch.object(ab.subprocess, "Popen", side_effect=run_cargo):
                report = ab.measured_cargo_run(tmp, target, "integration_pg_v3_e2e",
                                               {"pkg-pg": "integration_pg_v3_e2e"}, False)
            self.assertEqual(observed["command"], ab.command_for("integration_pg_v3_e2e"))
            self.assertEqual(observed["env"]["CARGO_TARGET_DIR"], str(target))
            self.assertEqual(report["non_fresh_compiler_artifact_count"], 1)
            self.assertRegex(report["lib_test_executable"]["sha256"], r"^[a-f0-9]{64}$")
            self.assertEqual(len(report["cargo_timings_html"]), 1)
            self.assertIsNone(report["process_tree_resources"])

    def test_resource_claim_only_when_gnu_time_was_used(self):
        with tempfile.TemporaryDirectory() as dirname:
            tmp = Path(dirname)
            target = tmp / "target-A"
            resource = tmp / "resource-integration_harness.txt"
            binary = target / "debug" / "deps" / "integration_harness-aa"
            timing = target / "cargo-timings" / "cargo-timing.html"

            class FakeProcess:
                returncode = 0

                def communicate(self):
                    return cargo_json([{
                        **artifact_event("integration_harness", package_id="pkg-a"),
                        "executable": str(binary),
                    }, {"reason": "build-finished", "success": True}]), ""

            def fake_popen(command, **_kwargs):
                self.assertEqual(command[0], "/usr/bin/time")
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"fixture")
                timing.parent.mkdir(parents=True, exist_ok=True)
                timing.write_text("<html>cargo timing</html>")
                resource.write_text("PHYSICAL_AB_RESOURCE\t6.0\t4.0\t1.0\t20500\t0\n")
                return FakeProcess()

            with patch.object(ab.shutil, "which", side_effect=lambda exe: f"/usr/bin/{exe}"), \
                 patch.object(ab.subprocess, "Popen", side_effect=fake_popen), \
                 patch.object(ab.subprocess, "run", return_value=subprocess.CompletedProcess(
                     ["time", "--version"], 0, stdout="time (GNU Time) 1.9\n", stderr="")):
                report = ab.measured_cargo_run(tmp, target, "integration_harness",
                                               {"pkg-a": "integration_harness"}, True)
            self.assertEqual(report["process_tree_resources"]["peak_resident_kib"], 20500)

    def test_gnu_time_version_requires_real_vendor_banner(self):
        for banner in (
            "time (GNU Time) 1.9\nCopyright (C) Free Software Foundation",
            "time (GNU time) 1.9\n",
            "GNU time 1.9",
        ):
            with self.subTest(valid=banner):
                self.assertTrue(ab.is_gnu_time_version(banner))
        for banner in (
            "", "time version 1.9", "time (BSD) 1.9",
            "BSD time 1.9; compatible with GNU Time",
            "not GNU Time 1.9", "GNU Time invalid", "GNU time 1.9 garbage",
        ):
            with self.subTest(invalid=banner):
                self.assertFalse(ab.is_gnu_time_version(banner))

    def test_resource_probe_fails_before_compile_on_bsd_time(self):
        with tempfile.TemporaryDirectory() as dirname:
            with patch.object(ab.shutil, "which", return_value="/usr/bin/time"), \
                 patch.object(ab.subprocess, "run", return_value=subprocess.CompletedProcess(
                     ["time", "--version"], 0, stdout="time (BSD) 1.9", stderr="")), \
                 patch.object(ab.subprocess, "Popen") as compile_process:
                with self.assertRaisesRegex(ab.EvidenceError, "requires GNU time"):
                    ab.measured_cargo_run(
                        Path(dirname), Path(dirname) / "empty-target",
                        "integration_harness", {}, True,
                    )
                compile_process.assert_not_called()

    def test_failure_does_not_leave_stale_success_report(self):
        with tempfile.TemporaryDirectory() as dirname:
            path = Path(dirname) / "physical.json"
            path.write_text('{"status":"ACTUAL_PG_V3_HARNESS_COLD_CARGO_AB_CAPTURED"}')
            with patch.object(ab, "physical_ab_report", side_effect=ab.EvidenceError("Cargo unavailable")), \
                 contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(ab.main(["--output", str(path)]), 2)
            report = json.loads(path.read_text())
            self.assertEqual(report["status"], "EVIDENCE_BLOCKED")
            self.assertFalse(report["actual_rustc_evidence"])
            with self.assertRaises(ab.EvidenceError):
                verify_physical_report(report)

    def test_validator_rejects_static_or_fabricated_physical_status(self):
        with self.assertRaisesRegex(ab.EvidenceError, "not completed"):
            verify_physical_report({"status": "MANIFEST_ONLY_UNCOMPILED", "actual_rustc_evidence": False})
        with self.assertRaisesRegex(ab.EvidenceError, "source provenance"):
            verify_physical_report({"schema_version": 1,
                "status": "ACTUAL_PG_V3_HARNESS_COLD_CARGO_AB_CAPTURED",
                "actual_rustc_evidence": True})


def main(argv: list[str]) -> int:
    if len(argv) == 2 and argv[0] == "--report":
        try:
            document = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
            verify_physical_report(document)
        except (ab.EvidenceError, ValueError, OSError, TypeError) as exc:
            print(f"EVIDENCE_BLOCKED: {exc}", file=sys.stderr)
            return 2
        print("ACTUAL_PG_V3_HARNESS_COLD_CARGO_AB_CAPTURED: verified")
        return 0
    if argv:
        print("Usage: check_pg_v3_harness_build.py [--report <JSON>]", file=sys.stderr)
        return 2
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(PhysicalCargoEvidenceTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
