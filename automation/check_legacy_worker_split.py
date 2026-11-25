#!/usr/bin/env python3
"""Offline guard for immutable, cold, non-bypassing Worker split measurements."""

from __future__ import annotations

import unittest
from unittest.mock import patch
import json
from pathlib import Path
import subprocess
import tempfile

from compare_physical_build_options import EvidenceError, workspace_graph
from measure_legacy_worker_split import (
    VARIANTS, build_command, measure, resolved_locked_runtime_graph, summarize_build,
    transitive_workspace_closure,
)


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
                self.assertIn("--message-format=json-render-diagnostics", command)
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

    def test_actual_locked_graph_includes_vendor_and_filters_dev_only_edges(self) -> None:
        packages = [
            {"id": name, "name": name, "source": None}
            for name in ("temporal_worker", "outbox_worker", "shared_owner")
        ] + [
            {"id": "external", "name": "neo4rs", "source": "registry+https://example.invalid"},
            {"id": "dev", "name": "dev_only", "source": "registry+https://example.invalid"},
        ]
        metadata = {"packages": packages,
                    "workspace_members": ["temporal_worker", "outbox_worker", "shared_owner"],
                    "resolve": {"nodes": [
                        {"id": "temporal_worker", "deps": [
                            {"pkg": "shared_owner", "dep_kinds": [{"kind": None}]},
                            {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]},
                        ]},
                        {"id": "outbox_worker", "deps": [
                            {"pkg": "shared_owner", "dep_kinds": [{"kind": "build"}]},
                        ]},
                        {"id": "shared_owner", "deps": [
                            {"pkg": "external", "dep_kinds": [{"kind": None}]},
                        ]},
                        {"id": "external", "deps": []},
                        {"id": "dev", "deps": []},
                    ]}}

        def simulated(command, **_kwargs):
            if command == ["rustc", "-vV"]:
                return subprocess.CompletedProcess(command, 0, "rustc 1.0\nhost: x86_64-unknown-linux-gnu\n", "")
            return subprocess.CompletedProcess(command, 0, json.dumps(metadata), "")

        with patch("measure_legacy_worker_split.subprocess.run", side_effect=simulated) as runner:
            report = resolved_locked_runtime_graph(Path("/isolated/exact-source"))
        self.assertEqual(report["shared_workspace_packages"], ["shared_owner"])
        self.assertIn("neo4rs", report["shared_resolved_packages"])
        self.assertNotIn("dev_only", report["temporal_worker"]["all_resolved_packages"])
        self.assertIn("--filter-platform", runner.call_args_list[1].args[0])
        self.assertEqual(report["temporal_worker"]["resolved_package_count"], 3)

    def test_locked_metadata_unknown_normal_edge_fails_closed(self) -> None:
        packages = [{"id": name, "name": name, "source": None}
                    for name in ("temporal_worker", "outbox_worker")]
        metadata = {"packages": packages, "workspace_members": [p["id"] for p in packages],
                    "resolve": {"nodes": [
                        {"id": "temporal_worker", "deps": [
                            {"pkg": "omitted", "dep_kinds": [{"kind": None}]}]},
                        {"id": "outbox_worker", "deps": []},
                    ]}}
        def fake(command, **_kwargs):
            return subprocess.CompletedProcess(
                command, 0,
                "host: x86_64-unknown-linux-gnu\n" if command[0] == "rustc" else json.dumps(metadata), "",
            )
        with patch("measure_legacy_worker_split.subprocess.run", side_effect=fake):
            with self.assertRaisesRegex(EvidenceError, "incomplete Cargo dependency kind"):
                resolved_locked_runtime_graph(Path("/isolated/exact-source"))

    def test_real_binary_observation_requires_artifact_path(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "omitted executable targets"):
            summarize_build({"compiler_artifacts": [], "non_fresh_units": []},
                            ("temporal_worker",))

    def test_split_probe_compiles_all_three_variants_cold_and_warm_in_distinct_targets(self) -> None:
        observed: dict[str, dict] = {}
        graph = {
            "temporal_worker": {"shared"},
            "outbox_worker": {"shared"},
            "shared": set(),
        }

        def simulate_build(command, _workspace, **kwargs):
            self.assertTrue(kwargs["isolated_target"])
            self.assertTrue(kwargs["measure_resources"])
            target = kwargs["target_dir"]
            packages = tuple(command[index + 1]
                             for index, part in enumerate(command) if part == "-p")
            variant = next(name for name, names in VARIANTS.items() if names == packages)
            state = observed.setdefault(variant, {"target": target, "invocations": 0})
            self.assertEqual(state["target"], target)
            is_cold = state["invocations"] == 0
            state["invocations"] += 1
            target.mkdir(parents=True, exist_ok=True)
            units = []
            for package in packages:
                executable = target / package
                if is_cold:
                    executable.write_bytes(f"compiled-{package}".encode())
                units.append({"kind": ["bin"], "target_name": package,
                              "executable": str(executable), "fresh": not is_cold})
            return {"compiler_artifacts": units,
                    "non_fresh_units": units if is_cold else [],
                    "elapsed_seconds": 30 if is_cold else 1}

        with (
            tempfile.TemporaryDirectory() as tmp,
            patch("measure_legacy_worker_split.workspace_graph", return_value=(graph, {})),
            patch("measure_legacy_worker_split.resolved_locked_runtime_graph", return_value={"real": True}),
            patch("measure_legacy_worker_split.compiler_artifacts", side_effect=simulate_build),
        ):
            root = Path(tmp)
            report = measure(root, root)
        self.assertEqual(set(observed), set(VARIANTS))
        self.assertEqual({variant: state["invocations"] for variant, state in observed.items()},
                         {variant: 2 for variant in VARIANTS})
        self.assertEqual(len({str(state["target"]) for state in observed.values()}), 3)
        self.assertEqual(report["locked_resolved_cargo_graph"], {"real": True})
        self.assertEqual(report["separate_cold_targets"]["combined"]["warm_observed"][
            "actual_non_fresh_units"], 0)


if __name__ == "__main__":
    unittest.main()
