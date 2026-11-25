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


def missing_workspace_lock_dependencies(rust_root: Path) -> list[str]:
    """Detect missing lock entries for *every* local Cargo path dependency.

    Unlike a `projection_contracts`-only assertion, this also catches an
    optional `postgres_store` edge added to an integration executable. Include
    dev/build/target-specific dependencies; Cargo resolves these into the
    workspace lock, even when a particular CI feature does not build them.
    """
    workspace = tomllib.loads((rust_root / "Cargo.toml").read_text())
    lock = tomllib.loads((rust_root / "Cargo.lock").read_text())
    locked_packages = {
        (package["name"], package["version"]): package
        for package in lock["package"]
        if "source" not in package
    }
    missing: list[str] = []
    for relative in workspace["workspace"]["members"]:
        owner_path = rust_root / relative / "Cargo.toml"
        manifest = tomllib.loads(owner_path.read_text())
        package = manifest["package"]
        key = (package["name"], package["version"])
        locked_owner = locked_packages.get(key)
        if locked_owner is None:
            missing.append(f"{package['name']}: package missing from Cargo.lock")
            continue
        resolved = set(locked_owner.get("dependencies", []))
        blocks = [manifest, *(target for target in manifest.get("target", {}).values()
                               if isinstance(target, dict))]
        for block in blocks:
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                for alias, config in block.get(section, {}).items():
                    if not isinstance(config, dict) or "path" not in config:
                        continue
                    target_path = (owner_path.parent / config["path"] / "Cargo.toml").resolve()
                    target = tomllib.loads(target_path.read_text())["package"]["name"]
                    if target not in resolved:
                        missing.append(
                            f"{package['name']}: local `{alias}` -> `{target}` missing from Cargo.lock"
                        )
    return missing


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
        self.assertEqual(missing_workspace_lock_dependencies(RUST_ROOT), [])

    def test_optional_new_owner_dependency_missing_from_lock_is_caught(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rust = Path(tmp)
            (rust / "Cargo.toml").write_text('[workspace]\nmembers = ["a", "b"]\n')
            for name in ("a", "b"):
                (rust / name).mkdir()
            (rust / "a/Cargo.toml").write_text(
                '[package]\nname="a"\nversion="0.1.0"\n'
                '[dependencies]\nb = { path="../b", optional=true }\n'
            )
            (rust / "b/Cargo.toml").write_text('[package]\nname="b"\nversion="0.1.0"\n')
            (rust / "Cargo.lock").write_text(
                'version=4\n[[package]]\nname="a"\nversion="0.1.0"\n'
                '[[package]]\nname="b"\nversion="0.1.0"\n'
            )
            self.assertEqual(missing_workspace_lock_dependencies(rust), [
                'a: local `b` -> `b` missing from Cargo.lock',
            ])

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
