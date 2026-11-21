#!/usr/bin/env python3
"""Offline positive and negative contracts for Product/Truth cache diagnostics."""
from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "automation/report_ci_sccache_write_errors.py"
WORKFLOW = ROOT / ".github/workflows/ci.yml"
spec = importlib.util.spec_from_file_location("sccache_diagnostic", SCRIPT)
assert spec is not None and spec.loader is not None
reporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reporter)
SHA = "a" * 40


class SccacheDiagnosticContract(unittest.TestCase):
    def test_all_error_families_and_only_write_failures_are_counted(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "debug.log"
            log.write_text(
                "DEBUG Error executing cache write: HTTP 429 Too Many Requests\n"
                "DEBUG Error executing cache write: status=403 Forbidden\n"
                "DEBUG Error executing cache write: status 503 unavailable\n"
                "DEBUG Error executing cache write: storage quota exceeded\n"
                "DEBUG Error executing cache write: deadline exceeded\n"
                "DEBUG Error executing cache write: backend storage failed\n"
                "DEBUG successful cache read HTTP 429\n",
                encoding="utf-8",
            )
            report = reporter.make_report({"cache_write_errors": 6, "cache_writes": 10}, log, SHA)
            self.assertEqual(report["logged_write_error_events"], 6)
            self.assertEqual(report["unattributed_write_errors"], 0)
            self.assertEqual(report["write_error_categories"], dict.fromkeys(reporter.CATEGORIES, 1))

    def test_missing_debug_log_does_not_pretend_errors_were_explained(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            report = reporter.make_report(
                {"cache_write_errors": 449, "cache_writes": 0}, Path(temp) / "missing", SHA
            )
            self.assertFalse(report["debug_log_present"])
            self.assertEqual(report["unattributed_write_errors"], 449)

    def test_partial_debug_log_reports_unattributed_errors(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "partial.log"
            log.write_text("DEBUG Error executing cache write: status=429\n", encoding="utf-8")
            report = reporter.make_report({"cache_write_errors": 7, "cache_writes": 2}, log, SHA)
            self.assertEqual(report["write_error_categories"]["http_429"], 1)
            self.assertEqual(report["unattributed_write_errors"], 6)

    def test_excess_debug_events_are_marked_as_inconsistent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "extra.log"
            log.write_text("DEBUG Error executing cache write: unknown\n" * 2, encoding="utf-8")
            report = reporter.make_report({"cache_write_errors": 1, "cache_writes": 2}, log, SHA)
            self.assertTrue(report["logged_events_exceed_stats"])

    def test_raw_log_and_access_token_are_not_emitted_or_saved(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "secrets.log"
            log.write_text(
                "DEBUG Error executing cache write: status=429 token=private-secret URL=https://private.invalid\n",
                encoding="utf-8",
            )
            report = reporter.make_report({"cache_write_errors": 1, "cache_writes": 0}, log, SHA)
            safe = json.dumps(report)
            self.assertNotIn("private-secret", safe)
            self.assertNotIn("private.invalid", safe)
            self.assertNotIn(str(log), safe)

    def test_invalid_source_sha_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(ValueError, "exact PR source"):
                reporter.make_report({"cache_write_errors": 0, "cache_writes": 0}, Path(temp) / "missing", "main")

    def test_missing_or_negative_or_boolean_counters_rejected(self) -> None:
        for stats in (
            {"cache_writes": 0}, {"cache_write_errors": -1, "cache_writes": 2},
            {"cache_write_errors": True, "cache_writes": 2},
        ):
            with self.subTest(stats=stats), tempfile.TemporaryDirectory() as temp:
                with self.assertRaisesRegex(ValueError, "sccache counter"):
                    reporter.make_report(stats, Path(temp) / "missing", SHA)

    def test_workflow_only_diagnoses_product_and_never_uploads_raw_log(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        architecture = source.split("  architecture-and-rust:\n", 1)[1].split("  rust-contract-smokes:\n", 1)[0]
        smokes = source.split("  rust-contract-smokes:\n", 1)[1].split("  product-truth:\n", 1)[0]
        product = source.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
        verify = source.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
        self.assertIn('SCCACHE_GHA_RW_MODE: "READ_ONLY"', product)
        self.assertIn('SCCACHE_LOG: "sccache::server=debug"', product)
        self.assertIn('SCCACHE_ERROR_LOG: /tmp/alegria-product-sccache.log', product)
        self.assertNotIn("${{ runner.temp }}", product)
        self.assertEqual(product.count("python3 automation/report_ci_sccache_write_errors.py --job product-truth"), 1)
        self.assertIn("automation/reports/sccache_diagnostic_product-truth.json", product)
        upload = product.split("- name: Upload measured Product/Truth phase timing", 1)[1]
        self.assertNotIn("alegria-product-sccache.log", upload)
        self.assertNotIn("SCCACHE_ERROR_LOG", upload)
        for other in (architecture, smokes, verify):
            self.assertNotIn("SCCACHE_GHA_RW_MODE:", other)
            self.assertNotIn("SCCACHE_LOG:", other)
            self.assertNotIn("report_ci_sccache_write_errors.py", other)
        self.assertEqual(source.count("SCCACHE_ERROR_LOG:"), 1)


if __name__ == "__main__":
    unittest.main()
