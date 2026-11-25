#!/usr/bin/env python3
"""Fast, offline regression checks for the opt-in Cargo owner-build profiler."""

from __future__ import annotations

import unittest

from measure_owner_rebuild import (
    BUILD_COMMAND, EXECUTABLE_PACKAGES, OWNER_SOURCES, ROOT,
    compiler_unit, numeric_stats_delta, owner_of_artifact, probe_marker, sha256,
    verified_study_source,
)


def example_artifact(source: str, *, fresh: bool = False) -> dict:
    return {
        "reason": "compiler-artifact",
        "package_id": "path+file:///repo#graph_neo4j@0.1.0",
        "target": {
            "name": "graph_neo4j",
            "kind": ["lib"],
            "src_path": source,
        },
        "profile": {"opt_level": "0", "debuginfo": 0},
        "fresh": fresh,
        "executable": None,
    }


class OwnerRebuildTests(unittest.TestCase):
    def test_four_distinct_real_owner_sources(self) -> None:
        self.assertEqual(set(OWNER_SOURCES), {
            "postgres_store", "graph_neo4j", "vector_qdrant", "temporal_orchestrator",
        })
        for path in OWNER_SOURCES.values():
            self.assertTrue((ROOT / path).is_file(), path)

    def test_locked_runtime_binary_build_without_release_profile(self) -> None:
        self.assertEqual(BUILD_COMMAND[:5], ("cargo", "build", "--locked", "--bins", "--timings"))
        self.assertTrue({"temporal_worker", "temporal_orchestrator", "temporal_pg_effects"}
                        <= set(EXECUTABLE_PACKAGES))
        for package in EXECUTABLE_PACKAGES:
            self.assertIn(("-p", package), tuple(zip(BUILD_COMMAND, BUILD_COMMAND[1:])))
        self.assertNotIn("--release", BUILD_COMMAND)

    def test_artifact_keeps_package_profile_kind_fresh_and_owner(self) -> None:
        source = str(ROOT / OWNER_SOURCES["graph_neo4j"])
        record = example_artifact(source)
        unit = compiler_unit(record)
        assert unit is not None
        self.assertEqual(unit["owner"], "graph_neo4j")
        self.assertEqual(unit["package_id"], record["package_id"])
        self.assertEqual(unit["kind"], ["lib"])
        self.assertFalse(unit["fresh"])
        self.assertEqual(unit["profile"], record["profile"])
        self.assertEqual(owner_of_artifact(record), "graph_neo4j")

    def test_missing_compiler_unit_evidence_is_rejected(self) -> None:
        record = example_artifact(str(ROOT / OWNER_SOURCES["postgres_store"]))
        for missing in ("package_id", "fresh", "profile"):
            modified = dict(record)
            modified.pop(missing)
            self.assertIsNone(compiler_unit(modified), missing)
        self.assertIsNone(compiler_unit({"reason": "build-finished", "success": True}))

    def test_marker_changes_exact_bytes_without_exposing_public_api(self) -> None:
        marker = probe_marker("graph_neo4j", "123-4")
        self.assertIn(b"const OWNER_BUILD_PROBE_GRAPH_NEO4J:", marker)
        self.assertIn(b'"123-4_graph_neo4j"', marker)
        self.assertNotIn(b"pub ", marker)
        self.assertNotEqual(sha256(b"source\n"), sha256(b"source\n" + marker))

    def test_sccache_deltas_are_structural_and_numeric(self) -> None:
        before = {"counts": {"cache_hits": 10, "cache_misses": 4}, "label": "same"}
        after = {"counts": {"cache_hits": 18, "cache_misses": 5}, "label": "same"}
        self.assertEqual(numeric_stats_delta(before, after), {
            "counts.cache_hits": 8, "counts.cache_misses": 1,
        })

    def test_owner_probe_requires_explicit_dispatch_and_exact_green_source(self) -> None:
        sha = "a" * 40
        allowed = {
            "GITHUB_ACTIONS": "true", "GITHUB_EVENT_NAME": "workflow_dispatch",
            "GITHUB_RUN_ID": "123456", "CI_SOURCE_SHA": sha,
            "GITHUB_SHA": "b" * 40,  # a branch head must not replace explicit SHA
        }
        self.assertEqual(verified_study_source(sha, allowed), sha)
        for change in (
            {"GITHUB_EVENT_NAME": "pull_request"},
            {"GITHUB_RUN_ID": ""},
            {"CI_SOURCE_SHA": "refs/heads/main"},
            {"CI_SOURCE_SHA": "b" * 40},
        ):
            with self.subTest(change=change), self.assertRaisesRegex(RuntimeError, "dispatch|exact selected"):
                verified_study_source(sha, allowed | change)


if __name__ == "__main__":
    unittest.main()
