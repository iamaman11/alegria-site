#!/usr/bin/env python3
"""Fast offline contract checks for exact-SHA four-owner compiler summaries."""

from __future__ import annotations

import unittest

from summarize_owner_build_evidence import OWNERS, markdown, summarize


SHA = "a" * 40


def compiler_study() -> dict:
    return {
        "status": "FOUR_OWNER_ACTUAL_RUSTC_AB_CAPTURED", "source_sha": SHA,
        "actual_rustc_evidence": True, "measurement_scope": "three binaries",
        "compiler": {"four_owner_probes": [
            {"owner": owner, "elapsed_seconds": 1.5, "cargo_non_fresh_bin_targets": ["temporal_worker"],
             "binary_files_written": ["temporal_worker"]}
            for owner in sorted(OWNERS)
        ]},
    }


class OwnerBuildEvidenceSummaryTests(unittest.TestCase):
    def test_compiler_study_requires_all_four_owners_and_echoes_actual_relinks(self) -> None:
        result = summarize(compiler_study(), "compiler-owners", SHA)
        self.assertEqual(len(result["owners"]), 4)
        self.assertTrue(all(owner["elapsed_seconds"] == 1.5 for owner in result["owners"]))
        self.assertTrue(all(owner["relinked_bins"] == ["temporal_worker"] for owner in result["owners"]))
        self.assertIn("NOT_PRODUCTION_ACCEPTANCE", result["status"])
        self.assertIn("not** proof", markdown(result))

    def test_rejects_missing_duplicate_and_unknown_owners(self) -> None:
        for invalid in ("missing", "duplicate", "unknown"):
            report = compiler_study()
            entries = report["compiler"]["four_owner_probes"]
            if invalid == "missing":
                entries.pop()
            elif invalid == "duplicate":
                entries[0]["owner"] = entries[1]["owner"]
            else:
                entries[0]["owner"] = "unknown_db"
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                summarize(report, "compiler-owners", SHA)

    def test_source_sha_and_status_are_required(self) -> None:
        report = compiler_study()
        for sha in ("b" * 40, "short"):
            with self.subTest(sha=sha), self.assertRaises(ValueError):
                summarize(report, "compiler-owners", sha)
        report["actual_rustc_evidence"] = False
        with self.assertRaisesRegex(ValueError, "actual Rust compiler"):
            summarize(report, "compiler-owners", SHA)

    def test_rejects_non_numeric_and_nonfinite_durations(self) -> None:
        for bad in (-1, "1.5", float("inf"), float("nan"), None):
            report = compiler_study()
            report["compiler"]["four_owner_probes"][0]["elapsed_seconds"] = bad
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                summarize(report, "compiler-owners", SHA)

    def test_rejects_duplicates_in_relink_lists(self) -> None:
        report = compiler_study()
        report["compiler"]["four_owner_probes"][0]["cargo_non_fresh_bin_targets"] = ["x", "x"]
        with self.assertRaisesRegex(ValueError, "duplicate binary"):
            summarize(report, "compiler-owners", SHA)

    def test_full_seven_binary_owner_rebuild_mode_uses_own_schema(self) -> None:
        report = {
            "status": "EVIDENCE_CAPTURED", "source_sha": SHA,
            "probes": [
                {"owner": owner, "build": {"elapsed_seconds": 2.75},
                 "relinked_binaries": ["outbox_worker"], "binary_files_written": ["outbox_worker"]}
                for owner in sorted(OWNERS)
            ],
        }
        output = summarize(report, "owner-rebuild", SHA)
        self.assertEqual(output["study"], "owner-rebuild")
        self.assertEqual({item["owner"] for item in output["owners"]}, OWNERS)


if __name__ == "__main__":
    unittest.main()
