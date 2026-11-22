#!/usr/bin/env python3
"""Offline regression fixtures for the Rust dependency impact analyzer."""

from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

from ci_impact import analyze, changed_paths, workspace_graph


class ImpactFixtures(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path("/synthetic/repo")
        self.owners = {
            "app/rust/crates/contracts": "contracts",
            "app/rust/crates/primitives": "primitives",
            "app/rust/crates/graph_neo4j": "graph_neo4j",
            "app/rust/crates/http_client": "http_client",
            "app/rust/crates/infrastructure": "infrastructure",
            "app/rust/crates/integration_harness": "integration_harness",
            "app/rust/crates/seo_application": "seo_application",
            "app/rust/crates/seo_ports": "seo_ports",
            "app/rust/crates/vector_qdrant": "vector_qdrant",
            "app/rust/services/gsc_sync": "gsc_sync",
            "app/rust/services/analytics_svc": "analytics_svc",
            "app/rust/services/temporal": "temporal_worker",
            "app/rust/services/outbox_worker": "outbox_worker",
        }
        self.reverse = {
            "contracts": {"primitives"},
            "primitives": {"temporal_worker"},
            "graph_neo4j": {"analytics_svc", "temporal_worker"},
            "http_client": {"gsc_sync"},
            "infrastructure": set(),
            "integration_harness": set(),
            "seo_application": set(),
            "seo_ports": set(),
            "vector_qdrant": {"temporal_worker"},
            "gsc_sync": set(),
            "analytics_svc": set(),
            "temporal_worker": set(),
            "outbox_worker": set(),
        }

    def impact(self, *paths: str) -> dict:
        return analyze(list(paths), self.owners, self.reverse, head="HEAD", base="BASE")

    def test_isolated_gsc_does_not_trigger_other_services(self) -> None:
        result = self.impact("app/rust/services/gsc_sync/src/main.rs")
        self.assertEqual(result["affected_crates"], ["gsc_sync"])
        self.assertTrue(result["mandatory_full_ci_gate"])
        self.assertFalse(result["publishing_a17_required"])
        self.assertFalse(result["outbox_a14_required"])
        self.assertFalse(result["knowledge_a13_required"])

    def test_publishing_change_selects_real_a17_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/finalize_publish.rs")
        self.assertTrue(result["publishing_a17_required"])
        self.assertFalse(result["outbox_a14_required"])
        self.assertFalse(result["knowledge_a13_required"])

    def test_outbox_change_selects_real_a14_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_outbox_adapter.rs")
        self.assertTrue(result["outbox_a14_required"])
        self.assertFalse(result["publishing_a17_required"])
        self.assertFalse(result["knowledge_a13_required"])

    def test_knowledge_read_change_selects_postgres_a13(self) -> None:
        result = self.impact("app/rust/crates/seo_application/src/knowledge.rs")
        self.assertTrue(result["knowledge_a13_required"])
        self.assertFalse(result["outbox_a14_required"])
        self.assertFalse(result["publishing_a17_required"])

    def test_real_neighbor_adapter_change_selects_existing_qdrant_a13(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/semantic_neighbor_search_adapter.rs")
        self.assertEqual(result["analysis"], "scoped")
        self.assertTrue(result["knowledge_a13_required"])
        self.assertFalse(result["publishing_a17_required"])
        self.assertFalse(result["outbox_a14_required"])

    def test_editorial_source_context_port_changes_select_real_qdrant_a13(self) -> None:
        for path in (
            "app/rust/crates/seo_application/src/drafting.rs",
            "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertTrue(result["knowledge_a13_required"])

    def test_unknown_global_change_fails_closed_to_a17(self) -> None:
        result = self.impact("app/db/schema.sql")
        self.assertEqual(result["analysis"], "full")
        self.assertTrue(result["publishing_a17_required"])
        self.assertTrue(result["outbox_a14_required"])
        self.assertTrue(result["knowledge_a13_required"])

    def test_isolated_analytics(self) -> None:
        self.assertEqual(self.impact("app/rust/services/analytics_svc/src/main.rs")["affected_services"], ["analytics_svc"])

    def test_graph_change_includes_reverse_consumers(self) -> None:
        got = self.impact("app/rust/crates/graph_neo4j/src/lib.rs")
        self.assertEqual(set(got["affected_crates"]), {"graph_neo4j", "analytics_svc", "temporal_worker"})

    def test_http_change_includes_gsc_only(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/http_client/src/lib.rs")["affected_crates"]), {"http_client", "gsc_sync"})

    def test_vector_change_does_not_rebuild_unrelated_analytics_or_gsc(self) -> None:
        self.assertEqual(
            set(self.impact("app/rust/crates/vector_qdrant/src/lib.rs")["affected_crates"]),
            {"vector_qdrant", "temporal_worker"},
        )

    def test_common_contracts_are_transitive(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/contracts/src/lib.rs")["affected_crates"]), {"contracts", "primitives", "temporal_worker"})

    def test_global_inputs_fail_closed(self) -> None:
        paths = (
            "contracts/proto/temporal_payloads.proto",
            "app/db/schema.sql",
            "app/rust/Cargo.lock",
            ".github/workflows/ci.yml",
            "automation/check_seo_crate_graph.py",
            "infra/cloudflare/seo-static/package.json",
            "docker-compose.yml",
            "analytics_lab/proto/analytics.proto",
        )
        for path in paths:
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(outcome["analysis"], "full")
                self.assertEqual(len(outcome["affected_crates"]), len(self.reverse))

    def test_unknown_file_is_full_scope(self) -> None:
        self.assertEqual(self.impact("unmapped/thing.txt")["analysis"], "full")

    def test_multiple_changes_include_all_consumers(self) -> None:
        self.assertEqual(
            set(self.impact("app/rust/services/gsc_sync/src/main.rs", "app/rust/crates/graph_neo4j/src/lib.rs")["affected_crates"]),
            {"gsc_sync", "graph_neo4j", "analytics_svc", "temporal_worker"},
        )

    def test_optional_dev_and_build_path_dependencies_in_graph(self) -> None:
        packages = [
            {"id": "a", "name": "contracts", "manifest_path": "/synthetic/repo/app/rust/crates/contracts/Cargo.toml", "dependencies": []},
            {"id": "b", "name": "gsc_sync", "manifest_path": "/synthetic/repo/app/rust/services/gsc_sync/Cargo.toml", "dependencies": [
                {"name": "contracts", "kind": "dev", "optional": True, "path": "/synthetic/repo/app/rust/crates/contracts"}
            ]},
        ]
        owners, reverse = workspace_graph({"workspace_members": ["a", "b"], "packages": packages}, root=self.root)
        self.assertEqual(owners["app/rust/services/gsc_sync"], "gsc_sync")
        self.assertEqual(reverse["contracts"], {"gsc_sync"})

    def test_git_rename_includes_old_and_new_paths(self) -> None:
        with patch("subprocess.check_output", return_value=b"R100\0app/rust/crates/graph_neo4j/src/old.rs\0app/rust/crates/graph_neo4j/src/new.rs\0"):
            self.assertEqual(
                changed_paths("BASE", "HEAD", root=self.root),
                ["app/rust/crates/graph_neo4j/src/new.rs", "app/rust/crates/graph_neo4j/src/old.rs"],
            )

    def test_git_deletion_is_retained(self) -> None:
        with patch("subprocess.check_output", return_value=b"D\0app/rust/services/gsc_sync/src/main.rs\0"):
            self.assertEqual(changed_paths("BASE", "HEAD", root=self.root), ["app/rust/services/gsc_sync/src/main.rs"])

    def test_invalid_git_data_fails_closed(self) -> None:
        with patch("subprocess.check_output", return_value=b"R100\0old.rs\0"):
            with self.assertRaises(ValueError):
                changed_paths("BASE", "HEAD", root=self.root)


if __name__ == "__main__":
    unittest.main()
