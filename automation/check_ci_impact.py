#!/usr/bin/env python3
"""Offline regression fixtures for the Rust dependency impact analyzer."""

from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

from ci_impact import CI_PR_OWNERS, SUITE_OWNERS, analyze, changed_paths, suite_commands_for_report, workspace_graph


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
        self.assertEqual(result["selected_suites"], [])
        self.assertEqual(result["selected_images"], [])

    def test_temporal_source_selects_real_restart_e2e_and_worker_image(self) -> None:
        result = self.impact("app/rust/services/temporal/src/main.rs")
        self.assertEqual(result["affected_crates"], ["temporal_worker"])
        self.assertEqual(result["selected_suites"], ["temporal_restart"])
        self.assertEqual(result["selected_images"], ["temporal-worker"])
        selected = SUITE_OWNERS[result["selected_suites"][0]]
        self.assertEqual(selected["prepare"][:3], ("cargo", "build", "--locked"))
        self.assertEqual(set(selected["runtime_binaries"]), {
            "INTEGRATION_TEMPORAL_WORKER_BIN", "INTEGRATION_OUTBOX_WORKER_BIN",
        })
        self.assertNotIn("--target-dir", selected["prepare"])

    def test_publishing_change_selects_real_a17_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/finalize_publish.rs")
        self.assertEqual(result["selected_suites"], ["publishing_a17"])

    def test_outbox_change_selects_real_a14_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_outbox_adapter.rs")
        self.assertEqual(result["selected_suites"], ["outbox_a14"])

    def test_verified_rule_projection_effects_select_real_outbox_and_knowledge_integrations(self) -> None:
        path = "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs"
        result = self.impact(path)
        self.assertEqual(result["analysis"], "scoped")
        self.assertEqual(result["selected_suites"], ["knowledge_a13", "outbox_a14"])

    def test_knowledge_read_change_selects_postgres_a13(self) -> None:
        result = self.impact("app/rust/crates/seo_application/src/knowledge.rs")
        self.assertEqual(result["selected_suites"], ["knowledge_a13"])

    def test_real_neighbor_adapter_change_selects_existing_qdrant_a13(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/semantic_neighbor_search_adapter.rs")
        self.assertEqual(result["analysis"], "scoped")
        self.assertEqual(result["selected_suites"], ["knowledge_a13"])

    def test_editorial_source_context_port_changes_select_real_qdrant_a13(self) -> None:
        for path in (
            "app/rust/crates/seo_application/src/drafting.rs",
            "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertIn("knowledge_a13", result["selected_suites"])

    def test_unknown_global_change_fails_closed_to_a17(self) -> None:
        result = self.impact("app/db/schema.sql")
        self.assertEqual(result["analysis"], "full")
        self.assertIn("publishing_a17", result["selected_suites"])
        self.assertIn("outbox_a14", result["selected_suites"])
        self.assertIn("knowledge_a13", result["selected_suites"])

    def test_isolated_analytics(self) -> None:
        self.assertEqual(self.impact("app/rust/services/analytics_svc/src/main.rs")["affected_services"], ["analytics_svc"])

    def test_graph_change_includes_reverse_consumers(self) -> None:
        got = self.impact("app/rust/crates/graph_neo4j/src/lib.rs")
        self.assertEqual(set(got["affected_crates"]), {"graph_neo4j", "analytics_svc", "temporal_worker"})
        self.assertEqual(got["selected_suites"], ["graph_query"])

    def test_http_change_includes_gsc_only(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/http_client/src/lib.rs")["affected_crates"]), {"http_client", "gsc_sync"})

    def test_vector_change_does_not_rebuild_unrelated_analytics_or_gsc(self) -> None:
        result = self.impact("app/rust/crates/vector_qdrant/src/lib.rs")
        self.assertEqual(
            set(result["affected_crates"]),
            {"vector_qdrant", "temporal_worker"},
        )
        self.assertEqual(result["selected_suites"], ["vector_projection"])

    def test_common_contracts_are_transitive(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/contracts/src/lib.rs")["affected_crates"]), {"contracts", "primitives", "temporal_worker"})

    def test_global_inputs_fail_closed(self) -> None:
        paths = (
            "contracts/proto/temporal_payloads.proto",
            "app/db/schema.sql",
            "app/rust/Cargo.lock",
            ".github/workflows/ci.yml",
            "automation/check_seo_crate_graph.py",
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

    def test_explanatory_markdown_needs_no_rust_suite_or_image(self) -> None:
        outcome = self.impact("docs/ARCHITECTURE_BOUNDARIES.md")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_images"], [])
        self.assertEqual(outcome["selected_ci_jobs"], [])
        self.assertEqual(set(outcome["ci_job_status"].values()), {"not_applicable"})
        self.assertIn("documentation", outcome["classified_inputs"]["docs/ARCHITECTURE_BOUNDARIES.md"])

    def test_historical_truth_report_is_not_a_live_rust_fixture(self) -> None:
        outcome = self.impact("docs/runs/r3_contradiction_handling.json")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_ci_jobs"], ["product-truth", "verify"])
        self.assertIn("historical", outcome["classified_inputs"]["docs/runs/r3_contradiction_handling.json"])

    def test_policy_markdown_still_runs_source_acceptance_without_rust_or_docker(self) -> None:
        outcome = self.impact("docs/OPS_RUNTIME_RUNBOOK.md")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_ci_jobs"], ["product-truth", "verify"])
        self.assertEqual(outcome["selected_images"], [])
        self.assertIn("source-policy", outcome["classified_inputs"]["docs/OPS_RUNTIME_RUNBOOK.md"])

    def test_analytics_lab_python_uses_python_owner_not_rust(self) -> None:
        outcome = self.impact("infra/analytics_lab/src/runner.py")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_ci_jobs"], ["verify"])

    def test_container_delivery_input_without_narrow_owner_fails_closed(self) -> None:
        for path in ("app/rust/services/temporal/Dockerfile", "infra/analytics_lab/Dockerfile"):
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(set(outcome["selected_ci_jobs"]), set(CI_PR_OWNERS))

    def test_live_truth_fixture_selects_its_real_e2e_without_other_images(self) -> None:
        path = "app/rust/crates/integration_harness/fixtures/visa_truth_certification/happy_path_publish_allowed_source.html"
        outcome = self.impact(path)
        self.assertEqual(outcome["scope"], "scoped")
        self.assertEqual(outcome["direct_crates"], ["integration_harness"])
        self.assertEqual(outcome["selected_suites"], ["truth_certification"])
        self.assertEqual(outcome["selected_images"], [])
        self.assertIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("product-truth", outcome["selected_ci_jobs"])
        suite_name, command = suite_commands_for_report(outcome)[0]
        self.assertEqual(suite_name, "truth_certification")
        self.assertIn("truth_certification_suite_matches_fixture_expectations", command)

    def test_delivery_script_selects_temporal_worker_image_without_rust(self) -> None:
        path = "automation/deploy_seo_static_cloudflare.py"
        outcome = self.impact(path)
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])
        self.assertNotIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("cloudflare-static-build", outcome["selected_ci_jobs"])
        self.assertIn("product-truth", outcome["selected_ci_jobs"])
        self.assertIn("verify", outcome["selected_ci_jobs"])
        self.assertIn(path, outcome["image_reasons"]["temporal-worker"][0])

    def test_unrelated_service_edit_selects_no_temporal_image(self) -> None:
        outcome = self.impact("app/rust/services/analytics_svc/src/main.rs")
        self.assertEqual(outcome["selected_images"], ["analytics-svc"])
        self.assertEqual(outcome["image_status"]["temporal-worker"], "not_applicable")

    def test_outbox_service_change_runs_real_outbox_suite_and_own_image(self) -> None:
        outcome = self.impact("app/rust/services/outbox_worker/src/worker.rs")
        self.assertEqual(outcome["selected_suites"], ["outbox_a14"])
        self.assertEqual(outcome["selected_images"], ["outbox-worker"])

    def test_cloudflare_static_is_worker_image_input_without_cargo(self) -> None:
        outcome = self.impact("infra/cloudflare/seo-static/package.json")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])

    def test_planner_or_ci_driver_change_selects_every_registered_suite_and_image(self) -> None:
        outcome = self.impact("automation/ci_impact.py")
        self.assertEqual(outcome["scope"], "full")
        self.assertEqual(len(outcome["selected_suites"]), 3)
        self.assertEqual(len(outcome["selected_images"]), 4)
        self.assertEqual(set(outcome["selected_ci_jobs"]), set(CI_PR_OWNERS))
        self.assertTrue(outcome["full_scope_reasons"])

    def test_unknown_or_missing_suite_in_report_is_rejected(self) -> None:
        outcome = self.impact("app/rust/services/gsc_sync/src/main.rs")
        outcome.pop("selected_suites")
        with self.assertRaises(ValueError):
            suite_commands_for_report(outcome)
        outcome["selected_suites"] = ["invented"]
        with self.assertRaises(ValueError):
            suite_commands_for_report(outcome)

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
