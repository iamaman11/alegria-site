#!/usr/bin/env python3
"""Fast CI regressions: native DAG owners may not import unrelated providers.

This tests the declared dependency closure and the narrow generic source
boundary, not an imaginary rustc A/B result. The frozen Qdrant V1 parent
still links qdrant_contracts and remains an explicit physical cutover blocker.
"""

from __future__ import annotations

from pathlib import Path
import tempfile
import tomllib
import unittest

from check_rust_adapter_boundaries import (
    ORCHESTRATOR_MANIFEST,
    DAG_PROVIDER_CRATES,
    generic_dag_source_failures,
    native_dag_owner_closure_failures,
    workspace_dependency_closure,
)


class NativeDagIsolationTests(unittest.TestCase):
    def test_new_projection_contract_package_and_consumers_have_exact_lock_entries(self) -> None:
        """Missing `cargo` locally must not silently leave a stale --locked build.

        Derive the expected crate dependency keys from its actual manifest,
        including Rust workspaces where a registry package lock entry carries
        a disambiguating version suffix (e.g. `thiserror 2.0.21`).
        """
        from check_rust_adapter_boundaries import RUST_ROOT

        workspace = tomllib.loads((RUST_ROOT / "Cargo.toml").read_text())
        self.assertIn("crates/projection_contracts", workspace["workspace"]["members"])
        manifest = tomllib.loads(
            (RUST_ROOT / "crates/projection_contracts/Cargo.toml").read_text()
        )
        lock = tomllib.loads((RUST_ROOT / "Cargo.lock").read_text())
        packages = {entry["name"]: entry for entry in lock["package"]
                    if entry.get("version") == "0.1.0"}
        locked = packages["projection_contracts"]
        self.assertEqual(
            {entry.split(" ", 1)[0] for entry in locked["dependencies"]},
            set(manifest["dependencies"]),
        )
        self.assertEqual(locked["version"], manifest["package"]["version"])
        for relative in workspace["workspace"]["members"]:
            owner_manifest = tomllib.loads((RUST_ROOT / relative / "Cargo.toml").read_text())
            deps = owner_manifest.get("dependencies", {})
            if "projection_contracts" in deps:
                self.assertIn("projection_contracts", packages[owner_manifest["package"]["name"]]["dependencies"])

    def test_current_native_owners_are_independent_in_cargo_closure(self) -> None:
        from check_rust_adapter_boundaries import (
            OPERATION_PROTOCOL_MANIFEST,
            PG_OWNER_MANIFEST,
            QDRANT_OWNER_MANIFEST,
        )

        protocol, _ = workspace_dependency_closure(OPERATION_PROTOCOL_MANIFEST)
        pg, _ = workspace_dependency_closure(PG_OWNER_MANIFEST)
        qdrant, _ = workspace_dependency_closure(QDRANT_OWNER_MANIFEST)
        self.assertEqual(native_dag_owner_closure_failures(protocol, pg, qdrant), [])
        self.assertNotIn("qdrant_contracts", protocol)
        self.assertNotIn("qdrant_contracts", pg)
        self.assertNotIn("postgres_store", qdrant)

    def test_future_direct_or_transitive_owner_dependencies_fail_closed(self) -> None:
        cases = (
            ({"operation_protocol", "qdrant_contracts"}, {"temporal_pg_effects"},
             {"temporal_qdrant_effects"}, "operation_protocol links provider"),
            ({"operation_protocol"}, {"temporal_pg_effects", "vector_qdrant"},
             {"temporal_qdrant_effects"}, "native PostgreSQL effect owner"),
            ({"operation_protocol"}, {"temporal_pg_effects"},
             {"temporal_qdrant_effects", "postgres_store"}, "native Qdrant effect owner"),
        )
        for protocol, pg, qdrant, expected in cases:
            with self.subTest(expected=expected):
                self.assertTrue(any(expected in failure for failure in
                                    native_dag_owner_closure_failures(protocol, pg, qdrant)))

    def test_source_boundary_rejects_other_owner_rust_import(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "owner.rs"
            source.write_text('use qdrant_contracts::QdrantCommand;\n')
            result = generic_dag_source_failures(source, {"qdrant_contracts"})
            self.assertEqual(len(result), 1)
            self.assertIn("imports unrelated provider", result[0])
            source.write_text('let owner_id = "qdrant_contracts";\n')
            self.assertEqual(generic_dag_source_failures(source, {"qdrant_contracts"}), [])

    def test_physical_parent_is_not_mislabeled_as_isolated(self) -> None:
        closure, _ = workspace_dependency_closure(ORCHESTRATOR_MANIFEST)
        self.assertEqual(closure & DAG_PROVIDER_CRATES, {"qdrant_contracts"})


if __name__ == "__main__":
    unittest.main()
