#!/usr/bin/env python3
"""Offline guard for immutable, cold, non-bypassing Worker split measurements."""

from __future__ import annotations

import unittest

from compare_physical_build_options import EvidenceError, workspace_graph
from measure_legacy_worker_split import VARIANTS, build_command, transitive_workspace_closure


class LegacyWorkerSplitContract(unittest.TestCase):
    def test_exact_compiler_targets_and_packages(self) -> None:
        self.assertEqual(VARIANTS, {
            "combined": ("temporal_worker", "outbox_worker"),
            "temporal_only": ("temporal_worker",),
            "outbox_only": ("outbox_worker",),
        })
        for packages in VARIANTS.values():
            with self.subTest(packages=packages):
                command = build_command(packages)
                self.assertEqual(command[:4], ["cargo", "build", "--locked", "--timings"])
                self.assertEqual(command.count("-p"), len(packages))
                self.assertEqual(command.count("--bin"), len(packages))
                for package in packages:
                    self.assertIn(package, command)

    def test_unknown_or_missing_runtime_cannot_masquerade_as_worker(self) -> None:
        for names in ((), ("temporal_orchestrator",), ("outbox_worker", "fake")):
            with self.subTest(names=names):
                with self.assertRaises(EvidenceError):
                    build_command(names)

    def test_owner_graph_is_the_real_manifest_closure(self) -> None:
        graph, _ = workspace_graph()
        temporal = transitive_workspace_closure(graph, "temporal_worker")
        outbox = transitive_workspace_closure(graph, "outbox_worker")
        self.assertTrue({"temporal_worker", "infrastructure", "runtime_composition",
                         "graph_neo4j", "vector_qdrant", "postgres_store"}.issubset(temporal))
        self.assertTrue({"outbox_worker", "projection_delivery", "vector_qdrant",
                         "postgres_store"}.issubset(outbox))
        self.assertGreater(len(temporal & outbox), 8)
        self.assertEqual(transitive_workspace_closure(graph, "temporal_orchestrator") & {
            "infrastructure", "runtime_composition", "vector_qdrant", "graph_neo4j",
        }, set())
        with self.assertRaises(EvidenceError):
            transitive_workspace_closure(graph, "no_such_worker")


if __name__ == "__main__":
    unittest.main()
