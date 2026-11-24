#!/usr/bin/env python3
"""Offline smoke for caching only when CI actually runs Cargo."""
from __future__ import annotations

import io
import json
import re
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from report_ci_cargo_cache import directory_bytes, record
import report_sccache_failures as sccache_reporter

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "a" * 40
OWNERS = ("architecture-and-rust", "verify")


def validate_cache_policy(workflow: str) -> None:
    # Split job blocks by YAML's two-space job indentation, not by nested steps.
    blocks = []
    for owner in OWNERS:
        marker = f"  {owner}:\n"
        if workflow.count(marker) != 1:
            raise ValueError(f"missing or duplicate {owner}")
        lines = workflow.split(marker, 1)[1].splitlines()
        body: list[str] = []
        for line in lines:
            if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
                break
            body.append(line)
        blocks.append("\n".join(body))
    saver, verify = blocks
    # The trusted-main compiler cache is useful only when its rustc profile
    # matches the PR reader. A one-sided DEBUG=0 change silently invalidates
    # every sccache entry, turning a warm regression into a cold build.
    prime = workflow.split("  rust-cache-prime:\n", 1)[1].split("  select-image:\n", 1)[0]
    for name, block in (("PR architecture", saver), ("PR verify", verify), ("accepted-main", prime)):
        for profile in ("DEV", "TEST"):
            key = f'CARGO_PROFILE_{profile}_DEBUG: "0"'
            if block.count(key) != 1:
                raise ValueError(f"{name} must use the same symbol-free Rust {profile} profile")
    if 'CARGO_PROFILE_DEV_DEBUG: "0"' in workflow.split("  architecture-and-rust:\n", 1)[0]:
        raise ValueError("CI-only Rust profile must not change repository-global release settings")
    if ("SCCACHE_GHA_ENABLED: \"on\"" not in saver
            or "uses: actions/github-script@v8" not in saver
            or "core.exportVariable('ACTIONS_RESULTS_URL', url)" not in saver
            or "core.exportVariable('ACTIONS_RUNTIME_TOKEN', token)" not in saver):
        raise ValueError("sccache must receive authenticated GitHub cache service environment")
    if saver.count("uses: actions/cache@v4") != 1:
        raise ValueError("architecture must be the sole registry cache writer")
    if "uses: actions/cache@v4" in verify or "uses: actions/cache/save" in verify:
        raise ValueError("verify must not initialize a second Cargo registry writer")
    if verify.count("uses: actions/cache/restore@v4") != 1:
        raise ValueError("verify must restore Cargo sources without writing cache")
    if "SCCACHE_GHA_RW_MODE: READ_ONLY" not in verify:
        raise ValueError("verify Cargo compilation must use read-only compiler cache")
    for step_name in (
        "Pin Rust for full workspace tests",
        "Reuse trusted main Rust compiler objects for full workspace tests",
        "Authorize full-scope Rust cache reads",
        "Restore shared Cargo dependency sources without a second writer",
        "Full workspace Rust unit tests (parallel with real E2E)",
    ):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        if "        if: needs.impact.outputs.cargo_scope == 'full'" not in step:
            raise ValueError(f"verify Cargo step {step_name} must be full-only")
    for block in blocks:
        # Reject target *cache payloads*, not valid references to a job-local
        # build directory in an unrelated environment variable or test step.
        for step in re.split(r"(?m)^      - ", block):
            if "uses: actions/cache" not in step:
                continue
            paths = re.findall(
                r"(?m)^          path: ([^\n]*)(?:\n((?:            [^\n]*\n)*))?", step
            )
            if any("app/rust/target" in "".join(parts) for parts in paths):
                raise ValueError("unbounded target transfer in registry-only A/B")
        if "~/.cargo/registry" not in block or "~/.cargo/git" not in block:
            raise ValueError("Cargo source dependency cache missing")
        if "key: cargo-registry-${{ runner.os }}-${{ hashFiles('app/rust/Cargo.lock') }}" not in block:
            raise ValueError("owners must use the same Cargo.lock source cache")
        if "CI_CARGO_CACHE_METRIC" in block:
            raise ValueError("metric marker belongs to reporting code, not workflow")
    product = workflow.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
    if "uses: actions/cache" in product or "RUSTC_WRAPPER" in product or "cargo tree" in product:
        raise ValueError("Product's Python-only fast path must not set up or inspect Cargo")


