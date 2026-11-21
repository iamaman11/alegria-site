#!/usr/bin/env python3
"""Offline safety contract for the single-writer, target-free Cargo A/B leg."""
from __future__ import annotations

import re
import tempfile
import unittest
from pathlib import Path

from report_ci_cargo_cache import directory_bytes, record

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "a" * 40
OWNERS = ("architecture-and-rust", "product-truth", "verify")


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
    saver, *readers = blocks
    if saver.count("uses: actions/cache@v4") != 1:
        raise ValueError("architecture must be the sole registry cache writer")
    if any("uses: actions/cache@v4" in block for block in readers):
        raise ValueError("read-only jobs must not publish a competing cache")
    if any(block.count("uses: actions/cache/restore@v4") != 1 for block in readers):
        raise ValueError("read-only jobs must restore dependency sources")
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
        if "report_ci_cargo_cache.py" not in block:
            raise ValueError("missing per-owner footprint report")


class CargoCachePolicy(unittest.TestCase):
    def test_actual_ci_has_one_registry_writer_and_no_target_archive(self) -> None:
        validate_cache_policy((ROOT / ".github/workflows/ci.yml").read_text())

    def test_a_second_saver_is_rejected(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace("uses: actions/cache/restore@v4", "uses: actions/cache@v4", 1)
        with self.assertRaisesRegex(ValueError, "read-only jobs"):
            validate_cache_policy(changed)

    def test_unbounded_target_regression_is_rejected(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        changed = original.replace("            ~/.cargo/git\n", "            ~/.cargo/git\n            app/rust/target\n", 1)
        with self.assertRaisesRegex(ValueError, "unbounded target"):
            validate_cache_policy(changed)
        single_path = original.replace("          path: |\n", "          path: app/rust/target\n", 1)
        with self.assertRaisesRegex(ValueError, "unbounded target"):
            validate_cache_policy(single_path)

    def test_target_reuse_without_archiving_is_permitted(self) -> None:
        original = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("WHOLE_PAGE_SEMANTIC_TARGET_DIR:", original)
        validate_cache_policy(original)

    def test_metrics_distinguish_uncached_target_and_missing_registry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "registry").mkdir()
            (root / "registry" / "a").write_bytes(b"x" * 123)
            target = root / "target"
            target.mkdir()
            (target / "b").write_bytes(b"y" * 234)
            report = record("verify", "false", SOURCE, root, target)
            self.assertEqual(report["source_sha"], SOURCE)
            self.assertFalse(report["registry_cache_hit"])
            self.assertGreaterEqual(report["registry_bytes"], 123)
            self.assertGreaterEqual(report["target_bytes_uncached"], 234)
            self.assertEqual(report["git_bytes"], 0)

    def test_rejects_invalid_cache_signal_and_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                record("verify", "unknown", SOURCE, root, root)
            with self.assertRaises(ValueError):
                record("verify", "true", "not-sha", root, root)
            with self.assertRaises(ValueError):
                record("unrecognized", "true", SOURCE, root, root)

    def test_missing_directory_is_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(directory_bytes(Path(tmp) / "absent"), 0)


if __name__ == "__main__":
    unittest.main()