class CargoCachePolicy(unittest.TestCase):
    def test_actual_ci_has_one_registry_writer_and_no_target_archive(self) -> None:
        validate_cache_policy((ROOT / ".github/workflows/ci.yml").read_text())

    def test_a_second_saver_in_python_verify_is_rejected(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace("uses: actions/cache/restore@v4", "uses: actions/cache@v4", 1)
        with self.assertRaisesRegex(ValueError, "second Cargo registry writer"):
            validate_cache_policy(changed)

    def test_verify_cargo_setup_cannot_run_for_scoped_or_non_rust_changes(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace(
            "      - name: Full workspace Rust unit tests (parallel with real E2E)\n"
            "        if: needs.impact.outputs.cargo_scope == 'full'",
            "      - name: Full workspace Rust unit tests (parallel with real E2E)\n"
            "        if: needs.impact.outputs.cargo_scope != 'none'",
            1,
        )
        with self.assertRaisesRegex(ValueError, "must be full-only"):
            validate_cache_policy(changed)

    def test_unbounded_target_regression_is_rejected(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace("            ~/.cargo/git\n", "            ~/.cargo/git\n            app/rust/target\n", 1)
        with self.assertRaisesRegex(ValueError, "unbounded target"):
            validate_cache_policy(changed)
        single_path = original.replace("          path: |\n", "          path: app/rust/target\n", 1)
        with self.assertRaisesRegex(ValueError, "unbounded target"):
            validate_cache_policy(single_path)

    def test_noncompiling_product_has_no_registry_restore(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        product = original.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
        self.assertNotIn("cargo-registry-", product)
        validate_cache_policy(original)

    def test_ci_dev_and_test_profiles_must_match_trusted_main(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        for name in ("DEV", "TEST"):
            key = f'CARGO_PROFILE_{name}_DEBUG: "0"'
            for occurrence in (1, 2, 3):
                position = original.find(key)
                if occurrence == 2:
                    position = original.find(key, position + len(key))
                self.assertGreaterEqual(position, 0)
                changed = original[:position] + f'CARGO_PROFILE_{name}_DEBUG: "2"' + original[position + len(key):]
                with self.subTest(name=name, occurrence=occurrence):
                    with self.assertRaisesRegex(ValueError, "same symbol-free Rust"):
                        validate_cache_policy(changed)

    def test_missing_authenticated_sccache_backend_is_rejected(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace("core.exportVariable('ACTIONS_RUNTIME_TOKEN', token);", "// missing token export")
        with self.assertRaisesRegex(ValueError, "authenticated"):
            validate_cache_policy(changed)

    def test_metrics_distinguish_uncached_target_and_missing_registry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "registry").mkdir()
            (root / "registry" / "a").write_bytes(b"x" * 123)
            target = root / "target"
            target.mkdir()
            (target / "b").write_bytes(b"y" * 234)
            report = record("architecture-and-rust", "false", SOURCE, root, target)
            self.assertEqual(report["source_sha"], SOURCE)
            self.assertFalse(report["registry_cache_hit"])
            self.assertGreaterEqual(report["registry_bytes"], 123)
            self.assertGreaterEqual(report["target_bytes_uncached"], 234)
            self.assertEqual(report["git_bytes"], 0)

    def test_architecture_owner_has_a_valid_cache_report(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            report = record("architecture-and-rust", "true", SOURCE, root, root / "target")
            self.assertEqual(report["owner"], "architecture-and-rust")
            self.assertTrue(report["registry_cache_hit"])

    def test_rejects_invalid_cache_signal_and_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                record("architecture-and-rust", "unknown", SOURCE, root, root)
            with self.assertRaises(ValueError):
                record("architecture-and-rust", "true", "not-sha", root, root)
            with self.assertRaises(ValueError):
                record("unrecognized", "true", SOURCE, root, root)

    def test_missing_directory_is_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(directory_bytes(Path(tmp) / "absent"), 0)


def sccache_stats(writes: int, errors: int, hits: int = 0, misses: int = 0) -> dict:
    """Match the real sccache 0.16 JSON ServerInfo.stats shape."""
    return {"stats": {
        "cache_writes": writes, "cache_write_errors": errors,
        "cache_hits": {"counts": {"Rust": hits}},
        "cache_misses": {"counts": {"Rust": misses}},
    }}


class LiveSccacheDiagnosticContract(unittest.TestCase):
    """Guard the reporter actually executed by the PR and trusted-main jobs.

    The previous standalone diagnostic reporter was not called anywhere in CI.
    Keeping these checks in this existing CI verify command makes the native
    cache accounting, error categories and secret handling real contracts.
    """

    def test_main_429_pressure_is_degraded_despite_1055_successful_writes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "main-debug.log"
            log.write_text(
                "DEBUG Error executing cache write: HTTP 429 Too Many Requests\n" * 40
                + "DEBUG Error executing cache write: storage unavailable\n" * 2,
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(1055, 42, 0, 1120), log, SOURCE, "READ_WRITE"
            )
            self.assertEqual(report["health"], "DEGRADED")
            self.assertEqual(report["source_sha"], SOURCE)
            self.assertEqual(report["cache_writes"], 1055)
            self.assertEqual(report["cache_write_errors"], 42)
            self.assertEqual(report["successful_write_percent"], 96.2)
            self.assertEqual(report["rust_hit_percent"], 0.0)
            self.assertEqual(report["classes"]["rate_limited_429"], 40)
            self.assertEqual(report["unexpected_write_errors"], 42)
            self.assertEqual(report["unattributed_write_errors"], 0)
            summary = sccache_reporter.format_summary(report)
            self.assertIn("health=DEGRADED", summary)
            self.assertIn("cache_writes=1055 cache_write_errors=42 http_429=40", summary)
            self.assertIn("source_sha=" + SOURCE, summary)

    def test_all_failure_families_count_only_cache_write_events(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "debug.log"
            log.write_text(
                "DEBUG Error executing cache write: HTTP 429 Too Many Requests\n"
                "DEBUG Error executing cache write: status=401 Unauthorized\n"
                "DEBUG Error executing cache write: status=403 Forbidden\n"
                "DEBUG Error executing cache write: status 503 unavailable\n"
                "DEBUG Error executing cache write: storage quota exceeded\n"
                "DEBUG Error executing cache write: deadline exceeded\n"
                "DEBUG Error executing cache write: backend storage failed\n"
                "DEBUG successful cache read HTTP 429\n",
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(10, 7, 3, 7), log, SOURCE, "READ_WRITE"
            )
            self.assertEqual(report["write_failure_log_lines"], 7)
            self.assertEqual(report["unattributed_write_errors"], 0)
            self.assertEqual(report["rust_hit_percent"], 30.0)
            self.assertEqual(report["classes"], {
                "rate_limited_429": 1, "unauthorized_401": 1, "forbidden_403": 1,
                "server_5xx": 1, "quota_or_capacity": 1, "timeout": 1,
                "unclassified": 1,
            })

    def test_missing_debug_log_retains_native_error_count_and_degraded_status(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = sccache_reporter.make_report(
                sccache_stats(0, 449), Path(tmp) / "missing.log", SOURCE, "READ_WRITE"
            )
            self.assertFalse(report["log_available"])
            self.assertEqual(report["unattributed_write_errors"], 449)
            self.assertEqual(report["health"], "DEGRADED")
            self.assertEqual(report["successful_write_percent"], 0.0)

    def test_partial_debug_log_reports_unattributed_errors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "partial.log"
            log.write_text("DEBUG Error executing cache write: status=429\n", encoding="utf-8")
            report = sccache_reporter.make_report(
                sccache_stats(2, 7), log, SOURCE, "READ_WRITE"
            )
            self.assertEqual(report["classes"]["rate_limited_429"], 1)
            self.assertEqual(report["unattributed_write_errors"], 6)
            self.assertEqual(report["health"], "DEGRADED")

    def test_excess_debug_events_are_explicitly_inconsistent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "extra.log"
            log.write_text("DEBUG Error executing cache write: unknown\n" * 2, encoding="utf-8")
            report = sccache_reporter.make_report(
                sccache_stats(2, 1), log, SOURCE, "READ_WRITE"
            )
            self.assertTrue(report["logged_events_exceed_stats"])
            self.assertEqual(report["health"], "DEGRADED")

    def test_pr_read_only_skips_are_expected_but_real_429_is_not(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "pr.log"
            log.write_text(
                "DEBUG Error executing cache write: Cannot write to read-only storage\n" * 3,
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(0, 3, 25, 75), log, SOURCE, "READ_ONLY"
            )
            self.assertEqual(report["health"], "EXPECTED_READ_ONLY")
            self.assertEqual(report["expected_read_only_skips"], 3)
            self.assertEqual(report["unexpected_write_errors"], 0)
            self.assertEqual(report["rust_hit_percent"], 25.0)
            log.write_text(
                "DEBUG Error executing cache write: Cannot write to read-only storage\n" * 3
                + "DEBUG Error executing cache write: HTTP 429 Too Many Requests\n",
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(0, 4), log, SOURCE, "READ_ONLY"
            )
            self.assertEqual(report["health"], "DEGRADED")
            self.assertEqual(report["unexpected_write_errors"], 1)

    def test_read_only_skips_not_counted_as_native_errors_are_still_expected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "pr.log"
            log.write_text(
                "DEBUG Error executing cache write: Cannot write to read-only storage\n" * 3,
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(0, 0, 9, 1), log, SOURCE, "READ_ONLY"
            )
            self.assertEqual(report["health"], "EXPECTED_READ_ONLY")
            self.assertEqual(report["read_only_skip_log_lines"], 3)
            self.assertEqual(report["expected_read_only_skips"], 0)
            self.assertFalse(report["logged_events_exceed_stats"])
            self.assertEqual(report["unexpected_write_failure_log_lines"], 0)
            # An HTTP 429 in the same read-only job must never be excused,
            # even if the native statistics have not counted the error yet.
            log.write_text(log.read_text(encoding="utf-8")
                           + "DEBUG Error executing cache write: HTTP 429 Too Many Requests\n",
                           encoding="utf-8")
            report = sccache_reporter.make_report(
                sccache_stats(0, 0), log, SOURCE, "READ_ONLY"
            )
            self.assertEqual(report["health"], "DEGRADED")
            self.assertEqual(report["classes"]["rate_limited_429"], 1)
            self.assertEqual(report["unexpected_write_failure_log_lines"], 1)
            self.assertTrue(report["logged_events_exceed_stats"])

    def test_missing_read_only_log_is_not_mistaken_for_expected_skips(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = sccache_reporter.make_report(
                sccache_stats(0, 3), Path(tmp) / "missing", SOURCE, "READ_ONLY"
            )
            self.assertEqual(report["health"], "DEGRADED")
            self.assertEqual(report["expected_read_only_skips"], 0)

    def test_logs_and_tokens_never_leave_safe_aggregate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "private-secrets.log"
            log.write_text(
                "DEBUG Error executing cache write: 429 token=private-secret "
                "URL=https://private.invalid/path\n",
                encoding="utf-8",
            )
            report = sccache_reporter.make_report(
                sccache_stats(0, 1), log, SOURCE, "READ_WRITE"
            )
            safe = json.dumps(report, sort_keys=True)
            self.assertNotIn("private-secret", safe)
            self.assertNotIn("private.invalid", safe)
            self.assertNotIn(str(log), safe)
            self.assertEqual(report["classes"], {"rate_limited_429": 1})
            summary = sccache_reporter.format_summary(report)
            self.assertNotIn("private-secret", summary)
            self.assertNotIn("private.invalid", summary)
            self.assertNotIn(str(log), summary)

    def test_invalid_source_mode_or_counter_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            missing = Path(tmp) / "missing.log"
            for source in ("main", "A" * 40, "", "f" * 39):
                with self.subTest(source=source), self.assertRaisesRegex(ValueError, "source SHA"):
                    sccache_reporter.make_report(sccache_stats(0, 0), missing, source, "READ_WRITE")
            with self.assertRaisesRegex(ValueError, "storage mode"):
                sccache_reporter.make_report(sccache_stats(0, 0), missing, SOURCE, "INVALID")
            for stats in (
                {"stats": {"cache_writes": 0}},
                sccache_stats(-1, 0), sccache_stats(0, -1),
                sccache_stats(True, 0), sccache_stats(0, True),
                {"stats": {"cache_writes": 0, "cache_write_errors": 0,
                            "cache_hits": {"counts": {"Rust": -1}},
                            "cache_misses": {"counts": {}}}},
            ):
                with self.subTest(stats=stats), self.assertRaisesRegex(ValueError, "sccache"):
                    sccache_reporter.make_report(stats, missing, SOURCE, "READ_WRITE")

    def test_empty_successful_writes_and_zero_errors_is_healthy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = sccache_reporter.make_report(
                sccache_stats(0, 0, 0, 0), Path(tmp) / "missing", SOURCE, "READ_WRITE"
            )
            self.assertEqual(report["health"], "OK")
            self.assertIsNone(report["successful_write_percent"])
            self.assertIsNone(report["rust_hit_percent"])

    def test_cli_uses_native_json_and_prints_safe_status_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "private-debug.log"
            log.write_text(
                "DEBUG Error executing cache write: 429 token=private-secret\n",
                encoding="utf-8",
            )
            completed = type("FakeResult", (), {"stdout": json.dumps(sccache_stats(8, 1, 1, 3))})()
            stdout = io.StringIO()
            with (patch.object(sccache_reporter.subprocess, "run", return_value=completed) as run,
                  patch.object(sccache_reporter.os, "environ", {
                      "CI_SOURCE_SHA": SOURCE, "SCCACHE_GHA_RW_MODE": "READ_WRITE",
                  }),
                  patch.object(sccache_reporter.argparse.ArgumentParser, "parse_args",
                               return_value=sccache_reporter.argparse.Namespace(log=log)),
                  redirect_stdout(stdout)):
                sccache_reporter.main()
            run.assert_called_once_with(
                ["sccache", "--show-stats", "--stats-format=json"],
                capture_output=True, text=True, check=True,
            )
            output = stdout.getvalue()
            self.assertIn('"health": "DEGRADED"', output)
            self.assertIn('"source_sha": "' + SOURCE + '"', output)
            self.assertIn("SCCACHE_CACHE_STATUS=DEGRADED", output)
            self.assertIn("SCCACHE_CACHE_SUMMARY=health=DEGRADED", output)
            self.assertIn("cache_writes=8 cache_write_errors=1 http_429=1", output)
            self.assertNotIn("private-secret", output)
            self.assertNotIn(str(log), output)

    def test_python_only_product_does_not_launch_rust_cache_diagnostics(self) -> None:
        source = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        architecture = source.split("  architecture-and-rust:\n", 1)[1].split("  product-truth:\n", 1)[0]
        product = source.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
        verify = source.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
        self.assertIn("bash automation/ci_fast_verify.sh product", product)
        self.assertNotIn("RUSTC_WRAPPER", product)
        self.assertNotIn("cargo test", product)
        self.assertNotIn("SCCACHE_LOG:", product)
        for block in (product, architecture, verify):
            self.assertNotIn("report_ci_sccache_write_errors.py", block)
        self.assertIn("SCCACHE_ERROR_LOG:", architecture)


if __name__ == "__main__":
    unittest.main()
